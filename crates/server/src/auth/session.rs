#![allow(dead_code)]
#![allow(unused)]

use crate::db::{CreateSessionInput, DatabaseBackend, Session};
use anyhow::{Context, Result};
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

                match row {
                    Some((id, user_id, token_hash, ip, user_agent, expires_at, created_at)) => {
                        let session_id = id.clone();
                        match row_to_session_sqlite(
                            id, user_id, token_hash, ip, user_agent, expires_at, created_at,
                        ) {
                            Ok(session) => Ok(Some(session)),
                            Err(err) => {
                                tracing::warn!(
                                    session_id = %session_id,
                                    "treating invalid session row as not found: {err}"
                                );
                                Ok(None)
                            }
                        }
                    }
                    None => Ok(None),
                }
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, Option<String>, Option<String>, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, user_id, token_hash, ip, user_agent, expires_at, created_at FROM sessions WHERE token_hash = $1 AND expires_at > $2",
                )
                .bind(token_hash)
                .bind(now)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(id, user_id, token_hash, ip, user_agent, expires_at, created_at)| Session {
                        id: id.to_string(),
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

    pub async fn find_by_id(&self, session_id: &str) -> Result<Option<Session>> {
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, String, String)>(
                    "SELECT id, user_id, token_hash, ip, user_agent, expires_at, created_at FROM sessions WHERE id = ? AND expires_at > ?",
                )
                .bind(session_id)
                .bind(now.to_rfc3339())
                .fetch_optional(pool)
                .await?;

                match row {
                    Some((id, user_id, token_hash, ip, user_agent, expires_at, created_at)) => {
                        let session_id = id.clone();
                        match row_to_session_sqlite(
                            id, user_id, token_hash, ip, user_agent, expires_at, created_at,
                        ) {
                            Ok(session) => Ok(Some(session)),
                            Err(err) => {
                                tracing::warn!(
                                    session_id = %session_id,
                                    "treating invalid session row as not found: {err}"
                                );
                                Ok(None)
                            }
                        }
                    }
                    None => Ok(None),
                }
            }
            DatabaseBackend::Postgres(pool) => {
                let session_uuid = uuid::Uuid::parse_str(session_id)
                    .map_err(|e| anyhow::anyhow!("invalid session id: {}", e))?;
                let row = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, Option<String>, Option<String>, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, user_id, token_hash, ip, user_agent, expires_at, created_at FROM sessions WHERE id = $1 AND expires_at > $2",
                )
                .bind(session_uuid)
                .bind(now)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(id, user_id, token_hash, ip, user_agent, expires_at, created_at)| Session {
                        id: id.to_string(),
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
                let session_id = uuid::Uuid::parse_str(session_id)
                    .map_err(|e| anyhow::anyhow!("invalid session id: {}", e))?;
                sqlx::query("DELETE FROM sessions WHERE id = $1")
                    .bind(session_id)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn delete_for_user(&self, user_id: UserId) -> Result<u64> {
        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => sqlx::query("DELETE FROM sessions WHERE user_id = ?")
                .bind(user_id.0.to_string())
                .execute(pool)
                .await?
                .rows_affected(),
            DatabaseBackend::Postgres(pool) => {
                sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                    .bind(user_id.0)
                    .execute(pool)
                    .await?
                    .rows_affected()
            }
        };
        Ok(affected)
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

fn row_to_session_sqlite(
    id: String,
    user_id: String,
    token_hash: String,
    ip: Option<String>,
    user_agent: Option<String>,
    expires_at: String,
    created_at: String,
) -> Result<Session> {
    uuid::Uuid::parse_str(&id).with_context(|| "invalid session id UUID")?;
    Ok(Session {
        id,
        user_id: UserId(
            uuid::Uuid::parse_str(&user_id).with_context(|| "invalid session user_id UUID")?,
        ),
        token_hash,
        ip,
        user_agent,
        expires_at: parse_session_rfc3339(&expires_at, "expires_at")?,
        created_at: parse_session_rfc3339(&created_at, "created_at")?,
    })
}

fn parse_session_rfc3339(value: &str, field: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid session {field} timestamp"))
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::hash_token;
    use crate::db::{CreateUserInput, UserRepository};
    use xlstatus_shared::UserRole;

    #[tokio::test]
    async fn invalid_session_created_at_is_treated_as_missing() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let token_hash = hash_token("dirty-session");
        insert_raw_session(
            &db,
            user.id.0.to_string().as_str(),
            &token_hash,
            &(Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            "not-a-timestamp",
        )
        .await;
        let repo = SessionRepository::new(db);

        assert!(repo
            .find_by_token_hash(&token_hash)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn invalid_session_timestamps_are_treated_as_missing() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let session_id = uuid::Uuid::now_v7().to_string();
        let token_hash = hash_token("dirty-session-time");
        insert_raw_session_with_id(
            &db,
            &session_id,
            user.id.0.to_string().as_str(),
            &token_hash,
            "9999-99-99T99:99:99Z",
            &Utc::now().to_rfc3339(),
        )
        .await;
        let repo = SessionRepository::new(db);

        assert!(repo.find_by_id(&session_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn non_uuid_session_id_is_treated_as_missing_by_token_hash() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let token_hash = hash_token("dirty-session-id");
        insert_raw_session_with_id(
            &db,
            "not-a-uuid",
            user.id.0.to_string().as_str(),
            &token_hash,
            &(Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            &Utc::now().to_rfc3339(),
        )
        .await;
        let repo = SessionRepository::new(db);

        assert!(repo
            .find_by_token_hash(&token_hash)
            .await
            .unwrap()
            .is_none());
    }

    async fn insert_raw_session(
        db: &DatabaseBackend,
        user_id: &str,
        token_hash: &str,
        expires_at: &str,
        created_at: &str,
    ) {
        insert_raw_session_with_id(
            db,
            &uuid::Uuid::now_v7().to_string(),
            user_id,
            token_hash,
            expires_at,
            created_at,
        )
        .await;
    }

    async fn insert_raw_session_with_id(
        db: &DatabaseBackend,
        id: &str,
        user_id: &str,
        token_hash: &str,
        expires_at: &str,
        created_at: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            r#"
            INSERT INTO sessions (id, user_id, token_hash, expires_at, created_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(token_hash)
        .bind(expires_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }
}
