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

const CHALLENGE_TTL_SECONDS: i64 = 60;
const MAX_PENDING_CHALLENGES: usize = 4096;
const MAX_PENDING_CHALLENGES_PER_AGENT: usize = 16;
const NONCE_HEX_LEN: usize = 64;

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

    let now = Utc::now();
    let mut challenges = state.agent_jwt_challenges.write().await;
    prune_expired_challenges(&mut challenges, now);
    if challenges.len() >= MAX_PENDING_CHALLENGES
        || pending_challenges_for_agent(&challenges, agent_id) >= MAX_PENDING_CHALLENGES_PER_AGENT
    {
        return Err(AppError::Forbidden(
            "too many pending JWT challenges".to_string(),
        ));
    }

    let mut nonce = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    let nonce = hex::encode(nonce);
    let expires_at = now + Duration::seconds(CHALLENGE_TTL_SECONDS);
    challenges.insert(challenge_key(agent_id, &nonce), expires_at);

    Ok(Json(ApiResponse::success(GetJwtChallengeResponse {
        nonce,
        expires_in: CHALLENGE_TTL_SECONDS,
    })))
}

pub async fn get_agent_jwt(
    State(state): State<AppState>,
    Json(req): Json<GetJwtRequest>,
) -> Result<Json<ApiResponse<GetJwtResponse>>, AppError> {
    let agent_id = parse_agent_id(&req.agent_id)?;

    if !valid_nonce_shape(&req.nonce) {
        return Err(AppError::Unauthorized(
            "JWT challenge not found".to_string(),
        ));
    }

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

fn prune_expired_challenges(
    challenges: &mut std::collections::HashMap<String, chrono::DateTime<Utc>>,
    now: chrono::DateTime<Utc>,
) {
    challenges.retain(|_, expires_at| *expires_at > now);
}

fn pending_challenges_for_agent(
    challenges: &std::collections::HashMap<String, chrono::DateTime<Utc>>,
    agent_id: AgentId,
) -> usize {
    let prefix = format!("{}:", agent_id.0);
    challenges
        .keys()
        .filter(|key| key.starts_with(&prefix))
        .count()
}

fn valid_nonce_shape(nonce: &str) -> bool {
    nonce.len() == NONCE_HEX_LEN && nonce.bytes().all(|b| b.is_ascii_hexdigit())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn challenge_pruning_and_agent_count_ignore_expired_items() {
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let other_agent_id = AgentId(uuid::Uuid::now_v7());
        let now = Utc::now();
        let mut challenges = HashMap::new();
        challenges.insert(
            challenge_key(agent_id, &"a".repeat(NONCE_HEX_LEN)),
            now + Duration::seconds(30),
        );
        challenges.insert(
            challenge_key(agent_id, &"b".repeat(NONCE_HEX_LEN)),
            now - Duration::seconds(1),
        );
        challenges.insert(
            challenge_key(other_agent_id, &"c".repeat(NONCE_HEX_LEN)),
            now + Duration::seconds(30),
        );

        prune_expired_challenges(&mut challenges, now);

        assert_eq!(challenges.len(), 2);
        assert_eq!(pending_challenges_for_agent(&challenges, agent_id), 1);
        assert_eq!(pending_challenges_for_agent(&challenges, other_agent_id), 1);
    }

    #[test]
    fn nonce_shape_requires_32_byte_hex() {
        assert!(valid_nonce_shape(&"a".repeat(NONCE_HEX_LEN)));
        assert!(valid_nonce_shape(&"A".repeat(NONCE_HEX_LEN)));
        assert!(!valid_nonce_shape(&"a".repeat(NONCE_HEX_LEN - 1)));
        assert!(!valid_nonce_shape(&"g".repeat(NONCE_HEX_LEN)));
    }
}
