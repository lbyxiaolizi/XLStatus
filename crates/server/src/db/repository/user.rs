#![allow(dead_code)]
#![allow(unused)]

use crate::db::{models::*, DatabaseBackend};
use crate::secrets::{decrypt_optional_secret, encrypt_secret};
use anyhow::{Context, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Utc};
use xlstatus_shared::UserId;
use xlstatus_shared::UserRole;

const USER_LIST_MAX_LIMIT: i64 = 500;
const USER_PASSWORD_MAX_BYTES: usize = 1024;
const USER_USERNAME_MAX_BYTES: usize = 255;

pub struct UserRepository {
    db: DatabaseBackend,
}

impl UserRepository {
    pub fn new(db: DatabaseBackend) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateUserInput) -> Result<User> {
        validate_username(&input.username)?;
        let id = UserId::new();
        let password_hash = self.hash_password(&input.password)?;
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO users (id, username, password_hash, role, token_version, created_at, updated_at)
                    VALUES (?, ?, ?, ?, 0, ?, ?)
                    "#,
                )
                .bind(id.0.to_string())
                .bind(&input.username)
                .bind(&password_hash)
                .bind(input.role.to_string())
                .bind(now.to_rfc3339())
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO users (id, username, password_hash, role, token_version, created_at, updated_at)
                    VALUES ($1, $2, $3, $4, 0, $5, $6)
                    "#,
                )
                .bind(id.0)
                .bind(&input.username)
                .bind(&password_hash)
                .bind(input.role.to_string())
                .bind(now)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }

        Ok(User {
            id,
            username: input.username,
            password_hash,
            role: input.role,
            token_version: 0,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn list(&self, limit: i64, offset: i64) -> Result<(Vec<User>, i64)> {
        let limit = limit.clamp(1, USER_LIST_MAX_LIMIT);
        let offset = offset.max(0);
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
                    .fetch_one(pool)
                    .await?;
                let rows =
                    sqlx::query_as::<_, (String, String, String, String, i32, String, String)>(
                        r#"
                    SELECT id, username, password_hash, role, token_version, created_at, updated_at
                    FROM users
                    ORDER BY created_at ASC
                    LIMIT ? OFFSET ?
                    "#,
                    )
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                Ok((
                    rows.into_iter()
                        .filter_map(|row| match sqlite_user_from_row(row) {
                            Ok(user) => Some(user),
                            Err(err) => {
                                tracing::warn!("skipping invalid user row: {err}");
                                None
                            }
                        })
                        .collect(),
                    total.0,
                ))
            }
            DatabaseBackend::Postgres(pool) => {
                let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
                    .fetch_one(pool)
                    .await?;
                let rows = sqlx::query_as::<
                    _,
                    (
                        uuid::Uuid,
                        String,
                        String,
                        String,
                        i32,
                        DateTime<Utc>,
                        DateTime<Utc>,
                    ),
                >(
                    r#"
                    SELECT id, username, password_hash, role, token_version, created_at, updated_at
                    FROM users
                    ORDER BY created_at ASC
                    LIMIT $1 OFFSET $2
                    "#,
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;

                Ok((
                    rows.into_iter()
                        .filter_map(|row| match postgres_user_from_row(row) {
                            Ok(user) => Some(user),
                            Err(err) => {
                                tracing::warn!("skipping invalid user row: {err}");
                                None
                            }
                        })
                        .collect(),
                    total.0,
                ))
            }
        }
    }

    pub async fn count_admins(&self) -> Result<i64> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin'")
                    .fetch_one(pool)
                    .await?;
                Ok(row.0)
            }
            DatabaseBackend::Postgres(pool) => {
                let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin'")
                    .fetch_one(pool)
                    .await?;
                Ok(row.0)
            }
        }
    }

    pub async fn update_role(&self, user_id: UserId, role: UserRole) -> Result<Option<User>> {
        let now = Utc::now();
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query("UPDATE users SET role = ?, updated_at = ? WHERE id = ?")
                    .bind(role.to_string())
                    .bind(now.to_rfc3339())
                    .bind(user_id.0.to_string())
                    .execute(pool)
                    .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query("UPDATE users SET role = $1, updated_at = $2 WHERE id = $3")
                    .bind(role.to_string())
                    .bind(now)
                    .bind(user_id.0)
                    .execute(pool)
                    .await?;
            }
        }
        self.find_by_id(user_id).await
    }

    pub async fn update_password(&self, user_id: UserId, password: &str) -> Result<Option<User>> {
        self.reset_password(user_id, password).await
    }

    pub async fn reset_password(&self, user_id: UserId, password: &str) -> Result<Option<User>> {
        let password_hash = self.hash_password(password)?;
        let now = Utc::now();
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE users SET password_hash = ?, token_version = token_version + 1, updated_at = ? WHERE id = ?",
                )
                .bind(&password_hash)
                .bind(now.to_rfc3339())
                .bind(user_id.0.to_string())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    "UPDATE users SET password_hash = $1, token_version = token_version + 1, updated_at = $2 WHERE id = $3",
                )
                .bind(&password_hash)
                .bind(now)
                .bind(user_id.0)
                .execute(pool)
                .await?;
            }
        }
        self.find_by_id(user_id).await
    }

    pub async fn totp_config(&self, user_id: UserId) -> Result<(Option<String>, bool)> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row: Option<(Option<String>, i64)> = sqlx::query_as(
                    "SELECT totp_secret, COALESCE(totp_enabled, 0) FROM users WHERE id = ?",
                )
                .bind(user_id.0.to_string())
                .fetch_optional(pool)
                .await?;
                let (secret, enabled) = row
                    .map(|(secret, enabled)| (secret, enabled != 0))
                    .unwrap_or((None, false));
                Ok((decrypt_optional_secret(secret)?, enabled))
            }
            DatabaseBackend::Postgres(pool) => {
                let row: Option<(Option<String>, bool)> = sqlx::query_as(
                    "SELECT totp_secret, COALESCE(totp_enabled, false) FROM users WHERE id = $1",
                )
                .bind(user_id.0)
                .fetch_optional(pool)
                .await?;
                let (secret, enabled) = row.unwrap_or((None, false));
                Ok((decrypt_optional_secret(secret)?, enabled))
            }
        }
    }

    pub async fn set_totp_secret(
        &self,
        user_id: UserId,
        secret: &str,
        enabled: bool,
    ) -> Result<()> {
        let now = Utc::now();
        let secret = encrypt_secret(secret)?;
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query("UPDATE users SET totp_secret = ?, totp_enabled = ?, updated_at = ? WHERE id = ?")
                    .bind(&secret)
                    .bind(if enabled { 1_i64 } else { 0_i64 })
                    .bind(now.to_rfc3339())
                    .bind(user_id.0.to_string())
                    .execute(pool)
                    .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query("UPDATE users SET totp_secret = $1, totp_enabled = $2, updated_at = $3 WHERE id = $4")
                    .bind(&secret)
                    .bind(enabled)
                    .bind(now)
                    .bind(user_id.0)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn set_totp_enabled(&self, user_id: UserId, enabled: bool) -> Result<()> {
        let now = Utc::now();
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query("UPDATE users SET totp_enabled = ?, updated_at = ? WHERE id = ?")
                    .bind(if enabled { 1_i64 } else { 0_i64 })
                    .bind(now.to_rfc3339())
                    .bind(user_id.0.to_string())
                    .execute(pool)
                    .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query("UPDATE users SET totp_enabled = $1, updated_at = $2 WHERE id = $3")
                    .bind(enabled)
                    .bind(now)
                    .bind(user_id.0)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn disable_totp(&self, user_id: UserId) -> Result<()> {
        let now = Utc::now();
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE users SET totp_secret = NULL, totp_enabled = 0, updated_at = ? WHERE id = ?",
                )
                .bind(now.to_rfc3339())
                .bind(user_id.0.to_string())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    "UPDATE users SET totp_secret = NULL, totp_enabled = false, updated_at = $1 WHERE id = $2",
                )
                .bind(now)
                .bind(user_id.0)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    pub async fn delete(&self, user_id: UserId) -> Result<bool> {
        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => sqlx::query("DELETE FROM users WHERE id = ?")
                .bind(user_id.0.to_string())
                .execute(pool)
                .await?
                .rows_affected(),
            DatabaseBackend::Postgres(pool) => sqlx::query("DELETE FROM users WHERE id = $1")
                .bind(user_id.0)
                .execute(pool)
                .await?
                .rows_affected(),
        };
        Ok(affected > 0)
    }

    pub async fn find_by_username(&self, username: &str) -> Result<Option<User>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, String, i32, String, String)>(
                    "SELECT id, username, password_hash, role, token_version, created_at, updated_at FROM users WHERE username = ?",
                )
                .bind(username)
                .fetch_optional(pool)
                .await?;

                match row {
                    Some(row) => match sqlite_user_from_row(row) {
                        Ok(user) => Ok(Some(user)),
                        Err(err) => {
                            tracing::warn!(username = %username, "treating invalid user row as not found: {err}");
                            Ok(None)
                        }
                    },
                    None => Ok(None),
                }
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, String, String, i32, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, username, password_hash, role, token_version, created_at, updated_at FROM users WHERE username = $1",
                )
                .bind(username)
                .fetch_optional(pool)
                .await?;

                match row {
                    Some(row) => match postgres_user_from_row(row) {
                        Ok(user) => Ok(Some(user)),
                        Err(err) => {
                            tracing::warn!(username = %username, "treating invalid user row as not found: {err}");
                            Ok(None)
                        }
                    },
                    None => Ok(None),
                }
            }
        }
    }

    pub async fn find_by_id(&self, user_id: UserId) -> Result<Option<User>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, String, i32, String, String)>(
                    "SELECT id, username, password_hash, role, token_version, created_at, updated_at FROM users WHERE id = ?",
                )
                .bind(user_id.0.to_string())
                .fetch_optional(pool)
                .await?;

                match row {
                    Some(row) => match sqlite_user_from_row(row) {
                        Ok(user) => Ok(Some(user)),
                        Err(err) => {
                            tracing::warn!(user_id = %user_id.0, "treating invalid user row as not found: {err}");
                            Ok(None)
                        }
                    },
                    None => Ok(None),
                }
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, String, String, i32, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, username, password_hash, role, token_version, created_at, updated_at FROM users WHERE id = $1",
                )
                .bind(user_id.0)
                .fetch_optional(pool)
                .await?;

                match row {
                    Some(row) => match postgres_user_from_row(row) {
                        Ok(user) => Ok(Some(user)),
                        Err(err) => {
                            tracing::warn!(user_id = %user_id.0, "treating invalid user row as not found: {err}");
                            Ok(None)
                        }
                    },
                    None => Ok(None),
                }
            }
        }
    }

    pub fn verify_password(&self, user: &User, password: &str) -> Result<bool> {
        if validate_user_password(password).is_err() {
            return Ok(false);
        }
        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|e| anyhow::anyhow!("Failed to parse password hash: {}", e))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    fn hash_password(&self, password: &str) -> Result<String> {
        validate_user_password(password)?;
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow::anyhow!("Failed to hash password: {}", e))?
            .to_string();
        Ok(password_hash)
    }
}

