//! M3 server listing + metrics endpoints.
//!
//! `/api/v1/servers` returns the rows in the `agents` table, plus
//! the latest `last_state_json` parsed into a flat object. The status
//! is derived from `last_seen_at` (within 30 s = online, otherwise
//! offline). `/api/v1/servers/:id/metrics` returns the MetricStore
//! time series for one agent.
//!
//! Both routes are protected by the standard session middleware and
//! scope-validated through `auth/rbac.rs`. The PAT path requires
//! `server:read` and a matching `server_id` in the PAT allowlist.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{require_sensitive_totp, AppError, AppState};
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::{AgentRepository, DatabaseBackend, UserRepository};

pub use crate::realtime::ws::ws_servers;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
#[cfg(test)]
use chrono::Duration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row};
use std::collections::HashSet;
use uuid::Uuid;
use xlstatus_shared::{AgentId, UserId};
use xlstatus_tsdb::{MetricSeries, QueryRange};

const ONLINE_THRESHOLD_SECS: i64 = 30;

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

#[derive(Debug, Serialize)]
pub struct ServerView {
    pub id: String,
    pub name: String,
    pub remark: Option<String>,
    pub public_note: Option<String>,
    pub expires_at: Option<String>,
    pub renewal_price: Option<String>,
    pub price: Option<String>,
    pub currency: Option<String>,
    pub billing_cycle: Option<String>,
    pub auto_renew: Option<bool>,
    pub traffic_quota_bytes: Option<i64>,
    pub traffic_quota_type: Option<String>,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub plan: Option<String>,
    pub tags: Vec<String>,
    pub accent_color: Option<String>,
    pub dashboard_visible: Option<bool>,
    pub hide_for_guest: Option<bool>,
    pub display_order: Option<i64>,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub cpu_percent: Option<f64>,
    pub memory_used: Option<i64>,
    pub memory_total: Option<i64>,
    pub load_1: Option<f64>,
    pub net_rx_bps: Option<i64>,
    pub net_tx_bps: Option<i64>,
    pub network_in_total: Option<i64>,
    pub network_out_total: Option<i64>,
    pub uptime_seconds: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ListServersResponse {
    pub servers: Vec<ServerView>,
    pub total: i64,
}

pub async fn list_servers(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(q): Query<ListQuery>,
) -> Result<Json<ApiResponse<ListServersResponse>>, AppError> {
    if !has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);

    let agent_repo = AgentRepository::new(state.db.clone());
    let (rows, total) = agent_repo.list_with_state(limit, offset).await?;
    let now = Utc::now();
    let mut servers = Vec::with_capacity(rows.len());
    for row in rows.into_iter() {
        if !server_visible(&auth, &row.agent.id) {
            continue;
        }
        let agent = row.agent;
        let last_seen_age = agent
            .last_seen_at
            .map(|ts| (now - ts).num_seconds())
            .unwrap_or(i64::MAX);
        let status = if agent.revoked_at.is_some() {
            "revoked"
        } else if last_seen_age <= ONLINE_THRESHOLD_SECS {
            "online"
        } else {
            "offline"
        };
        let parsed = row
            .last_state_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        let parsed_info = row
            .last_info_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        let dashboard = dashboard_metadata(
            row.dashboard_metadata_json.as_deref(),
            &[parsed_info.as_ref(), parsed.as_ref()],
        );
        let network_rates = network_rates_from_store(&state.metrics, agent.id.0);
        servers.push(ServerView {
            id: agent.id.0.to_string(),
            name: agent.name,
            remark: row.remark.or_else(|| {
                metadata_string(
                    &[parsed_info.as_ref(), parsed.as_ref()],
                    &["remark", "note", "description"],
                )
            }),
            public_note: dashboard.public_note.clone(),
            expires_at: row.expires_at.or_else(|| {
                metadata_string(
                    &[parsed_info.as_ref(), parsed.as_ref()],
                    &["expires_at", "expired_at", "expire_at", "due_at", "end_at"],
                )
            }),
            renewal_price: row.renewal_price.or_else(|| {
                metadata_string(
                    &[parsed_info.as_ref(), parsed.as_ref()],
                    &[
                        "renewal_price",
                        "renew_price",
                        "renewal",
                        "price",
                        "billing_price",
                    ],
                )
            }),
            price: dashboard.price.clone(),
            currency: dashboard.currency.clone(),
            billing_cycle: dashboard.billing_cycle.clone(),
            auto_renew: dashboard.auto_renew,
            traffic_quota_bytes: dashboard.traffic_quota_bytes,
            traffic_quota_type: dashboard.traffic_quota_type.clone(),
            provider: dashboard.provider,
            region: dashboard.region,
            plan: dashboard.plan,
            tags: dashboard.tags,
            accent_color: dashboard.accent_color,
            dashboard_visible: dashboard.dashboard_visible,
            hide_for_guest: dashboard.hide_for_guest,
            display_order: dashboard.display_order,
            status: status.to_string(),
            last_seen_at: agent.last_seen_at.map(|t| t.to_rfc3339()),
            cpu_percent: parsed
                .as_ref()
                .and_then(|v| v.get("cpu_percent"))
                .and_then(|v| v.as_f64()),
            memory_used: parsed
                .as_ref()
                .and_then(|v| v.get("memory_used"))
                .and_then(|v| v.as_i64()),
            memory_total: parsed
                .as_ref()
                .and_then(|v| v.get("memory_total"))
                .and_then(|v| v.as_i64()),
            load_1: parsed
                .as_ref()
                .and_then(|v| v.get("load_1"))
                .and_then(|v| v.as_f64()),
            net_rx_bps: parsed
                .as_ref()
                .and_then(|v| json_i64_by_keys(v, &["net_rx_bps", "network_in_speed"]))
                .or(network_rates.0),
            net_tx_bps: parsed
                .as_ref()
                .and_then(|v| json_i64_by_keys(v, &["net_tx_bps", "network_out_speed"]))
                .or(network_rates.1),
            network_in_total: parsed.as_ref().and_then(|v| network_total(v, "bytes_recv")),
            network_out_total: parsed.as_ref().and_then(|v| network_total(v, "bytes_sent")),
            uptime_seconds: parsed
                .as_ref()
                .and_then(|v| json_i64_by_keys(v, &["uptime_seconds", "uptime"])),
        });
    }
    Ok(Json(ApiResponse::success(ListServersResponse {
        servers,
        total,
    })))
}

#[derive(Debug, Serialize)]
pub struct ServerDetailResponse {
    pub id: String,
    pub name: String,
    pub remark: Option<String>,
    pub public_note: Option<String>,
    pub expires_at: Option<String>,
    pub renewal_price: Option<String>,
    pub price: Option<String>,
    pub currency: Option<String>,
    pub billing_cycle: Option<String>,
    pub auto_renew: Option<bool>,
    pub traffic_quota_bytes: Option<i64>,
    pub traffic_quota_type: Option<String>,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub plan: Option<String>,
    pub tags: Vec<String>,
    pub accent_color: Option<String>,
    pub dashboard_visible: Option<bool>,
    pub hide_for_guest: Option<bool>,
    pub display_order: Option<i64>,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub last_state: Option<serde_json::Value>,
    pub last_info: Option<serde_json::Value>,
}

