#![allow(dead_code)]
#![allow(unused)]

use crate::db::Db;
use anyhow::Result;
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
        let query = "SELECT id, owner_user_id, agent_id, name, provider, domain, record_id, zone_id, api_token, api_key, api_secret, webhook_url, current_ip, last_applied_ip, last_applied_at, enabled, created_at, updated_at FROM ddns_configs WHERE enabled = 1";
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                Ok(rows
                    .into_iter()
                    .map(|r| {
                        Ok(DdnsConfigRow {
                            id: r.try_get("id")?,
                            owner_user_id: r.try_get("owner_user_id")?,
                            agent_id: r.try_get("agent_id")?,
                            name: r.try_get("name")?,
                            provider: r.try_get("provider")?,
                            domain: r.try_get("domain")?,
                            record_id: r.try_get("record_id")?,
                            zone_id: r.try_get("zone_id")?,
                            api_token: r.try_get("api_token")?,
                            api_key: r.try_get("api_key")?,
                            api_secret: r.try_get("api_secret")?,
                            webhook_url: r.try_get("webhook_url")?,
                            current_ip: r.try_get("current_ip")?,
                            last_applied_ip: r.try_get("last_applied_ip")?,
                            last_applied_at: r.try_get("last_applied_at")?,
                            enabled: r.try_get::<i64, _>("enabled")? != 0,
                            created_at: r.try_get("created_at")?,
                            updated_at: r.try_get("updated_at")?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?)
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                Ok(rows
                    .into_iter()
                    .map(|r| {
                        Ok(DdnsConfigRow {
                            id: r.try_get("id")?,
                            owner_user_id: r.try_get("owner_user_id")?,
                            agent_id: r.try_get("agent_id")?,
                            name: r.try_get("name")?,
                            provider: r.try_get("provider")?,
                            domain: r.try_get("domain")?,
                            record_id: r.try_get("record_id")?,
                            zone_id: r.try_get("zone_id")?,
                            api_token: r.try_get("api_token")?,
                            api_key: r.try_get("api_key")?,
                            api_secret: r.try_get("api_secret")?,
                            webhook_url: r.try_get("webhook_url")?,
                            current_ip: r.try_get("current_ip")?,
                            last_applied_ip: r.try_get("last_applied_ip")?,
                            last_applied_at: r.try_get("last_applied_at")?,
                            enabled: r.try_get::<i64, _>("enabled")? != 0,
                            created_at: r.try_get("created_at")?,
                            updated_at: r.try_get("updated_at")?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?)
            }
        }
    }

    pub async fn get_by_id(db: &Db, id: &str) -> Result<Option<DdnsConfigRow>> {
        let query = "SELECT id, owner_user_id, agent_id, name, provider, domain, record_id, zone_id, api_token, api_key, api_secret, webhook_url, current_ip, last_applied_ip, last_applied_at, enabled, created_at, updated_at FROM ddns_configs WHERE id = ?";
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row_opt = sqlx::query(query).bind(id).fetch_optional(pool).await?;
                match row_opt {
                    Some(r) => Ok(Some(DdnsConfigRow {
                        id: r.try_get("id")?,
                        owner_user_id: r.try_get("owner_user_id")?,
                        agent_id: r.try_get("agent_id")?,
                        name: r.try_get("name")?,
                        provider: r.try_get("provider")?,
                        domain: r.try_get("domain")?,
                        record_id: r.try_get("record_id")?,
                        zone_id: r.try_get("zone_id")?,
                        api_token: r.try_get("api_token")?,
                        api_key: r.try_get("api_key")?,
                        api_secret: r.try_get("api_secret")?,
                        webhook_url: r.try_get("webhook_url")?,
                        current_ip: r.try_get("current_ip")?,
                        last_applied_ip: r.try_get("last_applied_ip")?,
                        last_applied_at: r.try_get("last_applied_at")?,
                        enabled: r.try_get::<i64, _>("enabled")? != 0,
                        created_at: r.try_get("created_at")?,
                        updated_at: r.try_get("updated_at")?,
                    })),
                    None => Ok(None),
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let row_opt = sqlx::query(query).bind(id).fetch_optional(pool).await?;
                match row_opt {
                    Some(r) => Ok(Some(DdnsConfigRow {
                        id: r.try_get("id")?,
                        owner_user_id: r.try_get("owner_user_id")?,
                        agent_id: r.try_get("agent_id")?,
                        name: r.try_get("name")?,
                        provider: r.try_get("provider")?,
                        domain: r.try_get("domain")?,
                        record_id: r.try_get("record_id")?,
                        zone_id: r.try_get("zone_id")?,
                        api_token: r.try_get("api_token")?,
                        api_key: r.try_get("api_key")?,
                        api_secret: r.try_get("api_secret")?,
                        webhook_url: r.try_get("webhook_url")?,
                        current_ip: r.try_get("current_ip")?,
                        last_applied_ip: r.try_get("last_applied_ip")?,
                        last_applied_at: r.try_get("last_applied_at")?,
                        enabled: r.try_get::<i64, _>("enabled")? != 0,
                        created_at: r.try_get("created_at")?,
                        updated_at: r.try_get("updated_at")?,
                    })),
                    None => Ok(None),
                }
            }
        }
    }

    pub async fn create(db: &Db, row: &DdnsConfigRow) -> Result<()> {
        let query = "INSERT INTO ddns_configs (id, owner_user_id, agent_id, name, provider, domain, record_id, zone_id, api_token, api_key, api_secret, webhook_url, current_ip, last_applied_ip, last_applied_at, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
        let enabled_int = if row.enabled { 1i64 } else { 0i64 };
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
                    .bind(&row.id)
                    .bind(&row.owner_user_id)
                    .bind(&row.agent_id)
                    .bind(&row.name)
                    .bind(&row.provider)
                    .bind(&row.domain)
                    .bind(&row.record_id)
                    .bind(&row.zone_id)
                    .bind(&row.api_token)
                    .bind(&row.api_key)
                    .bind(&row.api_secret)
                    .bind(&row.webhook_url)
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
                sqlx::query(query)
                    .bind(&row.id)
                    .bind(&row.owner_user_id)
                    .bind(&row.agent_id)
                    .bind(&row.name)
                    .bind(&row.provider)
                    .bind(&row.domain)
                    .bind(&row.record_id)
                    .bind(&row.zone_id)
                    .bind(&row.api_token)
                    .bind(&row.api_key)
                    .bind(&row.api_secret)
                    .bind(&row.webhook_url)
                    .bind(&row.current_ip)
                    .bind(&row.last_applied_ip)
                    .bind(&row.last_applied_at)
                    .bind(enabled_int)
                    .bind(&row.created_at)
                    .bind(&row.updated_at)
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
                sqlx::query("DELETE FROM ddns_configs WHERE id = ?")
                    .bind(id)
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
                    "UPDATE ddns_configs SET last_applied_ip = ?, last_applied_at = ? WHERE id = ?",
                )
                .bind(last_applied_ip)
                .bind(last_applied_at)
                .bind(id)
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
        let query = "SELECT id, config_id, old_ip, new_ip, success, error, applied_at FROM ddns_history WHERE config_id = ? ORDER BY applied_at DESC LIMIT ?";
        let mut entries: Vec<DdnsHistoryEntry> = Vec::new();
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(query)
                    .bind(config_id)
                    .bind(limit)
                    .fetch_all(pool)
                    .await?;
                for r in rows {
                    entries.push(DdnsHistoryEntry {
                        id: r.try_get("id")?,
                        config_id: r.try_get("config_id")?,
                        old_ip: r.try_get("old_ip")?,
                        new_ip: r.try_get("new_ip")?,
                        success: r.try_get::<i64, _>("success")? != 0,
                        error: r.try_get("error")?,
                        applied_at: r.try_get("applied_at")?,
                    });
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(query)
                    .bind(config_id)
                    .bind(limit)
                    .fetch_all(pool)
                    .await?;
                for r in rows {
                    entries.push(DdnsHistoryEntry {
                        id: r.try_get("id")?,
                        config_id: r.try_get("config_id")?,
                        old_ip: r.try_get("old_ip")?,
                        new_ip: r.try_get("new_ip")?,
                        success: r.try_get::<i64, _>("success")? != 0,
                        error: r.try_get("error")?,
                        applied_at: r.try_get("applied_at")?,
                    });
                }
            }
        }
        Ok(entries)
    }
}