fn validate_username(username: &str) -> Result<()> {
    if username.trim().is_empty() {
        anyhow::bail!("username must not be empty");
    }
    if username.len() > USER_USERNAME_MAX_BYTES {
        anyhow::bail!("username must be at most {USER_USERNAME_MAX_BYTES} bytes");
    }
    if username.contains('\0') {
        anyhow::bail!("username must not contain NUL bytes");
    }
    Ok(())
}

fn validate_user_password(password: &str) -> Result<()> {
    if password.is_empty() {
        anyhow::bail!("password must not be empty");
    }
    if password.len() > USER_PASSWORD_MAX_BYTES {
        anyhow::bail!("password must be at most {USER_PASSWORD_MAX_BYTES} bytes");
    }
    if password.contains('\0') {
        anyhow::bail!("password must not contain NUL bytes");
    }
    Ok(())
}

fn sqlite_user_from_row(
    (id, username, password_hash, role, token_version, created_at, updated_at): (
        String,
        String,
        String,
        String,
        i32,
        String,
        String,
    ),
) -> Result<User> {
    Ok(User {
        id: UserId(uuid::Uuid::parse_str(&id).context("invalid user id UUID")?),
        username,
        password_hash,
        role: role
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid user role: {e}"))?,
        token_version,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .context("invalid user created_at timestamp")?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at)
            .context("invalid user updated_at timestamp")?
            .with_timezone(&Utc),
    })
}

