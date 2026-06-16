use crate::db::{models::*, DatabaseBackend};
use anyhow::Result;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Utc};
use xlstatus_shared::UserId;

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

    pub async fn find_by_username(&self, username: &str) -> Result<Option<User>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, String, i32, String, String)>(
                    "SELECT id, username, password_hash, role, token_version, created_at, updated_at FROM users WHERE username = ?",
                )
                .bind(username)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(|(id, username, password_hash, role, token_version, created_at, updated_at)| {
                    User {
                        id: UserId(uuid::Uuid::parse_str(&id).unwrap()),
                        username,
                        password_hash,
                        role: role.parse().unwrap(),
                        token_version,
                        created_at: DateTime::parse_from_rfc3339(&created_at).unwrap().with_timezone(&Utc),
                        updated_at: DateTime::parse_from_rfc3339(&updated_at).unwrap().with_timezone(&Utc),
                    }
                }))
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, String, String, String, i32, DateTime<Utc>, DateTime<Utc>)>(
                    "SELECT id, username, password_hash, role, token_version, created_at, updated_at FROM users WHERE username = $1",
                )
                .bind(username)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(|(id, username, password_hash, role, token_version, created_at, updated_at)| {
                    User {
                        id: UserId(id),
                        username,
                        password_hash,
                        role: role.parse().unwrap(),
                        token_version,
                        created_at,
                        updated_at,
                    }
                }))
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
