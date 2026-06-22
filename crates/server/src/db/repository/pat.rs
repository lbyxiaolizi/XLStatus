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
                    .filter_map(
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
                            let pat_id = id.clone();
                            match row_to_pat_sqlite(
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
                            ) {
                                Ok(pat) => Some(pat),
                                Err(err) => {
                                    tracing::warn!(
                                        pat_id = %pat_id,
                                        "skipping invalid PAT row: {err}"
                                    );
                                    None
                                }
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
                    .filter_map(
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
                            let pat_id = id.to_string();
                            match row_to_pat_postgres(
                                pat_id.clone(),
                                UserId(user_id_uuid),
                                name,
                                token_hash,
                                scopes_json,
                                server_ids_json,
                                expires_at,
                                last_used_at,
                                last_used_ip,
                                created_at,
                                revoked_at,
                            ) {
                                Ok(pat) => Some(pat),
                                Err(err) => {
                                    tracing::warn!(
                                        pat_id = %pat_id,
                                        "skipping invalid PAT row: {err}"
                                    );
                                    None
                                }
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

                match row {
                    Some((
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
                    )) => {
                        let pat_id = id.clone();
                        match row_to_pat_sqlite(
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
                        ) {
                            Ok(pat) => Ok(Some(pat)),
                            Err(err) => {
                                tracing::warn!(
                                    pat_id = %pat_id,
                                    "treating invalid PAT row as not found: {err}"
                                );
                                Ok(None)
                            }
                        }
                    }
                    None => Ok(None),
                }
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

                match row {
                    Some((
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
                    )) => {
                        let pat_id = id.to_string();
                        match row_to_pat_postgres(
                            pat_id.clone(),
                            UserId(user_id_uuid),
                            name,
                            token_hash,
                            scopes_json,
                            server_ids_json,
                            expires_at,
                            last_used_at,
                            last_used_ip,
                            created_at,
                            revoked_at,
                        ) {
                            Ok(pat) => Ok(Some(pat)),
                            Err(err) => {
                                tracing::warn!(
                                    pat_id = %pat_id,
                                    "treating invalid PAT row as not found: {err}"
                                );
                                Ok(None)
                            }
                        }
                    }
                    None => Ok(None),
                }
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

                match row {
                    Some((
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
                    )) => {
                        let pat_id = id.clone();
                        match row_to_pat_sqlite(
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
                        ) {
                            Ok(pat) => Ok(Some(pat)),
                            Err(err) => {
                                tracing::warn!(
                                    pat_id = %pat_id,
                                    "treating invalid PAT row as not found: {err}"
                                );
                                Ok(None)
                            }
                        }
                    }
                    None => Ok(None),
                }
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

                match row {
                    Some((
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
                    )) => {
                        let pat_id = id.to_string();
                        match row_to_pat_postgres(
                            pat_id.clone(),
                            UserId(user_id_uuid),
                            name,
                            token_hash,
                            scopes_json,
                            server_ids_json,
                            expires_at,
                            last_used_at,
                            last_used_ip,
                            created_at,
                            revoked_at,
                        ) {
                            Ok(pat) => Ok(Some(pat)),
                            Err(err) => {
                                tracing::warn!(
                                    pat_id = %pat_id,
                                    "treating invalid PAT row as not found: {err}"
                                );
                                Ok(None)
                            }
                        }
                    }
                    None => Ok(None),
                }
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

    pub async fn revoke_all_for_user(&self, user_id: UserId) -> Result<u64> {
        let now = Utc::now();

        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => sqlx::query(
                "UPDATE personal_access_tokens SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL",
            )
            .bind(now.to_rfc3339())
            .bind(user_id.0.to_string())
            .execute(pool)
            .await?
            .rows_affected(),
            DatabaseBackend::Postgres(pool) => sqlx::query(
                "UPDATE personal_access_tokens SET revoked_at = $1 WHERE user_id = $2 AND revoked_at IS NULL",
            )
            .bind(now)
            .bind(user_id.0)
            .execute(pool)
            .await?
            .rows_affected(),
        };

        Ok(affected)
    }
}

fn parse_pat_uuid(value: &str, field: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(value).with_context(|| format!("invalid PAT {field} UUID"))
}

fn row_to_pat_sqlite(
    id: String,
    user_id_str: String,
    name: String,
    token_hash: String,
    scopes_json: String,
    server_ids_json: Option<String>,
    expires_at: Option<String>,
    last_used_at: Option<String>,
    last_used_ip: Option<String>,
    created_at: String,
    revoked_at: Option<String>,
) -> Result<PersonalAccessToken> {
    let user_id = UserId(parse_pat_uuid(&user_id_str, "user_id")?);
    let scopes = serde_json::from_str(&scopes_json)
        .with_context(|| format!("invalid PAT scopes JSON for {id}"))?;
    let server_ids = parse_optional_string_array_json(server_ids_json.as_deref(), "server_ids")?;
    Ok(PersonalAccessToken {
        id,
        user_id,
        name,
        token_hash,
        scopes,
        server_ids,
        expires_at: parse_optional_rfc3339(expires_at.as_deref(), "expires_at")?,
        last_used_at: parse_optional_rfc3339(last_used_at.as_deref(), "last_used_at")?,
        last_used_ip,
        created_at: parse_required_rfc3339(&created_at, "created_at")?,
        revoked_at: parse_optional_rfc3339(revoked_at.as_deref(), "revoked_at")?,
    })
}

fn row_to_pat_postgres(
    id: String,
    user_id: UserId,
    name: String,
    token_hash: String,
    scopes_json: serde_json::Value,
    server_ids_json: Option<serde_json::Value>,
    expires_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    last_used_ip: Option<String>,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
) -> Result<PersonalAccessToken> {
    let scopes = serde_json::from_value(scopes_json)
        .with_context(|| format!("invalid PAT scopes JSON for {id}"))?;
    let server_ids = match server_ids_json {
        Some(value) => {
            Some(serde_json::from_value(value).with_context(|| "invalid PAT server_ids JSON")?)
        }
        None => None,
    };
    Ok(PersonalAccessToken {
        id,
        user_id,
        name,
        token_hash,
        scopes,
        server_ids,
        expires_at,
        last_used_at,
        last_used_ip,
        created_at,
        revoked_at,
    })
}

fn parse_optional_string_array_json(
    value: Option<&str>,
    field: &str,
) -> Result<Option<Vec<String>>> {
    match value {
        Some(value) => serde_json::from_str(value)
            .map(Some)
            .with_context(|| format!("invalid PAT {field} JSON")),
        None => Ok(None),
    }
}

fn parse_required_rfc3339(value: &str, field: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid PAT {field} timestamp"))
        .map(|dt| dt.with_timezone(&Utc))
}

fn parse_optional_rfc3339(value: Option<&str>, field: &str) -> Result<Option<DateTime<Utc>>> {
    value
        .map(|value| parse_required_rfc3339(value, field))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::hash_token;
    use crate::db::{CreateUserInput, UserRepository};
    use xlstatus_shared::UserRole;

    #[tokio::test]
    async fn invalid_pat_server_ids_are_not_treated_as_global() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let repo = PATRepository::new(db.clone());
        let token_hash = hash_token("xlp_dirty");
        insert_raw_pat(
            &db,
            user.id,
            "dirty-server-ids",
            &token_hash,
            r#"["server:read"]"#,
            Some(r#"{"bad":"shape"}"#),
        )
        .await;

        assert!(repo
            .find_by_token_hash(&token_hash)
            .await
            .unwrap()
            .is_none());
        assert!(repo.list_by_user(user.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn invalid_pat_scopes_are_not_returned() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let repo = PATRepository::new(db.clone());
        let token_hash = hash_token("xlp_dirty_scopes");
        insert_raw_pat(&db, user.id, "dirty-scopes", &token_hash, "not-json", None).await;

        assert!(repo
            .find_by_token_hash(&token_hash)
            .await
            .unwrap()
            .is_none());
        assert!(repo.list_by_user(user.id).await.unwrap().is_empty());
    }

    async fn insert_raw_pat(
        db: &DatabaseBackend,
        user_id: UserId,
        name: &str,
        token_hash: &str,
        scopes: &str,
        server_ids: Option<&str>,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            r#"
            INSERT INTO personal_access_tokens
            (id, user_id, name, token_hash, scopes, server_ids, expires_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(user_id.0.to_string())
        .bind(name)
        .bind(token_hash)
        .bind(scopes)
        .bind(server_ids)
        .bind((Utc::now() + chrono::Duration::days(1)).to_rfc3339())
        .bind(Utc::now().to_rfc3339())
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
