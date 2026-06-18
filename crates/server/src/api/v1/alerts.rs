//! M4 REST API for alert rules and fired/recovered events.

use crate::alerts::engine::{AlertCondition, Operator, ResourceType, TriggerMode};
use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::repository::alerts::{AlertEventRepository, AlertRepository};
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
}

#[derive(Debug, Serialize)]
pub struct AlertRuleView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: String,
    pub conditions: Vec<JsonValue>,
    pub notification_group_id: Option<String>,
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
    let repo = AlertRepository::new(state.db.clone());
    let row = repo
        .create(
            &owner,
            &req.name,
            trigger,
            &conditions,
            req.notification_group_id.as_deref(),
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
    let rows = repo.list().await.map_err(AppError::Database)?;
    let total = rows.len() as i64;
    let start = q.offset.max(0) as usize;
    let end = (start + q.limit.clamp(1, 500) as usize).min(rows.len());
    let view: Vec<AlertRuleView> = rows[start..end].iter().map(|r| row_to_view(r)).collect();
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
    if !has_scope(&auth, "alert:write") {
        return Err(AppError::Forbidden("missing scope: alert:write".into()));
    }
    let repo = AlertRepository::new(state.db.clone());
    let removed = repo.delete(&id).await.map_err(AppError::Database)?;
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
    let events = repo
        .list_recent(q.limit.clamp(1, 500))
        .await
        .map_err(AppError::Database)?;
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
        created_at: r.created_at.to_rfc3339(),
        updated_at: r.updated_at.to_rfc3339(),
    }
}

fn parse_conditions(items: &[JsonValue]) -> Result<Vec<AlertCondition>, AppError> {
    let mut out = Vec::new();
    for v in items {
        let c: AlertCondition = serde_json::from_value(v.clone())
            .map_err(|e| AppError::BadRequest(format!("invalid condition: {e}")))?;
        out.push(c);
    }
    Ok(out)
}

#[allow(dead_code)]
fn _ensure_resource_in_scope(_: ResourceType, _: Operator) {}
#[allow(dead_code)]
fn _ensure_dt(_: DateTime<Utc>) {}
