#![allow(dead_code)]
#![allow(unused)]

use crate::db::{CreateTemporaryTransferTokenInput, DatabaseBackend, TemporaryTransferToken};
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::Row;
use xlstatus_shared::{AgentId, UserId};

pub struct TemporaryTransferTokenRepository {
    db: DatabaseBackend,
}

const TEMP_TRANSFER_COLUMNS: &str = r#"
    id, token_hash, server_id, path, op, issued_by_user_id, auth_kind,
    session_id, api_token_id, scope, expires_at, used_at, used_ip,
    used_status, used_error, agent_task_id, revoked_at, created_at,
    created_ip
"#;

const TEMP_TRANSFER_COLUMNS_T: &str = r#"
    t.id, t.token_hash, t.server_id, t.path, t.op, t.issued_by_user_id, t.auth_kind,
    t.session_id, t.api_token_id, t.scope, t.expires_at, t.used_at, t.used_ip,
    t.used_status, t.used_error, t.agent_task_id, t.revoked_at, t.created_at,
    t.created_ip
"#;

impl TemporaryTransferTokenRepository {
    pub fn new(db: DatabaseBackend) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        input: CreateTemporaryTransferTokenInput,
    ) -> Result<TemporaryTransferToken> {
        let id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO temporary_transfer_tokens
                    (id, token_hash, server_id, path, op, issued_by_user_id, auth_kind,
                     session_id, api_token_id, scope, expires_at, created_at, created_ip)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                )
                .bind(&id)
                .bind(&input.token_hash)
                .bind(input.server_id.0.to_string())
                .bind(&input.path)
                .bind(&input.op)
                .bind(input.issued_by_user_id.0.to_string())
                .bind(&input.auth_kind)
                .bind(input.session_id.as_deref())
                .bind(input.api_token_id.as_deref())
                .bind(&input.scope)
                .bind(input.expires_at.to_rfc3339())
                .bind(now.to_rfc3339())
                .bind(input.created_ip.as_deref())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                let server_id = input.server_id.0;
                let issued_by_user_id = input.issued_by_user_id.0;
                let session_id = input
                    .session_id
                    .as_deref()
                    .map(uuid::Uuid::parse_str)
                    .transpose()?;
                let api_token_id = input
                    .api_token_id
                    .as_deref()
                    .map(uuid::Uuid::parse_str)
                    .transpose()?;
                sqlx::query(
                    r#"
                    INSERT INTO temporary_transfer_tokens
                    (id, token_hash, server_id, path, op, issued_by_user_id, auth_kind,
                     session_id, api_token_id, scope, expires_at, created_at, created_ip)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                    "#,
                )
                .bind(&id)
                .bind(&input.token_hash)
                .bind(server_id)
                .bind(&input.path)
                .bind(&input.op)
                .bind(issued_by_user_id)
                .bind(&input.auth_kind)
                .bind(session_id)
                .bind(api_token_id)
                .bind(&input.scope)
                .bind(input.expires_at)
                .bind(now)
                .bind(input.created_ip.as_deref())
                .execute(pool)
                .await?;
            }
        }

        Ok(TemporaryTransferToken {
            id,
            token_hash: input.token_hash,
            server_id: input.server_id,
            path: input.path,
            op: input.op,
            issued_by_user_id: input.issued_by_user_id,
            auth_kind: input.auth_kind,
            session_id: input.session_id,
            api_token_id: input.api_token_id,
            scope: input.scope,
            expires_at: input.expires_at,
            used_at: None,
            used_ip: None,
            used_status: None,
            used_error: None,
            agent_task_id: None,
            revoked_at: None,
            created_at: now,
            created_ip: input.created_ip,
        })
    }

    pub async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<TemporaryTransferToken>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let sql = format!(
                    "SELECT {TEMP_TRANSFER_COLUMNS} FROM temporary_transfer_tokens WHERE token_hash = ?"
                );
                let row = sqlx::query(&sql)
                    .bind(token_hash)
                    .fetch_optional(pool)
                    .await?;
                row.map(sqlite_row_to_token).transpose()
            }
            DatabaseBackend::Postgres(pool) => {
                let sql = format!(
                    "SELECT {TEMP_TRANSFER_COLUMNS} FROM temporary_transfer_tokens WHERE token_hash = $1"
                );
                let row = sqlx::query(&sql)
                    .bind(token_hash)
                    .fetch_optional(pool)
                    .await?;
                Ok(row.map(postgres_row_to_token))
            }
        }
    }

    pub async fn find_by_id(&self, id: &str) -> Result<Option<TemporaryTransferToken>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let sql = format!(
                    "SELECT {TEMP_TRANSFER_COLUMNS} FROM temporary_transfer_tokens WHERE id = ?"
                );
                let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
                row.map(sqlite_row_to_token).transpose()
            }
            DatabaseBackend::Postgres(pool) => {
                let sql = format!(
                    "SELECT {TEMP_TRANSFER_COLUMNS} FROM temporary_transfer_tokens WHERE id = $1"
                );
                let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
                Ok(row.map(postgres_row_to_token))
            }
        }
    }

    pub async fn list(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<TemporaryTransferToken>, i64)> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let total: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM temporary_transfer_tokens")
                        .fetch_one(pool)
                        .await?;
                let sql = format!(
                    "SELECT {TEMP_TRANSFER_COLUMNS} FROM temporary_transfer_tokens ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
                );
                let rows = sqlx::query(&sql)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;
                let tokens = rows
                    .into_iter()
                    .map(sqlite_row_to_token)
                    .collect::<Result<Vec<_>>>()?;
                Ok((tokens, total.0))
            }
            DatabaseBackend::Postgres(pool) => {
                let total: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM temporary_transfer_tokens")
                        .fetch_one(pool)
                        .await?;
                let sql = format!(
                    "SELECT {TEMP_TRANSFER_COLUMNS} FROM temporary_transfer_tokens ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2"
                );
                let rows = sqlx::query(&sql)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;
                Ok((
                    rows.into_iter().map(postgres_row_to_token).collect(),
                    total.0,
                ))
            }
        }
    }

    pub async fn list_for_owner(
        &self,
        owner_user_id: UserId,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<TemporaryTransferToken>, i64)> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let owner = owner_user_id.0.to_string();
                let total: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM temporary_transfer_tokens WHERE issued_by_user_id = ?",
                )
                .bind(&owner)
                .fetch_one(pool)
                .await?;
                let sql = format!(
                    "SELECT {TEMP_TRANSFER_COLUMNS} FROM temporary_transfer_tokens WHERE issued_by_user_id = ? ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
                );
                let rows = sqlx::query(&sql)
                    .bind(&owner)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;
                let tokens = rows
                    .into_iter()
                    .map(sqlite_row_to_token)
                    .collect::<Result<Vec<_>>>()?;
                Ok((tokens, total.0))
            }
            DatabaseBackend::Postgres(pool) => {
                let total: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM temporary_transfer_tokens WHERE issued_by_user_id = $1",
                )
                .bind(owner_user_id.0)
                .fetch_one(pool)
                .await?;
                let sql = format!(
                    "SELECT {TEMP_TRANSFER_COLUMNS} FROM temporary_transfer_tokens WHERE issued_by_user_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2 OFFSET $3"
                );
                let rows = sqlx::query(&sql)
                    .bind(owner_user_id.0)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;
                Ok((
                    rows.into_iter().map(postgres_row_to_token).collect(),
                    total.0,
                ))
            }
        }
    }

    pub async fn list_for_owner_server_ids(
        &self,
        owner_user_id: UserId,
        server_ids: &[String],
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<TemporaryTransferToken>, i64)> {
        if server_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let owner = owner_user_id.0.to_string();
                let values = sqlite_placeholders(server_ids.len());
                let count_sql = format!(
                    "WITH allowed(server_id) AS (VALUES {values}) SELECT COUNT(*) FROM temporary_transfer_tokens t JOIN allowed a ON a.server_id = t.server_id WHERE t.issued_by_user_id = ?"
                );
                let mut count_query = sqlx::query_as::<_, (i64,)>(&count_sql);
                for id in server_ids {
                    count_query = count_query.bind(id);
                }
                let total = count_query.bind(&owner).fetch_one(pool).await?;

                let list_sql = format!(
                    "WITH allowed(server_id) AS (VALUES {values}) SELECT {TEMP_TRANSFER_COLUMNS_T} FROM temporary_transfer_tokens t JOIN allowed a ON a.server_id = t.server_id WHERE t.issued_by_user_id = ? ORDER BY t.created_at DESC, t.id DESC LIMIT ? OFFSET ?"
                );
                let mut list_query = sqlx::query(&list_sql);
                for id in server_ids {
                    list_query = list_query.bind(id);
                }
                let rows = list_query
                    .bind(&owner)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;
                let tokens = rows
                    .into_iter()
                    .map(sqlite_row_to_token)
                    .collect::<Result<Vec<_>>>()?;
                Ok((tokens, total.0))
            }
            DatabaseBackend::Postgres(pool) => {
                let parsed = parse_uuid_ids(server_ids)?;
                let total: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM temporary_transfer_tokens WHERE issued_by_user_id = $1 AND server_id = ANY($2::uuid[])",
                )
                .bind(owner_user_id.0)
                .bind(&parsed)
                .fetch_one(pool)
                .await?;
                let sql = format!(
                    "SELECT {TEMP_TRANSFER_COLUMNS} FROM temporary_transfer_tokens WHERE issued_by_user_id = $1 AND server_id = ANY($2::uuid[]) ORDER BY created_at DESC, id DESC LIMIT $3 OFFSET $4"
                );
                let rows = sqlx::query(&sql)
                    .bind(owner_user_id.0)
                    .bind(&parsed)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;
                Ok((
                    rows.into_iter().map(postgres_row_to_token).collect(),
                    total.0,
                ))
            }
        }
    }

    pub async fn revoke(&self, id: &str) -> Result<bool> {
        let now = Utc::now();
        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => sqlx::query(
                r#"
                    UPDATE temporary_transfer_tokens
                    SET revoked_at = ?
                    WHERE id = ? AND revoked_at IS NULL
                    "#,
            )
            .bind(now.to_rfc3339())
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected(),
            DatabaseBackend::Postgres(pool) => sqlx::query(
                r#"
                    UPDATE temporary_transfer_tokens
                    SET revoked_at = $1
                    WHERE id = $2 AND revoked_at IS NULL
                    "#,
            )
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected(),
        };
        Ok(affected == 1)
    }

    pub async fn mark_used_once(&self, id: &str, used_ip: Option<&str>) -> Result<bool> {
        let now = Utc::now();
        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => sqlx::query(
                r#"
                    UPDATE temporary_transfer_tokens
                    SET used_at = ?, used_ip = ?, used_status = 'started'
                    WHERE id = ? AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ?
                    "#,
            )
            .bind(now.to_rfc3339())
            .bind(used_ip)
            .bind(id)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?
            .rows_affected(),
            DatabaseBackend::Postgres(pool) => sqlx::query(
                r#"
                    UPDATE temporary_transfer_tokens
                    SET used_at = $1, used_ip = $2, used_status = 'started'
                    WHERE id = $3 AND used_at IS NULL AND revoked_at IS NULL AND expires_at > $1
                    "#,
            )
            .bind(now)
            .bind(used_ip)
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected(),
        };
        Ok(affected == 1)
    }

    pub async fn record_use_result(
        &self,
        id: &str,
        status: &str,
        agent_task_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let error = error.map(truncate_transfer_error);
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    r#"
                    UPDATE temporary_transfer_tokens
                    SET used_status = ?, agent_task_id = COALESCE(?, agent_task_id), used_error = ?
                    WHERE id = ?
                    "#,
                )
                .bind(status)
                .bind(agent_task_id)
                .bind(error.as_deref())
                .bind(id)
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    r#"
                    UPDATE temporary_transfer_tokens
                    SET used_status = $1, agent_task_id = COALESCE($2, agent_task_id), used_error = $3
                    WHERE id = $4
                    "#,
                )
                .bind(status)
                .bind(agent_task_id)
                .bind(error.as_deref())
                .bind(id)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }
}

