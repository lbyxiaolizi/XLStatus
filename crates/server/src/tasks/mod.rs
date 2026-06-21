pub mod scheduler;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use cron::Schedule;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use tracing::{info, warn};
use xlstatus_shared::tasks::*;

use crate::db::repository::tasks::{TaskRepository, TaskRunRepository};
use crate::db::Db;
use crate::grpc::{SessionRegistry, TaskResponseRegistry};
use crate::notifications::sender::{
    NotificationChannel, NotificationMessage, NotificationSender, NotificationSeverity,
};

pub(crate) const TASK_API_MAX_BODY_BYTES: usize = 256 * 1024;
pub(crate) const TASK_MAX_NAME_BYTES: usize = 128;
pub(crate) const TASK_MAX_SCHEDULE_BYTES: usize = 128;
pub(crate) const TASK_MAX_COMMAND_BYTES: usize = 8192;
pub(crate) const TASK_MAX_PAYLOAD_BYTES: usize = 64 * 1024;
pub(crate) const TASK_MAX_SELECTOR_BYTES: usize = 16 * 1024;
pub(crate) const TASK_MAX_SELECTOR_IDS: usize = 64;
pub(crate) const TASK_MAX_SELECTOR_TAGS: usize = 32;
pub(crate) const TASK_MAX_SELECTOR_TOKEN_BYTES: usize = 128;
pub(crate) const TASK_MAX_DISPATCH_TARGETS: usize = 64;

pub(crate) fn parse_task_schedule(schedule: &str) -> Result<Schedule> {
    let trimmed = schedule.trim();
    let field_count = trimmed.split_whitespace().count();
    let normalized = match field_count {
        5 => format!("0 {}", trimmed),
        6 | 7 => trimmed.to_string(),
        _ => return Err(anyhow!("Invalid cron expression")),
    };

    Schedule::from_str(&normalized).map_err(|err| anyhow!(err))
}

pub(crate) fn validate_task_definition(task: &Task) -> Result<()> {
    validate_sized_text(&task.name, 1, TASK_MAX_NAME_BYTES, "task.name")?;
    if let Some(schedule) = task.schedule.as_deref() {
        validate_sized_text(schedule, 1, TASK_MAX_SCHEDULE_BYTES, "task.schedule")?;
        parse_task_schedule(schedule).context("task.schedule is invalid")?;
    }
    if let Some(command) = task.command.as_deref() {
        let min = if task.task_type == TaskType::Shell {
            1
        } else {
            0
        };
        validate_sized_text(command, min, TASK_MAX_COMMAND_BYTES, "task.command")?;
    } else if task.task_type == TaskType::Shell {
        anyhow::bail!("task.command must be between 1 and {TASK_MAX_COMMAND_BYTES} bytes");
    }
    if let Some(payload) = task.payload_json.as_deref() {
        validate_sized_text(payload, 0, TASK_MAX_PAYLOAD_BYTES, "task.payload_json")?;
    }
    validate_sized_text(
        &task.server_selector_json,
        1,
        TASK_MAX_SELECTOR_BYTES,
        "task.server_selector_json",
    )?;
    let selector: ServerSelector = serde_json::from_str(&task.server_selector_json)
        .context("task.server_selector_json is invalid")?;
    validate_task_selector_shape(&selector)?;
    Ok(())
}

fn validate_sized_text(value: &str, min_bytes: usize, max_bytes: usize, field: &str) -> Result<()> {
    let len = value.len();
    if len < min_bytes || len > max_bytes {
        anyhow::bail!("{field} must be between {min_bytes} and {max_bytes} bytes");
    }
    Ok(())
}

fn validate_task_selector_shape(selector: &ServerSelector) -> Result<()> {
    ensure_selector_vec_allowed(&selector.server_ids, "server_ids")?;
    ensure_selector_vec_allowed(&selector.exclude_server_ids, "exclude_server_ids")?;
    ensure_selector_vec_allowed(&selector.group_ids, "group_ids")?;
    if selector.tag_names.len() > TASK_MAX_SELECTOR_TAGS {
        anyhow::bail!("tag_names contains too many entries");
    }
    for tag in &selector.tag_names {
        validate_sized_text(tag, 1, TASK_MAX_SELECTOR_TOKEN_BYTES, "tag_names entry")?;
    }
    if selector.tags.len() > TASK_MAX_SELECTOR_TAGS {
        anyhow::bail!("tags contains too many entries");
    }
    for (key, value) in &selector.tags {
        validate_sized_text(key, 1, TASK_MAX_SELECTOR_TOKEN_BYTES, "tags key")?;
        validate_sized_text(value, 0, TASK_MAX_SELECTOR_TOKEN_BYTES, "tags value")?;
    }
    Ok(())
}

