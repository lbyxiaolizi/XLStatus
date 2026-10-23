#![allow(dead_code)]
#![allow(unused_imports)]

use crate::db::Db;
use anyhow::{Context, Result};
use chrono::Utc;
use futures::{stream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration, MissedTickBehavior};
use tracing::{error, info, warn};
use xlstatus_shared::tasks::*;

use crate::db::repository::tasks::TaskRepository;
use crate::grpc::{SessionRegistry, TaskResponseRegistry};
use crate::tasks::{
    dispatch_task_to_agents, parse_task_schedule, TASK_SCHEDULED_RUNTIME_SCAN_LIMIT,
};

const TASK_SCHEDULED_MAX_CONCURRENT_EXECUTIONS: usize = 8;

/// Task scheduler that runs scheduled tasks
#[allow(dead_code)]
pub struct TaskScheduler {
    db: Db,
    session_registry: SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    // Map of task_id -> the expression and next execution time observed by
    // this process. The expression is retained so edits are rescheduled.
    scheduled_tasks: Arc<RwLock<HashMap<String, ScheduledTaskState>>>,
}

#[derive(Debug, Clone)]
struct ScheduledTaskState {
    expression: String,
    // None means that this finite schedule has no occurrence after the last
    // time it was evaluated. It remains dormant until its expression changes.
    next_run: Option<chrono::DateTime<Utc>>,
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
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

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
        let due_tasks = self.scan_due_tasks(now).await?;

        stream::iter(due_tasks)
            .for_each_concurrent(
                TASK_SCHEDULED_MAX_CONCURRENT_EXECUTIONS,
                |task| async move {
                    info!("Executing scheduled task: {} ({})", task.id, task.name);
                    if let Err(e) = self.execute_task(&task).await {
                        error!("Failed to execute task {}: {}", task.id, e);
                    }
                },
            )
            .await;

        Ok(())
    }

    /// Load and advance every scheduled-task page before starting remote work.
    async fn scan_due_tasks(&self, now: chrono::DateTime<Utc>) -> Result<Vec<Task>> {
        // Work against a snapshot so a later-page database error neither
        // removes unseen tasks nor advances already-due tasks without running
        // them. This scheduler is the sole writer in production.
        let mut scheduled = self.scheduled_tasks.read().await.clone();
        let mut observed_task_ids = std::collections::HashSet::new();
        let mut due_tasks = Vec::new();
        let mut cursor = None;

        loop {
            let page = TaskRepository::list_scheduled_page(
                &self.db,
                cursor.as_deref(),
                TASK_SCHEDULED_RUNTIME_SCAN_LIMIT,
            )
            .await
            .context("Failed to load scheduled task page")?;

            for task in page.tasks {
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
                observed_task_ids.insert(task.id.clone());

                // A newly observed task is scheduled for its next cron occurrence,
                // not executed immediately. This avoids running every remote task
                // whenever the Server process restarts.
                if update_schedule_state(&mut scheduled, &task.id, schedule_str, &schedule, now) {
                    due_tasks.push(task);
                }
            }

            match page.next_cursor {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => break,
            }
        }

        // Only commit state after every page was loaded successfully. A
        // transient error on a later page must not make older tasks look
        // deleted or advance a due task that was never dispatched.
        scheduled.retain(|task_id, _| observed_task_ids.contains(task_id));
        *self.scheduled_tasks.write().await = scheduled;

        Ok(due_tasks)
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
        let task_id = crate::api::v1::tasks::require_task_uuid_text(task_id, "task_id")
            .map_err(|_| anyhow::anyhow!("Invalid task_id"))?;
        let task = TaskRepository::get_by_id(&self.db, &task_id)
            .await?
            .context("Task not found")?;
        if !task.enabled {
            anyhow::bail!("Task is disabled");
        }

        info!("Manually triggering task: {} ({})", task.id, task.name);
        self.execute_task(&task).await
    }
}

