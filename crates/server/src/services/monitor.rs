//! M4 service monitor: periodic HTTP / TCP / ICMP probes persisted
//! into `service_results`. A separate API surface in
//! `api/v1/service_history.rs` reads back from the same table for the
//! dashboard's history view.

use crate::api::v1::services::{
    SERVICE_MAX_INTERVAL_SECONDS, SERVICE_MAX_NAME_BYTES, SERVICE_MAX_SERVER_IDS,
    SERVICE_MAX_TARGETS_PER_PROBE, SERVICE_MAX_TARGET_BYTES, SERVICE_MAX_TASK_IDS,
    SERVICE_MAX_TIMEOUT_SECONDS, SERVICE_MIN_INTERVAL_SECONDS, SERVICE_MIN_TIMEOUT_SECONDS,
};
use crate::db::Db;
use crate::grpc::{ensure_task_result_text_within, SessionRegistry, TaskResponseRegistry};
use crate::services::probe::{probe_http, probe_icmp, probe_tcp, ProbeType, ServiceProbe};
use crate::tasks::spawn_triggered_tasks;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{error, info, warn};
use xlstatus_proto_gen::xlstatus::v1::{
    server_task::Spec, HttpGetTask, IcmpPingTask, ServerTask, TaskOutcome, TaskType, TcpPingTask,
};
use xlstatus_shared::AgentId;

const SERVICE_AGENT_PROBE_STDOUT_MAX_BYTES: usize = 16 * 1024;
const SERVICE_AGENT_PROBE_ERROR_MAX_BYTES: usize = 4096;
const SERVICE_UUID_TEXT_LEN: usize = 36;
const LOAD_SERVICES_SQLITE: &str = "SELECT id, owner_user_id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, notification_group_id, COALESCE(cover_mode, 'local'), exclude_server_ids_json, failure_task_ids_json, recovery_task_ids_json FROM services WHERE enabled = 1";
const LOAD_SERVICES_POSTGRES: &str = "SELECT id::text, owner_user_id::text, name, type, target, interval_seconds, timeout_seconds, enabled, server_id::text, notification_group_id::text, COALESCE(cover_mode, 'local'), exclude_server_ids_json, failure_task_ids_json, recovery_task_ids_json FROM services WHERE enabled = TRUE";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub id: String,
    pub owner_user_id: Option<String>,
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

