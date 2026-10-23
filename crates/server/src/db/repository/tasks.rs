#![allow(dead_code)]
#![allow(unused)]

use crate::db::Db;
use crate::tasks::{validate_task_definition, TASK_SCHEDULED_RUNTIME_SCAN_LIMIT};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use tracing::warn;
use xlstatus_shared::tasks::*;

const TASK_REPO_MAX_LIMIT: i64 = 500;

pub struct TaskRepository;

#[derive(Debug)]
pub(crate) struct ScheduledTaskPage {
    pub tasks: Vec<Task>,
    pub next_cursor: Option<String>,
}

impl TaskRepository {
    /// Create a new task
    pub async fn create(db: &Db, task: &Task) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(TASK_INSERT_SQLITE)
                    .bind(&task.id)
                    .bind(&task.owner_user_id)
                    .bind(&task.name)
                    .bind(serde_json::to_string(&task.task_type)?)
                    .bind(&task.schedule)
                    .bind(&task.command)
                    .bind(&task.payload_json)
                    .bind(serde_json::to_string(&task.cover_mode)?)
                    .bind(&task.server_selector_json)
                    .bind(task.push_successful)
                    .bind(&task.notification_group_id)
                    .bind(task.enabled)
                    .bind(&task.created_at)
                    .bind(&task.updated_at)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(TASK_INSERT_POSTGRES)
                    .bind(&task.id)
                    .bind(parse_uuid(&task.owner_user_id, "owner_user_id")?)
                    .bind(&task.name)
                    .bind(serde_json::to_string(&task.task_type)?)
                    .bind(&task.schedule)
                    .bind(&task.command)
                    .bind(&task.payload_json)
                    .bind(serde_json::to_string(&task.cover_mode)?)
                    .bind(&task.server_selector_json)
                    .bind(task.push_successful)
                    .bind(&task.notification_group_id)
                    .bind(task.enabled)
                    .bind(parse_timestamp(&task.created_at, "created_at")?)
                    .bind(parse_timestamp(&task.updated_at, "updated_at")?)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    /// Get task by ID
    pub async fn get_by_id(db: &Db, id: &str) -> Result<Option<Task>> {
        let task_opt = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row_opt = sqlx::query(TASK_SELECT_SQLITE_BY_ID)
                    .bind(id)
                    .fetch_optional(pool)
                    .await?;

                match row_opt {
                    Some(row) => Some(Self::sqlite_row_to_task(row)?),
                    None => None,
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let row_opt = sqlx::query(TASK_SELECT_POSTGRES_BY_ID)
                    .bind(id)
                    .fetch_optional(pool)
                    .await?;

                match row_opt {
                    Some(row) => Some(Self::postgres_row_to_task(row)?),
                    None => None,
                }
            }
        };

