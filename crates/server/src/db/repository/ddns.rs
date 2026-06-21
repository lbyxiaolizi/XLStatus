#![allow(dead_code)]
#![allow(unused)]

use crate::db::Db;
use crate::secrets::{decrypt_optional_secret, encrypt_optional_secret};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use xlstatus_shared::ddns::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DdnsConfigRow {
    pub id: String,
    pub owner_user_id: String,
    pub agent_id: Option<String>,
    pub name: String,
    pub provider: String,
    pub domain: String,
    pub record_id: Option<String>,
    pub zone_id: Option<String>,
    pub api_token: Option<String>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub webhook_url: Option<String>,
    pub current_ip: Option<String>,
    pub last_applied_ip: Option<String>,
    pub last_applied_at: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub struct DdnsConfigRepository;

impl DdnsConfigRepository {
    pub async fn list_enabled(db: &Db) -> Result<Vec<DdnsConfigRow>> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(DDNS_CONFIG_SELECT_SQLITE_ENABLED)
                    .fetch_all(pool)
                    .await?;
                Ok(rows
                    .into_iter()
                    .map(sqlite_config_row_to_ddns)
                    .collect::<Result<Vec<_>>>()?)
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(DDNS_CONFIG_SELECT_POSTGRES_ENABLED)
                    .fetch_all(pool)
                    .await?;
                Ok(rows
                    .into_iter()
                    .map(postgres_config_row_to_ddns)
                    .collect::<Result<Vec<_>>>()?)
            }
        }
    }

    pub async fn get_by_id(db: &Db, id: &str) -> Result<Option<DdnsConfigRow>> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row_opt = sqlx::query(DDNS_CONFIG_SELECT_SQLITE_BY_ID)
                    .bind(id)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(sqlite_config_row_to_ddns).transpose()
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let row_opt = sqlx::query(DDNS_CONFIG_SELECT_POSTGRES_BY_ID)
                    .bind(parse_uuid(id, "id")?)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(postgres_config_row_to_ddns).transpose()
            }
        }
    }

    pub async fn create(db: &Db, row: &DdnsConfigRow) -> Result<()> {
        let enabled_int = if row.enabled { 1i64 } else { 0i64 };
        let api_token = encrypt_optional_secret(row.api_token.as_deref())?;
        let api_key = encrypt_optional_secret(row.api_key.as_deref())?;
        let api_secret = encrypt_optional_secret(row.api_secret.as_deref())?;
        let webhook_url = encrypt_optional_secret(row.webhook_url.as_deref())?;
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO ddns_configs (id, owner_user_id, agent_id, name, provider, domain, record_id, zone_id, api_token, api_key, api_secret, webhook_url, current_ip, last_applied_ip, last_applied_at, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
                )
                    .bind(&row.id)
                    .bind(&row.owner_user_id)
                    .bind(&row.agent_id)
                    .bind(&row.name)
                    .bind(&row.provider)
                    .bind(&row.domain)
                    .bind(&row.record_id)
                    .bind(&row.zone_id)
                    .bind(&api_token)
                    .bind(&api_key)
                    .bind(&api_secret)
                    .bind(&webhook_url)
                    .bind(&row.current_ip)
                    .bind(&row.last_applied_ip)
                    .bind(&row.last_applied_at)
                    .bind(enabled_int)
                    .bind(&row.created_at)
                    .bind(&row.updated_at)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO ddns_configs (id, owner_user_id, agent_id, name, provider, domain, record_id, zone_id, api_token, api_key, api_secret, webhook_url, current_ip, last_applied_ip, last_applied_at, enabled, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)"
                )
                    .bind(parse_uuid(&row.id, "id")?)
                    .bind(parse_uuid(&row.owner_user_id, "owner_user_id")?)
                    .bind(parse_optional_uuid(row.agent_id.as_deref(), "agent_id")?)
                    .bind(&row.name)
                    .bind(&row.provider)
                    .bind(&row.domain)
                    .bind(&row.record_id)
                    .bind(&row.zone_id)
                    .bind(&api_token)
                    .bind(&api_key)
                    .bind(&api_secret)
                    .bind(&webhook_url)
                    .bind(&row.current_ip)
                    .bind(&row.last_applied_ip)
                    .bind(parse_optional_timestamp(row.last_applied_at.as_deref(), "last_applied_at")?)
                    .bind(row.enabled)
                    .bind(parse_timestamp(&row.created_at, "created_at")?)
                    .bind(parse_timestamp(&row.updated_at, "updated_at")?)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn delete(db: &Db, id: &str) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query("DELETE FROM ddns_configs WHERE id = ?")
                    .bind(id)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query("DELETE FROM ddns_configs WHERE id = $1")
                    .bind(parse_uuid(id, "id")?)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn record_history(
        db: &Db,
        history_id: &str,
        config_id: &str,
        old_ip: Option<&str>,
        new_ip: &str,
        success: bool,
        error: Option<&str>,
        applied_at: &str,
    ) -> Result<()> {
        let success_int = if success { 1i64 } else { 0i64 };
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO ddns_history (id, config_id, old_ip, new_ip, success, error, applied_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
                )
                .bind(history_id)
                .bind(config_id)
                .bind(old_ip)
                .bind(new_ip)
                .bind(success_int)
                .bind(error)
                .bind(applied_at)
                .execute(pool)
                .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO ddns_history (id, config_id, old_ip, new_ip, success, error, applied_at) VALUES ($1, $2, $3, $4, $5, $6, $7)"
                )
                .bind(parse_uuid(history_id, "history_id")?)
                .bind(parse_uuid(config_id, "config_id")?)
                .bind(old_ip)
                .bind(new_ip)
                .bind(success)
                .bind(error)
                .bind(parse_timestamp(applied_at, "applied_at")?)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    pub async fn update_after_apply(
        db: &Db,
        id: &str,
        last_applied_ip: &str,
        last_applied_at: &str,
    ) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE ddns_configs SET last_applied_ip = ?, last_applied_at = ? WHERE id = ?",
                )
                .bind(last_applied_ip)
                .bind(last_applied_at)
                .bind(id)
                .execute(pool)
                .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    "UPDATE ddns_configs SET last_applied_ip = $1, last_applied_at = $2 WHERE id = $3",
                )
                .bind(last_applied_ip)
                .bind(parse_timestamp(last_applied_at, "last_applied_at")?)
                .bind(parse_uuid(id, "id")?)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }
}