fn ensure_selector_vec_allowed(values: &[String], field: &str) -> Result<()> {
    if values.len() > TASK_MAX_SELECTOR_IDS {
        anyhow::bail!("{field} contains too many entries");
    }
    for value in values {
        validate_sized_text(value, 1, TASK_MAX_SELECTOR_TOKEN_BYTES, field)?;
    }
    Ok(())
}

fn ensure_task_target_count_allowed(count: usize) -> Result<()> {
    if count > TASK_MAX_DISPATCH_TARGETS {
        anyhow::bail!(
            "task resolves to {count} target servers; maximum is {TASK_MAX_DISPATCH_TARGETS}"
        );
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct TaskDispatchSummary {
    pub success: usize,
    pub failure: usize,
    pub offline: usize,
    pub timeout: usize,
    pub total: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct TaskDispatchReport {
    pub task_id: String,
    pub summary: TaskDispatchSummary,
    pub runs: Vec<TaskRun>,
}

pub async fn dispatch_task_to_agents(
    db: &Db,
    session_registry: &SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    task: &Task,
) -> Result<TaskDispatchReport> {
    dispatch_task_to_agents_with_source(db, session_registry, response_registry, task, None).await
}

pub async fn dispatch_task_to_agents_with_source(
    db: &Db,
    session_registry: &SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    task: &Task,
    source_agent_id: Option<&str>,
) -> Result<TaskDispatchReport> {
    validate_task_definition(task).context("task definition is outside allowed safety limits")?;
    let selector: ServerSelector = serde_json::from_str(&task.server_selector_json)
        .context("task.server_selector_json is invalid")?;

    let shell_command = match task.task_type {
        TaskType::Shell => task.command.clone().unwrap_or_default(),
        _ => anyhow::bail!("Only Shell tasks can be dispatched today"),
    };
    if shell_command.trim().is_empty() {
        anyhow::bail!("Shell command is empty");
    }

    let target_agent_ids = resolve_server_ids(
        db,
        &selector,
        task.cover_mode,
        &task.owner_user_id,
        source_agent_id,
    )
    .await?;
    if target_agent_ids.is_empty() {
        anyhow::bail!("No target servers resolved from selector");
    }
    ensure_task_target_count_allowed(target_agent_ids.len())?;

    let mut runs = Vec::with_capacity(target_agent_ids.len());
    let mut success = 0usize;
    let mut failure = 0usize;
    let mut offline = 0usize;
    let mut timeout = 0usize;

    for agent_id_str in &target_agent_ids {
        let agent_uuid = match uuid::Uuid::parse_str(agent_id_str) {
            Ok(u) => u,
            Err(_) => {
                failure += 1;
                let run = create_run(
                    db,
                    &task.id,
                    agent_id_str,
                    TaskStatus::Failure,
                    0,
                    None,
                    false,
                    Some("invalid agent id".to_string()),
                )
                .await?;
                runs.push(run);
                continue;
            }
        };
        let agent_id = xlstatus_shared::AgentId(agent_uuid);

        let run_id = uuid::Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();

        if !session_registry.is_online(&agent_id).await {
            offline += 1;
            let run = TaskRun {
                id: run_id,
                task_id: task.id.clone(),
                server_id: agent_id_str.clone(),
                status: TaskStatus::Offline,
                delay_ms: Some(0),
                output: None,
                output_truncated: false,
                error: Some("agent offline".to_string()),
                created_at: now,
            };
            TaskRunRepository::create(db, &run).await?;
            runs.push(run);
            continue;
        }

        let rx = response_registry.register(run_id.clone()).await;
        if let Err(e) = session_registry
            .send_task(&agent_id, &run_id, &shell_command, 30)
            .await
        {
            response_registry.cancel(&run_id).await;
            failure += 1;
            let run = TaskRun {
                id: run_id,
                task_id: task.id.clone(),
                server_id: agent_id_str.clone(),
                status: TaskStatus::Failure,
                delay_ms: Some(0),
                output: None,
                output_truncated: false,
                error: Some(e),
                created_at: now,
            };
            TaskRunRepository::create(db, &run).await?;
            runs.push(run);
            continue;
        }

        let pending_run = TaskRun {
            id: run_id.clone(),
            task_id: task.id.clone(),
            server_id: agent_id_str.clone(),
            status: TaskStatus::Offline,
            delay_ms: Some(0),
            output: None,
            output_truncated: false,
            error: None,
            created_at: now.clone(),
        };
        TaskRunRepository::create(db, &pending_run).await?;

        let started = std::time::Instant::now();
        let final_run = match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => {
                let elapsed = started.elapsed().as_millis() as i64;
                use xlstatus_proto_gen::xlstatus::v1::TaskOutcome as Outcome;
                let mapped = match Outcome::try_from(result.status).unwrap_or(Outcome::Unspecified)
                {
                    Outcome::Success => {
                        success += 1;
                        TaskStatus::Success
                    }
                    Outcome::Failure => {
                        failure += 1;
                        TaskStatus::Failure
                    }
                    Outcome::Timeout => {
                        timeout += 1;
                        TaskStatus::Timeout
                    }
                    Outcome::Unspecified => {
                        failure += 1;
                        TaskStatus::Failure
                    }
                };
                let stderr = if result.stderr.is_empty() {
                    None
                } else {
                    Some(result.stderr)
                };
                let error = if !result.error.is_empty() {
                    Some(result.error)
                } else {
                    stderr
                };
                let output = if result.stdout.is_empty() {
                    None
                } else {
                    Some(result.stdout)
                };
                let output_truncated = output
                    .as_ref()
                    .map(|o| o.len() > 64 * 1024)
                    .unwrap_or(false);

                TaskRunRepository::update_result(
                    db,
                    &run_id,
                    mapped,
                    elapsed,
                    output.clone(),
                    output_truncated,
                    error.clone(),
                )
                .await?;

                TaskRun {
                    id: run_id,
                    task_id: task.id.clone(),
                    server_id: agent_id_str.clone(),
                    status: mapped,
                    delay_ms: Some(elapsed),
                    output,
                    output_truncated,
                    error,
                    created_at: now,
                }
            }
            Ok(Err(_canceled)) => {
                failure += 1;
                update_and_build_run(
                    db,
                    &run_id,
                    &task.id,
                    agent_id_str,
                    TaskStatus::Failure,
                    0,
                    None,
                    false,
                    Some("agent disconnected before reply".to_string()),
                    now,
                )
                .await?
            }
            Err(_elapsed) => {
                response_registry.cancel(&run_id).await;
                timeout += 1;
                update_and_build_run(
                    db,
                    &run_id,
                    &task.id,
                    agent_id_str,
                    TaskStatus::Timeout,
                    started.elapsed().as_millis() as i64,
                    None,
                    false,
                    Some("server-side timeout".to_string()),
                    now,
                )
                .await?
            }
        };
        runs.push(final_run);
    }

    let total = target_agent_ids.len();
    let last_result = format!(
        "success={}, failure={}, offline={}, timeout={}, total={}",
        success, failure, offline, timeout, total
    );
    TaskRepository::update_last_execution(db, &task.id, &Utc::now().to_rfc3339(), &last_result)
        .await?;

    let report = TaskDispatchReport {
        task_id: task.id.clone(),
        summary: TaskDispatchSummary {
            success,
            failure,
            offline,
            timeout,
            total,
        },
        runs,
    };

    if let Err(err) = send_task_dispatch_notification(db, task, &report).await {
        warn!("task {} notification failed: {}", task.id, err);
    }

    Ok(report)
}

pub fn spawn_triggered_tasks(
    db: Db,
    session_registry: SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    task_ids: Vec<String>,
    source: String,
    source_agent_id: Option<String>,
    owner_user_id: Option<String>,
) {
    let mut seen = std::collections::HashSet::new();
    for task_id in task_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
    {
        if !seen.insert(task_id.clone()) {
            continue;
        }
        let db = db.clone();
        let session_registry = session_registry.clone();
        let response_registry = response_registry.clone();
        let source = source.clone();
        let source_agent_id = source_agent_id.clone();
        let owner_user_id = owner_user_id.clone();
        tokio::spawn(async move {
            if let Err(err) = run_triggered_task(
                db,
                session_registry,
                response_registry,
                &task_id,
                &source,
                source_agent_id.as_deref(),
                owner_user_id.as_deref(),
            )
            .await
            {
                warn!("triggered task {} from {} failed: {}", task_id, source, err);
            }
        });
    }
}

async fn run_triggered_task(
    db: Db,
    session_registry: SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    task_id: &str,
    source: &str,
    source_agent_id: Option<&str>,
    owner_user_id: Option<&str>,
) -> Result<()> {
    let task = TaskRepository::get_by_id(&db, task_id)
        .await?
        .context("Task not found")?;
    if let Some(owner_user_id) = owner_user_id {
        if task.owner_user_id != owner_user_id {
            anyhow::bail!("Task owner does not match trigger owner");
        }
    }
    if !task.enabled {
        info!(
            "triggered task {} from {} skipped: disabled",
            task.id, source
        );
        return Ok(());
    }
    let report = dispatch_task_to_agents_with_source(
        &db,
        &session_registry,
        response_registry,
        &task,
        source_agent_id,
    )
    .await?;
    info!(
        "triggered task {} from {} completed: success={}, failure={}, offline={}, timeout={}, total={}",
        task.id,
        source,
        report.summary.success,
        report.summary.failure,
        report.summary.offline,
        report.summary.timeout,
        report.summary.total
    );
    Ok(())
}

async fn resolve_server_ids(
    db: &Db,
    selector: &ServerSelector,
    cover_mode: CoverMode,
    owner_user_id: &str,
    source_agent_id: Option<&str>,
) -> Result<Vec<String>> {
    let records = list_owned_agent_records(db, owner_user_id).await?;
    let owned_ids: HashSet<String> = records.iter().map(|record| record.id.clone()).collect();
    let mut candidates = Vec::new();
    let has_include_selector =
        selector.source_server || !selector.server_ids.is_empty() || !selector.group_ids.is_empty();

    if selector.source_server {
        if let Some(agent_id) = source_agent_id {
            if owned_ids.contains(agent_id) {
                candidates.push(agent_id.to_string());
            }
        }
    }

    for agent_id in &selector.server_ids {
        if owned_ids.contains(agent_id) {
            candidates.push(agent_id.clone());
        }
    }

    candidates.extend(list_group_agent_ids(db, owner_user_id, &selector.group_ids).await?);

    let tag_filters = normalized_selector_tags(selector);
    if !tag_filters.is_empty() {
        let tag_matched: HashSet<String> = records
            .iter()
            .filter(|record| agent_matches_tag_filters(record, &tag_filters))
            .map(|record| record.id.clone())
            .collect();
        if has_include_selector {
            candidates.retain(|agent_id| tag_matched.contains(agent_id));
        } else {
            candidates.extend(tag_matched);
        }
    } else if !has_include_selector {
        if matches!(cover_mode, CoverMode::All | CoverMode::Any) {
            candidates.extend(records.iter().map(|record| record.id.clone()));
        }
    }

    let excluded: HashSet<String> = selector.exclude_server_ids.iter().cloned().collect();
    let mut seen = HashSet::new();
    let mut resolved = Vec::new();
    for agent_id in candidates {
        if excluded.contains(&agent_id) || !owned_ids.contains(&agent_id) {
            continue;
        }
        if seen.insert(agent_id.clone()) {
            resolved.push(agent_id);
        }
    }

    if matches!(cover_mode, CoverMode::Any) {
        resolved.truncate(1);
    }

    Ok(resolved)
}

#[derive(Debug, Clone)]
struct AgentSelectorRecord {
    id: String,
    name: String,
    dashboard_metadata_json: Option<String>,
    last_info_json: Option<String>,
    last_state_json: Option<String>,
}

async fn list_owned_agent_records(
    db: &Db,
    owner_user_id: &str,
) -> Result<Vec<AgentSelectorRecord>> {
    use sqlx::Row;
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                "SELECT id, name, dashboard_metadata_json, last_info_json, last_state_json FROM agents WHERE owner_user_id = ? AND revoked_at IS NULL ORDER BY created_at ASC",
            )
            .bind(owner_user_id)
                .fetch_all(pool)
                .await?;
            Ok(rows
                .into_iter()
                .map(|row| AgentSelectorRecord {
                    id: row.try_get::<String, _>("id").unwrap_or_default(),
                    name: row.try_get::<String, _>("name").unwrap_or_default(),
                    dashboard_metadata_json: row.try_get("dashboard_metadata_json").ok(),
                    last_info_json: row.try_get("last_info_json").ok(),
                    last_state_json: row.try_get("last_state_json").ok(),
                })
                .collect())
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let owner_id = uuid::Uuid::parse_str(owner_user_id)?;
            let rows = sqlx::query(
                "SELECT id::text AS id, name, dashboard_metadata_json, last_info_json, last_state_json FROM agents WHERE owner_user_id = $1 AND revoked_at IS NULL ORDER BY created_at ASC",
            )
            .bind(owner_id)
                .fetch_all(pool)
                .await?;
            Ok(rows
                .into_iter()
                .map(|row| AgentSelectorRecord {
                    id: row.try_get::<String, _>("id").unwrap_or_default(),
                    name: row.try_get::<String, _>("name").unwrap_or_default(),
                    dashboard_metadata_json: row.try_get("dashboard_metadata_json").ok(),
                    last_info_json: row.try_get("last_info_json").ok(),
                    last_state_json: row.try_get("last_state_json").ok(),
                })
                .collect())
        }
    }
}

