//! M4 alert engine.
//!
//! See `docs/implementation-audit.md` for the full design notes.

use crate::api::v1::alerts::{
    ALERT_MAX_CONDITIONS, ALERT_MAX_CONDITION_BYTES, ALERT_MAX_CONSECUTIVE_FAILURES,
    ALERT_MAX_DAYS_BEFORE, ALERT_MAX_LATENCY_MS, ALERT_MAX_NAME_BYTES, ALERT_MAX_OFFLINE_SECONDS,
    ALERT_MAX_RESOURCE_DURATION_SECONDS, ALERT_MAX_RESOURCE_THRESHOLD, ALERT_MAX_TASK_IDS,
    ALERT_MAX_TRAFFIC_PERCENT,
};
use crate::db::Db;
use crate::grpc::{SessionRegistry, TaskResponseRegistry};
use crate::notifications::sender::{
    ensure_notification_channel_count_allowed, NotificationChannel, NotificationMessage,
    NotificationSender, NotificationSeverity, NOTIFICATION_MAX_GROUP_CHANNELS,
};
use crate::tasks::spawn_triggered_tasks;
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger_mode: TriggerMode,
    pub conditions: Vec<AlertCondition>,
    pub notification_group_id: Option<String>,
    pub failure_task_ids: Vec<String>,
    pub recovery_task_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    Always,
    Once,
}

