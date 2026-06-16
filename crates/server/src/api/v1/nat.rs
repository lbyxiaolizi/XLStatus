use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::api::types::ApiResponse;
use crate::auth::middleware::AuthUser;
use crate::db::repository::NatMappingRepository;
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
    State(db): State<Db>,
    AuthUser { user, .. }: AuthUser,
    Json(req): Json<CreateNatMappingRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(mapping_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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

    Ok(Json(ApiResponse {
        success: true,
        data: Some(NatMappingResponse { mapping }),
        error: None,
    }))
}

/// List NAT mappings for an agent
pub async fn list_nat_mappings(
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(agent_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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
    State(db): State<Db>,
    auth_user: AuthUser,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let mappings = NatMappingRepository::list_enabled(&db)
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

/// Update a NAT mapping
pub async fn update_nat_mapping(
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(mapping_id): Path<String>,
    Json(req): Json<UpdateNatMappingRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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

    Ok(Json(ApiResponse {
        success: true,
        data: Some(NatMappingResponse { mapping }),
        error: None,
    }))
}

/// Delete a NAT mapping
pub async fn delete_nat_mapping(
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(mapping_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    // Check if mapping exists
    let _ = NatMappingRepository::get_by_id(&db, &mapping_id)
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

    Ok((
        StatusCode::OK,
        Json(ApiResponse::<()> {
            success: true,
            data: None,
            error: None,
        }),
    ))
}
