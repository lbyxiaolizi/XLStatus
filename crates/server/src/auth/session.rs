#![allow(dead_code)]
#![allow(unused)]

use crate::db::{CreateSessionInput, DatabaseBackend, Session};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use xlstatus_shared::UserId;

pub const MAX_ACTIVE_SESSIONS_PER_USER: usize = 32;
const MAX_RAW_SESSION_TOKEN_BYTES: usize = 128;
const MAX_SESSION_TOKEN_HASH_BYTES: usize = 128;
const MAX_SESSION_ID_BYTES: usize = 64;
const MAX_SESSION_IP_BYTES: usize = 128;
const MAX_SESSION_USER_AGENT_BYTES: usize = 512;

pub struct SessionRepository {
    db: DatabaseBackend,
}

impl SessionRepository {
    pub fn new(db: DatabaseBackend) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateSessionInput, token_hash: String) -> Result<Session> {
        let token_hash = validate_token_hash(&token_hash)?.to_string();
        let now = Utc::now();
        if input.expires_at <= now {
            bail!("session expiration must be in the future");
        }

        let id = uuid::Uuid::now_v7().to_string();
        let ip = sanitize_optional_metadata(input.ip, MAX_SESSION_IP_BYTES);
        let user_agent = sanitize_optional_metadata(input.user_agent, MAX_SESSION_USER_AGENT_BYTES);
        let max_existing = (MAX_ACTIVE_SESSIONS_PER_USER.saturating_sub(1)) as i64;

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let mut tx = pool.begin().await?;
                let user_id_str = input.user_id.0.to_string();
                let now_str = now.to_rfc3339();

                sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
                    .bind(&now_str)
                    .execute(&mut *tx)
                    .await?;

                sqlx::query(
                    r#"
                    DELETE FROM sessions
                    WHERE user_id = ?
                      AND id NOT IN (
                        SELECT id FROM sessions
                        WHERE user_id = ? AND expires_at > ?
                        ORDER BY created_at DESC, rowid DESC
                        LIMIT ?
                      )
                    "#,
                )
                .bind(&user_id_str)
                .bind(&user_id_str)
                .bind(&now_str)
                .bind(max_existing)
                .execute(&mut *tx)
                .await?;

                sqlx::query(
                    r#"
                    INSERT INTO sessions (id, user_id, token_hash, ip, user_agent, expires_at, created_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    "#,
                )
                .bind(&id)
                .bind(&user_id_str)
                .bind(&token_hash)
                .bind(&ip)
                .bind(&user_agent)
                .bind(input.expires_at.to_rfc3339())
                .bind(&now_str)
                .execute(&mut *tx)
                .await?;

                tx.commit().await?;
            }
            DatabaseBackend::Postgres(pool) => {
                let pg_id = uuid::Uuid::parse_str(&id)
                    .map_err(|e| anyhow::anyhow!("session id must be uuid: {}", e))?;
                let mut tx = pool.begin().await?;

                sqlx::query("DELETE FROM sessions WHERE expires_at <= $1")
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;

                sqlx::query(
                    r#"
                    DELETE FROM sessions
                    WHERE user_id = $1
                      AND id NOT IN (
                        SELECT id FROM sessions
                        WHERE user_id = $1 AND expires_at > $2
                        ORDER BY created_at DESC, id DESC
                        LIMIT $3
                      )
                    "#,
                )
                .bind(input.user_id.0)
                .bind(now)
                .bind(max_existing)
                .execute(&mut *tx)
                .await?;

                sqlx::query(
                    r#"
                    INSERT INTO sessions (id, user_id, token_hash, ip, user_agent, expires_at, created_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    "#,
                )
                .bind(pg_id)
                .bind(input.user_id.0)
                .bind(&token_hash)
                .bind(&ip)
                .bind(&user_agent)
                .bind(input.expires_at)
                .bind(now)
                .execute(&mut *tx)
                .await?;

                tx.commit().await?;
            }
        }

        Ok(Session {
            id,
            user_id: input.user_id,
            token_hash,
            ip,
            user_agent,
            expires_at: input.expires_at,
            created_at: now,
        })
    }

    pub async fn find_by_token(&self, token: &str) -> Result<Option<Session>> {
        if !is_valid_raw_session_token(token) {
            return Ok(None);
        }
        let token_hash = crate::auth::hash_token(token);
        self.find_by_token_hash(&token_hash).await
    }

    pub async fn find_by_token_hash(&self, token_hash: &str) -> Result<Option<Session>> {
        let Ok(token_hash) = validate_token_hash(token_hash) else {
            return Ok(None);
        };
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
                            Ok(session) if session.expires_at > now => Ok(Some(session)),
                            Ok(_) => Ok(None),
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

                Ok(row
                    .map(
                        |(id, user_id, token_hash, ip, user_agent, expires_at, created_at)| {
                            Session {
                                id: id.to_string(),
                                user_id: UserId(user_id),
                                token_hash,
                                ip,
                                user_agent,
                                expires_at,
                                created_at,
                            }
                        },
                    )
                    .filter(|session| session.expires_at > now))
            }
        }
    }

    pub async fn find_by_id(&self, session_id: &str) -> Result<Option<Session>> {
        if session_id.is_empty() || session_id.len() > MAX_SESSION_ID_BYTES {
            return Ok(None);
        }
        let Ok(parsed_uuid) = uuid::Uuid::parse_str(session_id) else {
            return Ok(None);
        };
        if parsed_uuid.to_string() != session_id {
            return Ok(None);
        }
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
                            Ok(session) if session.expires_at > now => Ok(Some(session)),
                            Ok(_) => Ok(None),
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
                    "SELECT id, user_id, token_hash, ip, user_agent, expires_at, created_at FROM sessions WHERE id = $1 AND expires_at > $2",
                )
                .bind(parsed_uuid)
                .bind(now)
                .fetch_optional(pool)
                .await?;

                Ok(row
                    .map(
                        |(id, user_id, token_hash, ip, user_agent, expires_at, created_at)| {
                            Session {
                                id: id.to_string(),
                                user_id: UserId(user_id),
                                token_hash,
                                ip,
                                user_agent,
                                expires_at,
                                created_at,
                            }
                        },
                    )
                    .filter(|session| session.expires_at > now))
            }
        }
    }

    pub async fn delete(&self, session_id: &str) -> Result<()> {
        if session_id.is_empty() || session_id.len() > MAX_SESSION_ID_BYTES {
            bail!("invalid session id");
        }
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

fn is_valid_raw_session_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_RAW_SESSION_TOKEN_BYTES
        && token.is_ascii()
        && !token
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
}