impl TriggerMode {
    pub fn from_db(s: &str) -> Self {
        match s {
            "always" => TriggerMode::Always,
            _ => TriggerMode::Once,
        }
    }
    pub fn as_db(self) -> &'static str {
        match self {
            TriggerMode::Always => "always",
            TriggerMode::Once => "once",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AlertCondition {
    ServiceDown {
        service_id: String,
        consecutive_failures: u32,
    },
    ServiceLatency {
        service_id: String,
        max_latency_ms: i32,
    },
    CertificateExpiry {
        service_id: String,
        days_before: i64,
    },
    ServerExpiry {
        agent_id: String,
        days_before: i64,
    },
    ServerTrafficQuota {
        agent_id: String,
        percent: f64,
        #[serde(default)]
        direction: TrafficQuotaDirection,
    },
    ServerOffline {
        agent_id: String,
        offline_seconds: u64,
    },
    ServerResource {
        agent_id: String,
        resource: ResourceType,
        operator: Operator,
        threshold: f64,
        #[serde(default)]
        duration_seconds: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Cpu,
    Memory,
    Disk,
    Network,
    NetworkIn,
    NetworkOut,
    NetworkTotal,
    TrafficInTotal,
    TrafficOutTotal,
    Load,
    Load5,
    Load15,
    Swap,
    Tcp,
    Udp,
    Process,
    Temperature,
    Gpu,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrafficQuotaDirection {
    #[default]
    Total,
    In,
    Out,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    Gt,
    Lt,
    Gte,
    Lte,
}

impl Operator {
    fn compare(self, left: f64, right: f64) -> bool {
        match self {
            Operator::Gt => left > right,
            Operator::Lt => left < right,
            Operator::Gte => left >= right,
            Operator::Lte => left <= right,
        }
    }
}

#[derive(Debug, Clone)]
struct AlertState {
    rule_id: String,
    is_active: bool,
    last_fired_at: Option<chrono::DateTime<chrono::Utc>>,
    last_always_fire_at: Option<Instant>,
}

#[derive(Debug, Clone)]
struct TrafficWindow {
    bytes: u64,
    sampled_at: DateTime<Utc>,
    last_result: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ServiceResultScope {
    Local,
    Servers(Vec<String>),
}

struct LoadedAlertRuleRow {
    id: String,
    owner_user_id: String,
    name: String,
    enabled: bool,
    trigger_mode: String,
    rules_json: String,
    notification_group_id: Option<String>,
    fail_task_ids_json: Option<String>,
    recover_task_ids_json: Option<String>,
}

pub struct AlertEngine {
    db: Db,
    session_registry: SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    states: Arc<RwLock<HashMap<String, AlertState>>>,
    condition_windows: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    traffic_windows: Arc<RwLock<HashMap<String, TrafficWindow>>>,
    sender: Arc<NotificationSender>,
    latest: Arc<RwLock<HashMap<String, serde_json::Value>>>,
}

impl AlertEngine {
    pub fn new(
        db: Db,
        session_registry: SessionRegistry,
        response_registry: Arc<TaskResponseRegistry>,
    ) -> Self {
        Self {
            db,
            session_registry,
            response_registry,
            states: Arc::new(RwLock::new(HashMap::new())),
            condition_windows: Arc::new(RwLock::new(HashMap::new())),
            traffic_windows: Arc::new(RwLock::new(HashMap::new())),
            sender: Arc::new(NotificationSender::new()),
            latest: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn latest_handle(&self) -> Arc<RwLock<HashMap<String, serde_json::Value>>> {
        self.latest.clone()
    }

    pub async fn observe_agent_state(&self, agent_id: &str, payload: serde_json::Value) {
        self.latest
            .write()
            .await
            .insert(agent_id.to_string(), payload);
    }

    pub async fn evaluate_all(&self) -> Result<()> {
        let rules = self.load_alert_rules().await?;
        for rule in rules {
            if !rule.enabled {
                continue;
            }
            if let Err(e) = self.evaluate_rule(&rule).await {
                warn!("rule {} evaluate failed: {}", rule.id, e);
            }
        }
        Ok(())
    }

    async fn evaluate_rule(&self, rule: &AlertRule) -> Result<()> {
        let mut any_triggered = false;
        let mut source_agent_id = None;
        let mut trusted_conditions = Vec::new();
        for c in &rule.conditions {
            if !self.condition_belongs_to_rule_owner(rule, c).await? {
                warn!(
                    "rule {} condition skipped: condition resource is outside rule owner scope",
                    rule.id
                );
                continue;
            }
            trusted_conditions.push(c);
            if self.check_condition(c).await? {
                any_triggered = true;
                source_agent_id = condition_agent_id(c).map(str::to_string);
                break;
            }
        }
        let fallback_source_agent_id =
            source_agent_id.or_else(|| infer_condition_agent_id(trusted_conditions.into_iter()));

        match rule.trigger_mode {
            TriggerMode::Once => {
                let already = self
                    .states
                    .read()
                    .await
                    .get(&rule.id)
                    .map(|s| s.is_active)
                    .unwrap_or(false);
                if any_triggered && !already {
                    self.fire(rule, "fired", fallback_source_agent_id.as_deref())
                        .await?;
                    self.states.write().await.insert(
                        rule.id.clone(),
                        AlertState {
                            rule_id: rule.id.clone(),
                            is_active: true,
                            last_fired_at: Some(Utc::now()),
                            last_always_fire_at: None,
                        },
                    );
                } else if !any_triggered && already {
                    self.fire(rule, "recovered", fallback_source_agent_id.as_deref())
                        .await?;
                    self.states.write().await.insert(
                        rule.id.clone(),
                        AlertState {
                            rule_id: rule.id.clone(),
                            is_active: false,
                            last_fired_at: None,
                            last_always_fire_at: None,
                        },
                    );
                }
            }
            TriggerMode::Always => {
                let already = self
                    .states
                    .read()
                    .await
                    .get(&rule.id)
                    .map(|s| s.is_active)
                    .unwrap_or(false);
                if any_triggered {
                    let throttle_ok = {
                        let states = self.states.read().await;
                        match states.get(&rule.id).and_then(|s| s.last_always_fire_at) {
                            Some(t) => t.elapsed() >= Duration::from_secs(60),
                            None => true,
                        }
                    };
                    if throttle_ok {
                        self.fire(rule, "fired", fallback_source_agent_id.as_deref())
                            .await?;
                        self.states.write().await.insert(
                            rule.id.clone(),
                            AlertState {
                                rule_id: rule.id.clone(),
                                is_active: true,
                                last_fired_at: Some(Utc::now()),
                                last_always_fire_at: Some(Instant::now()),
                            },
                        );
                    }
                } else if already {
                    self.fire(rule, "recovered", fallback_source_agent_id.as_deref())
                        .await?;
                    self.states.write().await.insert(
                        rule.id.clone(),
                        AlertState {
                            rule_id: rule.id.clone(),
                            is_active: false,
                            last_fired_at: None,
                            last_always_fire_at: None,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    async fn condition_belongs_to_rule_owner(
        &self,
        rule: &AlertRule,
        condition: &AlertCondition,
    ) -> Result<bool> {
        match condition {
            AlertCondition::ServiceDown { service_id, .. }
            | AlertCondition::ServiceLatency { service_id, .. }
            | AlertCondition::CertificateExpiry { service_id, .. } => {
                self.service_belongs_to_owner(service_id, &rule.owner_user_id)
                    .await
            }
            AlertCondition::ServerExpiry { agent_id, .. }
            | AlertCondition::ServerTrafficQuota { agent_id, .. }
            | AlertCondition::ServerOffline { agent_id, .. }
            | AlertCondition::ServerResource { agent_id, .. } => {
                self.agent_belongs_to_owner(agent_id, &rule.owner_user_id)
                    .await
            }
        }
    }

    async fn check_condition(&self, c: &AlertCondition) -> Result<bool> {
        match c {
            AlertCondition::ServiceDown {
                service_id,
                consecutive_failures,
            } => {
                let n = *consecutive_failures as i64;
                self.count_recent_service_failures(service_id, n)
                    .await
                    .map(|failures| failures >= n)
            }
            AlertCondition::ServiceLatency {
                service_id,
                max_latency_ms,
            } => self
                .latest_service_latency_ms(service_id)
                .await
                .map(|lat| lat > *max_latency_ms as i64),
            AlertCondition::CertificateExpiry {
                service_id,
                days_before,
            } => self.latest_cert_not_after(service_id).await.map(|value| {
                value
                    .map(|not_after| not_after <= Utc::now() + chrono::Duration::days(*days_before))
                    .unwrap_or(false)
            }),
            AlertCondition::ServerExpiry {
                agent_id,
                days_before,
            } => self.server_expires_at(agent_id).await.map(|value| {
                value
                    .map(|expires_at| {
                        expires_at <= Utc::now() + chrono::Duration::days(*days_before)
                    })
                    .unwrap_or(false)
            }),
            AlertCondition::ServerTrafficQuota {
                agent_id,
                percent,
                direction,
            } => {
                self.check_server_traffic_quota(agent_id, *percent, *direction)
                    .await
            }
            AlertCondition::ServerOffline {
                agent_id,
                offline_seconds,
            } => {
                let since = self
                    .last_seen_age_seconds(agent_id)
                    .await
                    .unwrap_or(i64::MAX);
                Ok(since >= *offline_seconds as i64)
            }
            AlertCondition::ServerResource {
                agent_id,
                resource,
                operator,
                threshold,
                duration_seconds,
            } => {
                self.check_resource_condition(
                    agent_id,
                    *resource,
                    *operator,
                    *threshold,
                    *duration_seconds,
                )
                .await
            }
        }
    }

    async fn check_resource_condition(
        &self,
        agent_id: &str,
        resource: ResourceType,
        operator: Operator,
        threshold: f64,
        duration_seconds: u64,
    ) -> Result<bool> {
        let last = self.latest.read().await.get(agent_id).cloned();
        let Some(state) = last else { return Ok(false) };
        let key =
            format!("resource:{agent_id}:{resource:?}:{operator:?}:{threshold}:{duration_seconds}");
        if resource == ResourceType::Network && duration_seconds > 0 {
            return self
                .check_network_window(&key, &state, operator, threshold, duration_seconds)
                .await;
        }
        let Some(value) = resource_value(&state, resource) else {
            self.condition_windows.write().await.remove(&key);
            return Ok(false);
        };
        let triggered = operator.compare(value, threshold);
        self.apply_duration_window(&key, triggered, duration_seconds)
            .await
    }

    async fn apply_duration_window(
        &self,
        key: &str,
        triggered: bool,
        duration_seconds: u64,
    ) -> Result<bool> {
        if !triggered {
            self.condition_windows.write().await.remove(key);
            return Ok(false);
        }
        if duration_seconds == 0 {
            return Ok(true);
        }
        let now = Utc::now();
        let mut windows = self.condition_windows.write().await;
        let first_seen = windows.entry(key.to_string()).or_insert(now);
        Ok((now - *first_seen).num_seconds() >= duration_seconds as i64)
    }

    async fn check_network_window(
        &self,
        key: &str,
        state: &serde_json::Value,
        operator: Operator,
        threshold: f64,
        duration_seconds: u64,
    ) -> Result<bool> {
        let Some(bytes) = network_total_bytes(state) else {
            self.traffic_windows.write().await.remove(key);
            return Ok(false);
        };
        let now = Utc::now();
        let mut windows = self.traffic_windows.write().await;
        let Some(window) = windows.get_mut(key) else {
            windows.insert(
                key.to_string(),
                TrafficWindow {
                    bytes,
                    sampled_at: now,
                    last_result: false,
                },
            );
            return Ok(false);
        };
        let elapsed = (now - window.sampled_at).num_seconds().max(0) as u64;
        if elapsed < duration_seconds {
            return Ok(window.last_result);
        }

        let delta = bytes.saturating_sub(window.bytes) as f64;
        let triggered = operator.compare(delta, threshold);
        window.bytes = bytes;
        window.sampled_at = now;
        window.last_result = triggered;
        Ok(triggered)
    }

    async fn service_belongs_to_owner(
        &self,
        service_id: &str,
        owner_user_id: &str,
    ) -> Result<bool> {
        let Ok(owner_uuid) = uuid::Uuid::parse_str(owner_user_id) else {
            return Ok(false);
        };
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row: Option<(String,)> =
                    sqlx::query_as("SELECT owner_user_id FROM services WHERE id = ?")
                        .bind(service_id)
                        .fetch_optional(pool)
                        .await?;
                Ok(row
                    .and_then(|(owner,)| uuid::Uuid::parse_str(owner.trim()).ok())
                    .map(|owner| owner == owner_uuid)
                    .unwrap_or(false))
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let Ok(service_uuid) = uuid::Uuid::parse_str(service_id) else {
                    return Ok(false);
                };
                let row: Option<(uuid::Uuid,)> =
                    sqlx::query_as("SELECT owner_user_id FROM services WHERE id = $1")
                        .bind(service_uuid)
                        .fetch_optional(pool)
                        .await?;
                Ok(row.map(|(owner,)| owner == owner_uuid).unwrap_or(false))
            }
        }
    }

    async fn agent_belongs_to_owner(&self, agent_id: &str, owner_user_id: &str) -> Result<bool> {
        let Ok(owner_uuid) = uuid::Uuid::parse_str(owner_user_id) else {
            return Ok(false);
        };
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT owner_user_id FROM agents WHERE id = ? AND revoked_at IS NULL",
                )
                .bind(agent_id)
                .fetch_optional(pool)
                .await?;
                Ok(row
                    .and_then(|(owner,)| uuid::Uuid::parse_str(owner.trim()).ok())
                    .map(|owner| owner == owner_uuid)
                    .unwrap_or(false))
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let Ok(agent_uuid) = uuid::Uuid::parse_str(agent_id) else {
                    return Ok(false);
                };
                let row: Option<(uuid::Uuid,)> = sqlx::query_as(
                    "SELECT owner_user_id FROM agents WHERE id = $1 AND revoked_at IS NULL",
                )
                .bind(agent_uuid)
                .fetch_optional(pool)
                .await?;
                Ok(row.map(|(owner,)| owner == owner_uuid).unwrap_or(false))
            }
        }
    }

    async fn count_recent_service_failures(&self, service_id: &str, n: i64) -> Result<i64> {
        let scope = self.service_result_scope(service_id).await?;
        let ServiceResultScope::Servers(server_ids) = &scope else {
            return self
                .count_recent_local_service_failures(service_id, n)
                .await;
        };
        if server_ids.is_empty() {
            return Ok(0);
        }
        let rows: Vec<(String,)> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let sql = format!(
                    "SELECT status FROM service_results WHERE service_id = ? AND server_id IN ({}) ORDER BY created_at DESC LIMIT ?",
                    sqlite_placeholders(server_ids.len())
                );
                let mut query = sqlx::query_as(&sql).bind(service_id);
                for server_id in server_ids {
                    query = query.bind(server_id);
                }
                query.bind(n).fetch_all(pool).await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                let parsed_server_ids = parse_uuid_ids(server_ids)?;
                sqlx::query_as("SELECT status FROM service_results WHERE service_id = $1 AND server_id = ANY($2::uuid[]) ORDER BY created_at DESC LIMIT $3")
                    .bind(sid)
                    .bind(parsed_server_ids)
                    .bind(n)
                    .fetch_all(pool)
                    .await?
            }
        };
        Ok(rows.iter().filter(|(s,)| s == "failure").count() as i64)
    }

    async fn count_recent_local_service_failures(&self, service_id: &str, n: i64) -> Result<i64> {
        let rows: Vec<(String,)> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query_as("SELECT status FROM service_results WHERE service_id = ? AND server_id IS NULL ORDER BY created_at DESC LIMIT ?")
                    .bind(service_id)
                    .bind(n)
                    .fetch_all(pool)
                    .await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                sqlx::query_as("SELECT status FROM service_results WHERE service_id = $1 AND server_id IS NULL ORDER BY created_at DESC LIMIT $2")
                    .bind(sid)
                    .bind(n)
                    .fetch_all(pool)
                    .await?
            }
        };
        Ok(rows.iter().filter(|(s,)| s == "failure").count() as i64)
    }

    async fn latest_service_latency_ms(&self, service_id: &str) -> Result<i64> {
        let scope = self.service_result_scope(service_id).await?;
        let ServiceResultScope::Servers(server_ids) = &scope else {
            return self.latest_local_service_latency_ms(service_id).await;
        };
        if server_ids.is_empty() {
            return Ok(-1);
        }
        let row: Option<(Option<i64>,)> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let sql = format!(
                    "SELECT delay_ms FROM service_results WHERE service_id = ? AND status = 'success' AND server_id IN ({}) ORDER BY created_at DESC LIMIT 1",
                    sqlite_placeholders(server_ids.len())
                );
                let mut query = sqlx::query_as(&sql).bind(service_id);
                for server_id in server_ids {
                    query = query.bind(server_id);
                }
                query.fetch_optional(pool).await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                let parsed_server_ids = parse_uuid_ids(server_ids)?;
                sqlx::query_as("SELECT delay_ms FROM service_results WHERE service_id = $1 AND status = 'success' AND server_id = ANY($2::uuid[]) ORDER BY created_at DESC LIMIT 1")
                    .bind(sid)
                    .bind(parsed_server_ids)
                    .fetch_optional(pool)
                    .await?
            }
        };
        Ok(row.and_then(|(v,)| v).unwrap_or(-1))
    }

    async fn latest_local_service_latency_ms(&self, service_id: &str) -> Result<i64> {
        let row: Option<(Option<i64>,)> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query_as("SELECT delay_ms FROM service_results WHERE service_id = ? AND status = 'success' AND server_id IS NULL ORDER BY created_at DESC LIMIT 1")
                    .bind(service_id)
                    .fetch_optional(pool)
                    .await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                sqlx::query_as("SELECT delay_ms FROM service_results WHERE service_id = $1 AND status = 'success' AND server_id IS NULL ORDER BY created_at DESC LIMIT 1")
                    .bind(sid)
                    .fetch_optional(pool)
                    .await?
            }
        };
        Ok(row.and_then(|(v,)| v).unwrap_or(-1))
    }

