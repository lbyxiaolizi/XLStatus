//! M4 REST API for alert rules and fired/recovered events.

use crate::alerts::engine::{AlertCondition, Operator, ResourceType, TriggerMode};
use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::notifications::ensure_notification_group_owned_by;
use crate::api::v1::servers::server_visible;
use crate::api::v1::services::ensure_service_id_visible_to_auth;
use crate::api::v1::tasks::{ensure_task_ids_visible_to_auth_session, require_task_uuid_text};
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::repository::alerts::{AlertEventRepository, AlertRepository};
use crate::db::AgentRepository;
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

const ALERT_API_MAX_BODY_BYTES: usize = 64 * 1024;
const ALERT_MAX_NAME_BYTES: usize = 128;
const ALERT_MAX_CONDITIONS: usize = 32;
const ALERT_MAX_CONDITION_BYTES: usize = 4 * 1024;
const ALERT_MAX_TASK_IDS: usize = 32;
const ALERT_RULE_LIST_SCAN_BATCH: i64 = 500;
const ALERT_UUID_TEXT_LEN: usize = 36;

#[derive(Debug, Deserialize)]
pub struct CreateAlertRuleRequest {
    pub name: String,
    #[serde(default)]
    pub trigger: Option<String>,
    pub conditions: Vec<JsonValue>,
    #[serde(default)]
    pub notification_group_id: Option<String>,
    #[serde(default, alias = "fail_task_ids")]
    pub failure_task_ids: Vec<String>,
    #[serde(default, alias = "recover_task_ids")]
    pub recovery_task_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AlertRuleView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: String,
    pub conditions: Vec<JsonValue>,
    pub notification_group_id: Option<String>,
    pub failure_task_ids: Vec<String>,
    pub recovery_task_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn alert_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(ALERT_API_MAX_BODY_BYTES)
}

pub async fn create_alert_rule(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<CreateAlertRuleRequest>,
) -> Result<Json<ApiResponse<AlertRuleView>>, AppError> {
    if !has_scope(&auth, "alert:write") {
        return Err(AppError::Forbidden("missing scope: alert:write".into()));
    }
    let name = normalize_alert_name(&req.name)?;
    let conditions = parse_conditions(&req.conditions)?;
    let trigger = match req.trigger.as_deref() {
        Some("always") => TriggerMode::Always,
        _ => TriggerMode::Once,
    };
    let owner = auth.user_id.0.to_string();
    ensure_alert_conditions_visible_to_auth(&state.db, &auth, &conditions).await?;
    ensure_notification_group_owned_by(
        &state.db,
        auth.user_id.0,
        req.notification_group_id.as_deref(),
    )
    .await?;
    let failure_task_ids = normalize_id_list(req.failure_task_ids, "failure_task_ids")?;
    let recovery_task_ids = normalize_id_list(req.recovery_task_ids, "recovery_task_ids")?;
    ensure_tasks_visible_to_auth(&state.db, &auth, &failure_task_ids).await?;
    ensure_tasks_visible_to_auth(&state.db, &auth, &recovery_task_ids).await?;
    let repo = AlertRepository::new(state.db.clone());
    let row = repo
        .create(
            &owner,
            &name,
            trigger,
            &conditions,
            req.notification_group_id.as_deref(),
            &failure_task_ids,
            &recovery_task_ids,
        )
        .await
        .map_err(|e| AppError::Database(e))?;
    Ok(Json(ApiResponse::success(row_to_view(&row))))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}
fn default_limit() -> i64 {
    50
}