pub async fn get_server(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ServerDetailResponse>>, AppError> {
    if !has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    let agent_id = parse_agent_id(&id)?;
    if !server_visible(&auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let agent_repo = AgentRepository::new(state.db.clone());
    let row = agent_repo
        .find_by_id_with_state(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".to_string()))?;
    let agent = row.agent;
    let now = Utc::now();
    let last_seen_age = agent
        .last_seen_at
        .map(|ts| (now - ts).num_seconds())
        .unwrap_or(i64::MAX);
    let status = if agent.revoked_at.is_some() {
        "revoked"
    } else if last_seen_age <= ONLINE_THRESHOLD_SECS {
        "online"
    } else {
        "offline"
    };
    let last_state = row
        .last_state_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    let last_info = row
        .last_info_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    let dashboard = dashboard_metadata(
        row.dashboard_metadata_json.as_deref(),
        &[last_info.as_ref(), last_state.as_ref()],
    );
    Ok(Json(ApiResponse::success(ServerDetailResponse {
        id: agent.id.0.to_string(),
        name: agent.name,
        remark: row.remark.or_else(|| {
            metadata_string(
                &[last_info.as_ref(), last_state.as_ref()],
                &["remark", "note", "description"],
            )
        }),
        public_note: dashboard.public_note.clone(),
        expires_at: row.expires_at.or_else(|| {
            metadata_string(
                &[last_info.as_ref(), last_state.as_ref()],
                &["expires_at", "expired_at", "expire_at", "due_at", "end_at"],
            )
        }),
        renewal_price: row.renewal_price.or_else(|| {
            metadata_string(
                &[last_info.as_ref(), last_state.as_ref()],
                &[
                    "renewal_price",
                    "renew_price",
                    "renewal",
                    "price",
                    "billing_price",
                ],
            )
        }),
        price: dashboard.price.clone(),
        currency: dashboard.currency.clone(),
        billing_cycle: dashboard.billing_cycle.clone(),
        auto_renew: dashboard.auto_renew,
        traffic_quota_bytes: dashboard.traffic_quota_bytes,
        traffic_quota_type: dashboard.traffic_quota_type.clone(),
        provider: dashboard.provider,
        region: dashboard.region,
        plan: dashboard.plan,
        tags: dashboard.tags,
        accent_color: dashboard.accent_color,
        dashboard_visible: dashboard.dashboard_visible,
        hide_for_guest: dashboard.hide_for_guest,
        display_order: dashboard.display_order,
        status: status.to_string(),
        last_seen_at: agent.last_seen_at.map(|t| t.to_rfc3339()),
        last_state,
        last_info,
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateServerRequest {
    pub name: Option<String>,
    pub remark: Option<String>,
    pub expires_at: Option<String>,
    pub renewal_price: Option<String>,
    pub public_note: Option<String>,
    pub price: Option<String>,
    pub currency: Option<String>,
    pub billing_cycle: Option<String>,
    pub auto_renew: Option<bool>,
    pub traffic_quota_bytes: Option<i64>,
    pub traffic_quota_type: Option<String>,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub plan: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    pub accent_color: Option<String>,
    pub dashboard_visible: Option<bool>,
    pub hide_for_guest: Option<bool>,
    pub display_order: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct BatchUpdateServersRequest {
    pub server_ids: Vec<String>,
    pub action: ServerBatchAction,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dashboard_visible: Option<bool>,
    #[serde(default)]
    pub owner_user_id: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerBatchAction {
    SetTags,
    AddTags,
    RemoveTags,
    SetDashboardVisible,
    TransferOwner,
    Delete,
    MoveGroup,
}

#[derive(Debug, Serialize)]
pub struct BatchUpdateServersResponse {
    pub results: Vec<BatchUpdateServerResult>,
    pub updated: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize)]
pub struct BatchUpdateServerResult {
    pub id: String,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListServerTransfersQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub server_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ServerOwnerTransferView {
    pub id: String,
    pub server_id: String,
    pub from_user_id: Option<String>,
    pub to_user_id: String,
    pub requested_by_user_id: Option<String>,
    pub api_token_id: Option<String>,
    pub status: String,
    pub attempts: i64,
    pub error: Option<String>,
    pub completed_at: Option<String>,
    pub cancelled_at: Option<String>,
    pub last_attempt_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ServerOwnerTransferListResponse {
    pub transfers: Vec<ServerOwnerTransferView>,
    pub total: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateServerGroupRequest {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub display_order: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateServerGroupRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub display_order: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AddServerGroupMembersRequest {
    pub server_ids: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ServerGroupView {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub color: Option<String>,
    pub display_order: Option<i64>,
    pub server_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ServerGroupListResponse {
    pub groups: Vec<ServerGroupView>,
    pub total: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DashboardMetadata {
    public_note: Option<String>,
    provider: Option<String>,
    region: Option<String>,
    plan: Option<String>,
    price: Option<String>,
    currency: Option<String>,
    billing_cycle: Option<String>,
    auto_renew: Option<bool>,
    traffic_quota_bytes: Option<i64>,
    traffic_quota_type: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    accent_color: Option<String>,
    dashboard_visible: Option<bool>,
    hide_for_guest: Option<bool>,
    display_order: Option<i64>,
}

pub async fn update_server(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Json(req): Json<UpdateServerRequest>,
) -> Result<Json<ApiResponse<ServerDetailResponse>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    let agent_id = parse_agent_id(&id)?;
    if !server_visible(&auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if req.name.is_some() && name.is_none() {
        return Err(AppError::BadRequest("name is required".into()));
    }

    let remark = normalize_optional_label(req.remark);
    let expires_at = normalize_optional_label(req.expires_at);
    let renewal_price = normalize_optional_label(req.renewal_price);
    if matches!(req.traffic_quota_bytes, Some(value) if value < 0) {
        return Err(AppError::BadRequest(
            "traffic_quota_bytes must be greater than or equal to 0".into(),
        ));
    }
    let dashboard_metadata = DashboardMetadata {
        public_note: normalize_optional_label(req.public_note),
        provider: normalize_optional_label(req.provider),
        region: normalize_optional_label(req.region),
        plan: normalize_optional_label(req.plan),
        price: normalize_optional_label(req.price),
        currency: normalize_optional_label(req.currency),
        billing_cycle: normalize_optional_label(req.billing_cycle),
        auto_renew: req.auto_renew,
        traffic_quota_bytes: req.traffic_quota_bytes,
        traffic_quota_type: normalize_optional_label(req.traffic_quota_type),
        tags: normalize_tags(req.tags.unwrap_or_default()),
        accent_color: normalize_accent_color(req.accent_color)?,
        dashboard_visible: req.dashboard_visible,
        hide_for_guest: req.hide_for_guest,
        display_order: req.display_order,
    };
    let dashboard_metadata_json = serde_json::to_string(&dashboard_metadata)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let agent_repo = AgentRepository::new(state.db.clone());
    let updated = agent_repo
        .update_dashboard_metadata(
            agent_id,
            name,
            remark.as_deref(),
            expires_at.as_deref(),
            renewal_price.as_deref(),
            Some(&dashboard_metadata_json),
        )
        .await?;
    if !updated {
        return Err(AppError::NotFound("agent not found".into()));
    }

    get_server(State(state), auth, Path(id)).await
}

pub async fn batch_update_servers(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    Json(req): Json<BatchUpdateServersRequest>,
) -> Result<Json<ApiResponse<BatchUpdateServersResponse>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    if matches!(
        req.action,
        ServerBatchAction::Delete | ServerBatchAction::TransferOwner
    ) {
        require_sensitive_totp(&state.db, auth.user_id, &headers).await?;
    }
    if req.server_ids.is_empty() {
        return Err(AppError::BadRequest("server_ids is required".into()));
    }
    if req.server_ids.len() > 200 {
        return Err(AppError::BadRequest(
            "server_ids must contain at most 200 items".into(),
        ));
    }

    let target_owner = if matches!(req.action, ServerBatchAction::TransferOwner) {
        if !auth.role.is_admin() {
            return Err(AppError::Forbidden(
                "admin role required for ownership transfer".into(),
            ));
        }
        let owner = req
            .owner_user_id
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("owner_user_id is required".into()))
            .and_then(parse_user_id)?;
        let user_repo = UserRepository::new(state.db.clone());
        if user_repo.find_by_id(owner).await?.is_none() {
            return Err(AppError::NotFound("target user not found".into()));
        }
        Some(owner)
    } else {
        None
    };

    if matches!(req.action, ServerBatchAction::SetDashboardVisible)
        && req.dashboard_visible.is_none()
    {
        return Err(AppError::BadRequest("dashboard_visible is required".into()));
    }
    if matches!(req.action, ServerBatchAction::MoveGroup) && req.group_id.is_none() {
        return Err(AppError::BadRequest("group_id is required".into()));
    }
    if let (ServerBatchAction::MoveGroup, Some(group_id)) = (req.action, req.group_id.as_deref()) {
        load_server_group(&state.db, &auth, group_id).await?;
    }

    let normalized_tags = normalize_tags(req.tags);
    if matches!(
        req.action,
        ServerBatchAction::SetTags | ServerBatchAction::AddTags | ServerBatchAction::RemoveTags
    ) && normalized_tags.is_empty()
    {
        return Err(AppError::BadRequest("tags is required".into()));
    }

    let agent_repo = AgentRepository::new(state.db.clone());
    let actor_ip = client_ip_from_headers(&headers);
    let mut results = Vec::new();
    for id in dedupe_ids(req.server_ids) {
        let result = match apply_batch_action(
            &state.db,
            &agent_repo,
            &auth,
            &actor_ip,
            &id,
            req.action,
            &normalized_tags,
            req.dashboard_visible,
            target_owner,
            req.group_id.as_deref(),
        )
        .await
        {
            Ok(()) => BatchUpdateServerResult {
                id,
                success: true,
                error: None,
            },
            Err(err) => BatchUpdateServerResult {
                id,
                success: false,
                error: Some(err),
            },
        };
        results.push(result);
    }
    let updated = results.iter().filter(|item| item.success).count();
    let failed = results.len().saturating_sub(updated);
    Ok(Json(ApiResponse::success(BatchUpdateServersResponse {
        results,
        updated,
        failed,
    })))
}

pub async fn list_server_owner_transfers(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(q): Query<ListServerTransfersQuery>,
) -> Result<Json<ApiResponse<ServerOwnerTransferListResponse>>, AppError> {
    if !has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    require_transfer_admin(&auth)?;
    let limit = q.limit.clamp(1, 200);
    let offset = q.offset.max(0);
    let server_id = q
        .server_id
        .as_deref()
        .map(parse_agent_id)
        .transpose()?
        .map(|id| id.0.to_string());
    if let Some(server_id) = server_id.as_deref() {
        ensure_transfer_server_scope(&auth, server_id)?;
    }
    let (transfers, total) =
        load_server_owner_transfers(&state.db, &auth, server_id.as_deref(), limit, offset).await?;
    Ok(Json(ApiResponse::success(
        ServerOwnerTransferListResponse { transfers, total },
    )))
}

pub async fn retry_server_owner_transfer(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ServerOwnerTransferView>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    require_transfer_admin(&auth)?;
    require_sensitive_totp(&state.db, auth.user_id, &headers).await?;
    let transfer = load_server_owner_transfer(&state.db, &id)
        .await?
        .ok_or(AppError::NotFound("server transfer not found".into()))?;
    if transfer.status == "completed" {
        return Err(AppError::BadRequest(
            "completed transfer cannot be retried".into(),
        ));
    }
    ensure_transfer_server_scope(&auth, &transfer.server_id)?;

    let agent_id = parse_agent_id(&transfer.server_id)?;
    let target_owner = parse_user_id(&transfer.to_user_id)?;
    let agent_repo = AgentRepository::new(state.db.clone());
    let row = match agent_repo.find_by_id_with_state(agent_id).await? {
        Some(row) => row,
        None => {
            mark_server_owner_transfer_failed(&state.db, &id, "agent not found").await?;
            let updated = load_server_owner_transfer(&state.db, &id)
                .await?
                .ok_or(AppError::NotFound("server transfer not found".into()))?;
            return Ok(Json(ApiResponse::success(updated)));
        }
    };

    let actor_ip = client_ip_from_headers(&headers);
    let result = if row.agent.owner_user_id == target_owner {
        mark_server_owner_transfer_completed(&state.db, &id, row.agent.owner_user_id, &auth, true)
            .await
    } else {
        perform_owner_transfer_existing(
            &state.db,
            &id,
            agent_id,
            row.agent.owner_user_id,
            target_owner,
            &auth,
        )
        .await
    };
    match result {
        Ok(()) => {
            warn_if_audit_failed(
                record_server_transfer_audit(
                    &state.db,
                    &auth,
                    &actor_ip,
                    "server.transfer_owner.retry",
                    "success",
                    &id,
                    &transfer.server_id,
                    None,
                )
                .await,
            );
        }
        Err(err) => {
            let message = error_message(err);
            mark_server_owner_transfer_failed(&state.db, &id, &message).await?;
            warn_if_audit_failed(
                record_server_transfer_audit(
                    &state.db,
                    &auth,
                    &actor_ip,
                    "server.transfer_owner.retry",
                    "failure",
                    &id,
                    &transfer.server_id,
                    Some(&message),
                )
                .await,
            );
        }
    }

    let updated = load_server_owner_transfer(&state.db, &id)
        .await?
        .ok_or(AppError::NotFound("server transfer not found".into()))?;
    Ok(Json(ApiResponse::success(updated)))
}

pub async fn cancel_server_owner_transfer(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ServerOwnerTransferView>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    require_transfer_admin(&auth)?;
    let transfer = load_server_owner_transfer(&state.db, &id)
        .await?
        .ok_or(AppError::NotFound("server transfer not found".into()))?;
    if transfer.status == "completed" {
        return Err(AppError::BadRequest(
            "completed transfer cannot be cancelled".into(),
        ));
    }
    ensure_transfer_server_scope(&auth, &transfer.server_id)?;
    mark_server_owner_transfer_cancelled(&state.db, &id).await?;
    let actor_ip = client_ip_from_headers(&headers);
    warn_if_audit_failed(
        record_server_transfer_audit(
            &state.db,
            &auth,
            &actor_ip,
            "server.transfer_owner.cancel",
            "success",
            &id,
            &transfer.server_id,
            None,
        )
        .await,
    );
    let updated = load_server_owner_transfer(&state.db, &id)
        .await?
        .ok_or(AppError::NotFound("server transfer not found".into()))?;
    Ok(Json(ApiResponse::success(updated)))
}

pub async fn list_server_groups(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(q): Query<ListQuery>,
) -> Result<Json<ApiResponse<ServerGroupListResponse>>, AppError> {
    if !has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);
    let (mut groups, total) = load_server_groups(&state.db, &auth, limit, offset).await?;
    for group in &mut groups {
        group.server_ids = filter_visible_server_ids(&auth, std::mem::take(&mut group.server_ids));
    }
    Ok(Json(ApiResponse::success(ServerGroupListResponse {
        groups,
        total,
    })))
}

pub async fn create_server_group(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<CreateServerGroupRequest>,
) -> Result<Json<ApiResponse<ServerGroupView>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    let name = normalize_group_name(&req.name)?;
    let color = normalize_optional_label(req.color);
    let id = Uuid::now_v7();
    let now = Utc::now();
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO server_groups (id, owner_user_id, name, color, display_order, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(auth.user_id.0.to_string())
            .bind(&name)
            .bind(&color)
            .bind(req.display_order)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO server_groups (id, owner_user_id, name, color, display_order, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(id)
            .bind(auth.user_id.0)
            .bind(&name)
            .bind(&color)
            .bind(req.display_order.map(|value| value as i32))
            .bind(now)
            .bind(now)
            .execute(pool)
            .await?;
        }
    }
    let group = load_server_group(&state.db, &auth, &id.to_string()).await?;
    Ok(Json(ApiResponse::success(group)))
}

pub async fn update_server_group(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Json(req): Json<UpdateServerGroupRequest>,
) -> Result<Json<ApiResponse<ServerGroupView>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    let existing = load_server_group(&state.db, &auth, &id).await?;
    let name = match req.name {
        Some(value) => normalize_group_name(&value)?,
        None => existing.name,
    };
    let color = normalize_optional_label(req.color).or(existing.color);
    let display_order = req.display_order.or(existing.display_order);
    let now = Utc::now();
    let affected = match &state.db {
        DatabaseBackend::Sqlite(pool) => sqlx::query(
            "UPDATE server_groups SET name = ?, color = ?, display_order = ?, updated_at = ? WHERE id = ? AND owner_user_id = ?",
        )
        .bind(&name)
        .bind(&color)
        .bind(display_order)
        .bind(now.to_rfc3339())
        .bind(&id)
        .bind(auth.user_id.0.to_string())
        .execute(pool)
        .await?
        .rows_affected(),
        DatabaseBackend::Postgres(pool) => sqlx::query(
            "UPDATE server_groups SET name = $1, color = $2, display_order = $3, updated_at = $4 WHERE id = $5 AND owner_user_id = $6",
        )
        .bind(&name)
        .bind(&color)
        .bind(display_order.map(|value| value as i32))
        .bind(now)
        .bind(parse_uuid(&id)?)
        .bind(auth.user_id.0)
        .execute(pool)
        .await?
        .rows_affected(),
    };
    if affected == 0 {
        return Err(AppError::NotFound("server group not found".into()));
    }
    let group = load_server_group(&state.db, &auth, &id).await?;
    Ok(Json(ApiResponse::success(group)))
}

pub async fn delete_server_group(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    let affected = match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query("DELETE FROM server_groups WHERE id = ? AND owner_user_id = ?")
                .bind(&id)
                .bind(auth.user_id.0.to_string())
                .execute(pool)
                .await?
                .rows_affected()
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query("DELETE FROM server_groups WHERE id = $1 AND owner_user_id = $2")
                .bind(parse_uuid(&id)?)
                .bind(auth.user_id.0)
                .execute(pool)
                .await?
                .rows_affected()
        }
    };
    if affected == 0 {
        return Err(AppError::NotFound("server group not found".into()));
    }
    Ok(Json(ApiResponse::success(
        serde_json::json!({ "id": id, "deleted": true }),
    )))
}

pub async fn add_server_group_members(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Json(req): Json<AddServerGroupMembersRequest>,
) -> Result<Json<ApiResponse<ServerGroupView>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    if req.server_ids.is_empty() {
        return Err(AppError::BadRequest("server_ids is required".into()));
    }
    if req.server_ids.len() > 200 {
        return Err(AppError::BadRequest(
            "server_ids must contain at most 200 items".into(),
        ));
    }
    load_server_group(&state.db, &auth, &id).await?;
    let server_ids = dedupe_ids(req.server_ids);
    for server_id in &server_ids {
        ensure_group_server_access(&state.db, &auth, server_id).await?;
    }
    let now = Utc::now();
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            for server_id in server_ids {
                sqlx::query(
                    "INSERT INTO server_group_members (group_id, agent_id, created_at) VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
                )
                .bind(&id)
                .bind(&server_id)
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
            }
        }
        DatabaseBackend::Postgres(pool) => {
            let group_id = parse_uuid(&id)?;
            for server_id in server_ids {
                sqlx::query(
                    "INSERT INTO server_group_members (group_id, agent_id, created_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                )
                .bind(group_id)
                .bind(parse_uuid(&server_id)?)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }
    }
    let group = load_server_group(&state.db, &auth, &id).await?;
    Ok(Json(ApiResponse::success(group)))
}

pub async fn delete_server_group_member(
    State(state): State<AppState>,
    auth: AuthSession,
    Path((id, server_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<ServerGroupView>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    load_server_group(&state.db, &auth, &id).await?;
    if !server_visible(&auth, &parse_agent_id(&server_id)?) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query("DELETE FROM server_group_members WHERE group_id = ? AND agent_id = ?")
                .bind(&id)
                .bind(&server_id)
                .execute(pool)
                .await?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query("DELETE FROM server_group_members WHERE group_id = $1 AND agent_id = $2")
                .bind(parse_uuid(&id)?)
                .bind(parse_uuid(&server_id)?)
                .execute(pool)
                .await?;
        }
    }
    let group = load_server_group(&state.db, &auth, &id).await?;
    Ok(Json(ApiResponse::success(group)))
}

#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "1d".to_string()
}

#[derive(Debug, Serialize)]
pub struct MetricsResponse {
    pub agent_id: String,
    pub range: String,
    pub series: MetricSeries,
}

pub async fn get_server_metrics(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Query(q): Query<MetricsQuery>,
) -> Result<Json<ApiResponse<MetricsResponse>>, AppError> {
    if !has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    let agent_id = parse_agent_id(&id)?;
    if !server_visible(&auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let range = QueryRange::parse(&q.range).ok_or(AppError::BadRequest(format!(
        "unsupported range: {}",
        q.range
    )))?;
    let series = state
        .metrics
        .query(xlstatus_tsdb::AgentId(agent_id.0), range)?;
    Ok(Json(ApiResponse::success(MetricsResponse {
        agent_id: agent_id.0.to_string(),
        range: range.as_str().to_string(),
        series,
    })))
}

fn parse_agent_id(id: &str) -> Result<AgentId, AppError> {
    let parsed = uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::BadRequest(format!("invalid agent id: {id}")))?;
    Ok(AgentId(parsed))
}

fn parse_user_id(id: &str) -> Result<UserId, AppError> {
    let parsed = uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::BadRequest(format!("invalid user id: {id}")))?;
    Ok(UserId(parsed))
}

fn parse_uuid(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::BadRequest(format!("invalid uuid: {id}")))
}

async fn load_server_groups(
    db: &DatabaseBackend,
    auth: &AuthSession,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ServerGroupView>, i64), AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let total: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM server_groups WHERE owner_user_id = ?")
                    .bind(auth.user_id.0.to_string())
                    .fetch_one(pool)
                    .await?;
            let rows = sqlx::query(
                r#"
                SELECT id, owner_user_id, name, color, display_order, created_at, updated_at
                FROM server_groups
                WHERE owner_user_id = ?
                ORDER BY COALESCE(display_order, 999999), created_at ASC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(auth.user_id.0.to_string())
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?;
            let mut groups = Vec::with_capacity(rows.len());
            for row in rows {
                let mut group = server_group_from_sqlite_row(row)?;
                group.server_ids = load_server_group_members(db, &group.id).await?;
                groups.push(group);
            }
            Ok((groups, total.0))
        }
        DatabaseBackend::Postgres(pool) => {
            let total: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM server_groups WHERE owner_user_id = $1")
                    .bind(auth.user_id.0)
                    .fetch_one(pool)
                    .await?;
            let rows = sqlx::query(
                r#"
                SELECT id::text AS id, owner_user_id::text AS owner_user_id, name, color,
                       display_order::bigint AS display_order, created_at::text AS created_at,
                       updated_at::text AS updated_at
                FROM server_groups
                WHERE owner_user_id = $1
                ORDER BY COALESCE(display_order, 999999), created_at ASC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(auth.user_id.0)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?;
            let mut groups = Vec::with_capacity(rows.len());
            for row in rows {
                let mut group = server_group_from_pg_row(row)?;
                group.server_ids = load_server_group_members(db, &group.id).await?;
                groups.push(group);
            }
            Ok((groups, total.0))
        }
    }
}

async fn load_server_group(
    db: &DatabaseBackend,
    auth: &AuthSession,
    id: &str,
) -> Result<ServerGroupView, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query(
                r#"
                SELECT id, owner_user_id, name, color, display_order, created_at, updated_at
                FROM server_groups
                WHERE id = ? AND owner_user_id = ?
                "#,
            )
            .bind(id)
            .bind(auth.user_id.0.to_string())
            .fetch_optional(pool)
            .await?;
            let mut group = row
                .map(server_group_from_sqlite_row)
                .transpose()?
                .ok_or(AppError::NotFound("server group not found".into()))?;
            group.server_ids = load_server_group_members(db, &group.id).await?;
            group.server_ids = filter_visible_server_ids(auth, group.server_ids);
            Ok(group)
        }
        DatabaseBackend::Postgres(pool) => {
            let row = sqlx::query(
                r#"
                SELECT id::text AS id, owner_user_id::text AS owner_user_id, name, color,
                       display_order::bigint AS display_order, created_at::text AS created_at,
                       updated_at::text AS updated_at
                FROM server_groups
                WHERE id = $1 AND owner_user_id = $2
                "#,
            )
            .bind(parse_uuid(id)?)
            .bind(auth.user_id.0)
            .fetch_optional(pool)
            .await?;
            let mut group = row
                .map(server_group_from_pg_row)
                .transpose()?
                .ok_or(AppError::NotFound("server group not found".into()))?;
            group.server_ids = load_server_group_members(db, &group.id).await?;
            group.server_ids = filter_visible_server_ids(auth, group.server_ids);
            Ok(group)
        }
    }
}

async fn load_server_group_members(
    db: &DatabaseBackend,
    group_id: &str,
) -> Result<Vec<String>, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT agent_id FROM server_group_members WHERE group_id = ? ORDER BY created_at ASC",
            )
            .bind(group_id)
            .fetch_all(pool)
            .await?;
            Ok(rows.into_iter().map(|(id,)| id).collect())
        }
        DatabaseBackend::Postgres(pool) => {
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT agent_id::text FROM server_group_members WHERE group_id = $1 ORDER BY created_at ASC",
            )
            .bind(parse_uuid(group_id)?)
            .fetch_all(pool)
            .await?;
            Ok(rows.into_iter().map(|(id,)| id).collect())
        }
    }
}

