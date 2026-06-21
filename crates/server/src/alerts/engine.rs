//! M4 alert engine.
//!
//! See `docs/implementation-audit.md` for the full design notes.

use crate::db::Db;
use crate::grpc::{SessionRegistry, TaskResponseRegistry};
use crate::notifications::sender::{
    NotificationChannel, NotificationMessage, NotificationSender, NotificationSeverity,
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
        for c in &rule.conditions {
            if self.check_condition(c).await? {
                any_triggered = true;
                source_agent_id = condition_agent_id(c).map(str::to_string);
                break;
            }
        }
        let fallback_source_agent_id = source_agent_id.or_else(|| infer_rule_agent_id(rule));

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

    async fn count_recent_service_failures(&self, service_id: &str, n: i64) -> Result<i64> {
        let rows: Vec<(String,)> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query_as("SELECT status FROM service_results WHERE service_id = ? ORDER BY created_at DESC LIMIT ?")
                    .bind(service_id)
                    .bind(n)
                    .fetch_all(pool)
                    .await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                sqlx::query_as("SELECT status FROM service_results WHERE service_id = $1 ORDER BY created_at DESC LIMIT $2")
                    .bind(sid)
                    .bind(n)
                    .fetch_all(pool)
                    .await?
            }
        };
        Ok(rows.iter().filter(|(s,)| s == "failure").count() as i64)
    }

    async fn latest_service_latency_ms(&self, service_id: &str) -> Result<i64> {
        let row: Option<(Option<i64>,)> = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query_as("SELECT delay_ms FROM service_results WHERE service_id = ? AND status = 'success' ORDER BY created_at DESC LIMIT 1")
                    .bind(service_id)
                    .fetch_optional(pool)
                    .await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                sqlx::query_as("SELECT delay_ms FROM service_results WHERE service_id = $1 AND status = 'success' ORDER BY created_at DESC LIMIT 1")
                    .bind(sid)
                    .fetch_optional(pool)
                    .await?
            }
        };
        Ok(row.and_then(|(v,)| v).unwrap_or(-1))
    }

    async fn latest_cert_not_after(&self, service_id: &str) -> Result<Option<DateTime<Utc>>> {
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT cert_not_after FROM service_results WHERE service_id = ? AND cert_not_after IS NOT NULL ORDER BY created_at DESC LIMIT 1",
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
                    "SELECT cert_not_after FROM service_results WHERE service_id = $1 AND cert_not_after IS NOT NULL ORDER BY created_at DESC LIMIT 1",
                )
                .bind(sid)
                .fetch_optional(pool)
                .await?;
                Ok(row.map(|(value,)| value))
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
                sqlx::query_as("SELECT n.id, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls FROM notifications n JOIN notification_group_members ngm ON ngm.notification_id = n.id WHERE ngm.group_id = ?")
                    .bind(ng)
                    .fetch_all(pool)
                    .await?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query_as("SELECT n.id::text, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls FROM notifications n JOIN notification_group_members ngm ON ngm.notification_id = n.id WHERE ngm.group_id = $1")
                    .bind(ng)
                    .fetch_all(pool)
                    .await?
            }
        };
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
                    i64,
                    String,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                )> = sqlx::query_as(
                    "SELECT id, name, enabled, trigger_mode, rules_json, notification_group_id, fail_task_ids_json, recover_task_ids_json FROM alert_rules WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                for (
                    id,
                    name,
                    enabled,
                    trigger_mode,
                    rules_json,
                    notification_group_id,
                    fail_task_ids_json,
                    recover_task_ids_json,
                ) in rows
                {
                    out.push(AlertRule {
                        id,
                        name,
                        enabled: enabled != 0,
                        trigger_mode: TriggerMode::from_db(&trigger_mode),
                        conditions: serde_json::from_str(&rules_json)
                            .context("invalid rules_json")?,
                        notification_group_id,
                        failure_task_ids: parse_task_ids_json(fail_task_ids_json),
                        recovery_task_ids: parse_task_ids_json(recover_task_ids_json),
                    });
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(
                    String,
                    String,
                    bool,
                    String,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                )> = sqlx::query_as(
                    "SELECT id::text, name, enabled, trigger_mode, rules_json, notification_group_id::text, fail_task_ids_json, recover_task_ids_json FROM alert_rules WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                for (
                    id,
                    name,
                    enabled,
                    trigger_mode,
                    rules_json,
                    notification_group_id,
                    fail_task_ids_json,
                    recover_task_ids_json,
                ) in rows
                {
                    out.push(AlertRule {
                        id,
                        name,
                        enabled,
                        trigger_mode: TriggerMode::from_db(&trigger_mode),
                        conditions: serde_json::from_str(&rules_json)
                            .context("invalid rules_json")?,
                        notification_group_id,
                        failure_task_ids: parse_task_ids_json(fail_task_ids_json),
                        recovery_task_ids: parse_task_ids_json(recover_task_ids_json),
                    });
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
    let mut found: Option<&str> = None;
    for condition in &rule.conditions {
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

fn parse_task_ids_json(value: Option<String>) -> Vec<String> {
    value
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
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
}
