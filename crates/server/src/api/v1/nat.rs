#![allow(dead_code)]
#![allow(unused_imports)]

use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::api::types::ApiResponse;
use crate::api::v1::auth::AppState;
use crate::auth::middleware::{AuthKind, AuthSession, AuthUser};
use crate::auth::rbac::has_scope;
use crate::db::repository::NatMappingRepository;
use crate::db::AgentRepository;
use crate::db::Db;
use crate::nat::tunnel::nat_public_port_allowed;
use std::net::IpAddr;
use xlstatus_shared::nat::*;

const NAT_API_MAX_BODY_BYTES: usize = 64 * 1024;
const NAT_UUID_TEXT_LEN: usize = 36;
const NAT_MAX_LOCAL_HOST_BYTES: usize = 253;
const NAT_MAX_PROTOCOL_BYTES: usize = 16;
const NAT_MAX_DESCRIPTION_BYTES: usize = 1024;
const NAT_MAX_ALLOWED_SOURCES_BYTES: usize = 4096;
const NAT_MAX_ALLOWED_SOURCE_ENTRIES: usize = 64;
const NAT_MAX_ALLOWED_SOURCE_ENTRY_BYTES: usize = 128;
const NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING: u32 = 1024;
const NAT_MAX_IDLE_TIMEOUT_SECONDS: u32 = 24 * 60 * 60;
const NAT_MAX_BYTES_PER_TUNNEL: u64 = 1024 * 1024 * 1024 * 1024;
const NAT_MAX_BANDWIDTH_BYTES_PER_SECOND: u64 = 1024 * 1024 * 1024;
const NAT_MAX_RATE_LIMIT_WINDOW_SECONDS: u32 = 24 * 60 * 60;
const NAT_MAX_CONNECTIONS_PER_WINDOW: u32 = 100_000;
const NAT_MAX_BYTES_PER_WINDOW: u64 = 1024 * 1024 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct CreateNatMappingRequest {
    pub agent_id: String,
    pub local_host: String,
    pub local_port: u16,
    pub public_port: u16,
    pub protocol: String,
    pub description: Option<String>,
    pub allowed_sources: Option<String>,
    pub max_active_tunnels: Option<u32>,
    pub idle_timeout_seconds: Option<u32>,
    pub max_bytes_per_tunnel: Option<u64>,
    pub max_bandwidth_bytes_per_second: Option<u64>,
    pub rate_limit_window_seconds: Option<u32>,
    pub max_connections_per_window: Option<u32>,
    pub max_bytes_per_window: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNatMappingRequest {
    pub local_host: Option<String>,
    pub local_port: Option<u16>,
    pub public_port: Option<u16>,
    pub protocol: Option<String>,
    pub enabled: Option<bool>,
    pub description: Option<String>,
    pub allowed_sources: Option<String>,
    pub max_active_tunnels: Option<u32>,
    pub idle_timeout_seconds: Option<u32>,
    pub max_bytes_per_tunnel: Option<u64>,
    pub max_bandwidth_bytes_per_second: Option<u64>,
    pub rate_limit_window_seconds: Option<u32>,
    pub max_connections_per_window: Option<u32>,
    pub max_bytes_per_window: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct NatMappingResponse {
    pub mapping: NatMapping,
}

#[derive(Debug, Serialize)]
pub struct NatMappingListResponse {
    pub mappings: Vec<NatMapping>,
    pub total: usize,
}

#[derive(Debug)]
struct NormalizedCreateNatMappingRequest {
    agent_id: String,
    local_host: String,
    local_port: u16,
    public_port: u16,
    protocol: Protocol,
    description: Option<String>,
    allowed_sources: Option<String>,
    max_active_tunnels: Option<u32>,
    idle_timeout_seconds: Option<u32>,
    max_bytes_per_tunnel: Option<u64>,
    max_bandwidth_bytes_per_second: Option<u64>,
    rate_limit_window_seconds: Option<u32>,
    max_connections_per_window: Option<u32>,
    max_bytes_per_window: Option<u64>,
}

#[derive(Debug)]
struct NormalizedUpdateNatMappingRequest {
    local_host: Option<String>,
    local_port: Option<u16>,
    public_port: Option<u16>,
    protocol: Option<Protocol>,
    enabled: Option<bool>,
    description: Option<Option<String>>,
    allowed_sources: Option<Option<String>>,
    max_active_tunnels: Option<u32>,
    idle_timeout_seconds: Option<u32>,
    max_bytes_per_tunnel: Option<u64>,
    max_bandwidth_bytes_per_second: Option<u64>,
    rate_limit_window_seconds: Option<u32>,
    max_connections_per_window: Option<u32>,
    max_bytes_per_window: Option<u64>,
}

pub fn nat_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(NAT_API_MAX_BODY_BYTES)
}

/// Create a new NAT mapping
pub async fn create_nat_mapping(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreateNatMappingRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "nat:write")?;
    let req = normalize_create_nat_mapping_request(req).map_err(bad_request_with)?;
    require_active_nat_agent_or_403(&db, &auth_user, &req.agent_id).await?;
    validate_nat_local_target_or_403(&req.local_host)?;
    validate_local_port_or_403(req.local_port)?;
    validate_public_port_or_403(req.public_port)?;

    // Check if public port is already in use
    if let Ok(Some(_)) = NatMappingRepository::get_by_public_port(&db, req.public_port).await {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Public port {} is already in use", req.public_port)),
            }),
        ));
    }

    let now = Utc::now().to_rfc3339();
    let mapping = NatMapping {
        id: uuid::Uuid::now_v7().to_string(),
        agent_id: req.agent_id,
        local_host: req.local_host,
        local_port: req.local_port,
        public_port: req.public_port,
        protocol: req.protocol,
        enabled: true,
        description: req.description,
        allowed_sources: req.allowed_sources,
        max_active_tunnels: req.max_active_tunnels,
        idle_timeout_seconds: req.idle_timeout_seconds,
        max_bytes_per_tunnel: req.max_bytes_per_tunnel,
        max_bandwidth_bytes_per_second: req.max_bandwidth_bytes_per_second,
        rate_limit_window_seconds: req.rate_limit_window_seconds,
        max_connections_per_window: req.max_connections_per_window,
        max_bytes_per_window: req.max_bytes_per_window,
        created_at: now.clone(),
        updated_at: now,
    };

    NatMappingRepository::create(&db, &mapping)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to create NAT mapping: {}", e)),
                }),
            )
        })?;

    if let Some(manager) = crate::current_nat_manager() {
        if let Err(e) = manager.reload().await {
            tracing::warn!("NAT manager reload failed after create: {}", e);
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(ApiResponse {
            success: true,
            data: Some(NatMappingResponse { mapping }),
            error: None,
        }),
    ))
}

