use crate::api::types::*;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::hash_token;
use crate::auth::middleware::AuthUser;
use crate::db::{
    AgentRepository, CreateAgentInput, CreateEnrollmentTokenInput, EnrollmentTokenRepository,
};
use crate::grpc::SessionRegistry;
use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use xlstatus_proto_gen::xlstatus::v1::{server_message::Payload as ServerPayload, ServerMessage};
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
    auth_user: AuthUser,
    Json(req): Json<CreateEnrollmentTokenRequest>,
) -> Result<Json<ApiResponse<CreateEnrollmentTokenResponse>>, AppError> {
    if !auth_user.user.role.is_admin() {
        return Err(AppError::Forbidden("Admin role required".to_string()));
    }

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
            name: req.name.clone(),
            public_key: req.public_key,
            owner_user_id: token.created_by_user_id.clone(),
        })
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
            .await;
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let _ = sqlx::query(
                "INSERT INTO servers (id, owner_user_id, name, created_at, updated_at, agent_id) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (id) DO NOTHING"
            )
            .bind(&server_id)
            .bind(&user_id_str)
            .bind(&req.name)
            .bind(&now)
            .bind(&now)
            .bind(&server_id)
            .execute(pool)
            .await;
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
    if !auth_user.user.role.is_admin() {
        return Err(AppError::Forbidden("Admin role required".to_string()));
    }
    let agent_id = uuid::Uuid::parse_str(&agent_id)
        .map_err(|_| AppError::BadRequest(format!("invalid agent id: {agent_id}")))?;
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
    if let Err(e) = registry.send(&AgentId(agent_id), msg).await {
        // No live session is fine; the next reconnect attempt will be
        // rejected at the JWT challenge step. We still return 200 so
        // the admin's revoke succeeds.
        tracing::debug!("no live session to force_disconnect: {}", e);
    }
    Ok(Json(ApiResponse::success(serde_json::json!({
        "agent_id": agent_id.to_string(),
        "revoked": true,
    }))))
}

static REVOKE_REGISTRY: once_cell::sync::Lazy<parking_lot::RwLock<Option<Arc<SessionRegistry>>>> =
    once_cell::sync::Lazy::new(|| parking_lot::RwLock::new(None));

/// Wire the global session registry once at startup so the revoke
/// handler can reach it. main.rs calls this before binding.
pub fn set_revoke_registry(registry: Arc<SessionRegistry>) {
    *REVOKE_REGISTRY.write() = Some(registry);
}

fn revoke_registry() -> Option<Arc<SessionRegistry>> {
    REVOKE_REGISTRY.read().clone()
}