        Ok(task_opt)
    }

    /// List tasks for a user
    pub async fn list_by_user(
        db: &Db,
        user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Task>> {
        let limit = limit.clamp(1, TASK_REPO_MAX_LIMIT);
        let offset = offset.max(0);
        let tasks = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(TASK_SELECT_SQLITE_BY_USER)
                    .bind(user_id)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                rows.into_iter()
                    .map(Self::sqlite_row_to_task)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(TASK_SELECT_POSTGRES_BY_USER)
                    .bind(parse_uuid(user_id, "owner_user_id")?)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                rows.into_iter()
                    .map(Self::postgres_row_to_task)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(tasks)
    }

    /// List all enabled scheduled tasks.
    ///
    /// This compatibility helper traverses every keyset page rather than
    /// silently truncating the result at the scheduler's page size.
    pub async fn list_scheduled(db: &Db) -> Result<Vec<Task>> {
        let mut tasks = Vec::new();
        let mut cursor = None;

        loop {
            let page =
                Self::list_scheduled_page(db, cursor.as_deref(), TASK_SCHEDULED_RUNTIME_SCAN_LIMIT)
                    .await?;
            tasks.extend(page.tasks);

            match page.next_cursor {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => break,
            }
        }

        Ok(tasks)
    }

    /// List one stable keyset page of enabled scheduled tasks.
    ///
    /// Task IDs are immutable UUIDv7 values in normal operation. Scanning
    /// them in descending order keeps newly-created tasks ahead of an active
    /// cursor, so they are picked up on the next scheduler tick without
    /// shifting or starving older pages.
    pub(crate) async fn list_scheduled_page(
        db: &Db,
        cursor: Option<&str>,
        limit: i64,
    ) -> Result<ScheduledTaskPage> {
        if limit <= 0 {
            anyhow::bail!("scheduled task page limit must be positive");
        }
        let limit = limit.clamp(1, TASK_SCHEDULED_RUNTIME_SCAN_LIMIT);

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = match cursor {
                    Some(cursor) => {
                        sqlx::query(TASK_SELECT_SQLITE_SCHEDULED_AFTER)
                            .bind(cursor)
                            .bind(limit)
                            .fetch_all(pool)
                            .await?
                    }
                    None => {
                        sqlx::query(TASK_SELECT_SQLITE_SCHEDULED_FIRST)
                            .bind(limit)
                            .fetch_all(pool)
                            .await?
                    }
                };
                Self::sqlite_rows_to_scheduled_page(rows, limit)
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = match cursor {
                    Some(cursor) => {
                        sqlx::query(TASK_SELECT_POSTGRES_SCHEDULED_AFTER)
                            .bind(cursor)
                            .bind(limit)
                            .fetch_all(pool)
                            .await?
                    }
                    None => {
                        sqlx::query(TASK_SELECT_POSTGRES_SCHEDULED_FIRST)
                            .bind(limit)
                            .fetch_all(pool)
                            .await?
                    }
                };
                Self::postgres_rows_to_scheduled_page(rows, limit)
            }
        }
    }

    fn sqlite_rows_to_scheduled_page(
        rows: Vec<SqliteRow>,
        limit: i64,
    ) -> Result<ScheduledTaskPage> {
        let next_cursor = if rows.len() == limit as usize {
            rows.last()
                .map(|row| row.try_get::<String, _>("id"))
                .transpose()?
        } else {
            None
        };
        let tasks = rows
            .into_iter()
            .filter_map(|row| Self::validate_scheduled_task(Self::sqlite_row_to_task(row)))
            .collect();

        Ok(ScheduledTaskPage { tasks, next_cursor })
    }

    fn postgres_rows_to_scheduled_page(rows: Vec<PgRow>, limit: i64) -> Result<ScheduledTaskPage> {
        let next_cursor = if rows.len() == limit as usize {
            rows.last()
                .map(|row| row.try_get::<String, _>("id"))
                .transpose()?
        } else {
            None
        };
        let tasks = rows
            .into_iter()
            .filter_map(|row| Self::validate_scheduled_task(Self::postgres_row_to_task(row)))
            .collect();

        Ok(ScheduledTaskPage { tasks, next_cursor })
    }

    fn validate_scheduled_task(task: Result<Task>) -> Option<Task> {
        match task {
            Ok(task) => match validate_task_definition(&task) {
                Ok(()) => Some(task),
                Err(err) => {
                    warn!("historical scheduled task {} skipped: {err}", task.id);
                    None
                }
            },
            Err(err) => {
                warn!("historical scheduled task row skipped: {err}");
                None
            }
        }
    }

    /// Update a task
    pub async fn update(db: &Db, task: &Task) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(TASK_UPDATE_SQLITE)
                    .bind(&task.name)
                    .bind(&task.schedule)
                    .bind(&task.command)
                    .bind(&task.payload_json)
                    .bind(serde_json::to_string(&task.cover_mode)?)
                    .bind(&task.server_selector_json)
                    .bind(task.push_successful)
                    .bind(&task.notification_group_id)
                    .bind(task.enabled)
                    .bind(&task.updated_at)
                    .bind(&task.id)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(TASK_UPDATE_POSTGRES)
                    .bind(&task.name)
                    .bind(&task.schedule)
                    .bind(&task.command)
                    .bind(&task.payload_json)
                    .bind(serde_json::to_string(&task.cover_mode)?)
                    .bind(&task.server_selector_json)
                    .bind(task.push_successful)
                    .bind(&task.notification_group_id)
                    .bind(task.enabled)
                    .bind(parse_timestamp(&task.updated_at, "updated_at")?)
                    .bind(&task.id)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    /// Delete a task
    pub async fn delete(db: &Db, id: &str) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(TASK_DELETE_SQLITE)
                    .bind(id)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(TASK_DELETE_POSTGRES)
                    .bind(id)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    fn sqlite_row_to_task(row: SqliteRow) -> Result<Task> {
        let task_type_str: String = row.try_get("task_type")?;
        let cover_mode_str: String = row.try_get("cover_mode")?;

        Ok(Task {
            id: row.try_get("id")?,
            owner_user_id: row.try_get("owner_user_id")?,
            name: row.try_get("name")?,
            task_type: serde_json::from_str(&task_type_str)?,
            schedule: row.try_get("schedule")?,
            command: row.try_get("command")?,
            payload_json: row.try_get("payload_json")?,
            cover_mode: serde_json::from_str(&cover_mode_str)?,
            server_selector_json: row.try_get("server_selector_json")?,
            push_successful: row.try_get("push_successful")?,
            notification_group_id: row.try_get("notification_group_id")?,
            last_executed_at: row.try_get("last_executed_at")?,
            last_result: row.try_get("last_result")?,
            enabled: row.try_get("enabled")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }

    fn postgres_row_to_task(row: PgRow) -> Result<Task> {
        let task_type_str: String = row.try_get("task_type")?;
        let cover_mode_str: String = row.try_get("cover_mode")?;
        let owner_user_id: uuid::Uuid = row.try_get("owner_user_id")?;
        let last_executed_at: Option<DateTime<Utc>> = row.try_get("last_executed_at")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

        Ok(Task {
            id: row.try_get("id")?,
            owner_user_id: owner_user_id.to_string(),
            name: row.try_get("name")?,
            task_type: serde_json::from_str(&task_type_str)?,
            schedule: row.try_get("schedule")?,
            command: row.try_get("command")?,
            payload_json: row.try_get("payload_json")?,
            cover_mode: serde_json::from_str(&cover_mode_str)?,
            server_selector_json: row.try_get("server_selector_json")?,
            push_successful: row.try_get("push_successful")?,
            notification_group_id: row.try_get("notification_group_id")?,
            last_executed_at: last_executed_at.map(|value| value.to_rfc3339()),
            last_result: row.try_get("last_result")?,
            enabled: row.try_get("enabled")?,
            created_at: created_at.to_rfc3339(),
            updated_at: updated_at.to_rfc3339(),
        })
    }

    /// Update last execution info
    pub async fn update_last_execution(
        db: &Db,
        task_id: &str,
        executed_at: &str,
        result: &str,
    ) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(TASK_UPDATE_LAST_EXECUTION_SQLITE)
                    .bind(executed_at)
                    .bind(result)
                    .bind(task_id)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(TASK_UPDATE_LAST_EXECUTION_POSTGRES)
                    .bind(parse_timestamp(executed_at, "last_executed_at")?)
                    .bind(result)
                    .bind(task_id)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }
}