async fn list_group_agent_ids(
    db: &Db,
    owner_user_id: &str,
    group_ids: &[String],
) -> Result<Vec<String>> {
    if group_ids.is_empty() {
        return Ok(Vec::new());
    }

    use sqlx::Row;
    let mut out = Vec::new();
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            for group_id in group_ids {
                let rows = sqlx::query(
                    "SELECT sgm.agent_id FROM server_group_members sgm JOIN server_groups sg ON sg.id = sgm.group_id WHERE sg.id = ? AND sg.owner_user_id = ? ORDER BY sgm.created_at ASC",
                )
                .bind(group_id)
                .bind(owner_user_id)
                .fetch_all(pool)
                .await?;
                out.extend(
                    rows.into_iter()
                        .filter_map(|row| row.try_get::<String, _>("agent_id").ok()),
                );
            }
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let owner_id = uuid::Uuid::parse_str(owner_user_id)?;
            for group_id in group_ids {
                let Ok(group_uuid) = uuid::Uuid::parse_str(group_id) else {
                    continue;
                };
                let rows = sqlx::query(
                    "SELECT sgm.agent_id::text AS agent_id FROM server_group_members sgm JOIN server_groups sg ON sg.id = sgm.group_id WHERE sg.id = $1 AND sg.owner_user_id = $2 ORDER BY sgm.created_at ASC",
                )
                .bind(group_uuid)
                .bind(owner_id)
                .fetch_all(pool)
                .await?;
                out.extend(
                    rows.into_iter()
                        .filter_map(|row| row.try_get::<String, _>("agent_id").ok()),
                );
            }
        }
    }
    Ok(out)
}