fn server_group_from_sqlite_row(row: sqlx::sqlite::SqliteRow) -> Result<ServerGroupView, AppError> {
    Ok(ServerGroupView {
        id: row.try_get("id").map_err(db_err)?,
        owner_user_id: row.try_get("owner_user_id").map_err(db_err)?,
        name: row.try_get("name").map_err(db_err)?,
        color: row.try_get("color").map_err(db_err)?,
        display_order: row.try_get("display_order").map_err(db_err)?,
        server_ids: Vec::new(),
        created_at: row.try_get("created_at").map_err(db_err)?,
        updated_at: row.try_get("updated_at").map_err(db_err)?,
    })
}

fn server_group_from_pg_row(row: sqlx::postgres::PgRow) -> Result<ServerGroupView, AppError> {
    Ok(ServerGroupView {
        id: row.try_get("id").map_err(db_err)?,
        owner_user_id: row.try_get("owner_user_id").map_err(db_err)?,
        name: row.try_get("name").map_err(db_err)?,
        color: row.try_get("color").map_err(db_err)?,
        display_order: row.try_get("display_order").map_err(db_err)?,
        server_ids: Vec::new(),
        created_at: row.try_get("created_at").map_err(db_err)?,
        updated_at: row.try_get("updated_at").map_err(db_err)?,
    })
}

