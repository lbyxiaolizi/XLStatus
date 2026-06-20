#![allow(dead_code)]
#![allow(unused)]

use crate::db::{
    Agent, AgentWithState, CreateAgentInput, CreateEnrollmentTokenInput, DatabaseBackend,
    EnrollmentToken,
};
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

    pub async fn create(
        &self,
        input: CreateEnrollmentTokenInput,
        token_hash: String,
    ) -> Result<EnrollmentToken> {
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

    pub async fn find_and_use(
        &self,
        token_hash: &str,
        agent_id: AgentId,
    ) -> Result<Option<EnrollmentToken>> {
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

                if let Some((id, token_hash, created_by_user_id, expires_at, _, _, created_at)) =
                    row
                {
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

                if let Some((id, token_hash, created_by_user_id, expires_at, _, _, created_at)) =
                    row
                {
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

                Ok(row.map(
                    |(
                        id,
                        name,
                        public_key,
                        owner_user_id,
                        last_seen_at,
                        revoked_at,
                        created_at,
                        updated_at,
                    )| {
                        Agent {
                            id: AgentId(uuid::Uuid::parse_str(&id).unwrap()),
                            name,
                            public_key,
                            owner_user_id: UserId(uuid::Uuid::parse_str(&owner_user_id).unwrap()),
                            last_seen_at: last_seen_at.and_then(|s| {
                                DateTime::parse_from_rfc3339(&s)
                                    .ok()
                                    .map(|dt| dt.with_timezone(&Utc))
                            }),
                            revoked_at: revoked_at.and_then(|s| {
                                DateTime::parse_from_rfc3339(&s)
                                    .ok()
                                    .map(|dt| dt.with_timezone(&Utc))
                            }),
                            created_at: DateTime::parse_from_rfc3339(&created_at)
                                .unwrap()
                                .with_timezone(&Utc),
                            updated_at: DateTime::parse_from_rfc3339(&updated_at)
                                .unwrap()
                                .with_timezone(&Utc),
                        }
                    },
                ))
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, String, uuid::Uuid, Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at FROM agents WHERE id = $1",
                )
                .bind(id.0)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(
                        id,
                        name,
                        public_key,
                        owner_user_id,
                        last_seen_at,
                        revoked_at,
                        created_at,
                        updated_at,
                    )| {
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
                    },
                ))
            }
        }
    }

    pub async fn list_by_owner(
        &self,
        owner_user_id: UserId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Agent>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, String, String)>(
                    r#"
                    SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at
                    FROM agents
                    WHERE owner_user_id = ?
                    ORDER BY created_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
                .bind(owner_user_id.0.to_string())
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;

                Ok(rows
                    .into_iter()
                    .map(
                        |(
                            id,
                            name,
                            public_key,
                            owner_user_id,
                            last_seen_at,
                            revoked_at,
                            created_at,
                            updated_at,
                        )| {
                            Agent {
                                id: AgentId(uuid::Uuid::parse_str(&id).unwrap()),
                                name,
                                public_key,
                                owner_user_id: UserId(
                                    uuid::Uuid::parse_str(&owner_user_id).unwrap(),
                                ),
                                last_seen_at: last_seen_at.and_then(|s| {
                                    DateTime::parse_from_rfc3339(&s)
                                        .ok()
                                        .map(|dt| dt.with_timezone(&Utc))
                                }),
                                revoked_at: revoked_at.and_then(|s| {
                                    DateTime::parse_from_rfc3339(&s)
                                        .ok()
                                        .map(|dt| dt.with_timezone(&Utc))
                                }),
                                created_at: DateTime::parse_from_rfc3339(&created_at)
                                    .unwrap()
                                    .with_timezone(&Utc),
                                updated_at: DateTime::parse_from_rfc3339(&updated_at)
                                    .unwrap()
                                    .with_timezone(&Utc),
                            }
                        },
                    )
                    .collect())
            }
            DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query_as::<_, (uuid::Uuid, String, String, uuid::Uuid, Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>, DateTime<Utc>)>(
                    r#"
                    SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at
                    FROM agents
                    WHERE owner_user_id = $1
                    ORDER BY created_at DESC
                    LIMIT $2 OFFSET $3
                    "#,
                )
                .bind(owner_user_id.0)
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;

                Ok(rows
                    .into_iter()
                    .map(
                        |(
                            id,
                            name,
                            public_key,
                            owner_user_id,
                            last_seen_at,
                            revoked_at,
                            created_at,
                            updated_at,
                        )| {
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
                        },
                    )
                    .collect())
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

    /// M3: persist the latest HostState JSON reported by the agent.
    /// The full TSDB (crates/tsdb) is deferred to M8 per plan/08-roadmap.md.
    pub async fn update_last_state(&self, id: AgentId, state_json: &str) -> Result<()> {
        let now = Utc::now();
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE agents SET last_state_json = ?, last_state_at = ?, updated_at = ? WHERE id = ?",
                )
                .bind(state_json)
                .bind(now.to_rfc3339())
                .bind(now.to_rfc3339())
                .bind(id.0.to_string())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    "UPDATE agents SET last_state_json = $1, last_state_at = $2, updated_at = $3 WHERE id = $4",
                )
                .bind(state_json)
                .bind(now)
                .bind(now)
                .bind(id.0)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    /// M3: persist the latest HostInfo JSON reported by the agent.
    pub async fn update_last_info(&self, id: AgentId, info_json: &str) -> Result<()> {
        let now = Utc::now();
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE agents SET last_info_json = ?, last_info_at = ?, updated_at = ? WHERE id = ?",
                )
                .bind(info_json)
                .bind(now.to_rfc3339())
                .bind(now.to_rfc3339())
                .bind(id.0.to_string())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    "UPDATE agents SET last_info_json = $1, last_info_at = $2, updated_at = $3 WHERE id = $4",
                )
                .bind(info_json)
                .bind(now)
                .bind(now)
                .bind(id.0)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    pub async fn update_dashboard_metadata(
        &self,
        id: AgentId,
        name: Option<&str>,
        remark: Option<&str>,
        expires_at: Option<&str>,
        renewal_price: Option<&str>,
        dashboard_metadata_json: Option<&str>,
    ) -> Result<bool> {
        let now = Utc::now();
        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let result = sqlx::query(
                    r#"
                    UPDATE agents
                       SET name = COALESCE(?, name),
                           remark = ?,
                           expires_at = ?,
                           renewal_price = ?,
                           dashboard_metadata_json = ?,
                           updated_at = ?
                     WHERE id = ?
                    "#,
                )
                .bind(name)
                .bind(remark)
                .bind(expires_at)
                .bind(renewal_price)
                .bind(dashboard_metadata_json)
                .bind(now.to_rfc3339())
                .bind(id.0.to_string())
                .execute(pool)
                .await?;
                result.rows_affected()
            }
            DatabaseBackend::Postgres(pool) => {
                let result = sqlx::query(
                    r#"
                    UPDATE agents
                       SET name = COALESCE($1, name),
                           remark = $2,
                           expires_at = $3,
                           renewal_price = $4,
                           dashboard_metadata_json = $5,
                           updated_at = $6
                     WHERE id = $7
                    "#,
                )
                .bind(name)
                .bind(remark)
                .bind(expires_at)
                .bind(renewal_price)
                .bind(dashboard_metadata_json)
                .bind(now)
                .bind(id.0)
                .execute(pool)
                .await?;
                result.rows_affected()
            }
        };

        Ok(affected > 0)
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