    async fn latest_cert_not_after(&self, service_id: &str) -> Result<Option<DateTime<Utc>>> {
        let scope = self.service_result_scope(service_id).await?;
        let ServiceResultScope::Servers(server_ids) = &scope else {
            return self.latest_local_cert_not_after(service_id).await;
        };
        if server_ids.is_empty() {
            return Ok(None);
        }
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let sql = format!(
                    "SELECT cert_not_after FROM service_results WHERE service_id = ? AND cert_not_after IS NOT NULL AND server_id IN ({}) ORDER BY created_at DESC LIMIT 1",
                    sqlite_placeholders(server_ids.len())
                );
                let mut query = sqlx::query_as(&sql).bind(service_id);
                for server_id in server_ids {
                    query = query.bind(server_id);
                }
                let row: Option<(String,)> = query.fetch_optional(pool).await?;
                row.map(|(value,)| {
                    DateTime::parse_from_rfc3339(&value).map(|dt| dt.with_timezone(&Utc))
                })
                .transpose()
                .map_err(Into::into)
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                let parsed_server_ids = parse_uuid_ids(server_ids)?;
                let row: Option<(DateTime<Utc>,)> = sqlx::query_as(
                    "SELECT cert_not_after FROM service_results WHERE service_id = $1 AND cert_not_after IS NOT NULL AND server_id = ANY($2::uuid[]) ORDER BY created_at DESC LIMIT 1",
                )
                .bind(sid)
                .bind(parsed_server_ids)
                .fetch_optional(pool)
                .await?;
                Ok(row.map(|(value,)| value))
            }
        }
    }

    async fn latest_local_cert_not_after(&self, service_id: &str) -> Result<Option<DateTime<Utc>>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT cert_not_after FROM service_results WHERE service_id = ? AND cert_not_after IS NOT NULL AND server_id IS NULL ORDER BY created_at DESC LIMIT 1",
                )
                .bind(service_id)
                .fetch_optional(pool)
                .await?;
                row.map(|(value,)| {
                    DateTime::parse_from_rfc3339(&value).map(|dt| dt.with_timezone(&Utc))
                })
                .transpose()
                .map_err(Into::into)
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                let row: Option<(DateTime<Utc>,)> = sqlx::query_as(
                    "SELECT cert_not_after FROM service_results WHERE service_id = $1 AND cert_not_after IS NOT NULL AND server_id IS NULL ORDER BY created_at DESC LIMIT 1",
                )
                .bind(sid)
                .fetch_optional(pool)
                .await?;
                Ok(row.map(|(value,)| value))
            }
        }
    }

    async fn service_result_scope(&self, service_id: &str) -> Result<ServiceResultScope> {
        let Some((cover_mode, owner_user_id, legacy_server_id, exclude_server_ids)) =
            self.load_service_scope_config(service_id).await?
        else {
            return Ok(ServiceResultScope::Servers(Vec::new()));
        };
        match cover_mode.as_str() {
            "specific" => {
                let mut server_ids = self.load_service_server_ids(service_id).await?;
                if server_ids.is_empty() {
                    if let Some(server_id) = legacy_server_id {
                        server_ids.push(server_id);
                    }
                }
                Ok(ServiceResultScope::Servers(
                    self.filter_agent_ids_to_service_owner(
                        service_id,
                        owner_user_id.as_deref(),
                        server_ids,
                    )
                    .await?,
                ))
            }
            "all" => Ok(ServiceResultScope::Servers(
                self.load_active_agent_ids_for_owner(service_id, owner_user_id.as_deref())
                    .await?,
            )),
            "exclude" => {
                let excluded = normalize_uuid_id_list(exclude_server_ids);
                let mut server_ids = self
                    .load_active_agent_ids_for_owner(service_id, owner_user_id.as_deref())
                    .await?;
                server_ids.retain(|server_id| !excluded.iter().any(|id| id == server_id));
                Ok(ServiceResultScope::Servers(server_ids))
            }
            _ => Ok(ServiceResultScope::Local),
        }
    }

    async fn filter_agent_ids_to_service_owner(
        &self,
        service_id: &str,
        owner_user_id: Option<&str>,
        server_ids: Vec<String>,
    ) -> Result<Vec<String>> {
        let server_ids = normalize_uuid_id_list(server_ids);
        if server_ids.is_empty() {
            return Ok(Vec::new());
        }
        let Some(owner_user_id) = self
            .service_scope_owner(service_id, owner_user_id, &server_ids)
            .await?
        else {
            return Ok(Vec::new());
        };
        let owned = self
            .load_active_agent_ids_for_owner_uuid(owner_user_id)
            .await?;
        Ok(server_ids
            .into_iter()
            .filter(|server_id| owned.iter().any(|owned_id| owned_id == server_id))
            .collect())
    }

    async fn service_scope_owner(
        &self,
        service_id: &str,
        owner_user_id: Option<&str>,
        server_ids: &[String],
    ) -> Result<Option<uuid::Uuid>> {
        if let Some(owner_user_id) = owner_user_id {
            return match uuid::Uuid::parse_str(owner_user_id) {
                Ok(owner_user_id) => Ok(Some(owner_user_id)),
                Err(_) => {
                    warn!("service {service_id} result scope skipped: invalid owner");
                    Ok(None)
                }
            };
        }

        let owner_ids = self.load_agent_owner_ids(server_ids).await?;
        if owner_ids.len() != server_ids.len() {
            warn!("service {service_id} result scope skipped: missing owner");
            return Ok(None);
        }
        Ok(unique_uuid_value(owner_ids))
    }

    async fn load_service_scope_config(
        &self,
        service_id: &str,
    ) -> Result<Option<(String, Option<String>, Option<String>, Vec<String>)>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row: Option<(String, Option<String>, Option<String>, Option<String>)> =
                    sqlx::query_as(
                        "SELECT COALESCE(cover_mode, 'local'), owner_user_id, server_id, exclude_server_ids_json FROM services WHERE id = ?",
                )
                .bind(service_id)
                .fetch_optional(pool)
                .await?;
                Ok(
                    row.map(|(cover_mode, owner_user_id, server_id, exclude_json)| {
                        (
                            cover_mode,
                            owner_user_id
                                .map(|value| value.trim().to_string())
                                .filter(|value| !value.is_empty()),
                            server_id
                                .map(|value| value.trim().to_string())
                                .filter(|value| !value.is_empty()),
                            parse_id_list_json(exclude_json),
                        )
                    }),
                )
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                let row: Option<(String, Option<String>, Option<String>, Option<String>)> =
                    sqlx::query_as(
                        "SELECT COALESCE(cover_mode, 'local'), owner_user_id::text, server_id::text, exclude_server_ids_json FROM services WHERE id = $1",
                )
                .bind(sid)
                .fetch_optional(pool)
                .await?;
                Ok(
                    row.map(|(cover_mode, owner_user_id, server_id, exclude_json)| {
                        (
                            cover_mode,
                            owner_user_id
                                .map(|value| value.trim().to_string())
                                .filter(|value| !value.is_empty()),
                            server_id
                                .map(|value| value.trim().to_string())
                                .filter(|value| !value.is_empty()),
                            parse_id_list_json(exclude_json),
                        )
                    }),
                )
            }
        }
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
                Ok(normalize_id_list(
                    rows.into_iter().map(|(id,)| id).collect(),
                ))
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT server_id::text FROM service_servers WHERE service_id = $1 ORDER BY created_at ASC, server_id ASC",
                )
                .bind(sid)
                .fetch_all(pool)
                .await?;
                Ok(normalize_id_list(
                    rows.into_iter().map(|(id,)| id).collect(),
                ))
            }
        }
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
                let server_ids = parse_uuid_ids(server_ids)?;
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT owner_user_id::text FROM agents WHERE id = ANY($1::uuid[])",
                )
                .bind(server_ids)
                .fetch_all(pool)
                .await?;
                Ok(rows.into_iter().map(|(owner_id,)| owner_id).collect())
            }
        }
    }

    async fn load_active_agent_ids_for_owner(
        &self,
        service_id: &str,
        owner_user_id: Option<&str>,
    ) -> Result<Vec<String>> {
        let Some(owner_user_id) = owner_user_id else {
            warn!("service {service_id} result scope skipped: missing owner");
            return Ok(Vec::new());
        };
        let Ok(owner_user_id) = uuid::Uuid::parse_str(owner_user_id) else {
            warn!("service {service_id} result scope skipped: invalid owner");
            return Ok(Vec::new());
        };
        self.load_active_agent_ids_for_owner_uuid(owner_user_id)
            .await
    }

    async fn load_active_agent_ids_for_owner_uuid(
        &self,
        owner_user_id: uuid::Uuid,
    ) -> Result<Vec<String>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows: Vec<(String,)> = sqlx::query_as(
                    "SELECT id FROM agents WHERE owner_user_id = ? AND revoked_at IS NULL ORDER BY created_at ASC, id ASC",
                )
                .bind(owner_user_id.to_string())
                .fetch_all(pool)
                .await?;
                Ok(normalize_uuid_id_list(
                    rows.into_iter().map(|(id,)| id).collect(),
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
                    rows.into_iter().map(|(id,)| id).collect(),
                ))
            }
        }
    }

    async fn last_seen_age_seconds(&self, agent_id: &str) -> Result<i64> {
        let row: Option<(Option<String>,)> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query_as("SELECT last_seen_at FROM agents WHERE id = ?")
                    .bind(agent_id)
                    .fetch_optional(pool)
                    .await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query_as("SELECT last_seen_at::text FROM agents WHERE id = $1")
                    .bind(agent_id)
                    .fetch_optional(pool)
                    .await?
            }
        };
        let Some((Some(ts),)) = row else {
            return Ok(i64::MAX);
        };
        let parsed = DateTime::parse_from_rfc3339(&ts)?;
        Ok((Utc::now() - parsed.with_timezone(&Utc)).num_seconds())
    }

    async fn server_expires_at(&self, agent_id: &str) -> Result<Option<DateTime<Utc>>> {
        let row: Option<(Option<String>,)> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query_as("SELECT expires_at FROM agents WHERE id = ?")
                    .bind(agent_id)
                    .fetch_optional(pool)
                    .await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query_as("SELECT expires_at FROM agents WHERE id = $1")
                    .bind(agent_id)
                    .fetch_optional(pool)
                    .await?
            }
        };
        Ok(row
            .and_then(|(value,)| value)
            .and_then(|value| parse_server_expiry(&value)))
    }

    async fn check_server_traffic_quota(
        &self,
        agent_id: &str,
        percent: f64,
        direction: TrafficQuotaDirection,
    ) -> Result<bool> {
        if !percent.is_finite() || percent <= 0.0 {
            return Ok(false);
        }
        let Some(quota) = self.server_traffic_quota_bytes(agent_id).await? else {
            return Ok(false);
        };
        let last = self.latest.read().await.get(agent_id).cloned();
        let Some(state) = last else { return Ok(false) };
        let Some(current_percent) = traffic_quota_percent(&state, quota, direction) else {
            return Ok(false);
        };
        Ok(current_percent >= percent)
    }

    async fn server_traffic_quota_bytes(&self, agent_id: &str) -> Result<Option<u64>> {
        let row: Option<(Option<String>,)> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query_as("SELECT dashboard_metadata_json FROM agents WHERE id = ?")
                    .bind(agent_id)
                    .fetch_optional(pool)
                    .await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query_as("SELECT dashboard_metadata_json FROM agents WHERE id = $1")
                    .bind(agent_id)
                    .fetch_optional(pool)
                    .await?
            }
        };
        Ok(row
            .and_then(|(value,)| value)
            .as_deref()
            .and_then(metadata_traffic_quota_bytes))
    }

    async fn fire(
        &self,
        rule: &AlertRule,
        kind: &str,
        source_agent_id: Option<&str>,
    ) -> Result<()> {
        let id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now();
        let payload = serde_json::json!({
            "rule_id": rule.id,
            "rule_name": rule.name,
            "kind": kind,
            "agent_id": source_agent_id,
            "fired_at": now.to_rfc3339(),
        });
        let payload_text = serde_json::to_string(&payload)?;
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO alert_events (id, rule_id, agent_id, kind, payload_json, fired_at) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&rule.id)
                .bind(source_agent_id)
                .bind(kind)
                .bind(&payload_text)
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let pid = uuid::Uuid::parse_str(&id)?;
                let prid = uuid::Uuid::parse_str(&rule.id)?;
                let agent_id = source_agent_id
                    .map(uuid::Uuid::parse_str)
                    .transpose()
                    .context("invalid alert source agent id")?;
                sqlx::query(
                    "INSERT INTO alert_events (id, rule_id, agent_id, kind, payload_json, fired_at) VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .bind(pid)
                .bind(prid)
                .bind(agent_id)
                .bind(kind)
                .bind(&payload_text)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }
        info!("alert {} event for rule {} ({})", kind, rule.id, rule.name);
        self.trigger_tasks(rule, kind, source_agent_id);

        let channels = self.channels_for_rule(rule).await.unwrap_or_default();
        if channels.is_empty() {
            return Ok(());
        }
        let message = NotificationMessage {
            title: format!("[{}] {}", kind, rule.name),
            message: format!("Alert rule {} fired: {}", rule.name, kind),
            severity: if kind == "fired" {
                NotificationSeverity::Error
            } else {
                NotificationSeverity::Info
            },
            timestamp: now.to_rfc3339(),
            metadata: HashMap::new(),
        };
        for ch in channels {
            let s = self.sender.clone();
            let m = message.clone();
            tokio::spawn(async move {
                if let Err(e) = s.send(&ch, &m).await {
                    warn!("notification send failed: {}", e);
                }
            });
        }
        Ok(())
    }

    fn trigger_tasks(&self, rule: &AlertRule, kind: &str, source_agent_id: Option<&str>) {
        let task_ids = match kind {
            "fired" => &rule.failure_task_ids,
            "recovered" => &rule.recovery_task_ids,
            _ => return,
        };
        if task_ids.is_empty() {
            return;
        }
        spawn_triggered_tasks(
            self.db.clone(),
            self.session_registry.clone(),
            self.response_registry.clone(),
            task_ids.clone(),
            format!("alert:{}:{}", rule.id, kind),
            source_agent_id.map(str::to_string),
            Some(rule.owner_user_id.clone()),
        );
    }

    async fn channels_for_rule(&self, rule: &AlertRule) -> Result<Vec<NotificationChannel>> {
        let Some(ng) = &rule.notification_group_id else {
            return Ok(vec![]);
        };
        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            bool,
        )> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query_as("SELECT n.id, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls FROM notifications n JOIN notification_group_members ngm ON ngm.notification_id = n.id JOIN notification_groups ng ON ng.id = ngm.group_id WHERE ngm.group_id = ? AND ng.owner_user_id = ? AND n.owner_user_id = ? ORDER BY n.created_at ASC LIMIT ?")
                    .bind(ng)
                    .bind(&rule.owner_user_id)
                    .bind(&rule.owner_user_id)
                    .bind((NOTIFICATION_MAX_GROUP_CHANNELS + 1) as i64)
                    .fetch_all(pool)
                    .await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let Ok(group_id) = uuid::Uuid::parse_str(ng) else {
                    return Ok(Vec::new());
                };
                let Ok(owner_id) = uuid::Uuid::parse_str(&rule.owner_user_id) else {
                    return Ok(Vec::new());
                };
                sqlx::query_as("SELECT n.id::text, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls FROM notifications n JOIN notification_group_members ngm ON ngm.notification_id = n.id JOIN notification_groups ng ON ng.id = ngm.group_id WHERE ngm.group_id = $1 AND ng.owner_user_id = $2 AND n.owner_user_id = $2 ORDER BY n.created_at ASC LIMIT $3")
                    .bind(group_id)
                    .bind(owner_id)
                    .bind((NOTIFICATION_MAX_GROUP_CHANNELS + 1) as i64)
                    .fetch_all(pool)
                    .await?
            }
        };
        ensure_notification_channel_count_allowed(rows.len())?;
        let mut out = Vec::new();
        for (
            id,
            name,
            url,
            request_method,
            request_type,
            headers_json,
            body_template,
            verify_tls,
        ) in rows
        {
            let headers: HashMap<String, String> = headers_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            out.push(NotificationChannel {
                id,
                name,
                url,
                request_method,
                request_type,
                headers,
                body_template: body_template.unwrap_or_default(),
                verify_tls,
            });
        }
        Ok(out)
    }

    async fn load_alert_rules(&self) -> Result<Vec<AlertRule>> {
        let mut out = Vec::new();
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows: Vec<(
                    String,
                    String,
                    String,
                    i64,
                    String,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                )> = sqlx::query_as(
                    "SELECT id, owner_user_id, name, enabled, trigger_mode, rules_json, notification_group_id, fail_task_ids_json, recover_task_ids_json FROM alert_rules WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                for (
                    id,
                    owner_user_id,
                    name,
                    enabled,
                    trigger_mode,
                    rules_json,
                    notification_group_id,
                    fail_task_ids_json,
                    recover_task_ids_json,
                ) in rows
                {
                    let row = LoadedAlertRuleRow {
                        id,
                        owner_user_id,
                        name,
                        enabled: enabled != 0,
                        trigger_mode,
                        rules_json,
                        notification_group_id,
                        fail_task_ids_json,
                        recover_task_ids_json,
                    };
                    match validate_loaded_alert_rule(row) {
                        Ok(rule) => out.push(rule),
                        Err(err) => warn!("historical alert rule row skipped: {err}"),
                    }
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(
                    String,
                    String,
                    String,
                    bool,
                    String,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                )> = sqlx::query_as(
                    "SELECT id::text, owner_user_id::text, name, enabled, trigger_mode, rules_json, notification_group_id::text, fail_task_ids_json, recover_task_ids_json FROM alert_rules WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                for (
                    id,
                    owner_user_id,
                    name,
                    enabled,
                    trigger_mode,
                    rules_json,
                    notification_group_id,
                    fail_task_ids_json,
                    recover_task_ids_json,
                ) in rows
                {
                    let row = LoadedAlertRuleRow {
                        id,
                        owner_user_id,
                        name,
                        enabled,
                        trigger_mode,
                        rules_json,
                        notification_group_id,
                        fail_task_ids_json,
                        recover_task_ids_json,
                    };
                    match validate_loaded_alert_rule(row) {
                        Ok(rule) => out.push(rule),
                        Err(err) => warn!("historical alert rule row skipped: {err}"),
                    }
                }
            }
        }
        Ok(out)
    }

    pub async fn start(self: Arc<Self>) {
        info!("alert engine started");
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = self.evaluate_all().await {
                warn!("alert engine tick: {}", e);
            }
        }
    }
}

