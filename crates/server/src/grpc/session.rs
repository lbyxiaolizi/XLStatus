use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::{mpsc, watch, RwLock};
use tokio::time::{timeout, Duration};
use xlstatus_proto_gen::xlstatus::v1::{
    io_frame, IoError, IoFrame, ServerMessage, ServerTask, TaskResult,
};
use xlstatus_shared::AgentId;

pub type SessionSender = mpsc::Sender<Result<ServerMessage, tonic::Status>>;
pub type IoSender = mpsc::Sender<Result<IoFrame, tonic::Status>>;

const REGISTRY_SEND_TIMEOUT: Duration = Duration::from_secs(5);
const IO_SUBSCRIBER_SEND_TIMEOUT: Duration = Duration::from_secs(5);
const IO_STREAM_CLOSED_CODE: &str = "agent_io_stream_closed";

struct RegisteredSender<T> {
    sender: mpsc::Sender<T>,
    changes: watch::Sender<u64>,
}

impl<T> RegisteredSender<T> {
    fn new(sender: mpsc::Sender<T>) -> Self {
        let (changes, _) = watch::channel(0);
        Self { sender, changes }
    }

    fn notify_changed(&self) {
        self.changes.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }
}

async fn send_to_current<T>(
    senders: &RwLock<HashMap<AgentId, RegisteredSender<T>>>,
    revoked_agents: &RwLock<HashSet<AgentId>>,
    agent_id: &AgentId,
    value: T,
    missing_error: &'static str,
    closed_error: &'static str,
    timeout_error: &'static str,
) -> Result<(), String> {
    timeout(REGISTRY_SEND_TIMEOUT, async {
        loop {
            let (sender, mut changes) = {
                let revoked = revoked_agents.read().await;
                if revoked.contains(agent_id) {
                    return Err("Agent has been revoked".to_string());
                }
                let current = senders.read().await;
                let current = current
                    .get(agent_id)
                    .ok_or_else(|| missing_error.to_string())?;
                (current.sender.clone(), current.changes.subscribe())
            };

            let reservation = sender.clone().reserve_owned();
            tokio::pin!(reservation);
            let permit = tokio::select! {
                result = &mut reservation => match result {
                    Ok(permit) => permit,
                    Err(_) => {
                        let revoked = revoked_agents.read().await;
                        if revoked.contains(agent_id) {
                            return Err("Agent has been revoked".to_string());
                        }
                        let current = senders.read().await;
                        match current.get(agent_id) {
                            Some(current) if !current.sender.same_channel(&sender) => continue,
                            Some(_) => return Err(closed_error.to_string()),
                            None => return Err(missing_error.to_string()),
                        }
                    }
                },
                changed = changes.changed() => {
                    if changed.is_ok() {
                        continue;
                    }
                    return Err(missing_error.to_string());
                }
            };

            // The sender may have been replaced while capacity was unavailable.
            // Validate its identity under the registry lock before committing the
            // message so a reconnect cannot receive work through its stale stream.
            let revoked = revoked_agents.read().await;
            if revoked.contains(agent_id) {
                return Err("Agent has been revoked".to_string());
            }
            let current = senders.read().await;
            match current.get(agent_id) {
                Some(current) if current.sender.same_channel(&sender) => {
                    permit.send(value);
                    return Ok(());
                }
                Some(_) => continue,
                None => return Err(missing_error.to_string()),
            }
        }
    })
    .await
    .unwrap_or_else(|_| Err(timeout_error.to_string()))
}

#[derive(Clone)]
pub struct SessionRegistry {
    sessions: Arc<RwLock<HashMap<AgentId, RegisteredSender<Result<ServerMessage, tonic::Status>>>>>,
    revoked: Arc<RwLock<HashSet<AgentId>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            revoked: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub async fn register(&self, agent_id: AgentId, sender: SessionSender) -> bool {
        // Keep the revocation read guard until after insertion so a concurrent
        // disconnect cannot mark the agent revoked and then lose the race to a
        // late registration.
        let revoked = self.revoked.read().await;
        if revoked.contains(&agent_id) {
            tracing::warn!("refusing to register revoked Agent {} session", agent_id);
            return false;
        }
        let mut sessions = self.sessions.write().await;
        if let Some(previous) = sessions.insert(agent_id, RegisteredSender::new(sender)) {
            previous.notify_changed();
        }
        tracing::info!("Agent {} session registered", agent_id);
        true
    }

