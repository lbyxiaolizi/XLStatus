use crate::db::Db;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Alert rule condition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger_mode: TriggerMode,
    pub conditions: Vec<AlertCondition>,
    pub notification_group_id: Option<String>,
}

/// Trigger mode for alerts
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    /// Trigger every time condition is met
    Always,
    /// Trigger only once until recovered
    Once,
}

/// Alert condition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AlertCondition {
    /// Service is down
    ServiceDown {
        service_id: String,
        consecutive_failures: u32,
    },
    /// Service latency exceeds threshold
    ServiceLatency {
        service_id: String,
        max_latency_ms: i32,
    },
    /// Server is offline
    ServerOffline {
        server_id: String,
        offline_seconds: u64,
    },
    /// Server resource usage
    ServerResource {
        server_id: String,
        resource: ResourceType,
        operator: Operator,
        threshold: f64,
        duration_seconds: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Cpu,
    Memory,
    Disk,
    Network,
    Load,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    Gt,  // Greater than
    Lt,  // Less than
    Gte, // Greater than or equal
    Lte, // Less than or equal
}

/// Alert state tracking
#[derive(Debug, Clone)]
struct AlertState {
    rule_id: String,
    last_triggered_at: Option<chrono::DateTime<chrono::Utc>>,
    is_active: bool,
}

/// Alert engine
pub struct AlertEngine {
    db: Db,
    // Track alert states for "once" trigger mode
    alert_states: Arc<RwLock<HashMap<String, AlertState>>>,
}

impl AlertEngine {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            alert_states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Evaluate all alert rules
    pub async fn evaluate_all(&self) -> Result<()> {
        let rules = self.load_alert_rules().await?;

        for rule in rules {
            if !rule.enabled {
                continue;
            }

            if let Err(e) = self.evaluate_rule(&rule).await {
                warn!("Failed to evaluate alert rule {}: {}", rule.id, e);
            }
        }

        Ok(())
    }

    /// Evaluate a single alert rule
    async fn evaluate_rule(&self, rule: &AlertRule) -> Result<()> {
        let mut any_triggered = false;

        for condition in &rule.conditions {
            let triggered = self.check_condition(condition).await?;

            if triggered {
                any_triggered = true;
                break; // Any condition triggers the alert
            }
        }

        // Check if we should trigger based on trigger mode
        let should_trigger = match rule.trigger_mode {
            TriggerMode::Always => any_triggered,
            TriggerMode::Once => {
                let states = self.alert_states.read().await;
                match states.get(&rule.id) {
                    Some(state) if state.is_active => false, // Already active
                    _ => any_triggered,
                }
            }
        };

        if should_trigger {
            self.trigger_alert(rule).await?;

            // Update state
            let mut states = self.alert_states.write().await;
            states.insert(
                rule.id.clone(),
                AlertState {
                    rule_id: rule.id.clone(),
                    last_triggered_at: Some(chrono::Utc::now()),
                    is_active: true,
                },
            );
        } else if !any_triggered {
            // Check if we need to send recovery notification
            let was_active = {
                let states = self.alert_states.read().await;
                states.get(&rule.id).map(|s| s.is_active).unwrap_or(false)
            };

            if was_active {
                self.trigger_recovery(rule).await?;

                // Clear state
                let mut states = self.alert_states.write().await;
                states.insert(
                    rule.id.clone(),
                    AlertState {
                        rule_id: rule.id.clone(),
                        last_triggered_at: None,
                        is_active: false,
                    },
                );
            }
        }

        Ok(())
    }

