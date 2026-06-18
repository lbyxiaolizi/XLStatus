#![allow(dead_code)]
#![allow(unused_imports)]

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::api::types::ApiResponse;
use crate::api::v1::auth::AppState;
use crate::auth::middleware::{AuthSession, AuthUser};
use crate::auth::rbac::{can_access_server, has_scope};
use crate::db::repository::NatMappingRepository;
use crate::db::AgentRepository;
use crate::db::Db;
use xlstatus_shared::nat::*;

#[derive(Debug, Deserialize)]
pub struct CreateNatMappingRequest {
    pub agent_id: String,
    pub local_host: String,
    pub local_port: u16,
    pub public_port: u16,
    pub protocol: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNatMappingRequest {
    pub local_host: Option<String>,
    pub local_port: Option<u16>,
    pub public_port: Option<u16>,
    pub protocol: Option<String>,
    pub enabled: Option<bool>,
    pub description: Option<String>,
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

/// Create a new NAT mapping
pub async fn create_nat_mapping(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreateNatMappingRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "nat:write")?;
    validate_nat_agent_or_403(&db, &auth_user, &req.agent_id).await?;
    // Validate protocol
    let protocol = Protocol::from_str(&req.protocol).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Invalid protocol, must be 'tcp' or 'udp'".to_string()),
            }),
        )
    })?;

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
        protocol,
        enabled: true,
        description: req.description,
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
    validate_nat_agent_or_403(&db, &auth_user, &agent_id).await?;
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

/// List all enabled NAT mappings
pub async fn list_all_nat_mappings(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "nat:read")?;
    let mappings = NatMappingRepository::list_enabled(&db).await.map_err(|e| {
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

/// Update a NAT mapping
pub async fn update_nat_mapping(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(mapping_id): Path<String>,
    Json(req): Json<UpdateNatMappingRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "nat:write")?;
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

    // Apply updates
    if let Some(local_host) = req.local_host {
        mapping.local_host = local_host;
    }
    if let Some(local_port) = req.local_port {
        mapping.local_port = local_port;
    }
    if let Some(public_port) = req.public_port {
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
    if let Some(protocol_str) = req.protocol {
        let protocol = Protocol::from_str(&protocol_str).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("Invalid protocol".to_string()),
                }),
            )
        })?;
        mapping.protocol = protocol;
    }
    if let Some(enabled) = req.enabled {
        mapping.enabled = enabled;
    }
    if req.description.is_some() {
        mapping.description = req.description;
    }

    mapping.updated_at = Utc::now().to_rfc3339();

    require_nat_agent_or_403(&db, &auth_user, &mapping.agent_id).await?;
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

async fn validate_nat_agent_or_403(
    db: &Db,
    auth_user: &AuthUser,
    agent_id: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let session = session_of(auth_user);
    if !can_access_server(&session, agent_id) {
        return Err(forbidden_with(
            "agent is outside PAT server allowlist".to_string(),
        ));
    }

    if auth_user.is_pat() {
        // PAT callers cannot target agents they don't own.
        let repo = AgentRepository::new(db.clone());
        let agent = repo
            .find_by_id(xlstatus_shared::AgentId(
                uuid::Uuid::parse_str(agent_id)
                    .map_err(|_| bad_request_with("invalid agent_id".to_string()))?,
            ))
            .await
            .map_err(|e| internal_with(e.to_string()))?
            .ok_or_else(|| forbidden_with("agent not found".to_string()))?;
        if agent.owner_user_id != auth_user.user.id {
            return Err(forbidden_with(
                "agent is not owned by the calling user".to_string(),
            ));
        }
    } else if !auth_user.user.role.is_admin() {
        // Non-admin cookie users can only NAT their own agents.
        let repo = AgentRepository::new(db.clone());
        let agent = repo
            .find_by_id(xlstatus_shared::AgentId(
                uuid::Uuid::parse_str(agent_id)
                    .map_err(|_| bad_request_with("invalid agent_id".to_string()))?,
            ))
            .await
            .map_err(|e| internal_with(e.to_string()))?
            .ok_or_else(|| forbidden_with("agent not found".to_string()))?;
        if agent.owner_user_id != auth_user.user.id {
            return Err(forbidden_with(
                "agent is not owned by the calling user".to_string(),
            ));
        }
    }

    Ok(())
}

async fn require_nat_agent_or_403(
    db: &Db,
    auth_user: &AuthUser,
    agent_id: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    validate_nat_agent_or_403(db, auth_user, agent_id).await
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