    pub async fn unregister(
        &self,
        agent_id: &AgentId,
        sender: &mpsc::WeakSender<Result<ServerMessage, tonic::Status>>,
    ) -> bool {
        let mut sessions = self.sessions.write().await;
        let is_current = sessions.get(agent_id).is_some_and(|current| {
            sender
                .upgrade()
                .is_some_and(|sender| current.sender.same_channel(&sender))
        });
        if is_current {
            if let Some(current) = sessions.remove(agent_id) {
                current.notify_changed();
            }
            tracing::info!("Agent {} session unregistered", agent_id);
        } else {
            tracing::debug!("ignored stale Agent {} session cleanup", agent_id);
        }
        is_current
    }

    pub async fn disconnect(&self, agent_id: &AgentId) {
        self.revoked.write().await.insert(*agent_id);
        let removed = self.sessions.write().await.remove(agent_id);
        if let Some(current) = &removed {
            current.notify_changed();
        }
        if removed.is_some() {
            tracing::info!("Agent {} session disconnected", agent_id);
        }
    }

    pub async fn send(&self, agent_id: &AgentId, message: ServerMessage) -> Result<(), String> {
        send_to_current(
            &self.sessions,
            &self.revoked,
            agent_id,
            Ok(message),
            "Agent session not found",
            "Agent session channel is closed",
            "Timed out waiting for Agent session capacity",
        )
        .await
    }

    pub async fn broadcast(&self, message: ServerMessage) {
        let senders = self
            .sessions
            .read()
            .await
            .values()
            .map(|current| current.sender.clone())
            .collect::<Vec<_>>();
        for sender in senders {
            let _ = sender.send(Ok(message.clone())).await;
        }
    }

    /// M5: send a `ServerMessage::Task` to a specific agent's live
    /// session. Returns `Err` if there is no live session for that
    /// agent (the caller can then record an "offline" run).
    pub async fn send_task(
        &self,
        agent_id: &AgentId,
        task_id: &str,
        shell_command: &str,
        timeout_seconds: u32,
    ) -> Result<(), String> {
        self.send_shell_task(agent_id, task_id, shell_command, timeout_seconds, 64 * 1024)
            .await
    }

    /// Send a shell task with a caller-selected stdout/stderr byte cap.
    /// This keeps normal task output modest while allowing transfer
    /// endpoints to read larger files through the existing task channel.
    pub async fn send_shell_task(
        &self,
        agent_id: &AgentId,
        task_id: &str,
        shell_command: &str,
        timeout_seconds: u32,
        max_output_bytes: u64,
    ) -> Result<(), String> {
        use xlstatus_proto_gen::xlstatus::v1::{
            server_message::Payload, server_task::Spec, ServerMessage, ServerTask,
            ShellCommandTask, TaskType,
        };
        let msg = ServerMessage {
            payload: Some(Payload::Task(ServerTask {
                task_id: task_id.to_string(),
                task_type: TaskType::ShellCommand as i32,
                spec: Some(Spec::ShellCommand(ShellCommandTask {
                    command: shell_command.to_string(),
                    working_dir: String::new(),
                    env: std::collections::HashMap::new(),
                    timeout_seconds,
                    max_output_bytes,
                })),
            })),
        };
        self.send(agent_id, msg).await
    }

    pub async fn send_server_task(
        &self,
        agent_id: &AgentId,
        task: ServerTask,
    ) -> Result<(), String> {
        use xlstatus_proto_gen::xlstatus::v1::{server_message::Payload, ServerMessage};
        self.send(
            agent_id,
            ServerMessage {
                payload: Some(Payload::Task(task)),
            },
        )
        .await
    }

    pub async fn count(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.len()
    }

    /// Returns the set of agent IDs that have a live session.
    pub async fn online_agent_ids(&self) -> Vec<AgentId> {
        let sessions = self.sessions.read().await;
        sessions.keys().copied().collect()
    }