fn filter_visible_server_ids(auth: &AuthSession, server_ids: Vec<String>) -> Vec<String> {
    server_ids
        .into_iter()
        .filter(|id| {
            Uuid::parse_str(id)
                .map(|uuid| server_visible(auth, &AgentId(uuid)))
                .unwrap_or(false)
        })
        .collect()
}

async fn ensure_group_server_access(
    db: &DatabaseBackend,
    auth: &AuthSession,
    server_id: &str,
) -> Result<(), AppError> {
    let agent_id = parse_agent_id(server_id)?;
    if !server_visible(auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let agent = AgentRepository::new(db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    if !auth.role.is_admin() && agent.owner_user_id != auth.user_id {
        return Err(AppError::Forbidden("agent is owned by another user".into()));
    }
    Ok(())
}

fn normalize_group_name(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest("group name is required".into()));
    }
    if value.len() > 80 {
        return Err(AppError::BadRequest("group name is too long".into()));
    }
    Ok(value.to_string())
}

async fn apply_batch_action(
    db: &DatabaseBackend,
    agent_repo: &AgentRepository,
    auth: &AuthSession,
    actor_ip: &str,
    id: &str,
    action: ServerBatchAction,
    tags: &[String],
    dashboard_visible: Option<bool>,
    target_owner: Option<UserId>,
    target_group_id: Option<&str>,
) -> Result<(), String> {
    let agent_id = parse_agent_id(id).map_err(error_message)?;
    if !server_visible(auth, &agent_id) {
        return Err("agent not in scope".into());
    }
    let row = agent_repo
        .find_by_id_with_state(agent_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "agent not found".to_string())?;

    match action {
        ServerBatchAction::Delete => {
            if !auth.role.is_admin() && row.agent.owner_user_id != auth.user_id {
                return Err("agent is owned by another user".into());
            }
            delete_agent(db, agent_id).await.map_err(error_message)
        }
        ServerBatchAction::MoveGroup => {
            if !auth.role.is_admin() && row.agent.owner_user_id != auth.user_id {
                return Err("agent is owned by another user".into());
            }
            let target_group_id =
                target_group_id.ok_or_else(|| "group_id is required".to_string())?;
            move_agent_to_server_group(db, auth, agent_id, target_group_id)
                .await
                .map_err(error_message)
        }
        ServerBatchAction::TransferOwner => {
            let target_owner =
                target_owner.ok_or_else(|| "owner_user_id is required".to_string())?;
            let transfer_id = Uuid::now_v7().to_string();
            match perform_owner_transfer_new(
                db,
                &transfer_id,
                agent_id,
                row.agent.owner_user_id,
                target_owner,
                auth,
            )
            .await
            {
                Ok(()) => {
                    warn_if_audit_failed(
                        record_server_transfer_audit(
                            db,
                            auth,
                            actor_ip,
                            "server.transfer_owner",
                            "success",
                            &transfer_id,
                            id,
                            None,
                        )
                        .await,
                    );
                    Ok(())
                }
                Err(err) => {
                    let message = error_message(err);
                    warn_if_audit_failed(
                        record_failed_server_owner_transfer(
                            db,
                            &transfer_id,
                            agent_id,
                            row.agent.owner_user_id,
                            target_owner,
                            auth,
                            &message,
                        )
                        .await,
                    );
                    warn_if_audit_failed(
                        record_server_transfer_audit(
                            db,
                            auth,
                            actor_ip,
                            "server.transfer_owner",
                            "failure",
                            &transfer_id,
                            id,
                            Some(&message),
                        )
                        .await,
                    );
                    Err(message)
                }
            }
        }
        ServerBatchAction::SetTags
        | ServerBatchAction::AddTags
        | ServerBatchAction::RemoveTags
        | ServerBatchAction::SetDashboardVisible => {
            let parsed_info = row
                .last_info_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
            let parsed_state = row
                .last_state_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
            let mut metadata = dashboard_metadata(
                row.dashboard_metadata_json.as_deref(),
                &[parsed_info.as_ref(), parsed_state.as_ref()],
            );
            match action {
                ServerBatchAction::SetTags => metadata.tags = normalize_tags(tags.to_vec()),
                ServerBatchAction::AddTags => {
                    let mut next = metadata.tags;
                    next.extend(tags.iter().cloned());
                    metadata.tags = normalize_tags(next);
                }
                ServerBatchAction::RemoveTags => {
                    metadata
                        .tags
                        .retain(|tag| !tags.iter().any(|item| item == tag));
                }
                ServerBatchAction::SetDashboardVisible => {
                    metadata.dashboard_visible = dashboard_visible;
                }
                ServerBatchAction::TransferOwner
                | ServerBatchAction::Delete
                | ServerBatchAction::MoveGroup => unreachable!(),
            }
            let metadata_json = serde_json::to_string(&metadata).map_err(|e| e.to_string())?;
            update_agent_dashboard_metadata(db, agent_id, &metadata_json)
                .await
                .map_err(error_message)
        }
    }
}

