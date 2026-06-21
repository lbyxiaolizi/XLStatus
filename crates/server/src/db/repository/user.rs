#![allow(dead_code)]
#![allow(unused)]

use crate::db::{models::*, DatabaseBackend};
use crate::secrets::{decrypt_optional_secret, encrypt_secret};
use anyhow::Result;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Utc};
use xlstatus_shared::UserId;
use xlstatus_shared::UserRole;

pub struct UserRepository {
    db: DatabaseBackend,
}

impl UserRepository {
    pub fn new(db: DatabaseBackend) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateUserInput) -> Result<User> {
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
                    rows.into_iter().map(sqlite_user_from_row).collect(),
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
                    rows.into_iter().map(postgres_user_from_row).collect(),
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

                Ok(row.map(sqlite_user_from_row))
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, String, String, i32, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, username, password_hash, role, token_version, created_at, updated_at FROM users WHERE username = $1",
                )
                .bind(username)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(postgres_user_from_row))
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

                Ok(row.map(sqlite_user_from_row))
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, String, String, i32, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, username, password_hash, role, token_version, created_at, updated_at FROM users WHERE id = $1",
                )
                .bind(user_id.0)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(postgres_user_from_row))
            }
        }
    }

    pub fn verify_password(&self, user: &User, password: &str) -> Result<bool> {
        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|e| anyhow::anyhow!("Failed to parse password hash: {}", e))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    fn hash_password(&self, password: &str) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow::anyhow!("Failed to hash password: {}", e))?
            .to_string();
        Ok(password_hash)
    }
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
) -> User {
    User {
        id: UserId(uuid::Uuid::parse_str(&id).unwrap()),
        username,
        password_hash,
        role: role.parse().unwrap(),
        token_version,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .unwrap()
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at)
            .unwrap()
            .with_timezone(&Utc),
    }
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
) -> User {
    User {
        id: UserId(id),
        username,
        password_hash,
        role: role.parse().unwrap(),
        token_version,
        created_at,
        updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DatabaseBackend;
    use crate::secrets::is_encrypted_secret;
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
}
