use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use xlstatus_proto_gen::xlstatus::v1::{IoFrame, ServerMessage, ServerTask, TaskResult};
use xlstatus_shared::AgentId;

pub type SessionSender = mpsc::Sender<Result<ServerMessage, tonic::Status>>;
pub type IoSender = mpsc::Sender<Result<IoFrame, tonic::Status>>;

#[derive(Clone)]
pub struct SessionRegistry {
    sessions: Arc<RwLock<HashMap<AgentId, SessionSender>>>,
    revoked: Arc<RwLock<HashSet<AgentId>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            revoked: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub async fn register(&self, agent_id: AgentId, sender: SessionSender) {
        if self.revoked.read().await.contains(&agent_id) {
            tracing::warn!("refusing to register revoked Agent {} session", agent_id);
            return;
        }
        let mut sessions = self.sessions.write().await;
        sessions.insert(agent_id, sender);
        tracing::info!("Agent {} session registered", agent_id);
    }

    pub async fn unregister(&self, agent_id: &AgentId) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(agent_id);
        tracing::info!("Agent {} session unregistered", agent_id);
    }

    pub async fn disconnect(&self, agent_id: &AgentId) {
        self.revoked.write().await.insert(*agent_id);
        self.unregister(agent_id).await;
    }

    pub async fn send(&self, agent_id: &AgentId, message: ServerMessage) -> Result<(), String> {
        if self.revoked.read().await.contains(agent_id) {
            return Err("Agent has been revoked".to_string());
        }
        let sessions = self.sessions.read().await;
        if let Some(sender) = sessions.get(agent_id) {
            sender
                .send(Ok(message))
                .await
                .map_err(|e| format!("Failed to send message: {}", e))?;
            Ok(())
        } else {
            Err("Agent session not found".to_string())
        }
    }

    pub async fn broadcast(&self, message: ServerMessage) {
        let sessions = self.sessions.read().await;
        for sender in sessions.values() {
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
    async fn io_disconnect_blocks_late_frame_sends_and_reregister() {
        let registry = IoRegistry::new();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let (tx, mut rx) = mpsc::channel(1);
        registry.register_agent(agent_id, tx).await;

        registry.disconnect_agent(&agent_id).await;
        assert!(!registry.is_agent_online(&agent_id).await);
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

#[derive(Clone, Default)]
pub struct IoRegistry {
    agent_streams: Arc<RwLock<HashMap<AgentId, IoSender>>>,
    stream_subscribers: Arc<RwLock<HashMap<String, mpsc::Sender<IoFrame>>>>,
    revoked_agents: Arc<RwLock<HashSet<AgentId>>>,
}

impl IoRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register_agent(&self, agent_id: AgentId, sender: IoSender) {
        if self.revoked_agents.read().await.contains(&agent_id) {
            tracing::warn!("refusing to register revoked Agent {} io stream", agent_id);
            return;
        }
        let mut streams = self.agent_streams.write().await;
        streams.insert(agent_id, sender);
        tracing::info!("Agent {} io stream registered", agent_id);
    }

    pub async fn unregister_agent(&self, agent_id: &AgentId) {
        let mut streams = self.agent_streams.write().await;
        streams.remove(agent_id);
        tracing::info!("Agent {} io stream unregistered", agent_id);
    }

    pub async fn disconnect_agent(&self, agent_id: &AgentId) {
        self.revoked_agents.write().await.insert(*agent_id);
        self.unregister_agent(agent_id).await;
    }

    pub async fn subscribe_stream(&self, stream_id: String) -> mpsc::Receiver<IoFrame> {
        let (tx, rx) = mpsc::channel(128);
        let mut subscribers = self.stream_subscribers.write().await;
        subscribers.insert(stream_id, tx);
        rx
    }

    pub async fn unsubscribe_stream(&self, stream_id: &str) {
        let mut subscribers = self.stream_subscribers.write().await;
        subscribers.remove(stream_id);
    }

    pub async fn send_to_agent(&self, agent_id: &AgentId, frame: IoFrame) -> Result<(), String> {
        if self.revoked_agents.read().await.contains(agent_id) {
            return Err("Agent has been revoked".to_string());
        }
        let streams = self.agent_streams.read().await;
        if let Some(sender) = streams.get(agent_id) {
            sender
                .send(Ok(frame))
                .await
                .map_err(|e| format!("Failed to send IO frame: {}", e))
        } else {
            Err("Agent IO stream not found".to_string())
        }
    }

    pub async fn deliver_from_agent(&self, frame: IoFrame) -> bool {
        let sender = {
            let subscribers = self.stream_subscribers.read().await;
            subscribers.get(&frame.stream_id).cloned()
        };
        if let Some(sender) = sender {
            sender.send(frame).await.is_ok()
        } else {
            false
        }
    }

    pub async fn is_agent_online(&self, agent_id: &AgentId) -> bool {
        let streams = self.agent_streams.read().await;
        streams.contains_key(agent_id)
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
    pending: Arc<
        RwLock<
            HashMap<
                String,
                tokio::sync::oneshot::Sender<xlstatus_proto_gen::xlstatus::v1::TaskResult>,
            >,
        >,
    >,
}

impl TaskResponseRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a waiter for `task_id`. Returns a receiver that
    /// resolves with the `TaskResult` once the agent replies.
    pub async fn register(
        &self,
        task_id: String,
    ) -> tokio::sync::oneshot::Receiver<xlstatus_proto_gen::xlstatus::v1::TaskResult> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut pending = self.pending.write().await;
        pending.insert(task_id, tx);
        rx
    }

    /// Deliver a `TaskResult` to the waiter for `task_id`, if any.
    /// Returns true if a waiter consumed the value.
    pub async fn deliver(
        &self,
        task_id: &str,
        result: xlstatus_proto_gen::xlstatus::v1::TaskResult,
    ) -> bool {
        let mut pending = self.pending.write().await;
        if let Some(tx) = pending.remove(task_id) {
            let _ = tx.send(result);
            true
        } else {
            false
        }
    }

    /// Cancel a waiter (e.g. when the agent session is offline).
    pub async fn cancel(&self, task_id: &str) {
        let mut pending = self.pending.write().await;
        pending.remove(task_id);
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
}
