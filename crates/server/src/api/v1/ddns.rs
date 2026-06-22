#![allow(dead_code)]
#![allow(unused_imports)]

use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{require_sensitive_totp, AppError, AppState};
use crate::auth::middleware::{AuthKind, AuthSession, AuthUser};
use crate::auth::rbac::has_scope;
use crate::db::repository::ddns::{DdnsConfigRepository, DdnsConfigRow, DdnsHistoryRepository};
use crate::db::{AgentRepository, DatabaseBackend};
use crate::ddns::policy::{
    normalize_ddns_agent_id, normalize_ddns_provider, normalize_ddns_resource_uuid,
    normalize_optional_ddns_text, normalize_required_ddns_text, DDNS_API_MAX_BODY_BYTES,
    DDNS_MAX_DOMAIN_BYTES, DDNS_MAX_NAME_BYTES, DDNS_MAX_PROVIDER_BYTES, DDNS_MAX_RECORD_ID_BYTES,
    DDNS_MAX_SECRET_BYTES, DDNS_MAX_WEBHOOK_URL_BYTES, DDNS_MAX_ZONE_ID_BYTES, DDNS_UUID_TEXT_LEN,
};
use crate::security::validate_outbound_url;
use xlstatus_shared::ddns::{DdnsHistoryEntry, ProviderType};
use xlstatus_shared::AgentId;

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

#[derive(Debug)]
struct NormalizedDdnsConfigRequest {
    agent_id: Option<String>,
    name: String,
    provider: String,
    domain: String,
    record_id: Option<String>,
    zone_id: Option<String>,
    api_token: Option<String>,
    api_key: Option<String>,
    api_secret: Option<String>,
    webhook_url: Option<String>,
    enabled: bool,
}

pub fn ddns_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(DDNS_API_MAX_BODY_BYTES)
}

pub async fn create_ddns_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Json(req): Json<CreateDdnsConfigRequest>,
) -> Result<
    Json<ApiResponse<DdnsConfigResponse>>,
    (StatusCode, Json<ApiResponse<DdnsConfigResponse>>),
> {
    let db = state.db.clone();
    require_ddns_sensitive_admin(&db, &auth_user, &headers, "ddns:write").await?;
    let req = normalize_create_ddns_request(req)
        .map_err(|message| api_error::<DdnsConfigResponse>(StatusCode::BAD_REQUEST, message))?;
    ensure_ddns_agent_scope(&auth_user, req.agent_id.as_deref())?;
    ensure_ddns_agent_active_owner(&db, &auth_user, req.agent_id.as_deref()).await?;
    if req.provider == ProviderType::Webhook.as_str() {
        let url = req
            .webhook_url
            .as_deref()
            .expect("webhook_url is normalized as required for webhook provider");
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
        enabled: req.enabled,
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

fn normalize_create_ddns_request(
    req: CreateDdnsConfigRequest,
) -> Result<NormalizedDdnsConfigRequest, String> {
    let agent_id = normalize_ddns_agent_id(req.agent_id)?;
    let name = normalize_required_ddns_text(req.name, DDNS_MAX_NAME_BYTES, "name")?;
    let provider = normalize_ddns_provider(&req.provider)?;
    let domain = normalize_required_ddns_text(req.domain, DDNS_MAX_DOMAIN_BYTES, "domain")?;
    let record_id =
        normalize_optional_ddns_text(req.record_id, DDNS_MAX_RECORD_ID_BYTES, "record_id")?;
    let zone_id = normalize_optional_ddns_text(req.zone_id, DDNS_MAX_ZONE_ID_BYTES, "zone_id")?;
    let api_token =
        normalize_optional_ddns_text(req.api_token, DDNS_MAX_SECRET_BYTES, "api_token")?;
    let api_key = normalize_optional_ddns_text(req.api_key, DDNS_MAX_SECRET_BYTES, "api_key")?;
    let api_secret =
        normalize_optional_ddns_text(req.api_secret, DDNS_MAX_SECRET_BYTES, "api_secret")?;
    let webhook_url =
        normalize_optional_ddns_text(req.webhook_url, DDNS_MAX_WEBHOOK_URL_BYTES, "webhook_url")?;

    if provider == ProviderType::Webhook.as_str() && webhook_url.is_none() {
        return Err("webhook_url is required for webhook provider".to_string());
    }

    Ok(NormalizedDdnsConfigRequest {
        agent_id,
        name,
        provider,
        domain,
        record_id,
        zone_id,
        api_token,
        api_key,
        api_secret,
        webhook_url,
        enabled: req.enabled.unwrap_or(true),
    })
}

pub async fn delete_ddns_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiResponse<serde_json::Value>>)>
{
    require_ddns_sensitive_admin(&state.db, &auth_user, &headers, "ddns:delete").await?;
    let id = normalize_ddns_resource_uuid(id, "config_id")
        .map_err(|message| api_error::<serde_json::Value>(StatusCode::BAD_REQUEST, message))?;
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
    let config_id = normalize_ddns_resource_uuid(config_id, "config_id").map_err(|message| {
        api_error::<DdnsHistoryListResponse>(StatusCode::BAD_REQUEST, message)
    })?;
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
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiResponse<serde_json::Value>>)>
{
    require_ddns_sensitive_admin(&state.db, &auth_user, &headers, "ddns:write").await?;
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
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiResponse<serde_json::Value>>)>
{
    require_ddns_sensitive_admin(&state.db, &auth_user, &headers, "ddns:write").await?;
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

async fn require_ddns_sensitive_admin<T>(
    db: &DatabaseBackend,
    auth_user: &AuthUser,
    headers: &HeaderMap,
    scope: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<T>>)> {
    require_ddns_scope(auth_user, scope)?;
    if auth_user.is_pat() {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "DDNS sensitive action requires an admin cookie session",
        ));
    }
    require_sensitive_totp(db, auth_user.user.id, headers)
        .await
        .map_err(app_error_to_api)
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
    if ddns_server_id_allowed(allowed, agent_id) {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "DDNS config is outside PAT server allowlist",
        ))
    }
}