pub async fn list_alert_rules(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(q): Query<ListQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !has_scope(&auth, "alert:read") {
        return Err(AppError::Forbidden("missing scope: alert:read".into()));
    }
    let repo = AlertRepository::new(state.db.clone());
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);
    let owner = auth.user_id.0.to_string();
    let (rows, total) = if auth.server_ids.is_none() {
        if auth.role.is_admin() {
            (
                repo.list_page(limit, offset)
                    .await
                    .map_err(AppError::Database)?,
                repo.count().await.map_err(AppError::Database)?,
            )
        } else {
            (
                repo.list_by_owner_page(&owner, limit, offset)
                    .await
                    .map_err(AppError::Database)?,
                repo.count_by_owner(&owner)
                    .await
                    .map_err(AppError::Database)?,
            )
        }
    } else {
        list_alert_rules_visible_page(&state.db, &repo, &auth, &owner, limit, offset).await?
    };
    let view: Vec<AlertRuleView> = rows.iter().map(row_to_view).collect();
    Ok(Json(ApiResponse::success(serde_json::json!({
        "rules": view,
        "total": total,
    }))))
}

async fn list_alert_rules_visible_page(
    db: &crate::db::Db,
    repo: &AlertRepository,
    auth: &AuthSession,
    owner: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<crate::db::repository::alerts::AlertRuleRow>, i64), AppError> {
    let mut rows = Vec::new();
    let mut total = 0_i64;
    let mut skipped = 0_i64;
    let mut scan_offset = 0_i64;
    loop {
        let batch = if auth.role.is_admin() {
            repo.list_page(ALERT_RULE_LIST_SCAN_BATCH, scan_offset)
                .await
                .map_err(AppError::Database)?
        } else {
            repo.list_by_owner_page(owner, ALERT_RULE_LIST_SCAN_BATCH, scan_offset)
                .await
                .map_err(AppError::Database)?
        };
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        for row in batch {
            if !alert_rule_visible_to_auth(db, auth, &row).await? {
                continue;
            }
            total += 1;
            if skipped < offset {
                skipped += 1;
                continue;
            }
            if rows.len() < limit as usize {
                rows.push(row);
            }
        }
        if batch_len < ALERT_RULE_LIST_SCAN_BATCH as usize {
            break;
        }
        scan_offset += ALERT_RULE_LIST_SCAN_BATCH;
    }
    Ok((rows, total))
}

pub async fn delete_alert_rule(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !has_scope(&auth, "alert:delete") {
        return Err(AppError::Forbidden("missing scope: alert:delete".into()));
    }
    let id = require_alert_uuid_text(&id, "alert_rule_id")?;
    let repo = AlertRepository::new(state.db.clone());
    let owner = auth.user_id.0.to_string();
    let rule = repo
        .find_by_id(&id)
        .await
        .map_err(AppError::Database)?
        .ok_or_else(|| AppError::NotFound("alert rule not found".into()))?;
    if !alert_rule_visible_to_auth(&state.db, &auth, &rule).await? {
        return Err(AppError::Forbidden("alert rule not in scope".into()));
    }
    let removed = if auth.role.is_admin() {
        repo.delete(&id).await.map_err(AppError::Database)?
    } else {
        repo.delete_for_owner(&id, &owner)
            .await
            .map_err(AppError::Database)?
    };
    if !removed {
        return Err(AppError::NotFound("alert rule not found".into()));
    }
    Ok(Json(ApiResponse::success(
        serde_json::json!({"id": id, "deleted": true}),
    )))
}

pub async fn list_alert_events(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(q): Query<ListQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !has_scope(&auth, "alert:read") {
        return Err(AppError::Forbidden("missing scope: alert:read".into()));
    }
    let repo = AlertEventRepository::new(state.db.clone());
    let limit = q.limit.clamp(1, 500);
    let owner = auth.user_id.0.to_string();
    let events: Vec<crate::db::repository::alerts::AlertEventRow> = if auth.role.is_admin() {
        if let Some(server_ids) = auth.server_ids.as_ref() {
            repo.list_recent_for_server_ids(server_ids, limit)
                .await
                .map_err(AppError::Database)?
        } else {
            repo.list_recent(limit).await.map_err(AppError::Database)?
        }
    } else {
        if let Some(server_ids) = auth.server_ids.as_ref() {
            repo.list_recent_for_owner_server_ids(&owner, server_ids, limit)
                .await
                .map_err(AppError::Database)?
        } else {
            repo.list_recent_for_owner(&owner, limit)
                .await
                .map_err(AppError::Database)?
        }
    };
    let events: Vec<_> = events
        .into_iter()
        .filter(|event| alert_event_visible_to_auth(&auth, event))
        .collect();
    let view: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "rule_id": e.rule_id,
                "agent_id": e.agent_id,
                "service_id": e.service_id,
                "kind": e.kind,
                "payload": serde_json::from_str::<JsonValue>(&e.payload_json).unwrap_or(JsonValue::Null),
                "fired_at": e.fired_at.to_rfc3339(),
            })
        })
        .collect();
    Ok(Json(ApiResponse::success(serde_json::json!({
        "events": view,
        "total": view.len(),
    }))))
}

