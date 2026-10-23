#![allow(dead_code)]
#![allow(unused)]

use chrono::{DateTime, Utc};
use std::fmt;
use xlstatus_shared::{AgentId, UserId, UserRole};

const REDACTED_SECRET: &str = "[REDACTED]";

#[derive(Clone)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,
    pub token_version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl fmt::Debug for User {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("User")
            .field("id", &self.id)
            .field("username", &self.username)
            .field("password_hash", &REDACTED_SECRET)
            .field("role", &self.role)
            .field("token_version", &self.token_version)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone)]
pub struct Session {
    pub id: String,
    pub user_id: UserId,
    pub token_hash: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("token_hash", &REDACTED_SECRET)
            .field("ip", &self.ip)
            .field("user_agent", &self.user_agent)
            .field("expires_at", &self.expires_at)
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[derive(Clone)]
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

impl fmt::Debug for PersonalAccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersonalAccessToken")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("name", &self.name)
            .field("token_hash", &REDACTED_SECRET)
            .field("scopes", &self.scopes)
            .field("server_ids", &self.server_ids)
            .field("expires_at", &self.expires_at)
            .field("last_used_at", &self.last_used_at)
            .field("last_used_ip", &self.last_used_ip)
            .field("created_at", &self.created_at)
            .field("revoked_at", &self.revoked_at)
            .finish()
    }
}

#[derive(Clone)]
pub struct EnrollmentToken {
    pub id: String,
    pub token_hash: String,
    pub created_by_user_id: UserId,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub used_by_agent_id: Option<AgentId>,
    pub created_at: DateTime<Utc>,
}

impl fmt::Debug for EnrollmentToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnrollmentToken")
            .field("id", &self.id)
            .field("token_hash", &REDACTED_SECRET)
            .field("created_by_user_id", &self.created_by_user_id)
            .field("expires_at", &self.expires_at)
            .field("used_at", &self.used_at)
            .field("used_by_agent_id", &self.used_by_agent_id)
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[derive(Clone)]
pub struct TemporaryTransferToken {
    pub id: String,
    pub token_hash: String,
    pub server_id: AgentId,
    pub path: String,
    pub op: String,
    pub issued_by_user_id: UserId,
    pub auth_kind: String,
    pub session_id: Option<String>,
    pub api_token_id: Option<String>,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub used_ip: Option<String>,
    pub used_status: Option<String>,
    pub used_error: Option<String>,
    pub agent_task_id: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub created_ip: Option<String>,
}

impl fmt::Debug for TemporaryTransferToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TemporaryTransferToken")
            .field("id", &self.id)
            .field("token_hash", &REDACTED_SECRET)
            .field("server_id", &self.server_id)
            .field("path", &self.path)
            .field("op", &self.op)
            .field("issued_by_user_id", &self.issued_by_user_id)
            .field("auth_kind", &self.auth_kind)
            .field("session_id", &self.session_id)
            .field("api_token_id", &self.api_token_id)
            .field("scope", &self.scope)
            .field("expires_at", &self.expires_at)
            .field("used_at", &self.used_at)
            .field("used_ip", &self.used_ip)
            .field("used_status", &self.used_status)
            .field("used_error", &self.used_error)
            .field("agent_task_id", &self.agent_task_id)
            .field("revoked_at", &self.revoked_at)
            .field("created_at", &self.created_at)
            .field("created_ip", &self.created_ip)
            .finish()
    }
}

#[derive(Clone)]
pub struct CreateTemporaryTransferTokenInput {
    pub token_hash: String,
    pub server_id: AgentId,
    pub path: String,
    pub op: String,
    pub issued_by_user_id: UserId,
    pub auth_kind: String,
    pub session_id: Option<String>,
    pub api_token_id: Option<String>,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
    pub created_ip: Option<String>,
}

