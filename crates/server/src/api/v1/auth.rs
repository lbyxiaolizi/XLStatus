use crate::api::types::*;
use crate::auth::middleware::{derive_csrf_token, AuthUser, CSRF_COOKIE_NAME, SESSION_COOKIE_NAME};
use crate::auth::{generate_session_token, hash_token};
use crate::config::Config;
use crate::db::{CreateSessionInput, CreateUserInput, DatabaseBackend, UserRepository};
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{AppendHeaders, IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use xlstatus_shared::UserRole;

pub type AgentJwtChallengeStore = Arc<RwLock<HashMap<String, DateTime<Utc>>>>;

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

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let user_repo = UserRepository::new(state.db.clone());

    // Find user by username
    let user = user_repo
        .find_by_username(&req.username)
        .await?
        .ok_or(AppError::Unauthorized("Invalid credentials".to_string()))?;

    // Verify password
    if !user_repo.verify_password(&user, &req.password)? {
        return Err(AppError::Unauthorized("Invalid credentials".to_string()));
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
                ip: None,         // TODO: extract from request
                user_agent: None, // TODO: extract from request
                expires_at,
            },
            token_hash.clone(),
        )
        .await?;

    let csrf_token = derive_csrf_token(&token_hash);
    let session_cookie = format!(
        "{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
        SESSION_COOKIE_NAME,
        session_token,
        state.config.security.session_ttl_hours * 3600
    );
    let session_cookie = HeaderValue::from_str(&session_cookie)
        .map_err(|e| AppError::BadRequest(format!("Invalid session cookie: {}", e)))?;
    let csrf_cookie = format!(
        "{}={}; SameSite=Lax; Path=/; Max-Age={}",
        CSRF_COOKIE_NAME,
        csrf_token,
        state.config.security.session_ttl_hours * 3600
    );
    let csrf_cookie = HeaderValue::from_str(&csrf_cookie)
        .map_err(|e| AppError::BadRequest(format!("Invalid session cookie: {}", e)))?;

    Ok((
        AppendHeaders([
            (header::SET_COOKIE, session_cookie),
            (header::SET_COOKIE, csrf_cookie),
        ]),
        Json(ApiResponse::success(LoginResponse {
            user: UserInfo {
                id: user.id.0.to_string(),
                username: user.username,
                role: user.role.to_string(),
            },
            session_token,
        })),
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let session_repo = crate::auth::SessionRepository::new(state.db.clone());
    session_repo.delete(&auth_user.session_id).await?;
    let session_cookie = format!(
        "{}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0",
        SESSION_COOKIE_NAME
    );
    let session_cookie = HeaderValue::from_str(&session_cookie)
        .map_err(|e| AppError::BadRequest(format!("Invalid session cookie: {}", e)))?;
    let csrf_cookie = format!("{}=; SameSite=Lax; Path=/; Max-Age=0", CSRF_COOKIE_NAME);
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
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    if !auth_user.user.role.is_admin() {
        return Err(AppError::Forbidden("Admin role required".to_string()));
    }

    let user_repo = UserRepository::new(state.db.clone());

    // Parse role
    let role = req
        .role
        .parse::<UserRole>()
        .map_err(|e| AppError::BadRequest(e))?;

    // Create user
    let user = user_repo
        .create(CreateUserInput {
            username: req.username,
            password: req.password,
            role,
        })
        .await?;

    Ok(Json(ApiResponse::success(UserInfo {
        id: user.id.0.to_string(),
        username: user.username,
        role: user.role.to_string(),
    })))
}

// Error handling
#[derive(Debug)]
pub enum AppError {
    Database(anyhow::Error),
    Unauthorized(String),
    Forbidden(String),
    BadRequest(String),
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

impl From<xlstatus_tsdb::MetricError> for AppError {
    fn from(err: xlstatus_tsdb::MetricError) -> Self {
        AppError::Database(anyhow::anyhow!(err))
    }
}
