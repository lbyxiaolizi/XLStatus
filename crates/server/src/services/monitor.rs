//! M4 service monitor: periodic HTTP / TCP / ICMP probes persisted
//! into `service_results`. A separate API surface in
//! `api/v1/service_history.rs` reads back from the same table for the
//! dashboard's history view.

use crate::db::Db;
use crate::services::probe::{probe_http, probe_icmp, probe_tcp, ProbeType};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub enabled: bool,
    pub notification_group_id: Option<String>,
}

pub struct ServiceMonitor {
    db: Db,
    scheduled: Arc<RwLock<HashMap<String, chrono::DateTime<Utc>>>>,
}

impl ServiceMonitor {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            scheduled: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn start(self: Arc<Self>) {
        info!("service monitor started");
        let mut tick = interval(Duration::from_secs(10));
        loop {
            tick.tick().await;
            if let Err(e) = self.tick_once().await {
                error!("service monitor tick: {}", e);
            }
        }
    }

    async fn tick_once(&self) -> Result<()> {
        let now = Utc::now();
        let services = self.load_services().await?;
        for s in services {
            let should = {
                let sch = self.scheduled.read().await;
                sch.get(&s.id).map(|t| now >= *t).unwrap_or(true)
            };
            if should {
                let next = now + chrono::Duration::seconds(s.interval_seconds as i64);
                self.scheduled.write().await.insert(s.id.clone(), next);
                if let Err(e) = self.probe(&s).await {
                    warn!("probe {} failed: {}", s.id, e);
                }
            }
        }
        Ok(())
    }

    async fn probe(&self, s: &ServiceConfig) -> Result<()> {
        let probe_type = ProbeType::from_str(&s.kind).context("invalid kind")?;
        let result = match probe_type {
            ProbeType::Http => probe_http(&s.target, s.timeout_seconds).await?,
            ProbeType::Tcp => {
                let parts: Vec<&str> = s.target.split(':').collect();
                if parts.len() != 2 {
                    anyhow::bail!("tcp target must be host:port");
                }
                let host = parts[0];
                let port: u16 = parts[1].parse().context("invalid port")?;
                probe_tcp(host, port, s.timeout_seconds).await?
            }
            ProbeType::Icmp => probe_icmp(&s.target, s.timeout_seconds).await?,
        };
        self.save(&s.id, &result).await?;
        Ok(())
    }

    async fn load_services(&self) -> Result<Vec<ServiceConfig>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows: Vec<(
                    String, String, String, String, i64, i64, i64, Option<String>,
                )> = sqlx::query_as(
                    "SELECT id, name, type, target, interval_seconds, timeout_seconds, enabled, notification_group_id FROM services WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                Ok(rows
                    .into_iter()
                    .map(
                        |(
                            id,
                            name,
                            kind,
                            target,
                            interval_seconds,
                            timeout_seconds,
                            enabled,
                            notification_group_id,
                        )| {
                            ServiceConfig {
                                id,
                                name,
                                kind,
                                target,
                                interval_seconds: interval_seconds as u64,
                                timeout_seconds: timeout_seconds as u64,
                                enabled: enabled != 0,
                                notification_group_id,
                            }
                        },
                    )
                    .collect())
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(
                    String, String, String, String, i32, i32, bool, Option<String>,
                )> = sqlx::query_as(
                    "SELECT id::text, name, type, target, interval_seconds, timeout_seconds, enabled, notification_group_id::text FROM services WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                Ok(rows
                    .into_iter()
                    .map(
                        |(
                            id,
                            name,
                            kind,
                            target,
                            interval_seconds,
                            timeout_seconds,
                            enabled,
                            notification_group_id,
                        )| {
                            ServiceConfig {
                                id,
                                name,
                                kind,
                                target,
                                interval_seconds: interval_seconds as u64,
                                timeout_seconds: timeout_seconds as u64,
                                enabled,
                                notification_group_id,
                            }
                        },
                    )
                    .collect())
            }
        }
    }

    async fn save(
        &self,
        service_id: &str,
        result: &crate::services::probe::ServiceProbe,
    ) -> Result<()> {
        let id = uuid::Uuid::now_v7().to_string();
        let status = if result.success { "success" } else { "failure" };
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO service_results (id, service_id, status, delay_ms, status_code, error, cert_fingerprint, cert_not_after, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(service_id)
                .bind(status)
                .bind(result.latency_ms)
                .bind(result.status_code)
                .bind(&result.error)
                .bind(&result.cert_fingerprint)
                .bind(result.cert_not_after.as_ref().map(|ts| ts.to_rfc3339()))
                .bind(&now_str)
                .execute(pool)
                .await?;
                // Mirror into service_history for the legacy read API.
                let _ = sqlx::query(
                    "INSERT INTO service_history (id, service_id, success, latency_ms, status_code, error, checked_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(service_id)
                .bind(if result.success { 1i32 } else { 0i32 })
                .bind(result.latency_ms)
                .bind(result.status_code)
                .bind(&result.error)
                .bind(&now_str)
                .execute(pool)
                .await;
                self.prune_old_sqlite(pool).await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let pid = uuid::Uuid::parse_str(&id)?;
                let psid = uuid::Uuid::parse_str(service_id)?;
                sqlx::query(
                    "INSERT INTO service_results (id, service_id, status, delay_ms, status_code, error, cert_fingerprint, cert_not_after, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                )
                .bind(pid)
                .bind(psid)
                .bind(status)
                .bind(result.latency_ms)
                .bind(result.status_code)
                .bind(&result.error)
                .bind(&result.cert_fingerprint)
                .bind(result.cert_not_after.clone())
                .bind(now)
                .execute(pool)
                .await?;
                self.prune_old_postgres(pool).await?;
            }
        }
        Ok(())
    }

    async fn prune_old_sqlite(&self, pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<()> {
        let cutoff = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        sqlx::query("DELETE FROM service_results WHERE created_at < ?")
            .bind(&cutoff)
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM service_history WHERE checked_at < ?")
            .bind(&cutoff)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn prune_old_postgres(&self, pool: &sqlx::Pool<sqlx::Postgres>) -> Result<()> {
        let cutoff = Utc::now() - chrono::Duration::days(30);
        sqlx::query("DELETE FROM service_results WHERE created_at < $1")
            .bind(cutoff)
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM service_history WHERE checked_at < $1")
            .bind(cutoff)
            .execute(pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trip() {
        let cfg = ServiceConfig {
            id: "x".into(),
            name: "demo".into(),
            kind: "http".into(),
            target: "http://127.0.0.1:9".into(),
            interval_seconds: 30,
            timeout_seconds: 5,
            enabled: true,
            notification_group_id: None,
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: ServiceConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, "x");
    }
}