impl fmt::Debug for CreateTemporaryTransferTokenInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateTemporaryTransferTokenInput")
            .field("token_hash", &REDACTED_SECRET)
            .field("server_id", &self.server_id)
            .field("path", &self.path)
            .field("op", &self.op)
            .field("issued_by_user_id", &self.issued_by_user_id)
            .field("auth_kind", &self.auth_kind)
            .field("session_id", &self.session_id)
            .field("api_token_id", &self.api_token_id)
            .field("scope", &self.scope)
            .field("expires_at", &self.expires_at)
            .field("created_ip", &self.created_ip)
            .finish()
    }
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

#[derive(Clone)]
pub struct CreateUserInput {
    pub username: String,
    pub password: String,
    pub role: UserRole,
}

impl fmt::Debug for CreateUserInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateUserInput")
            .field("username", &self.username)
            .field("password", &REDACTED_SECRET)
            .field("role", &self.role)
            .finish()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_models_redact_credentials_and_hashes_in_debug() {
        let now = Utc::now();
        let user_id = UserId::new();
        let agent_id = AgentId::new();

        let user = User {
            id: user_id,
            username: "alice".into(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$supersecret".into(),
            role: UserRole::Admin,
            token_version: 1,
            created_at: now,
            updated_at: now,
        };
        let user_dbg = format!("{user:?}");
        assert!(user_dbg.contains("[REDACTED]"));
        assert!(!user_dbg.contains("supersecret"));

        let create_user = CreateUserInput {
            username: "alice".into(),
            password: "plaintext-password-123".into(),
            role: UserRole::Member,
        };
        let create_user_dbg = format!("{create_user:?}");
        assert!(create_user_dbg.contains("[REDACTED]"));
        assert!(!create_user_dbg.contains("plaintext-password-123"));

        let session = Session {
            id: "sess-1".into(),
            user_id,
            token_hash: "session-hash-secret".into(),
            ip: None,
            user_agent: None,
            expires_at: now,
            created_at: now,
        };
        let session_dbg = format!("{session:?}");
        assert!(session_dbg.contains("[REDACTED]"));
        assert!(!session_dbg.contains("session-hash-secret"));

        let pat = PersonalAccessToken {
            id: "pat-1".into(),
            user_id,
            name: "ci".into(),
            token_hash: "pat-hash-secret".into(),
            scopes: vec!["server:read".into()],
            server_ids: None,
            expires_at: Some(now),
            last_used_at: None,
            last_used_ip: None,
            created_at: now,
            revoked_at: None,
        };
        let pat_dbg = format!("{pat:?}");
        assert!(pat_dbg.contains("[REDACTED]"));
        assert!(!pat_dbg.contains("pat-hash-secret"));

        let enrollment = EnrollmentToken {
            id: "enr-1".into(),
            token_hash: "enrollment-hash-secret".into(),
            created_by_user_id: user_id,
            expires_at: now,
            used_at: None,
            used_by_agent_id: None,
            created_at: now,
        };
        let enrollment_dbg = format!("{enrollment:?}");
        assert!(enrollment_dbg.contains("[REDACTED]"));
        assert!(!enrollment_dbg.contains("enrollment-hash-secret"));

        let transfer = TemporaryTransferToken {
            id: "xlt-1".into(),
            token_hash: "transfer-hash-secret".into(),
            server_id: agent_id,
            path: "/tmp/a".into(),
            op: "download".into(),
            issued_by_user_id: user_id,
            auth_kind: "session".into(),
            session_id: None,
            api_token_id: None,
            scope: "transfer:read".into(),
            expires_at: now,
            used_at: None,
            used_ip: None,
            used_status: None,
            used_error: None,
            agent_task_id: None,
            revoked_at: None,
            created_at: now,
            created_ip: None,
        };
        let transfer_dbg = format!("{transfer:?}");
        assert!(transfer_dbg.contains("[REDACTED]"));
        assert!(!transfer_dbg.contains("transfer-hash-secret"));
    }
}