/// Get a NAT mapping by ID
pub async fn get_nat_mapping(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(mapping_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "nat:read")?;
    let mapping_id =
        normalize_nat_resource_uuid(mapping_id, "mapping_id").map_err(bad_request_with)?;
    let mapping = NatMappingRepository::get_by_id(&db, &mapping_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to get NAT mapping: {}", e)),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("NAT mapping not found".to_string()),
                }),
            )
        })?;

    require_nat_agent_or_403(&db, &auth_user, &mapping.agent_id).await?;
    Ok(Json(ApiResponse {
        success: true,
        data: Some(NatMappingResponse { mapping }),
        error: None,
    }))
}

/// List NAT mappings for an agent
pub async fn list_nat_mappings(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(agent_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "nat:read")?;
    let agent_id = normalize_nat_agent_id(agent_id).map_err(bad_request_with)?;
    require_nat_agent_or_403(&db, &auth_user, &agent_id).await?;
    let mappings = NatMappingRepository::list_by_agent(&db, &agent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to list NAT mappings: {}", e)),
                }),
            )
        })?;

    let total = mappings.len();

    Ok(Json(ApiResponse {
        success: true,
        data: Some(NatMappingListResponse { mappings, total }),
        error: None,
    }))
}

/// List enabled NAT mappings that can currently be loaded by the NAT runtime.
pub async fn list_all_nat_mappings(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "nat:read")?;
    require_admin_cookie_session_or_403(&auth_user)?;
    let mappings = list_runtime_nat_mappings(&db).await?;

    let total = mappings.len();

    Ok(Json(ApiResponse {
        success: true,
        data: Some(NatMappingListResponse { mappings, total }),
        error: None,
    }))
}

async fn list_runtime_nat_mappings(
    db: &Db,
) -> Result<Vec<NatMapping>, (StatusCode, Json<ApiResponse<()>>)> {
    NatMappingRepository::list_enabled_for_active_agents(db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to list NAT mappings: {}", e)),
                }),
            )
        })
}

/// Update a NAT mapping
pub async fn update_nat_mapping(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(mapping_id): Path<String>,
    Json(req): Json<UpdateNatMappingRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "nat:write")?;
    let mapping_id =
        normalize_nat_resource_uuid(mapping_id, "mapping_id").map_err(bad_request_with)?;
    let req = normalize_update_nat_mapping_request(req).map_err(bad_request_with)?;
    let mut mapping = NatMappingRepository::get_by_id(&db, &mapping_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to get NAT mapping: {}", e)),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("NAT mapping not found".to_string()),
                }),
            )
        })?;

    require_nat_agent_or_403(&db, &auth_user, &mapping.agent_id).await?;

    // Apply updates
    if let Some(local_host) = req.local_host {
        validate_nat_local_target_or_403(&local_host)?;
        mapping.local_host = local_host;
    }
    if let Some(local_port) = req.local_port {
        validate_local_port_or_403(local_port)?;
        mapping.local_port = local_port;
    }
    if let Some(public_port) = req.public_port {
        validate_public_port_or_403(public_port)?;
        // Check if new public port is available
        if public_port != mapping.public_port {
            if let Ok(Some(_)) = NatMappingRepository::get_by_public_port(&db, public_port).await {
                return Err((
                    StatusCode::CONFLICT,
                    Json(ApiResponse {
                        success: false,
                        data: None,
                        error: Some(format!("Public port {} is already in use", public_port)),
                    }),
                ));
            }
        }
        mapping.public_port = public_port;
    }
    if let Some(protocol) = req.protocol {
        mapping.protocol = protocol;
    }
    if req.enabled == Some(true) || (mapping.enabled && req.enabled != Some(false)) {
        require_active_nat_agent_or_403(&db, &auth_user, &mapping.agent_id).await?;
    }
    if let Some(enabled) = req.enabled {
        mapping.enabled = enabled;
    }
    if let Some(description) = req.description {
        mapping.description = description;
    }
    if let Some(allowed_sources) = req.allowed_sources {
        mapping.allowed_sources = allowed_sources;
    }
    if let Some(max_active_tunnels) = req.max_active_tunnels {
        mapping.max_active_tunnels = Some(max_active_tunnels);
    }
    if let Some(idle_timeout_seconds) = req.idle_timeout_seconds {
        mapping.idle_timeout_seconds = Some(idle_timeout_seconds);
    }
    if let Some(max_bytes_per_tunnel) = req.max_bytes_per_tunnel {
        mapping.max_bytes_per_tunnel = Some(max_bytes_per_tunnel);
    }
    if let Some(max_bandwidth_bytes_per_second) = req.max_bandwidth_bytes_per_second {
        mapping.max_bandwidth_bytes_per_second = Some(max_bandwidth_bytes_per_second);
    }
    if let Some(rate_limit_window_seconds) = req.rate_limit_window_seconds {
        mapping.rate_limit_window_seconds = Some(rate_limit_window_seconds);
    }
    if let Some(max_connections_per_window) = req.max_connections_per_window {
        mapping.max_connections_per_window = Some(max_connections_per_window);
    }
    if let Some(max_bytes_per_window) = req.max_bytes_per_window {
        mapping.max_bytes_per_window = Some(max_bytes_per_window);
    }

    mapping.updated_at = Utc::now().to_rfc3339();

    NatMappingRepository::update(&db, &mapping)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to update NAT mapping: {}", e)),
                }),
            )
        })?;

    if let Some(manager) = crate::current_nat_manager() {
        if let Err(e) = manager.reload().await {
            tracing::warn!("NAT manager reload failed after update: {}", e);
        }
    }

    Ok(Json(ApiResponse {
        success: true,
        data: Some(NatMappingResponse { mapping }),
        error: None,
    }))
}

