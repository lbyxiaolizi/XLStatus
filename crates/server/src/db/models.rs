#![allow(dead_code)]
#![allow(unused)]

use chrono::{DateTime, Utc};
use xlstatus_shared::{AgentId, UserId, UserRole};

#[derive(Debug, Clone)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,
    pub token_version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: UserId,
    pub token_hash: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PersonalAccessToken {
    pub id: String,
    pub user_id: UserId,
    pub name: String,
    pub token_hash: String,
    pub scopes: Vec<String>,
    pub server_ids: Option<Vec<String>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_used_ip: Option<String>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct EnrollmentToken {
    pub id: String,
    pub token_hash: String,
    pub created_by_user_id: UserId,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub used_by_agent_id: Option<AgentId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub public_key: String,
    pub owner_user_id: UserId,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateUserInput {
    pub username: String,
    pub password: String,
    pub role: UserRole,
}

#[derive(Debug, Clone)]
pub struct CreateSessionInput {
    pub user_id: UserId,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreatePATInput {
    pub user_id: UserId,
    pub name: String,
    pub scopes: Vec<String>,
    pub server_ids: Option<Vec<String>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateEnrollmentTokenInput {
    pub created_by_user_id: UserId,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateAgentInput {
    pub name: String,
    pub public_key: String,
    pub owner_user_id: UserId,
}

/// M3: read-side view of an agent with the most recent HostState
/// and HostInfo JSON columns. Used by the `/api/v1/servers` list and
/// detail endpoints so the rest of the codebase doesn't have to
/// grow a wider `Agent` struct just to expose two extra columns.
#[derive(Debug, Clone)]
pub struct AgentWithState {
    pub agent: Agent,
    pub remark: Option<String>,
    pub expires_at: Option<String>,
    pub renewal_price: Option<String>,
    pub dashboard_metadata_json: Option<String>,
    pub last_state_json: Option<String>,
    pub last_info_json: Option<String>,
}