fn update_schedule_state(
    scheduled: &mut HashMap<String, ScheduledTaskState>,
    task_id: &str,
    expression: &str,
    schedule: &cron::Schedule,
    now: chrono::DateTime<Utc>,
) -> bool {
    let existing = scheduled.get(task_id);
    let expression_changed = existing.is_some_and(|state| state.expression != expression);
    let first_observation = existing.is_none() || expression_changed;
    let should_run = existing.is_some_and(|state| {
        !expression_changed && state.next_run.is_some_and(|next_run| now >= next_run)
    });

    if first_observation || should_run {
        let next_run = schedule.after(&now).next();
        scheduled.insert(
            task_id.to_string(),
            ScheduledTaskState {
                expression: expression.to_string(),
                next_run,
            },
        );
    }

    should_run
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

    #[test]
    fn first_observation_and_schedule_edits_do_not_run_immediately() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let every_hour = parse_task_schedule("0 * * * *").unwrap();
        let every_minute = parse_task_schedule("* * * * *").unwrap();
        let mut scheduled = HashMap::new();

        assert!(!update_schedule_state(
            &mut scheduled,
            "task-1",
            "0 * * * *",
            &every_hour,
            now,
        ));
        let hourly_next = scheduled["task-1"].next_run.unwrap();
        assert!(hourly_next > now);
        assert!(!update_schedule_state(
            &mut scheduled,
            "task-1",
            "0 * * * *",
            &every_hour,
            now,
        ));

        assert!(!update_schedule_state(
            &mut scheduled,
            "task-1",
            "* * * * *",
            &every_minute,
            now,
        ));
        assert!(scheduled["task-1"].next_run.unwrap() < hourly_next);
    }

    #[test]
    fn due_schedule_runs_once_and_advances() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let schedule = parse_task_schedule("* * * * *").unwrap();
        let mut scheduled = HashMap::from([(
            "task-1".to_string(),
            ScheduledTaskState {
                expression: "* * * * *".to_string(),
                next_run: Some(now - chrono::Duration::seconds(1)),
            },
        )]);

        assert!(update_schedule_state(
            &mut scheduled,
            "task-1",
            "* * * * *",
            &schedule,
            now,
        ));
        assert!(scheduled["task-1"]
            .next_run
            .is_some_and(|next_run| next_run > now));
        assert!(!update_schedule_state(
            &mut scheduled,
            "task-1",
            "* * * * *",
            &schedule,
            now,
        ));
    }

    #[test]
    fn expired_finite_schedule_remains_dormant() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let expression = "0 0 0 1 1 * 2025";
        let schedule = parse_task_schedule(expression).unwrap();
        let mut scheduled = HashMap::new();

        assert!(!update_schedule_state(
            &mut scheduled,
            "expired-task",
            expression,
            &schedule,
            now,
        ));
        assert_eq!(scheduled["expired-task"].next_run, None);

        // A later scheduler tick must not synthesize a due time and run the
        // task periodically after the finite cron expression has expired.
        assert!(!update_schedule_state(
            &mut scheduled,
            "expired-task",
            expression,
            &schedule,
            now + chrono::Duration::days(30),
        ));
        assert_eq!(scheduled["expired-task"].next_run, None);
    }

    #[tokio::test]
    async fn second_page_due_task_dispatches_before_first_page_task_replies() {
        let db = test_db().await;
        let owner_id = "00000000-0000-0000-0000-000000000001";
        let agent_id_text = "00000000-0000-0000-0000-000000000201";
        let task_ids = seed_scheduled_tasks(
            &db,
            owner_id,
            agent_id_text,
            TASK_SCHEDULED_RUNTIME_SCAN_LIMIT as usize + 1,
        )
        .await;
        let first_page_due_id = task_ids.last().unwrap().clone();
        let second_page_due_id = task_ids.first().unwrap().clone();

        let sessions = SessionRegistry::new();
        let agent_id = xlstatus_shared::AgentId(uuid::Uuid::parse_str(agent_id_text).unwrap());
        let (session_tx, mut session_rx) = tokio::sync::mpsc::channel(4);
        assert!(sessions.register(agent_id, session_tx).await);
        let responses = Arc::new(TaskResponseRegistry::new());
        let scheduler = Arc::new(TaskScheduler::new(db, sessions, responses.clone()));

        let due_at = Utc::now() - chrono::Duration::seconds(1);
        {
            let mut scheduled = scheduler.scheduled_tasks.write().await;
            for task_id in [&first_page_due_id, &second_page_due_id] {
                scheduled.insert(
                    task_id.clone(),
                    ScheduledTaskState {
                        expression: "* * * * *".to_string(),
                        next_run: Some(due_at),
                    },
                );
            }
        }

        let running_scheduler = scheduler.clone();
        let run = tokio::spawn(async move { running_scheduler.check_and_run_tasks().await });
        let messages = tokio::time::timeout(Duration::from_secs(5), async {
            let first = session_rx
                .recv()
                .await
                .expect("first due task should be dispatched")
                .expect("first due task message should be valid");
            let second = session_rx
                .recv()
                .await
                .expect("second-page due task should be dispatched")
                .expect("second-page due task message should be valid");
            [first, second]
        })
        .await
        .expect("a slow first-page task must not block second-page dispatch");

        let dispatched = messages.map(dispatched_shell_task);
        let commands = dispatched
            .iter()
            .map(|(_, command)| command.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            commands,
            std::collections::HashSet::from([
                first_page_due_id.as_str(),
                second_page_due_id.as_str(),
            ])
        );
        assert!(!run.is_finished(), "both remote replies are still pending");
        assert_eq!(
            scheduler.scheduled_tasks.read().await.len(),
            task_ids.len(),
            "the full keyset scan must finish before remote execution waits"
        );

        for (run_id, _) in dispatched {
            let result_task_id = run_id.clone();
            assert!(
                responses
                    .deliver(
                        &agent_id,
                        &run_id,
                        xlstatus_proto_gen::xlstatus::v1::TaskResult {
                            task_id: result_task_id,
                            status: xlstatus_proto_gen::xlstatus::v1::TaskOutcome::Success as i32,
                            ..Default::default()
                        },
                    )
                    .await
            );
        }

        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("scheduler should finish after both replies")
            .expect("scheduler task should not panic")
            .expect("scheduler run should succeed");
    }

    #[tokio::test]
    async fn trigger_task_rejects_invalid_uuid_and_disabled_task() {
        let db = test_db().await;
        let owner_id = "00000000-0000-0000-0000-000000000001";
        let agent_id_text = "00000000-0000-0000-0000-000000000201";
        let task_ids = seed_scheduled_tasks(&db, owner_id, agent_id_text, 1).await;
        let task_id = task_ids.first().unwrap().clone();

        let Db::Sqlite(pool) = &db else {
            unreachable!();
        };
        sqlx::query("UPDATE tasks SET enabled = 0 WHERE id = ?")
            .bind(&task_id)
            .execute(pool)
            .await
            .unwrap();

        let scheduler = TaskScheduler::new(
            db,
            SessionRegistry::new(),
            Arc::new(TaskResponseRegistry::new()),
        );

        let bad_id_err = scheduler.trigger_task("not-a-uuid").await.unwrap_err();
        assert!(bad_id_err.to_string().contains("Invalid task_id"));

        let disabled_err = scheduler.trigger_task(&task_id).await.unwrap_err();
        assert!(disabled_err.to_string().contains("Task is disabled"));
    }

    async fn test_db() -> Db {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-task-scheduler-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = Db::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_scheduled_tasks(
        db: &Db,
        owner_id: &str,
        agent_id: &str,
        task_count: usize,
    ) -> Vec<String> {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, 'scheduler-owner', 'hash', 'member', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(owner_id)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at) VALUES (?, 'scheduler-agent', 'pk', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(agent_id)
        .bind(owner_id)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO servers (id, owner_user_id, name, created_at, updated_at, agent_id) VALUES (?, ?, 'scheduler-server', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?)",
        )
        .bind(agent_id)
        .bind(owner_id)
        .bind(agent_id)
        .execute(pool)
        .await
        .unwrap();

        let selector_json = serde_json::to_string(&ServerSelector {
            server_ids: vec![agent_id.to_string()],
            ..ServerSelector::default()
        })
        .unwrap();
        let task_ids = (0..task_count)
            .map(|index| format!("00000000-0000-0000-0000-{index:012}"))
            .collect::<Vec<_>>();
        let mut transaction = pool.begin().await.unwrap();
        for task_id in &task_ids {
            sqlx::query(
                "INSERT INTO tasks (id, owner_user_id, name, task_type, schedule, command, cover_mode, server_selector_json, enabled, push_successful, created_at, updated_at) VALUES (?, ?, 'scheduled', '\"shell\"', '* * * * *', ?, '\"specific\"', ?, 1, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            )
            .bind(task_id)
            .bind(owner_id)
            .bind(task_id)
            .bind(&selector_json)
            .execute(&mut *transaction)
            .await
            .unwrap();
        }
        transaction.commit().await.unwrap();
        task_ids
    }

    fn dispatched_shell_task(
        message: xlstatus_proto_gen::xlstatus::v1::ServerMessage,
    ) -> (String, String) {
        let Some(xlstatus_proto_gen::xlstatus::v1::server_message::Payload::Task(task)) =
            message.payload
        else {
            panic!("expected dispatched task message");
        };
        let Some(xlstatus_proto_gen::xlstatus::v1::server_task::Spec::ShellCommand(shell)) =
            task.spec
        else {
            panic!("expected shell task");
        };
        (task.task_id, shell.command)
    }
}
