//! M4 alert engine.
//!
//! See `docs/implementation-audit.md` for the full design notes.

use crate::db::Db;
use crate::notifications::sender::{
    NotificationChannel, NotificationMessage, NotificationSender, NotificationSeverity,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
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
    Load,
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
    states: Arc<RwLock<HashMap<String, AlertState>>>,
    condition_windows: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    traffic_windows: Arc<RwLock<HashMap<String, TrafficWindow>>>,
    sender: Arc<NotificationSender>,
    latest: Arc<RwLock<HashMap<String, serde_json::Value>>>,
}

impl AlertEngine {
    pub fn new(db: Db) -> Self {
        Self {
            db,
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
        for c in &rule.conditions {
            if self.check_condition(c).await? {
                any_triggered = true;
                break;
            }
        }

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
                    self.fire(rule, "fired").await?;
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
                    self.fire(rule, "recovered").await?;
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
                        self.fire(rule, "fired").await?;
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
                    self.fire(rule, "recovered").await?;
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

    async fn fire(&self, rule: &AlertRule, kind: &str) -> Result<()> {
        let id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now();
        let payload = serde_json::json!({
            "rule_id": rule.id,
            "rule_name": rule.name,
            "kind": kind,
            "fired_at": now.to_rfc3339(),
        });
        let payload_text = serde_json::to_string(&payload)?;
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO alert_events (id, rule_id, kind, payload_json, fired_at) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&rule.id)
                .bind(kind)
                .bind(&payload_text)
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let pid = uuid::Uuid::parse_str(&id)?;
                let prid = uuid::Uuid::parse_str(&rule.id)?;
                sqlx::query(
                    "INSERT INTO alert_events (id, rule_id, kind, payload_json, fired_at) VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(pid)
                .bind(prid)
                .bind(kind)
                .bind(&payload_text)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }
        info!("alert {} event for rule {} ({})", kind, rule.id, rule.name);

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
                let png = uuid::Uuid::parse_str(ng)?;
                sqlx::query_as("SELECT n.id::text, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls FROM notifications n JOIN notification_group_members ngm ON ngm.notification_id = n.id WHERE ngm.group_id = $1")
                    .bind(png)
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
                let rows: Vec<(String, String, i64, String, String, Option<String>)> = sqlx::query_as(
                    "SELECT id, name, enabled, trigger_mode, rules_json, notification_group_id FROM alert_rules WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                for (id, name, enabled, trigger_mode, rules_json, notification_group_id) in rows {
                    out.push(AlertRule {
                        id,
                        name,
                        enabled: enabled != 0,
                        trigger_mode: TriggerMode::from_db(&trigger_mode),
                        conditions: serde_json::from_str(&rules_json)
                            .context("invalid rules_json")?,
                        notification_group_id,
                    });
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(String, String, bool, String, String, Option<String>)> = sqlx::query_as(
                    "SELECT id::text, name, enabled, trigger_mode, rules_json, notification_group_id::text FROM alert_rules WHERE enabled = 1",
                )
                .fetch_all(pool)
                .await?;
                for (id, name, enabled, trigger_mode, rules_json, notification_group_id) in rows {
                    out.push(AlertRule {
                        id,
                        name,
                        enabled,
                        trigger_mode: TriggerMode::from_db(&trigger_mode),
                        conditions: serde_json::from_str(&rules_json)
                            .context("invalid rules_json")?,
                        notification_group_id,
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
        ResourceType::Load => state.get("load_1").and_then(|v| v.as_f64()),
    }
}

fn network_total_bytes(state: &serde_json::Value) -> Option<u64> {
    let net_io = state.get("net_io").and_then(|v| v.as_array())?;
    let mut total = 0u64;
    for nic in net_io {
        total = total.saturating_add(
            nic.get("bytes_sent")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
        );
        total = total.saturating_add(
            nic.get("bytes_recv")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
        );
    }
    Some(total)
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

    #[tokio::test]
    async fn observe_and_read_latest() {
        let pool = sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let db = Db::Sqlite(pool);
        let engine = Arc::new(AlertEngine::new(db));
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
