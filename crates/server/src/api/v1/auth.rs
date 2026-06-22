use crate::api::types::*;
use crate::auth::middleware::{derive_csrf_token, AuthUser, CSRF_COOKIE_NAME, SESSION_COOKIE_NAME};
use crate::auth::totp::{generate_totp_secret, otpauth_uri, verify_totp_code};
use crate::auth::{generate_session_token, hash_token};
use crate::config::Config;
use crate::db::{
    CreateSessionInput, CreateUserInput, DatabaseBackend, PATRepository, UserRepository,
};
use axum::{
    extract::{connect_info::ConnectInfo, DefaultBodyLimit, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{AppendHeaders, IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio::sync::RwLock;
use xlstatus_shared::{UserId, UserRole};

pub type AgentJwtChallengeStore = Arc<RwLock<HashMap<String, DateTime<Utc>>>>;

const LOGIN_FAILURE_THRESHOLD: i64 = 5;
const LOGIN_FAILURE_WINDOW_MINUTES: i64 = 15;
const LOGIN_BAN_MINUTES: i64 = 30;
pub(crate) const SENSITIVE_TOTP_HEADER: &str = "x-totp-code";
const AUTH_API_MAX_BODY_BYTES: usize = 64 * 1024;
const AUTH_LOGIN_MAX_BODY_BYTES: usize = 4 * 1024;
const AUTH_TOTP_MAX_BODY_BYTES: usize = 1024;
const AUTH_MAX_USERNAME_BYTES: usize = 128;
const AUTH_MIN_PASSWORD_BYTES: usize = 8;
const AUTH_MAX_PASSWORD_BYTES: usize = 1024;
const AUTH_MAX_ROLE_BYTES: usize = 32;
const AUTH_TOTP_CODE_BYTES: usize = 6;
const AUTH_MAX_WAF_BAN_IPS: usize = 128;
const AUTH_MAX_WAF_IP_FIELD_BYTES: usize = 4096;
const AUTH_MAX_WAF_REASON_BYTES: usize = 255;
const AUTH_MAX_WAF_BAN_MINUTES: i64 = 43_200;
const AUTH_UUID_TEXT_LEN: usize = 36;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseBackend,
    pub config: Arc<Config>,
    pub agent_jwt_challenges: AgentJwtChallengeStore,
    /// M3: time-series store for HostState samples. Wraps an
    /// in-memory backend today; a real TSDB drops in for M8.
    pub metrics: xlstatus_tsdb::MetricStore,
    /// M3: in-process pub/sub for live HostState events consumed by
    /// the `/ws/servers` WebSocket route.
    pub realtime: crate::realtime::BroadcastHub,
    /// M5: registry of live agent gRPC sessions. The tasks API
    /// uses this to dispatch `ServerMessage::Task` to a specific
    /// agent and wait for the matching `TaskResult` reply.
    pub session_registry: crate::grpc::SessionRegistry,
    /// M5: terminal session metadata keyed by session id.
    pub terminal_sessions: crate::api::v1::terminal::TerminalSessionRegistry,
    /// M5: live agent IO senders keyed by agent id.
    pub io_registry: crate::grpc::IoRegistry,
}

pub fn auth_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(AUTH_API_MAX_BODY_BYTES)
}

pub fn login_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(AUTH_LOGIN_MAX_BODY_BYTES)
}

pub fn totp_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(AUTH_TOTP_MAX_BODY_BYTES)
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Response, AppError> {
    let username = normalize_username(req.username)?;
    let password = normalize_password(req.password)?;
    let totp_code = normalize_optional_totp_code(req.totp_code)?;
    let client_ip = crate::security::client_ip_from_headers_and_peer(&headers, Some(peer_addr));
    let user_agent = header_value(&headers, header::USER_AGENT.as_str());
    if active_waf_ban(&state.db, &client_ip).await?.is_some() {
        record_waf_event(
            &state.db,
            &client_ip,
            Some(&username),
            "login_blocked",
            Some("active WAF ban"),
        )
        .await?;
        return Err(AppError::Forbidden(
            "IP temporarily blocked by WAF".to_string(),
        ));
    }

    let user_repo = UserRepository::new(state.db.clone());

    // Find user by username
    let Some(user) = user_repo.find_by_username(&username).await? else {
        register_login_failure(&state.db, &client_ip, &username, "unknown user").await?;
        return Err(AppError::Unauthorized("Invalid credentials".to_string()));
    };

    // Verify password
    if !user_repo.verify_password(&user, &password)? {
        register_login_failure(&state.db, &client_ip, &username, "invalid password").await?;
        return Err(AppError::Unauthorized("Invalid credentials".to_string()));
    }
    let (totp_secret, totp_enabled) = user_repo.totp_config(user.id).await?;
    if totp_enabled {
        let Some(totp_code) = totp_code.as_deref() else {
            return Ok(Json(ApiResponse::success(LoginResponse {
                user: None,
                mfa_required: true,
            }))
            .into_response());
        };
        let Some(secret) = totp_secret.as_deref() else {
            return Err(AppError::Unauthorized("Invalid credentials".to_string()));
        };
        if !verify_totp_code(secret, totp_code, Utc::now()) {
            register_login_failure(&state.db, &client_ip, &username, "invalid totp").await?;
            return Err(AppError::Unauthorized("Invalid TOTP code".to_string()));
        }
    }

    // Generate session token
    let session_token = generate_session_token();
    let token_hash = hash_token(&session_token);

    // Create session
    let session_repo = crate::auth::SessionRepository::new(state.db.clone());
    let expires_at = Utc::now() + Duration::hours(state.config.security.session_ttl_hours);

    session_repo
        .create(
            CreateSessionInput {
                user_id: user.id,
                ip: Some(client_ip.clone()),
                user_agent: user_agent.clone(),
                expires_at,
            },
            token_hash.clone(),
        )
        .await?;
    record_waf_event(
        &state.db,
        &client_ip,
        Some(&username),
        "login_success",
        None,
    )
    .await?;

    let csrf_token = derive_csrf_token(&token_hash);
    let secure_attr = cookie_secure_attr(state.config.security.cookie_secure);
    let session_cookie = format!(
        "{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}{}",
        SESSION_COOKIE_NAME,
        session_token,
        state.config.security.session_ttl_hours * 3600,
        secure_attr
    );
    let session_cookie = HeaderValue::from_str(&session_cookie)
        .map_err(|e| AppError::BadRequest(format!("Invalid session cookie: {}", e)))?;
    let csrf_cookie = format!(
        "{}={}; SameSite=Lax; Path=/; Max-Age={}{}",
        CSRF_COOKIE_NAME,
        csrf_token,
        state.config.security.session_ttl_hours * 3600,
        secure_attr
    );
    let csrf_cookie = HeaderValue::from_str(&csrf_cookie)
        .map_err(|e| AppError::BadRequest(format!("Invalid session cookie: {}", e)))?;

    Ok((
        AppendHeaders([
            (header::SET_COOKIE, session_cookie),
            (header::SET_COOKIE, csrf_cookie),
        ]),
        Json(ApiResponse::success(LoginResponse {
            user: Some(UserInfo {
                id: user.id.0.to_string(),
                username: user.username,
                role: user.role.to_string(),
                created_at: None,
                updated_at: None,
            }),
            mfa_required: false,
        })),
    )
        .into_response())
}