fn validate_token_hash(token_hash: &str) -> Result<&str> {
    if token_hash.is_empty()
        || token_hash.len() > MAX_SESSION_TOKEN_HASH_BYTES
        || !token_hash.is_ascii()
        || token_hash
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
    {
        bail!("invalid session token hash");
    }
    Ok(token_hash)
}

fn sanitize_optional_metadata(value: Option<String>, max_bytes: usize) -> Option<String> {
    value.and_then(|raw| {
        let filtered: String = raw.chars().filter(|ch| !ch.is_control()).collect();
        let trimmed = filtered.trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut end = trimmed.len().min(max_bytes);
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            None
        } else {
            Some(trimmed[..end].to_string())
        }
    })
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

    #[tokio::test]
    async fn enforces_per_user_active_session_cap_and_prunes_expired() {
        let db = test_db().await;
        let users = UserRepository::new(db.clone());
        let user = users
            .create(CreateUserInput {
                username: "capped-user".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let other_user = users
            .create(CreateUserInput {
                username: "other-user".into(),
                password: "secret".into(),
                role: UserRole::Member,
            })
            .await
            .unwrap();
        let repo = SessionRepository::new(db.clone());

        let other_token = "other-user-token";
        repo.create(
            CreateSessionInput {
                user_id: other_user.id,
                ip: Some("127.0.0.1".into()),
                user_agent: Some("test-agent".into()),
                expires_at: Utc::now() + chrono::Duration::hours(1),
            },
            hash_token(other_token),
        )
        .await
        .unwrap();

        let total_created = MAX_ACTIVE_SESSIONS_PER_USER + 5;
        let mut tokens = Vec::with_capacity(total_created);
        for index in 0..total_created {
            let token = format!("session-token-{index:04}");
            let hash = hash_token(&token);
            repo.create(
                CreateSessionInput {
                    user_id: user.id,
                    ip: Some("127.0.0.1".into()),
                    user_agent: Some("test-agent".into()),
                    expires_at: Utc::now() + chrono::Duration::hours(1),
                },
                hash,
            )
            .await
            .unwrap();
            tokens.push(token);
        }

        // Oldest 5 sessions for `user` should be evicted; newest MAX_ACTIVE_SESSIONS_PER_USER remain.
        for evicted in &tokens[..5] {
            assert!(
                repo.find_by_token(evicted).await.unwrap().is_none(),
                "expected old session {evicted} to be evicted"
            );
        }
        for retained in &tokens[5..] {
            assert!(
                repo.find_by_token(retained).await.unwrap().is_some(),
                "expected recent session {retained} to remain active"
            );
        }
        // Other user's session is untouched.
        assert!(repo.find_by_token(other_token).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn rejects_empty_oversized_or_expired_tokens_in_lookup_and_create() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "lookup-user".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let repo = SessionRepository::new(db);

        assert!(repo.find_by_token("").await.unwrap().is_none());
        assert!(repo
            .find_by_token(&"a".repeat(MAX_RAW_SESSION_TOKEN_BYTES + 1))
            .await
            .unwrap()
            .is_none());
        assert!(repo
            .find_by_token("token with spaces")
            .await
            .unwrap()
            .is_none());
        assert!(repo.find_by_token_hash("").await.unwrap().is_none());
        assert!(repo
            .find_by_token_hash(&"a".repeat(MAX_SESSION_TOKEN_HASH_BYTES + 1))
            .await
            .unwrap()
            .is_none());

        // Creating a session that is already expired must fail.
        assert!(repo
            .create(
                CreateSessionInput {
                    user_id: user.id,
                    ip: None,
                    user_agent: None,
                    expires_at: Utc::now() - chrono::Duration::minutes(1),
                },
                hash_token("already-expired"),
            )
            .await
            .is_err());
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
