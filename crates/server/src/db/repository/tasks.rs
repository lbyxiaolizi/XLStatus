#![allow(dead_code)]
#![allow(unused)]

use crate::db::Db;
use anyhow::Result;
use sqlx::Row;
use xlstatus_shared::tasks::*;

pub struct TaskRepository;

impl TaskRepository {
    /// Create a new task
    pub async fn create(db: &Db, task: &Task) -> Result<()> {
        let query = r#"
            INSERT INTO tasks (
                id, owner_user_id, name, task_type, schedule, command,
                payload_json, cover_mode, server_selector_json,
                push_successful, notification_group_id, enabled,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
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
                sqlx::query(query)
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
        }

        Ok(())
    }

    /// Get task by ID
    pub async fn get_by_id(db: &Db, id: &str) -> Result<Option<Task>> {
        let query = r#"
            SELECT id, owner_user_id, name, task_type, schedule, command,
                   payload_json, cover_mode, server_selector_json,
                   push_successful, notification_group_id, last_executed_at,
                   last_result, enabled, created_at, updated_at
            FROM tasks
            WHERE id = ?
        "#;

        let task_opt = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row_opt = sqlx::query(query).bind(id).fetch_optional(pool).await?;

                match row_opt {
                    Some(row) => Some(Self::row_to_task(row)?),
                    None => None,
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let row_opt = sqlx::query(query).bind(id).fetch_optional(pool).await?;

                match row_opt {
                    Some(row) => Some(Self::row_to_task(row)?),
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
        let query = r#"
            SELECT id, owner_user_id, name, task_type, schedule, command,
                   payload_json, cover_mode, server_selector_json,
                   push_successful, notification_group_id, last_executed_at,
                   last_result, enabled, created_at, updated_at
            FROM tasks
            WHERE owner_user_id = ?
            ORDER BY created_at DESC
            LIMIT ? OFFSET ?
        "#;

        let tasks = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(query)
                    .bind(user_id)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                rows.into_iter()
                    .map(Self::row_to_task)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(query)
                    .bind(user_id)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                rows.into_iter()
                    .map(Self::row_to_task)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(tasks)
    }

    /// List all enabled scheduled tasks
    pub async fn list_scheduled(db: &Db) -> Result<Vec<Task>> {
        let tasks = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let query = r#"
                    SELECT id, owner_user_id, name, task_type, schedule, command,
                           payload_json, cover_mode, server_selector_json,
                           push_successful, notification_group_id, last_executed_at,
                           last_result, enabled, created_at, updated_at
                    FROM tasks
                    WHERE enabled = 1 AND schedule IS NOT NULL
                    ORDER BY created_at DESC
                "#;
                let rows = sqlx::query(query).fetch_all(pool).await?;
                rows.into_iter()
                    .map(Self::row_to_task)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let query = r#"
                    SELECT id, owner_user_id, name, task_type, schedule, command,
                           payload_json, cover_mode, server_selector_json,
                           push_successful, notification_group_id, last_executed_at,
                           last_result, enabled, created_at, updated_at
                    FROM tasks
                    WHERE enabled = TRUE AND schedule IS NOT NULL
                    ORDER BY created_at DESC
                "#;
                let rows = sqlx::query(query).fetch_all(pool).await?;
                rows.into_iter()
                    .map(Self::row_to_task)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(tasks)
    }

    /// Update a task
    pub async fn update(db: &Db, task: &Task) -> Result<()> {
        let query = r#"
            UPDATE tasks
            SET name = ?, schedule = ?, command = ?, payload_json = ?,
                cover_mode = ?, server_selector_json = ?, push_successful = ?,
                notification_group_id = ?, enabled = ?, updated_at = ?
            WHERE id = ?
        "#;

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
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
                sqlx::query(query)
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
        }

        Ok(())
    }

    /// Delete a task
    pub async fn delete(db: &Db, id: &str) -> Result<()> {
        let query = "DELETE FROM tasks WHERE id = ?";

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query).bind(id).execute(pool).await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(query).bind(id).execute(pool).await?;
            }
        }

        Ok(())
    }

    /// Helper to convert row to Task (Sqlite)
    fn row_to_task<R: Row>(row: R) -> Result<Task>
    where
        String: for<'a> sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
        bool: for<'a> sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
        Option<String>: for<'a> sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
        usize: sqlx::ColumnIndex<R>,
        for<'a> &'a str: sqlx::ColumnIndex<R>,
    {
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

    /// Update last execution info
    pub async fn update_last_execution(
        db: &Db,
        task_id: &str,
        executed_at: &str,
        result: &str,
    ) -> Result<()> {
        let query = "UPDATE tasks SET last_executed_at = ?, last_result = ? WHERE id = ?";

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
                    .bind(executed_at)
                    .bind(result)
                    .bind(task_id)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(query)
                    .bind(executed_at)
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
        let query = "UPDATE task_runs SET status = ?, delay_ms = ?, output = ?, output_truncated = ?, error = ? WHERE id = ?";
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
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
                sqlx::query(query)
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

    /// Create a task run record
    pub async fn create(db: &Db, run: &TaskRun) -> Result<()> {
        let query = r#"
            INSERT INTO task_runs (
                id, task_id, server_id, status, delay_ms, output,
                output_truncated, error, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        let status_str = match run.status {
            TaskStatus::Offline => "pending",
            TaskStatus::Offline => "running",
            TaskStatus::Success => "success",
            TaskStatus::Failure => "failed",
            TaskStatus::Timeout => "timeout",
        };

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
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
                sqlx::query(query)
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
        let query = r#"
            SELECT id, task_id, server_id, status, delay_ms, output,
                   output_truncated, error, created_at
            FROM task_runs
            WHERE task_id = ?
            ORDER BY created_at DESC
            LIMIT ? OFFSET ?
        "#;

        let runs = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(query)
                    .bind(task_id)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                rows.into_iter()
                    .map(|row| {
                        let status_str: String = row.try_get("status")?;
                        let status = match status_str.as_str() {
                            "pending" => TaskStatus::Offline,
                            "running" => TaskStatus::Offline,
                            "success" => TaskStatus::Success,
                            "failed" => TaskStatus::Failure,
                            "timeout" => TaskStatus::Timeout,
                            _ => TaskStatus::Failure,
                        };

                        Ok(TaskRun {
                            id: row.try_get("id")?,
                            task_id: row.try_get("task_id")?,
                            server_id: row.try_get("server_id")?,
                            status,
                            delay_ms: row.try_get("delay_ms")?,
                            output: row.try_get("output")?,
                            output_truncated: row.try_get("output_truncated")?,
                            error: row.try_get("error")?,
                            created_at: row.try_get("created_at")?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(query)
                    .bind(task_id)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(pool)
                    .await?;

                rows.into_iter()
                    .map(|row| {
                        let status_str: String = row.try_get("status")?;
                        let status = match status_str.as_str() {
                            "pending" => TaskStatus::Offline,
                            "running" => TaskStatus::Offline,
                            "success" => TaskStatus::Success,
                            "failed" => TaskStatus::Failure,
                            "timeout" => TaskStatus::Timeout,
                            _ => TaskStatus::Failure,
                        };

                        Ok(TaskRun {
                            id: row.try_get("id")?,
                            task_id: row.try_get("task_id")?,
                            server_id: row.try_get("server_id")?,
                            status,
                            delay_ms: row.try_get("delay_ms")?,
                            output: row.try_get("output")?,
                            output_truncated: row.try_get("output_truncated")?,
                            error: row.try_get("error")?,
                            created_at: row.try_get("created_at")?,
                        })
                    })
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
        let query = r#"
            INSERT INTO audit_logs (
                id, user_id, api_token_id, action, resource_type, resource_id,
                server_id, ip, outcome, metadata_json, sensitive_hash, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
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
                sqlx::query(query)
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
        }

        Ok(())
    }
}
