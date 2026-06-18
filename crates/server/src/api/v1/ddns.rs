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
use crate::auth::middleware::AuthUser;
use crate::db::repository::ddns::{DdnsConfigRepository, DdnsConfigRow, DdnsHistoryRepository};
use crate::security::validate_outbound_url;
use xlstatus_shared::ddns::DdnsHistoryEntry;

#[derive(Debug, Deserialize)]
pub struct CreateDdnsConfigRequest {
    pub agent_id: Option<String>,
    pub name: String,
    pub provider: String,
    pub domain: String,
    pub record_id: Option<String>,
    pub zone_id: Option<String>,
    pub api_token: Option<String>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub webhook_url: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DdnsConfigResponse {
    pub config: DdnsConfigRow,
}

#[derive(Debug, Serialize)]
pub struct DdnsConfigListResponse {
    pub configs: Vec<DdnsConfigRow>,
    pub total: usize,
}

pub async fn create_ddns_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreateDdnsConfigRequest>,
) -> Result<
    Json<ApiResponse<DdnsConfigResponse>>,
    (StatusCode, Json<ApiResponse<DdnsConfigResponse>>),
> {
    let db = state.db.clone();
    if !auth_user.user.role.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("admin role required".to_string()),
            }),
        ));
    }
    if req.provider == "webhook" {
        let Some(url) = req.webhook_url.as_deref() else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("webhook_url is required for webhook provider".to_string()),
                }),
            ));
        };
        let probe_url = url
            .replace("{{ip}}", "198.51.100.10")
            .replace("{{hostname}}", "example.com");
        if let Err(e) = validate_outbound_url(&probe_url, "DDNS webhook").await {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(e.to_string()),
                }),
            ));
        }
    }
    let now = Utc::now().to_rfc3339();
    let row = DdnsConfigRow {
        id: uuid::Uuid::now_v7().to_string(),
        owner_user_id: auth_user.user.id.0.to_string(),
        agent_id: req.agent_id,
        name: req.name,
        provider: req.provider,
        domain: req.domain,
        record_id: req.record_id,
        zone_id: req.zone_id,
        api_token: req.api_token,
        api_key: req.api_key,
        api_secret: req.api_secret,
        webhook_url: req.webhook_url,
        current_ip: None,
        last_applied_ip: None,
        last_applied_at: None,
        enabled: req.enabled.unwrap_or(true),
        created_at: now.clone(),
        updated_at: now,
    };
    if let Err(e) = DdnsConfigRepository::create(&db, &row).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to create DDNS config: {}", e)),
            }),
        ));
    }
    Ok(Json(ApiResponse {
        success: true,
        data: Some(DdnsConfigResponse { config: row }),
        error: None,
    }))
}

pub async fn list_ddns_configs(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<
    Json<ApiResponse<DdnsConfigListResponse>>,
    (StatusCode, Json<ApiResponse<DdnsConfigListResponse>>),
> {
    let db = state.db.clone();
    if !auth_user.user.role.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("admin role required".to_string()),
            }),
        ));
    }
    // For M6 we just return enabled rows; an admin-scoped
    // all-rows listing can be added later if needed.
    let configs = DdnsConfigRepository::list_enabled(&db).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to list DDNS configs: {}", e)),
            }),
        )
    })?;
    Ok(Json(ApiResponse {
        success: true,
        data: Some(DdnsConfigListResponse {
            total: configs.len(),
            configs,
        }),
        error: None,
    }))
}

pub async fn delete_ddns_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiResponse<serde_json::Value>>)>
{
    if !auth_user.user.role.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("admin role required".to_string()),
            }),
        ));
    }
    DdnsConfigRepository::delete(&state.db, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to delete DDNS config: {}", e)),
                }),
            )
        })?;
    Ok(Json(ApiResponse {
        success: true,
        data: Some(serde_json::json!({"id": id})),
        error: None,
    }))
}

#[derive(Debug, Serialize)]
pub struct DdnsHistoryListResponse {
    pub history: Vec<DdnsHistoryEntry>,
    pub total: usize,
}

pub async fn list_ddns_history(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(config_id): Path<String>,
) -> Result<
    Json<ApiResponse<DdnsHistoryListResponse>>,
    (StatusCode, Json<ApiResponse<DdnsHistoryListResponse>>),
> {
    if !auth_user.user.role.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("admin role required".to_string()),
            }),
        ));
    }
    let history = DdnsHistoryRepository::list_for_config(&state.db, &config_id, 50)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to list DDNS history: {}", e)),
                }),
            )
        })?;
    Ok(Json(ApiResponse {
        success: true,
        data: Some(DdnsHistoryListResponse {
            total: history.len(),
            history,
        }),
        error: None,
    }))
}

/// M6: hot-reload DDNS providers from the database. Useful after
/// adding a new config without restarting the server.
pub async fn reload_ddns_providers(
    State(_state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiResponse<serde_json::Value>>)>
{
    if !auth_user.user.role.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("admin role required".to_string()),
            }),
        ));
    }
    let mgr = match crate::current_ddns_manager() {
        Some(m) => m,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("DDNS manager not running".to_string()),
                }),
            ));
        }
    };
    if let Err(e) = mgr.reload_providers().await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Reload failed: {}", e)),
            }),
        ));
    }
    Ok(Json(ApiResponse {
        success: true,
        data: Some(serde_json::json!({"reloaded": true})),
        error: None,
    }))
}

/// M6: run one DDNS provider check immediately.
pub async fn check_ddns_now(
    State(_state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiResponse<serde_json::Value>>)>
{
    if !auth_user.user.role.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("admin role required".to_string()),
            }),
        ));
    }
    let mgr = match crate::current_ddns_manager() {
        Some(m) => m,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("DDNS manager not running".to_string()),
                }),
            ));
        }
    };
    if let Err(e) = mgr.check_now().await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("DDNS check failed: {}", e)),
            }),
        ));
    }
    Ok(Json(ApiResponse {
        success: true,
        data: Some(serde_json::json!({"checked": true})),
        error: None,
    }))
}