fn condition_agent_id(condition: &AlertCondition) -> Option<&str> {
    match condition {
        AlertCondition::ServerExpiry { agent_id, .. }
        | AlertCondition::ServerTrafficQuota { agent_id, .. }
        | AlertCondition::ServerOffline { agent_id, .. }
        | AlertCondition::ServerResource { agent_id, .. } => Some(agent_id.as_str()),
        AlertCondition::ServiceDown { .. }
        | AlertCondition::ServiceLatency { .. }
        | AlertCondition::CertificateExpiry { .. } => None,
    }
}

fn infer_rule_agent_id(rule: &AlertRule) -> Option<String> {
    infer_condition_agent_id(rule.conditions.iter())
}

fn infer_condition_agent_id<'a>(
    conditions: impl IntoIterator<Item = &'a AlertCondition>,
) -> Option<String> {
    let mut found: Option<&str> = None;
    for condition in conditions {
        let Some(agent_id) = condition_agent_id(condition) else {
            continue;
        };
        match found {
            Some(existing) if existing != agent_id => return None,
            Some(_) => {}
            None => found = Some(agent_id),
        }
    }
    found.map(str::to_string)
}

fn resource_value(state: &serde_json::Value, resource: ResourceType) -> Option<f64> {
    match resource {
        ResourceType::Cpu => state.get("cpu_percent").and_then(|v| v.as_f64()),
        ResourceType::Memory => {
            let used = state.get("memory_used").and_then(|v| v.as_f64())?;
            let total = state.get("memory_total").and_then(|v| v.as_f64())?;
            (total > 0.0).then_some((used / total) * 100.0)
        }
        ResourceType::Disk => {
            let arr = state.get("disks").and_then(|v| v.as_array())?;
            let mut used: f64 = 0.0;
            let mut total: f64 = 0.0;
            for d in arr {
                if let (Some(u), Some(t)) = (
                    d.get("used").and_then(|v| v.as_f64()),
                    d.get("total").and_then(|v| v.as_f64()),
                ) {
                    used += u;
                    total += t;
                }
            }
            (total > 0.0).then_some((used / total) * 100.0)
        }
        ResourceType::Network => network_total_bytes(state).map(|bytes| bytes as f64),
        ResourceType::NetworkIn => json_f64_by_keys(
            state,
            &["net_rx_bps", "network_in_speed", "network_rx_bps", "rx_bps"],
        ),
        ResourceType::NetworkOut => json_f64_by_keys(
            state,
            &[
                "net_tx_bps",
                "network_out_speed",
                "network_tx_bps",
                "tx_bps",
            ],
        ),
        ResourceType::NetworkTotal => {
            let rx = resource_value(state, ResourceType::NetworkIn).unwrap_or_default();
            let tx = resource_value(state, ResourceType::NetworkOut).unwrap_or_default();
            (rx > 0.0 || tx > 0.0).then_some(rx + tx)
        }
        ResourceType::TrafficInTotal => {
            network_total_by_field(state, "bytes_recv").map(|bytes| bytes as f64)
        }
        ResourceType::TrafficOutTotal => {
            network_total_by_field(state, "bytes_sent").map(|bytes| bytes as f64)
        }
        ResourceType::Load => state.get("load_1").and_then(|v| v.as_f64()),
        ResourceType::Load5 => json_f64_by_keys(state, &["load_5", "load5"]),
        ResourceType::Load15 => json_f64_by_keys(state, &["load_15", "load15"]),
        ResourceType::Swap => {
            json_f64_by_keys(state, &["swap_percent", "swap_usage"]).or_else(|| {
                let used = json_f64_by_keys(state, &["swap_used"])?;
                let total = json_f64_by_keys(state, &["swap_total"])?;
                (total > 0.0).then_some((used / total) * 100.0)
            })
        }
        ResourceType::Tcp => {
            json_f64_by_keys(state, &["tcp_connections", "tcp_conn_count", "tcp_count"])
        }
        ResourceType::Udp => {
            json_f64_by_keys(state, &["udp_connections", "udp_conn_count", "udp_count"])
        }
        ResourceType::Process => {
            json_f64_by_keys(state, &["process_count", "processes", "processes_count"])
        }
        ResourceType::Temperature => {
            json_f64_by_keys(state, &["temperature", "cpu_temp", "max_temp", "temp"])
                .or_else(|| {
                    max_array_number(state, "temperatures", &["value", "temperature", "temp"])
                })
                .or_else(|| max_array_number(state, "components", &["temperature", "temp"]))
        }
        ResourceType::Gpu => {
            json_f64_by_keys(state, &["gpu_percent", "gpu_usage", "gpu_utilization"]).or_else(
                || max_array_number(state, "gpus", &["utilization", "usage", "gpu_percent"]),
            )
        }
    }
}