/// Delete a NAT mapping
pub async fn delete_nat_mapping(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(mapping_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "nat:delete")?;
    let mapping_id =
        normalize_nat_resource_uuid(mapping_id, "mapping_id").map_err(bad_request_with)?;
    // Check if mapping exists
    let mapping = NatMappingRepository::get_by_id(&db, &mapping_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to get NAT mapping: {}", e)),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("NAT mapping not found".to_string()),
                }),
            )
        })?;

    require_nat_agent_or_403(&db, &auth_user, &mapping.agent_id).await?;
    NatMappingRepository::delete(&db, &mapping_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to delete NAT mapping: {}", e)),
                }),
            )
        })?;

    if let Some(manager) = crate::current_nat_manager() {
        if let Err(e) = manager.reload().await {
            tracing::warn!("NAT manager reload failed after delete: {}", e);
        }
    }

    Ok((
        StatusCode::OK,
        Json(ApiResponse::<()> {
            success: true,
            data: None,
            error: None,
        }),
    ))
}

fn require_scope_or_403(
    auth_user: &AuthUser,
    required_scope: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let session = AuthSession {
        session_id: auth_user.session_id.clone(),
        user_id: auth_user.user.id,
        username: auth_user.user.username.clone(),
        role: auth_user.user.role,
        csrf_token: auth_user.csrf_token.clone(),
        auth_kind: auth_user.auth_kind.clone(),
        scopes: auth_user.scopes.clone(),
        server_ids: auth_user.server_ids.clone(),
        pat_id: auth_user.pat_id.clone(),
    };
    if has_scope(&session, required_scope) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("missing required scope: {}", required_scope)),
            }),
        ))
    }
}

fn session_of(auth_user: &AuthUser) -> AuthSession {
    AuthSession {
        session_id: auth_user.session_id.clone(),
        user_id: auth_user.user.id,
        username: auth_user.user.username.clone(),
        role: auth_user.user.role,
        csrf_token: auth_user.csrf_token.clone(),
        auth_kind: auth_user.auth_kind.clone(),
        scopes: auth_user.scopes.clone(),
        server_ids: auth_user.server_ids.clone(),
        pat_id: auth_user.pat_id.clone(),
    }
}

fn require_admin_or_403(auth_user: &AuthUser) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    if auth_user.user.role.is_admin() {
        Ok(())
    } else {
        Err(forbidden_with("admin role required".to_string()))
    }
}

fn require_admin_cookie_session_or_403(
    auth_user: &AuthUser,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    require_admin_or_403(auth_user)?;
    if matches!(auth_user.auth_kind, AuthKind::PersonalAccessToken) {
        return Err(forbidden_with("Cookie session required".to_string()));
    }
    Ok(())
}

fn validate_public_port_or_403(port: u16) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    if nat_public_port_allowed(port) {
        return Ok(());
    }
    Err((
        StatusCode::BAD_REQUEST,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some(format!(
                "public_port {} is below the configured NAT public port minimum",
                port
            )),
        }),
    ))
}

fn validate_local_port_or_403(port: u16) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    if port > 0 {
        return Ok(());
    }
    Err(bad_request_with(
        "local_port must be greater than zero".to_string(),
    ))
}

fn validate_nat_local_target_or_403(host: &str) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    if nat_private_targets_allowed() || nat_local_target_is_loopback(host) {
        return Ok(());
    }
    Err(bad_request_with(
        "NAT local_host must resolve to the Agent loopback interface unless private NAT targets are explicitly enabled".to_string(),
    ))
}

fn nat_local_target_is_loopback(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn nat_private_targets_allowed() -> bool {
    [
        "XLSTATUS_ALLOW_PRIVATE_NAT_TARGETS",
        "XLSTATUS_ALLOW_PRIVATE_OUTBOUND",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    })
}

