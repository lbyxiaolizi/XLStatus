#![allow(dead_code)]
#![allow(unused_imports)]

use crate::db::Db;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};
use xlstatus_shared::tasks::*;

use crate::db::repository::tasks::TaskRepository;
use crate::grpc::{SessionRegistry, TaskResponseRegistry};
use crate::tasks::{dispatch_task_to_agents, parse_task_schedule};

/// Task scheduler that runs scheduled tasks
#[allow(dead_code)]
pub struct TaskScheduler {
    db: Db,
    session_registry: SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    // Map of task_id -> next execution time
    scheduled_tasks: Arc<RwLock<HashMap<String, chrono::DateTime<Utc>>>>,
}

impl TaskScheduler {
    pub fn new(
        db: Db,
        session_registry: SessionRegistry,
        response_registry: Arc<TaskResponseRegistry>,
    ) -> Self {
        Self {
            db,
            session_registry,
            response_registry,
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
            let schedule = match parse_task_schedule(schedule_str) {
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
        let report = dispatch_task_to_agents(
            &self.db,
            &self.session_registry,
            self.response_registry.clone(),
            task,
        )
        .await?;
        info!(
            "Scheduled task {} completed: success={}, failure={}, offline={}, timeout={}, total={}",
            task.id,
            report.summary.success,
            report.summary.failure,
            report.summary.offline,
            report.summary.timeout,
            report.summary.total
        );

        Ok(())
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
        let schedule = parse_task_schedule("0 * * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert!(next > Utc::now());

        // Every day at midnight
        let schedule = parse_task_schedule("0 0 * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert!(next > Utc::now());

        // Every 5 minutes
        let schedule = parse_task_schedule("*/5 * * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert!(next > Utc::now());

        // Native six-field syntax is also accepted.
        let schedule = parse_task_schedule("0 */5 * * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert!(next > Utc::now());
    }

    #[test]
    fn test_invalid_cron() {
        let result = parse_task_schedule("invalid");
        assert!(result.is_err());
    }
}