fn row_to_view(r: &crate::db::repository::alerts::AlertRuleRow) -> AlertRuleView {
    AlertRuleView {
        id: r.id.clone(),
        name: r.name.clone(),
        enabled: r.enabled,
        trigger: match r.trigger_mode {
            TriggerMode::Always => "always".into(),
            TriggerMode::Once => "once".into(),
        },
        conditions: r
            .conditions
            .iter()
            .map(|c| serde_json::to_value(c).unwrap_or(JsonValue::Null))
            .collect(),
        notification_group_id: r.notification_group_id.clone(),
        failure_task_ids: r.failure_task_ids.clone(),
        recovery_task_ids: r.recovery_task_ids.clone(),
        created_at: r.created_at.to_rfc3339(),
        updated_at: r.updated_at.to_rfc3339(),
    }
}

async fn ensure_tasks_visible_to_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    task_ids: &[String],
) -> Result<(), AppError> {
    ensure_task_ids_visible_to_auth_session(db, auth, task_ids).await
}

async fn ensure_alert_conditions_visible_to_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    conditions: &[AlertCondition],
) -> Result<(), AppError> {
    for condition in conditions {
        match condition {
            AlertCondition::ServiceDown { service_id, .. }
            | AlertCondition::ServiceLatency { service_id, .. }
            | AlertCondition::CertificateExpiry { service_id, .. } => {
                ensure_service_id_visible_to_auth(db, auth, service_id).await?;
            }
            AlertCondition::ServerExpiry { agent_id, .. }
            | AlertCondition::ServerTrafficQuota { agent_id, .. }
            | AlertCondition::ServerOffline { agent_id, .. }
            | AlertCondition::ServerResource { agent_id, .. } => {
                ensure_alert_agent_visible_to_auth(db, auth, agent_id).await?;
            }
        }
    }
    Ok(())
}

async fn ensure_alert_agent_visible_to_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    agent_id: &str,
) -> Result<(), AppError> {
    let agent_id = uuid::Uuid::parse_str(agent_id)
        .map(xlstatus_shared::AgentId)
        .map_err(|e| AppError::BadRequest(format!("invalid agent_id: {e}")))?;
    let agent = AgentRepository::new(db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or_else(|| AppError::NotFound("server not found".into()))?;
    if !server_visible(auth, &agent.id)
        || !(auth.role.is_admin() || agent.owner_user_id == auth.user_id)
    {
        return Err(AppError::Forbidden("server not in scope".into()));
    }
    if agent.revoked_at.is_some() {
        return Err(AppError::BadRequest("server has been revoked".into()));
    }
    Ok(())
}

fn alert_event_visible_to_auth(
    auth: &AuthSession,
    event: &crate::db::repository::alerts::AlertEventRow,
) -> bool {
    let Some(server_ids) = &auth.server_ids else {
        return true;
    };
    event
        .agent_id
        .as_ref()
        .map(|agent_id| server_ids.iter().any(|allowed| allowed == agent_id))
        .unwrap_or(false)
}

async fn alert_rule_visible_to_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    rule: &crate::db::repository::alerts::AlertRuleRow,
) -> Result<bool, AppError> {
    if !auth.role.is_admin() && rule.owner_user_id != auth.user_id.0.to_string() {
        return Ok(false);
    }
    if auth.server_ids.is_none() {
        return Ok(true);
    };
    if rule.conditions.is_empty() {
        return Ok(false);
    }
    Ok(
        ensure_alert_conditions_visible_to_auth(db, auth, &rule.conditions)
            .await
            .is_ok(),
    )
}