struct LoadedServiceRow {
    id: String,
    owner_user_id: Option<String>,
    name: String,
    kind: String,
    target: String,
    interval_seconds: i64,
    timeout_seconds: i64,
    enabled: bool,
    server_id: Option<String>,
    notification_group_id: Option<String>,
    cover_mode: String,
    exclude_server_ids_json: Option<String>,
    failure_task_ids_json: Option<String>,
    recovery_task_ids_json: Option<String>,
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
                let interval_seconds = bounded_interval_seconds(s.interval_seconds);
                let next = now + chrono::Duration::seconds(interval_seconds as i64);
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
                let server_ids = self.covered_agent_ids(s).await?;
                ensure_probe_target_count(server_ids.len())?;
                for server_id in server_ids {
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
            "specific" => {
                let server_ids = self.specific_agent_ids(s).await?;
                ensure_probe_target_count(server_ids.len())?;
                for server_id in server_ids {
                    let result = match self.probe_via_agent(s, &server_id).await {
                        Ok(result) => result,
                        Err(e) => failure_probe(format!("agent probe failed: {e}")),
                    };
                    self.save(s, Some(&server_id), &result).await?;
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
        let Some(owner_user_id) = trusted_service_owner_uuid_from_config(s) else {
            warn!(
                "service {} {} probe skipped: missing or invalid owner",
                s.id, s.cover_mode
            );
            return Ok(Vec::new());
        };
        let all = self.load_agent_ids_for_owner(owner_user_id).await?;
        if s.cover_mode == "exclude" {
            let excluded = normalize_uuid_id_list(&s.exclude_server_ids);
            Ok(all
                .into_iter()
                .filter(|id| !excluded.iter().any(|excluded| excluded == id))
                .collect())
        } else {
            Ok(all)
        }
    }

    async fn specific_agent_ids(&self, s: &ServiceConfig) -> Result<Vec<String>> {
        let Some(owner_user_id) = self.service_probe_owner(s).await? else {
            warn!(
                "service {} specific probe skipped: missing trusted owner",
                s.id
            );
            return Ok(Vec::new());
        };
        let requested = normalize_uuid_id_list(&s.server_ids);
        if requested.is_empty() {
            return Ok(Vec::new());
        }
        let owned = self.load_agent_ids_for_owner(owner_user_id).await?;
        Ok(requested
            .into_iter()
            .filter(|id| owned.iter().any(|owned_id| owned_id == id))
            .collect())
    }

    async fn service_probe_owner(&self, service: &ServiceConfig) -> Result<Option<uuid::Uuid>> {
        if let Some(owner_user_id) = trusted_service_owner_from_config(service) {
            return match uuid::Uuid::parse_str(&owner_user_id) {
                Ok(owner_user_id) => Ok(Some(owner_user_id)),
                Err(_) => Ok(None),
            };
        }
        Ok(self
            .unique_owner_from_service_servers(service)
            .await?
            .as_deref()
            .and_then(|owner_user_id| uuid::Uuid::parse_str(owner_user_id).ok()))
    }

    async fn load_agent_ids_for_owner(&self, owner_user_id: uuid::Uuid) -> Result<Vec<String>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT id FROM agents WHERE owner_user_id = ? AND revoked_at IS NULL ORDER BY created_at ASC, id ASC",
                )
                .bind(owner_user_id.to_string())
                .fetch_all(pool)
                .await?;
                Ok(normalize_uuid_id_list(
                    &rows.into_iter().map(|(id,)| id).collect::<Vec<_>>(),
                ))
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT id::text FROM agents WHERE owner_user_id = $1 AND revoked_at IS NULL ORDER BY created_at ASC, id ASC",
                )
                .bind(owner_user_id)
                .fetch_all(pool)
                .await?;
                Ok(normalize_uuid_id_list(
                    &rows.into_iter().map(|(id,)| id).collect::<Vec<_>>(),
                ))
            }
        }
    }

    async fn probe_locally(&self, s: &ServiceConfig) -> Result<ServiceProbe> {
        let probe_type = ProbeType::from_str(&s.kind).context("invalid kind")?;
        let timeout_seconds = bounded_timeout_seconds(s.timeout_seconds);
        Ok(match probe_type {
            ProbeType::Http => probe_http(&s.target, timeout_seconds).await?,
            ProbeType::Tcp => {
                let (host, port) = parse_tcp_target(&s.target)?;
                probe_tcp(host, port, timeout_seconds).await?
            }
            ProbeType::Icmp => probe_icmp(&s.target, timeout_seconds).await?,
        })
    }

    async fn probe_via_agent(&self, s: &ServiceConfig, server_id: &str) -> Result<ServiceProbe> {
        let agent_id = AgentId(uuid::Uuid::parse_str(server_id).context("invalid server_id")?);
        if !self.session_registry.is_online(&agent_id).await {
            anyhow::bail!("agent offline");
        }

        let task_id = uuid::Uuid::now_v7().to_string();
        let task = build_agent_probe_task(&task_id, s)?;
        let rx = self
            .response_registry
            .register(task_id.clone(), agent_id)
            .await;
        if let Err(e) = self
            .session_registry
            .send_server_task(&agent_id, task)
            .await
        {
            self.response_registry.cancel(&task_id).await;
            anyhow::bail!(e);
        }

        let wait_seconds = bounded_timeout_seconds(s.timeout_seconds)
            .saturating_add(5)
            .max(5);
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
                    Option<String>,
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
                )> = sqlx::query_as(LOAD_SERVICES_SQLITE).fetch_all(pool).await?;
                let mut services = rows
                    .into_iter()
                    .filter_map(
                        |(
                            id,
                            owner_user_id,
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
                            let row = LoadedServiceRow {
                                id,
                                owner_user_id,
                                name,
                                kind,
                                target,
                                interval_seconds,
                                timeout_seconds,
                                enabled: enabled != 0,
                                server_id,
                                notification_group_id,
                                cover_mode,
                                exclude_server_ids_json,
                                failure_task_ids_json,
                                recovery_task_ids_json,
                            };
                            match validate_loaded_service_config(row) {
                                Ok(service) => Some(service),
                                Err(err) => {
                                    warn!("historical service monitor row skipped: {err}");
                                    None
                                }
                            }
                        },
                    )
                    .collect::<Vec<_>>();
                self.attach_service_server_ids(&mut services).await?;
                retain_valid_loaded_services(&mut services);
                Ok(services)
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(
                    String,
                    Option<String>,
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
                )> = sqlx::query_as(LOAD_SERVICES_POSTGRES)
                    .fetch_all(pool)
                    .await?;
                let mut services = rows
                    .into_iter()
                    .filter_map(
                        |(
                            id,
                            owner_user_id,
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
                            let row = LoadedServiceRow {
                                id,
                                owner_user_id,
                                name,
                                kind,
                                target,
                                interval_seconds: i64::from(interval_seconds),
                                timeout_seconds: i64::from(timeout_seconds),
                                enabled,
                                server_id,
                                notification_group_id,
                                cover_mode,
                                exclude_server_ids_json,
                                failure_task_ids_json,
                                recovery_task_ids_json,
                            };
                            match validate_loaded_service_config(row) {
                                Ok(service) => Some(service),
                                Err(err) => {
                                    warn!("historical service monitor row skipped: {err}");
                                    None
                                }
                            }
                        },
                    )
                    .collect::<Vec<_>>();
                self.attach_service_server_ids(&mut services).await?;
                retain_valid_loaded_services(&mut services);
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
                    "SELECT server_id FROM service_servers WHERE service_id = ? ORDER BY created_at ASC, server_id ASC LIMIT ?",
                )
                .bind(service_id)
                .bind((SERVICE_MAX_SERVER_IDS + 1) as i64)
                .fetch_all(pool)
                .await?;
                Ok(rows.into_iter().map(|(id,)| id).collect())
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let parsed = uuid::Uuid::parse_str(service_id).context("invalid service_id")?;
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT server_id::text FROM service_servers WHERE service_id = $1 ORDER BY created_at ASC, server_id ASC LIMIT $2",
                )
                .bind(parsed)
                .bind((SERVICE_MAX_SERVER_IDS + 1) as i64)
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
        let owner_user_id = match self.service_trigger_owner(service).await {
            Ok(Some(owner_user_id)) => owner_user_id,
            Ok(None) => {
                warn!(
                    "service {} trigger skipped: cannot determine trusted owner",
                    service.id
                );
                return;
            }
            Err(err) => {
                warn!(
                    "service {} trigger owner lookup failed: {}",
                    service.id, err
                );
                return;
            }
        };
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
            Some(owner_user_id),
        );
    }

    async fn service_trigger_owner(&self, service: &ServiceConfig) -> Result<Option<String>> {
        if let Some(owner_user_id) = trusted_service_owner_from_config(service) {
            return Ok(Some(owner_user_id));
        }
        self.unique_owner_from_service_servers(service).await
    }

    async fn unique_owner_from_service_servers(
        &self,
        service: &ServiceConfig,
    ) -> Result<Option<String>> {
        let server_ids = service_effective_server_ids(service);
        if server_ids.is_empty() {
            return Ok(None);
        }
        let owner_ids = self.load_agent_owner_ids(&server_ids).await?;
        if owner_ids.len() != server_ids.len() {
            return Ok(None);
        }
        Ok(unique_nonempty_value(owner_ids))
    }

    async fn load_agent_owner_ids(&self, server_ids: &[String]) -> Result<Vec<String>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let mut owner_ids = Vec::new();
                for server_id in server_ids {
                    let row: Option<(String,)> =
                        sqlx::query_as("SELECT owner_user_id FROM agents WHERE id = ?")
                            .bind(server_id)
                            .fetch_optional(pool)
                            .await?;
                    if let Some((owner_user_id,)) = row {
                        owner_ids.push(owner_user_id);
                    }
                }
                Ok(owner_ids)
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let parsed_ids = server_ids
                    .iter()
                    .map(|id| uuid::Uuid::parse_str(id).context("invalid server_id"))
                    .collect::<Result<Vec<_>>>()?;
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT owner_user_id::text FROM agents WHERE id = ANY($1::uuid[])",
                )
                .bind(parsed_ids)
                .fetch_all(pool)
                .await?;
                Ok(rows.into_iter().map(|(id,)| id).collect())
            }
        }
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
    let timeout_seconds = bounded_timeout_seconds(s.timeout_seconds) as u32;
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