async fn update_agent_dashboard_metadata(
    db: &DatabaseBackend,
    id: AgentId,
    dashboard_metadata_json: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    let affected = match db {
        DatabaseBackend::Sqlite(pool) => sqlx::query(
            "UPDATE agents SET dashboard_metadata_json = ?, updated_at = ? WHERE id = ?",
        )
        .bind(dashboard_metadata_json)
        .bind(now.to_rfc3339())
        .bind(id.0.to_string())
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected(),
        DatabaseBackend::Postgres(pool) => sqlx::query(
            "UPDATE agents SET dashboard_metadata_json = $1, updated_at = $2 WHERE id = $3",
        )
        .bind(dashboard_metadata_json)
        .bind(now)
        .bind(id.0)
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected(),
    };
    if affected == 0 {
        return Err(AppError::NotFound("agent not found".into()));
    }
    Ok(())
}

fn require_transfer_admin(auth: &AuthSession) -> Result<(), AppError> {
    if !auth.role.is_admin() {
        return Err(AppError::Forbidden(
            "admin role required for ownership transfer".into(),
        ));
    }
    Ok(())
}

fn ensure_transfer_server_scope(auth: &AuthSession, server_id: &str) -> Result<(), AppError> {
    let agent_id = parse_agent_id(server_id)?;
    if !server_visible(auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    Ok(())
}

async fn perform_owner_transfer_new(
    db: &DatabaseBackend,
    transfer_id: &str,
    agent_id: AgentId,
    from_owner: UserId,
    to_owner: UserId,
    auth: &AuthSession,
) -> Result<(), AppError> {
    persist_owner_transfer(db, Some(transfer_id), agent_id, from_owner, to_owner, auth).await
}

async fn perform_owner_transfer_existing(
    db: &DatabaseBackend,
    transfer_id: &str,
    agent_id: AgentId,
    from_owner: UserId,
    to_owner: UserId,
    auth: &AuthSession,
) -> Result<(), AppError> {
    persist_owner_transfer(db, Some(transfer_id), agent_id, from_owner, to_owner, auth).await
}

async fn persist_owner_transfer(
    db: &DatabaseBackend,
    transfer_id: Option<&str>,
    agent_id: AgentId,
    from_owner: UserId,
    to_owner: UserId,
    auth: &AuthSession,
) -> Result<(), AppError> {
    let transfer_id = transfer_id.unwrap_or("");
    let now = Utc::now();
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let now_text = now.to_rfc3339();
            let mut tx = pool.begin().await.map_err(db_err)?;
            let affected = sqlx::query(
                "UPDATE agents SET owner_user_id = ?, updated_at = ? WHERE id = ? AND owner_user_id = ?",
            )
            .bind(to_owner.0.to_string())
            .bind(&now_text)
            .bind(agent_id.0.to_string())
            .bind(from_owner.0.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db_err)?
            .rows_affected();
            if affected == 0 {
                return Err(AppError::BadRequest(
                    "agent owner changed before transfer could complete".into(),
                ));
            }
            sqlx::query(
                "DELETE FROM server_group_members WHERE agent_id = ? AND group_id IN (SELECT id FROM server_groups WHERE owner_user_id != ?)",
            )
            .bind(agent_id.0.to_string())
            .bind(to_owner.0.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
            if transfer_id.is_empty() {
                return Err(AppError::BadRequest("transfer id is required".into()));
            }
            let existing = sqlx::query("SELECT id FROM server_owner_transfers WHERE id = ?")
                .bind(transfer_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_err)?;
            if existing.is_some() {
                sqlx::query(
                    r#"
                    UPDATE server_owner_transfers
                    SET from_user_id = ?, to_user_id = ?, requested_by_user_id = ?,
                        api_token_id = ?, status = 'completed', attempts = attempts + 1,
                        error = NULL, completed_at = ?, cancelled_at = NULL,
                        last_attempt_at = ?, updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(from_owner.0.to_string())
                .bind(to_owner.0.to_string())
                .bind(auth.user_id.0.to_string())
                .bind(auth.pat_id.as_deref())
                .bind(&now_text)
                .bind(&now_text)
                .bind(&now_text)
                .bind(transfer_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            } else {
                sqlx::query(
                    r#"
                    INSERT INTO server_owner_transfers (
                        id, agent_id, from_user_id, to_user_id, requested_by_user_id,
                        api_token_id, status, attempts, error, completed_at, cancelled_at,
                        last_attempt_at, created_at, updated_at
                    ) VALUES (?, ?, ?, ?, ?, ?, 'completed', 1, NULL, ?, NULL, ?, ?, ?)
                    "#,
                )
                .bind(transfer_id)
                .bind(agent_id.0.to_string())
                .bind(from_owner.0.to_string())
                .bind(to_owner.0.to_string())
                .bind(auth.user_id.0.to_string())
                .bind(auth.pat_id.as_deref())
                .bind(&now_text)
                .bind(&now_text)
                .bind(&now_text)
                .bind(&now_text)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            }
            tx.commit().await.map_err(db_err)?;
        }
        DatabaseBackend::Postgres(pool) => {
            let mut tx = pool.begin().await.map_err(db_err)?;
            let affected = sqlx::query(
                "UPDATE agents SET owner_user_id = $1, updated_at = $2 WHERE id = $3 AND owner_user_id = $4",
            )
            .bind(to_owner.0)
            .bind(now)
            .bind(agent_id.0)
            .bind(from_owner.0)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?
            .rows_affected();
            if affected == 0 {
                return Err(AppError::BadRequest(
                    "agent owner changed before transfer could complete".into(),
                ));
            }
            sqlx::query(
                "DELETE FROM server_group_members WHERE agent_id = $1 AND group_id IN (SELECT id FROM server_groups WHERE owner_user_id != $2)",
            )
            .bind(agent_id.0)
            .bind(to_owner.0)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
            if transfer_id.is_empty() {
                return Err(AppError::BadRequest("transfer id is required".into()));
            }
            let api_token_id = auth
                .pat_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok());
            let existing = sqlx::query("SELECT id FROM server_owner_transfers WHERE id = $1")
                .bind(transfer_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_err)?;
            if existing.is_some() {
                sqlx::query(
                    r#"
                    UPDATE server_owner_transfers
                    SET from_user_id = $1, to_user_id = $2, requested_by_user_id = $3,
                        api_token_id = $4, status = 'completed', attempts = attempts + 1,
                        error = NULL, completed_at = $5, cancelled_at = NULL,
                        last_attempt_at = $6, updated_at = $7
                    WHERE id = $8
                    "#,
                )
                .bind(from_owner.0)
                .bind(to_owner.0)
                .bind(auth.user_id.0)
                .bind(api_token_id)
                .bind(now)
                .bind(now)
                .bind(now)
                .bind(transfer_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            } else {
                sqlx::query(
                    r#"
                    INSERT INTO server_owner_transfers (
                        id, agent_id, from_user_id, to_user_id, requested_by_user_id,
                        api_token_id, status, attempts, error, completed_at, cancelled_at,
                        last_attempt_at, created_at, updated_at
                    ) VALUES ($1, $2, $3, $4, $5, $6, 'completed', 1, NULL, $7, NULL, $8, $9, $10)
                    "#,
                )
                .bind(transfer_id)
                .bind(agent_id.0)
                .bind(from_owner.0)
                .bind(to_owner.0)
                .bind(auth.user_id.0)
                .bind(api_token_id)
                .bind(now)
                .bind(now)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            }
            tx.commit().await.map_err(db_err)?;
        }
    }
    Ok(())
}

async fn record_failed_server_owner_transfer(
    db: &DatabaseBackend,
    transfer_id: &str,
    agent_id: AgentId,
    from_owner: UserId,
    to_owner: UserId,
    auth: &AuthSession,
    error: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let now_text = now.to_rfc3339();
            sqlx::query(
                r#"
                INSERT INTO server_owner_transfers (
                    id, agent_id, from_user_id, to_user_id, requested_by_user_id,
                    api_token_id, status, attempts, error, completed_at, cancelled_at,
                    last_attempt_at, created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, 'failed', 1, ?, NULL, NULL, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    status = 'failed',
                    attempts = server_owner_transfers.attempts + 1,
                    error = excluded.error,
                    last_attempt_at = excluded.last_attempt_at,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(transfer_id)
            .bind(agent_id.0.to_string())
            .bind(from_owner.0.to_string())
            .bind(to_owner.0.to_string())
            .bind(auth.user_id.0.to_string())
            .bind(auth.pat_id.as_deref())
            .bind(error)
            .bind(&now_text)
            .bind(&now_text)
            .bind(&now_text)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        DatabaseBackend::Postgres(pool) => {
            let api_token_id = auth
                .pat_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok());
            sqlx::query(
                r#"
                INSERT INTO server_owner_transfers (
                    id, agent_id, from_user_id, to_user_id, requested_by_user_id,
                    api_token_id, status, attempts, error, completed_at, cancelled_at,
                    last_attempt_at, created_at, updated_at
                ) VALUES ($1, $2, $3, $4, $5, $6, 'failed', 1, $7, NULL, NULL, $8, $9, $10)
                ON CONFLICT(id) DO UPDATE SET
                    status = 'failed',
                    attempts = server_owner_transfers.attempts + 1,
                    error = excluded.error,
                    last_attempt_at = excluded.last_attempt_at,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(transfer_id)
            .bind(agent_id.0)
            .bind(from_owner.0)
            .bind(to_owner.0)
            .bind(auth.user_id.0)
            .bind(api_token_id)
            .bind(error)
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
    }
    Ok(())
}

async fn mark_server_owner_transfer_completed(
    db: &DatabaseBackend,
    transfer_id: &str,
    from_owner: UserId,
    auth: &AuthSession,
    increment_attempts: bool,
) -> Result<(), AppError> {
    let now = Utc::now();
    let attempts_sql = if increment_attempts {
        "attempts = attempts + 1,"
    } else {
        ""
    };
    let affected = match db {
        DatabaseBackend::Sqlite(pool) => {
            let now_text = now.to_rfc3339();
            let sql = format!(
                "UPDATE server_owner_transfers SET from_user_id = ?, requested_by_user_id = ?, api_token_id = ?, status = 'completed', {attempts_sql} error = NULL, completed_at = ?, cancelled_at = NULL, last_attempt_at = ?, updated_at = ? WHERE id = ?"
            );
            sqlx::query(&sql)
                .bind(from_owner.0.to_string())
                .bind(auth.user_id.0.to_string())
                .bind(auth.pat_id.as_deref())
                .bind(&now_text)
                .bind(&now_text)
                .bind(&now_text)
                .bind(transfer_id)
                .execute(pool)
                .await
                .map_err(db_err)?
                .rows_affected()
        }
        DatabaseBackend::Postgres(pool) => {
            let api_token_id = auth
                .pat_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok());
            let sql = format!(
                "UPDATE server_owner_transfers SET from_user_id = $1, requested_by_user_id = $2, api_token_id = $3, status = 'completed', {attempts_sql} error = NULL, completed_at = $4, cancelled_at = NULL, last_attempt_at = $5, updated_at = $6 WHERE id = $7"
            );
            sqlx::query(&sql)
                .bind(from_owner.0)
                .bind(auth.user_id.0)
                .bind(api_token_id)
                .bind(now)
                .bind(now)
                .bind(now)
                .bind(transfer_id)
                .execute(pool)
                .await
                .map_err(db_err)?
                .rows_affected()
        }
    };
    if affected == 0 {
        return Err(AppError::NotFound("server transfer not found".into()));
    }
    Ok(())
}

async fn mark_server_owner_transfer_failed(
    db: &DatabaseBackend,
    transfer_id: &str,
    error: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    let affected = match db {
        DatabaseBackend::Sqlite(pool) => {
            let now_text = now.to_rfc3339();
            sqlx::query(
                "UPDATE server_owner_transfers SET status = 'failed', attempts = attempts + 1, error = ?, last_attempt_at = ?, updated_at = ? WHERE id = ?",
            )
            .bind(error)
            .bind(&now_text)
            .bind(&now_text)
            .bind(transfer_id)
            .execute(pool)
            .await
            .map_err(db_err)?
            .rows_affected()
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                "UPDATE server_owner_transfers SET status = 'failed', attempts = attempts + 1, error = $1, last_attempt_at = $2, updated_at = $3 WHERE id = $4",
            )
            .bind(error)
            .bind(now)
            .bind(now)
            .bind(transfer_id)
            .execute(pool)
            .await
            .map_err(db_err)?
            .rows_affected()
        }
    };
    if affected == 0 {
        return Err(AppError::NotFound("server transfer not found".into()));
    }
    Ok(())
}

async fn mark_server_owner_transfer_cancelled(
    db: &DatabaseBackend,
    transfer_id: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    let affected = match db {
        DatabaseBackend::Sqlite(pool) => {
            let now_text = now.to_rfc3339();
            sqlx::query(
                "UPDATE server_owner_transfers SET status = 'cancelled', error = 'cancelled by user', cancelled_at = ?, updated_at = ? WHERE id = ? AND status != 'completed'",
            )
            .bind(&now_text)
            .bind(&now_text)
            .bind(transfer_id)
            .execute(pool)
            .await
            .map_err(db_err)?
            .rows_affected()
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                "UPDATE server_owner_transfers SET status = 'cancelled', error = 'cancelled by user', cancelled_at = $1, updated_at = $2 WHERE id = $3 AND status != 'completed'",
            )
            .bind(now)
            .bind(now)
            .bind(transfer_id)
            .execute(pool)
            .await
            .map_err(db_err)?
            .rows_affected()
        }
    };
    if affected == 0 {
        return Err(AppError::NotFound("server transfer not found".into()));
    }
    Ok(())
}

async fn load_server_owner_transfers(
    db: &DatabaseBackend,
    auth: &AuthSession,
    server_id: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ServerOwnerTransferView>, i64), AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            if matches!(auth.server_ids.as_ref(), Some(ids) if ids.is_empty()) {
                return Ok((Vec::new(), 0));
            }
            let mut count = QueryBuilder::<sqlx::Sqlite>::new(
                "SELECT COUNT(*) AS count FROM server_owner_transfers",
            );
            push_sqlite_transfer_filters(&mut count, auth, server_id);
            let total: i64 = count
                .build()
                .fetch_one(pool)
                .await
                .map_err(db_err)?
                .try_get("count")
                .map_err(db_err)?;

            let mut query = QueryBuilder::<sqlx::Sqlite>::new(
                r#"
                SELECT id, agent_id, from_user_id, to_user_id, requested_by_user_id,
                       api_token_id, status, attempts, error, completed_at, cancelled_at,
                       last_attempt_at, created_at, updated_at
                FROM server_owner_transfers
                "#,
            );
            push_sqlite_transfer_filters(&mut query, auth, server_id);
            query
                .push(" ORDER BY created_at DESC LIMIT ")
                .push_bind(limit)
                .push(" OFFSET ")
                .push_bind(offset);
            let rows = query.build().fetch_all(pool).await.map_err(db_err)?;
            let transfers = rows
                .into_iter()
                .map(server_transfer_from_sqlite_row)
                .collect::<Result<Vec<_>, _>>()?;
            Ok((transfers, total))
        }
        DatabaseBackend::Postgres(pool) => {
            if matches!(auth.server_ids.as_ref(), Some(ids) if ids.is_empty()) {
                return Ok((Vec::new(), 0));
            }
            let mut count = QueryBuilder::<sqlx::Postgres>::new(
                "SELECT COUNT(*) AS count FROM server_owner_transfers",
            );
            push_pg_transfer_filters(&mut count, auth, server_id)?;
            let total: i64 = count
                .build()
                .fetch_one(pool)
                .await
                .map_err(db_err)?
                .try_get("count")
                .map_err(db_err)?;

            let mut query = QueryBuilder::<sqlx::Postgres>::new(
                r#"
                SELECT id, agent_id::text AS agent_id, from_user_id::text AS from_user_id,
                       to_user_id::text AS to_user_id, requested_by_user_id::text AS requested_by_user_id,
                       api_token_id::text AS api_token_id, status, attempts::bigint AS attempts,
                       error, completed_at::text AS completed_at, cancelled_at::text AS cancelled_at,
                       last_attempt_at::text AS last_attempt_at, created_at::text AS created_at,
                       updated_at::text AS updated_at
                FROM server_owner_transfers
                "#,
            );
            push_pg_transfer_filters(&mut query, auth, server_id)?;
            query
                .push(" ORDER BY created_at DESC LIMIT ")
                .push_bind(limit)
                .push(" OFFSET ")
                .push_bind(offset);
            let rows = query.build().fetch_all(pool).await.map_err(db_err)?;
            let transfers = rows
                .into_iter()
                .map(server_transfer_from_pg_row)
                .collect::<Result<Vec<_>, _>>()?;
            Ok((transfers, total))
        }
    }
}