fn postgres_user_from_row(
    (id, username, password_hash, role, token_version, created_at, updated_at): (
        uuid::Uuid,
        String,
        String,
        String,
        i32,
        DateTime<Utc>,
        DateTime<Utc>,
    ),
) -> Result<User> {
    Ok(User {
        id: UserId(id),
        username,
        password_hash,
        role: role
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid user role: {e}"))?,
        token_version,
        created_at,
        updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{hash_token, SessionRepository};
    use crate::db::DatabaseBackend;
    use crate::db::{CreatePATInput, CreateSessionInput, PATRepository};
    use crate::secrets::is_encrypted_secret;
    use chrono::Duration;
    use sqlx::Row;

    #[tokio::test]
    async fn totp_secret_is_encrypted_at_rest() {
        let db = test_db().await;
        let repo = UserRepository::new(db.clone());
        let user = repo
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        repo.set_totp_secret(user.id, "BASE32SECRET", false)
            .await
            .unwrap();

        let raw = raw_totp_secret(&db, user.id).await.unwrap();
        assert!(is_encrypted_secret(&raw));
        assert_ne!(raw, "BASE32SECRET");

        let (secret, enabled) = repo.totp_config(user.id).await.unwrap();
        assert_eq!(secret.as_deref(), Some("BASE32SECRET"));
        assert!(!enabled);
    }

    #[tokio::test]
    async fn password_reset_can_revoke_target_sessions_and_pats() {
        let db = test_db().await;
        let users = UserRepository::new(db.clone());
        let sessions = SessionRepository::new(db.clone());
        let pats = PATRepository::new(db.clone());
        let target = users
            .create(CreateUserInput {
                username: "target".into(),
                password: "old-password".into(),
                role: UserRole::Member,
            })
            .await
            .unwrap();
        let other = users
            .create(CreateUserInput {
                username: "other".into(),
                password: "old-password".into(),
                role: UserRole::Member,
            })
            .await
            .unwrap();
        let target_session_hash = hash_token("target-session");
        let other_session_hash = hash_token("other-session");
        sessions
            .create(
                CreateSessionInput {
                    user_id: target.id,
                    ip: None,
                    user_agent: None,
                    expires_at: Utc::now() + Duration::hours(1),
                },
                target_session_hash.clone(),
            )
            .await
            .unwrap();
        sessions
            .create(
                CreateSessionInput {
                    user_id: other.id,
                    ip: None,
                    user_agent: None,
                    expires_at: Utc::now() + Duration::hours(1),
                },
                other_session_hash.clone(),
            )
            .await
            .unwrap();
        let target_pat_hash = hash_token("target-pat");
        let other_pat_hash = hash_token("other-pat");
        pats.create(
            CreatePATInput {
                user_id: target.id,
                name: "target".into(),
                scopes: vec!["server:read".into()],
                server_ids: Some(vec![uuid::Uuid::now_v7().to_string()]),
                expires_at: Some(Utc::now() + Duration::days(1)),
            },
            target_pat_hash.clone(),
        )
        .await
        .unwrap();
        pats.create(
            CreatePATInput {
                user_id: other.id,
                name: "other".into(),
                scopes: vec!["server:read".into()],
                server_ids: Some(vec![uuid::Uuid::now_v7().to_string()]),
                expires_at: Some(Utc::now() + Duration::days(1)),
            },
            other_pat_hash.clone(),
        )
        .await
        .unwrap();

        let reset = users
            .reset_password(target.id, "new-password")
            .await
            .unwrap()
            .unwrap();
        let deleted_sessions = sessions.delete_for_user(target.id).await.unwrap();
        let revoked_pats = pats.revoke_all_for_user(target.id).await.unwrap();

        assert_eq!(reset.token_version, target.token_version + 1);
        assert_eq!(deleted_sessions, 1);
        assert_eq!(revoked_pats, 1);
        assert!(sessions
            .find_by_token_hash(&target_session_hash)
            .await
            .unwrap()
            .is_none());
        assert!(pats
            .find_by_token_hash(&target_pat_hash)
            .await
            .unwrap()
            .is_none());
        assert!(sessions
            .find_by_token_hash(&other_session_hash)
            .await
            .unwrap()
            .is_some());
        assert!(pats
            .find_by_token_hash(&other_pat_hash)
            .await
            .unwrap()
            .is_some());
    }

    async fn test_db() -> DatabaseBackend {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-user-secret-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn raw_totp_secret(db: &DatabaseBackend, user_id: UserId) -> Result<String> {
        match db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query("SELECT totp_secret FROM users WHERE id = ?")
                    .bind(user_id.0.to_string())
                    .fetch_one(pool)
                    .await?;
                Ok(row.try_get("totp_secret")?)
            }
            DatabaseBackend::Postgres(_) => unreachable!(),
        }
    }

    #[tokio::test]
    async fn password_validation_and_user_list_pagination_clamping() {
        let db = test_db().await;
        let users = UserRepository::new(db);

        assert!(users
            .create(CreateUserInput {
                username: "bad-empty-pass".to_string(),
                password: "".to_string(),
                role: UserRole::Member,
            })
            .await
            .is_err());

        assert!(users
            .create(CreateUserInput {
                username: "bad-nul-pass".to_string(),
                password: "secret\0password".to_string(),
                role: UserRole::Member,
            })
            .await
            .is_err());

        assert!(users
            .create(CreateUserInput {
                username: "bad-huge-pass".to_string(),
                password: "a".repeat(USER_PASSWORD_MAX_BYTES + 1),
                role: UserRole::Member,
            })
            .await
            .is_err());

        let user = users
            .create(CreateUserInput {
                username: "valid-user".to_string(),
                password: "StrongPassword123!".to_string(),
                role: UserRole::Member,
            })
            .await
            .unwrap();

        assert!(users.reset_password(user.id, "").await.is_err());
        assert!(users.reset_password(user.id, "bad\0pass").await.is_err());
        assert!(users
            .update_password(user.id, &"x".repeat(USER_PASSWORD_MAX_BYTES + 1))
            .await
            .is_err());

        assert!(!users.verify_password(&user, "").unwrap());
        assert!(!users.verify_password(&user, "bad\0pass").unwrap());
        assert!(!users
            .verify_password(&user, &"x".repeat(USER_PASSWORD_MAX_BYTES + 1))
            .unwrap());
        assert!(users.verify_password(&user, "StrongPassword123!").unwrap());

        let (listed, total) = users.list(-1, -10).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(total, 1);
    }
}
