//! M4 service monitor: periodic HTTP / TCP / ICMP probes persisted
//! into `service_results`. A separate API surface in
//! `api/v1/service_history.rs` reads back from the same table for the
//! dashboard's history view.

use crate::db::Db;
use crate::grpc::{SessionRegistry, TaskResponseRegistry};
use crate::services::probe::{probe_http, probe_icmp, probe_tcp, ProbeType, ServiceProbe};
use crate::tasks::spawn_triggered_tasks;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{error, info, warn};
use xlstatus_proto_gen::xlstatus::v1::{
    server_task::Spec, HttpGetTask, IcmpPingTask, ServerTask, TaskOutcome, TaskType, TcpPingTask,
};
use xlstatus_shared::AgentId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub enabled: bool,
    pub cover_mode: String,
    pub server_ids: Vec<String>,
    pub exclude_server_ids: Vec<String>,
    pub notification_group_id: Option<String>,
    pub failure_task_ids: Vec<String>,
    pub recovery_task_ids: Vec<String>,
}

pub struct ServiceMonitor {
    db: Db,
    session_registry: SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    scheduled: Arc<RwLock<HashMap<String, chrono::DateTime<Utc>>>>,
    service_states: Arc<RwLock<HashMap<String, bool>>>,
}

impl ServiceMonitor {
    pub fn new(
        db: Db,
        session_registry: SessionRegistry,
        response_registry: Arc<TaskResponseRegistry>,
    ) -> Self {
        Self {
            db,
            session_registry,
            response_registry,
            scheduled: Arc::new(RwLock::new(HashMap::new())),
            service_states: Arc::new(RwLock::new(HashMap::new())),
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
        match s.cover_mode.as_str() {
            "all" | "exclude" => {
                for server_id in self.covered_agent_ids(s).await? {
                    if !self
                        .session_registry
                        .is_online(&AgentId(uuid::Uuid::parse_str(&server_id)?))
                        .await
                    {
                        continue;
                    }
                    let result = match self.probe_via_agent(s, &server_id).await {
                        Ok(result) => result,
                        Err(e) => failure_probe(format!("agent probe failed: {e}")),
                    };
                    self.save(s, Some(&server_id), &result).await?;
                }
                Ok(())
            }
            "specific" if !s.server_ids.is_empty() => {
                for server_id in &s.server_ids {
                    let result = match self.probe_via_agent(s, server_id).await {
                        Ok(result) => result,
                        Err(e) => failure_probe(format!("agent probe failed: {e}")),
                    };
                    self.save(s, Some(server_id), &result).await?;
                }
                Ok(())
            }
            _ => {
                let result = self.probe_locally(s).await?;
                self.save(s, None, &result).await?;
                Ok(())
            }
        }
    }

    async fn covered_agent_ids(&self, s: &ServiceConfig) -> Result<Vec<String>> {
        let all = self.load_agent_ids().await?;
        if s.cover_mode == "exclude" {
            Ok(all
                .into_iter()
                .filter(|id| !s.exclude_server_ids.iter().any(|excluded| excluded == id))
                .collect())
        } else {
            Ok(all)
        }
    }

    async fn load_agent_ids(&self) -> Result<Vec<String>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT id FROM agents WHERE revoked_at IS NULL ORDER BY created_at ASC",
                )
                .fetch_all(pool)
                .await?;
                Ok(rows.into_iter().map(|(id,)| id).collect())
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT id::text FROM agents WHERE revoked_at IS NULL ORDER BY created_at ASC",
                )
                .fetch_all(pool)
                .await?;
                Ok(rows.into_iter().map(|(id,)| id).collect())
            }
        }
    }

    async fn probe_locally(&self, s: &ServiceConfig) -> Result<ServiceProbe> {
        let probe_type = ProbeType::from_str(&s.kind).context("invalid kind")?;
        Ok(match probe_type {
            ProbeType::Http => probe_http(&s.target, s.timeout_seconds).await?,
            ProbeType::Tcp => {
                let (host, port) = parse_tcp_target(&s.target)?;
                probe_tcp(host, port, s.timeout_seconds).await?
            }
            ProbeType::Icmp => probe_icmp(&s.target, s.timeout_seconds).await?,
        })
    }

    async fn probe_via_agent(&self, s: &ServiceConfig, server_id: &str) -> Result<ServiceProbe> {
        let agent_id = AgentId(uuid::Uuid::parse_str(server_id).context("invalid server_id")?);
        if !self.session_registry.is_online(&agent_id).await {
            anyhow::bail!("agent offline");
        }

        let task_id = uuid::Uuid::now_v7().to_string();
        let task = build_agent_probe_task(&task_id, s)?;
        let rx = self.response_registry.register(task_id.clone()).await;
        if let Err(e) = self
            .session_registry
            .send_server_task(&agent_id, task)
            .await
        {
            self.response_registry.cancel(&task_id).await;
            anyhow::bail!(e);
        }

        let wait_seconds = s.timeout_seconds.saturating_add(5).max(5);
        let result = match tokio::time::timeout(Duration::from_secs(wait_seconds), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => anyhow::bail!("agent disconnected before reply"),
            Err(_) => {
                self.response_registry.cancel(&task_id).await;
                anyhow::bail!("agent probe timed out");
            }
        };
        service_probe_from_task_result(result)
    }

    async fn load_services(&self) -> Result<Vec<ServiceConfig>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows: Vec<(
                    String,
                    String,
                    String,
                    String,
                    i64,
                    i64,
                    i64,
                    Option<String>,
                    Option<String>,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                )> = sqlx::query_as(
                    "SELECT id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, notification_group_id, COALESCE(cover_mode, 'local'), exclude_server_ids_json, failure_task_ids_json, recovery_task_ids_json FROM services WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                let mut services = rows
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
                            server_id,
                            notification_group_id,
                            cover_mode,
                            exclude_server_ids_json,
                            failure_task_ids_json,
                            recovery_task_ids_json,
                        )| {
                            let server_ids = server_id.into_iter().collect();
                            ServiceConfig {
                                id,
                                name,
                                kind,
                                target,
                                interval_seconds: interval_seconds as u64,
                                timeout_seconds: timeout_seconds as u64,
                                enabled: enabled != 0,
                                cover_mode,
                                server_ids,
                                exclude_server_ids: parse_server_ids_json(exclude_server_ids_json),
                                notification_group_id,
                                failure_task_ids: parse_task_ids_json(failure_task_ids_json),
                                recovery_task_ids: parse_task_ids_json(recovery_task_ids_json),
                            }
                        },
                    )
                    .collect::<Vec<_>>();
                self.attach_service_server_ids(&mut services).await?;
                Ok(services)
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(
                    String,
                    String,
                    String,
                    String,
                    i32,
                    i32,
                    bool,
                    Option<String>,
                    Option<String>,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                )> = sqlx::query_as(
                    "SELECT id::text, name, type, target, interval_seconds, timeout_seconds, enabled, server_id::text, notification_group_id::text, COALESCE(cover_mode, 'local'), exclude_server_ids_json, failure_task_ids_json, recovery_task_ids_json FROM services WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                let mut services = rows
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
                            server_id,
                            notification_group_id,
                            cover_mode,
                            exclude_server_ids_json,
                            failure_task_ids_json,
                            recovery_task_ids_json,
                        )| {
                            let server_ids = server_id.into_iter().collect();
                            ServiceConfig {
                                id,
                                name,
                                kind,
                                target,
                                interval_seconds: interval_seconds as u64,
                                timeout_seconds: timeout_seconds as u64,
                                enabled,
                                cover_mode,
                                server_ids,
                                exclude_server_ids: parse_server_ids_json(exclude_server_ids_json),
                                notification_group_id,
                                failure_task_ids: parse_task_ids_json(failure_task_ids_json),
                                recovery_task_ids: parse_task_ids_json(recovery_task_ids_json),
                            }
                        },
                    )
                    .collect::<Vec<_>>();
                self.attach_service_server_ids(&mut services).await?;
                Ok(services)
            }
        }
    }

    async fn attach_service_server_ids(&self, services: &mut [ServiceConfig]) -> Result<()> {
        for service in services {
            let server_ids = self.load_service_server_ids(&service.id).await?;
            if !server_ids.is_empty() {
                service.server_ids = server_ids;
            }
        }
        Ok(())
    }

    async fn load_service_server_ids(&self, service_id: &str) -> Result<Vec<String>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT server_id FROM service_servers WHERE service_id = ? ORDER BY created_at ASC, server_id ASC",
                )
                .bind(service_id)
                .fetch_all(pool)
                .await?;
                Ok(rows.into_iter().map(|(id,)| id).collect())
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let parsed = uuid::Uuid::parse_str(service_id).context("invalid service_id")?;
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT server_id::text FROM service_servers WHERE service_id = $1 ORDER BY created_at ASC, server_id ASC",
                )
                .bind(parsed)
                .fetch_all(pool)
                .await?;
                Ok(rows.into_iter().map(|(id,)| id).collect())
            }
        }
    }

    async fn save(
        &self,
        service: &ServiceConfig,
        server_id: Option<&str>,
        result: &crate::services::probe::ServiceProbe,
    ) -> Result<()> {
        let service_id = &service.id;
        let previous_success = self.previous_service_success(service_id, server_id).await?;
        let id = uuid::Uuid::now_v7().to_string();
        let status = if result.success { "success" } else { "failure" };
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO service_results (id, service_id, server_id, status, delay_ms, status_code, error, cert_fingerprint, cert_not_after, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(service_id)
                .bind(server_id)
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
                let server_id = server_id.map(uuid::Uuid::parse_str).transpose()?;
                sqlx::query(
                    "INSERT INTO service_results (id, service_id, server_id, status, delay_ms, status_code, error, cert_fingerprint, cert_not_after, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                )
                .bind(pid)
                .bind(psid)
                .bind(server_id)
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
        self.handle_service_transition(service, server_id, result.success, previous_success)
            .await;
        Ok(())
    }

    async fn previous_service_success(
        &self,
        service_id: &str,
        server_id: Option<&str>,
    ) -> Result<Option<bool>> {
        let key = service_state_key(service_id, server_id);
        if let Some(value) = self.service_states.read().await.get(&key).copied() {
            return Ok(Some(value));
        }
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row: Option<(String,)> = match server_id {
                    Some(server_id) => sqlx::query_as(
                        "SELECT status FROM service_results WHERE service_id = ? AND server_id = ? ORDER BY created_at DESC LIMIT 1",
                    )
                    .bind(service_id)
                    .bind(server_id)
                    .fetch_optional(pool)
                    .await?,
                    None => sqlx::query_as(
                        "SELECT status FROM service_results WHERE service_id = ? AND server_id IS NULL ORDER BY created_at DESC LIMIT 1",
                    )
                    .bind(service_id)
                    .fetch_optional(pool)
                    .await?,
                };
                Ok(row.map(|(status,)| status == "success"))
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let service_id = uuid::Uuid::parse_str(service_id).context("invalid service_id")?;
                let row: Option<(String,)> = match server_id {
                    Some(server_id) => {
                        let server_id =
                            uuid::Uuid::parse_str(server_id).context("invalid server_id")?;
                        sqlx::query_as(
                            "SELECT status FROM service_results WHERE service_id = $1 AND server_id = $2 ORDER BY created_at DESC LIMIT 1",
                        )
                        .bind(service_id)
                        .bind(server_id)
                        .fetch_optional(pool)
                        .await?
                    }
                    None => sqlx::query_as(
                        "SELECT status FROM service_results WHERE service_id = $1 AND server_id IS NULL ORDER BY created_at DESC LIMIT 1",
                    )
                    .bind(service_id)
                    .fetch_optional(pool)
                    .await?,
                };
                Ok(row.map(|(status,)| status == "success"))
            }
        }
    }

    async fn handle_service_transition(
        &self,
        service: &ServiceConfig,
        server_id: Option<&str>,
        success: bool,
        previous_success: Option<bool>,
    ) {
        let key = service_state_key(&service.id, server_id);
        self.service_states.write().await.insert(key, success);
        if previous_success == Some(success) || (previous_success.is_none() && success) {
            return;
        }

        let task_ids = if success {
            &service.recovery_task_ids
        } else {
            &service.failure_task_ids
        };
        if task_ids.is_empty() {
            return;
        }
        spawn_triggered_tasks(
            self.db.clone(),
            self.session_registry.clone(),
            self.response_registry.clone(),
            task_ids.clone(),
            format!(
                "service:{}:{}:{}",
                service.id,
                server_id.unwrap_or("local"),
                if success { "recovered" } else { "failed" }
            ),
            server_id.map(str::to_string),
        );
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

#[derive(Debug, Deserialize)]
struct AgentProbeOutput {
    success: bool,
    latency_ms: Option<i32>,
    status_code: Option<i32>,
    error: Option<String>,
    cert_fingerprint: Option<String>,
    cert_not_after: Option<String>,
}

fn build_agent_probe_task(task_id: &str, s: &ServiceConfig) -> Result<ServerTask> {
    let probe_type = ProbeType::from_str(&s.kind).context("invalid kind")?;
    let timeout_seconds = s.timeout_seconds.min(u32::MAX as u64) as u32;
    let task_type = match &probe_type {
        ProbeType::Http => TaskType::HttpGet,
        ProbeType::Tcp => TaskType::TcpPing,
        ProbeType::Icmp => TaskType::IcmpPing,
    };
    let spec = match &probe_type {
        ProbeType::Http => Spec::HttpGet(HttpGetTask {
            url: s.target.clone(),
            timeout_seconds,
            verify_tls: true,
            headers: HashMap::new(),
        }),
        ProbeType::Tcp => {
            let (host, port) = parse_tcp_target(&s.target)?;
            Spec::TcpPing(TcpPingTask {
                host: host.to_string(),
                port: port as u32,
                timeout_seconds,
            })
        }
        ProbeType::Icmp => Spec::IcmpPing(IcmpPingTask {
            host: s.target.clone(),
            count: 4,
            timeout_seconds,
        }),
    };

    Ok(ServerTask {
        task_id: task_id.to_string(),
        task_type: task_type as i32,
        spec: Some(spec),
    })
}

fn service_probe_from_task_result(
    result: xlstatus_proto_gen::xlstatus::v1::TaskResult,
) -> Result<ServiceProbe> {
    let status = TaskOutcome::try_from(result.status).unwrap_or(TaskOutcome::Unspecified);
    if status != TaskOutcome::Success {
        let message = if !result.error.trim().is_empty() {
            result.error
        } else if !result.stderr.trim().is_empty() {
            result.stderr
        } else {
            format!("agent task status: {status:?}")
        };
        return Ok(failure_probe(message));
    }

    let output: AgentProbeOutput =
        serde_json::from_str(&result.stdout).context("agent probe output is invalid")?;
    Ok(ServiceProbe {
        id: uuid::Uuid::now_v7().to_string(),
        service_id: String::new(),
        success: output.success,
        latency_ms: output.latency_ms,
        status_code: output.status_code,
        error: output.error,
        cert_fingerprint: output.cert_fingerprint,
        cert_not_after: output
            .cert_not_after
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc)),
        checked_at: Utc::now(),
    })
}

