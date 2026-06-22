use crate::api::types::*;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::hash_token;
use crate::auth::middleware::{AuthKind, AuthUser};
use crate::db::{
    AgentRepository, CreateAgentInput, CreateEnrollmentTokenInput, EnrollmentTokenRepository,
};
use crate::grpc::{IoRegistry, SessionRegistry};
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use xlstatus_proto_gen::xlstatus::v1::{server_message::Payload as ServerPayload, ServerMessage};
use xlstatus_shared::AgentId;

const DEFAULT_ENROLLMENT_TOKEN_TTL_HOURS: i64 = 1;
const MAX_ENROLLMENT_TOKEN_TTL_HOURS: i64 = 24;
pub(crate) const AGENT_AUTH_API_MAX_BODY_BYTES: usize = 4 * 1024;
const AGENT_RESOURCE_UUID_TEXT_LEN: usize = 36;

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
    auth_user: AuthUser,
    Json(req): Json<CreateEnrollmentTokenRequest>,
) -> Result<Json<ApiResponse<CreateEnrollmentTokenResponse>>, AppError> {
    require_admin_cookie_session(&auth_user)?;

    let repo = EnrollmentTokenRepository::new(state.db.clone());

    // Generate token
    let bytes: [u8; 32] = rand::random();
    let token = format!("xle_{}", hex::encode(bytes));
    let token_hash = hash_token(&token);

    let expires_in = normalize_enrollment_token_ttl(req.expires_in_hours)?;
    let expires_at = Utc::now() + Duration::hours(expires_in);

    let enrollment_token = repo
        .create(
            CreateEnrollmentTokenInput {
                created_by_user_id: auth_user.user.id,
                expires_at,
            },
            token_hash,
        )
        .await?;

    Ok(Json(ApiResponse::success(CreateEnrollmentTokenResponse {
        token,
        expires_at: enrollment_token.expires_at.to_rfc3339(),
    })))
}

pub fn agent_auth_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(AGENT_AUTH_API_MAX_BODY_BYTES)
}

pub async fn enroll(
    State(state): State<AppState>,
    Json(req): Json<EnrollRequest>,
) -> Result<Json<ApiResponse<EnrollResponse>>, AppError> {
    validate_enroll_request(&req)?;
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
        .create_with_id(
            agent_id,
            CreateAgentInput {
                name: req.name.clone(),
                public_key: req.public_key,
                owner_user_id: token.created_by_user_id.clone(),
            },
        )
        .await?;

    // M5: also create a `servers` row so the agent's id exists in
    // the table referenced by `task_runs.server_id`, `services.*`
    // and other monitored-resource foreign keys.
    let server_id = agent.id.0.to_string();
    let user_id_str = token.created_by_user_id.0.to_string();
    let now = chrono::Utc::now().to_rfc3339();
    match &state.db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO servers (id, owner_user_id, name, created_at, updated_at, agent_id) VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind(&server_id)
            .bind(&user_id_str)
            .bind(&req.name)
            .bind(&now)
            .bind(&now)
            .bind(&server_id)
            .execute(pool)
            .await?;
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let _ = sqlx::query(
                "INSERT INTO servers (id, owner_user_id, name, created_at, updated_at, agent_id) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"
            )
            .bind(agent.id.0)
            .bind(token.created_by_user_id.0)
            .bind(&req.name)
            .bind(agent.created_at)
            .bind(agent.updated_at)
            .bind(agent.id.0)
            .execute(pool)
            .await?;
        }
    }

    Ok(Json(ApiResponse::success(EnrollResponse {
        agent_id: agent.id.0.to_string(),
        name: agent.name,
    })))
}