fn bounded_interval_seconds(seconds: u64) -> u64 {
    seconds.clamp(
        SERVICE_MIN_INTERVAL_SECONDS as u64,
        SERVICE_MAX_INTERVAL_SECONDS as u64,
    )
}

fn bounded_timeout_seconds(seconds: u64) -> u64 {
    seconds.clamp(
        SERVICE_MIN_TIMEOUT_SECONDS as u64,
        SERVICE_MAX_TIMEOUT_SECONDS as u64,
    )
}

fn ensure_probe_target_count(count: usize) -> Result<()> {
    if count > SERVICE_MAX_TARGETS_PER_PROBE {
        anyhow::bail!(
            "service probe resolves to {count} target servers; maximum is {SERVICE_MAX_TARGETS_PER_PROBE}"
        );
    }
    Ok(())
}

fn service_probe_from_task_result(
    result: xlstatus_proto_gen::xlstatus::v1::TaskResult,
) -> Result<ServiceProbe> {
    if let Err(message) = ensure_task_result_text_within(
        &result,
        SERVICE_AGENT_PROBE_STDOUT_MAX_BYTES,
        SERVICE_AGENT_PROBE_ERROR_MAX_BYTES,
        SERVICE_AGENT_PROBE_ERROR_MAX_BYTES,
        "agent probe result",
    ) {
        return Ok(failure_probe(message));
    }
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

fn validate_loaded_service_config(row: LoadedServiceRow) -> Result<ServiceConfig> {
    let id = require_canonical_uuid(&row.id, "service_id")?;
    let owner_user_id = row
        .owner_user_id
        .map(|value| require_canonical_uuid(&value, "owner_user_id"))
        .transpose()?;
    let name = row.name.trim().to_string();
    ensure_nonempty_bounded_text(&name, SERVICE_MAX_NAME_BYTES, "name")?;
    let kind = row.kind.trim().to_ascii_lowercase();
    let probe_type = ProbeType::from_str(&kind).context("invalid kind")?;
    let target = row.target.trim().to_string();
    ensure_nonempty_bounded_text(&target, SERVICE_MAX_TARGET_BYTES, "target")?;
    validate_probe_target_shape(&probe_type, &target)?;
    let interval_seconds = checked_runtime_seconds(
        row.interval_seconds,
        SERVICE_MIN_INTERVAL_SECONDS,
        SERVICE_MAX_INTERVAL_SECONDS,
        "interval_seconds",
    )?;
    let timeout_seconds = checked_runtime_seconds(
        row.timeout_seconds,
        SERVICE_MIN_TIMEOUT_SECONDS,
        SERVICE_MAX_TIMEOUT_SECONDS,
        "timeout_seconds",
    )?;
    let cover_mode = normalize_loaded_cover_mode(&row.cover_mode)?;
    let server_ids = match row.server_id {
        Some(server_id) => {
            bounded_normalized_uuid_list(vec![server_id], SERVICE_MAX_SERVER_IDS, "server_id")?
        }
        None => Vec::new(),
    };
    let exclude_server_ids = parse_bounded_normalized_uuid_json_list(
        row.exclude_server_ids_json,
        SERVICE_MAX_SERVER_IDS,
        "exclude_server_ids_json",
    )?;
    let notification_group_id = row
        .notification_group_id
        .map(|value| require_canonical_uuid(&value, "notification_group_id"))
        .transpose()?;
    let failure_task_ids = parse_bounded_canonical_uuid_json_list(
        row.failure_task_ids_json,
        SERVICE_MAX_TASK_IDS,
        "failure_task_ids_json",
    )?;
    let recovery_task_ids = parse_bounded_canonical_uuid_json_list(
        row.recovery_task_ids_json,
        SERVICE_MAX_TASK_IDS,
        "recovery_task_ids_json",
    )?;

    Ok(ServiceConfig {
        id,
        owner_user_id,
        name,
        kind,
        target,
        interval_seconds,
        timeout_seconds,
        enabled: row.enabled,
        cover_mode,
        server_ids,
        exclude_server_ids,
        notification_group_id,
        failure_task_ids,
        recovery_task_ids,
    })
}

fn retain_valid_loaded_services(services: &mut Vec<ServiceConfig>) {
    services.retain_mut(
        |service| match validate_loaded_service_runtime_lists(service) {
            Ok(()) => true,
            Err(err) => {
                warn!(
                    "historical service monitor row {} skipped after relation load: {err}",
                    service.id
                );
                false
            }
        },
    );
}

fn validate_loaded_service_runtime_lists(service: &mut ServiceConfig) -> Result<()> {
    service.server_ids = bounded_normalized_uuid_list(
        std::mem::take(&mut service.server_ids),
        SERVICE_MAX_SERVER_IDS,
        "server_ids",
    )?;
    service.exclude_server_ids = bounded_normalized_uuid_list(
        std::mem::take(&mut service.exclude_server_ids),
        SERVICE_MAX_SERVER_IDS,
        "exclude_server_ids",
    )?;
    service.failure_task_ids = bounded_canonical_uuid_list(
        std::mem::take(&mut service.failure_task_ids),
        SERVICE_MAX_TASK_IDS,
        "failure_task_ids",
    )?;
    service.recovery_task_ids = bounded_canonical_uuid_list(
        std::mem::take(&mut service.recovery_task_ids),
        SERVICE_MAX_TASK_IDS,
        "recovery_task_ids",
    )?;
    Ok(())
}

fn checked_runtime_seconds(value: i64, min: i32, max: i32, field: &str) -> Result<u64> {
    if value < i64::from(min) || value > i64::from(max) {
        anyhow::bail!("{field} must be between {min} and {max}");
    }
    Ok(value as u64)
}

fn ensure_nonempty_bounded_text(value: &str, max_bytes: usize, field: &str) -> Result<()> {
    if value.is_empty() {
        anyhow::bail!("{field} is required");
    }
    if value.len() > max_bytes {
        anyhow::bail!("{field} must be at most {max_bytes} bytes");
    }
    Ok(())
}

fn validate_probe_target_shape(probe_type: &ProbeType, target: &str) -> Result<()> {
    match probe_type {
        ProbeType::Http => {
            let parsed = reqwest::Url::parse(target).context("HTTP monitor URL is invalid")?;
            match parsed.scheme() {
                "http" | "https" => {}
                scheme => anyhow::bail!("HTTP monitor URL scheme '{scheme}' is not allowed"),
            }
            if !parsed.username().is_empty() || parsed.password().is_some() {
                anyhow::bail!("HTTP monitor URL must not include credentials");
            }
            parsed
                .host_str()
                .context("HTTP monitor URL must include a host")?;
            parsed
                .port_or_known_default()
                .context("HTTP monitor URL must include a port or known scheme")?;
            Ok(())
        }
        ProbeType::Tcp => parse_tcp_target(target).map(|_| ()),
        ProbeType::Icmp => Ok(()),
    }
}

fn normalize_loaded_cover_mode(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "local" | "all" | "specific" | "exclude" => Ok(value),
        _ => anyhow::bail!("cover_mode must be local, all, specific, or exclude"),
    }
}