pub struct DdnsHistoryRepository;

impl DdnsHistoryRepository {
    pub async fn list_for_config(
        db: &Db,
        config_id: &str,
        limit: i64,
    ) -> Result<Vec<DdnsHistoryEntry>> {
        let mut entries: Vec<DdnsHistoryEntry> = Vec::new();
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(
                    "SELECT id, config_id, old_ip, new_ip, success, error, applied_at FROM ddns_history WHERE config_id = ? ORDER BY applied_at DESC LIMIT ?",
                )
                    .bind(config_id)
                    .bind(limit)
                    .fetch_all(pool)
                    .await?;
                for r in rows {
                    entries.push(sqlite_history_row_to_entry(r)?);
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(
                    "SELECT id, config_id, old_ip, new_ip, success, error, applied_at FROM ddns_history WHERE config_id = $1 ORDER BY applied_at DESC LIMIT $2",
                )
                    .bind(parse_uuid(config_id, "config_id")?)
                    .bind(limit)
                    .fetch_all(pool)
                    .await?;
                for r in rows {
                    entries.push(postgres_history_row_to_entry(r)?);
                }
            }
        }
        Ok(entries)
    }
}

macro_rules! ddns_config_select {
    ($suffix:literal) => {
        concat!(
            "SELECT id, owner_user_id, agent_id, name, provider, domain, record_id, zone_id, ",
            "api_token, api_key, api_secret, webhook_url, current_ip, last_applied_ip, ",
            "last_applied_at, enabled, created_at, updated_at FROM ddns_configs ",
            $suffix
        )
    };
}