async fn load_server_owner_transfer(
    db: &DatabaseBackend,
    transfer_id: &str,
) -> Result<Option<ServerOwnerTransferView>, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query(
                r#"
                SELECT id, agent_id, from_user_id, to_user_id, requested_by_user_id,
                       api_token_id, status, attempts, error, completed_at, cancelled_at,
                       last_attempt_at, created_at, updated_at
                FROM server_owner_transfers
                WHERE id = ?
                "#,
            )
            .bind(transfer_id)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.map(server_transfer_from_sqlite_row).transpose()
        }
        DatabaseBackend::Postgres(pool) => {
            let row = sqlx::query(
                r#"
                SELECT id, agent_id::text AS agent_id, from_user_id::text AS from_user_id,
                       to_user_id::text AS to_user_id, requested_by_user_id::text AS requested_by_user_id,
                       api_token_id::text AS api_token_id, status, attempts::bigint AS attempts,
                       error, completed_at::text AS completed_at, cancelled_at::text AS cancelled_at,
                       last_attempt_at::text AS last_attempt_at, created_at::text AS created_at,
                       updated_at::text AS updated_at
                FROM server_owner_transfers
                WHERE id = $1
                "#,
            )
            .bind(transfer_id)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.map(server_transfer_from_pg_row).transpose()
        }
    }
}

