use crate::api::types::*;
use crate::api::v1::auth::{AgentJwtChallengeStore, AppError, AppState};
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
const AGENT_ID_TEXT_LEN: usize = 36;
const NONCE_HEX_LEN: usize = 64;
const SIGNATURE_HEX_LEN: usize = 128;
const CHALLENGE_REQUEST_CLOCK_SKEW_SECONDS: i64 = 120;
const REQUEST_CHALLENGE_KEY_PREFIX: &str = "request";
const CHALLENGE_RECORDS_PER_REQUEST: usize = 2;

#[derive(Debug, Deserialize)]
pub struct GetJwtChallengeRequest {
    pub agent_id: String,
    pub request_nonce: String,
    pub request_timestamp: i64,
    pub signature: String,
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

    validate_challenge_request_shape(&req)?;
    validate_challenge_request_timestamp(req.request_timestamp, Utc::now())?;
    verify_agent_signature(
        &agent.public_key,
        &challenge_request_signature_payload(agent_id, &req.request_nonce, req.request_timestamp),
        &req.signature,
    )?;

    let now = Utc::now();
    let mut challenges = state.agent_jwt_challenges.write().await;
    prune_expired_challenges(&mut challenges, now);
    let request_key = request_challenge_key(agent_id, &req.request_nonce);
    if challenges.contains_key(&request_key) {
        return Err(AppError::Unauthorized(
            "JWT challenge request already used".to_string(),
        ));
    }
    if challenges
        .len()
        .saturating_add(CHALLENGE_RECORDS_PER_REQUEST)
        > MAX_PENDING_CHALLENGES
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
    challenges.insert(request_key, expires_at);
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
    if !valid_signature_shape(&req.signature) {
        return Err(AppError::Unauthorized(
            "Agent signature verification failed".to_string(),
        ));
    }

