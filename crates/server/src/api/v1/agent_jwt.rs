use crate::api::types::*;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::sign_agent_jwt;
use crate::db::AgentRepository;
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use xlstatus_shared::AgentId;

#[derive(Debug, Deserialize)]
pub struct GetJwtRequest {
    pub agent_id: String,
}

#[derive(Debug, Serialize)]
pub struct GetJwtResponse {
    pub jwt: String,
    pub expires_in: i64,
}

pub async fn get_agent_jwt(
    State(state): State<AppState>,
    Json(req): Json<GetJwtRequest>,
) -> Result<Json<ApiResponse<GetJwtResponse>>, AppError> {
    let agent_id = AgentId(
        uuid::Uuid::parse_str(&req.agent_id)
            .map_err(|e| AppError::BadRequest(format!("Invalid agent_id: {}", e)))?,
    );

    // Verify agent exists
    let agent_repo = AgentRepository::new(state.db.clone());
    let _agent = agent_repo
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::BadRequest("Agent not found".to_string()))?;

    // Sign JWT
    let jwt = sign_agent_jwt(agent_id, &state.config.security.session_secret)?;

    Ok(Json(ApiResponse::success(GetJwtResponse {
        jwt,
        expires_in: 300, // 5 minutes
    })))
}