async fn ensure_ddns_agent_active_owner<T>(
    db: &crate::db::Db,
    auth_user: &AuthUser,
    agent_id: Option<&str>,
) -> Result<(), (StatusCode, Json<ApiResponse<T>>)> {
    let Some(agent_id) = agent_id else {
        return Ok(());
    };
    let agent_uuid = uuid::Uuid::parse_str(agent_id)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "agent_id must be a UUID"))?;
    let agent = AgentRepository::new(db.clone())
        .find_by_id(AgentId(agent_uuid))
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "agent not found"))?;
    if agent.owner_user_id != auth_user.user.id {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "DDNS config agent must be owned by the caller",
        ));
    }
    if agent.revoked_at.is_some() {
        return Err(api_error(StatusCode::FORBIDDEN, "agent has been revoked"));
    }
    Ok(())
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
        .map(|agent_id| ddns_server_id_allowed(allowed, agent_id))
        .unwrap_or(false)
}

fn ddns_server_id_allowed(allowed: &[String], agent_id: &str) -> bool {
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

fn app_error_to_api<T>(err: AppError) -> (StatusCode, Json<ApiResponse<T>>) {
    match err {
        AppError::Database(e) => {
            tracing::error!("Database error: {}", e);
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
        AppError::Unauthorized(message) => api_error(StatusCode::UNAUTHORIZED, message),
        AppError::Forbidden(message) => api_error(StatusCode::FORBIDDEN, message),
        AppError::BadRequest(message) => api_error(StatusCode::BAD_REQUEST, message),
        AppError::TooManyRequests(message) => api_error(StatusCode::TOO_MANY_REQUESTS, message),
        AppError::NotFound(message) => api_error(StatusCode::NOT_FOUND, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CreateAgentInput, CreateUserInput, DatabaseBackend, User, UserRepository};
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
        let allowed_server = uuid::Uuid::from_bytes([7; 16]).to_string();
        let denied_server = uuid::Uuid::from_bytes([8; 16]).to_string();
        let auth = admin_pat(vec!["ddns:read"], Some(vec![allowed_server.clone()]));

        assert!(ddns_config_visible_to_auth(
            &auth,
            &ddns_config(Some(&allowed_server))
        ));
        assert!(!ddns_config_visible_to_auth(
            &auth,
            &ddns_config(Some(&denied_server))
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

    #[tokio::test]
    async fn ddns_sensitive_actions_reject_admin_pat() {
        let db = test_db().await;
        let auth = admin_pat(vec!["ddns:write"], None);

        let err = require_ddns_sensitive_admin::<DdnsConfigResponse>(
            &db,
            &auth,
            &HeaderMap::new(),
            "ddns:write",
        )
        .await
        .unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn ddns_sensitive_actions_require_totp_when_enabled() {
        let db = test_db().await;
        let user_id = uuid::Uuid::from_bytes([1; 16]);
        seed_user(&db, user_id).await;
        seed_totp_enabled_user(&db, user_id).await;
        let auth = admin_cookie_for(UserId(user_id));

        let err = require_ddns_sensitive_admin::<DdnsConfigResponse>(
            &db,
            &auth,
            &HeaderMap::new(),
            "ddns:write",
        )
        .await
        .unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn ddns_sensitive_actions_allow_cookie_session_without_totp() {
        let db = test_db().await;
        let user_id = uuid::Uuid::from_bytes([1; 16]);
        seed_user(&db, user_id).await;
        let auth = admin_cookie_for(UserId(user_id));

        require_ddns_sensitive_admin::<DdnsConfigResponse>(
            &db,
            &auth,
            &HeaderMap::new(),
            "ddns:write",
        )
        .await
        .unwrap();
    }

    #[test]
    fn ddns_config_resource_limits_are_explicit() {
        assert_eq!(DDNS_API_MAX_BODY_BYTES, 64 * 1024);
        assert_eq!(DDNS_MAX_NAME_BYTES, 128);
        assert_eq!(DDNS_MAX_PROVIDER_BYTES, 64);
        assert_eq!(DDNS_MAX_DOMAIN_BYTES, 253);
        assert_eq!(DDNS_UUID_TEXT_LEN, 36);
        assert_eq!(DDNS_MAX_RECORD_ID_BYTES, 128);
        assert_eq!(DDNS_MAX_ZONE_ID_BYTES, 128);
        assert_eq!(DDNS_MAX_SECRET_BYTES, 4096);
        assert_eq!(DDNS_MAX_WEBHOOK_URL_BYTES, 2048);
    }

    #[test]
    fn ddns_provider_is_allowlisted_and_canonicalized() {
        assert_eq!(normalize_ddns_provider(" webhook ").unwrap(), "webhook");
        assert_eq!(
            normalize_ddns_provider("tencent_cloud").unwrap(),
            "tencent_cloud"
        );
        assert!(normalize_ddns_provider("route53").is_err());
        assert!(normalize_ddns_provider(&"a".repeat(DDNS_MAX_PROVIDER_BYTES + 1)).is_err());
    }

    #[test]
    fn ddns_agent_and_config_ids_require_canonical_uuid_text() {
        let id = uuid::Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();

        assert_eq!(
            normalize_ddns_agent_id(Some(id.to_string())).unwrap(),
            Some(id.to_string())
        );
        assert_eq!(
            normalize_ddns_resource_uuid(id.to_string(), "config_id").unwrap(),
            id.to_string()
        );
        assert!(normalize_ddns_agent_id(Some("server-a".into())).is_err());
        assert!(normalize_ddns_agent_id(Some(format!(" {} ", id))).is_err());
        assert!(normalize_ddns_agent_id(Some(id.simple().to_string())).is_err());
        assert!(normalize_ddns_agent_id(Some(id.to_string().to_uppercase())).is_err());
        assert!(normalize_ddns_agent_id(Some("a".repeat(DDNS_UUID_TEXT_LEN + 1))).is_err());
        assert!(normalize_ddns_resource_uuid(id.simple().to_string(), "config_id").is_err());
    }

    #[test]
    fn ddns_server_allowlist_uses_uuid_semantics() {
        let id = uuid::Uuid::from_bytes([5; 16]);

        assert!(ddns_server_id_allowed(
            &[id.simple().to_string()],
            &id.to_string()
        ));
        assert!(!ddns_server_id_allowed(
            &[uuid::Uuid::from_bytes([6; 16]).to_string()],
            &id.to_string()
        ));
        assert!(!ddns_server_id_allowed(
            &["server-a".into()],
            &id.to_string()
        ));
    }

    #[test]
    fn ddns_config_rejects_oversized_fields_and_requires_webhook_url() {
        assert!(normalize_required_ddns_text("cfg".into(), DDNS_MAX_NAME_BYTES, "name").is_ok());
        assert!(normalize_required_ddns_text(
            "a".repeat(DDNS_MAX_NAME_BYTES + 1),
            DDNS_MAX_NAME_BYTES,
            "name"
        )
        .is_err());
        assert!(normalize_required_ddns_text(
            "a".repeat(DDNS_MAX_DOMAIN_BYTES + 1),
            DDNS_MAX_DOMAIN_BYTES,
            "domain"
        )
        .is_err());
        assert!(normalize_optional_ddns_text(
            Some("a".repeat(DDNS_MAX_SECRET_BYTES + 1)),
            DDNS_MAX_SECRET_BYTES,
            "api_secret"
        )
        .is_err());
        assert!(normalize_optional_ddns_text(
            Some(format!(
                "https://example.com/{}",
                "a".repeat(DDNS_MAX_WEBHOOK_URL_BYTES)
            )),
            DDNS_MAX_WEBHOOK_URL_BYTES,
            "webhook_url"
        )
        .is_err());

        let mut req = ddns_create_request();
        req.provider = "webhook".into();
        req.webhook_url = None;
        assert!(normalize_create_ddns_request(req).is_err());
    }

    #[test]
    fn ddns_config_normalization_trims_and_defaults_enabled() {
        let mut req = ddns_create_request();
        let agent_id = uuid::Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
        req.agent_id = Some(agent_id.to_string());
        req.name = " cfg ".into();
        req.provider = " cloudflare ".into();
        req.domain = " example.com ".into();
        req.record_id = Some(" rec ".into());
        req.zone_id = Some(" ".into());
        req.api_token = Some(" token ".into());
        req.enabled = None;

        let normalized = normalize_create_ddns_request(req).unwrap();

        assert_eq!(normalized.agent_id, Some(agent_id.to_string()));
        assert_eq!(normalized.name, "cfg");
        assert_eq!(normalized.provider, "cloudflare");
        assert_eq!(normalized.domain, "example.com");
        assert_eq!(normalized.record_id, Some("rec".into()));
        assert_eq!(normalized.zone_id, None);
        assert_eq!(normalized.api_token, Some("token".into()));
        assert!(normalized.enabled);
    }

    #[tokio::test]
    async fn ddns_create_requires_active_owned_agent() {
        let db = test_db().await;
        let user_repo = UserRepository::new(db.clone());
        let owner = user_repo
            .create(CreateUserInput {
                username: format!("ddns-api-owner-{}", uuid::Uuid::now_v7()),
                password: "password123".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let other_owner = user_repo
            .create(CreateUserInput {
                username: format!("ddns-api-other-{}", uuid::Uuid::now_v7()),
                password: "password123".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let agent_repo = AgentRepository::new(db.clone());
        let active_agent = agent_repo
            .create_with_id(
                AgentId(uuid::Uuid::now_v7()),
                CreateAgentInput {
                    name: "active-ddns-agent".into(),
                    public_key: "pk".into(),
                    owner_user_id: owner.id,
                },
            )
            .await
            .unwrap();
        let revoked_agent = agent_repo
            .create_with_id(
                AgentId(uuid::Uuid::now_v7()),
                CreateAgentInput {
                    name: "revoked-ddns-agent".into(),
                    public_key: "pk".into(),
                    owner_user_id: owner.id,
                },
            )
            .await
            .unwrap();
        agent_repo.revoke(revoked_agent.id).await.unwrap();
        let foreign_agent = agent_repo
            .create_with_id(
                AgentId(uuid::Uuid::now_v7()),
                CreateAgentInput {
                    name: "foreign-ddns-agent".into(),
                    public_key: "pk".into(),
                    owner_user_id: other_owner.id,
                },
            )
            .await
            .unwrap();
        let auth = admin_cookie_for(owner.id);

        assert!(ensure_ddns_agent_active_owner::<DdnsConfigResponse>(
            &db,
            &auth,
            Some(&active_agent.id.0.to_string())
        )
        .await
        .is_ok());

        let err = ensure_ddns_agent_active_owner::<DdnsConfigResponse>(
            &db,
            &auth,
            Some(&revoked_agent.id.0.to_string()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);

        let err = ensure_ddns_agent_active_owner::<DdnsConfigResponse>(
            &db,
            &auth,
            Some(&foreign_agent.id.0.to_string()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);

        let err = ensure_ddns_agent_active_owner::<DdnsConfigResponse>(
            &db,
            &auth,
            Some(&uuid::Uuid::now_v7().to_string()),
        )
        .await
        .unwrap_err();
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

    fn admin_cookie_for(user_id: UserId) -> AuthUser {
        let now = Utc::now();
        AuthUser {
            user: User {
                id: user_id,
                username: "admin".into(),
                password_hash: "x".into(),
                role: UserRole::Admin,
                token_version: 0,
                created_at: now,
                updated_at: now,
            },
            session_id: "cookie-session".into(),
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: Vec::new(),
            server_ids: None,
            pat_id: None,
        }
    }

    async fn test_db() -> DatabaseBackend {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-ddns-api-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_user(db: &DatabaseBackend, id: uuid::Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO users (id, username, password_hash, role, token_version, created_at, updated_at)
            VALUES (?, ?, ?, ?, 0, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(format!("ddns-sensitive-{id}"))
        .bind("hash")
        .bind(UserRole::Admin.to_string())
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_totp_enabled_user(db: &DatabaseBackend, id: uuid::Uuid) {
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

    fn ddns_create_request() -> CreateDdnsConfigRequest {
        CreateDdnsConfigRequest {
            agent_id: None,
            name: "cfg".into(),
            provider: "cloudflare".into(),
            domain: "example.com".into(),
            record_id: None,
            zone_id: None,
            api_token: None,
            api_key: None,
            api_secret: None,
            webhook_url: None,
            enabled: Some(true),
        }
    }
}
