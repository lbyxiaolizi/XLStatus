#![allow(dead_code)]
#![allow(unused)]

use crate::db::{CreatePATInput, DatabaseBackend, PersonalAccessToken};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use xlstatus_shared::UserId;

pub struct PATRepository {
    db: DatabaseBackend,
}

impl PATRepository {
    pub fn new(db: DatabaseBackend) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        input: CreatePATInput,
        token_hash: String,
    ) -> Result<PersonalAccessToken> {
        let id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let scopes_json = serde_json::to_string(&input.scopes)?;
                let server_ids_json = input
                    .server_ids
                    .as_ref()
                    .map(|ids| serde_json::to_string(ids))
                    .transpose()?;

                sqlx::query(
                    r#"
                    INSERT INTO personal_access_tokens
                    (id, user_id, name, token_hash, scopes, server_ids, expires_at, created_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                )
                .bind(&id)
                .bind(input.user_id.0.to_string())
                .bind(&input.name)
                .bind(&token_hash)
                .bind(&scopes_json)
                .bind(server_ids_json.as_deref())
                .bind(input.expires_at.map(|dt| dt.to_rfc3339()))
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                let scopes_json = serde_json::to_value(&input.scopes)?;
                let server_ids_json = input
                    .server_ids
                    .as_ref()
                    .map(|ids| serde_json::to_value(ids))
                    .transpose()?;
                let id_uuid = parse_pat_uuid(&id, "id")?;

                sqlx::query(
                    r#"
                    INSERT INTO personal_access_tokens
                    (id, user_id, name, token_hash, scopes, server_ids, expires_at, created_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                    "#,
                )
                .bind(id_uuid)
                .bind(input.user_id.0)
                .bind(&input.name)
                .bind(&token_hash)
                .bind(&scopes_json)
                .bind(server_ids_json.as_ref())
                .bind(input.expires_at)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }

        Ok(PersonalAccessToken {
            id,
            user_id: input.user_id,
            name: input.name,
            token_hash,
            scopes: input.scopes,
            server_ids: input.server_ids,
            expires_at: input.expires_at,
            last_used_at: None,
            last_used_ip: None,
            created_at: now,
            revoked_at: None,
        })
    }

    pub async fn list_by_user(&self, user_id: UserId) -> Result<Vec<PersonalAccessToken>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query_as::<_, (String, String, String, String, String, Option<String>, Option<String>, Option<String>, Option<String>, String, Option<String>)>(
                    r#"
                    SELECT id, user_id, name, token_hash, scopes, server_ids, expires_at, last_used_at, last_used_ip, created_at, revoked_at
                    FROM personal_access_tokens
                    WHERE user_id = ? AND revoked_at IS NULL
                    ORDER BY created_at DESC
                    "#,
                )
                .bind(user_id.0.to_string())
                .fetch_all(pool)
                .await?;

                Ok(rows
                    .into_iter()
                    .map(
                        |(
                            id,
                            user_id_str,
                            name,
                            token_hash,
                            scopes_json,
                            server_ids_json,
                            expires_at,
                            last_used_at,
                            last_used_ip,
                            created_at,
                            revoked_at,
                        )| {
                            PersonalAccessToken {
                                id,
                                user_id: UserId(uuid::Uuid::parse_str(&user_id_str).unwrap()),
                                name,
                                token_hash,
                                scopes: serde_json::from_str(&scopes_json).unwrap(),
                                server_ids: server_ids_json
                                    .and_then(|s| serde_json::from_str(&s).ok()),
                                expires_at: expires_at.and_then(|s| {
                                    DateTime::parse_from_rfc3339(&s)
                                        .ok()
                                        .map(|dt| dt.with_timezone(&Utc))
                                }),
                                last_used_at: last_used_at.and_then(|s| {
                                    DateTime::parse_from_rfc3339(&s)
                                        .ok()
                                        .map(|dt| dt.with_timezone(&Utc))
                                }),
                                last_used_ip,
                                created_at: DateTime::parse_from_rfc3339(&created_at)
                                    .unwrap()
                                    .with_timezone(&Utc),
                                revoked_at: revoked_at.and_then(|s| {
                                    DateTime::parse_from_rfc3339(&s)
                                        .ok()
                                        .map(|dt| dt.with_timezone(&Utc))
                                }),
                            }
                        },
                    )
                    .collect())
            }
            DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, String, serde_json::Value, Option<serde_json::Value>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)>(
                    r#"
                    SELECT id, user_id, name, token_hash, scopes, server_ids, expires_at, last_used_at, last_used_ip, created_at, revoked_at
                    FROM personal_access_tokens
                    WHERE user_id = $1 AND revoked_at IS NULL
                    ORDER BY created_at DESC
                    "#,
                )
                .bind(user_id.0)
                .fetch_all(pool)
                .await?;

                Ok(rows
                    .into_iter()
                    .map(
                        |(
                            id,
                            user_id_uuid,
                            name,
                            token_hash,
                            scopes_json,
                            server_ids_json,
                            expires_at,
                            last_used_at,
                            last_used_ip,
                            created_at,
                            revoked_at,
                        )| {
                            PersonalAccessToken {
                                id: id.to_string(),
                                user_id: UserId(user_id_uuid),
                                name,
                                token_hash,
                                scopes: serde_json::from_value(scopes_json).unwrap(),
                                server_ids: server_ids_json
                                    .and_then(|v| serde_json::from_value(v).ok()),
                                expires_at,
                                last_used_at,
                                last_used_ip,
                                created_at,
                                revoked_at,
                            }
                        },
                    )
                    .collect())
            }
        }
    }

    pub async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessToken>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, String, String, Option<String>, Option<String>, Option<String>, Option<String>, String, Option<String>)>(
                    r#"
                    SELECT id, user_id, name, token_hash, scopes, server_ids, expires_at, last_used_at, last_used_ip, created_at, revoked_at
                    FROM personal_access_tokens
                    WHERE token_hash = ? AND revoked_at IS NULL
                    "#,
                )
                .bind(token_hash)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(
                        id,
                        user_id_str,
                        name,
                        token_hash,
                        scopes_json,
                        server_ids_json,
                        expires_at,
                        last_used_at,
                        last_used_ip,
                        created_at,
                        revoked_at,
                    )| PersonalAccessToken {
                        id,
                        user_id: UserId(uuid::Uuid::parse_str(&user_id_str).unwrap()),
                        name,
                        token_hash,
                        scopes: serde_json::from_str(&scopes_json).unwrap(),
                        server_ids: server_ids_json.and_then(|s| serde_json::from_str(&s).ok()),
                        expires_at: expires_at.and_then(|s| {
                            DateTime::parse_from_rfc3339(&s)
                                .ok()
                                .map(|dt| dt.with_timezone(&Utc))
                        }),
                        last_used_at: last_used_at.and_then(|s| {
                            DateTime::parse_from_rfc3339(&s)
                                .ok()
                                .map(|dt| dt.with_timezone(&Utc))
                        }),
                        last_used_ip,
                        created_at: DateTime::parse_from_rfc3339(&created_at)
                            .unwrap()
                            .with_timezone(&Utc),
                        revoked_at: revoked_at.and_then(|s| {
                            DateTime::parse_from_rfc3339(&s)
                                .ok()
                                .map(|dt| dt.with_timezone(&Utc))
                        }),
                    },
                ))
            }
            DatabaseBackend::Postgres(pool) => {
                let row = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, String, serde_json::Value, Option<serde_json::Value>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)>(
                    r#"
                    SELECT id, user_id, name, token_hash, scopes, server_ids, expires_at, last_used_at, last_used_ip, created_at, revoked_at
                    FROM personal_access_tokens
                    WHERE token_hash = $1 AND revoked_at IS NULL
                    "#,
                )
                .bind(token_hash)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(
                        id,
                        user_id_uuid,
                        name,
                        token_hash,
                        scopes_json,
                        server_ids_json,
                        expires_at,
                        last_used_at,
                        last_used_ip,
                        created_at,
                        revoked_at,
                    )| PersonalAccessToken {
                        id: id.to_string(),
                        user_id: UserId(user_id_uuid),
                        name,
                        token_hash,
                        scopes: serde_json::from_value(scopes_json).unwrap(),
                        server_ids: server_ids_json.and_then(|v| serde_json::from_value(v).ok()),
                        expires_at,
                        last_used_at,
                        last_used_ip,
                        created_at,
                        revoked_at,
                    },
                ))
            }
        }
    }

    pub async fn find_by_id(&self, id: &str) -> Result<Option<PersonalAccessToken>> {
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, String, String, Option<String>, Option<String>, Option<String>, Option<String>, String, Option<String>)>(
                    r#"
                    SELECT id, user_id, name, token_hash, scopes, server_ids, expires_at, last_used_at, last_used_ip, created_at, revoked_at
                    FROM personal_access_tokens
                    WHERE id = ?
                    "#,
                )
                .bind(id)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(
                        id,
                        user_id_str,
                        name,
                        token_hash,
                        scopes_json,
                        server_ids_json,
                        expires_at,
                        last_used_at,
                        last_used_ip,
                        created_at,
                        revoked_at,
                    )| PersonalAccessToken {
                        id,
                        user_id: UserId(uuid::Uuid::parse_str(&user_id_str).unwrap()),
                        name,
                        token_hash,
                        scopes: serde_json::from_str(&scopes_json).unwrap(),
                        server_ids: server_ids_json.and_then(|s| serde_json::from_str(&s).ok()),
                        expires_at: expires_at.and_then(|s| {
                            DateTime::parse_from_rfc3339(&s)
                                .ok()
                                .map(|dt| dt.with_timezone(&Utc))
                        }),
                        last_used_at: last_used_at.and_then(|s| {
                            DateTime::parse_from_rfc3339(&s)
                                .ok()
                                .map(|dt| dt.with_timezone(&Utc))
                        }),
                        last_used_ip,
                        created_at: DateTime::parse_from_rfc3339(&created_at)
                            .unwrap()
                            .with_timezone(&Utc),
                        revoked_at: revoked_at.and_then(|s| {
                            DateTime::parse_from_rfc3339(&s)
                                .ok()
                                .map(|dt| dt.with_timezone(&Utc))
                        }),
                    },
                ))
            }
            DatabaseBackend::Postgres(pool) => {
                let id = parse_pat_uuid(id, "id")?;
                let row = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, String, serde_json::Value, Option<serde_json::Value>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)>(
                    r#"
                    SELECT id, user_id, name, token_hash, scopes, server_ids, expires_at, last_used_at, last_used_ip, created_at, revoked_at
                    FROM personal_access_tokens
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(
                    |(
                        id,
                        user_id_uuid,
                        name,
                        token_hash,
                        scopes_json,
                        server_ids_json,
                        expires_at,
                        last_used_at,
                        last_used_ip,
                        created_at,
                        revoked_at,
                    )| PersonalAccessToken {
                        id: id.to_string(),
                        user_id: UserId(user_id_uuid),
                        name,
                        token_hash,
                        scopes: serde_json::from_value(scopes_json).unwrap(),
                        server_ids: server_ids_json.and_then(|v| serde_json::from_value(v).ok()),
                        expires_at,
                        last_used_at,
                        last_used_ip,
                        created_at,
                        revoked_at,
                    },
                ))
            }
        }
    }

    pub async fn mark_used(&self, id: &str, ip: Option<&str>) -> Result<()> {
        let now = Utc::now();

        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE personal_access_tokens SET last_used_at = ?, last_used_ip = ? WHERE id = ?",
                )
                .bind(now.to_rfc3339())
                .bind(ip)
                .bind(id)
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                let id = parse_pat_uuid(id, "id")?;
                sqlx::query(
                    "UPDATE personal_access_tokens SET last_used_at = $1, last_used_ip = $2 WHERE id = $3",
                )
                .bind(now)
                .bind(ip)
                .bind(id)
                .execute(pool)
                .await?;
            }
        }

        Ok(())
    }

    pub async fn revoke(&self, id: &str, user_id: UserId) -> Result<bool> {
        let now = Utc::now();

        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let result = sqlx::query(
                    "UPDATE personal_access_tokens SET revoked_at = ? WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
                )
                .bind(now.to_rfc3339())
                .bind(id)
                .bind(user_id.0.to_string())
                .execute(pool)
                .await?;
                result.rows_affected()
            }
            DatabaseBackend::Postgres(pool) => {
                let result = sqlx::query(
                    "UPDATE personal_access_tokens SET revoked_at = $1 WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL",
                )
                .bind(now)
                .bind(parse_pat_uuid(id, "id")?)
                .bind(user_id.0)
                .execute(pool)
                .await?;
                result.rows_affected()
            }
        };

        Ok(affected > 0)
    }
}

fn parse_pat_uuid(value: &str, field: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(value).with_context(|| format!("invalid PAT {field} UUID"))
}
