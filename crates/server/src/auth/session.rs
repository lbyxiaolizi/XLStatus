#![allow(dead_code)]
#![allow(unused)]

use crate::db::{CreateSessionInput, DatabaseBackend, Session};
use anyhow::Result;
use chrono::{DateTime, Utc};
use xlstatus_shared::UserId;

pub struct SessionRepository {
    db: DatabaseBackend,
}

impl SessionRepository {
    pub fn new(db: DatabaseBackend) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateSessionInput, token_hash: String) -> Result<Session> {
        let id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO sessions (id, user_id, token_hash, ip, user_agent, expires_at, created_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    "#,
                )
                .bind(&id)
                .bind(input.user_id.0.to_string())
                .bind(&token_hash)
                .bind(&input.ip)
                .bind(&input.user_agent)
                .bind(input.expires_at.to_rfc3339())
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                let pg_id = uuid::Uuid::parse_str(&id)
                    .map_err(|e| anyhow::anyhow!("session id must be uuid: {}", e))?;
                sqlx::query(
                    r#"
                    INSERT INTO sessions (id, user_id, token_hash, ip, user_agent, expires_at, created_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    "#,
                )
                .bind(pg_id)
                .bind(input.user_id.0)
                .bind(&token_hash)
                .bind(&input.ip)
                .bind(&input.user_agent)
                .bind(input.expires_at)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }

        Ok(Session {
            id,
            user_id: input.user_id,
            token_hash,
            ip: input.ip,
            user_agent: input.user_agent,
            expires_at: input.expires_at,
            created_at: now,
        })
    }

    pub async fn find_by_token_hash(&self, token_hash: &str) -> Result<Option<Session>> {
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, String, String)>(
                    "SELECT id, user_id, token_hash, ip, user_agent, expires_at, created_at FROM sessions WHERE token_hash = ? AND expires_at > ?",
                )
                .bind(token_hash)
                .bind(now.to_rfc3339())
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(id, user_id, token_hash, ip, user_agent, expires_at, created_at)| Session {
                        id,
                        user_id: UserId(uuid::Uuid::parse_str(&user_id).unwrap()),
                        token_hash,
                        ip,
                        user_agent,
                        expires_at: DateTime::parse_from_rfc3339(&expires_at)
                            .unwrap()
                            .with_timezone(&Utc),
                        created_at: DateTime::parse_from_rfc3339(&created_at)
                            .unwrap()
                            .with_timezone(&Utc),
                    },
                ))
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (String, uuid::Uuid, String, Option<String>, Option<String>, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, user_id, token_hash, ip, user_agent, expires_at, created_at FROM sessions WHERE token_hash = $1 AND expires_at > $2",
                )
                .bind(token_hash)
                .bind(now)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(id, user_id, token_hash, ip, user_agent, expires_at, created_at)| Session {
                        id,
                        user_id: UserId(user_id),
                        token_hash,
                        ip,
                        user_agent,
                        expires_at,
                        created_at,
                    },
                ))
            }
        }
    }

    pub async fn delete(&self, session_id: &str) -> Result<()> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query("DELETE FROM sessions WHERE id = ?")
                    .bind(session_id)
                    .execute(pool)
                    .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query("DELETE FROM sessions WHERE id = $1")
                    .bind(session_id)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn delete_expired(&self) -> Result<u64> {
        let now = Utc::now();

        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
                    .bind(now.to_rfc3339())
                    .execute(pool)
                    .await?;
                result.rows_affected()
            }
            DatabaseBackend::Postgres(pool) => {
                let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= $1")
                    .bind(now)
                    .execute(pool)
                    .await?;
                result.rows_affected()
            }
        };

        Ok(affected)
    }
}
