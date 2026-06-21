//! M4 REST API for alert rules and fired/recovered events.

use crate::alerts::engine::{AlertCondition, Operator, ResourceType, TriggerMode};
use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::notifications::ensure_notification_group_owned_by;
use crate::api::v1::servers::server_visible;
use crate::api::v1::services::ensure_service_id_visible_to_auth;
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::repository::alerts::{AlertEventRepository, AlertRepository};
use crate::db::AgentRepository;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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

pub async fn create_alert_rule(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<CreateAlertRuleRequest>,
) -> Result<Json<ApiResponse<AlertRuleView>>, AppError> {
    if !has_scope(&auth, "alert:write") {
        return Err(AppError::Forbidden("missing scope: alert:write".into()));
    }
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
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
    let failure_task_ids = normalize_id_list(req.failure_task_ids);
    let recovery_task_ids = normalize_id_list(req.recovery_task_ids);
    ensure_tasks_owned_by(&state.db, &owner, &failure_task_ids).await?;
    ensure_tasks_owned_by(&state.db, &owner, &recovery_task_ids).await?;
    let repo = AlertRepository::new(state.db.clone());
    let row = repo
        .create(
            &owner,
            &req.name,
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
    let owner = auth.user_id.0.to_string();
    let rows = if auth.role.is_admin() {
        repo.list().await.map_err(AppError::Database)?
    } else {
        repo.list_by_owner(&owner)
            .await
            .map_err(AppError::Database)?
    };
    let mut visible_rows = Vec::new();
    for row in rows {
        if alert_rule_visible_to_auth(&state.db, &auth, &row).await? {
            visible_rows.push(row);
        }
    }
    let total = visible_rows.len() as i64;
    let start = q.offset.max(0) as usize;
    let end = (start + q.limit.clamp(1, 500) as usize).min(visible_rows.len());
    let view: Vec<AlertRuleView> = visible_rows[start..end]
        .iter()
        .map(|r| row_to_view(r))
        .collect();
    Ok(Json(ApiResponse::success(serde_json::json!({
        "rules": view,
        "total": total,
    }))))
}

pub async fn delete_alert_rule(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !has_scope(&auth, "alert:delete") {
        return Err(AppError::Forbidden("missing scope: alert:delete".into()));
    }
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
    let events: Vec<crate::db::repository::alerts::AlertEventRow> = if auth.role.is_admin() {
        repo.list_recent(limit)
            .await
            .map_err(AppError::Database)?
            .into_iter()
            .filter(|event| alert_event_visible_to_auth(&auth, event))
            .collect()
    } else {
        repo.list_recent_for_owner(&auth.user_id.0.to_string(), limit)
            .await
            .map_err(AppError::Database)?
            .into_iter()
            .filter(|event| alert_event_visible_to_auth(&auth, event))
            .collect()
    };
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

async fn ensure_tasks_owned_by(
    db: &crate::db::Db,
    owner_user_id: &str,
    task_ids: &[String],
) -> Result<(), AppError> {
    for task_id in task_ids {
        let exists = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM tasks WHERE id = ? AND owner_user_id = ?")
                        .bind(task_id)
                        .bind(owner_user_id)
                        .fetch_one(pool)
                        .await
                        .map_err(db_err)?;
                row.0 > 0
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let owner = uuid::Uuid::parse_str(owner_user_id)
                    .map_err(|e| AppError::BadRequest(format!("invalid owner_user_id: {e}")))?;
                let row: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM tasks WHERE id = $1 AND owner_user_id = $2",
                )
                .bind(task_id)
                .bind(owner)
                .fetch_one(pool)
                .await
                .map_err(db_err)?;
                row.0 > 0
            }
        };
        if !exists {
            return Err(AppError::BadRequest(format!(
                "task {task_id} does not exist or is not owned by current user"
            )));
        }
    }
    Ok(())
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
    if server_visible(auth, &agent.id)
        && (auth.role.is_admin() || agent.owner_user_id == auth.user_id)
    {
        Ok(())
    } else {
        Err(AppError::Forbidden("server not in scope".into()))
    }
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

fn normalize_id_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn db_err(err: sqlx::Error) -> AppError {
    AppError::Database(anyhow::anyhow!(err))
}

fn parse_conditions(items: &[JsonValue]) -> Result<Vec<AlertCondition>, AppError> {
    if items.is_empty() {
        return Err(AppError::BadRequest(
            "at least one alert condition is required".into(),
        ));
    }
    let mut out = Vec::new();
    for v in items {
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

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
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
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        let rules_json = serde_json::to_string(&[AlertCondition::ServerOffline {
            agent_id: "00000000-0000-0000-0000-000000000101".into(),
            offline_seconds: 60,
        }])
        .unwrap();
        sqlx::query(
            "INSERT INTO alert_rules (id, owner_user_id, name, enabled, trigger_mode, rules_json, created_at, updated_at) VALUES (?, ?, ?, 1, 'once', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(name)
        .bind(rules_json)
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
}
