use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use xlstatus_proto_gen::xlstatus::v1::{AgentMessage, ServerMessage};
use xlstatus_shared::AgentId;

pub type SessionSender = mpsc::Sender<Result<ServerMessage, tonic::Status>>;

#[derive(Clone)]
pub struct SessionRegistry {
    sessions: Arc<RwLock<HashMap<AgentId, SessionSender>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register(&self, agent_id: AgentId, sender: SessionSender) {
        let mut sessions = self.sessions.write().await;
        sessions.insert(agent_id, sender);
        tracing::info!("Agent {} session registered", agent_id);
    }

    pub async fn unregister(&self, agent_id: &AgentId) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(agent_id);
        tracing::info!("Agent {} session unregistered", agent_id);
    }

    pub async fn send(&self, agent_id: &AgentId, message: ServerMessage) -> Result<(), String> {
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

    pub async fn count(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.len()
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