impl AgentRepository {
    /// M3: list all agents with their most recent HostState / HostInfo
    /// JSON columns. Used by `/api/v1/servers` so the dashboard can
    /// render CPU/memory/load without a second round-trip.
    pub async fn list_with_state(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<AgentWithState>, i64)> {
        let total = self.count_total().await?;
        let rows = match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)>(
                    r#"
                    SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at,
                           remark, expires_at, renewal_price, dashboard_metadata_json,
                           last_state_json, last_state_at, last_info_json, last_info_at
                    FROM agents
                    ORDER BY created_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;
                rows
            }
            DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query_as::<
                    _,
                    (
                        String,
                        String,
                        String,
                        String,
                        Option<String>,
                        Option<String>,
                        String,
                        String,
                        Option<String>,
                        Option<String>,
                        Option<String>,
                        Option<String>,
                        Option<String>,
                        Option<String>,
                        Option<String>,
                        Option<String>,
                    ),
                >(
                    r#"
                    SELECT id::text, name, public_key, owner_user_id::text,
                           to_char(last_seen_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                           to_char(revoked_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                           to_char(created_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                           to_char(updated_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                           remark, expires_at, renewal_price, dashboard_metadata_json,
                           last_state_json, last_state_at::text, last_info_json, last_info_at::text
                    FROM agents
                    ORDER BY created_at DESC
                    LIMIT $1 OFFSET $2
                    "#,
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;
                rows
            }
        };
        let out = rows
            .into_iter()
            .map(
                |(
                    id,
                    name,
                    public_key,
                    owner_user_id,
                    last_seen_at,
                    revoked_at,
                    created_at,
                    updated_at,
                    remark,
                    expires_at,
                    renewal_price,
                    dashboard_metadata_json,
                    last_state_json,
                    _last_state_at,
                    last_info_json,
                    _last_info_at,
                )| {
                    let agent = Agent {
                        id: AgentId(uuid::Uuid::parse_str(&id).unwrap()),
                        name,
                        public_key,
                        owner_user_id: UserId(uuid::Uuid::parse_str(&owner_user_id).unwrap()),
                        last_seen_at: last_seen_at.as_deref().and_then(parse_rfc3339_opt),
                        revoked_at: revoked_at.as_deref().and_then(parse_rfc3339_opt),
                        created_at: parse_rfc3339_opt(&created_at).unwrap_or_else(Utc::now),
                        updated_at: parse_rfc3339_opt(&updated_at).unwrap_or_else(Utc::now),
                    };
                    AgentWithState {
                        agent,
                        remark,
                        expires_at,
                        renewal_price,
                        dashboard_metadata_json,
                        last_state_json,
                        last_info_json,
                    }
                },
            )
            .collect();
        Ok((out, total))
    }

    pub async fn count_total(&self) -> Result<i64> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM agents")
                    .fetch_one(pool)
                    .await?;
                Ok(row.0)
            }
            DatabaseBackend::Postgres(pool) => {
                let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM agents")
                    .fetch_one(pool)
                    .await?;
                Ok(row.0)
            }
        }
    }
}