const DDNS_CONFIG_SELECT_SQLITE_ENABLED: &str = ddns_config_select!("WHERE enabled = 1");
const DDNS_CONFIG_SELECT_POSTGRES_ENABLED: &str = ddns_config_select!("WHERE enabled = TRUE");
const DDNS_CONFIG_SELECT_SQLITE_BY_ID: &str = ddns_config_select!("WHERE id = ?");
const DDNS_CONFIG_SELECT_POSTGRES_BY_ID: &str = ddns_config_select!("WHERE id = $1");

fn sqlite_config_row_to_ddns(row: SqliteRow) -> Result<DdnsConfigRow> {
    Ok(DdnsConfigRow {
        id: row.try_get("id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        agent_id: row.try_get("agent_id")?,
        name: row.try_get("name")?,
        provider: row.try_get("provider")?,
        domain: row.try_get("domain")?,
        record_id: row.try_get("record_id")?,
        zone_id: row.try_get("zone_id")?,
        api_token: decrypt_optional_secret(row.try_get("api_token")?)?,
        api_key: decrypt_optional_secret(row.try_get("api_key")?)?,
        api_secret: decrypt_optional_secret(row.try_get("api_secret")?)?,
        webhook_url: decrypt_optional_secret(row.try_get("webhook_url")?)?,
        current_ip: row.try_get("current_ip")?,
        last_applied_ip: row.try_get("last_applied_ip")?,
        last_applied_at: row.try_get("last_applied_at")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn postgres_config_row_to_ddns(row: PgRow) -> Result<DdnsConfigRow> {
    let id: uuid::Uuid = row.try_get("id")?;
    let owner_user_id: uuid::Uuid = row.try_get("owner_user_id")?;
    let agent_id: Option<uuid::Uuid> = row.try_get("agent_id")?;
    let last_applied_at: Option<DateTime<Utc>> = row.try_get("last_applied_at")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

    Ok(DdnsConfigRow {
        id: id.to_string(),
        owner_user_id: owner_user_id.to_string(),
        agent_id: agent_id.map(|value| value.to_string()),
        name: row.try_get("name")?,
        provider: row.try_get("provider")?,
        domain: row.try_get("domain")?,
        record_id: row.try_get("record_id")?,
        zone_id: row.try_get("zone_id")?,
        api_token: decrypt_optional_secret(row.try_get("api_token")?)?,
        api_key: decrypt_optional_secret(row.try_get("api_key")?)?,
        api_secret: decrypt_optional_secret(row.try_get("api_secret")?)?,
        webhook_url: decrypt_optional_secret(row.try_get("webhook_url")?)?,
        current_ip: row.try_get("current_ip")?,
        last_applied_ip: row.try_get("last_applied_ip")?,
        last_applied_at: last_applied_at.map(|value| value.to_rfc3339()),
        enabled: row.try_get("enabled")?,
        created_at: created_at.to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
    })
}

fn sqlite_history_row_to_entry(row: SqliteRow) -> Result<DdnsHistoryEntry> {
    Ok(DdnsHistoryEntry {
        id: row.try_get("id")?,
        config_id: row.try_get("config_id")?,
        old_ip: row.try_get("old_ip")?,
        new_ip: row.try_get("new_ip")?,
        success: row.try_get::<i64, _>("success")? != 0,
        error: row.try_get("error")?,
        applied_at: row.try_get("applied_at")?,
    })
}

fn postgres_history_row_to_entry(row: PgRow) -> Result<DdnsHistoryEntry> {
    let id: uuid::Uuid = row.try_get("id")?;
    let config_id: uuid::Uuid = row.try_get("config_id")?;
    let applied_at: DateTime<Utc> = row.try_get("applied_at")?;

    Ok(DdnsHistoryEntry {
        id: id.to_string(),
        config_id: config_id.to_string(),
        old_ip: row.try_get("old_ip")?,
        new_ip: row.try_get("new_ip")?,
        success: row.try_get("success")?,
        error: row.try_get("error")?,
        applied_at: applied_at.to_rfc3339(),
    })
}

fn parse_uuid(value: &str, field: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(value).with_context(|| format!("invalid DDNS {field} UUID"))
}

fn parse_optional_uuid(value: Option<&str>, field: &str) -> Result<Option<uuid::Uuid>> {
    value.map(|value| parse_uuid(value, field)).transpose()
}

fn parse_timestamp(value: &str, field: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .with_context(|| format!("invalid DDNS {field} timestamp"))
}

fn parse_optional_timestamp(value: Option<&str>, field: &str) -> Result<Option<DateTime<Utc>>> {
    value.map(|value| parse_timestamp(value, field)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CreateUserInput, DatabaseBackend, UserRepository};
    use crate::secrets::is_encrypted_secret;
    use xlstatus_shared::UserRole;

    #[tokio::test]
    async fn ddns_create_encrypts_secret_columns_and_reads_plaintext() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();

        let row = DdnsConfigRow {
            id: uuid::Uuid::now_v7().to_string(),
            owner_user_id: user.id.0.to_string(),
            agent_id: None,
            name: "cf".into(),
            provider: "cloudflare".into(),
            domain: "example.com".into(),
            record_id: None,
            zone_id: Some("zone-id".into()),
            api_token: Some("cloudflare-token".into()),
            api_key: Some("key".into()),
            api_secret: Some("secret".into()),
            webhook_url: Some("https://hook.example.com/path?token=secret".into()),
            current_ip: None,
            last_applied_ip: None,
            last_applied_at: None,
            enabled: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        DdnsConfigRepository::create(&db, &row).await.unwrap();

        let raw = raw_secret_columns(&db, &row.id).await.unwrap();
        assert!(is_encrypted_secret(raw.api_token.as_deref().unwrap()));
        assert!(is_encrypted_secret(raw.api_key.as_deref().unwrap()));
        assert!(is_encrypted_secret(raw.api_secret.as_deref().unwrap()));
        assert!(is_encrypted_secret(raw.webhook_url.as_deref().unwrap()));
        assert_ne!(raw.api_token.as_deref(), Some("cloudflare-token"));

        let found = DdnsConfigRepository::get_by_id(&db, &row.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.api_token.as_deref(), Some("cloudflare-token"));
        assert_eq!(found.api_key.as_deref(), Some("key"));
        assert_eq!(found.api_secret.as_deref(), Some("secret"));
        assert_eq!(
            found.webhook_url.as_deref(),
            Some("https://hook.example.com/path?token=secret")
        );
    }

    async fn test_db() -> DatabaseBackend {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-ddns-secret-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    #[derive(Debug)]
    struct RawSecretColumns {
        api_token: Option<String>,
        api_key: Option<String>,
        api_secret: Option<String>,
        webhook_url: Option<String>,
    }

    async fn raw_secret_columns(db: &DatabaseBackend, id: &str) -> Result<RawSecretColumns> {
        match db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query(
                    "SELECT api_token, api_key, api_secret, webhook_url FROM ddns_configs WHERE id = ?",
                )
                .bind(id)
                .fetch_one(pool)
                .await?;
                Ok(RawSecretColumns {
                    api_token: row.try_get("api_token")?,
                    api_key: row.try_get("api_key")?,
                    api_secret: row.try_get("api_secret")?,
                    webhook_url: row.try_get("webhook_url")?,
                })
            }
            DatabaseBackend::Postgres(_) => unreachable!(),
        }
    }
}