fn normalized_selector_tags(selector: &ServerSelector) -> Vec<String> {
    let mut tags = selector
        .tag_names
        .iter()
        .filter_map(|tag| normalize_tag_filter(tag))
        .collect::<Vec<_>>();

    for (key, value) in &selector.tags {
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() || value == "*" || value.eq_ignore_ascii_case("true") {
            if let Some(tag) = normalize_tag_filter(key) {
                tags.push(tag);
            }
            continue;
        }
        if let Some(tag) = normalize_tag_filter(value) {
            tags.push(tag);
        }
        if let Some(tag) = normalize_tag_filter(&format!("{key}:{value}")) {
            tags.push(tag);
        }
    }

    let mut seen = HashSet::new();
    tags.into_iter()
        .filter(|tag| seen.insert(tag.clone()))
        .collect()
}

fn normalize_tag_filter(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_start_matches('#').to_lowercase();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn agent_matches_tag_filters(record: &AgentSelectorRecord, filters: &[String]) -> bool {
    let tags = agent_tags(record);
    filters.iter().all(|filter| tags.contains(filter))
}

fn agent_tags(record: &AgentSelectorRecord) -> HashSet<String> {
    let mut out = HashSet::new();
    for tag in record
        .dashboard_metadata_json
        .as_deref()
        .into_iter()
        .chain(record.last_info_json.as_deref())
        .chain(record.last_state_json.as_deref())
        .filter_map(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .flat_map(|value| tags_from_json_source(&value))
    {
        if let Some(tag) = normalize_tag_filter(&tag) {
            out.insert(tag);
        }
    }
    if let Some(tag) = normalize_tag_filter(&record.name) {
        out.insert(tag);
    }
    out
}

fn tags_from_json_source(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["tags", "labels"] {
        if let Some(value) = value.get(key) {
            out.extend(tags_from_json_value(value));
        }
    }
    for container in ["metadata", "custom"] {
        if let Some(child) = value.get(container) {
            for key in ["tags", "labels"] {
                if let Some(value) = child.get(key) {
                    out.extend(tags_from_json_value(value));
                }
            }
        }
    }
    out
}

fn tags_from_json_value(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        serde_json::Value::String(value) => value
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        serde_json::Value::Object(map) => map
            .iter()
            .flat_map(|(key, value)| {
                if let Some(label) = value.as_str() {
                    vec![label.to_string(), format!("{key}:{label}")]
                } else if value.as_bool() == Some(true) {
                    vec![key.to_string()]
                } else {
                    Vec::new()
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

async fn send_task_dispatch_notification(
    db: &Db,
    task: &Task,
    report: &TaskDispatchReport,
) -> Result<()> {
    let Some(group_id) = task.notification_group_id.as_deref() else {
        return Ok(());
    };
    let has_failure =
        report.summary.failure > 0 || report.summary.offline > 0 || report.summary.timeout > 0;
    if !has_failure && !task.push_successful {
        return Ok(());
    }

    let channels = notification_channels_for_group(db, group_id, &task.owner_user_id).await?;
    if channels.is_empty() {
        return Ok(());
    }

    let result = if has_failure { "failure" } else { "success" };
    let severity = if has_failure {
        NotificationSeverity::Error
    } else {
        NotificationSeverity::Info
    };
    let mut metadata = HashMap::new();
    metadata.insert("task_id".to_string(), task.id.clone());
    metadata.insert("task_name".to_string(), task.name.clone());
    metadata.insert("result".to_string(), result.to_string());
    metadata.insert("success".to_string(), report.summary.success.to_string());
    metadata.insert("failure".to_string(), report.summary.failure.to_string());
    metadata.insert("offline".to_string(), report.summary.offline.to_string());
    metadata.insert("timeout".to_string(), report.summary.timeout.to_string());
    metadata.insert("total".to_string(), report.summary.total.to_string());

    let message = NotificationMessage {
        title: format!(
            "任务执行{}：{}",
            if has_failure { "异常" } else { "成功" },
            task.name
        ),
        message: format!(
            "任务 {} 执行完成：success={}, failure={}, offline={}, timeout={}, total={}",
            task.name,
            report.summary.success,
            report.summary.failure,
            report.summary.offline,
            report.summary.timeout,
            report.summary.total
        ),
        severity,
        timestamp: Utc::now().to_rfc3339(),
        metadata,
    };

    let sender = Arc::new(NotificationSender::new());
    for channel in channels {
        let sender = sender.clone();
        let message = message.clone();
        tokio::spawn(async move {
            if let Err(err) = sender.send(&channel, &message).await {
                warn!("task notification send failed: {}", err);
            }
        });
    }
    Ok(())
}

async fn notification_channels_for_group(
    db: &Db,
    group_id: &str,
    owner_user_id: &str,
) -> Result<Vec<NotificationChannel>> {
    let rows: Vec<(
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        bool,
    )> = match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            sqlx::query_as("SELECT n.id, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls FROM notifications n JOIN notification_group_members ngm ON ngm.notification_id = n.id JOIN notification_groups ng ON ng.id = ngm.group_id WHERE ngm.group_id = ? AND ng.owner_user_id = ? AND n.owner_user_id = ?")
                .bind(group_id)
                .bind(owner_user_id)
                .bind(owner_user_id)
                .fetch_all(pool)
                .await?
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let group_uuid = uuid::Uuid::parse_str(group_id)?;
            let owner_uuid = uuid::Uuid::parse_str(owner_user_id)?;
            sqlx::query_as("SELECT n.id::text, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls FROM notifications n JOIN notification_group_members ngm ON ngm.notification_id = n.id JOIN notification_groups ng ON ng.id = ngm.group_id WHERE ngm.group_id = $1 AND ng.owner_user_id = $2 AND n.owner_user_id = $2")
                .bind(group_uuid)
                .bind(owner_uuid)
                .fetch_all(pool)
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                name,
                url,
                request_method,
                request_type,
                headers_json,
                body_template,
                verify_tls,
            )| {
                let headers: HashMap<String, String> = headers_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok())
                    .unwrap_or_default();
                NotificationChannel {
                    id,
                    name,
                    url,
                    request_method,
                    request_type,
                    headers,
                    body_template: body_template.unwrap_or_default(),
                    verify_tls,
                }
            },
        )
        .collect())
}

async fn create_run(
    db: &Db,
    task_id: &str,
    server_id: &str,
    status: TaskStatus,
    delay_ms: i64,
    output: Option<String>,
    output_truncated: bool,
    error: Option<String>,
) -> Result<TaskRun> {
    let run = TaskRun {
        id: uuid::Uuid::now_v7().to_string(),
        task_id: task_id.to_string(),
        server_id: server_id.to_string(),
        status,
        delay_ms: Some(delay_ms),
        output,
        output_truncated,
        error,
        created_at: Utc::now().to_rfc3339(),
    };
    TaskRunRepository::create(db, &run).await?;
    Ok(run)
}

async fn update_and_build_run(
    db: &Db,
    run_id: &str,
    task_id: &str,
    server_id: &str,
    status: TaskStatus,
    delay_ms: i64,
    output: Option<String>,
    output_truncated: bool,
    error: Option<String>,
    created_at: String,
) -> Result<TaskRun> {
    TaskRunRepository::update_result(
        db,
        run_id,
        status,
        delay_ms,
        output.clone(),
        output_truncated,
        error.clone(),
    )
    .await?;

    Ok(TaskRun {
        id: run_id.to_string(),
        task_id: task_id.to_string(),
        server_id: server_id.to_string(),
        status,
        delay_ms: Some(delay_ms),
        output,
        output_truncated,
        error,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlstatus_shared::tasks::{CoverMode, ServerSelector, Task, TaskType};

    #[tokio::test]
    async fn triggered_task_requires_matching_owner_when_provided() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_task(
            &db,
            "00000000-0000-0000-0000-000000000301",
            other,
            "other-task",
        )
        .await;

        let err = run_triggered_task(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
            "00000000-0000-0000-0000-000000000301",
            "alert:dirty",
            None,
            Some(owner),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("owner does not match"));
    }

    #[test]
    fn task_definition_bounds_command_and_selector() {
        let mut task = sample_task();
        task.command = Some("x".repeat(TASK_MAX_COMMAND_BYTES + 1));
        assert!(validate_task_definition(&task)
            .unwrap_err()
            .to_string()
            .contains("task.command"));

        let mut task = sample_task();
        task.server_selector_json = serde_json::to_string(&ServerSelector {
            server_ids: (0..=TASK_MAX_SELECTOR_IDS)
                .map(|idx| format!("server-{idx}"))
                .collect(),
            ..ServerSelector::default()
        })
        .unwrap();
        assert!(validate_task_definition(&task)
            .unwrap_err()
            .to_string()
            .contains("too many entries"));
    }

    #[test]
    fn task_dispatch_target_count_is_bounded() {
        assert!(ensure_task_target_count_allowed(TASK_MAX_DISPATCH_TARGETS).is_ok());
        let err = ensure_task_target_count_allowed(TASK_MAX_DISPATCH_TARGETS + 1).unwrap_err();
        assert!(err.to_string().contains("maximum"));
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

    async fn seed_task(db: &Db, id: &str, owner: &str, name: &str) {
        let mut task = sample_task();
        task.id = id.to_string();
        task.owner_user_id = owner.to_string();
        task.name = name.to_string();
        TaskRepository::create(db, &task).await.unwrap();
    }

    fn sample_task() -> Task {
        Task {
            id: "00000000-0000-0000-0000-000000000101".to_string(),
            owner_user_id: "00000000-0000-0000-0000-000000000001".to_string(),
            name: "task".to_string(),
            task_type: TaskType::Shell,
            schedule: None,
            command: Some("true".to_string()),
            payload_json: None,
            cover_mode: CoverMode::Specific,
            server_selector_json: serde_json::to_string(&ServerSelector {
                server_ids: Vec::new(),
                group_ids: Vec::new(),
                tag_names: Vec::new(),
                tags: HashMap::new(),
                exclude_server_ids: Vec::new(),
                source_server: false,
            })
            .unwrap(),
            push_successful: false,
            notification_group_id: None,
            last_executed_at: None,
            last_result: None,
            enabled: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }
}