fn normalize_create_nat_mapping_request(
    req: CreateNatMappingRequest,
) -> Result<NormalizedCreateNatMappingRequest, String> {
    Ok(NormalizedCreateNatMappingRequest {
        agent_id: normalize_nat_agent_id(req.agent_id)?,
        local_host: normalize_required_nat_text(
            req.local_host,
            NAT_MAX_LOCAL_HOST_BYTES,
            "local_host",
        )?,
        local_port: req.local_port,
        public_port: req.public_port,
        protocol: normalize_nat_protocol(&req.protocol)?,
        description: normalize_optional_nat_text(
            req.description,
            NAT_MAX_DESCRIPTION_BYTES,
            "description",
        )?,
        allowed_sources: normalize_allowed_sources(req.allowed_sources.as_deref())?,
        max_active_tunnels: normalize_bounded_u32(
            req.max_active_tunnels,
            NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING,
            "max_active_tunnels",
        )?,
        idle_timeout_seconds: normalize_bounded_u32(
            req.idle_timeout_seconds,
            NAT_MAX_IDLE_TIMEOUT_SECONDS,
            "idle_timeout_seconds",
        )?,
        max_bytes_per_tunnel: normalize_bounded_u64(
            req.max_bytes_per_tunnel,
            NAT_MAX_BYTES_PER_TUNNEL,
            "max_bytes_per_tunnel",
        )?,
        max_bandwidth_bytes_per_second: normalize_bounded_u64(
            req.max_bandwidth_bytes_per_second,
            NAT_MAX_BANDWIDTH_BYTES_PER_SECOND,
            "max_bandwidth_bytes_per_second",
        )?,
        rate_limit_window_seconds: normalize_bounded_u32(
            req.rate_limit_window_seconds,
            NAT_MAX_RATE_LIMIT_WINDOW_SECONDS,
            "rate_limit_window_seconds",
        )?,
        max_connections_per_window: normalize_bounded_u32(
            req.max_connections_per_window,
            NAT_MAX_CONNECTIONS_PER_WINDOW,
            "max_connections_per_window",
        )?,
        max_bytes_per_window: normalize_bounded_u64(
            req.max_bytes_per_window,
            NAT_MAX_BYTES_PER_WINDOW,
            "max_bytes_per_window",
        )?,
    })
}

fn normalize_update_nat_mapping_request(
    req: UpdateNatMappingRequest,
) -> Result<NormalizedUpdateNatMappingRequest, String> {
    Ok(NormalizedUpdateNatMappingRequest {
        local_host: req
            .local_host
            .map(|value| normalize_required_nat_text(value, NAT_MAX_LOCAL_HOST_BYTES, "local_host"))
            .transpose()?,
        local_port: req.local_port,
        public_port: req.public_port,
        protocol: req
            .protocol
            .as_deref()
            .map(normalize_nat_protocol)
            .transpose()?,
        enabled: req.enabled,
        description: match req.description {
            Some(value) => Some(normalize_optional_nat_text(
                Some(value),
                NAT_MAX_DESCRIPTION_BYTES,
                "description",
            )?),
            None => None,
        },
        allowed_sources: match req.allowed_sources {
            Some(value) => Some(normalize_allowed_sources(Some(&value))?),
            None => None,
        },
        max_active_tunnels: normalize_bounded_u32(
            req.max_active_tunnels,
            NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING,
            "max_active_tunnels",
        )?,
        idle_timeout_seconds: normalize_bounded_u32(
            req.idle_timeout_seconds,
            NAT_MAX_IDLE_TIMEOUT_SECONDS,
            "idle_timeout_seconds",
        )?,
        max_bytes_per_tunnel: normalize_bounded_u64(
            req.max_bytes_per_tunnel,
            NAT_MAX_BYTES_PER_TUNNEL,
            "max_bytes_per_tunnel",
        )?,
        max_bandwidth_bytes_per_second: normalize_bounded_u64(
            req.max_bandwidth_bytes_per_second,
            NAT_MAX_BANDWIDTH_BYTES_PER_SECOND,
            "max_bandwidth_bytes_per_second",
        )?,
        rate_limit_window_seconds: normalize_bounded_u32(
            req.rate_limit_window_seconds,
            NAT_MAX_RATE_LIMIT_WINDOW_SECONDS,
            "rate_limit_window_seconds",
        )?,
        max_connections_per_window: normalize_bounded_u32(
            req.max_connections_per_window,
            NAT_MAX_CONNECTIONS_PER_WINDOW,
            "max_connections_per_window",
        )?,
        max_bytes_per_window: normalize_bounded_u64(
            req.max_bytes_per_window,
            NAT_MAX_BYTES_PER_WINDOW,
            "max_bytes_per_window",
        )?,
    })
}

fn normalize_nat_agent_id(value: String) -> Result<String, String> {
    normalize_nat_resource_uuid(value, "agent_id")
}

fn normalize_nat_resource_uuid(value: String, field: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() != NAT_UUID_TEXT_LEN {
        return Err(format!("{field} must be a canonical UUID"));
    }
    let parsed =
        uuid::Uuid::parse_str(&value).map_err(|_| format!("{field} must be a canonical UUID"))?;
    let canonical = parsed.to_string();
    if canonical != value {
        return Err(format!("{field} must be a canonical UUID"));
    }
    Ok(canonical)
}

fn normalize_nat_protocol(value: &str) -> Result<Protocol, String> {
    let value = normalize_required_nat_text(value.to_string(), NAT_MAX_PROTOCOL_BYTES, "protocol")?;
    Protocol::from_str(&value).ok_or_else(|| "protocol must be tcp or udp".to_string())
}

fn normalize_required_nat_text(
    value: String,
    max_bytes: usize,
    field: &str,
) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > max_bytes {
        return Err(format!("{field} must be at most {max_bytes} bytes"));
    }
    Ok(value)
}

