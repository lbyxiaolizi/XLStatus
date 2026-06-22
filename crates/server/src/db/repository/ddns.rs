#![allow(dead_code)]
#![allow(unused)]

use crate::db::Db;
use crate::ddns::policy::{
    normalize_ddns_provider, normalize_optional_ddns_text, normalize_required_ddns_text,
    DDNS_MAX_DOMAIN_BYTES, DDNS_MAX_NAME_BYTES, DDNS_MAX_RECORD_ID_BYTES, DDNS_MAX_SECRET_BYTES,
    DDNS_MAX_WEBHOOK_URL_BYTES, DDNS_MAX_ZONE_ID_BYTES,
};
use crate::secrets::{decrypt_optional_secret, encrypt_optional_secret};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use tracing::warn;
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
                    .filter_map(|row| match sqlite_config_row_to_ddns(row) {
                        Ok(row) => match validate_runtime_ddns_config(row) {
                            Ok(row) => Some(row),
                            Err(err) => {
                                warn!("skipping invalid historical DDNS config: {err:#}");
                                None
                            }
                        },
                        Err(err) => {
                            warn!("skipping unreadable historical DDNS config: {err:#}");
                            None
                        }
                    })
                    .collect())
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(DDNS_CONFIG_SELECT_POSTGRES_ENABLED)
                    .fetch_all(pool)
                    .await?;
                Ok(rows
                    .into_iter()
                    .filter_map(|row| match postgres_config_row_to_ddns(row) {
                        Ok(row) => match validate_runtime_ddns_config(row) {
                            Ok(row) => Some(row),
                            Err(err) => {
                                warn!("skipping invalid historical DDNS config: {err:#}");
                                None
                            }
                        },
                        Err(err) => {
                            warn!("skipping unreadable historical DDNS config: {err:#}");
                            None
                        }
                    })
                    .collect())
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