pub struct TaskRunRepository;

impl TaskRunRepository {
    /// M5: update a task run with the final status and output once
    /// the agent's TaskResult arrives.
    pub async fn update_result(
        db: &Db,
        run_id: &str,
        status: TaskStatus,
        delay_ms: i64,
        output: Option<String>,
        output_truncated: bool,
        error: Option<String>,
    ) -> Result<()> {
        let status_str = match status {
            TaskStatus::Offline => "pending",
            TaskStatus::Success => "success",
            TaskStatus::Failure => "failed",
            TaskStatus::Timeout => "timeout",
        };
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(TASK_RUN_UPDATE_RESULT_SQLITE)
                    .bind(status_str)
                    .bind(delay_ms)
                    .bind(&output)
                    .bind(output_truncated)
                    .bind(&error)
                    .bind(run_id)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(TASK_RUN_UPDATE_RESULT_POSTGRES)
                    .bind(status_str)
                    .bind(delay_ms)
                    .bind(&output)
                    .bind(output_truncated)
                    .bind(&error)
                    .bind(run_id)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    /// Mark an abandoned in-flight run as failed without overwriting a result
    /// that may have completed concurrently.
    pub async fn settle_pending_after_cancellation(
        db: &Db,
        run_id: &str,
        error: &str,
    ) -> Result<bool> {
        let rows_affected = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => sqlx::query(TASK_RUN_SETTLE_PENDING_SQLITE)
                .bind(error)
                .bind(run_id)
                .execute(pool)
                .await?
                .rows_affected(),
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(TASK_RUN_SETTLE_PENDING_POSTGRES)
                    .bind(error)
                    .bind(run_id)
                    .execute(pool)
                    .await?
                    .rows_affected()
            }
        };
        Ok(rows_affected > 0)
    }

    /// A restarted Server cannot receive replies for task IDs registered by
    /// the previous process, so all pre-existing pending rows are terminal.
    pub async fn settle_pending_after_restart(db: &Db, error: &str) -> Result<u64> {
        let rows_affected = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(TASK_RUN_SETTLE_ALL_PENDING_SQLITE)
                    .bind(error)
                    .execute(pool)
                    .await?
                    .rows_affected()
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(TASK_RUN_SETTLE_ALL_PENDING_POSTGRES)
                    .bind(error)
                    .execute(pool)
                    .await?
                    .rows_affected()
            }
        };
        Ok(rows_affected)
    }

    /// Create a task run record
    pub async fn create(db: &Db, run: &TaskRun) -> Result<()> {
        let status_str = match run.status {
            TaskStatus::Offline => "pending",
            TaskStatus::Success => "success",
            TaskStatus::Failure => "failed",
            TaskStatus::Timeout => "timeout",
        };

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(TASK_RUN_INSERT_SQLITE)
                    .bind(&run.id)
                    .bind(&run.task_id)
                    .bind(&run.server_id)
                    .bind(status_str)
                    .bind(run.delay_ms)
                    .bind(&run.output)
                    .bind(run.output_truncated)
                    .bind(&run.error)
                    .bind(&run.created_at)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(TASK_RUN_INSERT_POSTGRES)
                    .bind(&run.id)
                    .bind(&run.task_id)
                    .bind(parse_uuid(&run.server_id, "server_id")?)
                    .bind(status_str)
                    .bind(run.delay_ms)
                    .bind(&run.output)
                    .bind(run.output_truncated)
                    .bind(&run.error)
                    .bind(parse_timestamp(&run.created_at, "created_at")?)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    /// List task runs for a task
    pub async fn list_by_task(
        db: &Db,
        task_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskRun>> {
        let limit = limit.clamp(1, TASK_REPO_MAX_LIMIT);
        let offset = offset.max(0);
        let runs = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(TASK_RUN_SELECT_SQLITE_BY_TASK)
                    .bind(task_id)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                rows.into_iter()
                    .map(sqlite_row_to_task_run)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(TASK_RUN_SELECT_POSTGRES_BY_TASK)
                    .bind(task_id)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                rows.into_iter()
                    .map(postgres_row_to_task_run)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(runs)
    }

    /// List task runs for a task, filtered by a PAT server allowlist before pagination.
    pub async fn list_by_task_for_server_ids(
        db: &Db,
        task_id: &str,
        server_ids: &[String],
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskRun>> {
        if server_ids.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, TASK_REPO_MAX_LIMIT);
        let offset = offset.max(0);
        let runs = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let placeholders = sqlite_placeholders(server_ids.len());
                let sql = format!(
                    r#"
                    SELECT id, task_id, server_id, status, delay_ms, output,
                           output_truncated, error, created_at
                    FROM task_runs
                    WHERE task_id = ? AND server_id IN ({placeholders})
                    ORDER BY created_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                );
                let mut query = sqlx::query(&sql).bind(task_id);
                for server_id in server_ids {
                    query = query.bind(server_id);
                }
                let rows = query.bind(limit).bind(offset).fetch_all(pool).await?;

                rows.into_iter()
                    .map(sqlite_row_to_task_run)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let parsed = parse_uuid_ids(server_ids)?;
                let rows = sqlx::query(TASK_RUN_SELECT_POSTGRES_BY_TASK_AND_SERVERS)
                    .bind(task_id)
                    .bind(&parsed)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                rows.into_iter()
                    .map(postgres_row_to_task_run)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(runs)
    }
}

pub struct AuditLogRepository;

impl AuditLogRepository {
    /// Create an audit log entry
    pub async fn create(db: &Db, log: &AuditLog) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(AUDIT_LOG_INSERT_SQLITE)
                    .bind(&log.id)
                    .bind(&log.user_id)
                    .bind(&log.api_token_id)
                    .bind(&log.action)
                    .bind(&log.resource_type)
                    .bind(&log.resource_id)
                    .bind(&log.server_id)
                    .bind(&log.ip)
                    .bind(&log.outcome)
                    .bind(&log.metadata_json)
                    .bind(&log.sensitive_hash)
                    .bind(&log.created_at)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(AUDIT_LOG_INSERT_POSTGRES)
                    .bind(&log.id)
                    .bind(parse_optional_uuid(log.user_id.as_deref(), "user_id")?)
                    .bind(parse_optional_uuid(
                        log.api_token_id.as_deref(),
                        "api_token_id",
                    )?)
                    .bind(&log.action)
                    .bind(&log.resource_type)
                    .bind(&log.resource_id)
                    .bind(&log.server_id)
                    .bind(&log.ip)
                    .bind(&log.outcome)
                    .bind(&log.metadata_json)
                    .bind(&log.sensitive_hash)
                    .bind(parse_timestamp(&log.created_at, "created_at")?)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }
}

