mod session;

pub use session::SessionRegistry;

use crate::db::{AgentRepository, DatabaseBackend};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use xlstatus_proto_gen::xlstatus::v1::agent_service_server::AgentService;
use xlstatus_proto_gen::xlstatus::v1::{agent_message, server_message, AgentMessage, ServerMessage, IoFrame};
use xlstatus_shared::AgentId;

pub struct AgentServiceImpl {
    db: DatabaseBackend,
    session_registry: SessionRegistry,
}

impl AgentServiceImpl {
    pub fn new(db: DatabaseBackend, session_registry: SessionRegistry) -> Self {
        Self {
            db,
            session_registry,
        }
    }
}

#[tonic::async_trait]
impl AgentService for AgentServiceImpl {
    type SessionStream = ReceiverStream<Result<ServerMessage, Status>>;
    type IoStreamStream = ReceiverStream<Result<IoFrame, Status>>;

    async fn session(
        &self,
        request: Request<tonic::Streaming<AgentMessage>>,
    ) -> Result<Response<Self::SessionStream>, Status> {
        // TODO: Extract and verify JWT from metadata
        let agent_id = AgentId::new(); // Placeholder

        let mut in_stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);

        // Register session
        self.session_registry.register(agent_id, tx.clone()).await;

        let db = self.db.clone();
        let session_registry = self.session_registry.clone();

        // Spawn task to handle incoming messages
        tokio::spawn(async move {
            while let Ok(Some(msg)) = in_stream.message().await {
                match msg.payload {
                    Some(agent_message::Payload::Heartbeat(_heartbeat)) => {
                        tracing::debug!("Heartbeat from agent {}", agent_id);

                        // Update last_seen_at
                        let agent_repo = AgentRepository::new(db.clone());
                        if let Err(e) = agent_repo.update_last_seen(agent_id).await {
                            tracing::error!("Failed to update last_seen: {}", e);
                        }
                    }
                    Some(agent_message::Payload::HostState(state)) => {
                        tracing::debug!(
                            "Host state from agent {}: CPU={:.1}%, Mem={}/{}",
                            agent_id,
                            state.cpu_percent,
                            state.memory_used,
                            state.memory_total
                        );

                        // TODO: Store metrics in TSDB
                    }
                    Some(agent_message::Payload::TaskResult(result)) => {
                        tracing::debug!("Task result from agent {}: {}", agent_id, result.task_id);
                        // TODO: Handle task result
                    }
                    Some(agent_message::Payload::HostInfoUpdate(_info)) => {
                        tracing::debug!("Host info update from agent {}", agent_id);
                        // TODO: Store host info
                    }
                    Some(agent_message::Payload::GeoIpReport(_report)) => {
                        tracing::debug!("GeoIP report from agent {}", agent_id);
                        // TODO: Store GeoIP info
                    }
                    None => {}
                }
            }

            // Unregister when stream ends
            session_registry.unregister(&agent_id).await;
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn io_stream(
        &self,
        request: Request<tonic::Streaming<IoFrame>>,
    ) -> Result<Response<Self::IoStreamStream>, Status> {
        let mut in_stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);

        // Spawn task to handle IO frames
        tokio::spawn(async move {
            while let Ok(Some(frame)) = in_stream.message().await {
                tracing::debug!("IO frame stream_id={}, seq={}", frame.stream_id, frame.sequence);
                // TODO: Route IO frames to appropriate handlers (terminal, file transfer)
                // For now, just echo back
                let _ = tx.send(Ok(frame)).await;
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