fn normalize_alert_name(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if value.len() > ALERT_MAX_NAME_BYTES {
        return Err(AppError::BadRequest(format!(
            "name exceeds {ALERT_MAX_NAME_BYTES} bytes"
        )));
    }
    Ok(value.to_string())
}

fn normalize_id_list(values: Vec<String>, field: &str) -> Result<Vec<String>, AppError> {
    let mut out = Vec::new();
    for value in values {
        if value.is_empty() {
            continue;
        }
        let task_id = require_task_uuid_text(&value, "task_id")?;
        if out.iter().any(|existing| existing == &task_id) {
            continue;
        }
        if out.len() >= ALERT_MAX_TASK_IDS {
            return Err(AppError::BadRequest(format!(
                "{field} exceeds {ALERT_MAX_TASK_IDS} items"
            )));
        }
        out.push(task_id);
    }
    Ok(out)
}

fn require_alert_uuid_text(value: &str, field: &str) -> Result<String, AppError> {
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if value.len() != ALERT_UUID_TEXT_LEN {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|_| AppError::BadRequest(format!("{field} must be a canonical UUID")))?;
    if parsed.to_string() != value {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    Ok(value.to_string())
}

fn parse_conditions(items: &[JsonValue]) -> Result<Vec<AlertCondition>, AppError> {
    if items.is_empty() {
        return Err(AppError::BadRequest(
            "at least one alert condition is required".into(),
        ));
    }
    if items.len() > ALERT_MAX_CONDITIONS {
        return Err(AppError::BadRequest(format!(
            "conditions exceeds {ALERT_MAX_CONDITIONS} items"
        )));
    }
    let mut out = Vec::new();
    for v in items {
        let condition_len = serde_json::to_vec(v)
            .map_err(|e| AppError::BadRequest(format!("invalid condition: {e}")))?
            .len();
        if condition_len > ALERT_MAX_CONDITION_BYTES {
            return Err(AppError::BadRequest(format!(
                "condition exceeds {ALERT_MAX_CONDITION_BYTES} bytes"
            )));
        }
        let c: AlertCondition = serde_json::from_value(v.clone())
            .map_err(|e| AppError::BadRequest(format!("invalid condition: {e}")))?;
        validate_condition(&c)?;
        out.push(c);
    }
    Ok(out)
}

fn validate_condition(condition: &AlertCondition) -> Result<(), AppError> {
    match condition {
        AlertCondition::CertificateExpiry { days_before, .. }
        | AlertCondition::ServerExpiry { days_before, .. } => {
            if *days_before < 0 {
                return Err(AppError::BadRequest(
                    "days_before must be greater than or equal to 0".into(),
                ));
            }
        }
        AlertCondition::ServerTrafficQuota { percent, .. } => {
            if !percent.is_finite() || *percent <= 0.0 {
                return Err(AppError::BadRequest(
                    "percent must be greater than 0".into(),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

#[allow(dead_code)]
fn _ensure_resource_in_scope(_: ResourceType, _: Operator) {}
#[allow(dead_code)]
fn _ensure_dt(_: DateTime<Utc>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::middleware::AuthKind;
    use crate::db::DatabaseBackend;
    use std::sync::Arc;
    use xlstatus_shared::tasks::{CoverMode, TaskType};
    use xlstatus_shared::{UserId, UserRole};

    #[tokio::test]
    async fn alert_rule_rejects_other_owner_server_condition() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let other_server = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(&db, other_server, other, "other").await;

        let err = ensure_alert_conditions_visible_to_auth(
            &db,
            &member_alert_session(owner),
            &[AlertCondition::ServerOffline {
                agent_id: other_server.to_string(),
                offline_seconds: 60,
            }],
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn alert_rule_rejects_revoked_server_condition() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let revoked_server = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();

        seed_user(&db, owner, "owner", "member").await;
        seed_agent(&db, revoked_server, owner, "revoked").await;
        revoke_agent(&db, revoked_server).await;

        let err = ensure_alert_conditions_visible_to_auth(
            &db,
            &member_alert_session(owner),
            &[AlertCondition::ServerOffline {
                agent_id: revoked_server.to_string(),
                offline_seconds: 60,
            }],
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)));
        assert!(app_error_message(&err).contains("revoked"));
    }

    #[tokio::test]
    async fn alert_rule_rejects_other_owner_service_condition() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let other_server = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();
        let other_service = "00000000-0000-0000-0000-000000000302";

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(&db, other_server, other, "other").await;
        seed_service(&db, other_service, other_server).await;

        let err = ensure_alert_conditions_visible_to_auth(
            &db,
            &member_alert_session(owner),
            &[AlertCondition::ServiceDown {
                service_id: other_service.to_string(),
                consecutive_failures: 1,
            }],
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn alert_rule_rejects_other_owner_notification_group() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let other_group = "00000000-0000-0000-0000-000000000402";

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_notification_group(&db, other_group, other, "other-group").await;

        let err = ensure_notification_group_owned_by(&db, owner, Some(other_group))
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn alert_rule_trigger_tasks_must_be_visible_to_pat_allowlist() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let allowed_server = "00000000-0000-0000-0000-000000000101";
        let blocked_server = "00000000-0000-0000-0000-000000000202";
        let blocked_task = "00000000-0000-0000-0000-000000000301";

        seed_user(&db, owner, "owner", "admin").await;
        seed_task_with_selector(
            &db,
            blocked_task,
            owner,
            serde_json::json!({ "server_ids": [blocked_server] }),
        )
        .await;

        let err = ensure_tasks_visible_to_auth(
            &db,
            &pat_alert_session(owner, UserRole::Admin, &[allowed_server]),
            &[blocked_task.to_string()],
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn list_alert_events_filters_by_rule_owner() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let owner_rule = "00000000-0000-0000-0000-000000000501";
        let other_rule = "00000000-0000-0000-0000-000000000502";
        let owner_server = "00000000-0000-0000-0000-000000000101";
        let other_server = "00000000-0000-0000-0000-000000000202";

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_alert_rule(&db, owner_rule, owner, "owner-rule").await;
        seed_alert_rule(&db, other_rule, other, "other-rule").await;
        seed_alert_event(
            &db,
            "00000000-0000-0000-0000-000000000601",
            owner_rule,
            owner_server,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_alert_event(
            &db,
            "00000000-0000-0000-0000-000000000602",
            other_rule,
            other_server,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let events = AlertEventRepository::new(db.clone())
            .list_recent_for_owner(&owner.to_string(), 10)
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].rule_id, owner_rule);
    }

    #[tokio::test]
    async fn admin_pat_alert_events_filter_allowlist_before_limit() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let allowed_rule = "00000000-0000-0000-0000-000000000511";
        let other_rule = "00000000-0000-0000-0000-000000000512";
        let allowed_server = "00000000-0000-0000-0000-000000000111";
        let other_server = "00000000-0000-0000-0000-000000000222";

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_alert_rule(&db, allowed_rule, owner, "allowed-rule").await;
        seed_alert_rule(&db, other_rule, other, "other-rule").await;
        seed_alert_event(
            &db,
            "00000000-0000-0000-0000-000000000611",
            allowed_rule,
            allowed_server,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_alert_event(
            &db,
            "00000000-0000-0000-0000-000000000612",
            other_rule,
            other_server,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let response = list_alert_events(
            State(test_state(db.clone())),
            pat_alert_session(owner, UserRole::Admin, &[allowed_server]),
            Query(ListQuery {
                limit: 1,
                offset: 0,
            }),
        )
        .await
        .unwrap()
        .0;
        let data = response.data.unwrap();
        let events = data["events"].as_array().unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["rule_id"], allowed_rule);
        assert_eq!(events[0]["agent_id"], allowed_server);
        assert_eq!(data["total"], 1);
    }

    #[tokio::test]
    async fn member_pat_alert_events_filter_owner_and_allowlist_before_limit() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let owner_allowed_rule = "00000000-0000-0000-0000-000000000521";
        let owner_other_rule = "00000000-0000-0000-0000-000000000522";
        let other_allowed_rule = "00000000-0000-0000-0000-000000000523";
        let allowed_server = "00000000-0000-0000-0000-000000000121";
        let owner_other_server = "00000000-0000-0000-0000-000000000122";

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_alert_rule(&db, owner_allowed_rule, owner, "owner-allowed-rule").await;
        seed_alert_rule(&db, owner_other_rule, owner, "owner-other-rule").await;
        seed_alert_rule(&db, other_allowed_rule, other, "other-allowed-rule").await;
        seed_alert_event(
            &db,
            "00000000-0000-0000-0000-000000000621",
            owner_allowed_rule,
            allowed_server,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_alert_event(
            &db,
            "00000000-0000-0000-0000-000000000622",
            owner_other_rule,
            owner_other_server,
            "2026-01-03T00:00:00Z",
        )
        .await;
        seed_alert_event(
            &db,
            "00000000-0000-0000-0000-000000000623",
            other_allowed_rule,
            allowed_server,
            "2026-01-04T00:00:00Z",
        )
        .await;

        let response = list_alert_events(
            State(test_state(db.clone())),
            pat_alert_session(owner, UserRole::Member, &[allowed_server]),
            Query(ListQuery {
                limit: 1,
                offset: 0,
            }),
        )
        .await
        .unwrap()
        .0;
        let data = response.data.unwrap();
        let events = data["events"].as_array().unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["rule_id"], owner_allowed_rule);
        assert_eq!(events[0]["agent_id"], allowed_server);
        assert_eq!(data["total"], 1);
    }

    #[tokio::test]
    async fn admin_pat_alert_rules_filter_allowlist_before_pagination() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let allowed_server = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000131").unwrap();
        let blocked_server = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000232").unwrap();
        let blocked_rule = "00000000-0000-0000-0000-000000000531";
        let allowed_rule = "00000000-0000-0000-0000-000000000532";

        seed_user(&db, owner, "owner", "admin").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(&db, allowed_server, owner, "allowed").await;
        seed_agent(&db, blocked_server, other, "blocked").await;
        seed_alert_rule_with_condition(
            &db,
            blocked_rule,
            other,
            "blocked-rule",
            server_offline_condition(blocked_server),
            "2026-01-03T00:00:00Z",
        )
        .await;
        seed_alert_rule_with_condition(
            &db,
            allowed_rule,
            owner,
            "allowed-rule",
            server_offline_condition(allowed_server),
            "2026-01-02T00:00:00Z",
        )
        .await;

        let response = list_alert_rules(
            State(test_state(db.clone())),
            pat_alert_session(owner, UserRole::Admin, &[&allowed_server.to_string()]),
            Query(ListQuery {
                limit: 1,
                offset: 0,
            }),
        )
        .await
        .unwrap()
        .0;
        let data = response.data.unwrap();
        let rules = data["rules"].as_array().unwrap();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["id"], allowed_rule);
        assert_eq!(data["total"], 1);
    }

    #[tokio::test]
    async fn member_alert_rules_ignore_other_owner_invalid_json() {
        let db = test_db().await;
        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let owner_server = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000141").unwrap();
        let owner_rule = "00000000-0000-0000-0000-000000000541";
        let dirty_rule = "00000000-0000-0000-0000-000000000542";

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(&db, owner_server, owner, "owner-server").await;
        seed_alert_rule_with_condition(
            &db,
            owner_rule,
            owner,
            "owner-rule",
            server_offline_condition(owner_server),
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_invalid_alert_rule_json(&db, dirty_rule, other, "dirty-rule", "2026-01-03T00:00:00Z")
            .await;

        let response = list_alert_rules(
            State(test_state(db.clone())),
            member_alert_session(owner),
            Query(ListQuery {
                limit: 10,
                offset: 0,
            }),
        )
        .await
        .unwrap()
        .0;
        let data = response.data.unwrap();
        let rules = data["rules"].as_array().unwrap();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["id"], owner_rule);
        assert_eq!(data["total"], 1);
    }

    #[test]
    fn alert_rule_resource_limits_are_explicit() {
        assert_eq!(ALERT_API_MAX_BODY_BYTES, 64 * 1024);
        assert_eq!(ALERT_MAX_NAME_BYTES, 128);
        assert_eq!(ALERT_MAX_CONDITIONS, 32);
        assert_eq!(ALERT_MAX_TASK_IDS, 32);
        assert_eq!(ALERT_UUID_TEXT_LEN, 36);
    }

    #[test]
    fn alert_rule_path_ids_require_canonical_uuid_text() {
        let canonical = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        assert_eq!(
            require_alert_uuid_text(canonical, "alert_rule_id").unwrap(),
            canonical
        );

        for value in [
            "alert-rule-a",
            " aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa ",
            "aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaaaaaa",
            "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaax",
        ] {
            assert!(
                require_alert_uuid_text(value, "alert_rule_id").is_err(),
                "{value:?} should be rejected"
            );
        }
    }

    #[test]
    fn alert_rule_rejects_oversized_name_and_task_lists() {
        assert!(normalize_alert_name("cpu alert").is_ok());
        assert!(normalize_alert_name(&"a".repeat(ALERT_MAX_NAME_BYTES + 1)).is_err());

        let valid_ids = (0..ALERT_MAX_TASK_IDS)
            .map(|idx| format!("00000000-0000-0000-0000-{idx:012}"))
            .collect::<Vec<_>>();
        assert!(normalize_id_list(valid_ids, "failure_task_ids").is_ok());

        let too_many_ids = (0..=ALERT_MAX_TASK_IDS)
            .map(|idx| format!("00000000-0000-0000-0000-{idx:012}"))
            .collect::<Vec<_>>();
        assert!(normalize_id_list(too_many_ids, "failure_task_ids").is_err());
        assert!(normalize_id_list(vec!["not-a-uuid".into()], "failure_task_ids").is_err());
        for value in [
            " 00000000-0000-0000-0000-000000000001",
            "00000000-0000-0000-0000-000000000001 ",
            "00000000000000000000000000000001",
            "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
        ] {
            assert!(
                normalize_id_list(vec![value.into()], "failure_task_ids").is_err(),
                "{value:?} should be rejected"
            );
        }
    }

    #[test]
    fn alert_rule_rejects_oversized_conditions() {
        let condition = serde_json::json!({
            "type": "server_offline",
            "agent_id": "00000000-0000-0000-0000-000000000101",
            "offline_seconds": 60
        });
        assert!(parse_conditions(&[condition.clone()]).is_ok());

        let too_many = vec![condition.clone(); ALERT_MAX_CONDITIONS + 1];
        assert!(parse_conditions(&too_many).is_err());

        let oversized = serde_json::json!({
            "type": "server_offline",
            "agent_id": "00000000-0000-0000-0000-000000000101",
            "offline_seconds": 60,
            "padding": "a".repeat(ALERT_MAX_CONDITION_BYTES)
        });
        assert!(parse_conditions(&[oversized]).is_err());
    }

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    fn test_state(db: DatabaseBackend) -> AppState {
        AppState {
            db,
            config: Arc::new(crate::config::Config::default()),
            agent_jwt_challenges: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            metrics: xlstatus_tsdb::MetricStore::in_memory(),
            realtime: crate::realtime::BroadcastHub::new(),
            session_registry: crate::grpc::SessionRegistry::new(),
            terminal_sessions: crate::api::v1::terminal::TerminalSessionRegistry::new(),
            io_registry: crate::grpc::IoRegistry::new(),
        }
    }

    fn member_alert_session(user_id: uuid::Uuid) -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: UserId(user_id),
            username: "member".into(),
            role: UserRole::Member,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: vec!["alert:read".into(), "alert:write".into()],
            server_ids: None,
            pat_id: None,
        }
    }

    fn pat_alert_session(user_id: uuid::Uuid, role: UserRole, server_ids: &[&str]) -> AuthSession {
        AuthSession {
            session_id: "pat-session".into(),
            user_id: UserId(user_id),
            username: "pat".into(),
            role,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::PersonalAccessToken,
            scopes: vec!["alert:read".into()],
            server_ids: Some(server_ids.iter().map(|id| id.to_string()).collect()),
            pat_id: Some("00000000-0000-0000-0000-000000000701".into()),
        }
    }

    async fn seed_user(db: &DatabaseBackend, id: uuid::Uuid, username: &str, role: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'x', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_agent(db: &DatabaseBackend, id: uuid::Uuid, owner: uuid::Uuid, name: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at) VALUES (?, ?, 'pk', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(owner.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn revoke_agent(db: &DatabaseBackend, id: uuid::Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query("UPDATE agents SET revoked_at = '2026-06-22T00:00:00Z' WHERE id = ?")
            .bind(id.to_string())
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_service(db: &DatabaseBackend, id: &str, server_id: uuid::Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO services (id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, cover_mode, created_at, updated_at) VALUES (?, 'svc', 'http', 'https://example.com', 60, 10, 1, ?, 'specific', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(server_id.to_string())
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO service_servers (service_id, server_id, created_at) VALUES (?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(server_id.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_notification_group(
        db: &DatabaseBackend,
        id: &str,
        owner: uuid::Uuid,
        name: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notification_groups (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_alert_rule(db: &DatabaseBackend, id: &str, owner: uuid::Uuid, name: &str) {
        seed_alert_rule_with_condition(
            db,
            id,
            owner,
            name,
            server_offline_condition(
                uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap(),
            ),
            "2026-01-01T00:00:00Z",
        )
        .await;
    }

    fn server_offline_condition(agent_id: uuid::Uuid) -> AlertCondition {
        AlertCondition::ServerOffline {
            agent_id: agent_id.to_string(),
            offline_seconds: 60,
        }
    }

    async fn seed_alert_rule_with_condition(
        db: &DatabaseBackend,
        id: &str,
        owner: uuid::Uuid,
        name: &str,
        condition: AlertCondition,
        created_at: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        let rules_json = serde_json::to_string(&[condition]).unwrap();
        sqlx::query(
            "INSERT INTO alert_rules (id, owner_user_id, name, enabled, trigger_mode, rules_json, created_at, updated_at) VALUES (?, ?, ?, 1, 'once', ?, ?, ?)",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(name)
        .bind(rules_json)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_invalid_alert_rule_json(
        db: &DatabaseBackend,
        id: &str,
        owner: uuid::Uuid,
        name: &str,
        created_at: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO alert_rules (id, owner_user_id, name, enabled, trigger_mode, rules_json, created_at, updated_at) VALUES (?, ?, ?, 1, 'once', 'not-json', ?, ?)",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(name)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_alert_event(
        db: &DatabaseBackend,
        id: &str,
        rule_id: &str,
        agent_id: &str,
        fired_at: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO alert_events (id, rule_id, agent_id, kind, payload_json, fired_at) VALUES (?, ?, ?, 'fired', '{}', ?)",
        )
        .bind(id)
        .bind(rule_id)
        .bind(agent_id)
        .bind(fired_at)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_task_with_selector(
        db: &DatabaseBackend,
        id: &str,
        owner: uuid::Uuid,
        selector: serde_json::Value,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO tasks (id, owner_user_id, name, task_type, command, cover_mode, server_selector_json, created_at, updated_at) VALUES (?, ?, 'task', ?, 'true', ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(serde_json::to_string(&TaskType::Shell).unwrap())
        .bind(serde_json::to_string(&CoverMode::Specific).unwrap())
        .bind(selector.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    fn app_error_message(err: &AppError) -> String {
        match err {
            AppError::BadRequest(message)
            | AppError::Forbidden(message)
            | AppError::Unauthorized(message)
            | AppError::NotFound(message)
            | AppError::TooManyRequests(message) => message.clone(),
            AppError::Database(err) => err.to_string(),
        }
    }
}
