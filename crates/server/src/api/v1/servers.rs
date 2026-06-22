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
use crate::api::v1::geoip::{lookup_agent_geo_location, AgentGeoLocation};
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::auth::rbac::has_scope;
use crate::db::{Agent, AgentRepository, DatabaseBackend, UserRepository};

pub use crate::realtime::ws::ws_servers;
use axum::{
    extract::{connect_info::ConnectInfo, DefaultBodyLimit, Path, Query, State},
    http::HeaderMap,
    Json,
};
#[cfg(test)]
use chrono::Duration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row};
use std::collections::HashSet;
use std::net::SocketAddr;
use uuid::Uuid;
use xlstatus_shared::{AgentId, UserId};
use xlstatus_tsdb::{MetricSeries, QueryRange};

const ONLINE_THRESHOLD_SECS: i64 = 30;
const SERVER_MANAGEMENT_API_MAX_BODY_BYTES: usize = 64 * 1024;
const SERVER_NAME_MAX_BYTES: usize = 128;
const SERVER_LABEL_MAX_BYTES: usize = 512;
const SERVER_DASHBOARD_METADATA_MAX_BYTES: usize = 16 * 1024;
const SERVER_TAG_INPUT_MAX_ITEMS: usize = 64;
const SERVER_TAG_INPUT_MAX_BYTES: usize = 128;
const SERVER_BATCH_MAX_SERVER_IDS: usize = 200;
const SERVER_UUID_TEXT_LEN: usize = 36;

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

pub fn server_management_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(SERVER_MANAGEMENT_API_MAX_BODY_BYTES)
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
    pub country: Option<String>,
    pub city: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub location: Option<ServerLocationView>,
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

