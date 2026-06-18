use crate::api::types::*;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::sign_agent_jwt;
use crate::db::AgentRepository;
use axum::{extract::State, Json};
use chrono::{Duration, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use xlstatus_shared::AgentId;

#[derive(Debug, Deserialize)]
pub struct GetJwtChallengeRequest {
    pub agent_id: String,
}

#[derive(Debug, Serialize)]
pub struct GetJwtChallengeResponse {
    pub nonce: String,
    pub expires_in: i64,
}

#[derive(Debug, Deserialize)]
pub struct GetJwtRequest {
    pub agent_id: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Serialize)]
pub struct GetJwtResponse {
    pub jwt: String,
    pub expires_in: i64,
}

pub async fn get_agent_jwt_challenge(
    State(state): State<AppState>,
    Json(req): Json<GetJwtChallengeRequest>,
) -> Result<Json<ApiResponse<GetJwtChallengeResponse>>, AppError> {
    let agent_id = parse_agent_id(&req.agent_id)?;

    let agent_repo = AgentRepository::new(state.db.clone());
    let agent = agent_repo
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::BadRequest("Agent not found".to_string()))?;

    if agent.revoked_at.is_some() {
        return Err(AppError::Unauthorized("Agent has been revoked".to_string()));
    }

    let mut nonce = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    let nonce = hex::encode(nonce);
    let expires_at = Utc::now() + Duration::seconds(60);
    state
        .agent_jwt_challenges
        .write()
        .await
        .insert(challenge_key(agent_id, &nonce), expires_at);

    Ok(Json(ApiResponse::success(GetJwtChallengeResponse {
        nonce,
        expires_in: 60,
    })))
}

pub async fn get_agent_jwt(
    State(state): State<AppState>,
    Json(req): Json<GetJwtRequest>,
) -> Result<Json<ApiResponse<GetJwtResponse>>, AppError> {
    let agent_id = parse_agent_id(&req.agent_id)?;

    let challenge_key = challenge_key(agent_id, &req.nonce);
    let expires_at = state
        .agent_jwt_challenges
        .write()
        .await
        .remove(&challenge_key)
        .ok_or(AppError::Unauthorized(
            "JWT challenge not found".to_string(),
        ))?;
    if expires_at < Utc::now() {
        return Err(AppError::Unauthorized("JWT challenge expired".to_string()));
    }

    // Verify agent exists
    let agent_repo = AgentRepository::new(state.db.clone());
    let agent = agent_repo
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::BadRequest("Agent not found".to_string()))?;

    if agent.revoked_at.is_some() {
        return Err(AppError::Unauthorized("Agent has been revoked".to_string()));
    }

    verify_agent_signature(&agent.public_key, &req.nonce, &req.signature)?;

    // Sign JWT
    let jwt = sign_agent_jwt(agent_id, &state.config.security.session_secret)?;

    Ok(Json(ApiResponse::success(GetJwtResponse {
        jwt,
        expires_in: 300, // 5 minutes
    })))
}

fn parse_agent_id(agent_id: &str) -> Result<AgentId, AppError> {
    Ok(AgentId(uuid::Uuid::parse_str(agent_id).map_err(|e| {
        AppError::BadRequest(format!("Invalid agent_id: {}", e))
    })?))
}

fn challenge_key(agent_id: AgentId, nonce: &str) -> String {
    format!("{}:{}", agent_id.0, nonce)
}

fn verify_agent_signature(public_key: &str, nonce: &str, signature: &str) -> Result<(), AppError> {
    let public_key_bytes = hex::decode(public_key)
        .map_err(|_| AppError::BadRequest("Agent public key is not Ed25519 hex".to_string()))?;
    let public_key_bytes: [u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| AppError::BadRequest("Agent public key must be 32 bytes".to_string()))?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
        .map_err(|_| AppError::BadRequest("Agent public key is invalid".to_string()))?;

    let signature_bytes = hex::decode(signature)
        .map_err(|_| AppError::BadRequest("Agent signature is not hex".to_string()))?;
    let signature_bytes: [u8; 64] = signature_bytes
        .try_into()
        .map_err(|_| AppError::BadRequest("Agent signature must be 64 bytes".to_string()))?;
    let signature = Signature::from_bytes(&signature_bytes);

    verifying_key
        .verify(nonce.as_bytes(), &signature)
        .map_err(|_| AppError::Unauthorized("Agent signature verification failed".to_string()))
}