fn parse_bounded_normalized_uuid_json_list(
    value: Option<String>,
    max_len: usize,
    field: &str,
) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let values = serde_json::from_str::<Vec<String>>(&value)
        .with_context(|| format!("{field} must be a JSON string array"))?;
    bounded_normalized_uuid_list(values, max_len, field)
}

fn parse_bounded_canonical_uuid_json_list(
    value: Option<String>,
    max_len: usize,
    field: &str,
) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let values = serde_json::from_str::<Vec<String>>(&value)
        .with_context(|| format!("{field} must be a JSON string array"))?;
    bounded_canonical_uuid_list(values, max_len, field)
}

fn bounded_normalized_uuid_list(
    values: Vec<String>,
    max_len: usize,
    field: &str,
) -> Result<Vec<String>> {
    if values.len() > max_len {
        anyhow::bail!("{field} must contain at most {max_len} entries");
    }
    let mut out = Vec::new();
    for value in values {
        let id = canonical_uuid(&value, field)?;
        if !out.iter().any(|existing| existing == &id) {
            out.push(id);
        }
    }
    Ok(out)
}

fn bounded_canonical_uuid_list(
    values: Vec<String>,
    max_len: usize,
    field: &str,
) -> Result<Vec<String>> {
    if values.len() > max_len {
        anyhow::bail!("{field} must contain at most {max_len} entries");
    }
    let mut out = Vec::new();
    for value in values {
        let id = require_canonical_uuid(&value, field)?;
        if !out.iter().any(|existing| existing == &id) {
            out.push(id);
        }
    }
    Ok(out)
}