fn validate_runtime_ddns_config(row: DdnsConfigRow) -> Result<DdnsConfigRow> {
    let id = require_runtime_uuid_text(&row.id, "id")?;
    let owner_user_id = normalize_runtime_uuid_text(&row.owner_user_id, "owner_user_id")?;
    let agent_id = normalize_optional_runtime_uuid(row.agent_id, "agent_id")?;
    let name = normalize_required_ddns_text(row.name, DDNS_MAX_NAME_BYTES, "name")
        .map_err(|err| anyhow::anyhow!("invalid historical DDNS name: {err}"))?;
    let provider = normalize_ddns_provider(&row.provider)
        .map_err(|err| anyhow::anyhow!("invalid historical DDNS provider: {err}"))?;
    let domain = normalize_required_ddns_text(row.domain, DDNS_MAX_DOMAIN_BYTES, "domain")
        .map_err(|err| anyhow::anyhow!("invalid historical DDNS domain: {err}"))?;
    let record_id =
        normalize_optional_ddns_text(row.record_id, DDNS_MAX_RECORD_ID_BYTES, "record_id")
            .map_err(|err| anyhow::anyhow!("invalid historical DDNS record_id: {err}"))?;
    let zone_id = normalize_optional_ddns_text(row.zone_id, DDNS_MAX_ZONE_ID_BYTES, "zone_id")
        .map_err(|err| anyhow::anyhow!("invalid historical DDNS zone_id: {err}"))?;
    let api_token = normalize_optional_ddns_text(row.api_token, DDNS_MAX_SECRET_BYTES, "api_token")
        .map_err(|err| anyhow::anyhow!("invalid historical DDNS api_token: {err}"))?;
    let api_key = normalize_optional_ddns_text(row.api_key, DDNS_MAX_SECRET_BYTES, "api_key")
        .map_err(|err| anyhow::anyhow!("invalid historical DDNS api_key: {err}"))?;
    let api_secret =
        normalize_optional_ddns_text(row.api_secret, DDNS_MAX_SECRET_BYTES, "api_secret")
            .map_err(|err| anyhow::anyhow!("invalid historical DDNS api_secret: {err}"))?;
    let webhook_url =
        normalize_optional_ddns_text(row.webhook_url, DDNS_MAX_WEBHOOK_URL_BYTES, "webhook_url")
            .map_err(|err| anyhow::anyhow!("invalid historical DDNS webhook_url: {err}"))?;
    if provider == ProviderType::Webhook.as_str() && webhook_url.is_none() {
        anyhow::bail!("invalid historical DDNS webhook_url: webhook_url is required");
    }
    let current_ip = normalize_optional_runtime_ip(row.current_ip, "current_ip")?;
    let last_applied_ip = normalize_optional_runtime_ip(row.last_applied_ip, "last_applied_ip")?;
    if let Some(value) = row.last_applied_at.as_deref() {
        parse_timestamp(value, "last_applied_at")?;
    }

    Ok(DdnsConfigRow {
        id,
        owner_user_id,
        agent_id,
        name,
        provider,
        domain,
        record_id,
        zone_id,
        api_token,
        api_key,
        api_secret,
        webhook_url,
        current_ip,
        last_applied_ip,
        last_applied_at: row.last_applied_at,
        enabled: row.enabled,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn require_runtime_uuid_text(value: &str, field: &str) -> Result<String> {
    let value = value.trim();
    let parsed = uuid::Uuid::parse_str(value)
        .with_context(|| format!("{field} must be a parseable UUID"))?;
    if value.len() != 36 {
        anyhow::bail!("{field} must be a 36 byte UUID");
    }
    Ok(value.to_string())
}

fn normalize_runtime_uuid_text(value: &str, field: &str) -> Result<String> {
    let parsed = uuid::Uuid::parse_str(value.trim())
        .with_context(|| format!("{field} must be a parseable UUID"))?;
    Ok(parsed.to_string())
}

fn normalize_optional_runtime_uuid(value: Option<String>, field: &str) -> Result<Option<String>> {
    value
        .map(|value| normalize_runtime_uuid_text(&value, field))
        .transpose()
}

fn normalize_optional_runtime_ip(value: Option<String>, field: &str) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 64 {
        anyhow::bail!("{field} must be at most 64 bytes");
    }
    Ok(Some(
        value
            .parse::<std::net::IpAddr>()
            .with_context(|| format!("{field} must be an IP address"))?
            .to_string(),
    ))
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

    #[tokio::test]
    async fn invalid_historical_ddns_runtime_rows_are_skipped() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "runtime-owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let owner_id = user.id.0.to_string();
        let valid_id = uuid::Uuid::now_v7().to_string();
        insert_raw_config(
            &db,
            RawDdnsConfig {
                id: valid_id.clone(),
                owner_user_id: owner_id.clone(),
                name: "valid",
                provider: "dummy",
                domain: "valid.example.com",
                api_token: None,
                webhook_url: None,
                last_applied_ip: Some("198.51.100.10"),
            },
        )
        .await;
        insert_raw_config(
            &db,
            RawDdnsConfig {
                id: uuid::Uuid::now_v7().to_string(),
                owner_user_id: owner_id.clone(),
                name: "bad-provider",
                provider: "route53",
                domain: "bad-provider.example.com",
                api_token: None,
                webhook_url: None,
                last_applied_ip: None,
            },
        )
        .await;
        insert_raw_config(
            &db,
            RawDdnsConfig {
                id: uuid::Uuid::now_v7().to_string(),
                owner_user_id: owner_id.clone(),
                name: "oversized-secret",
                provider: "cloudflare",
                domain: "oversized-secret.example.com",
                api_token: Some("s".repeat(DDNS_MAX_SECRET_BYTES + 1)),
                webhook_url: None,
                last_applied_ip: None,
            },
        )
        .await;
        insert_raw_config(
            &db,
            RawDdnsConfig {
                id: uuid::Uuid::now_v7().to_string(),
                owner_user_id: owner_id.clone(),
                name: "missing-webhook",
                provider: "webhook",
                domain: "missing-webhook.example.com",
                api_token: None,
                webhook_url: None,
                last_applied_ip: None,
            },
        )
        .await;
        insert_raw_config(
            &db,
            RawDdnsConfig {
                id: uuid::Uuid::now_v7().to_string(),
                owner_user_id: owner_id,
                name: "bad-ip",
                provider: "dummy",
                domain: "bad-ip.example.com",
                api_token: None,
                webhook_url: None,
                last_applied_ip: Some("not-an-ip"),
            },
        )
        .await;

        let configs = DdnsConfigRepository::list_enabled(&db).await.unwrap();

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].id, valid_id);
        assert_eq!(configs[0].last_applied_ip.as_deref(), Some("198.51.100.10"));
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

    struct RawDdnsConfig {
        id: String,
        owner_user_id: String,
        name: &'static str,
        provider: &'static str,
        domain: &'static str,
        api_token: Option<String>,
        webhook_url: Option<String>,
        last_applied_ip: Option<&'static str>,
    }

    async fn insert_raw_config(db: &DatabaseBackend, config: RawDdnsConfig) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO ddns_configs (id, owner_user_id, name, provider, domain, api_token, webhook_url, last_applied_ip, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(config.id)
        .bind(config.owner_user_id)
        .bind(config.name)
        .bind(config.provider)
        .bind(config.domain)
        .bind(config.api_token)
        .bind(config.webhook_url)
        .bind(config.last_applied_ip)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
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
