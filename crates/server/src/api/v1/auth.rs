use crate::api::types::*;
use crate::auth::{generate_session_token, hash_token};
use crate::config::Config;
use crate::db::{CreateSessionInput, CreateUserInput, DatabaseBackend, UserRepository};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{Duration, Utc};
use std::sync::Arc;
use xlstatus_shared::UserRole;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseBackend,
    pub config: Arc<Config>,
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
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
                ip: None, // TODO: extract from request
                user_agent: None, // TODO: extract from request
                expires_at,
            },
            token_hash,
        )
        .await?;

    Ok(Json(ApiResponse::success(LoginResponse {
        user: UserInfo {
            id: user.id.0.to_string(),
            username: user.username,
            role: user.role.to_string(),
        },
        session_token,
    })))
}

pub async fn logout(
    State(state): State<AppState>,
    // TODO: extract session from cookie
) -> Result<Json<ApiResponse<()>>, AppError> {
    // TODO: delete session
    Ok(Json(ApiResponse::success(())))
}

pub async fn create_user(
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
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
    BadRequest(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Database(e) => {
                tracing::error!("Database error: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
        };

        (status, Json(ApiResponse::<()>::error(message))).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Database(err)
    }
}