pub async fn logout(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let session_repo = crate::auth::SessionRepository::new(state.db.clone());
    session_repo.delete(&auth_user.session_id).await?;
    let secure_attr = cookie_secure_attr(state.config.security.cookie_secure);
    let session_cookie = format!(
        "{}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{}",
        SESSION_COOKIE_NAME, secure_attr
    );
    let session_cookie = HeaderValue::from_str(&session_cookie)
        .map_err(|e| AppError::BadRequest(format!("Invalid session cookie: {}", e)))?;
    let csrf_cookie = format!(
        "{}=; SameSite=Lax; Path=/; Max-Age=0{}",
        CSRF_COOKIE_NAME, secure_attr
    );
    let csrf_cookie = HeaderValue::from_str(&csrf_cookie)
        .map_err(|e| AppError::BadRequest(format!("Invalid session cookie: {}", e)))?;

    Ok((
        AppendHeaders([
            (header::SET_COOKIE, session_cookie),
            (header::SET_COOKIE, csrf_cookie),
        ]),
        Json(ApiResponse::success(())),
    ))
}

pub async fn create_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    require_admin_cookie_session(&auth_user)?;
    require_sensitive_totp(&state.db, auth_user.user.id, &headers).await?;

    let user_repo = UserRepository::new(state.db.clone());
    let username = normalize_username(req.username)?;
    let password = normalize_new_password(req.password)?;
    let role = normalize_role(req.role)?;

    // Create user
    let user = user_repo
        .create(CreateUserInput {
            username,
            password,
            role,
        })
        .await?;

    Ok(Json(ApiResponse::success(UserInfo {
        id: user.id.0.to_string(),
        username: user.username,
        role: user.role.to_string(),
        created_at: Some(user.created_at.to_rfc3339()),
        updated_at: Some(user.updated_at.to_rfc3339()),
    })))
}

#[derive(Debug, Deserialize)]
pub struct ListUsersQuery {
    #[serde(default = "default_user_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_user_limit() -> i64 {
    100
}

pub async fn list_users(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<ListUsersQuery>,
) -> Result<Json<ApiResponse<ListUsersResponse>>, AppError> {
    require_admin_cookie_session(&auth_user)?;
    let user_repo = UserRepository::new(state.db.clone());
    let (users, total) = user_repo
        .list(q.limit.clamp(1, 500), q.offset.max(0))
        .await?;
    Ok(Json(ApiResponse::success(ListUsersResponse {
        users: users.into_iter().map(user_info).collect(),
        total,
    })))
}

pub async fn update_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    require_admin_cookie_session(&auth_user)?;
    require_sensitive_totp(&state.db, auth_user.user.id, &headers).await?;
    let target_id = parse_user_id(&id)?;
    let user_repo = UserRepository::new(state.db.clone());
    let existing = user_repo
        .find_by_id(target_id)
        .await?
        .ok_or(AppError::NotFound("user not found".to_string()))?;

    if let Some(role) = req.role {
        let next_role = normalize_role(role)?;
        if existing.role.is_admin() && !next_role.is_admin() {
            if existing.id == auth_user.user.id {
                return Err(AppError::BadRequest(
                    "cannot demote your own admin account".to_string(),
                ));
            }
            if user_repo.count_admins().await? <= 1 {
                return Err(AppError::BadRequest(
                    "cannot demote the last admin user".to_string(),
                ));
            }
        }
        user_repo
            .update_role(target_id, next_role)
            .await?
            .ok_or(AppError::NotFound("user not found".to_string()))?;
    }

    if let Some(password) = req.password {
        let password = normalize_new_password(password)?;
        user_repo
            .reset_password(target_id, &password)
            .await?
            .ok_or(AppError::NotFound("user not found".to_string()))?;
        revoke_user_credentials(&state.db, target_id).await?;
    }

    let updated = user_repo
        .find_by_id(target_id)
        .await?
        .ok_or(AppError::NotFound("user not found".to_string()))?;
    Ok(Json(ApiResponse::success(user_info(updated))))
}

pub async fn delete_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    require_admin_cookie_session(&auth_user)?;
    require_sensitive_totp(&state.db, auth_user.user.id, &headers).await?;
    let target_id = parse_user_id(&id)?;
    if target_id == auth_user.user.id {
        return Err(AppError::BadRequest(
            "cannot delete your own account".to_string(),
        ));
    }
    let user_repo = UserRepository::new(state.db.clone());
    let existing = user_repo
        .find_by_id(target_id)
        .await?
        .ok_or(AppError::NotFound("user not found".to_string()))?;
    if existing.role.is_admin() && user_repo.count_admins().await? <= 1 {
        return Err(AppError::BadRequest(
            "cannot delete the last admin user".to_string(),
        ));
    }
    if !user_repo.delete(target_id).await? {
        return Err(AppError::NotFound("user not found".to_string()));
    }
    Ok(Json(ApiResponse::success(())))
}

#[derive(Debug, Serialize)]
pub struct TotpStatusResponse {
    pub enabled: bool,
    pub setup_pending: bool,
}

