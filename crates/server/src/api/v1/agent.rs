use crate::api::types::*;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::hash_token;
use crate::db::{AgentRepository, CreateAgentInput, CreateEnrollmentTokenInput, EnrollmentTokenRepository};
use axum::{extract::State, Json};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use xlstatus_shared::AgentId;

#[derive(Debug, Deserialize)]
pub struct CreateEnrollmentTokenRequest {
    pub expires_in_hours: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateEnrollmentTokenResponse {
    pub token: String,
    pub expires_at: String,
}

#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    pub name: String,
    pub enrollment_token: String,
    pub public_key: String,
}

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub agent_id: String,
    pub name: String,
}

pub async fn create_enrollment_token(
    State(state): State<AppState>,
    Json(req): Json<CreateEnrollmentTokenRequest>,
) -> Result<Json<ApiResponse<CreateEnrollmentTokenResponse>>, AppError> {
    // TEMPORARY: Get admin user
    let user_repo = crate::db::UserRepository::new(state.db.clone());
    let user = user_repo
        .find_by_username("admin")
        .await?
        .ok_or(AppError::Unauthorized("No admin user".to_string()))?;

    let repo = EnrollmentTokenRepository::new(state.db.clone());

    // Generate token
    let bytes: [u8; 32] = rand::random();
    let token = format!("xle_{}", hex::encode(bytes));
    let token_hash = hash_token(&token);

    let expires_in = req.expires_in_hours.unwrap_or(1);
    let expires_at = Utc::now() + Duration::hours(expires_in);

    let enrollment_token = repo
        .create(
            CreateEnrollmentTokenInput {
                created_by_user_id: user.id,
                expires_at,
            },
            token_hash,
        )
        .await?;

    Ok(Json(ApiResponse::success(
        CreateEnrollmentTokenResponse {
            token,
            expires_at: enrollment_token.expires_at.to_rfc3339(),
        },
    )))
}

pub async fn enroll(
    State(state): State<AppState>,
    Json(req): Json<EnrollRequest>,
) -> Result<Json<ApiResponse<EnrollResponse>>, AppError> {
    let token_hash = hash_token(&req.enrollment_token);

    let enrollment_repo = EnrollmentTokenRepository::new(state.db.clone());
    let agent_id = AgentId::new();

    // Find and mark token as used
    let token = enrollment_repo
        .find_and_use(&token_hash, agent_id)
        .await?
        .ok_or(AppError::BadRequest(
            "Invalid or expired enrollment token".to_string(),
        ))?;

    // Create agent
    let agent_repo = AgentRepository::new(state.db.clone());
    let agent = agent_repo
        .create(CreateAgentInput {
            name: req.name,
            public_key: req.public_key,
            owner_user_id: token.created_by_user_id,
        })
        .await?;

    Ok(Json(ApiResponse::success(EnrollResponse {
        agent_id: agent.id.0.to_string(),
        name: agent.name,
    })))
}