fn require_canonical_uuid(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.len() != SERVICE_UUID_TEXT_LEN {
        anyhow::bail!("{field} must be a canonical UUID");
    }
    let parsed = uuid::Uuid::parse_str(trimmed).context("invalid UUID")?;
    if parsed.to_string() != trimmed {
        anyhow::bail!("{field} must be a canonical UUID");
    }
    Ok(trimmed.to_string())
}

fn canonical_uuid(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{field} contains an empty UUID");
    }
    let parsed = uuid::Uuid::parse_str(trimmed).context("invalid UUID")?;
    Ok(parsed.to_string())
}

fn trusted_service_owner_from_config(service: &ServiceConfig) -> Option<String> {
    service
        .owner_user_id
        .as_deref()
        .map(str::trim)
        .filter(|owner| !owner.is_empty())
        .map(str::to_string)
}

fn trusted_service_owner_uuid_from_config(service: &ServiceConfig) -> Option<uuid::Uuid> {
    trusted_service_owner_from_config(service)
        .as_deref()
        .and_then(|owner| uuid::Uuid::parse_str(owner).ok())
}

fn service_effective_server_ids(service: &ServiceConfig) -> Vec<String> {
    if service.cover_mode != "specific" {
        return Vec::new();
    }
    let mut server_ids = Vec::new();
    for server_id in &service.server_ids {
        let trimmed = server_id.trim();
        if !trimmed.is_empty() && !server_ids.iter().any(|existing| existing == trimmed) {
            server_ids.push(trimmed.to_string());
        }
    }
    server_ids
}

fn normalize_uuid_id_list(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let Ok(id) = uuid::Uuid::parse_str(value.trim()) else {
            continue;
        };
        let id = id.to_string();
        if !out.iter().any(|existing| existing == &id) {
            out.push(id);
        }
    }
    out
}

fn unique_nonempty_value(values: Vec<String>) -> Option<String> {
    let values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();
    if values.len() == 1 {
        values.into_iter().next()
    } else {
        None
    }
}