impl AgentRepository {
    /// M3: same as `find_by_id` but also returns the persisted
    /// `last_state_json` / `last_info_json` columns. Used by the
    /// `/api/v1/servers/:id` detail endpoint.
    pub async fn find_by_id_with_state(&self, id: AgentId) -> Result<Option<AgentWithState>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)>(
                    "SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at, remark, expires_at, renewal_price, dashboard_metadata_json, last_state_json, last_state_at, last_info_json, last_info_at FROM agents WHERE id = ?",
                )
                .bind(id.0.to_string())
                .fetch_optional(pool)
                .await?;
                Ok(row.map(
                    |(
                        id,
                        name,
                        public_key,
                        owner_user_id,
                        last_seen_at,
                        revoked_at,
                        created_at,
                        updated_at,
                        remark,
                        expires_at,
                        renewal_price,
                        dashboard_metadata_json,
                        last_state_json,
                        _last_state_at,
                        last_info_json,
                        _last_info_at,
                    )| {
                        AgentWithState {
                            agent: Agent {
                                id: AgentId(uuid::Uuid::parse_str(&id).unwrap()),
                                name,
                                public_key,
                                owner_user_id: UserId(
                                    uuid::Uuid::parse_str(&owner_user_id).unwrap(),
                                ),
                                last_seen_at: last_seen_at.as_deref().and_then(parse_rfc3339_opt),
                                revoked_at: revoked_at.as_deref().and_then(parse_rfc3339_opt),
                                created_at: parse_rfc3339_opt(&created_at).unwrap_or_else(Utc::now),
                                updated_at: parse_rfc3339_opt(&updated_at).unwrap_or_else(Utc::now),
                            },
                            remark,
                            expires_at,
                            renewal_price,
                            dashboard_metadata_json,
                            last_state_json,
                            last_info_json,
                        }
                    },
                ))
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, String, uuid::Uuid, Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>, DateTime<Utc>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<DateTime<Utc>>, Option<String>, Option<DateTime<Utc>>)>(
                    "SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at, remark, expires_at, renewal_price, dashboard_metadata_json, last_state_json, last_state_at, last_info_json, last_info_at FROM agents WHERE id = $1",
                )
                .bind(id.0)
                .fetch_optional(pool)
                .await?;
                Ok(row.map(
                    |(
                        id,
                        name,
                        public_key,
                        owner_user_id,
                        last_seen_at,
                        revoked_at,
                        created_at,
                        updated_at,
                        remark,
                        expires_at,
                        renewal_price,
                        dashboard_metadata_json,
                        last_state_json,
                        _last_state_at,
                        last_info_json,
                        _last_info_at,
                    )| {
                        AgentWithState {
                            agent: Agent {
                                id: AgentId(id),
                                name,
                                public_key,
                                owner_user_id: UserId(owner_user_id),
                                last_seen_at,
                                revoked_at,
                                created_at,
                                updated_at,
                            },
                            remark,
                            expires_at,
                            renewal_price,
                            dashboard_metadata_json,
                            last_state_json,
                            last_info_json,
                        }
                    },
                ))
            }
        }
    }
}

/// M3: best-effort rfc3339 parse that returns None on bad input.
fn parse_rfc3339_opt(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}
