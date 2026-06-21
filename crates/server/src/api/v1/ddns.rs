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
use crate::auth::middleware::{AuthKind, AuthSession, AuthUser};
use crate::auth::rbac::has_scope;
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
    pub config: DdnsConfigView,
}

#[derive(Debug, Serialize)]
pub struct DdnsConfigListResponse {
    pub configs: Vec<DdnsConfigView>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct DdnsConfigView {
    pub id: String,
    pub owner_user_id: String,
    pub agent_id: Option<String>,
    pub name: String,
    pub provider: String,
    pub domain: String,
    pub record_id: Option<String>,
    pub zone_id: Option<String>,
    pub api_token_configured: bool,
    pub api_key_configured: bool,
    pub api_secret_configured: bool,
    pub webhook_url_configured: bool,
    pub current_ip: Option<String>,
    pub last_applied_ip: Option<String>,
    pub last_applied_at: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
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
    require_ddns_scope(&auth_user, "ddns:write")?;
    ensure_ddns_agent_scope(&auth_user, req.agent_id.as_deref())?;
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
        data: Some(DdnsConfigResponse {
            config: DdnsConfigView::from_row(&row),
        }),
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
    require_ddns_scope(&auth_user, "ddns:read")?;
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
    let configs: Vec<_> = configs
        .into_iter()
        .filter(|config| ddns_config_visible_to_auth(&auth_user, config))
        .collect();
    Ok(Json(ApiResponse {
        success: true,
        data: Some(DdnsConfigListResponse {
            total: configs.len(),
            configs: configs.iter().map(DdnsConfigView::from_row).collect(),
        }),
        error: None,
    }))
}

impl DdnsConfigView {
    fn from_row(row: &DdnsConfigRow) -> Self {
        Self {
            id: row.id.clone(),
            owner_user_id: row.owner_user_id.clone(),
            agent_id: row.agent_id.clone(),
            name: row.name.clone(),
            provider: row.provider.clone(),
            domain: row.domain.clone(),
            record_id: row.record_id.clone(),
            zone_id: row.zone_id.clone(),
            api_token_configured: has_secret(&row.api_token),
            api_key_configured: has_secret(&row.api_key),
            api_secret_configured: has_secret(&row.api_secret),
            webhook_url_configured: has_secret(&row.webhook_url),
            current_ip: row.current_ip.clone(),
            last_applied_ip: row.last_applied_ip.clone(),
            last_applied_at: row.last_applied_at.clone(),
            enabled: row.enabled,
            created_at: row.created_at.clone(),
            updated_at: row.updated_at.clone(),
        }
    }
}

fn has_secret(value: &Option<String>) -> bool {
    value
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

pub async fn delete_ddns_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiResponse<serde_json::Value>>)>
{
    require_ddns_scope_json(&auth_user, "ddns:delete")?;
    let Some(config) = DdnsConfigRepository::get_by_id(&state.db, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to load DDNS config: {}", e)),
                }),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("DDNS config not found".to_string()),
            }),
        ));
    };
    ensure_ddns_config_visible_json(&auth_user, &config)?;
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
    require_ddns_scope(&auth_user, "ddns:read")?;
    let Some(config) = DdnsConfigRepository::get_by_id(&state.db, &config_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to load DDNS config: {}", e)),
                }),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("DDNS config not found".to_string()),
            }),
        ));
    };
    ensure_ddns_config_visible(&auth_user, &config)?;
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
    require_ddns_global_admin(&auth_user, "ddns:write")?;
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
    require_ddns_global_admin(&auth_user, "ddns:write")?;
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

fn require_ddns_scope<T>(
    auth_user: &AuthUser,
    scope: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<T>>)> {
    if !auth_user.user.role.is_admin() {
        return Err(api_error(StatusCode::FORBIDDEN, "admin role required"));
    }
    if auth_user.is_pat() && !has_scope(&auth_session(auth_user), scope) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            format!("missing required scope: {scope}"),
        ));
    }
    Ok(())
}

fn require_ddns_scope_json(
    auth_user: &AuthUser,
    scope: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<serde_json::Value>>)> {
    require_ddns_scope(auth_user, scope)
}

fn require_ddns_global_admin(
    auth_user: &AuthUser,
    scope: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<serde_json::Value>>)> {
    require_ddns_scope_json(auth_user, scope)?;
    if auth_user.is_pat() {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "DDNS global action requires an admin cookie session",
        ));
    }
    Ok(())
}

fn ensure_ddns_agent_scope<T>(
    auth_user: &AuthUser,
    agent_id: Option<&str>,
) -> Result<(), (StatusCode, Json<ApiResponse<T>>)> {
    let Some(allowed) = auth_user.server_ids.as_ref() else {
        return Ok(());
    };
    let Some(agent_id) = agent_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "PAT-scoped DDNS configs must target a server in the allowlist",
        ));
    };
    if allowed.iter().any(|allowed_id| allowed_id == agent_id) {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "DDNS config is outside PAT server allowlist",
        ))
    }
}