#[derive(Debug, Serialize)]
pub struct TotpSetupResponse {
    pub secret: String,
    pub otpauth_uri: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct TotpCodeRequest {
    pub code: Option<String>,
}

pub async fn get_totp_status(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<ApiResponse<TotpStatusResponse>>, AppError> {
    require_cookie_session(&auth_user)?;
    let user_repo = UserRepository::new(state.db.clone());
    let (secret, enabled) = user_repo.totp_config(auth_user.user.id).await?;
    Ok(Json(ApiResponse::success(TotpStatusResponse {
        enabled,
        setup_pending: secret.is_some() && !enabled,
    })))
}

pub async fn setup_totp(
    State(state): State<AppState>,
    auth_user: AuthUser,
    req: Option<Json<TotpCodeRequest>>,
) -> Result<Json<ApiResponse<TotpSetupResponse>>, AppError> {
    require_cookie_session(&auth_user)?;
    let user_repo = UserRepository::new(state.db.clone());
    let (existing_secret, enabled) = user_repo.totp_config(auth_user.user.id).await?;
    if enabled {
        let existing_secret =
            existing_secret.ok_or(AppError::BadRequest("TOTP secret is missing".to_string()))?;
        let code = req
            .as_ref()
            .and_then(|Json(req)| req.code.as_deref())
            .map(|code| normalize_totp_code(code.to_string()))
            .transpose()?
            .ok_or(AppError::BadRequest(
                "Current TOTP code is required".to_string(),
            ))?;
        if !verify_totp_code(&existing_secret, &code, Utc::now()) {
            return Err(AppError::BadRequest("Invalid TOTP code".to_string()));
        }
    }
    let secret = generate_totp_secret();
    user_repo
        .set_totp_secret(auth_user.user.id, &secret, false)
        .await?;
    Ok(Json(ApiResponse::success(TotpSetupResponse {
        otpauth_uri: otpauth_uri("XLStatus", &auth_user.user.username, &secret),
        secret,
        enabled: false,
    })))
}

pub async fn enable_totp(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<TotpCodeRequest>,
) -> Result<Json<ApiResponse<TotpStatusResponse>>, AppError> {
    require_cookie_session(&auth_user)?;
    let user_repo = UserRepository::new(state.db.clone());
    let (secret, _) = user_repo.totp_config(auth_user.user.id).await?;
    let secret = secret.ok_or(AppError::BadRequest(
        "TOTP setup has not been started".to_string(),
    ))?;
    let code = req
        .code
        .map(normalize_totp_code)
        .transpose()?
        .ok_or(AppError::BadRequest("TOTP code is required".to_string()))?;
    if !verify_totp_code(&secret, &code, Utc::now()) {
        return Err(AppError::BadRequest("Invalid TOTP code".to_string()));
    }
    user_repo.set_totp_enabled(auth_user.user.id, true).await?;
    Ok(Json(ApiResponse::success(TotpStatusResponse {
        enabled: true,
        setup_pending: false,
    })))
}

pub async fn disable_totp(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<TotpCodeRequest>,
) -> Result<Json<ApiResponse<TotpStatusResponse>>, AppError> {
    require_cookie_session(&auth_user)?;
    let user_repo = UserRepository::new(state.db.clone());
    let (secret, enabled) = user_repo.totp_config(auth_user.user.id).await?;
    if enabled {
        let secret = secret.ok_or(AppError::BadRequest("TOTP secret is missing".to_string()))?;
        let code = req
            .code
            .map(normalize_totp_code)
            .transpose()?
            .ok_or(AppError::BadRequest("TOTP code is required".to_string()))?;
        if !verify_totp_code(&secret, &code, Utc::now()) {
            return Err(AppError::BadRequest("Invalid TOTP code".to_string()));
        }
    }
    user_repo.disable_totp(auth_user.user.id).await?;
    Ok(Json(ApiResponse::success(TotpStatusResponse {
        enabled: false,
        setup_pending: false,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    #[serde(default = "default_session_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_session_limit() -> i64 {
    100
}

#[derive(Debug, serde::Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub role: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub expires_at: String,
    pub created_at: String,
    pub is_current: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionInfo>,
    pub total: i64,
}

pub async fn list_sessions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<ApiResponse<ListSessionsResponse>>, AppError> {
    require_cookie_session(&auth_user)?;
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);
    let admin = auth_user.user.role.is_admin() && !auth_user.is_pat();
    let now = Utc::now();
    let (sessions, total) = match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let (rows, total) = if admin {
                let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, String, String)>(
                    r#"
                    SELECT s.id, s.user_id, u.username, u.role, s.ip, s.user_agent, s.expires_at, s.created_at
                    FROM sessions s
                    JOIN users u ON u.id = s.user_id
                    WHERE s.expires_at > ?
                    ORDER BY s.created_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
                .bind(now.to_rfc3339())
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;
                let total: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE expires_at > ?")
                        .bind(now.to_rfc3339())
                        .fetch_one(pool)
                        .await?;
                (rows, total.0)
            } else {
                let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, String, String)>(
                    r#"
                    SELECT s.id, s.user_id, u.username, u.role, s.ip, s.user_agent, s.expires_at, s.created_at
                    FROM sessions s
                    JOIN users u ON u.id = s.user_id
                    WHERE s.expires_at > ? AND s.user_id = ?
                    ORDER BY s.created_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
                .bind(now.to_rfc3339())
                .bind(auth_user.user.id.0.to_string())
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;
                let total: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM sessions WHERE expires_at > ? AND user_id = ?",
                )
                .bind(now.to_rfc3339())
                .bind(auth_user.user.id.0.to_string())
                .fetch_one(pool)
                .await?;
                (rows, total.0)
            };
            (
                rows.into_iter()
                    .map(|row| session_info_from_sqlite(row, &auth_user.session_id))
                    .collect(),
                total,
            )
        }
        DatabaseBackend::Postgres(pool) => {
            let (rows, total) = if admin {
                let rows = sqlx::query_as::<
                    _,
                    (
                        String,
                        String,
                        String,
                        String,
                        Option<String>,
                        Option<String>,
                        String,
                        String,
                    ),
                >(
                    r#"
                    SELECT s.id::text, s.user_id::text, u.username, u.role, s.ip, s.user_agent,
                           to_char(s.expires_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                           to_char(s.created_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
                    FROM sessions s
                    JOIN users u ON u.id = s.user_id
                    WHERE s.expires_at > $1
                    ORDER BY s.created_at DESC
                    LIMIT $2 OFFSET $3
                    "#,
                )
                .bind(now)
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;
                let total: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE expires_at > $1")
                        .bind(now)
                        .fetch_one(pool)
                        .await?;
                (rows, total.0)
            } else {
                let rows = sqlx::query_as::<
                    _,
                    (
                        String,
                        String,
                        String,
                        String,
                        Option<String>,
                        Option<String>,
                        String,
                        String,
                    ),
                >(
                    r#"
                    SELECT s.id::text, s.user_id::text, u.username, u.role, s.ip, s.user_agent,
                           to_char(s.expires_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                           to_char(s.created_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
                    FROM sessions s
                    JOIN users u ON u.id = s.user_id
                    WHERE s.expires_at > $1 AND s.user_id = $2
                    ORDER BY s.created_at DESC
                    LIMIT $3 OFFSET $4
                    "#,
                )
                .bind(now)
                .bind(auth_user.user.id.0)
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;
                let total: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM sessions WHERE expires_at > $1 AND user_id = $2",
                )
                .bind(now)
                .bind(auth_user.user.id.0)
                .fetch_one(pool)
                .await?;
                (rows, total.0)
            };
            (
                rows.into_iter()
                    .map(|row| session_info_from_sqlite(row, &auth_user.session_id))
                    .collect(),
                total,
            )
        }
    };
    Ok(Json(ApiResponse::success(ListSessionsResponse {
        sessions,
        total,
    })))
}

pub async fn delete_session(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_cookie_session(&auth_user)?;
    let session_id = normalize_auth_resource_uuid(&id, "session id")?;
    if !auth_user.user.role.is_admin() && session_id != auth_user.session_id {
        return Err(AppError::Forbidden(
            "cannot delete another user's session".into(),
        ));
    }
    require_sensitive_totp(&state.db, auth_user.user.id, &headers).await?;
    let session_repo = crate::auth::SessionRepository::new(state.db.clone());
    session_repo.delete(&session_id).await?;
    Ok(Json(ApiResponse::success(
        serde_json::json!({"id": session_id, "deleted": true}),
    )))
}

#[derive(Debug, Deserialize)]
pub struct ListWafBansQuery {
    #[serde(default = "default_waf_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_waf_limit() -> i64 {
    100
}

#[derive(Debug, Serialize, Clone)]
pub struct WafBanView {
    pub id: String,
    pub ip: String,
    pub reason: String,
    pub failed_count: i64,
    pub banned_until: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ListWafBansResponse {
    pub bans: Vec<WafBanView>,
    pub total: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateWafBansRequest {
    pub ips: Vec<String>,
    pub reason: Option<String>,
    pub minutes: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateWafBansResponse {
    pub bans: Vec<WafBanView>,
}

pub async fn list_waf_bans(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<ListWafBansQuery>,
) -> Result<Json<ApiResponse<ListWafBansResponse>>, AppError> {
    require_admin_cookie_session(&auth_user)?;
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);
    let now = Utc::now();
    let (bans, total) = match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query_as::<_, (String, String, String, i64, String, String, String)>(
                r#"
                SELECT id, ip, reason, failed_count, banned_until, created_at, updated_at
                FROM waf_bans
                WHERE banned_until > ?
                ORDER BY banned_until DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(now.to_rfc3339())
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?;
            let total: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM waf_bans WHERE banned_until > ?")
                    .bind(now.to_rfc3339())
                    .fetch_one(pool)
                    .await?;
            (rows.into_iter().map(waf_ban_from_row).collect(), total.0)
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query_as::<_, (String, String, String, i64, String, String, String)>(
                r#"
                SELECT id, ip, reason, failed_count,
                       to_char(banned_until, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                       to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                       to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
                FROM waf_bans
                WHERE banned_until > $1
                ORDER BY banned_until DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(now)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?;
            let total: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM waf_bans WHERE banned_until > $1")
                    .bind(now)
                    .fetch_one(pool)
                    .await?;
            (rows.into_iter().map(waf_ban_from_row).collect(), total.0)
        }
    };
    Ok(Json(ApiResponse::success(ListWafBansResponse {
        bans,
        total,
    })))
}

pub async fn create_waf_bans(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Json(req): Json<CreateWafBansRequest>,
) -> Result<Json<ApiResponse<CreateWafBansResponse>>, AppError> {
    require_admin_cookie_session(&auth_user)?;
    require_sensitive_totp(&state.db, auth_user.user.id, &headers).await?;
    let ips = parse_manual_ban_ips(req.ips)?;
    if ips.is_empty() {
        return Err(AppError::BadRequest("at least one IP is required".into()));
    }
    let reason = normalize_waf_reason(req.reason);
    let minutes = normalize_waf_minutes(req.minutes);
    let banned_until = Utc::now() + Duration::minutes(minutes);
    let mut bans = Vec::with_capacity(ips.len());
    for ip in ips {
        upsert_waf_ban(&state.db, &ip, &reason, 0, banned_until).await?;
        record_waf_event(
            &state.db,
            &ip,
            Some(&auth_user.user.username),
            "manual_ban",
            Some(&reason),
        )
        .await?;
        if let Some(ban) = active_waf_ban(&state.db, &ip).await? {
            bans.push(ban);
        }
    }
    Ok(Json(ApiResponse::success(CreateWafBansResponse { bans })))
}

pub async fn delete_waf_ban(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_admin_cookie_session(&auth_user)?;
    require_sensitive_totp(&state.db, auth_user.user.id, &headers).await?;
    let ban_id = normalize_auth_resource_uuid(&id, "WAF ban id")?;
    let affected = match &state.db {
        DatabaseBackend::Sqlite(pool) => sqlx::query("DELETE FROM waf_bans WHERE id = ?")
            .bind(&ban_id)
            .execute(pool)
            .await?
            .rows_affected(),
        DatabaseBackend::Postgres(pool) => sqlx::query("DELETE FROM waf_bans WHERE id = $1")
            .bind(&ban_id)
            .execute(pool)
            .await?
            .rows_affected(),
    };
    if affected == 0 {
        return Err(AppError::NotFound("WAF ban not found".into()));
    }
    Ok(Json(ApiResponse::success(
        serde_json::json!({"id": ban_id, "deleted": true}),
    )))
}

fn require_admin(auth_user: &AuthUser) -> Result<(), AppError> {
    if !auth_user.user.role.is_admin() {
        return Err(AppError::Forbidden("Admin role required".to_string()));
    }
    Ok(())
}

fn require_cookie_session(auth_user: &AuthUser) -> Result<(), AppError> {
    if auth_user.is_pat() {
        return Err(AppError::Forbidden("Cookie session required".to_string()));
    }
    Ok(())
}

fn require_admin_cookie_session(auth_user: &AuthUser) -> Result<(), AppError> {
    require_admin(auth_user)?;
    require_cookie_session(auth_user)
}

pub(crate) fn cookie_secure_attr(enabled: bool) -> &'static str {
    if enabled {
        "; Secure"
    } else {
        ""
    }
}

pub(crate) async fn require_sensitive_totp(
    db: &DatabaseBackend,
    user_id: UserId,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    let user_repo = UserRepository::new(db.clone());
    let (secret, enabled) = user_repo.totp_config(user_id).await?;
    if !enabled {
        return Ok(());
    }
    let secret = secret.ok_or(AppError::Forbidden("TOTP secret is missing".into()))?;
    let code = sensitive_totp_code_from_headers(headers)
        .ok_or(AppError::Forbidden("TOTP code is required".into()))?;
    if !verify_totp_code(&secret, &code, Utc::now()) {
        return Err(AppError::Forbidden("Invalid TOTP code".into()));
    }
    Ok(())
}

fn sensitive_totp_code_from_headers(headers: &HeaderMap) -> Option<String> {
    header_value(headers, SENSITIVE_TOTP_HEADER)
        .or_else(|| header_value(headers, "x-sensitive-totp-code"))
}

fn session_info_from_sqlite(
    (id, user_id, username, role, ip, user_agent, expires_at, created_at): (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
    ),
    current_session_id: &str,
) -> SessionInfo {
    SessionInfo {
        is_current: id == current_session_id,
        id,
        user_id,
        username,
        role,
        ip,
        user_agent,
        expires_at,
        created_at,
    }
}

fn parse_user_id(id: &str) -> Result<UserId, AppError> {
    normalize_auth_resource_uuid(id, "user id").map(|id| {
        UserId(uuid::Uuid::parse_str(&id).expect("canonical UUID must parse after validation"))
    })
}

fn normalize_auth_resource_uuid(id: &str, field: &str) -> Result<String, AppError> {
    if id.len() != AUTH_UUID_TEXT_LEN {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    let parsed = uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::BadRequest(format!("{field} must be a canonical UUID")))?;
    let canonical = parsed.to_string();
    if canonical != id {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    Ok(canonical)
}

fn normalize_username(value: String) -> Result<String, AppError> {
    normalize_required_auth_text(value, AUTH_MAX_USERNAME_BYTES, "username")
}

fn normalize_password(value: String) -> Result<String, AppError> {
    if value.len() > AUTH_MAX_PASSWORD_BYTES {
        return Err(AppError::BadRequest(format!(
            "password must be at most {AUTH_MAX_PASSWORD_BYTES} bytes"
        )));
    }
    if value.trim().is_empty() {
        return Err(AppError::BadRequest("password is required".into()));
    }
    Ok(value)
}

fn normalize_new_password(value: String) -> Result<String, AppError> {
    let value = normalize_password(value)?;
    if value.len() < AUTH_MIN_PASSWORD_BYTES {
        return Err(AppError::BadRequest(format!(
            "password must be at least {AUTH_MIN_PASSWORD_BYTES} bytes"
        )));
    }
    Ok(value)
}

fn normalize_role(value: String) -> Result<UserRole, AppError> {
    let value = normalize_required_auth_text(value, AUTH_MAX_ROLE_BYTES, "role")?;
    value.parse::<UserRole>().map_err(AppError::BadRequest)
}

fn normalize_optional_totp_code(value: Option<String>) -> Result<Option<String>, AppError> {
    value.map(normalize_totp_code).transpose()
}

fn normalize_totp_code(value: String) -> Result<String, AppError> {
    let value = value.trim().to_string();
    if value.len() != AUTH_TOTP_CODE_BYTES || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AppError::BadRequest("TOTP code must be 6 digits".into()));
    }
    Ok(value)
}

fn normalize_required_auth_text(
    value: String,
    max_bytes: usize,
    field: &str,
) -> Result<String, AppError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if value.len() > max_bytes {
        return Err(AppError::BadRequest(format!(
            "{field} must be at most {max_bytes} bytes"
        )));
    }
    Ok(value)
}

fn user_info(user: crate::db::User) -> UserInfo {
    UserInfo {
        id: user.id.0.to_string(),
        username: user.username,
        role: user.role.to_string(),
        created_at: Some(user.created_at.to_rfc3339()),
        updated_at: Some(user.updated_at.to_rfc3339()),
    }
}

async fn revoke_user_credentials(db: &DatabaseBackend, user_id: UserId) -> Result<(), AppError> {
    crate::auth::SessionRepository::new(db.clone())
        .delete_for_user(user_id)
        .await?;
    PATRepository::new(db.clone())
        .revoke_all_for_user(user_id)
        .await?;
    Ok(())
}

fn parse_manual_ban_ips(values: Vec<String>) -> Result<Vec<String>, AppError> {
    if values.len() > AUTH_MAX_WAF_BAN_IPS {
        return Err(AppError::BadRequest(format!(
            "ips must contain at most {AUTH_MAX_WAF_BAN_IPS} items"
        )));
    }
    let mut seen = HashSet::new();
    let mut ips = Vec::new();
    for raw in values {
        if raw.len() > AUTH_MAX_WAF_IP_FIELD_BYTES {
            return Err(AppError::BadRequest(format!(
                "ip field must be at most {AUTH_MAX_WAF_IP_FIELD_BYTES} bytes"
            )));
        }
        for value in raw.split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace()) {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let ip = value
                .parse::<IpAddr>()
                .map_err(|_| AppError::BadRequest(format!("invalid IP address: {value}")))?
                .to_string();
            if seen.insert(ip.clone()) {
                if ips.len() >= AUTH_MAX_WAF_BAN_IPS {
                    return Err(AppError::BadRequest(format!(
                        "ips must contain at most {AUTH_MAX_WAF_BAN_IPS} unique addresses"
                    )));
                }
                ips.push(ip);
            }
        }
    }
    Ok(ips)
}

fn normalize_waf_reason(value: Option<String>) -> String {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_utf8_to_bytes(value, AUTH_MAX_WAF_REASON_BYTES).to_string())
        .unwrap_or_else(|| "manual WAF ban".to_string())
}

fn normalize_waf_minutes(value: Option<i64>) -> i64 {
    value
        .unwrap_or(LOGIN_BAN_MINUTES)
        .clamp(1, AUTH_MAX_WAF_BAN_MINUTES)
}

fn truncate_utf8_to_bytes(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = 0;
    for (idx, ch) in value.char_indices() {
        let next = idx + ch.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    &value[..end]
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_auth_resource_uuid, normalize_new_password, normalize_optional_totp_code,
        normalize_role, normalize_username, normalize_waf_minutes, normalize_waf_reason,
        parse_manual_ban_ips, parse_user_id, require_admin_cookie_session, revoke_user_credentials,
        sensitive_totp_code_from_headers, AppError, AUTH_API_MAX_BODY_BYTES,
        AUTH_LOGIN_MAX_BODY_BYTES, AUTH_MAX_PASSWORD_BYTES, AUTH_MAX_ROLE_BYTES,
        AUTH_MAX_USERNAME_BYTES, AUTH_MAX_WAF_BAN_IPS, AUTH_MAX_WAF_BAN_MINUTES,
        AUTH_MAX_WAF_IP_FIELD_BYTES, AUTH_MAX_WAF_REASON_BYTES, AUTH_MIN_PASSWORD_BYTES,
        AUTH_TOTP_CODE_BYTES, AUTH_TOTP_MAX_BODY_BYTES, AUTH_UUID_TEXT_LEN, SENSITIVE_TOTP_HEADER,
    };
    use crate::api::types::{ApiResponse, LoginResponse, UserInfo};
    use crate::auth::middleware::{AuthKind, AuthUser};
    use crate::auth::{hash_token, SessionRepository};
    use crate::db::{
        CreatePATInput, CreateSessionInput, CreateUserInput, DatabaseBackend, PATRepository, User,
        UserRepository,
    };
    use axum::http::{HeaderMap, HeaderValue};
    use chrono::{Duration, Utc};
    use xlstatus_shared::{UserId, UserRole};

    #[test]
    fn parses_manual_ban_ips_with_deduplication() {
        let ips = parse_manual_ban_ips(vec![
            " 192.0.2.1, 192.0.2.1 ".into(),
            "2001:db8::1\n198.51.100.10".into(),
        ])
        .unwrap();

        assert_eq!(ips, vec!["192.0.2.1", "2001:db8::1", "198.51.100.10"]);
    }

    #[test]
    fn rejects_invalid_manual_ban_ip() {
        assert!(parse_manual_ban_ips(vec!["not-an-ip".into()]).is_err());
    }

    #[test]
    fn auth_resource_limits_are_explicit() {
        assert_eq!(AUTH_API_MAX_BODY_BYTES, 64 * 1024);
        assert_eq!(AUTH_LOGIN_MAX_BODY_BYTES, 4 * 1024);
        assert_eq!(AUTH_TOTP_MAX_BODY_BYTES, 1024);
        assert_eq!(AUTH_MAX_USERNAME_BYTES, 128);
        assert_eq!(AUTH_MIN_PASSWORD_BYTES, 8);
        assert_eq!(AUTH_MAX_PASSWORD_BYTES, 1024);
        assert_eq!(AUTH_MAX_ROLE_BYTES, 32);
        assert_eq!(AUTH_TOTP_CODE_BYTES, 6);
        assert_eq!(AUTH_MAX_WAF_BAN_IPS, 128);
        assert_eq!(AUTH_MAX_WAF_IP_FIELD_BYTES, 4096);
        assert_eq!(AUTH_MAX_WAF_REASON_BYTES, 255);
        assert_eq!(AUTH_MAX_WAF_BAN_MINUTES, 43_200);
        assert_eq!(AUTH_UUID_TEXT_LEN, 36);
    }

    #[test]
    fn auth_resource_ids_require_canonical_uuid_text() {
        let id = uuid::Uuid::now_v7();
        let canonical = id.to_string();

        assert_eq!(
            normalize_auth_resource_uuid(&canonical, "resource id").unwrap(),
            canonical
        );
        assert_eq!(parse_user_id(&canonical).unwrap().0, id);
        assert!(
            normalize_auth_resource_uuid(&"a".repeat(AUTH_UUID_TEXT_LEN + 1), "resource id")
                .is_err()
        );
        assert!(normalize_auth_resource_uuid(&id.simple().to_string(), "resource id").is_err());
        assert!(normalize_auth_resource_uuid(&canonical.to_uppercase(), "resource id").is_err());
    }

    #[test]
    fn auth_text_fields_are_bounded() {
        assert_eq!(normalize_username(" alice ".into()).unwrap(), "alice");
        assert!(normalize_username("a".repeat(AUTH_MAX_USERNAME_BYTES + 1)).is_err());
        assert!(normalize_new_password("a".repeat(AUTH_MIN_PASSWORD_BYTES)).is_ok());
        assert_eq!(
            normalize_new_password(" secret-space ".into()).unwrap(),
            " secret-space "
        );
        assert!(normalize_new_password("short".into()).is_err());
        assert!(normalize_new_password("a".repeat(AUTH_MAX_PASSWORD_BYTES + 1)).is_err());
        assert!(normalize_role(" admin ".into()).is_ok());
        assert!(normalize_role("a".repeat(AUTH_MAX_ROLE_BYTES + 1)).is_err());
    }

    #[test]
    fn totp_codes_are_shape_checked() {
        assert_eq!(
            normalize_optional_totp_code(Some(" 123456 ".into()))
                .unwrap()
                .as_deref(),
            Some("123456")
        );
        assert!(normalize_optional_totp_code(Some("12345".into())).is_err());
        assert!(normalize_optional_totp_code(Some("abcdef".into())).is_err());
    }

    #[test]
    fn waf_manual_bans_are_bounded() {
        let too_many = (0..=AUTH_MAX_WAF_BAN_IPS)
            .map(|idx| format!("192.0.2.{idx}"))
            .collect::<Vec<_>>();
        assert!(parse_manual_ban_ips(too_many).is_err());
        assert!(parse_manual_ban_ips(vec!["1".repeat(AUTH_MAX_WAF_IP_FIELD_BYTES + 1)]).is_err());

        let reason = normalize_waf_reason(Some("x".repeat(AUTH_MAX_WAF_REASON_BYTES + 1)));
        assert_eq!(reason.len(), AUTH_MAX_WAF_REASON_BYTES);
        let unicode_reason = normalize_waf_reason(Some("测".repeat(AUTH_MAX_WAF_REASON_BYTES)));
        assert!(unicode_reason.len() <= AUTH_MAX_WAF_REASON_BYTES);
        assert!(unicode_reason.is_char_boundary(unicode_reason.len()));
        assert_eq!(normalize_waf_reason(Some(" ".into())), "manual WAF ban");
        assert_eq!(normalize_waf_minutes(Some(0)), 1);
        assert_eq!(
            normalize_waf_minutes(Some(AUTH_MAX_WAF_BAN_MINUTES + 1)),
            AUTH_MAX_WAF_BAN_MINUTES
        );
    }

    #[test]
    fn reads_sensitive_totp_code_header() {
        let mut headers = HeaderMap::new();
        headers.insert(SENSITIVE_TOTP_HEADER, HeaderValue::from_static(" 123456 "));
        assert_eq!(
            sensitive_totp_code_from_headers(&headers).as_deref(),
            Some("123456")
        );
    }

    #[test]
    fn login_response_does_not_serialize_session_token() {
        let response = ApiResponse::success(LoginResponse {
            user: Some(UserInfo {
                id: "user-id".into(),
                username: "alice".into(),
                role: "admin".into(),
                created_at: None,
                updated_at: None,
            }),
            mfa_required: false,
        });
        let value = serde_json::to_value(response).unwrap();
        assert!(value.pointer("/data/session_token").is_none());
    }

    #[test]
    fn account_admin_helpers_allow_cookie_session() {
        let auth = auth_user(AuthKind::Session);

        assert!(require_admin_cookie_session(&auth).is_ok());
    }

    #[test]
    fn account_admin_helpers_reject_pat_session() {
        let auth = auth_user(AuthKind::PersonalAccessToken);

        assert!(matches!(
            require_admin_cookie_session(&auth),
            Err(AppError::Forbidden(_))
        ));
    }

    #[tokio::test]
    async fn revoke_user_credentials_removes_sessions_and_pats_for_target_only() {
        let db = test_db().await;
        let users = UserRepository::new(db.clone());
        let sessions = SessionRepository::new(db.clone());
        let pats = PATRepository::new(db.clone());
        let target = users
            .create(CreateUserInput {
                username: "target".into(),
                password: "password123".into(),
                role: UserRole::Member,
            })
            .await
            .unwrap();
        let other = users
            .create(CreateUserInput {
                username: "other".into(),
                password: "password123".into(),
                role: UserRole::Member,
            })
            .await
            .unwrap();
        let target_session_hash = hash_token("target-session");
        let other_session_hash = hash_token("other-session");
        sessions
            .create(
                CreateSessionInput {
                    user_id: target.id,
                    ip: None,
                    user_agent: None,
                    expires_at: Utc::now() + Duration::hours(1),
                },
                target_session_hash.clone(),
            )
            .await
            .unwrap();
        sessions
            .create(
                CreateSessionInput {
                    user_id: other.id,
                    ip: None,
                    user_agent: None,
                    expires_at: Utc::now() + Duration::hours(1),
                },
                other_session_hash.clone(),
            )
            .await
            .unwrap();
        let target_pat_hash = hash_token("target-pat");
        let other_pat_hash = hash_token("other-pat");
        pats.create(
            CreatePATInput {
                user_id: target.id,
                name: "target".into(),
                scopes: vec!["server:read".into()],
                server_ids: Some(vec![uuid::Uuid::now_v7().to_string()]),
                expires_at: Some(Utc::now() + Duration::days(1)),
            },
            target_pat_hash.clone(),
        )
        .await
        .unwrap();
        pats.create(
            CreatePATInput {
                user_id: other.id,
                name: "other".into(),
                scopes: vec!["server:read".into()],
                server_ids: Some(vec![uuid::Uuid::now_v7().to_string()]),
                expires_at: Some(Utc::now() + Duration::days(1)),
            },
            other_pat_hash.clone(),
        )
        .await
        .unwrap();

        revoke_user_credentials(&db, target.id).await.unwrap();

        assert!(sessions
            .find_by_token_hash(&target_session_hash)
            .await
            .unwrap()
            .is_none());
        assert!(pats
            .find_by_token_hash(&target_pat_hash)
            .await
            .unwrap()
            .is_none());
        assert!(sessions
            .find_by_token_hash(&other_session_hash)
            .await
            .unwrap()
            .is_some());
        assert!(pats
            .find_by_token_hash(&other_pat_hash)
            .await
            .unwrap()
            .is_some());
    }

    fn auth_user(auth_kind: AuthKind) -> AuthUser {
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
            session_id: "sess".into(),
            csrf_token: "csrf".into(),
            auth_kind,
            scopes: vec!["admin:*".into()],
            server_ids: None,
            pat_id: Some("pat".into()),
        }
    }

    async fn test_db() -> DatabaseBackend {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-auth-credential-revoke-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }
}

async fn register_login_failure(
    db: &DatabaseBackend,
    ip: &str,
    username: &str,
    reason: &str,
) -> Result<(), AppError> {
    record_waf_event(db, ip, Some(username), "login_failed", Some(reason)).await?;
    let since = Utc::now() - Duration::minutes(LOGIN_FAILURE_WINDOW_MINUTES);
    let failures = count_recent_auth_failures(db, ip, since).await?;
    if failures >= LOGIN_FAILURE_THRESHOLD {
        upsert_waf_ban(
            db,
            ip,
            "too many failed login attempts",
            failures,
            Utc::now() + Duration::minutes(LOGIN_BAN_MINUTES),
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn register_oauth_failure(
    db: &DatabaseBackend,
    ip: &str,
    provider: Option<&str>,
    reason: &str,
) -> Result<(), AppError> {
    record_waf_event(db, ip, provider, "oauth_failed", Some(reason)).await?;
    let since = Utc::now() - Duration::minutes(LOGIN_FAILURE_WINDOW_MINUTES);
    let failures = count_recent_auth_failures(db, ip, since).await?;
    if failures >= LOGIN_FAILURE_THRESHOLD {
        upsert_waf_ban(
            db,
            ip,
            "too many failed authentication attempts",
            failures,
            Utc::now() + Duration::minutes(LOGIN_BAN_MINUTES),
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn register_pat_failure(
    db: &DatabaseBackend,
    ip: &str,
    token_ref: Option<&str>,
    reason: &str,
) -> Result<(), AppError> {
    record_waf_event(db, ip, token_ref, "pat_failed", Some(reason)).await?;
    let since = Utc::now() - Duration::minutes(LOGIN_FAILURE_WINDOW_MINUTES);
    let failures = count_recent_auth_failures(db, ip, since).await?;
    if failures >= LOGIN_FAILURE_THRESHOLD {
        upsert_waf_ban(
            db,
            ip,
            "too many failed authentication attempts",
            failures,
            Utc::now() + Duration::minutes(LOGIN_BAN_MINUTES),
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn register_agent_auth_failure(
    db: &DatabaseBackend,
    ip: &str,
    agent_ref: Option<&str>,
    reason: &str,
) -> Result<(), AppError> {
    record_waf_event(db, ip, agent_ref, "agent_auth_failed", Some(reason)).await?;
    let since = Utc::now() - Duration::minutes(LOGIN_FAILURE_WINDOW_MINUTES);
    let failures = count_recent_auth_failures(db, ip, since).await?;
    if failures >= LOGIN_FAILURE_THRESHOLD {
        upsert_waf_ban(
            db,
            ip,
            "too many failed authentication attempts",
            failures,
            Utc::now() + Duration::minutes(LOGIN_BAN_MINUTES),
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn active_waf_ban(
    db: &DatabaseBackend,
    ip: &str,
) -> Result<Option<WafBanView>, AppError> {
    let now = Utc::now();
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query_as::<_, (String, String, String, i64, String, String, String)>(
                r#"
                SELECT id, ip, reason, failed_count, banned_until, created_at, updated_at
                FROM waf_bans
                WHERE ip = ? AND banned_until > ?
                "#,
            )
            .bind(ip)
            .bind(now.to_rfc3339())
            .fetch_optional(pool)
            .await?;
            Ok(row.map(waf_ban_from_row))
        }
        DatabaseBackend::Postgres(pool) => {
            let row = sqlx::query_as::<_, (String, String, String, i64, String, String, String)>(
                r#"
                SELECT id, ip, reason, failed_count,
                       to_char(banned_until, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                       to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                       to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
                FROM waf_bans
                WHERE ip = $1 AND banned_until > $2
                "#,
            )
            .bind(ip)
            .bind(now)
            .fetch_optional(pool)
            .await?;
            Ok(row.map(waf_ban_from_row))
        }
    }
}

async fn count_recent_auth_failures(
    db: &DatabaseBackend,
    ip: &str,
    since: DateTime<Utc>,
) -> Result<i64, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM waf_events WHERE ip = ? AND outcome IN ('login_failed', 'oauth_failed', 'pat_failed', 'agent_auth_failed') AND created_at >= ?",
            )
            .bind(ip)
            .bind(since.to_rfc3339())
            .fetch_one(pool)
            .await?;
            Ok(row.0)
        }
        DatabaseBackend::Postgres(pool) => {
            let row: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM waf_events WHERE ip = $1 AND outcome IN ('login_failed', 'oauth_failed', 'pat_failed', 'agent_auth_failed') AND created_at >= $2",
            )
            .bind(ip)
            .bind(since)
            .fetch_one(pool)
            .await?;
            Ok(row.0)
        }
    }
}

pub(crate) async fn record_waf_event(
    db: &DatabaseBackend,
    ip: &str,
    username: Option<&str>,
    outcome: &str,
    reason: Option<&str>,
) -> Result<(), AppError> {
    let id = uuid::Uuid::now_v7().to_string();
    let now = Utc::now();
    let username = username
        .map(|value| value.trim().chars().take(255).collect::<String>())
        .filter(|value| !value.is_empty());
    match db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO waf_events (id, ip, username, outcome, reason, created_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(ip)
            .bind(&username)
            .bind(outcome)
            .bind(reason)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO waf_events (id, ip, username, outcome, reason, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&id)
            .bind(ip)
            .bind(&username)
            .bind(outcome)
            .bind(reason)
            .bind(now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

async fn upsert_waf_ban(
    db: &DatabaseBackend,
    ip: &str,
    reason: &str,
    failed_count: i64,
    banned_until: DateTime<Utc>,
) -> Result<(), AppError> {
    let id = uuid::Uuid::now_v7().to_string();
    let now = Utc::now();
    match db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                r#"
                INSERT INTO waf_bans (id, ip, reason, failed_count, banned_until, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(ip) DO UPDATE SET
                    reason = excluded.reason,
                    failed_count = excluded.failed_count,
                    banned_until = excluded.banned_until,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&id)
            .bind(ip)
            .bind(reason)
            .bind(failed_count)
            .bind(banned_until.to_rfc3339())
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                r#"
                INSERT INTO waf_bans (id, ip, reason, failed_count, banned_until, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT(ip) DO UPDATE SET
                    reason = excluded.reason,
                    failed_count = excluded.failed_count,
                    banned_until = excluded.banned_until,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&id)
            .bind(ip)
            .bind(reason)
            .bind(failed_count)
            .bind(banned_until)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

fn waf_ban_from_row(
    (id, ip, reason, failed_count, banned_until, created_at, updated_at): (
        String,
        String,
        String,
        i64,
        String,
        String,
        String,
    ),
) -> WafBanView {
    WafBanView {
        id,
        ip,
        reason,
        failed_count,
        banned_until,
        created_at,
        updated_at,
    }
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

// Error handling
#[derive(Debug)]
pub enum AppError {
    Database(anyhow::Error),
    Unauthorized(String),
    Forbidden(String),
    BadRequest(String),
    TooManyRequests(String),
    NotFound(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Database(e) => {
                tracing::error!("Database error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::TooManyRequests(msg) => (StatusCode::TOO_MANY_REQUESTS, msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
        };

        (status, Json(ApiResponse::<()>::error(message))).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Database(err)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Database(anyhow::anyhow!(err))
    }
}

impl From<xlstatus_tsdb::MetricError> for AppError {
    fn from(err: xlstatus_tsdb::MetricError) -> Self {
        AppError::Database(anyhow::anyhow!(err))
    }
}
