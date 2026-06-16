use crate::db::{Agent, CreateAgentInput, CreateEnrollmentTokenInput, DatabaseBackend, EnrollmentToken};
use anyhow::Result;
use chrono::{DateTime, Utc};
use xlstatus_shared::{AgentId, UserId};

pub struct EnrollmentTokenRepository {
    db: DatabaseBackend,
}

impl EnrollmentTokenRepository {
    pub fn new(db: DatabaseBackend) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateEnrollmentTokenInput, token_hash: String) -> Result<EnrollmentToken> {
        let id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO enrollment_tokens (id, token_hash, created_by_user_id, expires_at, created_at)
                    VALUES (?, ?, ?, ?, ?)
                    "#,
                )
                .bind(&id)
                .bind(&token_hash)
                .bind(input.created_by_user_id.0.to_string())
                .bind(input.expires_at.to_rfc3339())
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO enrollment_tokens (id, token_hash, created_by_user_id, expires_at, created_at)
                    VALUES ($1, $2, $3, $4, $5)
                    "#,
                )
                .bind(uuid::Uuid::parse_str(&id)?)
                .bind(&token_hash)
                .bind(input.created_by_user_id.0)
                .bind(input.expires_at)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }

        Ok(EnrollmentToken {
            id,
            token_hash,
            created_by_user_id: input.created_by_user_id,
            expires_at: input.expires_at,
            used_at: None,
            used_by_agent_id: None,
            created_at: now,
        })
    }

    pub async fn find_and_use(&self, token_hash: &str, agent_id: AgentId) -> Result<Option<EnrollmentToken>> {
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                // Find unused, valid token
                let row = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, String)>(
                    "SELECT id, token_hash, created_by_user_id, expires_at, used_at, used_by_agent_id, created_at FROM enrollment_tokens WHERE token_hash = ? AND used_at IS NULL AND expires_at > ?",
                )
                .bind(token_hash)
                .bind(now.to_rfc3339())
                .fetch_optional(pool)
                .await?;

                if let Some((id, token_hash, created_by_user_id, expires_at, _, _, created_at)) = row {
                    // Mark as used
                    sqlx::query("UPDATE enrollment_tokens SET used_at = ?, used_by_agent_id = ? WHERE id = ?")
                        .bind(now.to_rfc3339())
                        .bind(agent_id.0.to_string())
                        .bind(&id)
                        .execute(pool)
                        .await?;

                    Ok(Some(EnrollmentToken {
                        id,
                        token_hash,
                        created_by_user_id: UserId(uuid::Uuid::parse_str(&created_by_user_id)?),
                        expires_at: DateTime::parse_from_rfc3339(&expires_at)?.with_timezone(&Utc),
                        used_at: Some(now),
                        used_by_agent_id: Some(agent_id),
                        created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
                    }))
                } else {
                    Ok(None)
                }
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, uuid::Uuid, DateTime<Utc>, Option<DateTime<Utc>>, Option<uuid::Uuid>, DateTime<Utc>)>(
                    "SELECT id, token_hash, created_by_user_id, expires_at, used_at, used_by_agent_id, created_at FROM enrollment_tokens WHERE token_hash = $1 AND used_at IS NULL AND expires_at > $2",
                )
                .bind(token_hash)
                .bind(now)
                .fetch_optional(pool)
                .await?;

                if let Some((id, token_hash, created_by_user_id, expires_at, _, _, created_at)) = row {
                    sqlx::query("UPDATE enrollment_tokens SET used_at = $1, used_by_agent_id = $2 WHERE id = $3")
                        .bind(now)
                        .bind(agent_id.0)
                        .bind(id)
                        .execute(pool)
                        .await?;

                    Ok(Some(EnrollmentToken {
                        id: id.to_string(),
                        token_hash,
                        created_by_user_id: UserId(created_by_user_id),
                        expires_at,
                        used_at: Some(now),
                        used_by_agent_id: Some(agent_id),
                        created_at,
                    }))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

pub struct AgentRepository {
    db: DatabaseBackend,
}

impl AgentRepository {
    pub fn new(db: DatabaseBackend) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateAgentInput) -> Result<Agent> {
        let id = AgentId::new();
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at)
                    VALUES (?, ?, ?, ?, ?, ?)
                    "#,
                )
                .bind(id.0.to_string())
                .bind(&input.name)
                .bind(&input.public_key)
                .bind(input.owner_user_id.0.to_string())
                .bind(now.to_rfc3339())
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at)
                    VALUES ($1, $2, $3, $4, $5, $6)
                    "#,
                )
                .bind(id.0)
                .bind(&input.name)
                .bind(&input.public_key)
                .bind(input.owner_user_id.0)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }

        Ok(Agent {
            id,
            name: input.name,
            public_key: input.public_key,
            owner_user_id: input.owner_user_id,
            last_seen_at: None,
            revoked_at: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn find_by_id(&self, id: AgentId) -> Result<Option<Agent>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, String, String)>(
                    "SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at FROM agents WHERE id = ?",
                )
                .bind(id.0.to_string())
                .fetch_optional(pool)
                .await?;

                Ok(row.map(|(id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at)| {
                    Agent {
                        id: AgentId(uuid::Uuid::parse_str(&id).unwrap()),
                        name,
                        public_key,
                        owner_user_id: UserId(uuid::Uuid::parse_str(&owner_user_id).unwrap()),
                        last_seen_at: last_seen_at.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                        revoked_at: revoked_at.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
                        created_at: DateTime::parse_from_rfc3339(&created_at).unwrap().with_timezone(&Utc),
                        updated_at: DateTime::parse_from_rfc3339(&updated_at).unwrap().with_timezone(&Utc),
                    }
                }))
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, String, uuid::Uuid, Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at FROM agents WHERE id = $1",
                )
                .bind(id.0)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(|(id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at)| {
                    Agent {
                        id: AgentId(id),
                        name,
                        public_key,
                        owner_user_id: UserId(owner_user_id),
                        last_seen_at,
                        revoked_at,
                        created_at,
                        updated_at,
                    }
                }))
            }
        }
    }

    pub async fn update_last_seen(&self, id: AgentId) -> Result<()> {
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query("UPDATE agents SET last_seen_at = ?, updated_at = ? WHERE id = ?")
                    .bind(now.to_rfc3339())
                    .bind(now.to_rfc3339())
                    .bind(id.0.to_string())
                    .execute(pool)
                    .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query("UPDATE agents SET last_seen_at = $1, updated_at = $2 WHERE id = $3")
                    .bind(now)
                    .bind(now)
                    .bind(id.0)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    pub async fn revoke(&self, id: AgentId) -> Result<bool> {
        let now = Utc::now();

        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let result = sqlx::query("UPDATE agents SET revoked_at = ?, updated_at = ? WHERE id = ? AND revoked_at IS NULL")
                    .bind(now.to_rfc3339())
                    .bind(now.to_rfc3339())
                    .bind(id.0.to_string())
                    .execute(pool)
                    .await?;
                result.rows_affected()
            }
            DatabaseBackend::Postgres(pool) => {
                let result = sqlx::query("UPDATE agents SET revoked_at = $1, updated_at = $2 WHERE id = $3 AND revoked_at IS NULL")
                    .bind(now)
                    .bind(now)
                    .bind(id.0)
                    .execute(pool)
                    .await?;
                result.rows_affected()
            }
        };

        Ok(affected > 0)
    }
}
