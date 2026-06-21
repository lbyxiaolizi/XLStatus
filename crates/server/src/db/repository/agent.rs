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
                let result = sqlx::query(
                    "UPDATE enrollment_tokens SET used_at = ?, used_by_agent_id = ? WHERE token_hash = ? AND used_at IS NULL AND expires_at > ?",
                )
                .bind(now.to_rfc3339())
                .bind(agent_id.0.to_string())
                .bind(token_hash)
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
                if result.rows_affected() == 0 {
                    return Ok(None);
                }

                let row = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, String)>(
                    "SELECT id, token_hash, created_by_user_id, expires_at, used_at, used_by_agent_id, created_at FROM enrollment_tokens WHERE token_hash = ? AND used_by_agent_id = ?",
                )
                .bind(token_hash)
                .bind(agent_id.0.to_string())
                .fetch_optional(pool)
                .await?;

                let Some((
                    id,
                    token_hash,
                    created_by_user_id,
                    expires_at,
                    used_at,
                    used_by_agent_id,
                    created_at,
                )) = row
                else {
                    return Ok(None);
                };

                Ok(Some(EnrollmentToken {
                    id,
                    token_hash,
                    created_by_user_id: UserId(uuid::Uuid::parse_str(&created_by_user_id)?),
                    expires_at: DateTime::parse_from_rfc3339(&expires_at)?.with_timezone(&Utc),
                    used_at: used_at
                        .map(|value| {
                            DateTime::parse_from_rfc3339(&value).map(|dt| dt.with_timezone(&Utc))
                        })
                        .transpose()?,
                    used_by_agent_id: used_by_agent_id
                        .map(|value| uuid::Uuid::parse_str(&value).map(AgentId))
                        .transpose()?,
                    created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
                }))
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, uuid::Uuid, DateTime<Utc>, Option<DateTime<Utc>>, Option<uuid::Uuid>, DateTime<Utc>)>(
                    r#"
                    UPDATE enrollment_tokens
                    SET used_at = $1, used_by_agent_id = $2
                    WHERE token_hash = $3 AND used_at IS NULL AND expires_at > $1
                    RETURNING id, token_hash, created_by_user_id, expires_at, used_at, used_by_agent_id, created_at
                    "#,
                )
                .bind(now)
                .bind(agent_id.0)
                .bind(token_hash)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(
                        id,
                        token_hash,
                        created_by_user_id,
                        expires_at,
                        used_at,
                        used_by_agent_id,
                        created_at,
                    )| {
                        EnrollmentToken {
                            id: id.to_string(),
                            token_hash,
                            created_by_user_id: UserId(created_by_user_id),
                            expires_at,
                            used_at,
                            used_by_agent_id: used_by_agent_id.map(AgentId),
                            created_at,
                        }
                    },
                ))
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
        self.create_with_id(AgentId::new(), input).await
    }

    pub async fn create_with_id(&self, id: AgentId, input: CreateAgentInput) -> Result<Agent> {
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
        Ok((agent_with_state_rows(rows), total))
    }

    pub async fn list_with_state_by_owner(
        &self,
        owner_user_id: UserId,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<AgentWithState>, i64)> {
        let rows = match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)>(
                    r#"
                    SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at,
                           remark, expires_at, renewal_price, dashboard_metadata_json,
                           last_state_json, last_state_at, last_info_json, last_info_at
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
                .await?
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query_as::<
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
                    WHERE owner_user_id = $1
                    ORDER BY created_at DESC
                    LIMIT $2 OFFSET $3
                    "#,
                )
                .bind(owner_user_id.0)
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?
            }
        };
        let total = self.count_by_owner(owner_user_id).await?;
        Ok((agent_with_state_rows(rows), total))
    }

    pub async fn list_with_state_by_server_ids(
        &self,
        owner_user_id: Option<UserId>,
        server_ids: &[String],
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<AgentWithState>, i64)> {
        if server_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let placeholders = std::iter::repeat_n("?", server_ids.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let owner_filter = owner_user_id
                    .map(|_| " AND owner_user_id = ?")
                    .unwrap_or_default();
                let count_sql = format!(
                    "SELECT COUNT(*) FROM agents WHERE id IN ({placeholders}){owner_filter}"
                );
                let mut count_query = sqlx::query_as::<_, (i64,)>(&count_sql);
                for id in server_ids {
                    count_query = count_query.bind(id);
                }
                if let Some(owner) = owner_user_id {
                    count_query = count_query.bind(owner.0.to_string());
                }
                let total = count_query.fetch_one(pool).await?.0;

                let list_sql = format!(
                    r#"
                    SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at, created_at, updated_at,
                           remark, expires_at, renewal_price, dashboard_metadata_json,
                           last_state_json, last_state_at, last_info_json, last_info_at
                    FROM agents
                    WHERE id IN ({placeholders}){owner_filter}
                    ORDER BY created_at DESC
                    LIMIT ? OFFSET ?
                    "#
                );
                let mut list_query = sqlx::query_as::<_, AgentWithStateRow>(&list_sql);
                for id in server_ids {
                    list_query = list_query.bind(id);
                }
                if let Some(owner) = owner_user_id {
                    list_query = list_query.bind(owner.0.to_string());
                }
                let rows = list_query.bind(limit).bind(offset).fetch_all(pool).await?;
                Ok((agent_with_state_rows(rows), total))
            }
            DatabaseBackend::Postgres(pool) => {
                let parsed_ids: Vec<uuid::Uuid> = server_ids
                    .iter()
                    .map(|id| uuid::Uuid::parse_str(id))
                    .collect::<std::result::Result<_, _>>()?;
                let total = if let Some(owner) = owner_user_id {
                    let row: (i64,) = sqlx::query_as(
                        "SELECT COUNT(*) FROM agents WHERE id = ANY($1::uuid[]) AND owner_user_id = $2",
                    )
                    .bind(&parsed_ids)
                    .bind(owner.0)
                    .fetch_one(pool)
                    .await?;
                    row.0
                } else {
                    let row: (i64,) =
                        sqlx::query_as("SELECT COUNT(*) FROM agents WHERE id = ANY($1::uuid[])")
                            .bind(&parsed_ids)
                            .fetch_one(pool)
                            .await?;
                    row.0
                };

                let rows = if let Some(owner) = owner_user_id {
                    sqlx::query_as::<_, AgentWithStateRow>(
                        r#"
                        SELECT id::text, name, public_key, owner_user_id::text,
                               to_char(last_seen_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                               to_char(revoked_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                               to_char(created_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                               to_char(updated_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                               remark, expires_at, renewal_price, dashboard_metadata_json,
                               last_state_json, last_state_at::text, last_info_json, last_info_at::text
                        FROM agents
                        WHERE id = ANY($1::uuid[]) AND owner_user_id = $2
                        ORDER BY created_at DESC
                        LIMIT $3 OFFSET $4
                        "#,
                    )
                    .bind(&parsed_ids)
                    .bind(owner.0)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?
                } else {
                    sqlx::query_as::<_, AgentWithStateRow>(
                        r#"
                        SELECT id::text, name, public_key, owner_user_id::text,
                               to_char(last_seen_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                               to_char(revoked_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                               to_char(created_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                               to_char(updated_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                               remark, expires_at, renewal_price, dashboard_metadata_json,
                               last_state_json, last_state_at::text, last_info_json, last_info_at::text
                        FROM agents
                        WHERE id = ANY($1::uuid[])
                        ORDER BY created_at DESC
                        LIMIT $2 OFFSET $3
                        "#,
                    )
                    .bind(&parsed_ids)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?
                };
                Ok((agent_with_state_rows(rows), total))
            }
        }
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

    pub async fn count_by_owner(&self, owner_user_id: UserId) -> Result<i64> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM agents WHERE owner_user_id = ?")
                        .bind(owner_user_id.0.to_string())
                        .fetch_one(pool)
                        .await?;
                Ok(row.0)
            }
            DatabaseBackend::Postgres(pool) => {
                let row: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM agents WHERE owner_user_id = $1")
                        .bind(owner_user_id.0)
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

type AgentWithStateRow = (
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
);

fn agent_with_state_rows(rows: Vec<AgentWithStateRow>) -> Vec<AgentWithState> {
    rows.into_iter()
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
        .collect()
}

/// M3: best-effort rfc3339 parse that returns None on bad input.
fn parse_rfc3339_opt(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CreateUserInput, UserRepository};
    use xlstatus_shared::UserRole;

    #[tokio::test]
    async fn enrollment_token_can_only_be_consumed_once() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "password123".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let repo = EnrollmentTokenRepository::new(db);
        let token_hash = "hashed-token";
        repo.create(
            CreateEnrollmentTokenInput {
                created_by_user_id: user.id,
                expires_at: Utc::now() + chrono::Duration::hours(1),
            },
            token_hash.into(),
        )
        .await
        .unwrap();

        let first_agent = AgentId(uuid::Uuid::now_v7());
        let second_agent = AgentId(uuid::Uuid::now_v7());
        let first = repo.find_and_use(token_hash, first_agent).await.unwrap();
        let second = repo.find_and_use(token_hash, second_agent).await.unwrap();

        assert!(first.is_some());
        assert_eq!(first.unwrap().used_by_agent_id, Some(first_agent));
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn agent_can_be_created_with_preallocated_id_for_enrollment_audit() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner-fixed".into(),
                password: "password123".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let token_repo = EnrollmentTokenRepository::new(db.clone());
        let token_hash = "hashed-fixed-token";
        token_repo
            .create(
                CreateEnrollmentTokenInput {
                    created_by_user_id: user.id,
                    expires_at: Utc::now() + chrono::Duration::hours(1),
                },
                token_hash.into(),
            )
            .await
            .unwrap();

        let agent_id = AgentId(uuid::Uuid::now_v7());
        let token = token_repo
            .find_and_use(token_hash, agent_id)
            .await
            .unwrap()
            .unwrap();
        let agent = AgentRepository::new(db)
            .create_with_id(
                agent_id,
                CreateAgentInput {
                    name: "agent".into(),
                    public_key: "public-key".into(),
                    owner_user_id: token.created_by_user_id,
                },
            )
            .await
            .unwrap();

        assert_eq!(token.used_by_agent_id, Some(agent.id));
        assert_eq!(agent.id, agent_id);
    }

    async fn test_db() -> DatabaseBackend {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-agent-repo-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }
}
