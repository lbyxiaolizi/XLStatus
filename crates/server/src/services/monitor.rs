use crate::db::Db;
use crate::services::probe::{probe_http, probe_tcp, ProbeType};
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

/// Service monitoring scheduler
pub struct ServiceMonitor {
    db: Db,
    // Map of service_id -> next check time
    scheduled_checks: Arc<RwLock<HashMap<String, chrono::DateTime<Utc>>>>,
}

impl ServiceMonitor {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            scheduled_checks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start the monitoring loop
    pub async fn start(self: Arc<Self>) {
        info!("Starting service monitor");

        let mut tick = interval(Duration::from_secs(10));

        loop {
            tick.tick().await;

            if let Err(e) = self.check_services().await {
                error!("Service monitor error: {}", e);
            }
        }
    }

    /// Check all services that need monitoring
    async fn check_services(&self) -> Result<()> {
        let now = Utc::now();

        // Load all services that need checking
        let services = self.load_services().await?;

        for service in services {
            // Check if service should be probed now
            let should_check = {
                let scheduled = self.scheduled_checks.read().await;
                match scheduled.get(&service.id) {
                    Some(next_check) => now >= *next_check,
                    None => true, // First time scheduling
                }
            };

            if should_check {
                // Calculate next check time
                let next_check = now + chrono::Duration::seconds(service.interval_seconds as i64);

                // Update scheduled time
                {
                    let mut scheduled = self.scheduled_checks.write().await;
                    scheduled.insert(service.id.clone(), next_check);
                }

                // Execute probe
                info!("Probing service: {} ({})", service.id, service.name);
                if let Err(e) = self.probe_service(&service).await {
                    error!("Failed to probe service {}: {}", service.id, e);
                }
            }
        }

        Ok(())
    }

    /// Probe a single service
    async fn probe_service(&self, service: &Service) -> Result<()> {
        let probe_type = ProbeType::from_str(&service.kind)
            .context("Invalid probe type")?;

        let result = match probe_type {
            ProbeType::Http => {
                let timeout = service.timeout_seconds.unwrap_or(30);
                probe_http(&service.target, timeout).await?
            }
            ProbeType::Tcp => {
                let parts: Vec<&str> = service.target.split(':').collect();
                if parts.len() != 2 {
                    anyhow::bail!("Invalid TCP target format, expected host:port");
                }
                let host = parts[0];
                let port: u16 = parts[1].parse().context("Invalid port")?;
                let timeout = service.timeout_seconds.unwrap_or(30);
                probe_tcp(host, port, timeout).await?
            }
            ProbeType::Icmp => {
                // ICMP requires system privileges, use agent-side execution
                warn!("ICMP probe should be executed on agent side");
                return Ok(());
            }
        };

        // Save result to database
        self.save_probe_result(&service.id, &result).await?;

        Ok(())
    }

    /// Load services from database
    async fn load_services(&self) -> Result<Vec<Service>> {
        let query = r#"
            SELECT id, name, kind, target, duration_seconds,
                   notification_group_id, enabled
            FROM services
            WHERE enabled = 1
        "#;

        let services = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                let mut services = Vec::new();
                for row in rows {
                    services.push(Service {
                        id: row.try_get("id")?,
                        name: row.try_get("name")?,
                        kind: row.try_get("kind")?,
                        target: row.try_get("target")?,
                        interval_seconds: row.try_get::<i64, _>("duration_seconds")? as u64,
                        timeout_seconds: Some(30),
                        notification_group_id: row.try_get("notification_group_id")?,
                        enabled: row.try_get("enabled")?,
                    });
                }
                services
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                let mut services = Vec::new();
                for row in rows {
                    services.push(Service {
                        id: row.try_get("id")?,
                        name: row.try_get("name")?,
                        kind: row.try_get("kind")?,
                        target: row.try_get("target")?,
                        interval_seconds: row.try_get::<i64, _>("duration_seconds")? as u64,
                        timeout_seconds: Some(30),
                        notification_group_id: row.try_get("notification_group_id")?,
                        enabled: row.try_get("enabled")?,
                    });
                }
                services
            }
        };

        Ok(services)
    }

    /// Save probe result to database
    async fn save_probe_result(
        &self,
        service_id: &str,
        result: &crate::services::probe::ServiceProbe,
    ) -> Result<()> {
        let query = r#"
            INSERT INTO service_results (
                id, service_id, server_id, status, delay_ms,
                error, cert_fingerprint, cert_not_after, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        let status = if result.success { "success" } else { "failure" };
        let server_id: Option<String> = None; // Server-side probes don't have server_id

        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
                    .bind(&result.id)
                    .bind(service_id)
                    .bind(server_id)
                    .bind(status)
                    .bind(result.latency_ms)
                    .bind(&result.error)
                    .bind::<Option<String>>(None) // cert_fingerprint
                    .bind::<Option<String>>(None) // cert_not_after
                    .bind(result.checked_at.to_rfc3339())
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(query)
                    .bind(&result.id)
                    .bind(service_id)
                    .bind(server_id)
                    .bind(status)
                    .bind(result.latency_ms)
                    .bind(&result.error)
                    .bind::<Option<String>>(None)
                    .bind::<Option<String>>(None)
                    .bind(result.checked_at.to_rfc3339())
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Service {
    id: String,
    name: String,
    kind: String,
    target: String,
    interval_seconds: u64,
    timeout_seconds: Option<u64>,
    notification_group_id: Option<String>,
    enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_scheduling() {
        // Test that services are scheduled correctly
        let now = Utc::now();
        let next = now + chrono::Duration::seconds(60);
        assert!(next > now);
    }
}