    /// Check if a condition is met
    async fn check_condition(&self, condition: &AlertCondition) -> Result<bool> {
        match condition {
            AlertCondition::ServiceDown {
                service_id,
                consecutive_failures,
            } => {
                let query = r#"
                    SELECT status
                    FROM service_results
                    WHERE service_id = ?
                    ORDER BY created_at DESC
                    LIMIT ?
                "#;

                let is_down = match &self.db {
                    crate::db::DatabaseBackend::Sqlite(pool) => {
                        let rows = sqlx::query(query)
                            .bind(service_id)
                            .bind(*consecutive_failures as i64)
                            .fetch_all(pool)
                            .await?;

                        if rows.len() < *consecutive_failures as usize {
                            false
                        } else {
                            rows.iter().all(|row| {
                                row.try_get::<String, _>("status")
                                    .map(|s| s == "failure")
                                    .unwrap_or(false)
                            })
                        }
                    }
                    crate::db::DatabaseBackend::Postgres(pool) => {
                        let rows = sqlx::query(query)
                            .bind(service_id)
                            .bind(*consecutive_failures as i64)
                            .fetch_all(pool)
                            .await?;

                        if rows.len() < *consecutive_failures as usize {
                            false
                        } else {
                            rows.iter().all(|row| {
                                row.try_get::<String, _>("status")
                                    .map(|s| s == "failure")
                                    .unwrap_or(false)
                            })
                        }
                    }
                };

                Ok(is_down)
            }

            AlertCondition::ServiceLatency {
                service_id,
                max_latency_ms,
            } => {
                let query = r#"
                    SELECT delay_ms
                    FROM service_results
                    WHERE service_id = ? AND status = 'success'
                    ORDER BY created_at DESC
                    LIMIT 1
                "#;

                let exceeds = match &self.db {
                    crate::db::DatabaseBackend::Sqlite(pool) => {
                        match sqlx::query(query)
                            .bind(service_id)
                            .fetch_optional(pool)
                            .await?
                        {
                            Some(row) => {
                                let latency: Option<i32> = row.try_get("delay_ms")?;
                                latency.map(|l| l > *max_latency_ms).unwrap_or(false)
                            }
                            None => false,
                        }
                    }
                    crate::db::DatabaseBackend::Postgres(pool) => {
                        match sqlx::query(query)
                            .bind(service_id)
                            .fetch_optional(pool)
                            .await?
                        {
                            Some(row) => {
                                let latency: Option<i32> = row.try_get("delay_ms")?;
                                latency.map(|l| l > *max_latency_ms).unwrap_or(false)
                            }
                            None => false,
                        }
                    }
                };

                Ok(exceeds)
            }

            AlertCondition::ServerOffline {
                server_id,
                offline_seconds,
            } => {
                let query = r#"
                    SELECT last_seen_at
                    FROM servers
                    WHERE id = ?
                "#;

                let is_offline = match &self.db {
                    crate::db::DatabaseBackend::Sqlite(pool) => {
                        match sqlx::query(query)
                            .bind(server_id)
                            .fetch_optional(pool)
                            .await?
                        {
                            Some(row) => {
                                let last_seen: Option<String> = row.try_get("last_seen_at")?;
                                match last_seen {
                                    Some(last_seen) => {
                                        let last_seen_time =
                                            chrono::DateTime::parse_from_rfc3339(&last_seen)?;
                                        let now = chrono::Utc::now();
                                        let elapsed = (now.timestamp() - last_seen_time.timestamp()).abs();
                                        elapsed > *offline_seconds as i64
                                    }
                                    None => true,
                                }
                            }
                            None => false,
                        }
                    }
                    crate::db::DatabaseBackend::Postgres(pool) => {
                        match sqlx::query(query)
                            .bind(server_id)
                            .fetch_optional(pool)
                            .await?
                        {
                            Some(row) => {
                                let last_seen: Option<String> = row.try_get("last_seen_at")?;
                                match last_seen {
                                    Some(last_seen) => {
                                        let last_seen_time =
                                            chrono::DateTime::parse_from_rfc3339(&last_seen)?;
                                        let now = chrono::Utc::now();
                                        let elapsed = (now.timestamp() - last_seen_time.timestamp()).abs();
                                        elapsed > *offline_seconds as i64
                                    }
                                    None => true,
                                }
                            }
                            None => false,
                        }
                    }
                };

                Ok(is_offline)
            }

            AlertCondition::ServerResource { .. } => {
                // TODO: Implement resource condition checking
                Ok(false)
            }
        }
    }

    /// Trigger an alert
    async fn trigger_alert(&self, rule: &AlertRule) -> Result<()> {
        info!("Alert triggered: {} ({})", rule.id, rule.name);
        // TODO: Send notification via notification system
        Ok(())
    }

    /// Trigger a recovery notification
    async fn trigger_recovery(&self, rule: &AlertRule) -> Result<()> {
        info!("Alert recovered: {} ({})", rule.id, rule.name);
        // TODO: Send recovery notification
        Ok(())
    }

    /// Load alert rules from database
    async fn load_alert_rules(&self) -> Result<Vec<AlertRule>> {
        let query = r#"
            SELECT id, name, enabled, trigger_mode, rules_json,
                   notification_group_id
            FROM alert_rules
            WHERE enabled = 1
        "#;

        let rules = match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                let mut rules = Vec::new();
                for row in rows {
                    let trigger_mode_str: String = row.try_get("trigger_mode")?;
                    let trigger_mode = match trigger_mode_str.as_str() {
                        "always" => TriggerMode::Always,
                        "once" => TriggerMode::Once,
                        _ => TriggerMode::Once,
                    };

                    let rules_json: String = row.try_get("rules_json")?;
                    let conditions: Vec<AlertCondition> = serde_json::from_str(&rules_json)?;

                    rules.push(AlertRule {
                        id: row.try_get("id")?,
                        name: row.try_get("name")?,
                        enabled: row.try_get("enabled")?,
                        trigger_mode,
                        conditions,
                        notification_group_id: row.try_get("notification_group_id")?,
                    });
                }
                rules
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                let mut rules = Vec::new();
                for row in rows {
                    let trigger_mode_str: String = row.try_get("trigger_mode")?;
                    let trigger_mode = match trigger_mode_str.as_str() {
                        "always" => TriggerMode::Always,
                        "once" => TriggerMode::Once,
                        _ => TriggerMode::Once,
                    };

                    let rules_json: String = row.try_get("rules_json")?;
                    let conditions: Vec<AlertCondition> = serde_json::from_str(&rules_json)?;

                    rules.push(AlertRule {
                        id: row.try_get("id")?,
                        name: row.try_get("name")?,
                        enabled: row.try_get("enabled")?,
                        trigger_mode,
                        conditions,
                        notification_group_id: row.try_get("notification_group_id")?,
                    });
                }
                rules
            }
        };

        Ok(rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigger_mode() {
        let always = TriggerMode::Always;
        let once = TriggerMode::Once;

        assert!(matches!(always, TriggerMode::Always));
        assert!(matches!(once, TriggerMode::Once));
    }

    #[test]
    fn test_condition_serialization() {
        let condition = AlertCondition::ServiceDown {
            service_id: "test".to_string(),
            consecutive_failures: 3,
        };

        let json = serde_json::to_string(&condition).unwrap();
        let deserialized: AlertCondition = serde_json::from_str(&json).unwrap();

        match deserialized {
            AlertCondition::ServiceDown {
                consecutive_failures,
                ..
            } => {
                assert_eq!(consecutive_failures, 3);
            }
            _ => panic!("Wrong condition type"),
        }
    }
}