fn push_sqlite_transfer_filters<'a>(
    builder: &mut QueryBuilder<'a, sqlx::Sqlite>,
    auth: &'a AuthSession,
    server_id: Option<&'a str>,
) {
    let mut has_where = false;
    if let Some(server_id) = server_id {
        builder.push(" WHERE agent_id = ").push_bind(server_id);
        has_where = true;
    }
    if let Some(allow) = auth.server_ids.as_ref() {
        builder.push(if has_where { " AND " } else { " WHERE " });
        builder.push("agent_id IN (");
        let mut separated = builder.separated(", ");
        for id in allow {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
}

fn push_pg_transfer_filters<'a>(
    builder: &mut QueryBuilder<'a, sqlx::Postgres>,
    auth: &'a AuthSession,
    server_id: Option<&'a str>,
) -> Result<(), AppError> {
    let mut has_where = false;
    if let Some(server_id) = server_id {
        builder
            .push(" WHERE agent_id = ")
            .push_bind(parse_uuid(server_id)?);
        has_where = true;
    }
    if let Some(allow) = auth.server_ids.as_ref() {
        builder.push(if has_where { " AND " } else { " WHERE " });
        builder.push("agent_id IN (");
        let mut separated = builder.separated(", ");
        for id in allow {
            separated.push_bind(parse_uuid(id)?);
        }
        separated.push_unseparated(")");
    }
    Ok(())
}

fn server_transfer_from_sqlite_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<ServerOwnerTransferView, AppError> {
    Ok(ServerOwnerTransferView {
        id: row.try_get("id").map_err(db_err)?,
        server_id: row.try_get("agent_id").map_err(db_err)?,
        from_user_id: row.try_get("from_user_id").map_err(db_err)?,
        to_user_id: row.try_get("to_user_id").map_err(db_err)?,
        requested_by_user_id: row.try_get("requested_by_user_id").map_err(db_err)?,
        api_token_id: row.try_get("api_token_id").map_err(db_err)?,
        status: row.try_get("status").map_err(db_err)?,
        attempts: row.try_get("attempts").map_err(db_err)?,
        error: row.try_get("error").map_err(db_err)?,
        completed_at: row.try_get("completed_at").map_err(db_err)?,
        cancelled_at: row.try_get("cancelled_at").map_err(db_err)?,
        last_attempt_at: row.try_get("last_attempt_at").map_err(db_err)?,
        created_at: row.try_get("created_at").map_err(db_err)?,
        updated_at: row.try_get("updated_at").map_err(db_err)?,
    })
}

fn server_transfer_from_pg_row(
    row: sqlx::postgres::PgRow,
) -> Result<ServerOwnerTransferView, AppError> {
    Ok(ServerOwnerTransferView {
        id: row.try_get("id").map_err(db_err)?,
        server_id: row.try_get("agent_id").map_err(db_err)?,
        from_user_id: row.try_get("from_user_id").map_err(db_err)?,
        to_user_id: row.try_get("to_user_id").map_err(db_err)?,
        requested_by_user_id: row.try_get("requested_by_user_id").map_err(db_err)?,
        api_token_id: row.try_get("api_token_id").map_err(db_err)?,
        status: row.try_get("status").map_err(db_err)?,
        attempts: row.try_get("attempts").map_err(db_err)?,
        error: row.try_get("error").map_err(db_err)?,
        completed_at: row.try_get("completed_at").map_err(db_err)?,
        cancelled_at: row.try_get("cancelled_at").map_err(db_err)?,
        last_attempt_at: row.try_get("last_attempt_at").map_err(db_err)?,
        created_at: row.try_get("created_at").map_err(db_err)?,
        updated_at: row.try_get("updated_at").map_err(db_err)?,
    })
}

async fn record_server_transfer_audit(
    db: &DatabaseBackend,
    auth: &AuthSession,
    ip: &str,
    action: &str,
    outcome: &str,
    transfer_id: &str,
    server_id: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    let metadata = serde_json::json!({
        "transfer_id": transfer_id,
        "server_id": server_id,
        "error": error,
    });
    let metadata_json =
        serde_json::to_string(&metadata).map_err(|e| AppError::BadRequest(e.to_string()))?;
    match db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                r#"
                INSERT INTO audit_logs (
                    id, user_id, api_token_id, action, resource_type, resource_id,
                    server_id, ip, outcome, metadata_json, sensitive_hash, created_at
                ) VALUES (?, ?, ?, ?, 'server_owner_transfer', ?, ?, ?, ?, ?, NULL, ?)
                "#,
            )
            .bind(&id)
            .bind(auth.user_id.0.to_string())
            .bind(auth.pat_id.as_deref())
            .bind(action)
            .bind(transfer_id)
            .bind(server_id)
            .bind(ip)
            .bind(outcome)
            .bind(&metadata_json)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        DatabaseBackend::Postgres(pool) => {
            let api_token_id = auth
                .pat_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok());
            sqlx::query(
                r#"
                INSERT INTO audit_logs (
                    id, user_id, api_token_id, action, resource_type, resource_id,
                    server_id, ip, outcome, metadata_json, sensitive_hash, created_at
                ) VALUES ($1, $2, $3, $4, 'server_owner_transfer', $5, $6, $7, $8, $9, NULL, $10)
                "#,
            )
            .bind(&id)
            .bind(auth.user_id.0)
            .bind(api_token_id)
            .bind(action)
            .bind(transfer_id)
            .bind(server_id)
            .bind(ip)
            .bind(outcome)
            .bind(&metadata_json)
            .bind(now)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
    }
    Ok(())
}

fn warn_if_audit_failed(result: Result<(), AppError>) {
    if let Err(err) = result {
        tracing::warn!("failed to write server transfer audit record: {:?}", err);
    }
}

async fn delete_agent(db: &DatabaseBackend, id: AgentId) -> Result<(), AppError> {
    let affected = match db {
        DatabaseBackend::Sqlite(pool) => sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(id.0.to_string())
            .execute(pool)
            .await
            .map_err(db_err)?
            .rows_affected(),
        DatabaseBackend::Postgres(pool) => sqlx::query("DELETE FROM agents WHERE id = $1")
            .bind(id.0)
            .execute(pool)
            .await
            .map_err(db_err)?
            .rows_affected(),
    };
    if affected == 0 {
        return Err(AppError::NotFound("agent not found".into()));
    }
    Ok(())
}

async fn move_agent_to_server_group(
    db: &DatabaseBackend,
    auth: &AuthSession,
    agent_id: AgentId,
    group_id: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    match db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "DELETE FROM server_group_members WHERE agent_id = ? AND group_id IN (SELECT id FROM server_groups WHERE owner_user_id = ?)",
            )
            .bind(agent_id.0.to_string())
            .bind(auth.user_id.0.to_string())
            .execute(pool)
            .await
            .map_err(db_err)?;
            sqlx::query(
                "INSERT INTO server_group_members (group_id, agent_id, created_at) VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
            )
            .bind(group_id)
            .bind(agent_id.0.to_string())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        DatabaseBackend::Postgres(pool) => {
            let group_id = parse_uuid(group_id)?;
            sqlx::query(
                "DELETE FROM server_group_members WHERE agent_id = $1 AND group_id IN (SELECT id FROM server_groups WHERE owner_user_id = $2)",
            )
            .bind(agent_id.0)
            .bind(auth.user_id.0)
            .execute(pool)
            .await
            .map_err(db_err)?;
            sqlx::query(
                "INSERT INTO server_group_members (group_id, agent_id, created_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(group_id)
            .bind(agent_id.0)
            .bind(now)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
    }
    Ok(())
}

fn dedupe_ids(ids: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        let id = id.trim();
        if id.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        out.push(id.to_string());
    }
    out
}

fn error_message(error: AppError) -> String {
    match error {
        AppError::Database(_) => "database error".to_string(),
        AppError::Unauthorized(message)
        | AppError::Forbidden(message)
        | AppError::BadRequest(message)
        | AppError::NotFound(message) => message,
    }
}

fn db_err(err: sqlx::Error) -> AppError {
    AppError::Database(anyhow::anyhow!(err))
}

fn client_ip_from_headers(headers: &HeaderMap) -> String {
    header_value(headers, "x-forwarded-for")
        .and_then(|value| {
            value
                .split(',')
                .next()
                .map(str::trim)
                .map(ToString::to_string)
        })
        .filter(|value| !value.is_empty())
        .or_else(|| header_value(headers, "x-real-ip"))
        .unwrap_or_else(|| "unknown".to_string())
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// Helper used by tests and the public docs: build a ServerView from
/// (agent, parsed_state). Pulled out of the route so the offline/online
/// decision is unit-testable in isolation.
pub fn build_server_view(
    agent_id: AgentId,
    name: String,
    last_seen_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    last_state: Option<&serde_json::Value>,
) -> ServerView {
    let now = Utc::now();
    let age = last_seen_at
        .map(|ts| (now - ts).num_seconds())
        .unwrap_or(i64::MAX);
    let status = if revoked_at.is_some() {
        "revoked"
    } else if age <= ONLINE_THRESHOLD_SECS {
        "online"
    } else {
        "offline"
    };
    ServerView {
        id: agent_id.0.to_string(),
        name,
        remark: None,
        public_note: None,
        expires_at: None,
        renewal_price: None,
        price: None,
        currency: None,
        billing_cycle: None,
        auto_renew: None,
        traffic_quota_bytes: None,
        traffic_quota_type: None,
        provider: None,
        region: None,
        plan: None,
        tags: Vec::new(),
        accent_color: None,
        dashboard_visible: None,
        hide_for_guest: None,
        display_order: None,
        status: status.to_string(),
        last_seen_at: last_seen_at.map(|t| t.to_rfc3339()),
        cpu_percent: last_state
            .and_then(|v| v.get("cpu_percent"))
            .and_then(|v| v.as_f64()),
        memory_used: last_state
            .and_then(|v| v.get("memory_used"))
            .and_then(|v| v.as_i64()),
        memory_total: last_state
            .and_then(|v| v.get("memory_total"))
            .and_then(|v| v.as_i64()),
        load_1: last_state
            .and_then(|v| v.get("load_1"))
            .and_then(|v| v.as_f64()),
        net_rx_bps: last_state
            .and_then(|v| json_i64_by_keys(v, &["net_rx_bps", "network_in_speed"])),
        net_tx_bps: last_state
            .and_then(|v| json_i64_by_keys(v, &["net_tx_bps", "network_out_speed"])),
        network_in_total: last_state.and_then(|v| network_total(v, "bytes_recv")),
        network_out_total: last_state.and_then(|v| network_total(v, "bytes_sent")),
        uptime_seconds: last_state.and_then(|v| json_i64_by_keys(v, &["uptime_seconds", "uptime"])),
    }
}

fn network_rates_from_store(
    metrics: &xlstatus_tsdb::MetricStore,
    agent_id: Uuid,
) -> (Option<i64>, Option<i64>) {
    let Ok(series) = metrics.query(xlstatus_tsdb::AgentId(agent_id), QueryRange::Day1) else {
        return (None, None);
    };

    (
        network_rate_from_series(&series, "bytes_recv"),
        network_rate_from_series(&series, "bytes_sent"),
    )
}

fn network_rate_from_series(series: &MetricSeries, field: &str) -> Option<i64> {
    let mut latest: Option<(&DateTime<Utc>, i64)> = None;
    for sample in series.samples.iter().rev() {
        let Some(total) = network_total(&sample.fields_json, field) else {
            continue;
        };
        if let Some((latest_at, latest_total)) = latest {
            let elapsed_ms = (*latest_at - sample.sample_at).num_milliseconds();
            let delta = latest_total.checked_sub(total)?;
            if elapsed_ms > 0 && delta >= 0 {
                return Some(((delta as f64) / (elapsed_ms as f64 / 1000.0)).round() as i64);
            }
        }
        latest = Some((&sample.sample_at, total));
    }
    None
}

fn network_total(value: &serde_json::Value, field: &str) -> Option<i64> {
    let direct_keys = match field {
        "bytes_recv" => &["network_in_total", "net_rx_bytes", "bytes_recv_total"][..],
        "bytes_sent" => &["network_out_total", "net_tx_bytes", "bytes_sent_total"][..],
        _ => &[][..],
    };
    if let Some(value) = json_i64_by_keys(value, direct_keys) {
        return Some(value);
    }

    let net_io = value
        .get("net_io")
        .or_else(|| value.get("network_interfaces"))
        .and_then(|v| v.as_array())?;
    let mut total = 0_i64;
    let mut found = false;
    for item in net_io {
        if let Some(value) = item.get(field).and_then(json_i64) {
            total = total.saturating_add(value);
            found = true;
        }
    }
    found.then_some(total)
}

fn json_i64_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(json_i64) {
            return Some(value);
        }
    }
    None
}