fn truncate_transfer_error(error: &str) -> String {
    const MAX_ERROR_CHARS: usize = 2000;
    error.chars().take(MAX_ERROR_CHARS).collect()
}

fn sqlite_placeholders(len: usize) -> String {
    std::iter::repeat_n("(?)", len)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_uuid_ids(ids: &[String]) -> Result<Vec<uuid::Uuid>> {
    ids.iter()
        .map(|id| uuid::Uuid::parse_str(id).map_err(Into::into))
        .collect()
}

fn sqlite_row_to_token(row: sqlx::sqlite::SqliteRow) -> Result<TemporaryTransferToken> {
    let id: String = row.try_get("id")?;
    let token_hash: String = row.try_get("token_hash")?;
    let server_id: String = row.try_get("server_id")?;
    let path: String = row.try_get("path")?;
    let op: String = row.try_get("op")?;
    let issued_by_user_id: String = row.try_get("issued_by_user_id")?;
    let auth_kind: String = row.try_get("auth_kind")?;
    let session_id: Option<String> = row.try_get("session_id")?;
    let api_token_id: Option<String> = row.try_get("api_token_id")?;
    let scope: String = row.try_get("scope")?;
    let expires_at: String = row.try_get("expires_at")?;
    let used_at: Option<String> = row.try_get("used_at")?;
    let used_ip: Option<String> = row.try_get("used_ip")?;
    let used_status: Option<String> = row.try_get("used_status")?;
    let used_error: Option<String> = row.try_get("used_error")?;
    let agent_task_id: Option<String> = row.try_get("agent_task_id")?;
    let revoked_at: Option<String> = row.try_get("revoked_at")?;
    let created_at: String = row.try_get("created_at")?;
    let created_ip: Option<String> = row.try_get("created_ip")?;
    Ok(TemporaryTransferToken {
        id,
        token_hash,
        server_id: AgentId(uuid::Uuid::parse_str(&server_id)?),
        path,
        op,
        issued_by_user_id: UserId(uuid::Uuid::parse_str(&issued_by_user_id)?),
        auth_kind,
        session_id,
        api_token_id,
        scope,
        expires_at: DateTime::parse_from_rfc3339(&expires_at)?.with_timezone(&Utc),
        used_at: used_at
            .map(|value| DateTime::parse_from_rfc3339(&value))
            .transpose()?
            .map(|dt| dt.with_timezone(&Utc)),
        used_ip,
        used_status,
        used_error,
        agent_task_id,
        revoked_at: revoked_at
            .map(|value| DateTime::parse_from_rfc3339(&value))
            .transpose()?
            .map(|dt| dt.with_timezone(&Utc)),
        created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
        created_ip,
    })
}

fn postgres_row_to_token(row: sqlx::postgres::PgRow) -> TemporaryTransferToken {
    let id: String = row.get("id");
    let token_hash: String = row.get("token_hash");
    let server_id: uuid::Uuid = row.get("server_id");
    let path: String = row.get("path");
    let op: String = row.get("op");
    let issued_by_user_id: uuid::Uuid = row.get("issued_by_user_id");
    let auth_kind: String = row.get("auth_kind");
    let session_id: Option<uuid::Uuid> = row.get("session_id");
    let api_token_id: Option<uuid::Uuid> = row.get("api_token_id");
    let scope: String = row.get("scope");
    let expires_at: DateTime<Utc> = row.get("expires_at");
    let used_at: Option<DateTime<Utc>> = row.get("used_at");
    let used_ip: Option<String> = row.get("used_ip");
    let used_status: Option<String> = row.get("used_status");
    let used_error: Option<String> = row.get("used_error");
    let agent_task_id: Option<String> = row.get("agent_task_id");
    let revoked_at: Option<DateTime<Utc>> = row.get("revoked_at");
    let created_at: DateTime<Utc> = row.get("created_at");
    let created_ip: Option<String> = row.get("created_ip");
    TemporaryTransferToken {
        id,
        token_hash,
        server_id: AgentId(server_id),
        path,
        op,
        issued_by_user_id: UserId(issued_by_user_id),
        auth_kind,
        session_id: session_id.map(|id| id.to_string()),
        api_token_id: api_token_id.map(|id| id.to_string()),
        scope,
        expires_at,
        used_at,
        used_ip,
        used_status,
        used_error,
        agent_task_id,
        revoked_at,
        created_at,
        created_ip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::hash_token;
    use crate::db::{AgentRepository, DatabaseBackend, UserRepository};
    use crate::db::{CreateAgentInput, CreateTemporaryTransferTokenInput, CreateUserInput};
    use xlstatus_shared::UserRole;

    #[tokio::test]
    async fn temporary_transfer_token_is_one_time_use() {
        let (repo, created, token_hash) = seeded_token("xlt_test").await;

        let found = repo.find_by_token_hash(&token_hash).await.unwrap().unwrap();
        assert_eq!(found.id, created.id);
        assert_eq!(found.path, "/tmp/file.txt");
        assert!(repo
            .mark_used_once(&created.id, Some("203.0.113.10"))
            .await
            .unwrap());
        assert!(!repo
            .mark_used_once(&created.id, Some("203.0.113.11"))
            .await
            .unwrap());
        repo.record_use_result(&created.id, "success", Some("task-1"), None)
            .await
            .unwrap();
        let used = repo.find_by_id(&created.id).await.unwrap().unwrap();
        assert_eq!(used.used_ip.as_deref(), Some("203.0.113.10"));
        assert_eq!(used.used_status.as_deref(), Some("success"));
        assert_eq!(used.agent_task_id.as_deref(), Some("task-1"));
        assert!(used.used_error.is_none());
    }

    #[tokio::test]
    async fn temporary_transfer_tokens_can_be_listed_and_revoked() {
        let (repo, created, _) = seeded_token("xlt_revoke").await;

        let (tokens, total) = repo.list(10, 0).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].id, created.id);
        assert!(tokens[0].revoked_at.is_none());

        assert!(repo.revoke(&created.id).await.unwrap());
        assert!(!repo.revoke(&created.id).await.unwrap());
        let revoked = repo.find_by_id(&created.id).await.unwrap().unwrap();
        assert!(revoked.revoked_at.is_some());
        assert!(!repo.mark_used_once(&created.id, None).await.unwrap());
    }

    #[tokio::test]
    async fn temporary_transfer_list_filters_by_owner_and_server_allowlist_in_sql() {
        let db = test_db().await;
        let user_repo = UserRepository::new(db.clone());
        let owner = user_repo
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let other_owner = user_repo
            .create(CreateUserInput {
                username: "other".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let agent_repo = AgentRepository::new(db.clone());
        let allowed_agent = agent_repo
            .create(CreateAgentInput {
                name: "allowed".into(),
                public_key: "public".into(),
                owner_user_id: owner.id,
            })
            .await
            .unwrap();
        let blocked_agent = agent_repo
            .create(CreateAgentInput {
                name: "blocked".into(),
                public_key: "public".into(),
                owner_user_id: owner.id,
            })
            .await
            .unwrap();
        let other_agent = agent_repo
            .create(CreateAgentInput {
                name: "other-agent".into(),
                public_key: "public".into(),
                owner_user_id: other_owner.id,
            })
            .await
            .unwrap();
        let repo = TemporaryTransferTokenRepository::new(db);
        let allowed =
            create_token_for(&repo, "xlt_allowed", owner.id, allowed_agent.id, "download").await;
        let blocked =
            create_token_for(&repo, "xlt_blocked", owner.id, blocked_agent.id, "download").await;
        let other = create_token_for(
            &repo,
            "xlt_other",
            other_owner.id,
            other_agent.id,
            "download",
        )
        .await;

        let (owner_tokens, owner_total) = repo.list_for_owner(owner.id, 10, 0).await.unwrap();
        assert_eq!(owner_total, 2);
        let owner_ids: Vec<_> = owner_tokens.iter().map(|token| token.id.as_str()).collect();
        assert!(owner_ids.contains(&allowed.id.as_str()));
        assert!(owner_ids.contains(&blocked.id.as_str()));
        assert!(!owner_ids.contains(&other.id.as_str()));

        let (allowed_tokens, allowed_total) = repo
            .list_for_owner_server_ids(owner.id, &[allowed_agent.id.0.to_string()], 10, 0)
            .await
            .unwrap();
        assert_eq!(allowed_total, 1);
        assert_eq!(allowed_tokens.len(), 1);
        assert_eq!(allowed_tokens[0].id, allowed.id);
    }

    async fn seeded_token(
        token: &str,
    ) -> (
        TemporaryTransferTokenRepository,
        TemporaryTransferToken,
        String,
    ) {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "agent".into(),
                public_key: "public".into(),
                owner_user_id: user.id,
            })
            .await
            .unwrap();

        let repo = TemporaryTransferTokenRepository::new(db);
        let token_hash = hash_token(token);
        let created = repo
            .create(CreateTemporaryTransferTokenInput {
                token_hash: token_hash.clone(),
                server_id: agent.id,
                path: "/tmp/file.txt".into(),
                op: "download".into(),
                issued_by_user_id: user.id,
                auth_kind: "session".into(),
                session_id: None,
                api_token_id: None,
                scope: "transfer:read".into(),
                expires_at: Utc::now() + chrono::Duration::minutes(5),
                created_ip: Some("127.0.0.1".into()),
            })
            .await
            .unwrap();
        (repo, created, token_hash)
    }

    async fn create_token_for(
        repo: &TemporaryTransferTokenRepository,
        token: &str,
        owner_id: UserId,
        server_id: AgentId,
        op: &str,
    ) -> TemporaryTransferToken {
        repo.create(CreateTemporaryTransferTokenInput {
            token_hash: hash_token(token),
            server_id,
            path: "/tmp/file.txt".into(),
            op: op.into(),
            issued_by_user_id: owner_id,
            auth_kind: "session".into(),
            session_id: None,
            api_token_id: None,
            scope: "transfer:read".into(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
            created_ip: Some("127.0.0.1".into()),
        })
        .await
        .unwrap()
    }

    async fn test_db() -> DatabaseBackend {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-temp-transfer-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }
}