fn service_state_key(service_id: &str, server_id: Option<&str>) -> String {
    format!("{}:{}", service_id, server_id.unwrap_or("local"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DatabaseBackend;
    use crate::grpc::{SessionRegistry, TaskResponseRegistry};

    #[test]
    fn config_round_trip() {
        let cfg = ServiceConfig {
            id: "x".into(),
            owner_user_id: Some("owner".into()),
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
        assert_eq!(back.owner_user_id.as_deref(), Some("owner"));
    }

    #[test]
    fn postgres_load_services_query_uses_boolean_literal() {
        assert!(LOAD_SERVICES_SQLITE.contains("WHERE enabled = 1"));
        assert!(LOAD_SERVICES_POSTGRES.contains("WHERE enabled = TRUE"));
        assert!(!LOAD_SERVICES_POSTGRES.contains("enabled = 1"));
    }

    #[test]
    fn trusted_owner_uses_service_owner_only_when_present() {
        let mut service = test_service_config();
        assert_eq!(trusted_service_owner_from_config(&service), None);

        service.owner_user_id = Some(" owner-1 ".into());
        assert_eq!(
            trusted_service_owner_from_config(&service).as_deref(),
            Some("owner-1")
        );

        service.owner_user_id = Some(" ".into());
        assert_eq!(trusted_service_owner_from_config(&service), None);
    }

    #[test]
    fn service_owner_fallback_only_uses_specific_server_scope() {
        let mut service = test_service_config();
        service.cover_mode = "specific".into();
        service.server_ids = vec!["srv-a".into(), "srv-a".into(), " srv-b ".into()];
        assert_eq!(
            service_effective_server_ids(&service),
            vec!["srv-a".to_string(), "srv-b".to_string()]
        );

        service.cover_mode = "exclude".into();
        assert!(service_effective_server_ids(&service).is_empty());
    }

    #[test]
    fn unique_owner_requires_exactly_one_nonempty_value() {
        assert_eq!(
            unique_nonempty_value(vec![" owner ".into(), "owner".into()]).as_deref(),
            Some("owner")
        );
        assert_eq!(
            unique_nonempty_value(vec!["owner".into(), "other".into()]),
            None
        );
        assert_eq!(unique_nonempty_value(vec![" ".into()]), None);
    }

    #[test]
    fn normalizes_uuid_id_lists() {
        assert_eq!(
            normalize_uuid_id_list(&[
                "00000000-0000-0000-0000-000000000101".into(),
                "00000000000000000000000000000101".into(),
                "not-a-uuid".into(),
            ]),
            vec!["00000000-0000-0000-0000-000000000101".to_string()]
        );
    }

    #[test]
    fn loaded_service_config_rejects_invalid_historical_fields() {
        let valid = test_loaded_service_row();
        let service = validate_loaded_service_config(valid).unwrap();
        assert_eq!(service.kind, "http");
        assert_eq!(service.cover_mode, "exclude");
        assert_eq!(
            service.exclude_server_ids,
            vec!["00000000-0000-0000-0000-000000000101".to_string()]
        );

        let mut malformed_exclude = test_loaded_service_row();
        malformed_exclude.exclude_server_ids_json = Some("not json".into());
        assert!(validate_loaded_service_config(malformed_exclude).is_err());

        let mut negative_interval = test_loaded_service_row();
        negative_interval.interval_seconds = -1;
        assert!(validate_loaded_service_config(negative_interval).is_err());

        let mut unknown_cover = test_loaded_service_row();
        unknown_cover.cover_mode = "remote".into();
        assert!(validate_loaded_service_config(unknown_cover).is_err());

        let mut simple_task_id = test_loaded_service_row();
        simple_task_id.failure_task_ids_json =
            Some(r#"["00000000000000000000000000000401"]"#.into());
        assert!(validate_loaded_service_config(simple_task_id).is_err());
    }

    #[test]
    fn service_monitor_bounds_timeout_interval_and_target_count() {
        assert_eq!(
            bounded_interval_seconds(1),
            SERVICE_MIN_INTERVAL_SECONDS as u64
        );
        assert_eq!(
            bounded_interval_seconds(u64::MAX),
            SERVICE_MAX_INTERVAL_SECONDS as u64
        );
        assert_eq!(
            bounded_timeout_seconds(0),
            SERVICE_MIN_TIMEOUT_SECONDS as u64
        );
        assert_eq!(
            bounded_timeout_seconds(u64::MAX),
            SERVICE_MAX_TIMEOUT_SECONDS as u64
        );
        assert!(ensure_probe_target_count(SERVICE_MAX_TARGETS_PER_PROBE).is_ok());
        assert!(ensure_probe_target_count(SERVICE_MAX_TARGETS_PER_PROBE + 1).is_err());
        assert_eq!(SERVICE_AGENT_PROBE_STDOUT_MAX_BYTES, 16 * 1024);
        assert_eq!(SERVICE_AGENT_PROBE_ERROR_MAX_BYTES, 4096);
    }

    #[test]
    fn service_agent_probe_rejects_oversized_task_result_text() {
        let result = xlstatus_proto_gen::xlstatus::v1::TaskResult {
            status: TaskOutcome::Success as i32,
            exit_code: 0,
            stdout: "x".repeat(SERVICE_AGENT_PROBE_STDOUT_MAX_BYTES + 1),
            ..Default::default()
        };

        let probe = service_probe_from_task_result(result).unwrap();
        assert!(!probe.success);
        assert!(probe
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("stdout exceeds"));
    }

    #[test]
    fn agent_probe_task_uses_bounded_timeout() {
        let mut service = test_service_config();
        service.cover_mode = "specific".into();
        service.kind = "tcp".into();
        service.target = "example.com:443".into();
        service.timeout_seconds = u64::MAX;

        let task = build_agent_probe_task("task", &service).unwrap();
        let Some(Spec::TcpPing(tcp)) = task.spec else {
            panic!("expected tcp ping task");
        };

        assert_eq!(tcp.timeout_seconds, SERVICE_MAX_TIMEOUT_SECONDS as u32);
    }

    #[tokio::test]
    async fn invalid_historical_service_monitor_rows_are_not_loaded() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let service_ok = "00000000-0000-0000-0000-000000000301";
        let service_bad_interval = "00000000-0000-0000-0000-000000000302";
        let service_bad_exclude = "00000000-0000-0000-0000-000000000303";
        let service_bad_cover = "00000000-0000-0000-0000-000000000304";
        let service_too_many_servers = "00000000-0000-0000-0000-000000000305";
        seed_user(&db, owner, "owner").await;
        seed_service(
            &db,
            service_ok,
            Some(owner),
            "http",
            "https://example.com",
            60,
            10,
            "exclude",
            Some(r#"["00000000000000000000000000000101"]"#),
        )
        .await;
        seed_service(
            &db,
            service_bad_interval,
            Some(owner),
            "http",
            "https://example.com",
            -1,
            10,
            "local",
            None,
        )
        .await;
        seed_service(
            &db,
            service_bad_exclude,
            Some(owner),
            "http",
            "https://example.com",
            60,
            10,
            "exclude",
            Some("not json"),
        )
        .await;
        seed_service(
            &db,
            service_bad_cover,
            Some(owner),
            "http",
            "https://example.com",
            60,
            10,
            "remote",
            None,
        )
        .await;
        seed_service(
            &db,
            service_too_many_servers,
            Some(owner),
            "http",
            "https://example.com",
            60,
            10,
            "specific",
            None,
        )
        .await;
        for index in 0..=SERVICE_MAX_SERVER_IDS {
            let server_id = format!("00000000-0000-0000-0000-{:012}", 600 + index);
            seed_agent(&db, &server_id, owner, &format!("srv-{index}")).await;
            seed_service_server(&db, service_too_many_servers, &server_id).await;
        }

        let monitor = ServiceMonitor::new(
            db,
            SessionRegistry::new(),
            Arc::new(TaskResponseRegistry::new()),
        );
        let services = monitor.load_services().await.unwrap();

        assert_eq!(services.len(), 1);
        assert_eq!(services[0].id, service_ok);
        assert_eq!(services[0].cover_mode, "exclude");
        assert_eq!(
            services[0].exclude_server_ids,
            vec!["00000000-0000-0000-0000-000000000101".to_string()]
        );
    }

    #[tokio::test]
    async fn service_trigger_owner_falls_back_to_unique_specific_server_owner() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let own_server = "00000000-0000-0000-0000-000000000101";
        let other_server = "00000000-0000-0000-0000-000000000202";
        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, own_server, owner, "own").await;
        seed_agent(&db, other_server, other, "other").await;

        let monitor = ServiceMonitor::new(
            db,
            SessionRegistry::new(),
            Arc::new(TaskResponseRegistry::new()),
        );

        let mut service = test_service_config();
        service.cover_mode = "specific".into();
        service.server_ids = vec![own_server.into()];
        assert_eq!(
            monitor
                .service_trigger_owner(&service)
                .await
                .unwrap()
                .as_deref(),
            Some(owner)
        );

        service.server_ids = vec![own_server.into(), other_server.into()];
        assert!(monitor
            .service_trigger_owner(&service)
            .await
            .unwrap()
            .is_none());

        service.cover_mode = "local".into();
        service.server_ids = vec![own_server.into()];
        assert!(monitor
            .service_trigger_owner(&service)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn service_monitor_all_cover_mode_uses_service_owner_scope() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let own_server = "00000000-0000-0000-0000-000000000101";
        let other_server = "00000000-0000-0000-0000-000000000202";
        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, own_server, owner, "own").await;
        seed_agent(&db, other_server, other, "other").await;

        let monitor = ServiceMonitor::new(
            db,
            SessionRegistry::new(),
            Arc::new(TaskResponseRegistry::new()),
        );
        let mut service = test_service_config();
        service.cover_mode = "all".into();
        service.owner_user_id = Some(owner.into());

        assert_eq!(
            monitor.covered_agent_ids(&service).await.unwrap(),
            vec![own_server.to_string()]
        );
    }

    #[tokio::test]
    async fn service_monitor_exclude_cover_mode_uses_service_owner_scope() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let own_a = "00000000-0000-0000-0000-000000000101";
        let own_b = "00000000-0000-0000-0000-000000000102";
        let other_server = "00000000-0000-0000-0000-000000000202";
        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, own_a, owner, "own-a").await;
        seed_agent(&db, own_b, owner, "own-b").await;
        seed_agent(&db, other_server, other, "other").await;

        let monitor = ServiceMonitor::new(
            db,
            SessionRegistry::new(),
            Arc::new(TaskResponseRegistry::new()),
        );
        let mut service = test_service_config();
        service.cover_mode = "exclude".into();
        service.owner_user_id = Some(owner.into());
        service.exclude_server_ids = vec![
            "00000000000000000000000000000101".into(),
            other_server.into(),
        ];

        assert_eq!(
            monitor.covered_agent_ids(&service).await.unwrap(),
            vec![own_b.to_string()]
        );
    }

    #[tokio::test]
    async fn service_monitor_all_cover_mode_without_owner_skips_targets() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let server = "00000000-0000-0000-0000-000000000101";
        seed_user(&db, owner, "owner").await;
        seed_agent(&db, server, owner, "server").await;

        let monitor = ServiceMonitor::new(
            db,
            SessionRegistry::new(),
            Arc::new(TaskResponseRegistry::new()),
        );
        let mut service = test_service_config();
        service.cover_mode = "all".into();

        assert!(monitor
            .covered_agent_ids(&service)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn service_monitor_specific_cover_mode_filters_to_owner_scope() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let own_server = "00000000-0000-0000-0000-000000000101";
        let other_server = "00000000-0000-0000-0000-000000000202";
        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, own_server, owner, "own").await;
        seed_agent(&db, other_server, other, "other").await;

        let monitor = ServiceMonitor::new(
            db,
            SessionRegistry::new(),
            Arc::new(TaskResponseRegistry::new()),
        );
        let mut service = test_service_config();
        service.cover_mode = "specific".into();
        service.owner_user_id = Some(owner.into());
        service.server_ids = vec![own_server.into(), other_server.into(), "not-a-uuid".into()];

        assert_eq!(
            monitor.specific_agent_ids(&service).await.unwrap(),
            vec![own_server.to_string()]
        );
    }

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_user(db: &DatabaseBackend, id: &str, username: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'x', 'member', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(username)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_agent(db: &DatabaseBackend, id: &str, owner: &str, name: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at) VALUES (?, ?, 'pk', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(name)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_service(
        db: &DatabaseBackend,
        id: &str,
        owner: Option<&str>,
        kind: &str,
        target: &str,
        interval_seconds: i64,
        timeout_seconds: i64,
        cover_mode: &str,
        exclude_server_ids_json: Option<&str>,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO services (id, owner_user_id, name, type, target, interval_seconds, timeout_seconds, enabled, cover_mode, exclude_server_ids_json, created_at, updated_at) VALUES (?, ?, 'demo', ?, ?, ?, ?, 1, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .bind(kind)
        .bind(target)
        .bind(interval_seconds)
        .bind(timeout_seconds)
        .bind(cover_mode)
        .bind(exclude_server_ids_json)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_service_server(db: &DatabaseBackend, service_id: &str, server_id: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO service_servers (service_id, server_id, created_at) VALUES (?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(service_id)
        .bind(server_id)
        .execute(pool)
        .await
        .unwrap();
    }

    fn test_service_config() -> ServiceConfig {
        ServiceConfig {
            id: "00000000-0000-0000-0000-000000000301".into(),
            owner_user_id: None,
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
        }
    }

    fn test_loaded_service_row() -> LoadedServiceRow {
        LoadedServiceRow {
            id: "00000000-0000-0000-0000-000000000301".into(),
            owner_user_id: Some("00000000-0000-0000-0000-000000000001".into()),
            name: "demo".into(),
            kind: "HTTP".into(),
            target: "https://example.com".into(),
            interval_seconds: 60,
            timeout_seconds: 10,
            enabled: true,
            server_id: Some("00000000000000000000000000000102".into()),
            notification_group_id: Some("00000000-0000-0000-0000-000000000201".into()),
            cover_mode: "EXCLUDE".into(),
            exclude_server_ids_json: Some(r#"["00000000000000000000000000000101"]"#.into()),
            failure_task_ids_json: Some(r#"["00000000-0000-0000-0000-000000000401"]"#.into()),
            recovery_task_ids_json: Some(r#"["00000000-0000-0000-0000-000000000402"]"#.into()),
        }
    }
}