fn failure_probe(error: impl Into<String>) -> ServiceProbe {
    ServiceProbe {
        id: uuid::Uuid::now_v7().to_string(),
        service_id: String::new(),
        success: false,
        latency_ms: None,
        status_code: None,
        error: Some(error.into()),
        cert_fingerprint: None,
        cert_not_after: None,
        checked_at: Utc::now(),
    }
}

fn parse_tcp_target(target: &str) -> Result<(&str, u16)> {
    let (host, port) = target
        .rsplit_once(':')
        .context("tcp target must be host:port")?;
    if host.trim().is_empty() {
        anyhow::bail!("tcp host is required");
    }
    let port = port.parse::<u16>().context("invalid port")?;
    Ok((host, port))
}

fn parse_server_ids_json(value: Option<String>) -> Vec<String> {
    value
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn parse_task_ids_json(value: Option<String>) -> Vec<String> {
    value
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn service_state_key(service_id: &str, server_id: Option<&str>) -> String {
    format!("{}:{}", service_id, server_id.unwrap_or("local"))
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
            cover_mode: "local".into(),
            server_ids: Vec::new(),
            exclude_server_ids: Vec::new(),
            notification_group_id: None,
            failure_task_ids: Vec::new(),
            recovery_task_ids: Vec::new(),
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: ServiceConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, "x");
    }
}