fn normalize_optional_nat_text(
    value: Option<String>,
    max_bytes: usize,
    field: &str,
) -> Result<Option<String>, String> {
    let Some(value) = value.map(|value| value.trim().to_string()) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > max_bytes {
        return Err(format!("{field} must be at most {max_bytes} bytes"));
    }
    Ok(Some(value))
}

fn normalize_bounded_u32(
    value: Option<u32>,
    max_value: u32,
    field: &str,
) -> Result<Option<u32>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value == 0 {
        return Err(format!("{field} must be greater than zero"));
    }
    if value > max_value {
        return Err(format!("{field} must be less than or equal to {max_value}"));
    }
    Ok(Some(value))
}

fn normalize_bounded_u64(
    value: Option<u64>,
    max_value: u64,
    field: &str,
) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value == 0 {
        return Err(format!("{field} must be greater than zero"));
    }
    if value > max_value {
        return Err(format!("{field} must be less than or equal to {max_value}"));
    }
    Ok(Some(value))
}

fn normalize_allowed_sources(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value.map(str::trim) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > NAT_MAX_ALLOWED_SOURCES_BYTES {
        return Err(format!(
            "allowed_sources must be at most {NAT_MAX_ALLOWED_SOURCES_BYTES} bytes"
        ));
    }
    let entries: Vec<String> = value
        .split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect();
    if entries.is_empty() {
        return Ok(None);
    }
    if entries.len() > NAT_MAX_ALLOWED_SOURCE_ENTRIES {
        return Err(format!(
            "allowed_sources must contain at most {NAT_MAX_ALLOWED_SOURCE_ENTRIES} entries"
        ));
    }
    for entry in &entries {
        if entry.len() > NAT_MAX_ALLOWED_SOURCE_ENTRY_BYTES {
            return Err(format!(
                "allowed source entry must be at most {NAT_MAX_ALLOWED_SOURCE_ENTRY_BYTES} bytes"
            ));
        }
        if !crate::nat::tunnel::nat_source_entry_valid(entry) {
            return Err(format!("invalid NAT allowed source CIDR or IP: {entry}"));
        }
    }
    Ok(Some(entries.join(",")))
}

async fn load_authorized_nat_agent_or_403(
    db: &Db,
    auth_user: &AuthUser,
    agent_id: &str,
) -> Result<crate::db::Agent, (StatusCode, Json<ApiResponse<()>>)> {
    let agent_id = normalize_nat_agent_id(agent_id.to_string()).map_err(bad_request_with)?;
    let session = session_of(auth_user);
    if !nat_server_id_allowed(session.server_ids.as_deref(), &agent_id) {
        return Err(forbidden_with(
            "agent is outside PAT server allowlist".to_string(),
        ));
    }

    let agent_uuid = uuid::Uuid::parse_str(&agent_id)
        .map_err(|_| bad_request_with("invalid agent_id".to_string()))?;
    let repo = AgentRepository::new(db.clone());
    let agent = repo
        .find_by_id(xlstatus_shared::AgentId(agent_uuid))
        .await
        .map_err(|e| internal_with(e.to_string()))?
        .ok_or_else(|| forbidden_with("agent not found".to_string()))?;
    if auth_user.is_pat() || !auth_user.user.role.is_admin() {
        if agent.owner_user_id != auth_user.user.id {
            return Err(forbidden_with(
                "agent is not owned by the calling user".to_string(),
            ));
        }
    }

    Ok(agent)
}

fn nat_server_id_allowed(allowed: Option<&[String]>, agent_id: &str) -> bool {
    let Some(allowed) = allowed else {
        return true;
    };
    let Ok(agent_uuid) = uuid::Uuid::parse_str(agent_id) else {
        return false;
    };
    allowed.iter().any(|allowed_id| {
        allowed_id == agent_id
            || uuid::Uuid::parse_str(allowed_id)
                .map(|allowed_uuid| allowed_uuid == agent_uuid)
                .unwrap_or(false)
    })
}

async fn require_nat_agent_or_403(
    db: &Db,
    auth_user: &AuthUser,
    agent_id: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    load_authorized_nat_agent_or_403(db, auth_user, agent_id)
        .await
        .map(|_| ())
}