fn network_total_bytes(state: &serde_json::Value) -> Option<u64> {
    let recv = network_total_by_field(state, "bytes_recv").unwrap_or_default();
    let sent = network_total_by_field(state, "bytes_sent").unwrap_or_default();
    (recv > 0 || sent > 0).then_some(recv.saturating_add(sent))
}

fn traffic_used_bytes(state: &serde_json::Value, direction: TrafficQuotaDirection) -> Option<u64> {
    match direction {
        TrafficQuotaDirection::Total => network_total_bytes(state),
        TrafficQuotaDirection::In => network_total_by_field(state, "bytes_recv"),
        TrafficQuotaDirection::Out => network_total_by_field(state, "bytes_sent"),
    }
}

fn traffic_quota_percent(
    state: &serde_json::Value,
    quota_bytes: u64,
    direction: TrafficQuotaDirection,
) -> Option<f64> {
    if quota_bytes == 0 {
        return None;
    }
    let used = traffic_used_bytes(state, direction)?;
    Some((used as f64 / quota_bytes as f64) * 100.0)
}

fn metadata_traffic_quota_bytes(value: &str) -> Option<u64> {
    let parsed = serde_json::from_str::<serde_json::Value>(value).ok()?;
    metadata_u64_from_value(
        &parsed,
        &[
            "traffic_quota_bytes",
            "traffic_quota",
            "quota_bytes",
            "bandwidth_quota_bytes",
            "monthly_traffic_bytes",
        ],
    )
}