fn ensure_ddns_config_visible<T>(
    auth_user: &AuthUser,
    config: &DdnsConfigRow,
) -> Result<(), (StatusCode, Json<ApiResponse<T>>)> {
    if ddns_config_visible_to_auth(auth_user, config) {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "DDNS config is outside PAT server allowlist",
        ))
    }
}

fn ensure_ddns_config_visible_json(
    auth_user: &AuthUser,
    config: &DdnsConfigRow,
) -> Result<(), (StatusCode, Json<ApiResponse<serde_json::Value>>)> {
    ensure_ddns_config_visible(auth_user, config)
}

fn ddns_config_visible_to_auth(auth_user: &AuthUser, config: &DdnsConfigRow) -> bool {
    let Some(allowed) = auth_user.server_ids.as_ref() else {
        return true;
    };
    config
        .agent_id
        .as_ref()
        .map(|agent_id| allowed.iter().any(|allowed_id| allowed_id == agent_id))
        .unwrap_or(false)
}

fn auth_session(auth_user: &AuthUser) -> AuthSession {
    AuthSession {
        session_id: auth_user.session_id.clone(),
        user_id: auth_user.user.id,
        username: auth_user.user.username.clone(),
        role: auth_user.user.role,
        csrf_token: auth_user.csrf_token.clone(),
        auth_kind: AuthKind::PersonalAccessToken,
        scopes: auth_user.scopes.clone(),
        server_ids: auth_user.server_ids.clone(),
        pat_id: auth_user.pat_id.clone(),
    }
}

fn api_error<T>(
    status: StatusCode,
    message: impl Into<String>,
) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        status,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some(message.into()),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::User;
    use chrono::Utc;
    use xlstatus_shared::{UserId, UserRole};

    #[test]
    fn admin_pat_without_ddns_scope_is_rejected() {
        let auth = admin_pat(vec!["server:read"], None);

        let err = require_ddns_scope::<DdnsConfigListResponse>(&auth, "ddns:read").unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn admin_pat_ddns_visibility_respects_server_allowlist() {
        let auth = admin_pat(vec!["ddns:read"], Some(vec!["server-a".into()]));

        assert!(ddns_config_visible_to_auth(
            &auth,
            &ddns_config(Some("server-a"))
        ));
        assert!(!ddns_config_visible_to_auth(
            &auth,
            &ddns_config(Some("server-b"))
        ));
        assert!(!ddns_config_visible_to_auth(&auth, &ddns_config(None)));
    }

    #[test]
    fn scoped_admin_pat_cannot_create_unbound_ddns_config() {
        let auth = admin_pat(vec!["ddns:write"], Some(vec!["server-a".into()]));

        let err = ensure_ddns_agent_scope::<DdnsConfigResponse>(&auth, None).unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn scoped_admin_pat_cannot_run_ddns_global_actions() {
        let auth = admin_pat(vec!["ddns:write"], Some(vec!["server-a".into()]));

        let err = require_ddns_global_admin(&auth, "ddns:write").unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn unscoped_admin_pat_cannot_run_ddns_global_actions() {
        let auth = admin_pat(vec!["ddns:write"], None);

        let err = require_ddns_global_admin(&auth, "ddns:write").unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    fn admin_pat(scopes: Vec<&str>, server_ids: Option<Vec<String>>) -> AuthUser {
        let now = Utc::now();
        AuthUser {
            user: User {
                id: UserId(uuid::Uuid::from_bytes([1; 16])),
                username: "admin".into(),
                password_hash: "x".into(),
                role: UserRole::Admin,
                token_version: 0,
                created_at: now,
                updated_at: now,
            },
            session_id: "pat-session".into(),
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::PersonalAccessToken,
            scopes: scopes.into_iter().map(str::to_string).collect(),
            server_ids,
            pat_id: Some("pat".into()),
        }
    }

    fn ddns_config(agent_id: Option<&str>) -> DdnsConfigRow {
        DdnsConfigRow {
            id: "config".into(),
            owner_user_id: uuid::Uuid::from_bytes([1; 16]).to_string(),
            agent_id: agent_id.map(str::to_string),
            name: "cfg".into(),
            provider: "webhook".into(),
            domain: "example.com".into(),
            record_id: None,
            zone_id: None,
            api_token: None,
            api_key: None,
            api_secret: None,
            webhook_url: None,
            current_ip: None,
            last_applied_ip: None,
            last_applied_at: None,
            enabled: true,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }
}