async fn require_active_nat_agent_or_403(
    db: &Db,
    auth_user: &AuthUser,
    agent_id: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let agent = load_authorized_nat_agent_or_403(db, auth_user, agent_id).await?;
    if agent.revoked_at.is_some() {
        return Err(forbidden_with("agent has been revoked".to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CreateAgentInput, CreateUserInput, DatabaseBackend, UserRepository};
    use xlstatus_shared::{AgentId, UserId, UserRole};

    #[test]
    fn nat_global_list_rejects_admin_pat() {
        let auth = auth_user(AuthKind::PersonalAccessToken, UserRole::Admin);
        assert!(require_admin_cookie_session_or_403(&auth).is_err());
    }

    #[test]
    fn nat_global_list_allows_admin_cookie_session() {
        let auth = auth_user(AuthKind::Session, UserRole::Admin);
        assert!(require_admin_cookie_session_or_403(&auth).is_ok());
    }

    #[test]
    fn nat_allowed_sources_normalizes_valid_entries() {
        let normalized =
            normalize_allowed_sources(Some("127.0.0.1, 203.0.113.0/24\n::1/128")).unwrap();

        assert_eq!(
            normalized.as_deref(),
            Some("127.0.0.1,203.0.113.0/24,::1/128")
        );
    }

    #[test]
    fn nat_allowed_sources_rejects_invalid_entries() {
        assert!(normalize_allowed_sources(Some("not-a-cidr")).is_err());
    }

    #[test]
    fn nat_mapping_resource_limits_are_explicit() {
        assert_eq!(NAT_API_MAX_BODY_BYTES, 64 * 1024);
        assert_eq!(NAT_UUID_TEXT_LEN, 36);
        assert_eq!(NAT_MAX_LOCAL_HOST_BYTES, 253);
        assert_eq!(NAT_MAX_PROTOCOL_BYTES, 16);
        assert_eq!(NAT_MAX_DESCRIPTION_BYTES, 1024);
        assert_eq!(NAT_MAX_ALLOWED_SOURCES_BYTES, 4096);
        assert_eq!(NAT_MAX_ALLOWED_SOURCE_ENTRIES, 64);
        assert_eq!(NAT_MAX_ALLOWED_SOURCE_ENTRY_BYTES, 128);
        assert_eq!(NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING, 1024);
        assert_eq!(NAT_MAX_IDLE_TIMEOUT_SECONDS, 24 * 60 * 60);
        assert_eq!(NAT_MAX_BYTES_PER_TUNNEL, 1024 * 1024 * 1024 * 1024);
        assert_eq!(NAT_MAX_BANDWIDTH_BYTES_PER_SECOND, 1024 * 1024 * 1024);
        assert_eq!(NAT_MAX_RATE_LIMIT_WINDOW_SECONDS, 24 * 60 * 60);
        assert_eq!(NAT_MAX_CONNECTIONS_PER_WINDOW, 100_000);
        assert_eq!(NAT_MAX_BYTES_PER_WINDOW, 1024 * 1024 * 1024 * 1024);
    }

    #[test]
    fn nat_agent_id_and_protocol_are_normalized() {
        let agent_id = uuid::Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();

        assert_eq!(
            normalize_nat_agent_id(agent_id.to_string()).unwrap(),
            agent_id.to_string()
        );
        assert!(normalize_nat_agent_id("server-a".into()).is_err());
        assert!(normalize_nat_agent_id(format!(" {} ", agent_id)).is_err());
        assert!(normalize_nat_agent_id(agent_id.simple().to_string()).is_err());
        assert!(normalize_nat_agent_id(agent_id.to_string().to_uppercase()).is_err());
        assert!(normalize_nat_agent_id("a".repeat(NAT_UUID_TEXT_LEN + 1)).is_err());
        assert_eq!(
            normalize_nat_resource_uuid(agent_id.to_string(), "mapping_id").unwrap(),
            agent_id.to_string()
        );
        assert!(normalize_nat_resource_uuid(agent_id.simple().to_string(), "mapping_id").is_err());

        assert!(matches!(
            normalize_nat_protocol(" TCP ").unwrap(),
            Protocol::Tcp
        ));
        assert!(normalize_nat_protocol("icmp").is_err());
        assert!(normalize_nat_protocol(&"a".repeat(NAT_MAX_PROTOCOL_BYTES + 1)).is_err());
    }

    #[test]
    fn nat_text_and_allowed_sources_are_bounded() {
        assert!(normalize_required_nat_text(
            "127.0.0.1".into(),
            NAT_MAX_LOCAL_HOST_BYTES,
            "local_host"
        )
        .is_ok());
        assert!(normalize_required_nat_text(
            "a".repeat(NAT_MAX_LOCAL_HOST_BYTES + 1),
            NAT_MAX_LOCAL_HOST_BYTES,
            "local_host"
        )
        .is_err());
        assert!(normalize_optional_nat_text(
            Some("a".repeat(NAT_MAX_DESCRIPTION_BYTES + 1)),
            NAT_MAX_DESCRIPTION_BYTES,
            "description"
        )
        .is_err());

        let entries = (0..NAT_MAX_ALLOWED_SOURCE_ENTRIES)
            .map(|idx| format!("203.0.113.{idx}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(normalize_allowed_sources(Some(&entries)).is_ok());

        let too_many_entries = (0..=NAT_MAX_ALLOWED_SOURCE_ENTRIES)
            .map(|idx| format!("203.0.113.{idx}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(normalize_allowed_sources(Some(&too_many_entries)).is_err());
        assert!(
            normalize_allowed_sources(Some(&"a".repeat(NAT_MAX_ALLOWED_SOURCES_BYTES + 1)))
                .is_err()
        );
        assert!(normalize_allowed_sources(Some("not-a-cidr")).is_err());
    }

    #[test]
    fn nat_policy_values_are_bounded() {
        assert!(normalize_bounded_u32(
            Some(1),
            NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING,
            "max_active_tunnels"
        )
        .is_ok());
        assert!(normalize_bounded_u32(
            Some(0),
            NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING,
            "max_active_tunnels"
        )
        .is_err());
        assert!(normalize_bounded_u32(
            Some(NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING + 1),
            NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING,
            "max_active_tunnels"
        )
        .is_err());
        assert!(
            normalize_bounded_u64(Some(1), NAT_MAX_BYTES_PER_TUNNEL, "max_bytes_per_tunnel")
                .is_ok()
        );
        assert!(
            normalize_bounded_u64(Some(0), NAT_MAX_BYTES_PER_TUNNEL, "max_bytes_per_tunnel")
                .is_err()
        );
        assert!(normalize_bounded_u64(
            Some(NAT_MAX_BYTES_PER_TUNNEL + 1),
            NAT_MAX_BYTES_PER_TUNNEL,
            "max_bytes_per_tunnel"
        )
        .is_err());
    }

    #[test]
    fn nat_create_request_normalizes_inputs() {
        let agent_id = uuid::Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
        let mut req = create_nat_request(agent_id);
        req.local_host = " 127.0.0.1 ".into();
        req.protocol = " TCP ".into();
        req.description = Some(" demo ".into());
        req.allowed_sources = Some("127.0.0.1, ::1/128".into());
        req.max_active_tunnels = Some(2);

        let normalized = normalize_create_nat_mapping_request(req).unwrap();

        assert_eq!(normalized.agent_id, agent_id.to_string());
        assert_eq!(normalized.local_host, "127.0.0.1");
        assert!(matches!(normalized.protocol, Protocol::Tcp));
        assert_eq!(normalized.description.as_deref(), Some("demo"));
        assert_eq!(
            normalized.allowed_sources.as_deref(),
            Some("127.0.0.1,::1/128")
        );
        assert_eq!(normalized.max_active_tunnels, Some(2));
    }

    #[test]
    fn nat_server_allowlist_uses_uuid_semantics() {
        let agent_id = uuid::Uuid::from_bytes([6; 16]);

        assert!(nat_server_id_allowed(None, &agent_id.to_string()));
        assert!(nat_server_id_allowed(
            Some(&[agent_id.simple().to_string()]),
            &agent_id.to_string()
        ));
        assert!(!nat_server_id_allowed(
            Some(&[uuid::Uuid::from_bytes([7; 16]).to_string()]),
            &agent_id.to_string()
        ));
        assert!(!nat_server_id_allowed(
            Some(&["server-a".into()]),
            &agent_id.to_string()
        ));
    }

    #[test]
    fn nat_local_target_allows_loopback_by_default() {
        std::env::remove_var("XLSTATUS_ALLOW_PRIVATE_NAT_TARGETS");
        std::env::remove_var("XLSTATUS_ALLOW_PRIVATE_OUTBOUND");

        assert!(validate_nat_local_target_or_403("127.0.0.1").is_ok());
        assert!(validate_nat_local_target_or_403("localhost").is_ok());
        assert!(validate_nat_local_target_or_403("::1").is_ok());
    }

    #[test]
    fn nat_local_target_rejects_private_non_loopback_by_default() {
        std::env::remove_var("XLSTATUS_ALLOW_PRIVATE_NAT_TARGETS");
        std::env::remove_var("XLSTATUS_ALLOW_PRIVATE_OUTBOUND");

        assert!(validate_nat_local_target_or_403("192.168.1.10").is_err());
        assert!(validate_nat_local_target_or_403("10.0.0.5").is_err());
    }

    #[test]
    fn nat_local_target_escape_hatch_allows_private_targets() {
        std::env::set_var("XLSTATUS_ALLOW_PRIVATE_NAT_TARGETS", "1");
        assert!(validate_nat_local_target_or_403("192.168.1.10").is_ok());
        std::env::remove_var("XLSTATUS_ALLOW_PRIVATE_NAT_TARGETS");
    }

    #[tokio::test]
    async fn nat_write_rejects_revoked_agent_but_visibility_allows_cleanup() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: format!("nat-api-owner-{}", uuid::Uuid::now_v7()),
                password: "password123".into(),
                role: UserRole::Member,
            })
            .await
            .unwrap();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let agent_repo = AgentRepository::new(db.clone());
        agent_repo
            .create_with_id(
                agent_id,
                CreateAgentInput {
                    name: "revoked-nat-agent".into(),
                    public_key: "pk".into(),
                    owner_user_id: user.id,
                },
            )
            .await
            .unwrap();
        agent_repo.revoke(agent_id).await.unwrap();
        let auth = auth_user_for(user.id, AuthKind::Session, UserRole::Member);

        assert!(
            require_nat_agent_or_403(&db, &auth, &agent_id.0.to_string())
                .await
                .is_ok()
        );
        let err = require_active_nat_agent_or_403(&db, &auth, &agent_id.0.to_string())
            .await
            .unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn nat_global_runtime_list_ignores_revoked_agent_mappings() {
        let db = test_db().await;
        let active_mapping = create_nat_mapping_record(&db, 18080, false).await;
        let revoked_mapping = create_nat_mapping_record(&db, 18081, true).await;

        let mappings = match list_runtime_nat_mappings(&db).await {
            Ok(mappings) => mappings,
            Err((status, _)) => panic!("runtime NAT list failed with {status}"),
        };
        let ids = mappings
            .iter()
            .map(|mapping| mapping.id.as_str())
            .collect::<Vec<_>>();

        assert!(ids.contains(&active_mapping.id.as_str()));
        assert!(!ids.contains(&revoked_mapping.id.as_str()));
    }

    #[test]
    fn nat_mapping_tunnel_limit_rejects_zero() {
        assert!(normalize_bounded_u32(
            Some(0),
            NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING,
            "max_active_tunnels"
        )
        .is_err());
        assert!(normalize_bounded_u32(
            Some(1),
            NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING,
            "max_active_tunnels"
        )
        .is_ok());
        assert!(normalize_bounded_u32(
            None,
            NAT_MAX_ACTIVE_TUNNELS_PER_MAPPING,
            "max_active_tunnels"
        )
        .is_ok());
    }

    #[test]
    fn nat_idle_timeout_and_byte_limit_reject_zero() {
        assert!(normalize_bounded_u32(
            Some(0),
            NAT_MAX_IDLE_TIMEOUT_SECONDS,
            "idle_timeout_seconds"
        )
        .is_err());
        assert!(
            normalize_bounded_u64(Some(0), NAT_MAX_BYTES_PER_TUNNEL, "max_bytes_per_tunnel")
                .is_err()
        );
        assert!(normalize_bounded_u64(
            Some(0),
            NAT_MAX_BANDWIDTH_BYTES_PER_SECOND,
            "max_bandwidth_bytes_per_second"
        )
        .is_err());
        assert!(normalize_bounded_u32(
            Some(30),
            NAT_MAX_IDLE_TIMEOUT_SECONDS,
            "idle_timeout_seconds"
        )
        .is_ok());
        assert!(normalize_bounded_u64(
            Some(1024),
            NAT_MAX_BYTES_PER_TUNNEL,
            "max_bytes_per_tunnel"
        )
        .is_ok());
        assert!(normalize_bounded_u64(
            Some(1024),
            NAT_MAX_BANDWIDTH_BYTES_PER_SECOND,
            "max_bandwidth_bytes_per_second"
        )
        .is_ok());
    }

    #[test]
    fn nat_rate_window_limits_reject_zero_and_overflow() {
        assert!(normalize_bounded_u32(
            Some(0),
            NAT_MAX_RATE_LIMIT_WINDOW_SECONDS,
            "rate_limit_window_seconds"
        )
        .is_err());
        assert!(normalize_bounded_u32(
            Some(NAT_MAX_RATE_LIMIT_WINDOW_SECONDS + 1),
            NAT_MAX_RATE_LIMIT_WINDOW_SECONDS,
            "rate_limit_window_seconds"
        )
        .is_err());
        assert!(
            normalize_bounded_u64(Some(0), NAT_MAX_BYTES_PER_WINDOW, "max_bytes_per_window")
                .is_err()
        );
        assert!(normalize_bounded_u64(
            Some(NAT_MAX_BYTES_PER_WINDOW + 1),
            NAT_MAX_BYTES_PER_WINDOW,
            "max_bytes_per_window"
        )
        .is_err());
        assert!(normalize_bounded_u32(
            Some(60),
            NAT_MAX_RATE_LIMIT_WINDOW_SECONDS,
            "rate_limit_window_seconds"
        )
        .is_ok());
        assert!(normalize_bounded_u32(
            Some(10),
            NAT_MAX_CONNECTIONS_PER_WINDOW,
            "max_connections_per_window"
        )
        .is_ok());
        assert!(normalize_bounded_u64(
            Some(1024),
            NAT_MAX_BYTES_PER_WINDOW,
            "max_bytes_per_window"
        )
        .is_ok());
    }

    fn auth_user(auth_kind: AuthKind, role: UserRole) -> AuthUser {
        auth_user_for(UserId(uuid::Uuid::from_bytes([8; 16])), auth_kind, role)
    }

    fn auth_user_for(user_id: UserId, auth_kind: AuthKind, role: UserRole) -> AuthUser {
        AuthUser {
            user: crate::db::User {
                id: user_id,
                username: "admin".into(),
                password_hash: "hash".into(),
                role,
                token_version: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            session_id: "session".into(),
            csrf_token: "csrf".into(),
            auth_kind,
            scopes: vec!["nat:read".into()],
            server_ids: None,
            pat_id: None,
        }
    }

    async fn test_db() -> DatabaseBackend {
        let path =
            std::env::temp_dir().join(format!("xlstatus-nat-api-test-{}.db", uuid::Uuid::now_v7()));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn create_nat_mapping_record(
        db: &DatabaseBackend,
        public_port: u16,
        revoke_agent: bool,
    ) -> NatMapping {
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: format!("nat-runtime-owner-{}", uuid::Uuid::now_v7()),
                password: "password123".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let agent_repo = AgentRepository::new(db.clone());
        agent_repo
            .create_with_id(
                agent_id,
                CreateAgentInput {
                    name: "nat-runtime-agent".into(),
                    public_key: "pk".into(),
                    owner_user_id: user.id,
                },
            )
            .await
            .unwrap();
        if revoke_agent {
            agent_repo.revoke(agent_id).await.unwrap();
        }

        let now = Utc::now().to_rfc3339();
        let mapping = NatMapping {
            id: uuid::Uuid::now_v7().to_string(),
            agent_id: agent_id.0.to_string(),
            local_host: "127.0.0.1".into(),
            local_port: 8080,
            public_port,
            protocol: Protocol::Tcp,
            enabled: true,
            description: None,
            allowed_sources: None,
            max_active_tunnels: None,
            idle_timeout_seconds: None,
            max_bytes_per_tunnel: None,
            max_bandwidth_bytes_per_second: None,
            rate_limit_window_seconds: None,
            max_connections_per_window: None,
            max_bytes_per_window: None,
            created_at: now.clone(),
            updated_at: now,
        };
        NatMappingRepository::create(db, &mapping).await.unwrap();
        mapping
    }

    fn create_nat_request(agent_id: uuid::Uuid) -> CreateNatMappingRequest {
        CreateNatMappingRequest {
            agent_id: agent_id.to_string(),
            local_host: "127.0.0.1".into(),
            local_port: 8080,
            public_port: 18080,
            protocol: "tcp".into(),
            description: None,
            allowed_sources: None,
            max_active_tunnels: None,
            idle_timeout_seconds: None,
            max_bytes_per_tunnel: None,
            max_bandwidth_bytes_per_second: None,
            rate_limit_window_seconds: None,
            max_connections_per_window: None,
            max_bytes_per_window: None,
        }
    }
}

fn forbidden_with(msg: String) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::FORBIDDEN,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some(msg),
        }),
    )
}

fn bad_request_with(msg: String) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some(msg),
        }),
    )
}

fn internal_with(msg: String) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some(msg),
        }),
    )
}