fn json_i64(value: &serde_json::Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    if let Some(value) = value.as_u64() {
        return i64::try_from(value).ok();
    }
    if let Some(value) = value.as_f64() {
        if value.is_finite() && value >= 0.0 && value <= i64::MAX as f64 {
            return Some(value.round() as i64);
        }
    }
    None
}

fn metadata_string(sources: &[Option<&serde_json::Value>], keys: &[&str]) -> Option<String> {
    for source in sources.iter().flatten() {
        if let Some(value) = metadata_string_from_value(source, keys) {
            return Some(value);
        }
    }
    None
}

fn metadata_string_from_value(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(json_label) {
            return Some(value);
        }
    }

    for container in ["billing", "plan", "metadata", "custom", "traffic", "limits"] {
        if let Some(child) = value.get(container) {
            for key in keys {
                if let Some(value) = child.get(*key).and_then(json_label) {
                    return Some(value);
                }
            }
        }
    }

    None
}

fn metadata_i64(sources: &[Option<&serde_json::Value>], keys: &[&str]) -> Option<i64> {
    for source in sources.iter().flatten() {
        if let Some(value) = metadata_i64_from_value(source, keys) {
            return Some(value);
        }
    }
    None
}

fn metadata_i64_from_value(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(json_i64) {
            return Some(value);
        }
    }

    for container in ["billing", "plan", "metadata", "custom", "traffic", "limits"] {
        if let Some(child) = value.get(container) {
            for key in keys {
                if let Some(value) = child.get(*key).and_then(json_i64) {
                    return Some(value);
                }
            }
        }
    }

    None
}

fn json_label(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn normalize_optional_label(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn dashboard_metadata(
    stored: Option<&str>,
    fallback_sources: &[Option<&serde_json::Value>],
) -> DashboardMetadata {
    let mut out = stored
        .and_then(|value| serde_json::from_str::<DashboardMetadata>(value).ok())
        .unwrap_or_default();

    if out.public_note.is_none() {
        out.public_note = metadata_string(fallback_sources, &["public_note", "public_description"]);
    }
    if out.provider.is_none() {
        out.provider = metadata_string(
            fallback_sources,
            &["provider", "vendor", "datacenter", "isp"],
        );
    }
    if out.region.is_none() {
        out.region = metadata_string(fallback_sources, &["region", "location", "country", "city"]);
    }
    if out.plan.is_none() {
        out.plan = metadata_string(
            fallback_sources,
            &["plan", "package", "sku", "product", "instance_type"],
        );
    }
    if out.price.is_none() {
        out.price = metadata_string(
            fallback_sources,
            &["price", "billing_price", "monthly_price", "amount"],
        );
    }
    if out.currency.is_none() {
        out.currency = metadata_string(fallback_sources, &["currency", "currency_code"]);
    }
    if out.billing_cycle.is_none() {
        out.billing_cycle = metadata_string(
            fallback_sources,
            &["billing_cycle", "cycle", "billing_period", "period"],
        );
    }
    if out.traffic_quota_bytes.is_none() {
        out.traffic_quota_bytes = metadata_i64(
            fallback_sources,
            &[
                "traffic_quota_bytes",
                "traffic_quota",
                "quota_bytes",
                "bandwidth_quota_bytes",
                "monthly_traffic_bytes",
            ],
        );
    }
    if out.traffic_quota_type.is_none() {
        out.traffic_quota_type = metadata_string(
            fallback_sources,
            &[
                "traffic_quota_type",
                "quota_type",
                "traffic_type",
                "bandwidth_type",
            ],
        );
    }
    if out.accent_color.is_none() {
        out.accent_color = metadata_string(fallback_sources, &["accent_color", "color"])
            .and_then(|value| normalize_accent_color(Some(value)).ok())
            .flatten();
    }
    if out.tags.is_empty() {
        out.tags = metadata_tags(fallback_sources);
    }

    out.tags = normalize_tags(out.tags);
    out
}

fn metadata_tags(sources: &[Option<&serde_json::Value>]) -> Vec<String> {
    for source in sources.iter().flatten() {
        for key in ["tags", "labels"] {
            if let Some(value) = source.get(key) {
                let tags = tags_from_json(value);
                if !tags.is_empty() {
                    return tags;
                }
            }
        }
        for container in ["metadata", "custom"] {
            if let Some(child) = source.get(container) {
                for key in ["tags", "labels"] {
                    if let Some(value) = child.get(key) {
                        let tags = tags_from_json(value);
                        if !tags.is_empty() {
                            return tags;
                        }
                    }
                }
            }
        }
    }
    Vec::new()
}

fn tags_from_json(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => normalize_tags(
            items
                .iter()
                .filter_map(|item| json_label(item))
                .collect::<Vec<_>>(),
        ),
        serde_json::Value::String(value) => normalize_tags(
            value
                .split([',', ';', '，', '、'])
                .map(str::to_string)
                .collect::<Vec<_>>(),
        ),
        _ => Vec::new(),
    }
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for tag in tags {
        let cleaned = tag.trim();
        if cleaned.is_empty() || out.iter().any(|existing| existing == cleaned) {
            continue;
        }
        out.push(cleaned.chars().take(24).collect());
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn normalize_accent_color(value: Option<String>) -> Result<Option<String>, AppError> {
    let Some(value) = normalize_optional_label(value) else {
        return Ok(None);
    };
    let is_hex = value.len() == 7
        && value.starts_with('#')
        && value.chars().skip(1).all(|ch| ch.is_ascii_hexdigit());
    if is_hex {
        return Ok(Some(value));
    }
    Err(AppError::BadRequest(
        "accent_color must be a hex color like #db2777".into(),
    ))
}

/// Per-agent visibility check that respects both admin (always visible)
/// and PAT (must be in the PAT's `server_ids` allowlist). Lives next
/// to the route so the API contract is local and unit-testable.
pub fn server_visible(auth: &AuthSession, agent_id: &AgentId) -> bool {
    match &auth.server_ids {
        None => true,
        Some(allow) => {
            let id_str = agent_id.0.to_string();
            allow.iter().any(|a| a == &id_str)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use xlstatus_shared::AgentId;

    #[test]
    fn online_when_fresh() {
        let view = build_server_view(
            AgentId(uuid::Uuid::from_bytes([1; 16])),
            "n1".into(),
            Some(Utc::now()),
            None,
            None,
        );
        assert_eq!(view.status, "online");
    }

    #[test]
    fn offline_when_stale() {
        let view = build_server_view(
            AgentId(uuid::Uuid::from_bytes([2; 16])),
            "n2".into(),
            Some(Utc::now() - Duration::seconds(120)),
            None,
            None,
        );
        assert_eq!(view.status, "offline");
    }

    #[test]
    fn revoked_overrides_anything() {
        let view = build_server_view(
            AgentId(uuid::Uuid::from_bytes([3; 16])),
            "n3".into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
        );
        assert_eq!(view.status, "revoked");
    }

    #[test]
    fn extracts_metrics_from_state() {
        let state = json!({
            "cpu_percent": 42.0,
            "memory_used": 1000,
            "memory_total": 2000,
            "load_1": 0.5,
            "network_in_total": 3000,
            "network_out_total": 4000,
            "uptime_seconds": 500,
        });
        let view = build_server_view(
            AgentId(uuid::Uuid::from_bytes([4; 16])),
            "n4".into(),
            Some(Utc::now()),
            None,
            Some(&state),
        );
        assert_eq!(view.cpu_percent, Some(42.0));
        assert_eq!(view.memory_used, Some(1000));
        assert_eq!(view.memory_total, Some(2000));
        assert_eq!(view.load_1, Some(0.5));
        assert_eq!(view.network_in_total, Some(3000));
        assert_eq!(view.network_out_total, Some(4000));
        assert_eq!(view.uptime_seconds, Some(500));
    }

    #[test]
    fn extracts_network_totals_from_interfaces() {
        let state = json!({
            "net_io": [
                { "interface": "eth0", "bytes_recv": 1000, "bytes_sent": 2000 },
                { "interface": "eth1", "bytes_recv": 3000, "bytes_sent": 4000 }
            ]
        });

        assert_eq!(network_total(&state, "bytes_recv"), Some(4000));
        assert_eq!(network_total(&state, "bytes_sent"), Some(6000));
    }

    #[test]
    fn derives_network_rate_from_metric_samples() {
        let agent_id = xlstatus_tsdb::AgentId(uuid::Uuid::from_bytes([5; 16]));
        let series = MetricSeries {
            agent_id: agent_id.clone(),
            samples: vec![
                xlstatus_tsdb::MetricSample {
                    agent_id: agent_id.clone(),
                    sample_at: Utc::now() - Duration::seconds(10),
                    fields_json: json!({ "network_in_total": 1000 }),
                },
                xlstatus_tsdb::MetricSample {
                    agent_id,
                    sample_at: Utc::now(),
                    fields_json: json!({ "network_in_total": 6000 }),
                },
            ],
        };

        assert_eq!(network_rate_from_series(&series, "bytes_recv"), Some(500));
    }
}
