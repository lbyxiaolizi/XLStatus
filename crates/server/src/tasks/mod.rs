pub mod scheduler;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use cron::Schedule;
use std::str::FromStr;
use std::sync::Arc;
use xlstatus_shared::tasks::*;

use crate::db::repository::tasks::{TaskRepository, TaskRunRepository};
use crate::db::Db;
use crate::grpc::{SessionRegistry, TaskResponseRegistry};

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
    let selector: ServerSelector = serde_json::from_str(&task.server_selector_json)
        .context("task.server_selector_json is invalid")?;

    let shell_command = match task.task_type {
        TaskType::Shell => task.command.clone().unwrap_or_default(),
        _ => anyhow::bail!("Only Shell tasks can be dispatched today"),
    };
    if shell_command.trim().is_empty() {
        anyhow::bail!("Shell command is empty");
    }

    let target_agent_ids = resolve_server_ids(db, &selector, task.cover_mode).await?;
    if target_agent_ids.is_empty() {
        anyhow::bail!("No target servers resolved from selector");
    }

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

    Ok(TaskDispatchReport {
        task_id: task.id.clone(),
        summary: TaskDispatchSummary {
            success,
            failure,
            offline,
            timeout,
            total,
        },
        runs,
    })
}

async fn resolve_server_ids(
    db: &Db,
    selector: &ServerSelector,
    cover_mode: CoverMode,
) -> Result<Vec<String>> {
    if !selector.server_ids.is_empty() {
        return Ok(match cover_mode {
            CoverMode::Any => selector.server_ids.iter().take(1).cloned().collect(),
            CoverMode::All | CoverMode::Specific => selector.server_ids.clone(),
        });
    }

    match cover_mode {
        CoverMode::Specific => Ok(Vec::new()),
        CoverMode::All => list_all_agent_ids(db).await,
        CoverMode::Any => Ok(list_all_agent_ids(db).await?.into_iter().take(1).collect()),
    }
}

async fn list_all_agent_ids(db: &Db) -> Result<Vec<String>> {
    use sqlx::Row;
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query("SELECT id FROM agents WHERE revoked_at IS NULL")
                .fetch_all(pool)
                .await?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("id").ok())
                .collect())
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query("SELECT id::text AS id FROM agents WHERE revoked_at IS NULL")
                .fetch_all(pool)
                .await?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("id").ok())
                .collect())
        }
    }
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