macro_rules! task_select {
    ($suffix:literal) => {
        concat!(
            "SELECT id, owner_user_id, name, task_type, schedule, command, ",
            "payload_json, cover_mode, server_selector_json, ",
            "push_successful, notification_group_id, last_executed_at, ",
            "last_result, enabled, created_at, updated_at FROM tasks ",
            $suffix
        )
    };
}

const TASK_INSERT_SQLITE: &str = r#"
    INSERT INTO tasks (
        id, owner_user_id, name, task_type, schedule, command,
        payload_json, cover_mode, server_selector_json,
        push_successful, notification_group_id, enabled,
        created_at, updated_at
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
"#;
const TASK_INSERT_POSTGRES: &str = r#"
    INSERT INTO tasks (
        id, owner_user_id, name, task_type, schedule, command,
        payload_json, cover_mode, server_selector_json,
        push_successful, notification_group_id, enabled,
        created_at, updated_at
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
"#;
const TASK_SELECT_SQLITE_BY_ID: &str = task_select!("WHERE id = ?");
const TASK_SELECT_POSTGRES_BY_ID: &str = task_select!("WHERE id = $1");
const TASK_SELECT_SQLITE_BY_USER: &str =
    task_select!("WHERE owner_user_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?");
const TASK_SELECT_POSTGRES_BY_USER: &str =
    task_select!("WHERE owner_user_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3");
const TASK_SELECT_SQLITE_SCHEDULED_FIRST: &str =
    task_select!("WHERE enabled = 1 AND schedule IS NOT NULL ORDER BY id DESC LIMIT ?");
const TASK_SELECT_SQLITE_SCHEDULED_AFTER: &str =
    task_select!("WHERE enabled = 1 AND schedule IS NOT NULL AND id < ? ORDER BY id DESC LIMIT ?");
const TASK_SELECT_POSTGRES_SCHEDULED_FIRST: &str =
    task_select!("WHERE enabled = TRUE AND schedule IS NOT NULL ORDER BY id DESC LIMIT $1");
const TASK_SELECT_POSTGRES_SCHEDULED_AFTER: &str = task_select!(
    "WHERE enabled = TRUE AND schedule IS NOT NULL AND id < $1 ORDER BY id DESC LIMIT $2"
);
const TASK_UPDATE_SQLITE: &str = r#"
    UPDATE tasks
    SET name = ?, schedule = ?, command = ?, payload_json = ?,
        cover_mode = ?, server_selector_json = ?, push_successful = ?,
        notification_group_id = ?, enabled = ?, updated_at = ?
    WHERE id = ?
"#;
const TASK_UPDATE_POSTGRES: &str = r#"
    UPDATE tasks
    SET name = $1, schedule = $2, command = $3, payload_json = $4,
        cover_mode = $5, server_selector_json = $6, push_successful = $7,
        notification_group_id = $8, enabled = $9, updated_at = $10
    WHERE id = $11
"#;
const TASK_DELETE_SQLITE: &str = "DELETE FROM tasks WHERE id = ?";
const TASK_DELETE_POSTGRES: &str = "DELETE FROM tasks WHERE id = $1";
const TASK_UPDATE_LAST_EXECUTION_SQLITE: &str =
    "UPDATE tasks SET last_executed_at = ?, last_result = ? WHERE id = ?";
const TASK_UPDATE_LAST_EXECUTION_POSTGRES: &str =
    "UPDATE tasks SET last_executed_at = $1, last_result = $2 WHERE id = $3";

const TASK_RUN_UPDATE_RESULT_SQLITE: &str =
    "UPDATE task_runs SET status = ?, delay_ms = ?, output = ?, output_truncated = ?, error = ? WHERE id = ?";
const TASK_RUN_UPDATE_RESULT_POSTGRES: &str =
    "UPDATE task_runs SET status = $1, delay_ms = $2, output = $3, output_truncated = $4, error = $5 WHERE id = $6";
const TASK_RUN_SETTLE_PENDING_SQLITE: &str =
    "UPDATE task_runs SET status = 'failed', error = ? WHERE id = ? AND status = 'pending'";
const TASK_RUN_SETTLE_PENDING_POSTGRES: &str =
    "UPDATE task_runs SET status = 'failed', error = $1 WHERE id = $2 AND status = 'pending'";
const TASK_RUN_SETTLE_ALL_PENDING_SQLITE: &str =
    "UPDATE task_runs SET status = 'failed', error = ? WHERE status = 'pending'";
const TASK_RUN_SETTLE_ALL_PENDING_POSTGRES: &str =
    "UPDATE task_runs SET status = 'failed', error = $1 WHERE status = 'pending'";
const TASK_RUN_INSERT_SQLITE: &str = r#"
    INSERT INTO task_runs (
        id, task_id, server_id, status, delay_ms, output,
        output_truncated, error, created_at
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
"#;
const TASK_RUN_INSERT_POSTGRES: &str = r#"
    INSERT INTO task_runs (
        id, task_id, server_id, status, delay_ms, output,
        output_truncated, error, created_at
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
"#;
const TASK_RUN_SELECT_SQLITE_BY_TASK: &str = r#"
    SELECT id, task_id, server_id, status, delay_ms, output,
           output_truncated, error, created_at
    FROM task_runs
    WHERE task_id = ?
    ORDER BY created_at DESC
    LIMIT ? OFFSET ?
"#;
const TASK_RUN_SELECT_POSTGRES_BY_TASK: &str = r#"
    SELECT id, task_id, server_id, status, delay_ms, output,
           output_truncated, error, created_at
    FROM task_runs
    WHERE task_id = $1
    ORDER BY created_at DESC
    LIMIT $2 OFFSET $3
"#;
const TASK_RUN_SELECT_POSTGRES_BY_TASK_AND_SERVERS: &str = r#"
    SELECT id, task_id, server_id, status, delay_ms, output,
           output_truncated, error, created_at
    FROM task_runs
    WHERE task_id = $1 AND server_id = ANY($2::uuid[])
    ORDER BY created_at DESC
    LIMIT $3 OFFSET $4
"#;
const AUDIT_LOG_INSERT_SQLITE: &str = r#"
    INSERT INTO audit_logs (
        id, user_id, api_token_id, action, resource_type, resource_id,
        server_id, ip, outcome, metadata_json, sensitive_hash, created_at
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
"#;
const AUDIT_LOG_INSERT_POSTGRES: &str = r#"
    INSERT INTO audit_logs (
        id, user_id, api_token_id, action, resource_type, resource_id,
        server_id, ip, outcome, metadata_json, sensitive_hash, created_at
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
"#;

fn sqlite_row_to_task_run(row: SqliteRow) -> Result<TaskRun> {
    Ok(TaskRun {
        id: row.try_get("id")?,
        task_id: row.try_get("task_id")?,
        server_id: row.try_get("server_id")?,
        status: parse_task_status(row.try_get::<String, _>("status")?.as_str()),
        delay_ms: row.try_get("delay_ms")?,
        output: row.try_get("output")?,
        output_truncated: row.try_get("output_truncated")?,
        error: row.try_get("error")?,
        created_at: row.try_get("created_at")?,
    })
}

fn postgres_row_to_task_run(row: PgRow) -> Result<TaskRun> {
    let server_id: uuid::Uuid = row.try_get("server_id")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;

    Ok(TaskRun {
        id: row.try_get("id")?,
        task_id: row.try_get("task_id")?,
        server_id: server_id.to_string(),
        status: parse_task_status(row.try_get::<String, _>("status")?.as_str()),
        delay_ms: row.try_get("delay_ms")?,
        output: row.try_get("output")?,
        output_truncated: row.try_get("output_truncated")?,
        error: row.try_get("error")?,
        created_at: created_at.to_rfc3339(),
    })
}

fn parse_task_status(value: &str) -> TaskStatus {
    match value {
        "pending" | "running" => TaskStatus::Offline,
        "success" => TaskStatus::Success,
        "failed" => TaskStatus::Failure,
        "timeout" => TaskStatus::Timeout,
        _ => TaskStatus::Failure,
    }
}

fn parse_uuid(value: &str, field: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(value).with_context(|| format!("invalid task repository {field} UUID"))
}

fn parse_optional_uuid(value: Option<&str>, field: &str) -> Result<Option<uuid::Uuid>> {
    value.map(|value| parse_uuid(value, field)).transpose()
}

fn parse_uuid_ids(ids: &[String]) -> Result<Vec<uuid::Uuid>> {
    ids.iter()
        .map(|id| parse_uuid(id, "server_id"))
        .collect::<Result<Vec<_>>>()
}

fn sqlite_placeholders(len: usize) -> String {
    std::iter::repeat_n("?", len).collect::<Vec<_>>().join(", ")
}

fn parse_timestamp(value: &str, field: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .with_context(|| format!("invalid task repository {field} timestamp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CreateAgentInput, CreateUserInput, DatabaseBackend, UserRepository};
    use xlstatus_shared::{AgentId, UserRole};

    #[test]
    fn postgres_queries_do_not_use_sqlite_placeholders() {
        let queries = [
            TASK_INSERT_POSTGRES,
            TASK_SELECT_POSTGRES_BY_ID,
            TASK_SELECT_POSTGRES_BY_USER,
            TASK_SELECT_POSTGRES_SCHEDULED_FIRST,
            TASK_SELECT_POSTGRES_SCHEDULED_AFTER,
            TASK_UPDATE_POSTGRES,
            TASK_DELETE_POSTGRES,
            TASK_UPDATE_LAST_EXECUTION_POSTGRES,
            TASK_RUN_UPDATE_RESULT_POSTGRES,
            TASK_RUN_SETTLE_PENDING_POSTGRES,
            TASK_RUN_SETTLE_ALL_PENDING_POSTGRES,
            TASK_RUN_INSERT_POSTGRES,
            TASK_RUN_SELECT_POSTGRES_BY_TASK,
            TASK_RUN_SELECT_POSTGRES_BY_TASK_AND_SERVERS,
            AUDIT_LOG_INSERT_POSTGRES,
        ];

        for query in queries {
            assert!(!query.contains('?'), "{query}");
        }
    }

    #[test]
    fn task_repository_postgres_uuid_and_timestamp_parsers_reject_invalid_values() {
        assert!(parse_uuid("not-a-uuid", "owner_user_id").is_err());
        assert!(parse_timestamp("not-a-timestamp", "created_at").is_err());
        assert!(parse_optional_uuid(Some("not-a-uuid"), "api_token_id").is_err());
    }

    #[tokio::test]
    async fn task_repository_sqlite_round_trips_task_and_runs() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "task-owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let agent = crate::db::repository::AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "task-agent".into(),
                public_key: "public-key".into(),
                owner_user_id: user.id,
            })
            .await
            .unwrap();

        let now = Utc::now().to_rfc3339();
        match &db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO servers (id, owner_user_id, name, created_at, updated_at, agent_id) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(agent.id.0.to_string())
                .bind(user.id.0.to_string())
                .bind("task-agent")
                .bind(&now)
                .bind(&now)
                .bind(agent.id.0.to_string())
                .execute(pool)
                .await
                .unwrap();
            }
            DatabaseBackend::Postgres(_) => unreachable!(),
        }

        let task = Task {
            id: uuid::Uuid::now_v7().to_string(),
            owner_user_id: user.id.0.to_string(),
            name: "backup".into(),
            task_type: TaskType::Shell,
            schedule: Some("0 * * * *".into()),
            command: Some("echo ok".into()),
            payload_json: None,
            cover_mode: CoverMode::Specific,
            server_selector_json: serde_json::json!({
                "server_ids": [agent.id.0.to_string()]
            })
            .to_string(),
            push_successful: true,
            notification_group_id: None,
            last_executed_at: None,
            last_result: None,
            enabled: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        TaskRepository::create(&db, &task).await.unwrap();
        let found = TaskRepository::get_by_id(&db, &task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.owner_user_id, task.owner_user_id);
        assert_eq!(found.command.as_deref(), Some("echo ok"));

        TaskRepository::update_last_execution(&db, &task.id, &now, "success=1")
            .await
            .unwrap();
        let found = TaskRepository::get_by_id(&db, &task.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.last_executed_at.as_deref(), Some(now.as_str()));
        assert_eq!(found.last_result.as_deref(), Some("success=1"));

        let run = TaskRun {
            id: uuid::Uuid::now_v7().to_string(),
            task_id: task.id.clone(),
            server_id: agent.id.0.to_string(),
            status: TaskStatus::Success,
            delay_ms: Some(25),
            output: Some("ok".into()),
            output_truncated: false,
            error: None,
            created_at: now.clone(),
        };
        TaskRunRepository::create(&db, &run).await.unwrap();
        assert!(!TaskRunRepository::settle_pending_after_cancellation(
            &db,
            &run.id,
            "must not replace success",
        )
        .await
        .unwrap());

        let cancelled_run = TaskRun {
            id: uuid::Uuid::now_v7().to_string(),
            task_id: task.id.clone(),
            server_id: agent.id.0.to_string(),
            status: TaskStatus::Offline,
            delay_ms: Some(0),
            output: None,
            output_truncated: false,
            error: None,
            created_at: now.clone(),
        };
        TaskRunRepository::create(&db, &cancelled_run)
            .await
            .unwrap();
        assert!(TaskRunRepository::settle_pending_after_cancellation(
            &db,
            &cancelled_run.id,
            "dispatch cancelled",
        )
        .await
        .unwrap());

        let restart_run = TaskRun {
            id: uuid::Uuid::now_v7().to_string(),
            task_id: task.id.clone(),
            server_id: agent.id.0.to_string(),
            status: TaskStatus::Offline,
            delay_ms: Some(0),
            output: None,
            output_truncated: false,
            error: None,
            created_at: now,
        };
        TaskRunRepository::create(&db, &restart_run).await.unwrap();
        assert_eq!(
            TaskRunRepository::settle_pending_after_restart(&db, "Server restarted")
                .await
                .unwrap(),
            1
        );

        let runs = TaskRunRepository::list_by_task(&db, &task.id, 10, 0)
            .await
            .unwrap();
        assert_eq!(runs.len(), 3);
        let completed = runs.iter().find(|item| item.id == run.id).unwrap();
        assert_eq!(completed.server_id, agent.id.0.to_string());
        assert_eq!(completed.status, TaskStatus::Success);
        assert_eq!(completed.output.as_deref(), Some("ok"));
        let cancelled = runs
            .iter()
            .find(|item| item.id == cancelled_run.id)
            .unwrap();
        assert_eq!(cancelled.status, TaskStatus::Failure);
        assert_eq!(cancelled.error.as_deref(), Some("dispatch cancelled"));
        let restarted = runs.iter().find(|item| item.id == restart_run.id).unwrap();
        assert_eq!(restarted.status, TaskStatus::Failure);
        assert_eq!(restarted.error.as_deref(), Some("Server restarted"));
    }

    #[tokio::test]
    async fn invalid_historical_scheduled_tasks_are_not_loaded() {
        let db = test_db().await;
        let user_id = "00000000-0000-0000-0000-000000000001";
        seed_user(&db, user_id).await;

        let mut valid = test_task(user_id, "00000000-0000-0000-0000-000000000101");
        valid.schedule = Some("0 * * * *".into());
        TaskRepository::create(&db, &valid).await.unwrap();

        seed_raw_task(
            &db,
            user_id,
            "00000000-0000-0000-0000-000000000102",
            "bad-kind",
            r#""unknown""#,
            r#""specific""#,
            Some("0 * * * *"),
            Some("true"),
            "{}",
        )
        .await;
        seed_raw_task(
            &db,
            user_id,
            "00000000-0000-0000-0000-000000000103",
            "bad-selector",
            r#""shell""#,
            r#""specific""#,
            Some("0 * * * *"),
            Some("true"),
            "not json",
        )
        .await;
        seed_raw_task(
            &db,
            user_id,
            "00000000-0000-0000-0000-000000000104",
            "bad-schedule",
            r#""shell""#,
            r#""specific""#,
            Some("not a cron"),
            Some("true"),
            "{}",
        )
        .await;
        seed_raw_task(
            &db,
            user_id,
            "00000000-0000-0000-0000-000000000105",
            "bad-command",
            r#""shell""#,
            r#""specific""#,
            Some("0 * * * *"),
            Some(&"x".repeat(crate::tasks::TASK_MAX_COMMAND_BYTES + 1)),
            "{}",
        )
        .await;

        let scheduled = TaskRepository::list_scheduled(&db).await.unwrap();

        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].id, valid.id);
        assert_eq!(scheduled[0].command.as_deref(), Some("true"));
    }

    #[tokio::test]
    async fn scheduled_task_scan_traverses_more_than_runtime_page_limit() {
        let db = test_db().await;
        let user_id = "00000000-0000-0000-0000-000000000001";
        seed_user(&db, user_id).await;
        let task_count = TASK_SCHEDULED_RUNTIME_SCAN_LIMIT as usize + 1;
        let task_ids = (0..task_count)
            .map(|index| format!("00000000-0000-0000-0000-{index:012}"))
            .collect::<Vec<_>>();

        let DatabaseBackend::Sqlite(pool) = &db else {
            unreachable!();
        };
        let mut transaction = pool.begin().await.unwrap();
        for task_id in &task_ids {
            sqlx::query(
                "INSERT INTO tasks (id, owner_user_id, name, task_type, schedule, command, cover_mode, server_selector_json, enabled, push_successful, created_at, updated_at) VALUES (?, ?, 'scheduled', '\"shell\"', '0 * * * *', 'true', '\"specific\"', '{}', 1, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            )
            .bind(task_id)
            .bind(user_id)
            .execute(&mut *transaction)
            .await
            .unwrap();
        }
        transaction.commit().await.unwrap();

        let first_page =
            TaskRepository::list_scheduled_page(&db, None, TASK_SCHEDULED_RUNTIME_SCAN_LIMIT)
                .await
                .unwrap();
        assert_eq!(
            first_page.tasks.len(),
            TASK_SCHEDULED_RUNTIME_SCAN_LIMIT as usize
        );
        assert_eq!(first_page.tasks[0].id, task_ids[task_count - 1]);
        assert_eq!(first_page.tasks.last().unwrap().id, task_ids[1]);
        assert_eq!(
            first_page.next_cursor.as_deref(),
            Some(task_ids[1].as_str())
        );

        let second_page = TaskRepository::list_scheduled_page(
            &db,
            first_page.next_cursor.as_deref(),
            TASK_SCHEDULED_RUNTIME_SCAN_LIMIT,
        )
        .await
        .unwrap();
        assert_eq!(second_page.tasks.len(), 1);
        assert_eq!(second_page.tasks[0].id, task_ids[0]);
        assert_eq!(second_page.next_cursor, None);

        let scheduled = TaskRepository::list_scheduled(&db).await.unwrap();
        assert_eq!(scheduled.len(), task_count);
        assert_eq!(scheduled.first().unwrap().id, task_ids[task_count - 1]);
        assert_eq!(scheduled.last().unwrap().id, task_ids[0]);
        assert!(TaskRepository::list_scheduled_page(&db, None, 0)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn task_and_run_repositories_clamp_negative_and_oversized_pagination() {
        let db = test_db().await;
        let user_id = "00000000-0000-0000-0000-000000000001";
        let server_id = "00000000-0000-0000-0000-000000000101";
        seed_user(&db, user_id).await;

        let DatabaseBackend::Sqlite(pool) = &db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO servers (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, 'srv', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(server_id)
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();

        let t1 = test_task(user_id, "00000000-0000-0000-0000-000000000201");
        let t2 = test_task(user_id, "00000000-0000-0000-0000-000000000202");
        TaskRepository::create(&db, &t1).await.unwrap();
        TaskRepository::create(&db, &t2).await.unwrap();

        assert_eq!(
            TaskRepository::list_by_user(&db, user_id, -1, -10)
                .await
                .unwrap()
                .len(),
            1
        );

        for idx in 0..3 {
            let run = TaskRun {
                id: format!("00000000-0000-0000-0000-00000000030{idx}"),
                task_id: t1.id.clone(),
                server_id: server_id.into(),
                status: TaskStatus::Success,
                delay_ms: Some(10),
                output: None,
                output_truncated: false,
                error: None,
                created_at: format!("2026-01-01T00:00:0{idx}Z"),
            };
            TaskRunRepository::create(&db, &run).await.unwrap();
        }

        assert_eq!(
            TaskRunRepository::list_by_task(&db, &t1.id, -1, -10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            TaskRunRepository::list_by_task_for_server_ids(
                &db,
                &t1.id,
                &[server_id.to_string()],
                -1,
                -10,
            )
            .await
            .unwrap()
            .len(),
            1
        );
    }

    async fn test_db() -> DatabaseBackend {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-task-repository-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_user(db: &DatabaseBackend, id: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'hash', 'member', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("user-{id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_raw_task(
        db: &DatabaseBackend,
        owner: &str,
        id: &str,
        name: &str,
        task_type: &str,
        cover_mode: &str,
        schedule: Option<&str>,
        command: Option<&str>,
        selector: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO tasks (id, owner_user_id, name, task_type, schedule, command, cover_mode, server_selector_json, enabled, push_successful, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .bind(name)
        .bind(task_type)
        .bind(schedule)
        .bind(command)
        .bind(cover_mode)
        .bind(selector)
        .execute(pool)
        .await
        .unwrap();
    }

    fn test_task(owner: &str, id: &str) -> Task {
        Task {
            id: id.to_string(),
            owner_user_id: owner.to_string(),
            name: "valid".into(),
            task_type: TaskType::Shell,
            schedule: None,
            command: Some("true".into()),
            payload_json: None,
            cover_mode: CoverMode::Specific,
            server_selector_json: serde_json::to_string(&ServerSelector::default()).unwrap(),
            push_successful: false,
            notification_group_id: None,
            last_executed_at: None,
            last_result: None,
            enabled: true,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }
}