/// M2: revoke an agent and immediately notify any connected session
/// via `ServerMessage::ForceDisconnect` so the agent tears down its
/// stream without waiting for the next reconnect.
pub async fn revoke_agent(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(agent_id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_admin_cookie_session(&auth_user)?;
    let agent_id = parse_agent_resource_id(&agent_id)?;
    let agent_repo = AgentRepository::new(state.db.clone());
    let revoked = agent_repo.revoke(AgentId(agent_id)).await?;
    if !revoked {
        return Err(AppError::NotFound(
            "agent not found or already revoked".into(),
        ));
    }
    let msg = ServerMessage {
        payload: Some(ServerPayload::ForceDisconnect(
            xlstatus_proto_gen::xlstatus::v1::ForceDisconnect {
                reason: "agent revoked by administrator".to_string(),
            },
        )),
    };
    // Session registry is process-local; not part of AppState today
    // to keep the type small. Resolve it from a sidecar Arc that
    // main.rs wires in.
    let registry = revoke_registry()
        .ok_or_else(|| AppError::BadRequest("revoke registry not initialized".into()))?;
    let agent_id = AgentId(agent_id);
    if let Err(e) = registry.session.send(&agent_id, msg).await {
        // No live session is fine; the next reconnect attempt will be
        // rejected at the JWT challenge step. We still return 200 so
        // the admin's revoke succeeds.
        tracing::debug!("no live session to force_disconnect: {}", e);
    }
    registry.session.disconnect(&agent_id).await;
    registry.io.disconnect_agent(&agent_id).await;
    if let Some(manager) = crate::current_nat_manager() {
        if let Err(e) = manager.reload().await {
            tracing::warn!("NAT manager reload failed after agent revoke: {}", e);
        }
    }
    Ok(Json(ApiResponse::success(serde_json::json!({
        "agent_id": agent_id.0.to_string(),
        "revoked": true,
    }))))
}

#[derive(Clone)]
pub struct RevokeRegistry {
    session: Arc<SessionRegistry>,
    io: Arc<IoRegistry>,
}

static REVOKE_REGISTRY: once_cell::sync::Lazy<parking_lot::RwLock<Option<RevokeRegistry>>> =
    once_cell::sync::Lazy::new(|| parking_lot::RwLock::new(None));

/// Wire the global session registry once at startup so the revoke
/// handler can reach it. main.rs calls this before binding.
pub fn set_revoke_registry(session: Arc<SessionRegistry>, io: Arc<IoRegistry>) {
    *REVOKE_REGISTRY.write() = Some(RevokeRegistry { session, io });
}

fn revoke_registry() -> Option<RevokeRegistry> {
    REVOKE_REGISTRY.read().clone()
}

fn parse_agent_resource_id(agent_id: &str) -> Result<uuid::Uuid, AppError> {
    if agent_id.len() != AGENT_RESOURCE_UUID_TEXT_LEN {
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
    Ok(parsed)
}

fn normalize_enrollment_token_ttl(expires_in_hours: Option<i64>) -> Result<i64, AppError> {
    let ttl = expires_in_hours.unwrap_or(DEFAULT_ENROLLMENT_TOKEN_TTL_HOURS);
    if !(1..=MAX_ENROLLMENT_TOKEN_TTL_HOURS).contains(&ttl) {
        return Err(AppError::BadRequest(format!(
            "expires_in_hours must be between 1 and {MAX_ENROLLMENT_TOKEN_TTL_HOURS}"
        )));
    }
    Ok(ttl)
}

fn validate_enroll_request(req: &EnrollRequest) -> Result<(), AppError> {
    let name = req.name.trim();
    if name.is_empty() || name.len() > 255 {
        return Err(AppError::BadRequest(
            "agent name must be between 1 and 255 characters".into(),
        ));
    }
    if !valid_agent_public_key(&req.public_key) {
        return Err(AppError::BadRequest(
            "agent public_key must be a 32-byte Ed25519 hex public key".into(),
        ));
    }
    if !req.enrollment_token.starts_with("xle_") || req.enrollment_token.len() != 68 {
        return Err(AppError::BadRequest("invalid enrollment token".to_string()));
    }
    Ok(())
}

fn valid_agent_public_key(public_key: &str) -> bool {
    public_key.len() == 64 && public_key.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn require_admin_cookie_session(auth_user: &AuthUser) -> Result<(), AppError> {
    if !auth_user.user.role.is_admin() {
        return Err(AppError::Forbidden("Admin role required".to_string()));
    }
    if matches!(auth_user.auth_kind, AuthKind::PersonalAccessToken) {
        return Err(AppError::Forbidden("Cookie session required".to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlstatus_shared::{UserId, UserRole};

    #[test]
    fn agent_global_admin_actions_reject_admin_pat() {
        let auth = auth_user(AuthKind::PersonalAccessToken, UserRole::Admin);
        let err = require_admin_cookie_session(&auth).unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn agent_global_admin_actions_allow_admin_cookie_session() {
        let auth = auth_user(AuthKind::Session, UserRole::Admin);
        assert!(require_admin_cookie_session(&auth).is_ok());
    }

    #[test]
    fn enrollment_token_ttl_is_bounded() {
        let _ = agent_auth_body_limit();
        assert_eq!(AGENT_AUTH_API_MAX_BODY_BYTES, 4 * 1024);
        assert_eq!(AGENT_RESOURCE_UUID_TEXT_LEN, 36);
        assert_eq!(normalize_enrollment_token_ttl(None).unwrap(), 1);
        assert_eq!(normalize_enrollment_token_ttl(Some(24)).unwrap(), 24);
        assert!(matches!(
            normalize_enrollment_token_ttl(Some(0)),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            normalize_enrollment_token_ttl(Some(25)),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn agent_resource_ids_require_canonical_uuid_text() {
        let agent_id = uuid::Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();

        assert_eq!(
            parse_agent_resource_id(&agent_id.to_string()).unwrap(),
            agent_id
        );
        assert!(parse_agent_resource_id("agent-a").is_err());
        assert!(parse_agent_resource_id(&format!(" {} ", agent_id)).is_err());
        assert!(parse_agent_resource_id(&agent_id.simple().to_string()).is_err());
        assert!(parse_agent_resource_id(&agent_id.to_string().to_uppercase()).is_err());
        assert!(parse_agent_resource_id(&"a".repeat(AGENT_RESOURCE_UUID_TEXT_LEN + 1)).is_err());
    }

    #[test]
    fn enroll_request_validates_before_token_consumption() {
        let valid = EnrollRequest {
            name: "agent".into(),
            enrollment_token: format!("xle_{}", "a".repeat(64)),
            public_key: "b".repeat(64),
        };
        assert!(validate_enroll_request(&valid).is_ok());

        let mut bad_key = valid;
        bad_key.public_key = "not-hex".into();
        assert!(matches!(
            validate_enroll_request(&bad_key),
            Err(AppError::BadRequest(_))
        ));

        let bad_token = EnrollRequest {
            name: "agent".into(),
            enrollment_token: "xle_short".into(),
            public_key: "b".repeat(64),
        };
        assert!(matches!(
            validate_enroll_request(&bad_token),
            Err(AppError::BadRequest(_))
        ));
    }

    fn auth_user(auth_kind: AuthKind, role: UserRole) -> AuthUser {
        AuthUser {
            user: crate::db::User {
                id: UserId(uuid::Uuid::from_bytes([1; 16])),
                username: "admin".into(),
                password_hash: "hash".into(),
                role,
                token_version: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            session_id: "session".into(),
            csrf_token: "csrf".into(),
            auth_kind,
            scopes: vec!["admin:*".into()],
            server_ids: None,
            pat_id: None,
        }
    }
}