#[derive(Debug, Clone, Serialize)]
pub struct ServerLocationView {
    pub source: String,
    pub provider: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: Option<String>,
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
    let (rows, total) = if let Some(server_ids) = auth.server_ids.as_deref() {
        let owner_filter = (!auth.role.is_admin()).then_some(auth.user_id);
        agent_repo
            .list_with_state_by_server_ids(owner_filter, server_ids, limit, offset)
            .await?
    } else if auth.role.is_admin() {
        agent_repo.list_with_state(limit, offset).await?
    } else {
        agent_repo
            .list_with_state_by_owner(auth.user_id, limit, offset)
            .await?
    };
    let now = Utc::now();
    let mut servers = Vec::with_capacity(rows.len());
    for row in rows.into_iter() {
        if !agent_visible(&auth, &row.agent) {
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
        let location = server_location_view(
            &dashboard,
            lookup_agent_geo_location(&state.db, agent.id).await,
        );
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
            country: location.as_ref().and_then(|item| item.country.clone()),
            city: location.as_ref().and_then(|item| item.city.clone()),
            latitude: location.as_ref().and_then(|item| item.latitude),
            longitude: location.as_ref().and_then(|item| item.longitude),
            location,
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
    pub country: Option<String>,
    pub city: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub location: Option<ServerLocationView>,
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
    let agent_repo = AgentRepository::new(state.db.clone());
    let row = agent_repo
        .find_by_id_with_state(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".to_string()))?;
    ensure_agent_visible(&auth, &row.agent)?;
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
    let location = server_location_view(
        &dashboard,
        lookup_agent_geo_location(&state.db, agent.id).await,
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
        country: location.as_ref().and_then(|item| item.country.clone()),
        city: location.as_ref().and_then(|item| item.city.clone()),
        latitude: location.as_ref().and_then(|item| item.latitude),
        longitude: location.as_ref().and_then(|item| item.longitude),
        location,
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
    pub country: Option<String>,
    pub city: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
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
    country: Option<String>,
    city: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
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
    let agent = AgentRepository::new(state.db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    ensure_agent_visible(&auth, &agent)?;
    let name = req
        .name
        .as_deref()
        .map(|value| normalize_server_name(value, "name"))
        .transpose()?;

    let remark = normalize_optional_label(req.remark, "remark")?;
    let expires_at = normalize_optional_label(req.expires_at, "expires_at")?;
    let renewal_price = normalize_optional_label(req.renewal_price, "renewal_price")?;
    if matches!(req.traffic_quota_bytes, Some(value) if value < 0) {
        return Err(AppError::BadRequest(
            "traffic_quota_bytes must be greater than or equal to 0".into(),
        ));
    }
    let dashboard_metadata = DashboardMetadata {
        public_note: normalize_optional_label(req.public_note, "public_note")?,
        provider: normalize_optional_label(req.provider, "provider")?,
        region: normalize_optional_label(req.region, "region")?,
        country: normalize_optional_label(req.country, "country")?,
        city: normalize_optional_label(req.city, "city")?,
        latitude: normalize_optional_coordinate(req.latitude, "latitude", -90.0, 90.0)?,
        longitude: normalize_optional_coordinate(req.longitude, "longitude", -180.0, 180.0)?,
        plan: normalize_optional_label(req.plan, "plan")?,
        price: normalize_optional_label(req.price, "price")?,
        currency: normalize_optional_label(req.currency, "currency")?,
        billing_cycle: normalize_optional_label(req.billing_cycle, "billing_cycle")?,
        auto_renew: req.auto_renew,
        traffic_quota_bytes: req.traffic_quota_bytes,
        traffic_quota_type: normalize_optional_label(req.traffic_quota_type, "traffic_quota_type")?,
        tags: normalize_tag_input(req.tags.unwrap_or_default())?,
        accent_color: normalize_accent_color(req.accent_color)?,
        dashboard_visible: req.dashboard_visible,
        hide_for_guest: req.hide_for_guest,
        display_order: normalize_display_order(req.display_order, "display_order")?,
    };
    let dashboard_metadata_json = dashboard_metadata_json(&dashboard_metadata)?;
    let agent_repo = AgentRepository::new(state.db.clone());
    let updated = agent_repo
        .update_dashboard_metadata(
            agent_id,
            name.as_deref(),
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
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
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
    let server_ids = dedupe_ids(req.server_ids, SERVER_BATCH_MAX_SERVER_IDS, "server_ids")?;
    if server_ids.is_empty() {
        return Err(AppError::BadRequest("server_ids is required".into()));
    }

    let target_owner = if matches!(req.action, ServerBatchAction::TransferOwner) {
        if !auth.role.is_admin() {
            return Err(AppError::Forbidden(
                "admin role required for ownership transfer".into(),
            ));
        }
        require_admin_cookie_session(&auth)?;
        let owner = req
            .owner_user_id
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("owner_user_id is required".into()))
            .and_then(|id| normalize_uuid_text(id, "owner_user_id"))
            .and_then(|id| parse_user_id(&id))?;
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
    let group_id = req
        .group_id
        .as_deref()
        .map(|id| normalize_uuid_text(id, "group_id"))
        .transpose()?;
    if let (ServerBatchAction::MoveGroup, Some(group_id)) = (req.action, group_id.as_deref()) {
        load_server_group(&state.db, &auth, group_id).await?;
    }

    let normalized_tags = normalize_tag_input(req.tags)?;
    if matches!(
        req.action,
        ServerBatchAction::SetTags | ServerBatchAction::AddTags | ServerBatchAction::RemoveTags
    ) && normalized_tags.is_empty()
    {
        return Err(AppError::BadRequest("tags is required".into()));
    }

    let agent_repo = AgentRepository::new(state.db.clone());
    let actor_ip = client_ip_from_headers(&headers, peer_addr);
    let mut results = Vec::new();
    for id in server_ids {
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
            group_id.as_deref(),
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
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
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

    let actor_ip = client_ip_from_headers(&headers, peer_addr);
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
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
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
            "completed transfer cannot be cancelled".into(),
        ));
    }
    ensure_transfer_server_scope(&auth, &transfer.server_id)?;
    mark_server_owner_transfer_cancelled(&state.db, &id).await?;
    let actor_ip = client_ip_from_headers(&headers, peer_addr);
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
        group.server_ids = filter_visible_server_ids(&auth, &group.server_ids);
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
    let color = normalize_optional_label(req.color, "color")?;
    let display_order = normalize_display_order(req.display_order, "display_order")?;
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
            .bind(display_order)
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
            .bind(display_order.map(|value| value as i32))
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
    let color = normalize_optional_label(req.color, "color")?.or(existing.color);
    let display_order = match req.display_order {
        Some(value) => normalize_display_order(Some(value), "display_order")?,
        None => existing.display_order,
    };
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
    load_server_group(&state.db, &auth, &id).await?;
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
    let server_ids = dedupe_ids(req.server_ids, SERVER_BATCH_MAX_SERVER_IDS, "server_ids")?;
    if server_ids.is_empty() {
        return Err(AppError::BadRequest("server_ids is required".into()));
    }
    load_server_group(&state.db, &auth, &id).await?;
    for server_id in &server_ids {
        ensure_group_server_active(&state.db, &auth, server_id).await?;
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
    ensure_group_server_access(&state.db, &auth, &server_id).await?;
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
    let agent = AgentRepository::new(state.db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    ensure_agent_visible(&auth, &agent)?;
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
    let id = require_server_uuid_text(id, "server_id")?;
    let parsed = uuid::Uuid::parse_str(&id).expect("canonical UUID must parse after validation");
    Ok(AgentId(parsed))
}

fn parse_user_id(id: &str) -> Result<UserId, AppError> {
    let id = require_server_uuid_text(id, "user_id")?;
    let parsed = uuid::Uuid::parse_str(&id).expect("canonical UUID must parse after validation");
    Ok(UserId(parsed))
}

fn parse_uuid(id: &str) -> Result<Uuid, AppError> {
    let id = require_server_uuid_text(id, "resource_id")?;
    Ok(Uuid::parse_str(&id).expect("canonical UUID must parse after validation"))
}

async fn load_server_groups(
    db: &DatabaseBackend,
    auth: &AuthSession,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ServerGroupView>, i64), AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let owner = auth.user_id.0.to_string();
            let mut count_query = QueryBuilder::<sqlx::Sqlite>::new(
                "SELECT COUNT(*) FROM server_groups sg WHERE sg.owner_user_id = ",
            );
            count_query.push_bind(&owner);
            if let Some(server_ids) = auth.server_ids.as_ref() {
                push_sqlite_server_group_allowlist_filter(&mut count_query, &owner, server_ids);
            }
            let (total,): (i64,) = count_query.build_query_as().fetch_one(pool).await?;

            let mut query = QueryBuilder::<sqlx::Sqlite>::new(
                r#"
                SELECT sg.id AS id, sg.owner_user_id AS owner_user_id, sg.name AS name,
                       sg.color AS color, sg.display_order AS display_order,
                       sg.created_at AS created_at, sg.updated_at AS updated_at
                FROM server_groups sg
                WHERE sg.owner_user_id =
                "#,
            );
            query.push_bind(&owner);
            if let Some(server_ids) = auth.server_ids.as_ref() {
                push_sqlite_server_group_allowlist_filter(&mut query, &owner, server_ids);
            }
            query
                .push(" ORDER BY COALESCE(sg.display_order, 999999), sg.created_at ASC LIMIT ")
                .push_bind(limit)
                .push(" OFFSET ")
                .push_bind(offset);
            let rows = query.build().fetch_all(pool).await?;
            let mut groups = Vec::with_capacity(rows.len());
            for row in rows {
                let mut group = server_group_from_sqlite_row(row)?;
                group.server_ids = load_server_group_members(db, &group.id, auth.user_id).await?;
                groups.push(group);
            }
            Ok((groups, total))
        }
        DatabaseBackend::Postgres(pool) => {
            let mut count_query = QueryBuilder::<sqlx::Postgres>::new(
                "SELECT COUNT(*) FROM server_groups sg WHERE sg.owner_user_id = ",
            );
            count_query.push_bind(auth.user_id.0);
            if let Some(server_ids) = auth.server_ids.as_ref() {
                push_pg_server_group_allowlist_filter(&mut count_query, auth.user_id, server_ids)?;
            }
            let (total,): (i64,) = count_query.build_query_as().fetch_one(pool).await?;

            let mut query = QueryBuilder::<sqlx::Postgres>::new(
                r#"
                SELECT sg.id::text AS id, sg.owner_user_id::text AS owner_user_id,
                       sg.name AS name, sg.color AS color,
                       sg.display_order::bigint AS display_order,
                       sg.created_at::text AS created_at, sg.updated_at::text AS updated_at
                FROM server_groups sg
                WHERE sg.owner_user_id =
                "#,
            );
            query.push_bind(auth.user_id.0);
            if let Some(server_ids) = auth.server_ids.as_ref() {
                push_pg_server_group_allowlist_filter(&mut query, auth.user_id, server_ids)?;
            }
            query
                .push(" ORDER BY COALESCE(sg.display_order, 999999), sg.created_at ASC LIMIT ")
                .push_bind(limit)
                .push(" OFFSET ")
                .push_bind(offset);
            let rows = query.build().fetch_all(pool).await?;
            let mut groups = Vec::with_capacity(rows.len());
            for row in rows {
                let mut group = server_group_from_pg_row(row)?;
                group.server_ids = load_server_group_members(db, &group.id, auth.user_id).await?;
                groups.push(group);
            }
            Ok((groups, total))
        }
    }
}

fn push_sqlite_server_group_allowlist_filter<'a>(
    builder: &mut QueryBuilder<'a, sqlx::Sqlite>,
    owner_user_id: &'a str,
    server_ids: &'a [String],
) {
    builder.push(
        r#"
        AND NOT EXISTS (
            SELECT 1
            FROM server_group_members sgm
            JOIN agents a ON a.id = sgm.agent_id
            WHERE sgm.group_id = sg.id
              AND a.owner_user_id =
        "#,
    );
    builder.push_bind(owner_user_id);
    if !server_ids.is_empty() {
        builder.push(" AND a.id NOT IN (");
        let mut separated = builder.separated(", ");
        for id in server_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    builder.push(")");
}

fn push_pg_server_group_allowlist_filter(
    builder: &mut QueryBuilder<'_, sqlx::Postgres>,
    owner_user_id: UserId,
    server_ids: &[String],
) -> Result<(), AppError> {
    builder.push(
        r#"
        AND NOT EXISTS (
            SELECT 1
            FROM server_group_members sgm
            JOIN agents a ON a.id = sgm.agent_id
            WHERE sgm.group_id = sg.id
              AND a.owner_user_id =
        "#,
    );
    builder.push_bind(owner_user_id.0);
    if !server_ids.is_empty() {
        builder.push(" AND a.id NOT IN (");
        let mut separated = builder.separated(", ");
        for id in server_ids {
            separated.push_bind(parse_uuid(id)?);
        }
        separated.push_unseparated(")");
    }
    builder.push(")");
    Ok(())
}

async fn load_server_group(
    db: &DatabaseBackend,
    auth: &AuthSession,
    id: &str,
) -> Result<ServerGroupView, AppError> {
    let id = require_server_uuid_text(id, "group_id")?;
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query(
                r#"
                SELECT id, owner_user_id, name, color, display_order, created_at, updated_at
                FROM server_groups
                WHERE id = ? AND owner_user_id = ?
                "#,
            )
            .bind(&id)
            .bind(auth.user_id.0.to_string())
            .fetch_optional(pool)
            .await?;
            let mut group = row
                .map(server_group_from_sqlite_row)
                .transpose()?
                .ok_or(AppError::NotFound("server group not found".into()))?;
            group.server_ids = load_server_group_members(db, &group.id, auth.user_id).await?;
            ensure_server_group_visible(auth, &group.server_ids)?;
            group.server_ids = filter_visible_server_ids(auth, &group.server_ids);
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
            .bind(parse_uuid(&id)?)
            .bind(auth.user_id.0)
            .fetch_optional(pool)
            .await?;
            let mut group = row
                .map(server_group_from_pg_row)
                .transpose()?
                .ok_or(AppError::NotFound("server group not found".into()))?;
            group.server_ids = load_server_group_members(db, &group.id, auth.user_id).await?;
            ensure_server_group_visible(auth, &group.server_ids)?;
            group.server_ids = filter_visible_server_ids(auth, &group.server_ids);
            Ok(group)
        }
    }
}

async fn load_server_group_members(
    db: &DatabaseBackend,
    group_id: &str,
    owner_user_id: UserId,
) -> Result<Vec<String>, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows: Vec<(String,)> = sqlx::query_as(
                r#"
                SELECT sgm.agent_id
                FROM server_group_members sgm
                JOIN agents a ON a.id = sgm.agent_id
                WHERE sgm.group_id = ? AND a.owner_user_id = ?
                ORDER BY sgm.created_at ASC
                "#,
            )
            .bind(group_id)
            .bind(owner_user_id.0.to_string())
            .fetch_all(pool)
            .await?;
            Ok(rows.into_iter().map(|(id,)| id).collect())
        }
        DatabaseBackend::Postgres(pool) => {
            let rows: Vec<(String,)> = sqlx::query_as(
                r#"
                SELECT sgm.agent_id::text
                FROM server_group_members sgm
                JOIN agents a ON a.id = sgm.agent_id
                WHERE sgm.group_id = $1 AND a.owner_user_id = $2
                ORDER BY sgm.created_at ASC
                "#,
            )
            .bind(parse_uuid(group_id)?)
            .bind(owner_user_id.0)
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

fn filter_visible_server_ids(auth: &AuthSession, server_ids: &[String]) -> Vec<String> {
    server_ids
        .iter()
        .filter(|id| {
            Uuid::parse_str(id)
                .map(|uuid| server_visible(auth, &AgentId(uuid)))
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

fn server_group_visible(auth: &AuthSession, server_ids: &[String]) -> bool {
    auth.server_ids.is_none()
        || server_ids.iter().all(|id| {
            Uuid::parse_str(id)
                .map(|uuid| server_visible(auth, &AgentId(uuid)))
                .unwrap_or(false)
        })
}

fn ensure_server_group_visible(auth: &AuthSession, server_ids: &[String]) -> Result<(), AppError> {
    if server_group_visible(auth, server_ids) {
        Ok(())
    } else {
        Err(AppError::Forbidden("server group not in scope".into()))
    }
}

async fn ensure_group_server_access(
    db: &DatabaseBackend,
    auth: &AuthSession,
    server_id: &str,
) -> Result<(), AppError> {
    let agent_id = parse_agent_id(server_id)?;
    let agent = AgentRepository::new(db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    ensure_agent_visible(auth, &agent)
}

async fn ensure_group_server_active(
    db: &DatabaseBackend,
    auth: &AuthSession,
    server_id: &str,
) -> Result<(), AppError> {
    let agent_id = parse_agent_id(server_id)?;
    let agent = AgentRepository::new(db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    ensure_agent_visible(auth, &agent)?;
    if agent.revoked_at.is_some() {
        return Err(AppError::BadRequest("agent has been revoked".into()));
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
    let row = agent_repo
        .find_by_id_with_state(agent_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "agent not found".to_string())?;
    ensure_agent_visible(auth, &row.agent).map_err(error_message)?;

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
            if row.agent.revoked_at.is_some() {
                return Err("agent has been revoked".into());
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
            let metadata_json = dashboard_metadata_json(&metadata).map_err(error_message)?;
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
    if matches!(auth.auth_kind, AuthKind::PersonalAccessToken) {
        return Err(AppError::Forbidden(
            "Cookie session required for ownership transfer".into(),
        ));
    }
    Ok(())
}

fn require_admin_cookie_session(auth: &AuthSession) -> Result<(), AppError> {
    if !auth.role.is_admin() {
        return Err(AppError::Forbidden("admin role required".into()));
    }
    if matches!(auth.auth_kind, AuthKind::PersonalAccessToken) {
        return Err(AppError::Forbidden("Cookie session required".into()));
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
    let transfer_id = require_server_uuid_text(transfer_id, "transfer_id")?;
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
            .bind(&transfer_id)
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
            .bind(&transfer_id)
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

fn dedupe_ids(ids: Vec<String>, max_items: usize, field: &str) -> Result<Vec<String>, AppError> {
    if ids.len() > max_items {
        return Err(AppError::BadRequest(format!(
            "{field} must contain at most {max_items} items"
        )));
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        let id = normalize_uuid_text(&id, field)?;
        if !seen.insert(id.clone()) {
            continue;
        }
        out.push(id);
    }
    Ok(out)
}

fn error_message(error: AppError) -> String {
    match error {
        AppError::Database(_) => "database error".to_string(),
        AppError::Unauthorized(message)
        | AppError::Forbidden(message)
        | AppError::BadRequest(message)
        | AppError::TooManyRequests(message)
        | AppError::NotFound(message) => message,
    }
}

fn db_err(err: sqlx::Error) -> AppError {
    AppError::Database(anyhow::anyhow!(err))
}

fn client_ip_from_headers(headers: &HeaderMap, peer_addr: SocketAddr) -> String {
    crate::security::client_ip_from_headers_and_peer(headers, Some(peer_addr))
}

/// Helper used by tests and the public docs: build a ServerView from
/// (agent, parsed_state). Pulled out of the route so the offline/online
/// decision is unit-testable in isolation.
#[cfg(test)]
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
        country: None,
        city: None,
        latitude: None,
        longitude: None,
        location: None,
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
fn metadata_f64(sources: &[Option<&serde_json::Value>], keys: &[&str]) -> Option<f64> {
    for source in sources.iter().flatten() {
        if let Some(value) = metadata_f64_from_value(source, keys) {
            return Some(value);
        }
    }
    None
}

fn metadata_f64_from_value(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(json_f64) {
            return Some(value);
        }
    }

    for container in ["geo", "location", "metadata", "custom", "network"] {
        if let Some(child) = value.get(container) {
            for key in keys {
                if let Some(value) = child.get(*key).and_then(json_f64) {
                    return Some(value);
                }
            }
        }
    }

    None
}

fn json_f64(value: &serde_json::Value) -> Option<f64> {
    let value = value.as_f64().or_else(|| value.as_str()?.parse().ok())?;
    value.is_finite().then_some(value)
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

fn normalize_server_name(value: &str, field: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if value.len() > SERVER_NAME_MAX_BYTES {
        return Err(AppError::BadRequest(format!("{field} is too long")));
    }
    Ok(value.to_string())
}

fn normalize_optional_label(
    value: Option<String>,
    field: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > SERVER_LABEL_MAX_BYTES {
        return Err(AppError::BadRequest(format!("{field} is too long")));
    }
    Ok(Some(value.to_string()))
}

fn normalize_uuid_text(value: &str, field: &str) -> Result<String, AppError> {
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    require_server_uuid_text(value, field)
}

fn require_server_uuid_text(value: &str, field: &str) -> Result<String, AppError> {
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if value.len() != SERVER_UUID_TEXT_LEN {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    let parsed = Uuid::parse_str(value)
        .map_err(|_| AppError::BadRequest(format!("{field} must be a canonical UUID")))?;
    if parsed.to_string() != value {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    Ok(value.to_string())
}

fn normalize_display_order(value: Option<i64>, field: &str) -> Result<Option<i64>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value < i32::MIN as i64 || value > i32::MAX as i64 {
        return Err(AppError::BadRequest(format!("{field} is out of range")));
    }
    Ok(Some(value))
}

fn dashboard_metadata_json(metadata: &DashboardMetadata) -> Result<String, AppError> {
    let value = serde_json::to_string(metadata).map_err(|e| AppError::BadRequest(e.to_string()))?;
    if value.len() > SERVER_DASHBOARD_METADATA_MAX_BYTES {
        return Err(AppError::BadRequest(
            "dashboard metadata is too large".into(),
        ));
    }
    Ok(value)
}

fn normalize_optional_coordinate(
    value: Option<f64>,
    field: &str,
    min: f64,
    max: f64,
) -> Result<Option<f64>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.is_finite() || value < min || value > max {
        return Err(AppError::BadRequest(format!("{field} is invalid")));
    }
    Ok(Some((value * 1_000_000.0).round() / 1_000_000.0))
}

fn server_location_view(
    dashboard: &DashboardMetadata,
    geoip: Option<AgentGeoLocation>,
) -> Option<ServerLocationView> {
    let manual_has_location = dashboard.country.is_some()
        || dashboard.region.is_some()
        || dashboard.city.is_some()
        || dashboard.latitude.is_some()
        || dashboard.longitude.is_some();
    if manual_has_location {
        return Some(ServerLocationView {
            source: "manual".into(),
            provider: None,
            country: dashboard.country.clone(),
            region: dashboard.region.clone(),
            city: dashboard.city.clone(),
            latitude: dashboard.latitude,
            longitude: dashboard.longitude,
            timezone: None,
        });
    }

    geoip.map(|location| ServerLocationView {
        source: location.source,
        provider: Some(location.provider),
        country: location.country,
        region: location.region,
        city: location.city,
        latitude: location.latitude,
        longitude: location.longitude,
        timezone: location.timezone,
    })
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
        out.region = metadata_string(
            fallback_sources,
            &["region", "geo_region", "state", "province", "location"],
        );
    }
    if out.country.is_none() {
        out.country = metadata_string(
            fallback_sources,
            &["country", "geo_country", "country_name"],
        );
    }
    if out.city.is_none() {
        out.city = metadata_string(fallback_sources, &["city", "geo_city"]);
    }
    if out.latitude.is_none() {
        out.latitude = metadata_f64(fallback_sources, &["latitude", "lat", "geo_latitude"]);
    }
    if out.longitude.is_none() {
        out.longitude = metadata_f64(
            fallback_sources,
            &["longitude", "lon", "lng", "geo_longitude"],
        );
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

fn normalize_tag_input(tags: Vec<String>) -> Result<Vec<String>, AppError> {
    if tags.len() > SERVER_TAG_INPUT_MAX_ITEMS {
        return Err(AppError::BadRequest(format!(
            "tags must contain at most {SERVER_TAG_INPUT_MAX_ITEMS} items"
        )));
    }
    for tag in &tags {
        let cleaned = tag.trim();
        if cleaned.len() > SERVER_TAG_INPUT_MAX_BYTES {
            return Err(AppError::BadRequest("tag is too long".into()));
        }
    }
    Ok(normalize_tags(tags))
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
    let Some(value) = normalize_optional_label(value, "accent_color")? else {
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

pub fn agent_visible(auth: &AuthSession, agent: &Agent) -> bool {
    server_visible(auth, &agent.id) && (auth.role.is_admin() || agent.owner_user_id == auth.user_id)
}

pub fn ensure_agent_visible(auth: &AuthSession, agent: &Agent) -> Result<(), AppError> {
    if agent_visible(auth, agent) {
        Ok(())
    } else {
        Err(AppError::Forbidden("agent not in scope".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;
    use xlstatus_shared::{AgentId, UserId, UserRole};

    fn auth_session(auth_kind: AuthKind, role: UserRole) -> AuthSession {
        AuthSession {
            session_id: "session".into(),
            user_id: UserId(uuid::Uuid::from_bytes([9; 16])),
            username: "admin".into(),
            role,
            csrf_token: "csrf".into(),
            auth_kind,
            scopes: vec!["server:read".into(), "server:write".into()],
            server_ids: None,
            pat_id: None,
        }
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

    async fn seed_user(db: &DatabaseBackend, id: Uuid, username: &str, role: &str) {
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

    async fn seed_totp_enabled_user(db: &DatabaseBackend, id: Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query("UPDATE users SET totp_secret = ?, totp_enabled = 1 WHERE id = ?")
            .bind("totp-secret")
            .bind(id.to_string())
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_agent(db: &DatabaseBackend, id: Uuid, owner: Uuid, name: &str, created_at: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at) VALUES (?, ?, 'pk', ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(owner.to_string())
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn revoke_agent(db: &DatabaseBackend, id: Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query("UPDATE agents SET revoked_at = '2026-06-22T00:00:00Z' WHERE id = ?")
            .bind(id.to_string())
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_server_group(db: &DatabaseBackend, id: Uuid, owner: Uuid, name: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO server_groups (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(owner.to_string())
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_dirty_server_group(db: &DatabaseBackend, id: &str, owner: Uuid, name: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO server_groups (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_dirty_server_owner_transfer(
        db: &DatabaseBackend,
        id: &str,
        server_id: Uuid,
        to_user_id: Uuid,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            r#"
            INSERT INTO server_owner_transfers (
                id, agent_id, to_user_id, status, attempts,
                last_attempt_at, created_at, updated_at
            ) VALUES (?, ?, ?, 'failed', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')
            "#,
        )
        .bind(id)
        .bind(server_id.to_string())
        .bind(to_user_id.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_server_owner_transfer(
        db: &DatabaseBackend,
        id: Uuid,
        server_id: Uuid,
        from_user_id: Uuid,
        to_user_id: Uuid,
        status: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            r#"
            INSERT INTO server_owner_transfers (
                id, agent_id, from_user_id, to_user_id, requested_by_user_id,
                status, attempts, last_attempt_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')
            "#,
        )
        .bind(id.to_string())
        .bind(server_id.to_string())
        .bind(from_user_id.to_string())
        .bind(to_user_id.to_string())
        .bind(from_user_id.to_string())
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_server_group_with_order(
        db: &DatabaseBackend,
        id: Uuid,
        owner: Uuid,
        name: &str,
        display_order: i64,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO server_groups (id, owner_user_id, name, display_order, created_at, updated_at) VALUES (?, ?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(owner.to_string())
        .bind(name)
        .bind(display_order)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_server_group_member(db: &DatabaseBackend, group_id: Uuid, server_id: Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO server_group_members (group_id, agent_id, created_at) VALUES (?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(group_id.to_string())
        .bind(server_id.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn admin_pat_server_list_respects_server_allowlist_total() {
        let db = test_db().await;
        let admin = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let allowed_server = Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();
        let blocked_server = Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();

        seed_user(&db, admin, "admin", "admin").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(
            &db,
            allowed_server,
            admin,
            "allowed",
            "2026-01-01T00:00:00Z",
        )
        .await;
        seed_agent(
            &db,
            blocked_server,
            other,
            "blocked",
            "2026-01-02T00:00:00Z",
        )
        .await;

        let mut auth = auth_session(AuthKind::PersonalAccessToken, UserRole::Admin);
        auth.user_id = UserId(admin);
        auth.server_ids = Some(vec![allowed_server.to_string()]);
        auth.pat_id = Some("pat".into());

        let Json(response) = list_servers(
            State(test_state(db)),
            auth,
            Query(ListQuery {
                limit: 50,
                offset: 0,
            }),
        )
        .await
        .unwrap();
        let data = response.data.unwrap();

        assert_eq!(data.total, 1);
        assert_eq!(data.servers.len(), 1);
        assert_eq!(data.servers[0].id, allowed_server.to_string());
    }

    #[tokio::test]
    async fn admin_pat_server_groups_respect_server_allowlist() {
        let db = test_db().await;
        let admin = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let allowed_server = Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();
        let blocked_server = Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();
        let foreign_server = Uuid::parse_str("00000000-0000-0000-0000-000000000203").unwrap();
        let allowed_group = Uuid::parse_str("00000000-0000-0000-0000-000000000301").unwrap();
        let blocked_group = Uuid::parse_str("00000000-0000-0000-0000-000000000302").unwrap();
        let foreign_dirty_group = Uuid::parse_str("00000000-0000-0000-0000-000000000303").unwrap();

        seed_user(&db, admin, "admin", "admin").await;
        seed_user(
            &db,
            Uuid::parse_str("00000000-0000-0000-0000-000000000009").unwrap(),
            "other",
            "member",
        )
        .await;
        seed_agent(
            &db,
            allowed_server,
            admin,
            "allowed",
            "2026-01-01T00:00:00Z",
        )
        .await;
        seed_agent(
            &db,
            blocked_server,
            admin,
            "blocked",
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_agent(
            &db,
            foreign_server,
            Uuid::parse_str("00000000-0000-0000-0000-000000000009").unwrap(),
            "foreign",
            "2026-01-03T00:00:00Z",
        )
        .await;
        seed_server_group_with_order(&db, blocked_group, admin, "blocked-group", 1).await;
        seed_server_group_with_order(&db, allowed_group, admin, "allowed-group", 2).await;
        seed_server_group_with_order(&db, foreign_dirty_group, admin, "foreign-dirty-group", 3)
            .await;
        seed_server_group_member(&db, allowed_group, allowed_server).await;
        seed_server_group_member(&db, blocked_group, blocked_server).await;
        seed_server_group_member(&db, foreign_dirty_group, foreign_server).await;

        let mut auth = auth_session(AuthKind::PersonalAccessToken, UserRole::Admin);
        auth.user_id = UserId(admin);
        auth.server_ids = Some(vec![allowed_server.to_string()]);
        auth.pat_id = Some("pat".into());

        let (groups, total) = load_server_groups(&db, &auth, 1, 0).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, allowed_group.to_string());

        let (groups, total) = load_server_groups(&db, &auth, 10, 0).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].server_ids, vec![allowed_server.to_string()]);
        assert!(groups[1].server_ids.is_empty());

        let err = load_server_group(&db, &auth, &blocked_group.to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));

        let update_err = update_server_group(
            State(test_state(db.clone())),
            auth.clone(),
            Path(blocked_group.to_string()),
            Json(UpdateServerGroupRequest {
                name: Some("renamed".into()),
                color: None,
                display_order: None,
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(update_err, AppError::Forbidden(_)));

        let delete_err =
            delete_server_group(State(test_state(db)), auth, Path(blocked_group.to_string()))
                .await
                .unwrap_err();
        assert!(matches!(delete_err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn server_group_members_ignore_cross_owner_dirty_members() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let owner_server = Uuid::parse_str("00000000-0000-0000-0000-000000000111").unwrap();
        let other_server = Uuid::parse_str("00000000-0000-0000-0000-000000000222").unwrap();
        let group_id = Uuid::parse_str("00000000-0000-0000-0000-000000000333").unwrap();

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(
            &db,
            owner_server,
            owner,
            "owner-server",
            "2026-01-01T00:00:00Z",
        )
        .await;
        seed_agent(
            &db,
            other_server,
            other,
            "other-server",
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_server_group(&db, group_id, owner, "owner-group").await;
        seed_server_group_member(&db, group_id, owner_server).await;
        seed_server_group_member(&db, group_id, other_server).await;

        let mut auth = auth_session(AuthKind::Session, UserRole::Member);
        auth.user_id = UserId(owner);

        let group = load_server_group(&db, &auth, &group_id.to_string())
            .await
            .unwrap();

        assert_eq!(group.server_ids, vec![owner_server.to_string()]);
    }

    #[tokio::test]
    async fn server_group_lookup_rejects_dirty_text_id_before_sql() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        seed_user(&db, owner, "owner", "member").await;
        seed_dirty_server_group(&db, "group-a", owner, "dirty-group").await;

        let mut auth = auth_session(AuthKind::Session, UserRole::Member);
        auth.user_id = UserId(owner);

        let err = load_server_group(&db, &auth, "group-a").await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn server_owner_transfer_lookup_rejects_dirty_text_id_before_sql() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let server_id = Uuid::parse_str("00000000-0000-0000-0000-000000000111").unwrap();
        seed_user(&db, owner, "owner", "member").await;
        seed_agent(
            &db,
            server_id,
            owner,
            "owner-server",
            "2026-01-01T00:00:00Z",
        )
        .await;
        seed_dirty_server_owner_transfer(&db, "transfer-a", server_id, owner).await;

        let err = load_server_owner_transfer(&db, "transfer-a")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn server_group_write_rejects_revoked_members_but_allows_cleanup() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let active_server = Uuid::parse_str("00000000-0000-0000-0000-000000000111").unwrap();
        let revoked_server = Uuid::parse_str("00000000-0000-0000-0000-000000000222").unwrap();
        let group_id = Uuid::parse_str("00000000-0000-0000-0000-000000000333").unwrap();

        seed_user(&db, owner, "owner", "member").await;
        seed_agent(
            &db,
            active_server,
            owner,
            "active-server",
            "2026-01-01T00:00:00Z",
        )
        .await;
        seed_agent(
            &db,
            revoked_server,
            owner,
            "revoked-server",
            "2026-01-02T00:00:00Z",
        )
        .await;
        revoke_agent(&db, revoked_server).await;
        seed_server_group(&db, group_id, owner, "owner-group").await;

        let mut auth = auth_session(AuthKind::Session, UserRole::Admin);
        auth.user_id = UserId(owner);

        let Json(response) = add_server_group_members(
            State(test_state(db.clone())),
            auth.clone(),
            Path(group_id.to_string()),
            Json(AddServerGroupMembersRequest {
                server_ids: vec![active_server.to_string()],
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            response.data.unwrap().server_ids,
            vec![active_server.to_string()]
        );

        let add_err = add_server_group_members(
            State(test_state(db.clone())),
            auth.clone(),
            Path(group_id.to_string()),
            Json(AddServerGroupMembersRequest {
                server_ids: vec![revoked_server.to_string()],
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(add_err, AppError::BadRequest(_)));

        let group = load_server_group(&db, &auth, &group_id.to_string())
            .await
            .unwrap();
        assert_eq!(group.server_ids, vec![active_server.to_string()]);

        let Json(batch_response) = batch_update_servers(
            State(test_state(db.clone())),
            auth.clone(),
            ConnectInfo("127.0.0.1:12345".parse().unwrap()),
            HeaderMap::new(),
            Json(BatchUpdateServersRequest {
                server_ids: vec![revoked_server.to_string()],
                action: ServerBatchAction::MoveGroup,
                tags: vec![],
                dashboard_visible: None,
                owner_user_id: None,
                group_id: Some(group_id.to_string()),
            }),
        )
        .await
        .unwrap();
        let data = batch_response.data.unwrap();
        assert_eq!(data.updated, 0);
        assert_eq!(data.failed, 1);
        assert!(data.results[0]
            .error
            .as_deref()
            .is_some_and(|message| message.contains("revoked")));

        seed_server_group_member(&db, group_id, revoked_server).await;
        let group = load_server_group(&db, &auth, &group_id.to_string())
            .await
            .unwrap();
        assert_eq!(group.server_ids.len(), 2);
        assert!(group.server_ids.contains(&active_server.to_string()));
        assert!(group.server_ids.contains(&revoked_server.to_string()));

        let Json(response) = delete_server_group_member(
            State(test_state(db)),
            auth,
            Path((group_id.to_string(), revoked_server.to_string())),
        )
        .await
        .unwrap();
        assert_eq!(
            response.data.unwrap().server_ids,
            vec![active_server.to_string()]
        );
    }

    #[test]
    fn ownership_transfer_rejects_admin_pat() {
        let auth = auth_session(AuthKind::PersonalAccessToken, UserRole::Admin);
        let err = require_transfer_admin(&auth).unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn ownership_transfer_allows_admin_cookie_session() {
        let auth = auth_session(AuthKind::Session, UserRole::Admin);
        assert!(require_transfer_admin(&auth).is_ok());
    }

    #[tokio::test]
    async fn cancel_ownership_transfer_requires_sensitive_totp_when_enabled() {
        let db = test_db().await;
        let admin = Uuid::from_bytes([9; 16]);
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();
        let target = Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();
        let server = Uuid::parse_str("00000000-0000-0000-0000-000000000303").unwrap();
        let transfer = Uuid::parse_str("00000000-0000-0000-0000-000000000404").unwrap();
        seed_user(&db, admin, "admin", "admin").await;
        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, target, "target", "member").await;
        seed_totp_enabled_user(&db, admin).await;
        seed_agent(&db, server, owner, "server", "2026-01-01T00:00:00Z").await;
        seed_server_owner_transfer(&db, transfer, server, owner, target, "failed").await;

        let auth = auth_session(AuthKind::Session, UserRole::Admin);
        let err = cancel_server_owner_transfer(
            State(test_state(db.clone())),
            auth,
            ConnectInfo("127.0.0.1:8080".parse().unwrap()),
            HeaderMap::new(),
            Path(transfer.to_string()),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
        let still_failed = load_server_owner_transfer(&db, &transfer.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still_failed.status, "failed");
        assert!(still_failed.cancelled_at.is_none());
    }

    #[test]
    fn server_management_resource_limits_are_explicit() {
        let _ = server_management_body_limit();
        assert_eq!(SERVER_MANAGEMENT_API_MAX_BODY_BYTES, 64 * 1024);
        assert_eq!(SERVER_NAME_MAX_BYTES, 128);
        assert_eq!(SERVER_LABEL_MAX_BYTES, 512);
        assert_eq!(SERVER_DASHBOARD_METADATA_MAX_BYTES, 16 * 1024);
        assert_eq!(SERVER_TAG_INPUT_MAX_ITEMS, 64);
        assert_eq!(SERVER_BATCH_MAX_SERVER_IDS, 200);
        assert_eq!(SERVER_UUID_TEXT_LEN, 36);
    }

    #[test]
    fn server_dashboard_labels_are_bounded() {
        let err =
            normalize_server_name(&"n".repeat(SERVER_NAME_MAX_BYTES + 1), "name").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        let err =
            normalize_optional_label(Some("x".repeat(SERVER_LABEL_MAX_BYTES + 1)), "public_note")
                .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn server_dashboard_metadata_json_is_bounded() {
        let metadata = DashboardMetadata {
            public_note: Some("x".repeat(SERVER_DASHBOARD_METADATA_MAX_BYTES)),
            ..Default::default()
        };
        let err = dashboard_metadata_json(&metadata).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn server_tag_input_is_bounded_before_truncation() {
        let tags = (0..=SERVER_TAG_INPUT_MAX_ITEMS)
            .map(|idx| format!("tag-{idx}"))
            .collect();
        let err = normalize_tag_input(tags).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        let err =
            normalize_tag_input(vec!["x".repeat(SERVER_TAG_INPUT_MAX_BYTES + 1)]).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn server_resource_ids_must_be_canonical_uuid_text() {
        let id = Uuid::parse_str("018f7e34-1234-4abc-8def-abcdefabcdef").unwrap();
        let canonical = id.to_string();

        assert_eq!(parse_agent_id(&canonical).unwrap().0, id);
        assert_eq!(parse_user_id(&canonical).unwrap().0, id);
        assert_eq!(parse_uuid(&canonical).unwrap(), id);
        assert_eq!(
            require_server_uuid_text(&canonical, "server_id").unwrap(),
            canonical
        );

        assert!(require_server_uuid_text("server-a", "server_id").is_err());
        assert!(require_server_uuid_text(&format!(" {id} "), "server_id").is_err());
        assert!(require_server_uuid_text(&id.simple().to_string(), "server_id").is_err());
        assert!(require_server_uuid_text(&canonical.to_uppercase(), "server_id").is_err());
        assert!(
            require_server_uuid_text(&"a".repeat(SERVER_UUID_TEXT_LEN + 1), "server_id").is_err()
        );
    }

    #[test]
    fn server_id_lists_are_bounded_and_require_canonical_ids() {
        let ids = (0..=SERVER_BATCH_MAX_SERVER_IDS)
            .map(|idx| Uuid::from_u128(idx as u128 + 1).to_string())
            .collect();
        let err = dedupe_ids(ids, SERVER_BATCH_MAX_SERVER_IDS, "server_ids").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        let deduped = dedupe_ids(
            vec![
                "00000000-0000-0000-0000-000000000001".into(),
                "00000000-0000-0000-0000-000000000001".into(),
            ],
            SERVER_BATCH_MAX_SERVER_IDS,
            "server_ids",
        )
        .unwrap();
        assert_eq!(deduped, vec!["00000000-0000-0000-0000-000000000001"]);

        let err = dedupe_ids(
            vec!["00000000000000000000000000000001".into()],
            SERVER_BATCH_MAX_SERVER_IDS,
            "server_ids",
        )
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn server_display_order_matches_database_integer_range() {
        assert!(normalize_display_order(Some(i32::MAX as i64), "display_order").is_ok());
        let err = normalize_display_order(Some(i32::MAX as i64 + 1), "display_order").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

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