    let challenge_key = challenge_key(agent_id, &req.nonce);
    let expires_at = agent_jwt_challenge_expires_at(&state.agent_jwt_challenges, &challenge_key)
        .await?
        .ok_or(AppError::Unauthorized(
            "JWT challenge not found".to_string(),
        ))?;
    if expires_at < Utc::now() {
        state
            .agent_jwt_challenges
            .write()
            .await
            .remove(&challenge_key);
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
    consume_agent_jwt_challenge(&state.agent_jwt_challenges, &challenge_key, expires_at).await?;

    // Sign JWT
    let jwt = sign_agent_jwt(agent_id, &state.config.security.session_secret)?;

    Ok(Json(ApiResponse::success(GetJwtResponse {
        jwt,
        expires_in: 300, // 5 minutes
    })))
}

fn parse_agent_id(agent_id: &str) -> Result<AgentId, AppError> {
    if agent_id.len() != AGENT_ID_TEXT_LEN {
        return Err(AppError::BadRequest(
            "agent_id must be a canonical UUID".into(),
        ));
    }
    let parsed = uuid::Uuid::parse_str(agent_id)
        .map_err(|_| AppError::BadRequest("agent_id must be a canonical UUID".into()))?;
    if parsed.to_string() != agent_id {
        return Err(AppError::BadRequest(
            "agent_id must be a canonical UUID".into(),
        ));
    }
    Ok(AgentId(parsed))
}

fn challenge_key(agent_id: AgentId, nonce: &str) -> String {
    format!("{}:{}", agent_id.0, nonce)
}

fn request_challenge_key(agent_id: AgentId, request_nonce: &str) -> String {
    format!(
        "{REQUEST_CHALLENGE_KEY_PREFIX}:{}:{}",
        agent_id.0, request_nonce
    )
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

fn validate_challenge_request_shape(req: &GetJwtChallengeRequest) -> Result<(), AppError> {
    if !valid_nonce_shape(&req.request_nonce) {
        return Err(AppError::Unauthorized(
            "JWT challenge request is invalid".to_string(),
        ));
    }
    if !valid_signature_shape(&req.signature) {
        return Err(AppError::Unauthorized(
            "Agent signature verification failed".to_string(),
        ));
    }
    Ok(())
}

fn validate_challenge_request_timestamp(
    request_timestamp: i64,
    now: chrono::DateTime<Utc>,
) -> Result<(), AppError> {
    let now = now.timestamp();
    if request_timestamp < now - CHALLENGE_REQUEST_CLOCK_SKEW_SECONDS
        || request_timestamp > now + CHALLENGE_REQUEST_CLOCK_SKEW_SECONDS
    {
        return Err(AppError::Unauthorized(
            "JWT challenge request expired".to_string(),
        ));
    }
    Ok(())
}

fn challenge_request_signature_payload(
    agent_id: AgentId,
    request_nonce: &str,
    request_timestamp: i64,
) -> String {
    format!(
        "xlstatus-agent-jwt-challenge:{}:{}:{}",
        agent_id.0, request_nonce, request_timestamp
    )
}

fn valid_nonce_shape(nonce: &str) -> bool {
    nonce.len() == NONCE_HEX_LEN && nonce.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_signature_shape(signature: &str) -> bool {
    signature.len() == SIGNATURE_HEX_LEN && signature.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn consume_agent_jwt_challenge(
    store: &AgentJwtChallengeStore,
    challenge_key: &str,
    expected_expires_at: chrono::DateTime<Utc>,
) -> Result<(), AppError> {
    let mut challenges = store.write().await;
    match challenges.get(challenge_key) {
        Some(expires_at) if *expires_at == expected_expires_at => {
            challenges.remove(challenge_key);
            Ok(())
        }
        _ => Err(AppError::Unauthorized(
            "JWT challenge not found".to_string(),
        )),
    }
}

async fn agent_jwt_challenge_expires_at(
    store: &AgentJwtChallengeStore,
    challenge_key: &str,
) -> Result<Option<chrono::DateTime<Utc>>, AppError> {
    Ok(store.read().await.get(challenge_key).copied())
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
    use crate::db::{
        AgentRepository, CreateAgentInput, CreateUserInput, DatabaseBackend, UserRepository,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use xlstatus_shared::UserRole;

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

    #[test]
    fn signature_shape_requires_64_byte_hex() {
        assert!(valid_signature_shape(&"a".repeat(SIGNATURE_HEX_LEN)));
        assert!(valid_signature_shape(&"A".repeat(SIGNATURE_HEX_LEN)));
        assert!(!valid_signature_shape(&"a".repeat(SIGNATURE_HEX_LEN - 1)));
        assert!(!valid_signature_shape(&"g".repeat(SIGNATURE_HEX_LEN)));
    }

    #[test]
    fn agent_id_requires_canonical_uuid_text() {
        let id = uuid::Uuid::now_v7();

        assert_eq!(parse_agent_id(&id.to_string()).unwrap().0, id);
        assert!(matches!(
            parse_agent_id(&"a".repeat(AGENT_ID_TEXT_LEN + 1)),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            parse_agent_id(&id.simple().to_string()),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            parse_agent_id(&id.to_string().to_uppercase()),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn challenge_request_timestamp_has_short_window() {
        let now = Utc::now();
        assert!(validate_challenge_request_timestamp(now.timestamp(), now).is_ok());
        assert!(matches!(
            validate_challenge_request_timestamp(
                now.timestamp() - CHALLENGE_REQUEST_CLOCK_SKEW_SECONDS - 1,
                now
            ),
            Err(AppError::Unauthorized(_))
        ));
        assert!(matches!(
            validate_challenge_request_timestamp(
                now.timestamp() + CHALLENGE_REQUEST_CLOCK_SKEW_SECONDS + 1,
                now
            ),
            Err(AppError::Unauthorized(_))
        ));
    }

    #[test]
    fn invalid_signature_does_not_remove_challenge() {
        let public_key = "b".repeat(64);
        let nonce = "a".repeat(NONCE_HEX_LEN);
        let signature = "c".repeat(SIGNATURE_HEX_LEN);

        let err = verify_agent_signature(&public_key, &nonce, &signature).unwrap_err();

        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn jwt_challenge_requires_agent_signature_before_reserving_nonce() {
        let (state, agent_id, _signing_key) = seeded_agent_state().await;
        let invalid_signature = "c".repeat(SIGNATURE_HEX_LEN);
        let err = get_agent_jwt_challenge(
            State(state.clone()),
            Json(GetJwtChallengeRequest {
                agent_id: agent_id.to_string(),
                request_nonce: "a".repeat(NONCE_HEX_LEN),
                request_timestamp: Utc::now().timestamp(),
                signature: invalid_signature,
            }),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Unauthorized(_)));
        assert!(state.agent_jwt_challenges.read().await.is_empty());
    }

    #[tokio::test]
    async fn jwt_challenge_rejects_replayed_signed_request_nonce() {
        let (state, agent_id, signing_key) = seeded_agent_state().await;
        let request_nonce = "a".repeat(NONCE_HEX_LEN);
        let request_timestamp = Utc::now().timestamp();
        let signature =
            sign_challenge_request(&signing_key, agent_id, &request_nonce, request_timestamp);
        let request = || GetJwtChallengeRequest {
            agent_id: agent_id.to_string(),
            request_nonce: request_nonce.clone(),
            request_timestamp,
            signature: signature.clone(),
        };

        let _ = get_agent_jwt_challenge(State(state.clone()), Json(request()))
            .await
            .expect("first signed challenge request succeeds");
        let err = get_agent_jwt_challenge(State(state.clone()), Json(request()))
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Unauthorized(message) if message.contains("already used")));
        let challenges = state.agent_jwt_challenges.read().await;
        assert_eq!(challenges.len(), 2);
        assert!(challenges.contains_key(&request_challenge_key(agent_id, &request_nonce)));
    }

    #[tokio::test]
    async fn jwt_challenge_reserves_two_slots_for_global_limit() {
        let (state, agent_id, signing_key) = seeded_agent_state().await;
        let now = Utc::now();
        {
            let mut challenges = state.agent_jwt_challenges.write().await;
            for i in 0..(MAX_PENDING_CHALLENGES - 1) {
                challenges.insert(format!("filler:{i}"), now + Duration::seconds(30));
            }
        }
        let request_nonce = "a".repeat(NONCE_HEX_LEN);
        let request_timestamp = now.timestamp();
        let signature =
            sign_challenge_request(&signing_key, agent_id, &request_nonce, request_timestamp);

        let err = get_agent_jwt_challenge(
            State(state.clone()),
            Json(GetJwtChallengeRequest {
                agent_id: agent_id.to_string(),
                request_nonce,
                request_timestamp,
                signature,
            }),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(message) if message.contains("too many")));
        assert_eq!(
            state.agent_jwt_challenges.read().await.len(),
            MAX_PENDING_CHALLENGES - 1
        );
    }

    #[tokio::test]
    async fn jwt_challenge_is_consumed_only_after_explicit_consume() {
        let store: AgentJwtChallengeStore = Arc::new(RwLock::new(HashMap::new()));
        let key = "agent:nonce".to_string();
        let expires_at = Utc::now() + Duration::seconds(30);
        store.write().await.insert(key.clone(), expires_at);

        assert_eq!(
            agent_jwt_challenge_expires_at(&store, &key).await.unwrap(),
            Some(expires_at)
        );
        assert!(store.read().await.contains_key(&key));

        let err = consume_agent_jwt_challenge(&store, &key, expires_at + Duration::seconds(1))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
        assert!(store.read().await.contains_key(&key));

        consume_agent_jwt_challenge(&store, &key, expires_at)
            .await
            .unwrap();
        assert!(!store.read().await.contains_key(&key));
    }

    async fn seeded_agent_state() -> (AppState, AgentId, SigningKey) {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: format!("user-{}", uuid::Uuid::now_v7()),
                password: "password".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let public_key = hex::encode(signing_key.verifying_key().to_bytes());
        let agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "agent".into(),
                public_key,
                owner_user_id: user.id,
            })
            .await
            .unwrap();
        let state = AppState {
            db,
            config: std::sync::Arc::new(crate::config::Config::default()),
            agent_jwt_challenges: Arc::new(RwLock::new(HashMap::new())),
            metrics: xlstatus_tsdb::MetricStore::in_memory(),
            realtime: crate::realtime::BroadcastHub::new(),
            session_registry: crate::grpc::SessionRegistry::new(),
            terminal_sessions: crate::api::v1::terminal::TerminalSessionRegistry::new(),
            io_registry: crate::grpc::IoRegistry::new(),
        };
        (state, agent.id, signing_key)
    }

    fn sign_challenge_request(
        signing_key: &SigningKey,
        agent_id: AgentId,
        request_nonce: &str,
        request_timestamp: i64,
    ) -> String {
        let payload =
            challenge_request_signature_payload(agent_id, request_nonce, request_timestamp);
        hex::encode(signing_key.sign(payload.as_bytes()).to_bytes())
    }
}
