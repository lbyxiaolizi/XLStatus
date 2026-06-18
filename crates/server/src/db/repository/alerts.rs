//! M4 / M6 repository for `alert_rules`, `alert_events`. We split
//! SQLite and PG queries because PG needs `::text` casts on UUID
//! columns but SQLite chokes on the same syntax.

use crate::alerts::engine::{AlertCondition, TriggerMode};
use crate::db::{DatabaseBackend, Db};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AlertRuleRow {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger_mode: TriggerMode,
    pub conditions: Vec<AlertCondition>,
    pub notification_group_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct AlertRepository {
    db: Db,
}

impl AlertRepository {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        owner_user_id: &str,
        name: &str,
        trigger_mode: TriggerMode,
        conditions: &[AlertCondition],
        notification_group_id: Option<&str>,
    ) -> Result<AlertRuleRow> {
        let id = Uuid::now_v7().to_string();
        let now = Utc::now();
        let conditions_json = serde_json::to_string(conditions)?;
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO alert_rules (id, owner_user_id, name, enabled, trigger_mode, rules_json, notification_group_id, created_at, updated_at) VALUES (?, ?, ?, 1, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(owner_user_id)
                .bind(name)
                .bind(trigger_mode.as_db())
                .bind(&conditions_json)
                .bind(notification_group_id)
                .bind(now.to_rfc3339())
                .bind(now.to_rfc3339())
                .execute(pool)
                .await?;
            }
            DatabaseBackend::Postgres(pool) => {
                let pid = Uuid::parse_str(&id)?;
                let poid = Uuid::parse_str(owner_user_id)?;
                let png = notification_group_id.map(Uuid::parse_str).transpose()?;
                sqlx::query(
                    "INSERT INTO alert_rules (id, owner_user_id, name, enabled, trigger_mode, rules_json, notification_group_id, created_at, updated_at) VALUES ($1, $2, $3, true, $4, $5, $6, $7, $8)",
                )
                .bind(pid)
                .bind(poid)
                .bind(name)
                .bind(trigger_mode.as_db())
                .bind(&conditions_json)
                .bind(png)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }
        Ok(AlertRuleRow {
            id,
            owner_user_id: owner_user_id.to_string(),
            name: name.to_string(),
            enabled: true,
            trigger_mode,
            conditions: conditions.to_vec(),
            notification_group_id: notification_group_id.map(|s| s.to_string()),
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn list(&self) -> Result<Vec<AlertRuleRow>> {
        let mut out = Vec::new();
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let rows: Vec<(
                    String, String, String, i64, String, String, Option<String>, String, String,
                )> = sqlx::query_as(
                    "SELECT id, owner_user_id, name, enabled, trigger_mode, rules_json, notification_group_id, created_at, updated_at FROM alert_rules ORDER BY created_at DESC",
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
                    created_at,
                    updated_at,
                ) in rows
                {
                    out.push(AlertRuleRow {
                        id,
                        owner_user_id,
                        name,
                        enabled: enabled != 0,
                        trigger_mode: TriggerMode::from_db(&trigger_mode),
                        conditions: serde_json::from_str(&rules_json)
                            .context("invalid rules_json")?,
                        notification_group_id,
                        created_at: parse_dt(&created_at)?,
                        updated_at: parse_dt(&updated_at)?,
                    });
                }
            }
            DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(
                    String, String, String, bool, String, String, Option<String>, String, String,
                )> = sqlx::query_as(
                    "SELECT id::text, owner_user_id::text, name, enabled, trigger_mode, rules_json, notification_group_id::text, created_at::text, updated_at::text FROM alert_rules ORDER BY created_at DESC",
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
                    created_at,
                    updated_at,
                ) in rows
                {
                    out.push(AlertRuleRow {
                        id,
                        owner_user_id,
                        name,
                        enabled,
                        trigger_mode: TriggerMode::from_db(&trigger_mode),
                        conditions: serde_json::from_str(&rules_json)
                            .context("invalid rules_json")?,
                        notification_group_id,
                        created_at: parse_dt(&created_at)?,
                        updated_at: parse_dt(&updated_at)?,
                    });
                }
            }
        }
        Ok(out)
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let q = "DELETE FROM alert_rules WHERE id = $1";
        let affected = match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(q).bind(id).execute(pool).await?.rows_affected()
            }
            DatabaseBackend::Postgres(pool) => {
                let pid = Uuid::parse_str(id)?;
                sqlx::query(q)
                    .bind(pid)
                    .execute(pool)
                    .await?
                    .rows_affected()
            }
        };
        Ok(affected > 0)
    }
}

#[derive(Debug, Clone)]
pub struct AlertEventRow {
    pub id: String,
    pub rule_id: String,
    pub agent_id: Option<String>,
    pub service_id: Option<String>,
    pub kind: String,
    pub payload_json: String,
    pub fired_at: DateTime<Utc>,
}

pub struct AlertEventRepository {
    db: Db,
}

impl AlertEventRepository {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn list_recent(&self, limit: i64) -> Result<Vec<AlertEventRow>> {
        let mut out = Vec::new();
        match &self.db {
            DatabaseBackend::Sqlite(pool) => {
                let rows: Vec<(String, String, Option<String>, Option<String>, String, String, String)> = sqlx::query_as(
                    "SELECT id, rule_id, agent_id, service_id, kind, payload_json, fired_at FROM alert_events ORDER BY fired_at DESC LIMIT ?",
                )
                .bind(limit)
                .fetch_all(pool)
                .await?;
                for (id, rule_id, agent_id, service_id, kind, payload_json, fired_at) in rows {
                    out.push(AlertEventRow {
                        id,
                        rule_id,
                        agent_id,
                        service_id,
                        kind,
                        payload_json,
                        fired_at: parse_dt(&fired_at)?,
                    });
                }
            }
            DatabaseBackend::Postgres(pool) => {
                let rows: Vec<(String, String, Option<String>, Option<String>, String, String, String)> = sqlx::query_as(
                    "SELECT id::text, rule_id::text, agent_id::text, service_id::text, kind, payload_json, fired_at::text FROM alert_events ORDER BY fired_at DESC LIMIT $1",
                )
                .bind(limit)
                .fetch_all(pool)
                .await?;
                for (id, rule_id, agent_id, service_id, kind, payload_json, fired_at) in rows {
                    out.push(AlertEventRow {
                        id,
                        rule_id,
                        agent_id,
                        service_id,
                        kind,
                        payload_json,
                        fired_at: parse_dt(&fired_at)?,
                    });
                }
            }
        }
        Ok(out)
    }
}

fn parse_dt(s: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::engine::{Operator, ResourceType};

    #[test]
    fn alert_condition_serde() {
        let rule = AlertRuleRow {
            id: "x".into(),
            owner_user_id: "u".into(),
            name: "cpu".into(),
            enabled: true,
            trigger_mode: TriggerMode::Once,
            conditions: vec![AlertCondition::ServerResource {
                agent_id: "a1".into(),
                resource: ResourceType::Cpu,
                operator: Operator::Gt,
                threshold: 80.0,
                duration_seconds: 0,
            }],
            notification_group_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let j = serde_json::to_string(&rule.conditions).unwrap();
        let back: Vec<AlertCondition> = serde_json::from_str(&j).unwrap();
        assert_eq!(back.len(), 1);
    }
}