    /// Returns true if a session exists for `agent_id`.
    pub async fn is_online(&self, agent_id: &AgentId) -> bool {
        let sessions = self.sessions.read().await;
        sessions.contains_key(agent_id)
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[tokio::test]
    async fn session_disconnect_blocks_late_task_sends_and_reregister() {
        let registry = SessionRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (tx, mut rx) = mpsc::channel(1);
        registry.register(agent_id, tx).await;

        registry.disconnect(&agent_id).await;
        assert!(!registry.is_online(&agent_id).await);
        assert!(registry
            .send_task(&agent_id, "task-1", "true", 1)
            .await
            .unwrap_err()
            .contains("revoked"));
        assert!(rx.try_recv().is_err());

        let (tx, mut rx) = mpsc::channel(1);
        registry.register(agent_id, tx).await;
        assert!(!registry.is_online(&agent_id).await);
        assert!(registry
            .send_task(&agent_id, "task-2", "true", 1)
            .await
            .unwrap_err()
            .contains("revoked"));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn stale_session_cleanup_does_not_unregister_replacement() {
        let registry = SessionRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (old_tx, mut old_rx) = mpsc::channel(1);
        let (new_tx, mut new_rx) = mpsc::channel(1);
        let old_registration = old_tx.downgrade();
        let new_registration = new_tx.downgrade();
        assert!(registry.register(agent_id, old_tx.clone()).await);
        assert!(registry.register(agent_id, new_tx.clone()).await);

        assert!(!registry.unregister(&agent_id, &old_registration).await);
        registry
            .send_task(&agent_id, "replacement-task", "true", 1)
            .await
            .unwrap();

        assert!(old_rx.try_recv().is_err());
        assert!(new_rx.try_recv().is_ok());
        assert!(registry.unregister(&agent_id, &new_registration).await);
        assert!(!registry.is_online(&agent_id).await);
    }

    #[tokio::test]
    async fn blocked_session_send_retargets_replacement() {
        let registry = SessionRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (old_tx, mut old_rx) = mpsc::channel(1);
        assert!(registry.register(agent_id, old_tx.clone()).await);
        old_tx.send(Ok(ServerMessage::default())).await.unwrap();

        let pending_registry = registry.clone();
        let pending = tokio::spawn(async move {
            pending_registry
                .send_task(&agent_id, "replacement-task", "true", 1)
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!pending.is_finished());

        let (new_tx, mut new_rx) = mpsc::channel(1);
        assert!(registry.register(agent_id, new_tx).await);
        tokio::time::timeout(Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        assert!(new_rx.try_recv().unwrap().unwrap().payload.is_some());
        assert!(old_rx.try_recv().unwrap().unwrap().payload.is_none());
        assert!(old_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn blocked_session_send_wakes_on_disconnect() {
        let registry = SessionRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (tx, mut rx) = mpsc::channel(1);
        assert!(registry.register(agent_id, tx.clone()).await);
        tx.send(Ok(ServerMessage::default())).await.unwrap();

        let pending_registry = registry.clone();
        let pending = tokio::spawn(async move {
            pending_registry
                .send_task(&agent_id, "revoked-task", "true", 1)
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!pending.is_finished());

        registry.disconnect(&agent_id).await;
        let error = tokio::time::timeout(Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.contains("revoked"));
        assert!(rx.try_recv().unwrap().unwrap().payload.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn replacing_only_registered_sender_closes_old_stream() {
        let registry = SessionRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (old_tx, mut old_rx) = mpsc::channel(1);
        let old_registration = old_tx.downgrade();
        assert!(registry.register(agent_id, old_tx).await);

        let (new_tx, _new_rx) = mpsc::channel(1);
        assert!(registry.register(agent_id, new_tx).await);

        assert!(tokio::time::timeout(Duration::from_secs(1), old_rx.recv())
            .await
            .unwrap()
            .is_none());
        assert!(!registry.unregister(&agent_id, &old_registration).await);
    }

    #[tokio::test]
    async fn io_disconnect_blocks_late_frame_sends_and_reregister() {
        let registry = IoRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (tx, mut rx) = mpsc::channel(1);
        registry.register_agent(agent_id, tx).await;
        let mut subscriber = registry
            .subscribe_stream("stream-before-revoke".to_string(), agent_id)
            .await;

        registry.disconnect_agent(&agent_id).await;
        assert!(!registry.is_agent_online(&agent_id).await);
        assert_eq!(registry.subscriber_count().await, 0);
        let closed = subscriber.recv().await.unwrap();
        assert!(matches!(
            closed.payload,
            Some(io_frame::Payload::Error(IoError { ref code, .. }))
                if code == IO_STREAM_CLOSED_CODE
        ));
        assert!(subscriber.recv().await.is_none());
        assert!(registry
            .send_to_agent(
                &agent_id,
                IoFrame {
                    stream_id: "stream-1".into(),
                    sequence: 1,
                    agent_id: agent_id.0.to_string(),
                    payload: None,
                },
            )
            .await
            .unwrap_err()
            .contains("revoked"));
        assert!(rx.try_recv().is_err());

        let (tx, mut rx) = mpsc::channel(1);
        registry.register_agent(agent_id, tx).await;
        assert!(!registry.is_agent_online(&agent_id).await);
        assert!(registry
            .send_to_agent(
                &agent_id,
                IoFrame {
                    stream_id: "stream-2".into(),
                    sequence: 1,
                    agent_id: agent_id.0.to_string(),
                    payload: None,
                },
            )
            .await
            .unwrap_err()
            .contains("revoked"));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn replacing_io_stream_closes_existing_subscribers_only() {
        let registry = IoRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (old_tx, _old_rx) = mpsc::channel(1);
        let old_registration = old_tx.downgrade();
        assert!(registry.register_agent(agent_id, old_tx).await);
        let mut old_subscriber = registry
            .subscribe_stream("old-stream".to_string(), agent_id)
            .await;

        let (new_tx, _new_rx) = mpsc::channel(1);
        let new_registration = new_tx.downgrade();
        assert!(registry.register_agent(agent_id, new_tx).await);
        let closed = old_subscriber.recv().await.unwrap();
        assert!(matches!(
            closed.payload,
            Some(io_frame::Payload::Error(IoError { ref code, .. }))
                if code == IO_STREAM_CLOSED_CODE
        ));
        assert!(old_subscriber.recv().await.is_none());

        let mut current_subscriber = registry
            .subscribe_stream("current-stream".to_string(), agent_id)
            .await;
        assert!(
            !registry
                .unregister_agent(&agent_id, &old_registration)
                .await
        );
        assert_eq!(registry.subscriber_count().await, 1);
        assert!(
            registry
                .deliver_from_agent(
                    &agent_id,
                    IoFrame {
                        stream_id: "current-stream".into(),
                        sequence: 7,
                        ..IoFrame::default()
                    },
                )
                .await
        );
        assert_eq!(current_subscriber.recv().await.unwrap().sequence, 7);

        assert!(
            registry
                .unregister_agent(&agent_id, &new_registration)
                .await
        );
        assert!(current_subscriber.recv().await.is_some());
        assert!(current_subscriber.recv().await.is_none());
        assert_eq!(registry.subscriber_count().await, 0);
    }

    #[tokio::test]
    async fn concurrent_subscribe_and_disconnect_leaves_no_stale_subscribers() {
        let registry = IoRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (tx, _rx) = mpsc::channel(1);
        assert!(registry.register_agent(agent_id, tx).await);
        let barrier = Arc::new(tokio::sync::Barrier::new(17));
        let mut subscriptions = Vec::new();
        for index in 0..16 {
            let registry = registry.clone();
            let barrier = barrier.clone();
            subscriptions.push(tokio::spawn(async move {
                barrier.wait().await;
                registry
                    .subscribe_stream(format!("racing-stream-{index}"), agent_id)
                    .await
            }));
        }

        barrier.wait().await;
        registry.disconnect_agent(&agent_id).await;
        let mut receivers = Vec::new();
        for subscription in subscriptions {
            receivers.push(subscription.await.unwrap());
        }
        assert_eq!(registry.subscriber_count().await, 0);
        for mut receiver in receivers {
            while receiver.recv().await.is_some() {}
        }
    }

    #[tokio::test]
    async fn full_subscriber_queue_wakes_immediately_on_disconnect() {
        let registry = IoRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (tx, _rx) = mpsc::channel(1);
        assert!(registry.register_agent(agent_id, tx).await);
        let mut receiver = registry
            .subscribe_stream("full-stream".to_string(), agent_id)
            .await;
        let sender = registry
            .stream_subscribers
            .read()
            .await
            .get("full-stream")
            .unwrap()
            .sender
            .clone();
        for sequence in 0..128 {
            sender
                .try_send(IoFrame {
                    stream_id: "full-stream".into(),
                    sequence,
                    ..IoFrame::default()
                })
                .unwrap();
        }
        drop(sender);

        let blocked_registry = registry.clone();
        let blocked_delivery = tokio::spawn(async move {
            blocked_registry
                .deliver_from_agent(
                    &agent_id,
                    IoFrame {
                        stream_id: "full-stream".into(),
                        sequence: 129,
                        ..IoFrame::default()
                    },
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!blocked_delivery.is_finished());

        registry.disconnect_agent(&agent_id).await;
        assert!(
            !tokio::time::timeout(Duration::from_secs(1), blocked_delivery)
                .await
                .expect("disconnect should wake a blocked subscriber delivery")
                .unwrap()
        );
        while receiver.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn stale_io_cleanup_does_not_unregister_replacement() {
        let registry = IoRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (old_tx, mut old_rx) = mpsc::channel(1);
        let (new_tx, mut new_rx) = mpsc::channel(1);
        let old_registration = old_tx.downgrade();
        let new_registration = new_tx.downgrade();
        assert!(registry.register_agent(agent_id, old_tx.clone()).await);
        assert!(registry.register_agent(agent_id, new_tx.clone()).await);

        assert!(
            !registry
                .unregister_agent(&agent_id, &old_registration)
                .await
        );
        registry
            .send_to_agent(
                &agent_id,
                IoFrame {
                    stream_id: "replacement-stream".into(),
                    sequence: 1,
                    agent_id: agent_id.0.to_string(),
                    payload: None,
                },
            )
            .await
            .unwrap();

        assert!(old_rx.try_recv().is_err());
        assert!(new_rx.try_recv().is_ok());
        assert!(
            registry
                .unregister_agent(&agent_id, &new_registration)
                .await
        );
        assert!(!registry.is_agent_online(&agent_id).await);
    }

    #[tokio::test]
    async fn blocked_io_send_retargets_replacement() {
        let registry = IoRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (old_tx, mut old_rx) = mpsc::channel(1);
        assert!(registry.register_agent(agent_id, old_tx.clone()).await);
        old_tx.send(Ok(IoFrame::default())).await.unwrap();

        let pending_registry = registry.clone();
        let pending = tokio::spawn(async move {
            pending_registry
                .send_to_agent(
                    &agent_id,
                    IoFrame {
                        stream_id: "replacement-stream".into(),
                        ..IoFrame::default()
                    },
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!pending.is_finished());

        let (new_tx, mut new_rx) = mpsc::channel(1);
        assert!(registry.register_agent(agent_id, new_tx).await);
        tokio::time::timeout(Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        assert_eq!(
            new_rx.try_recv().unwrap().unwrap().stream_id,
            "replacement-stream"
        );
        assert!(old_rx.try_recv().unwrap().unwrap().stream_id.is_empty());
        assert!(old_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn io_delivery_requires_matching_agent() {
        let registry = IoRegistry::new();
        let expected_agent = AgentId(uuid::Uuid::now_v7());
        let other_agent = AgentId(uuid::Uuid::now_v7());
        let (tx, _io_rx) = mpsc::channel(1);
        assert!(registry.register_agent(expected_agent, tx).await);
        let mut rx = registry
            .subscribe_stream("stream-1".to_string(), expected_agent)
            .await;

        assert!(
            !registry
                .deliver_from_agent(
                    &other_agent,
                    IoFrame {
                        stream_id: "stream-1".into(),
                        sequence: 1,
                        agent_id: other_agent.0.to_string(),
                        payload: None,
                    },
                )
                .await
        );
        assert!(rx.try_recv().is_err());

        assert!(
            registry
                .deliver_from_agent(
                    &expected_agent,
                    IoFrame {
                        stream_id: "stream-1".into(),
                        sequence: 2,
                        agent_id: expected_agent.0.to_string(),
                        payload: None,
                    },
                )
                .await
        );
        let delivered = rx.try_recv().unwrap();
        assert_eq!(delivered.sequence, 2);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskResultTextTruncation {
    pub stdout: bool,
    pub stderr: bool,
    pub error: bool,
}

impl TaskResultTextTruncation {
    pub fn any(self) -> bool {
        self.stdout || self.stderr || self.error
    }
}

pub const fn base64_encoded_len(decoded_len: usize) -> usize {
    let full = decoded_len / 3;
    let rem = decoded_len % 3;
    full.saturating_mul(4)
        .saturating_add(if rem == 0 { 0 } else { 4 })
}

pub fn ensure_task_result_text_within(
    result: &TaskResult,
    stdout_max: usize,
    stderr_max: usize,
    error_max: usize,
    context: &str,
) -> Result<(), String> {
    if result.stdout.len() > stdout_max {
        return Err(format!("{context} stdout exceeds {stdout_max} bytes"));
    }
    if result.stderr.len() > stderr_max {
        return Err(format!("{context} stderr exceeds {stderr_max} bytes"));
    }
    if result.error.len() > error_max {
        return Err(format!("{context} error exceeds {error_max} bytes"));
    }
    Ok(())
}

pub fn truncate_task_result_text(
    result: &mut TaskResult,
    stdout_max: usize,
    stderr_max: usize,
    error_max: usize,
) -> TaskResultTextTruncation {
    let (stdout, stdout_truncated) = truncate_utf8_to_bytes(&result.stdout, stdout_max);
    let (stderr, stderr_truncated) = truncate_utf8_to_bytes(&result.stderr, stderr_max);
    let (error, error_truncated) = truncate_utf8_to_bytes(&result.error, error_max);
    result.stdout = stdout;
    result.stderr = stderr;
    result.error = error;
    TaskResultTextTruncation {
        stdout: stdout_truncated,
        stderr: stderr_truncated,
        error: error_truncated,
    }
}

pub fn truncate_utf8_to_bytes(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    if max_bytes == 0 {
        return (String::new(), true);
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

#[derive(Clone)]
pub struct IoRegistry {
    agent_streams: Arc<RwLock<HashMap<AgentId, RegisteredSender<Result<IoFrame, tonic::Status>>>>>,
    stream_subscribers: Arc<RwLock<HashMap<String, IoSubscriber>>>,
    revoked_agents: Arc<RwLock<HashSet<AgentId>>>,
}

#[derive(Clone)]
struct IoSubscriber {
    agent_id: AgentId,
    sender: mpsc::Sender<IoFrame>,
    changes: watch::Sender<u64>,
}

impl IoSubscriber {
    fn new(agent_id: AgentId, sender: mpsc::Sender<IoFrame>) -> Self {
        let (changes, _) = watch::channel(0);
        Self {
            agent_id,
            sender,
            changes,
        }
    }

    fn notify_closed(&self) {
        self.changes.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }
}

impl IoRegistry {
    pub fn new() -> Self {
        Self {
            agent_streams: Arc::new(RwLock::new(HashMap::new())),
            stream_subscribers: Arc::new(RwLock::new(HashMap::new())),
            revoked_agents: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub async fn register_agent(&self, agent_id: AgentId, sender: IoSender) -> bool {
        let revoked = self.revoked_agents.read().await;
        if revoked.contains(&agent_id) {
            tracing::warn!("refusing to register revoked Agent {} io stream", agent_id);
            return false;
        }
        let closed_subscribers = {
            let mut streams = self.agent_streams.write().await;
            if let Some(previous) = streams.insert(agent_id, RegisteredSender::new(sender)) {
                previous.notify_changed();
                let mut subscribers = self.stream_subscribers.write().await;
                take_agent_subscribers(&mut subscribers, &agent_id)
            } else {
                Vec::new()
            }
        };
        notify_closed_subscribers(
            closed_subscribers,
            &agent_id,
            "Agent IO stream was replaced",
        );
        tracing::info!("Agent {} io stream registered", agent_id);
        true
    }

    pub async fn unregister_agent(
        &self,
        agent_id: &AgentId,
        sender: &mpsc::WeakSender<Result<IoFrame, tonic::Status>>,
    ) -> bool {
        let closed_subscribers = {
            let mut streams = self.agent_streams.write().await;
            let is_current = streams.get(agent_id).is_some_and(|current| {
                sender
                    .upgrade()
                    .is_some_and(|sender| current.sender.same_channel(&sender))
            });
            if !is_current {
                None
            } else {
                if let Some(current) = streams.remove(agent_id) {
                    current.notify_changed();
                }
                let mut subscribers = self.stream_subscribers.write().await;
                Some(take_agent_subscribers(&mut subscribers, agent_id))
            }
        };
        let Some(closed_subscribers) = closed_subscribers else {
            tracing::debug!("ignored stale Agent {} io stream cleanup", agent_id);
            return false;
        };
        notify_closed_subscribers(closed_subscribers, agent_id, "Agent IO stream disconnected");
        tracing::info!("Agent {} io stream unregistered", agent_id);
        true
    }

    pub async fn disconnect_agent(&self, agent_id: &AgentId) {
        self.revoked_agents.write().await.insert(*agent_id);
        let (removed, closed_subscribers) = {
            let mut streams = self.agent_streams.write().await;
            let removed = streams.remove(agent_id);
            if let Some(current) = &removed {
                current.notify_changed();
            }
            let mut subscribers = self.stream_subscribers.write().await;
            let closed_subscribers = take_agent_subscribers(&mut subscribers, agent_id);
            (removed, closed_subscribers)
        };
        notify_closed_subscribers(closed_subscribers, agent_id, "Agent was revoked");
        if removed.is_some() {
            tracing::info!("Agent {} io stream disconnected", agent_id);
        }
    }

    pub async fn subscribe_stream(
        &self,
        stream_id: String,
        agent_id: AgentId,
    ) -> mpsc::Receiver<IoFrame> {
        let (tx, rx) = mpsc::channel(128);
        let revoked = self.revoked_agents.read().await;
        if revoked.contains(&agent_id) {
            return rx;
        }
        let streams = self.agent_streams.read().await;
        if !streams.contains_key(&agent_id) {
            return rx;
        }
        let mut subscribers = self.stream_subscribers.write().await;
        let replaced = subscribers.insert(stream_id, IoSubscriber::new(agent_id, tx));
        if let Some(replaced) = replaced {
            replaced.notify_closed();
        }
        rx
    }

    pub async fn unsubscribe_stream(&self, stream_id: &str) {
        let mut subscribers = self.stream_subscribers.write().await;
        if let Some(subscriber) = subscribers.remove(stream_id) {
            subscriber.notify_closed();
        }
    }

    pub async fn send_to_agent(&self, agent_id: &AgentId, frame: IoFrame) -> Result<(), String> {
        send_to_current(
            &self.agent_streams,
            &self.revoked_agents,
            agent_id,
            Ok(frame),
            "Agent IO stream not found",
            "Agent IO stream channel is closed",
            "Timed out waiting for Agent IO stream capacity",
        )
        .await
    }

    pub async fn deliver_from_agent(&self, agent_id: &AgentId, frame: IoFrame) -> bool {
        let delivery = {
            let subscribers = self.stream_subscribers.read().await;
            subscribers
                .get(&frame.stream_id)
                .filter(|subscriber| subscriber.agent_id == *agent_id)
                .map(|subscriber| (subscriber.sender.clone(), subscriber.changes.subscribe()))
        };
        let Some((sender, mut changes)) = delivery else {
            return false;
        };
        tokio::time::timeout(IO_SUBSCRIBER_SEND_TIMEOUT, async {
            tokio::select! {
                biased;
                _ = changes.changed() => false,
                result = sender.send(frame) => result.is_ok(),
            }
        })
        .await
        .unwrap_or(false)
    }

    pub async fn is_agent_online(&self, agent_id: &AgentId) -> bool {
        let streams = self.agent_streams.read().await;
        streams.contains_key(agent_id)
    }

    #[cfg(test)]
    pub async fn subscriber_count(&self) -> usize {
        self.stream_subscribers.read().await.len()
    }
}

fn take_agent_subscribers(
    subscribers: &mut HashMap<String, IoSubscriber>,
    agent_id: &AgentId,
) -> Vec<(String, IoSubscriber)> {
    let stream_ids = subscribers
        .iter()
        .filter(|(_, subscriber)| subscriber.agent_id == *agent_id)
        .map(|(stream_id, _)| stream_id.clone())
        .collect::<Vec<_>>();
    stream_ids
        .into_iter()
        .filter_map(|stream_id| {
            subscribers
                .remove(&stream_id)
                .map(|subscriber| (stream_id, subscriber))
        })
        .collect()
}

fn notify_closed_subscribers(
    subscribers: Vec<(String, IoSubscriber)>,
    agent_id: &AgentId,
    reason: &str,
) {
    for (stream_id, subscriber) in subscribers {
        subscriber.notify_closed();
        let _ = subscriber.sender.try_send(IoFrame {
            stream_id,
            agent_id: agent_id.0.to_string(),
            payload: Some(io_frame::Payload::Error(IoError {
                code: IO_STREAM_CLOSED_CODE.to_string(),
                message: reason.to_string(),
            })),
            ..IoFrame::default()
        });
    }
}

impl Default for IoRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// M5: a registry of in-flight task dispatch requests, each waiting
/// for a `TaskResult` reply on the agent's gRPC session.
///
/// Keyed by `task_id` (assigned at dispatch time). The HTTP handler
/// inserts a oneshot sender, sends the gRPC task message, and awaits
/// the receiver. The gRPC `session` loop looks up the sender when
/// an `AgentMessage::TaskResult` arrives and forwards the result.
#[derive(Clone, Default)]
pub struct TaskResponseRegistry {
    pending: Arc<Mutex<HashMap<String, PendingTaskResponse>>>,
}

struct PendingTaskResponse {
    agent_id: AgentId,
    registration: Arc<()>,
    sender: tokio::sync::oneshot::Sender<xlstatus_proto_gen::xlstatus::v1::TaskResult>,
}

/// A cancellation-safe task result waiter.
///
/// Dropping the waiter removes its registry entry, including when an HTTP
/// request or a spawned dispatch future is aborted before its timeout fires.
pub struct TaskResponseReceiver {
    task_id: String,
    registration: Arc<()>,
    pending: Arc<Mutex<HashMap<String, PendingTaskResponse>>>,
    receiver: tokio::sync::oneshot::Receiver<xlstatus_proto_gen::xlstatus::v1::TaskResult>,
}

impl TaskResponseReceiver {
    #[cfg(test)]
    fn try_recv(
        &mut self,
    ) -> Result<
        xlstatus_proto_gen::xlstatus::v1::TaskResult,
        tokio::sync::oneshot::error::TryRecvError,
    > {
        self.receiver.try_recv()
    }
}

impl Future for TaskResponseReceiver {
    type Output = Result<
        xlstatus_proto_gen::xlstatus::v1::TaskResult,
        tokio::sync::oneshot::error::RecvError,
    >;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.receiver).poll(cx)
    }
}

impl Drop for TaskResponseReceiver {
    fn drop(&mut self) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let owns_registration = pending
            .get(&self.task_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.registration, &self.registration));
        if owns_registration {
            pending.remove(&self.task_id);
        }
    }
}

impl TaskResponseRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a waiter for `task_id`. Returns a receiver that
    /// resolves with the `TaskResult` once the agent replies.
    pub async fn register(&self, task_id: String, agent_id: AgentId) -> TaskResponseReceiver {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let registration = Arc::new(());
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.insert(
            task_id.clone(),
            PendingTaskResponse {
                agent_id,
                registration: registration.clone(),
                sender: tx,
            },
        );
        TaskResponseReceiver {
            task_id,
            registration,
            pending: self.pending.clone(),
            receiver: rx,
        }
    }

    /// Deliver a `TaskResult` to the waiter for `task_id`, if any.
    /// Returns true if a waiter consumed the value.
    pub async fn deliver(
        &self,
        agent_id: &AgentId,
        task_id: &str,
        result: xlstatus_proto_gen::xlstatus::v1::TaskResult,
    ) -> bool {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(waiter) = pending.get(task_id) else {
            return false;
        };
        if waiter.agent_id != *agent_id {
            return false;
        }
        let waiter = pending
            .remove(task_id)
            .expect("pending task waiter existed before removal");
        let _ = waiter.sender.send(result);
        true
    }

    /// Cancel a waiter (e.g. when the agent session is offline).
    pub async fn cancel(&self, task_id: &str) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.remove(task_id);
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_result_text_bounds_report_specific_field() {
        let result = TaskResult {
            stdout: "a".repeat(5),
            stderr: "b".repeat(3),
            error: String::new(),
            ..TaskResult::default()
        };

        assert!(ensure_task_result_text_within(&result, 5, 3, 0, "agent result").is_ok());
        let err = ensure_task_result_text_within(&result, 4, 3, 0, "agent result").unwrap_err();
        assert!(err.contains("stdout exceeds 4 bytes"));
    }

    #[test]
    fn task_result_text_truncation_preserves_utf8_boundaries() {
        let mut result = TaskResult {
            stdout: "a你b".to_string(),
            stderr: "stderr".to_string(),
            error: "error".to_string(),
            ..TaskResult::default()
        };

        let truncated = truncate_task_result_text(&mut result, 2, 10, 0);

        assert!(truncated.stdout);
        assert!(!truncated.stderr);
        assert!(truncated.error);
        assert_eq!(result.stdout, "a");
        assert_eq!(result.stderr, "stderr");
        assert_eq!(result.error, "");
    }

    #[test]
    fn base64_encoded_length_matches_padding_rules() {
        assert_eq!(base64_encoded_len(0), 0);
        assert_eq!(base64_encoded_len(1), 4);
        assert_eq!(base64_encoded_len(2), 4);
        assert_eq!(base64_encoded_len(3), 4);
        assert_eq!(base64_encoded_len(4), 8);
    }

    #[tokio::test]
    async fn task_result_delivery_requires_matching_agent() {
        let registry = TaskResponseRegistry::new();
        let expected_agent = AgentId(uuid::Uuid::now_v7());
        let other_agent = AgentId(uuid::Uuid::now_v7());
        let mut rx = registry
            .register("task-1".to_string(), expected_agent)
            .await;

        let spoofed = TaskResult {
            task_id: "task-1".into(),
            stdout: "spoofed".into(),
            ..TaskResult::default()
        };
        assert!(!registry.deliver(&other_agent, "task-1", spoofed).await);
        assert!(rx.try_recv().is_err());

        let expected = TaskResult {
            task_id: "task-1".into(),
            stdout: "expected".into(),
            ..TaskResult::default()
        };
        assert!(registry.deliver(&expected_agent, "task-1", expected).await);
        let delivered = rx.await.unwrap();
        assert_eq!(delivered.stdout, "expected");
    }

    #[tokio::test]
    async fn dropping_task_result_waiter_removes_only_its_registration() {
        let registry = TaskResponseRegistry::new();
        let agent = AgentId(uuid::Uuid::now_v7());
        let old_waiter = registry.register("task-1".to_string(), agent).await;
        let new_waiter = registry.register("task-1".to_string(), agent).await;
        assert_eq!(registry.pending_count(), 1);

        drop(old_waiter);
        assert_eq!(registry.pending_count(), 1);
        assert!(
            registry
                .deliver(
                    &agent,
                    "task-1",
                    TaskResult {
                        task_id: "task-1".into(),
                        ..TaskResult::default()
                    },
                )
                .await
        );
        new_waiter.await.unwrap();
        assert_eq!(registry.pending_count(), 0);

        let abandoned = registry.register("task-2".to_string(), agent).await;
        assert_eq!(registry.pending_count(), 1);
        drop(abandoned);
        assert_eq!(registry.pending_count(), 0);
    }
}
