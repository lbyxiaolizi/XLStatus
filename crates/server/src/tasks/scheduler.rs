use crate::db::Db;
use anyhow::{Context, Result};
use chrono::Utc;
use cron::Schedule;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};
use xlstatus_shared::tasks::*;

use crate::db::repository::tasks::{TaskRepository, TaskRunRepository};

/// Task scheduler that runs scheduled tasks
pub struct TaskScheduler {
    db: Db,
    // Map of task_id -> next execution time
    scheduled_tasks: Arc<RwLock<HashMap<String, chrono::DateTime<Utc>>>>,
}

impl TaskScheduler {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            scheduled_tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start the scheduler loop
    pub async fn start(self: Arc<Self>) {
        info!("Starting task scheduler");

        let mut tick = interval(Duration::from_secs(30));

        loop {
            tick.tick().await;

            if let Err(e) = self.check_and_run_tasks().await {
                error!("Task scheduler error: {}", e);
            }
        }
    }

    /// Check for tasks that need to run and execute them
    async fn check_and_run_tasks(&self) -> Result<()> {
        let now = Utc::now();

        // Load all scheduled tasks
        let tasks = TaskRepository::list_scheduled(&self.db)
            .await
            .context("Failed to load scheduled tasks")?;

        for task in tasks {
            if !task.enabled {
                continue;
            }

            let schedule_str = match &task.schedule {
                Some(s) => s,
                None => continue,
            };

            // Parse cron schedule
            let schedule = match Schedule::from_str(schedule_str) {
                Ok(s) => s,
                Err(e) => {
                    warn!("Invalid cron schedule for task {}: {}", task.id, e);
                    continue;
                }
            };

            // Check if task should run
            let should_run = {
                let scheduled = self.scheduled_tasks.read().await;
                match scheduled.get(&task.id) {
                    Some(next_run) => now >= *next_run,
                    None => true, // First time scheduling
                }
            };

            if should_run {
                // Calculate next run time
                let next_run = schedule
                    .upcoming(Utc)
                    .next()
                    .unwrap_or_else(|| now + chrono::Duration::hours(1));

                // Update scheduled time
                {
                    let mut scheduled = self.scheduled_tasks.write().await;
                    scheduled.insert(task.id.clone(), next_run);
                }

                // Execute task
                info!("Executing scheduled task: {} ({})", task.id, task.name);
                if let Err(e) = self.execute_task(&task).await {
                    error!("Failed to execute task {}: {}", task.id, e);
                }
            }
        }

        Ok(())
    }

    /// Execute a task on selected servers
    async fn execute_task(&self, task: &Task) -> Result<()> {
        // Parse server selector
        let selector: ServerSelector = serde_json::from_str(&task.server_selector_json)
            .context("Failed to parse server selector")?;

        // Resolve target servers
        let server_ids = self.resolve_servers(&selector, task.cover_mode).await?;

        if server_ids.is_empty() {
            warn!("No servers matched for task {}", task.id);
            return Ok(());
        }

        info!(
            "Task {} will run on {} server(s)",
            task.id,
            server_ids.len()
        );

        // TODO: Send task to agents via gRPC
        // For now, just record that we would execute
        for server_id in server_ids {
            let run = TaskRun {
                id: format!("run_{}", uuid::Uuid::now_v7()),
                task_id: task.id.clone(),
                server_id: server_id.clone(),
                status: TaskStatus::Success,
                delay_ms: Some(0),
                output: Some("Task dispatched (implementation pending)".to_string()),
                output_truncated: false,
                error: None,
                created_at: Utc::now().to_rfc3339(),
            };

            TaskRunRepository::create(&self.db, &run).await?;
        }

        // Update last execution time
        TaskRepository::update_last_execution(
            &self.db,
            &task.id,
            &Utc::now().to_rfc3339(),
            "dispatched",
        )
        .await?;

        Ok(())
    }

    /// Resolve server IDs based on selector
    async fn resolve_servers(
        &self,
        selector: &ServerSelector,
        cover_mode: CoverMode,
    ) -> Result<Vec<String>> {
        // If specific servers are selected, use them
        if !selector.server_ids.is_empty() {
            return Ok(selector.server_ids.clone());
        }

        // TODO: Implement group and tag-based selection
        // For now, return empty list
        Ok(Vec::new())
    }

    /// Manually trigger a task execution
    pub async fn trigger_task(&self, task_id: &str) -> Result<()> {
        let task = TaskRepository::get_by_id(&self.db, task_id)
            .await?
            .context("Task not found")?;

        info!("Manually triggering task: {} ({})", task.id, task.name);
        self.execute_task(&task).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_schedule_parsing() {
        // Every hour
        let schedule = Schedule::from_str("0 * * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert!(next > Utc::now());

        // Every day at midnight
        let schedule = Schedule::from_str("0 0 * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert!(next > Utc::now());

        // Every 5 minutes
        let schedule = Schedule::from_str("*/5 * * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert!(next > Utc::now());
    }

    #[test]
    fn test_invalid_cron() {
        let result = Schedule::from_str("invalid");
        assert!(result.is_err());
    }
}