fn metadata_u64_from_value(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(json_u64) {
            return Some(value);
        }
    }
    for container in ["billing", "plan", "metadata", "custom", "traffic", "limits"] {
        if let Some(child) = value.get(container) {
            for key in keys {
                if let Some(value) = child.get(*key).and_then(json_u64) {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn parse_server_expiry(value: &str) -> Option<DateTime<Utc>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(value.with_timezone(&Utc));
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return date
            .and_hms_opt(23, 59, 59)
            .map(|value| DateTime::from_naive_utc_and_offset(value, Utc));
    }
    None
}

fn validate_loaded_alert_rule(row: LoadedAlertRuleRow) -> Result<AlertRule> {
    let id = require_canonical_uuid(&row.id, "alert_rule_id")?;
    let owner_user_id = require_canonical_uuid(&row.owner_user_id, "owner_user_id")?;
    let name = row.name.trim().to_string();
    ensure_bounded_nonempty_text(&name, ALERT_MAX_NAME_BYTES, "name")?;
    let trigger_mode = parse_trigger_mode(&row.trigger_mode)?;
    let conditions = parse_bounded_alert_conditions(&row.rules_json)?;
    let notification_group_id = row
        .notification_group_id
        .map(|value| require_canonical_uuid(&value, "notification_group_id"))
        .transpose()?;
    let failure_task_ids =
        parse_bounded_task_ids_json(row.fail_task_ids_json, "fail_task_ids_json")?;
    let recovery_task_ids =
        parse_bounded_task_ids_json(row.recover_task_ids_json, "recover_task_ids_json")?;

    Ok(AlertRule {
        id,
        owner_user_id,
        name,
        enabled: row.enabled,
        trigger_mode,
        conditions,
        notification_group_id,
        failure_task_ids,
        recovery_task_ids,
    })
}

fn parse_trigger_mode(value: &str) -> Result<TriggerMode> {
    match value.trim() {
        "always" => Ok(TriggerMode::Always),
        "once" => Ok(TriggerMode::Once),
        _ => anyhow::bail!("trigger_mode must be always or once"),
    }
}

fn parse_bounded_alert_conditions(value: &str) -> Result<Vec<AlertCondition>> {
    if value.len() > ALERT_MAX_CONDITIONS * ALERT_MAX_CONDITION_BYTES {
        anyhow::bail!("rules_json exceeds runtime budget");
    }
    let raw =
        serde_json::from_str::<Vec<serde_json::Value>>(value).context("invalid rules_json")?;
    if raw.is_empty() {
        anyhow::bail!("at least one alert condition is required");
    }
    if raw.len() > ALERT_MAX_CONDITIONS {
        anyhow::bail!("conditions exceeds {ALERT_MAX_CONDITIONS} items");
    }
    let mut conditions = Vec::new();
    for condition in raw {
        let bytes = serde_json::to_vec(&condition)?.len();
        if bytes > ALERT_MAX_CONDITION_BYTES {
            anyhow::bail!("condition exceeds {ALERT_MAX_CONDITION_BYTES} bytes");
        }
        let condition: AlertCondition =
            serde_json::from_value(condition).context("invalid alert condition")?;
        validate_runtime_condition(&condition)?;
        conditions.push(condition);
    }
    Ok(conditions)
}

fn validate_runtime_condition(condition: &AlertCondition) -> Result<()> {
    match condition {
        AlertCondition::ServiceDown {
            service_id,
            consecutive_failures,
        } => {
            require_canonical_uuid(service_id, "service_id")?;
            if *consecutive_failures == 0 || *consecutive_failures > ALERT_MAX_CONSECUTIVE_FAILURES
            {
                anyhow::bail!(
                    "consecutive_failures must be between 1 and {ALERT_MAX_CONSECUTIVE_FAILURES}"
                );
            }
        }
        AlertCondition::ServiceLatency {
            service_id,
            max_latency_ms,
        } => {
            require_canonical_uuid(service_id, "service_id")?;
            if *max_latency_ms <= 0 || *max_latency_ms > ALERT_MAX_LATENCY_MS {
                anyhow::bail!("max_latency_ms must be between 1 and {ALERT_MAX_LATENCY_MS}");
            }
        }
        AlertCondition::CertificateExpiry {
            service_id,
            days_before,
        } => {
            require_canonical_uuid(service_id, "service_id")?;
            ensure_days_before(*days_before)?;
        }
        AlertCondition::ServerExpiry {
            agent_id,
            days_before,
        } => {
            require_canonical_uuid(agent_id, "agent_id")?;
            ensure_days_before(*days_before)?;
        }
        AlertCondition::ServerTrafficQuota {
            agent_id, percent, ..
        } => {
            require_canonical_uuid(agent_id, "agent_id")?;
            if !percent.is_finite() || *percent <= 0.0 || *percent > ALERT_MAX_TRAFFIC_PERCENT {
                anyhow::bail!(
                    "percent must be greater than 0 and at most {ALERT_MAX_TRAFFIC_PERCENT}"
                );
            }
        }
        AlertCondition::ServerOffline {
            agent_id,
            offline_seconds,
        } => {
            require_canonical_uuid(agent_id, "agent_id")?;
            if *offline_seconds == 0 || *offline_seconds > ALERT_MAX_OFFLINE_SECONDS {
                anyhow::bail!("offline_seconds must be between 1 and {ALERT_MAX_OFFLINE_SECONDS}");
            }
        }
        AlertCondition::ServerResource {
            agent_id,
            threshold,
            duration_seconds,
            ..
        } => {
            require_canonical_uuid(agent_id, "agent_id")?;
            if !threshold.is_finite() || threshold.abs() > ALERT_MAX_RESOURCE_THRESHOLD {
                anyhow::bail!(
                    "threshold must be finite and within +/-{ALERT_MAX_RESOURCE_THRESHOLD}"
                );
            }
            if *duration_seconds > ALERT_MAX_RESOURCE_DURATION_SECONDS {
                anyhow::bail!(
                    "duration_seconds must be at most {ALERT_MAX_RESOURCE_DURATION_SECONDS}"
                );
            }
        }
    }
    Ok(())
}

fn ensure_days_before(value: i64) -> Result<()> {
    if !(0..=ALERT_MAX_DAYS_BEFORE).contains(&value) {
        anyhow::bail!("days_before must be between 0 and {ALERT_MAX_DAYS_BEFORE}");
    }
    Ok(())
}

fn parse_bounded_task_ids_json(value: Option<String>, field: &str) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let values = serde_json::from_str::<Vec<String>>(&value)
        .with_context(|| format!("{field} must be a JSON string array"))?;
    if values.len() > ALERT_MAX_TASK_IDS {
        anyhow::bail!("{field} exceeds {ALERT_MAX_TASK_IDS} items");
    }
    let mut out = Vec::new();
    for value in values {
        let id = require_canonical_uuid(&value, "task_id")?;
        if !out.iter().any(|existing| existing == &id) {
            out.push(id);
        }
    }
    Ok(out)
}

fn ensure_bounded_nonempty_text(value: &str, max_bytes: usize, field: &str) -> Result<()> {
    if value.is_empty() {
        anyhow::bail!("{field} is required");
    }
    if value.len() > max_bytes {
        anyhow::bail!("{field} exceeds {max_bytes} bytes");
    }
    Ok(())
}

fn require_canonical_uuid(value: &str, field: &str) -> Result<String> {
    let value = value.trim();
    if value.len() != 36 {
        anyhow::bail!("{field} must be a canonical UUID");
    }
    let parsed = uuid::Uuid::parse_str(value)
        .with_context(|| format!("{field} must be a canonical UUID"))?;
    if parsed.to_string() != value {
        anyhow::bail!("{field} must be a canonical UUID");
    }
    Ok(value.to_string())
}

fn parse_id_list_json(value: Option<String>) -> Vec<String> {
    value
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .map(normalize_id_list)
        .unwrap_or_default()
}

fn normalize_id_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn normalize_uuid_id_list(values: Vec<String>) -> Vec<String> {
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

fn unique_uuid_value(values: Vec<String>) -> Option<uuid::Uuid> {
    let mut found = None;
    for value in values {
        let Ok(id) = uuid::Uuid::parse_str(value.trim()) else {
            return None;
        };
        match found {
            Some(existing) if existing != id => return None,
            Some(_) => {}
            None => found = Some(id),
        }
    }
    found
}

fn sqlite_placeholders(len: usize) -> String {
    std::iter::repeat_n("?", len).collect::<Vec<_>>().join(", ")
}

fn parse_uuid_ids(ids: &[String]) -> Result<Vec<uuid::Uuid>> {
    ids.iter()
        .map(|id| uuid::Uuid::parse_str(id).context("invalid server_id"))
        .collect()
}

fn network_total_by_field(state: &serde_json::Value, field: &str) -> Option<u64> {
    let direct_keys = match field {
        "bytes_recv" => &["network_in_total", "net_rx_bytes", "bytes_recv_total"][..],
        "bytes_sent" => &["network_out_total", "net_tx_bytes", "bytes_sent_total"][..],
        _ => &[][..],
    };
    for key in direct_keys {
        if let Some(value) = state.get(*key).and_then(json_u64) {
            return Some(value);
        }
    }
    let net_io = state.get("net_io").and_then(|v| v.as_array())?;
    let mut total = 0u64;
    for nic in net_io {
        total = total.saturating_add(nic.get(field).and_then(json_u64).unwrap_or_default());
    }
    Some(total)
}

fn json_f64_by_keys(state: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(value) = state.get(*key).and_then(json_f64) {
            return Some(value);
        }
    }
    None
}

fn json_f64(value: &serde_json::Value) -> Option<f64> {
    if let Some(value) = value.as_f64() {
        return value.is_finite().then_some(value);
    }
    value
        .as_str()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn json_u64(value: &serde_json::Value) -> Option<u64> {
    if let Some(value) = value.as_u64() {
        return Some(value);
    }
    if let Some(value) = value.as_i64() {
        return u64::try_from(value).ok();
    }
    value
        .as_str()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn max_array_number(state: &serde_json::Value, array_key: &str, keys: &[&str]) -> Option<f64> {
    state
        .get(array_key)
        .and_then(|value| value.as_array())
        .and_then(|items| {
            items
                .iter()
                .filter_map(|item| json_f64_by_keys(item, keys))
                .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_compare() {
        assert!(Operator::Gt.compare(2.0, 1.0));
        assert!(!Operator::Gt.compare(1.0, 1.0));
        assert!(Operator::Gte.compare(1.0, 1.0));
        assert!(Operator::Lte.compare(1.0, 1.0));
        assert!(!Operator::Lt.compare(1.0, 1.0));
    }

    #[test]
    fn trigger_mode_round_trip() {
        assert_eq!(TriggerMode::from_db("always"), TriggerMode::Always);
        assert_eq!(TriggerMode::from_db("once"), TriggerMode::Once);
        assert_eq!(TriggerMode::from_db("garbage"), TriggerMode::Once);
        assert_eq!(TriggerMode::Always.as_db(), "always");
        assert_eq!(TriggerMode::Once.as_db(), "once");
    }

    #[test]
    fn condition_round_trip() {
        let c = AlertCondition::ServerResource {
            agent_id: "a".into(),
            resource: ResourceType::Cpu,
            operator: Operator::Gt,
            threshold: 80.0,
            duration_seconds: 60,
        };
        let j = serde_json::to_string(&c).unwrap();
        let back: AlertCondition = serde_json::from_str(&j).unwrap();
        match back {
            AlertCondition::ServerResource { threshold, .. } => {
                assert!((threshold - 80.0).abs() < 1e-6)
            }
            _ => panic!(),
        }
    }

    #[test]
    fn server_asset_conditions_round_trip() {
        let expiry = AlertCondition::ServerExpiry {
            agent_id: "a".into(),
            days_before: 14,
        };
        let quota = AlertCondition::ServerTrafficQuota {
            agent_id: "a".into(),
            percent: 80.0,
            direction: TrafficQuotaDirection::Out,
        };

        let expiry_back: AlertCondition =
            serde_json::from_str(&serde_json::to_string(&expiry).unwrap()).unwrap();
        let quota_back: AlertCondition =
            serde_json::from_str(&serde_json::to_string(&quota).unwrap()).unwrap();

        assert!(matches!(
            expiry_back,
            AlertCondition::ServerExpiry {
                days_before: 14,
                ..
            }
        ));
        assert!(matches!(
            quota_back,
            AlertCondition::ServerTrafficQuota {
                percent,
                direction: TrafficQuotaDirection::Out,
                ..
            } if (percent - 80.0).abs() < 1e-6
        ));
    }

    #[test]
    fn extracts_extended_resource_values() {
        let state = serde_json::json!({
            "swap_used": 512,
            "swap_total": 1024,
            "net_rx_bps": 1000,
            "net_tx_bps": 2000,
            "network_in_total": 3000,
            "network_out_total": 4000,
            "load_5": 0.7,
            "load_15": 0.9,
            "tcp_connections": 42,
            "udp_count": 7,
            "process_count": 128,
            "temperatures": [{ "value": 41.0 }, { "value": 55.5 }],
            "gpus": [{ "utilization": 76.0 }]
        });

        assert_eq!(resource_value(&state, ResourceType::Swap), Some(50.0));
        assert_eq!(
            resource_value(&state, ResourceType::NetworkIn),
            Some(1000.0)
        );
        assert_eq!(
            resource_value(&state, ResourceType::NetworkOut),
            Some(2000.0)
        );
        assert_eq!(
            resource_value(&state, ResourceType::NetworkTotal),
            Some(3000.0)
        );
        assert_eq!(
            resource_value(&state, ResourceType::TrafficInTotal),
            Some(3000.0)
        );
        assert_eq!(
            resource_value(&state, ResourceType::TrafficOutTotal),
            Some(4000.0)
        );
        assert_eq!(resource_value(&state, ResourceType::Load5), Some(0.7));
        assert_eq!(resource_value(&state, ResourceType::Load15), Some(0.9));
        assert_eq!(resource_value(&state, ResourceType::Tcp), Some(42.0));
        assert_eq!(resource_value(&state, ResourceType::Udp), Some(7.0));
        assert_eq!(resource_value(&state, ResourceType::Process), Some(128.0));
        assert_eq!(
            resource_value(&state, ResourceType::Temperature),
            Some(55.5)
        );
        assert_eq!(resource_value(&state, ResourceType::Gpu), Some(76.0));
    }

    #[test]
    fn parses_asset_expiry_and_traffic_quota() {
        assert!(parse_server_expiry("2026-12-31").is_some());
        assert!(parse_server_expiry("2026-12-31T08:00:00Z").is_some());
        assert!(parse_server_expiry("not-a-date").is_none());

        assert_eq!(
            metadata_traffic_quota_bytes(r#"{"traffic":{"traffic_quota_bytes":1099511627776}}"#),
            Some(1_099_511_627_776)
        );

        let state = serde_json::json!({
            "network_in_total": 400,
            "network_out_total": 600
        });
        assert_eq!(
            traffic_quota_percent(&state, 1000, TrafficQuotaDirection::Total),
            Some(100.0)
        );
        assert_eq!(
            traffic_quota_percent(&state, 1000, TrafficQuotaDirection::In),
            Some(40.0)
        );
        assert_eq!(
            traffic_quota_percent(&state, 1000, TrafficQuotaDirection::Out),
            Some(60.0)
        );
    }

    #[tokio::test]
    async fn observe_and_read_latest() {
        let pool = sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let db = Db::Sqlite(pool);
        let engine = Arc::new(AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        ));
        engine
            .observe_agent_state("agent-1", serde_json::json!({"cpu_percent": 42.0}))
            .await;
        let h = engine.latest_handle();
        let m = h.read().await;
        assert_eq!(
            m.get("agent-1")
                .unwrap()
                .get("cpu_percent")
                .unwrap()
                .as_f64()
                .unwrap(),
            42.0
        );
    }

    #[tokio::test]
    async fn channels_for_rule_requires_notification_group_owner() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let group = "00000000-0000-0000-0000-000000000301";
        let notification = "00000000-0000-0000-0000-000000000401";

        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_notification_group(&db, group, other, "other-group").await;
        seed_notification(&db, notification, other, "other-channel").await;
        seed_notification_group_member(&db, group, notification).await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );
        let dirty_rule = AlertRule {
            id: "00000000-0000-0000-0000-000000000501".into(),
            owner_user_id: owner.into(),
            name: "dirty-rule".into(),
            enabled: true,
            trigger_mode: TriggerMode::Once,
            conditions: Vec::new(),
            notification_group_id: Some(group.into()),
            failure_task_ids: Vec::new(),
            recovery_task_ids: Vec::new(),
        };

        let channels = engine.channels_for_rule(&dirty_rule).await.unwrap();
        assert!(channels.is_empty());
    }

    #[test]
    fn loaded_alert_rule_rejects_invalid_historical_fields() {
        let valid = test_loaded_alert_rule_row();
        let rule = validate_loaded_alert_rule(valid).unwrap();
        assert_eq!(rule.trigger_mode, TriggerMode::Once);
        assert_eq!(rule.failure_task_ids.len(), 1);

        let mut malformed_tasks = test_loaded_alert_rule_row();
        malformed_tasks.fail_task_ids_json = Some("not json".into());
        assert!(validate_loaded_alert_rule(malformed_tasks).is_err());

        let mut simple_task_id = test_loaded_alert_rule_row();
        simple_task_id.fail_task_ids_json = Some(r#"["00000000000000000000000000000401"]"#.into());
        assert!(validate_loaded_alert_rule(simple_task_id).is_err());

        let mut unknown_trigger = test_loaded_alert_rule_row();
        unknown_trigger.trigger_mode = "sometimes".into();
        assert!(validate_loaded_alert_rule(unknown_trigger).is_err());

        let mut oversized_offline = test_loaded_alert_rule_row();
        oversized_offline.rules_json = serde_json::to_string(&[AlertCondition::ServerOffline {
            agent_id: "00000000-0000-0000-0000-000000000101".into(),
            offline_seconds: ALERT_MAX_OFFLINE_SECONDS + 1,
        }])
        .unwrap();
        assert!(validate_loaded_alert_rule(oversized_offline).is_err());

        let mut zero_failures = test_loaded_alert_rule_row();
        zero_failures.rules_json = serde_json::to_string(&[AlertCondition::ServiceDown {
            service_id: "00000000-0000-0000-0000-000000000301".into(),
            consecutive_failures: 0,
        }])
        .unwrap();
        assert!(validate_loaded_alert_rule(zero_failures).is_err());
    }

    #[tokio::test]
    async fn invalid_historical_alert_rules_are_not_loaded() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let valid_rule = "00000000-0000-0000-0000-000000000501";
        let bad_json_rule = "00000000-0000-0000-0000-000000000502";
        let bad_trigger_rule = "00000000-0000-0000-0000-000000000503";
        let bad_tasks_rule = "00000000-0000-0000-0000-000000000504";
        let bad_condition_rule = "00000000-0000-0000-0000-000000000505";
        seed_user(&db, owner, "owner").await;
        seed_raw_alert_rule(
            &db,
            valid_rule,
            owner,
            "valid",
            "once",
            &serde_json::to_string(&[AlertCondition::ServerOffline {
                agent_id: "00000000-0000-0000-0000-000000000101".into(),
                offline_seconds: 60,
            }])
            .unwrap(),
            Some(r#"["00000000-0000-0000-0000-000000000401"]"#),
        )
        .await;
        seed_raw_alert_rule(
            &db,
            bad_json_rule,
            owner,
            "bad-json",
            "once",
            "not json",
            None,
        )
        .await;
        seed_raw_alert_rule(
            &db,
            bad_trigger_rule,
            owner,
            "bad-trigger",
            "sometimes",
            &serde_json::to_string(&[AlertCondition::ServerOffline {
                agent_id: "00000000-0000-0000-0000-000000000101".into(),
                offline_seconds: 60,
            }])
            .unwrap(),
            None,
        )
        .await;
        seed_raw_alert_rule(
            &db,
            bad_tasks_rule,
            owner,
            "bad-tasks",
            "once",
            &serde_json::to_string(&[AlertCondition::ServerOffline {
                agent_id: "00000000-0000-0000-0000-000000000101".into(),
                offline_seconds: 60,
            }])
            .unwrap(),
            Some("not json"),
        )
        .await;
        seed_raw_alert_rule(
            &db,
            bad_condition_rule,
            owner,
            "bad-condition",
            "once",
            &serde_json::to_string(&[AlertCondition::ServerOffline {
                agent_id: "00000000-0000-0000-0000-000000000101".into(),
                offline_seconds: 0,
            }])
            .unwrap(),
            None,
        )
        .await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );
        let rules = engine.load_alert_rules().await.unwrap();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, valid_rule);
        assert_eq!(
            rules[0].failure_task_ids,
            vec!["00000000-0000-0000-0000-000000000401".to_string()]
        );
    }

    #[tokio::test]
    async fn alert_engine_skips_server_conditions_outside_rule_owner() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let other_server = "00000000-0000-0000-0000-000000000202";

        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, other_server, other, "other").await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );
        let rule = AlertRule {
            id: "00000000-0000-0000-0000-000000000501".into(),
            owner_user_id: owner.into(),
            name: "dirty-server-rule".into(),
            enabled: true,
            trigger_mode: TriggerMode::Once,
            conditions: vec![AlertCondition::ServerOffline {
                agent_id: other_server.into(),
                offline_seconds: 1,
            }],
            notification_group_id: None,
            failure_task_ids: Vec::new(),
            recovery_task_ids: Vec::new(),
        };

        assert!(!engine
            .condition_belongs_to_rule_owner(&rule, &rule.conditions[0])
            .await
            .unwrap());
        engine.evaluate_rule(&rule).await.unwrap();
        assert!(!engine
            .states
            .read()
            .await
            .get(&rule.id)
            .map(|state| state.is_active)
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn alert_engine_skips_service_conditions_outside_rule_owner() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let other_server = "00000000-0000-0000-0000-000000000202";
        let other_service = "00000000-0000-0000-0000-000000000302";

        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, other_server, other, "other").await;
        seed_service_with_servers(&db, other_service, other, &[other_server], "specific").await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000401",
            other_service,
            other_server,
            "failure",
            None,
            None,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );
        let rule = AlertRule {
            id: "00000000-0000-0000-0000-000000000502".into(),
            owner_user_id: owner.into(),
            name: "dirty-service-rule".into(),
            enabled: true,
            trigger_mode: TriggerMode::Once,
            conditions: vec![AlertCondition::ServiceDown {
                service_id: other_service.into(),
                consecutive_failures: 1,
            }],
            notification_group_id: None,
            failure_task_ids: Vec::new(),
            recovery_task_ids: Vec::new(),
        };

        assert!(!engine
            .condition_belongs_to_rule_owner(&rule, &rule.conditions[0])
            .await
            .unwrap());
        engine.evaluate_rule(&rule).await.unwrap();
        assert!(!engine
            .states
            .read()
            .await
            .get(&rule.id)
            .map(|state| state.is_active)
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn alert_recovery_source_ignores_skipped_foreign_agent_conditions() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let owner_server = "00000000-0000-0000-0000-000000000101";
        let other_server = "00000000-0000-0000-0000-000000000202";
        let rule_id = "00000000-0000-0000-0000-000000000503";

        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, owner_server, owner, "owner-server").await;
        seed_agent(&db, other_server, other, "other-server").await;
        seed_alert_rule(&db, rule_id, owner, "recovery-source-rule").await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );
        let rule = AlertRule {
            id: rule_id.into(),
            owner_user_id: owner.into(),
            name: "recovery-source-rule".into(),
            enabled: true,
            trigger_mode: TriggerMode::Once,
            conditions: vec![
                AlertCondition::ServerOffline {
                    agent_id: other_server.into(),
                    offline_seconds: 1,
                },
                AlertCondition::ServerResource {
                    agent_id: owner_server.into(),
                    resource: ResourceType::Cpu,
                    operator: Operator::Gt,
                    threshold: 99.0,
                    duration_seconds: 0,
                },
            ],
            notification_group_id: None,
            failure_task_ids: Vec::new(),
            recovery_task_ids: Vec::new(),
        };

        engine.states.write().await.insert(
            rule.id.clone(),
            AlertState {
                rule_id: rule.id.clone(),
                is_active: true,
                last_fired_at: Some(Utc::now()),
                last_always_fire_at: None,
            },
        );
        engine.evaluate_rule(&rule).await.unwrap();

        let events = alert_events_for_rule(&engine.db, rule_id).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "recovered");
        assert_eq!(events[0].agent_id.as_deref(), Some(owner_server));
    }

    #[tokio::test]
    async fn service_down_uses_current_service_server_scope() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let allowed_server = "00000000-0000-0000-0000-000000000101";
        let stale_server = "00000000-0000-0000-0000-000000000202";
        let service = "00000000-0000-0000-0000-000000000301";

        seed_user(&db, owner, "owner").await;
        seed_agent(&db, allowed_server, owner, "allowed").await;
        seed_agent(&db, stale_server, owner, "stale").await;
        seed_service_with_servers(&db, service, owner, &[allowed_server], "specific").await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000401",
            service,
            allowed_server,
            "success",
            None,
            None,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000402",
            service,
            stale_server,
            "failure",
            None,
            None,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );

        assert_eq!(
            engine
                .count_recent_service_failures(service, 1)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn service_latency_uses_local_scope_for_local_services() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let server = "00000000-0000-0000-0000-000000000101";
        let service = "00000000-0000-0000-0000-000000000302";

        seed_user(&db, owner, "owner").await;
        seed_agent(&db, server, owner, "server").await;
        seed_service_with_servers(&db, service, owner, &[], "local").await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000411",
            service,
            "",
            "success",
            Some(50),
            None,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000412",
            service,
            server,
            "success",
            Some(5000),
            None,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );

        assert_eq!(engine.latest_service_latency_ms(service).await.unwrap(), 50);
    }

    #[tokio::test]
    async fn certificate_expiry_uses_current_service_server_scope() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let allowed_server = "00000000-0000-0000-0000-000000000101";
        let stale_server = "00000000-0000-0000-0000-000000000202";
        let service = "00000000-0000-0000-0000-000000000303";

        seed_user(&db, owner, "owner").await;
        seed_agent(&db, allowed_server, owner, "allowed").await;
        seed_agent(&db, stale_server, owner, "stale").await;
        seed_service_with_servers(&db, service, owner, &[allowed_server], "specific").await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000421",
            service,
            allowed_server,
            "success",
            Some(10),
            Some("2026-06-01T00:00:00Z"),
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000422",
            service,
            stale_server,
            "success",
            Some(10),
            Some("2026-01-01T00:00:00Z"),
            "2026-01-03T00:00:00Z",
        )
        .await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );

        let not_after = engine
            .latest_cert_not_after(service)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(not_after.to_rfc3339(), "2026-06-01T00:00:00+00:00");
    }

    #[tokio::test]
    async fn service_alert_all_scope_uses_service_owner_agents() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let own_server = "00000000-0000-0000-0000-000000000101";
        let other_server = "00000000-0000-0000-0000-000000000202";
        let service = "00000000-0000-0000-0000-000000000304";

        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, own_server, owner, "own").await;
        seed_agent(&db, other_server, other, "other").await;
        seed_service_with_servers(&db, service, owner, &[], "all").await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000431",
            service,
            own_server,
            "success",
            Some(50),
            None,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000432",
            service,
            other_server,
            "failure",
            Some(5000),
            None,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );

        assert_eq!(
            engine
                .count_recent_service_failures(service, 1)
                .await
                .unwrap(),
            0
        );
        assert_eq!(engine.latest_service_latency_ms(service).await.unwrap(), 50);
    }

    #[tokio::test]
    async fn service_alert_specific_scope_filters_to_service_owner_agents() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let own_server = "00000000-0000-0000-0000-000000000101";
        let other_server = "00000000-0000-0000-0000-000000000202";
        let service = "00000000-0000-0000-0000-000000000307";

        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, own_server, owner, "own").await;
        seed_agent(&db, other_server, other, "other").await;
        seed_service_with_servers(&db, service, owner, &[own_server, other_server], "specific")
            .await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000461",
            service,
            own_server,
            "success",
            Some(25),
            None,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000462",
            service,
            other_server,
            "failure",
            Some(5000),
            None,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );

        assert_eq!(
            engine
                .count_recent_service_failures(service, 1)
                .await
                .unwrap(),
            0
        );
        assert_eq!(engine.latest_service_latency_ms(service).await.unwrap(), 25);
    }

    #[tokio::test]
    async fn service_alert_exclude_scope_uses_service_owner_agents() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let own_a = "00000000-0000-0000-0000-000000000101";
        let own_b = "00000000-0000-0000-0000-000000000102";
        let other_server = "00000000-0000-0000-0000-000000000202";
        let service = "00000000-0000-0000-0000-000000000305";

        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, own_a, owner, "own-a").await;
        seed_agent(&db, own_b, owner, "own-b").await;
        seed_agent(&db, other_server, other, "other").await;
        seed_service_with_servers(&db, service, owner, &[], "exclude").await;
        set_service_excludes(&db, service, &[own_a, other_server]).await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000441",
            service,
            own_a,
            "failure",
            Some(1000),
            None,
            "2026-01-03T00:00:00Z",
        )
        .await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000442",
            service,
            own_b,
            "success",
            Some(40),
            None,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000443",
            service,
            other_server,
            "failure",
            Some(5000),
            None,
            "2026-01-04T00:00:00Z",
        )
        .await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );

        assert_eq!(
            engine
                .count_recent_service_failures(service, 1)
                .await
                .unwrap(),
            0
        );
        assert_eq!(engine.latest_service_latency_ms(service).await.unwrap(), 40);
    }

    #[tokio::test]
    async fn service_alert_all_scope_without_owner_skips_global_results() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let server = "00000000-0000-0000-0000-000000000101";
        let service = "00000000-0000-0000-0000-000000000306";

        seed_user(&db, owner, "owner").await;
        seed_agent(&db, server, owner, "server").await;
        seed_service_without_owner(&db, service, "all").await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000451",
            service,
            server,
            "failure",
            Some(5000),
            None,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let engine = AlertEngine::new(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
        );

        assert_eq!(
            engine
                .count_recent_service_failures(service, 1)
                .await
                .unwrap(),
            0
        );
        assert_eq!(engine.latest_service_latency_ms(service).await.unwrap(), -1);
    }

    async fn test_db() -> Db {
        let db = Db::connect("sqlite::memory:", true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_user(db: &Db, id: &str, username: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'hash', 'member', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(username)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_agent(db: &Db, id: &str, owner: &str, name: &str) {
        let Db::Sqlite(pool) = db else {
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

    async fn seed_service_with_servers(
        db: &Db,
        id: &str,
        owner: &str,
        server_ids: &[&str],
        cover_mode: &str,
    ) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        let primary = server_ids.first().copied();
        sqlx::query(
            "INSERT INTO services (id, owner_user_id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, cover_mode, created_at, updated_at) VALUES (?, ?, 'svc', 'http', 'https://example.com', 60, 10, 1, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .bind(primary)
        .bind(cover_mode)
        .execute(pool)
        .await
        .unwrap();
        for server_id in server_ids {
            sqlx::query(
                "INSERT INTO service_servers (service_id, server_id, created_at) VALUES (?, ?, '2026-01-01T00:00:00Z')",
            )
            .bind(id)
            .bind(server_id)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn seed_service_without_owner(db: &Db, id: &str, cover_mode: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO services (id, name, type, target, interval_seconds, timeout_seconds, enabled, cover_mode, created_at, updated_at) VALUES (?, 'svc', 'http', 'https://example.com', 60, 10, 1, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(cover_mode)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn set_service_excludes(db: &Db, id: &str, server_ids: &[&str]) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        let value = serde_json::to_string(server_ids).unwrap();
        sqlx::query("UPDATE services SET exclude_server_ids_json = ? WHERE id = ?")
            .bind(value)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_service_result(
        db: &Db,
        id: &str,
        service_id: &str,
        server_id: &str,
        status: &str,
        delay_ms: Option<i64>,
        cert_not_after: Option<&str>,
        created_at: &str,
    ) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO service_results (id, service_id, server_id, status, delay_ms, cert_not_after, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(service_id)
        .bind((!server_id.is_empty()).then_some(server_id))
        .bind(status)
        .bind(delay_ms)
        .bind(cert_not_after)
        .bind(created_at)
        .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_alert_rule(db: &Db, id: &str, owner: &str, name: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        let rules_json = serde_json::to_string(&Vec::<AlertCondition>::new()).unwrap();
        sqlx::query(
            "INSERT INTO alert_rules (id, owner_user_id, name, enabled, trigger_mode, rules_json, created_at, updated_at) VALUES (?, ?, ?, 1, 'once', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .bind(name)
        .bind(rules_json)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_raw_alert_rule(
        db: &Db,
        id: &str,
        owner: &str,
        name: &str,
        trigger_mode: &str,
        rules_json: &str,
        fail_task_ids_json: Option<&str>,
    ) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO alert_rules (id, owner_user_id, name, enabled, trigger_mode, rules_json, fail_task_ids_json, created_at, updated_at) VALUES (?, ?, ?, 1, ?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .bind(name)
        .bind(trigger_mode)
        .bind(rules_json)
        .bind(fail_task_ids_json)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn alert_events_for_rule(
        db: &Db,
        rule_id: &str,
    ) -> Vec<crate::db::repository::alerts::AlertEventRow> {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        let rows: Vec<(String, String, Option<String>, Option<String>, String, String, String)> =
            sqlx::query_as(
                "SELECT id, rule_id, agent_id, service_id, kind, payload_json, fired_at FROM alert_events WHERE rule_id = ? ORDER BY fired_at ASC",
            )
            .bind(rule_id)
            .fetch_all(pool)
            .await
            .unwrap();
        rows.into_iter()
            .map(
                |(id, rule_id, agent_id, service_id, kind, payload_json, fired_at)| {
                    crate::db::repository::alerts::AlertEventRow {
                        id,
                        rule_id,
                        agent_id,
                        service_id,
                        kind,
                        payload_json,
                        fired_at: DateTime::parse_from_rfc3339(&fired_at)
                            .unwrap()
                            .with_timezone(&Utc),
                    }
                },
            )
            .collect()
    }

    async fn seed_notification_group(db: &Db, id: &str, owner: &str, name: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notification_groups (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_notification(db: &Db, id: &str, owner: &str, name: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notifications (id, owner_user_id, name, url, request_method, request_type, verify_tls, format_metric_units, created_at, updated_at) VALUES (?, ?, ?, 'https://example.com/hook', 'POST', 'json', 1, 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_notification_group_member(db: &Db, group_id: &str, notification_id: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notification_group_members (group_id, notification_id) VALUES (?, ?)",
        )
        .bind(group_id)
        .bind(notification_id)
        .execute(pool)
        .await
        .unwrap();
    }

    fn test_loaded_alert_rule_row() -> LoadedAlertRuleRow {
        LoadedAlertRuleRow {
            id: "00000000-0000-0000-0000-000000000501".into(),
            owner_user_id: "00000000-0000-0000-0000-000000000001".into(),
            name: "cpu-alert".into(),
            enabled: true,
            trigger_mode: "once".into(),
            rules_json: serde_json::to_string(&[AlertCondition::ServerOffline {
                agent_id: "00000000-0000-0000-0000-000000000101".into(),
                offline_seconds: 60,
            }])
            .unwrap(),
            notification_group_id: Some("00000000-0000-0000-0000-000000000301".into()),
            fail_task_ids_json: Some(r#"["00000000-0000-0000-0000-000000000401"]"#.into()),
            recover_task_ids_json: Some(r#"["00000000-0000-0000-0000-000000000402"]"#.into()),
        }
    }
}
