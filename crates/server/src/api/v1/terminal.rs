use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::HeaderMap,
    response::Response,
    Json,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use xlstatus_proto_gen::xlstatus::v1::{io_frame, IoClose, IoData, IoError, IoFrame};
use xlstatus_shared::{
    terminal::{TerminalBridgeMessage, TerminalClientMessage, TerminalServerMessage},
    AgentId,
};

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::servers::{ensure_agent_visible, server_visible};
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::AgentRepository;

const TERMINAL_SESSION_TTL_SECONDS: i64 = 300;
const MAX_PENDING_TERMINAL_SESSIONS: usize = 256;

#[derive(Clone, Default)]
pub struct TerminalSessionRegistry {
    inner: Arc<RwLock<HashMap<String, TerminalSession>>>,
}

#[derive(Debug, Clone)]
pub struct TerminalSession {
    pub id: String,
    pub agent_id: AgentId,
    pub cols: u16,
    pub rows: u16,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTerminalSessionRequest {
    pub agent_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Serialize)]
pub struct CreateTerminalSessionResponse {
    pub session_id: String,
    pub agent_id: String,
    pub cols: u16,
    pub rows: u16,
    pub created_at: String,
}

pub async fn create_terminal_session(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<CreateTerminalSessionRequest>,
) -> Result<Json<ApiResponse<CreateTerminalSessionResponse>>, AppError> {
    if !has_scope(&auth, "server:exec") {
        return Err(AppError::Forbidden("missing scope: server:exec".into()));
    }
    let agent_id = parse_agent_id(&req.agent_id)?;
    if !server_visible(&auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let agent = AgentRepository::new(state.db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    ensure_agent_visible(&auth, &agent)?;
    if !state.session_registry.is_online(&agent_id).await
        || !state.io_registry.is_agent_online(&agent_id).await
    {
        return Err(AppError::BadRequest("agent is offline".into()));
    }
    let session = state
        .terminal_sessions
        .create(agent_id, sanitize_cols(req.cols), sanitize_rows(req.rows))
        .await?;
    Ok(Json(ApiResponse::success(CreateTerminalSessionResponse {
        session_id: session.id.clone(),
        agent_id: session.agent_id.0.to_string(),
        cols: session.cols,
        rows: session.rows,
        created_at: session.created_at.to_rfc3339(),
    })))
}

pub async fn ws_terminal(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    crate::security::validate_websocket_origin(&headers, &state.config.server.cors_allowed_origins)
        .map_err(AppError::Forbidden)?;
    if !has_scope(&auth, "server:exec") {
        return Err(AppError::Forbidden("missing scope: server:exec".into()));
    }
    let Some(session) = state.terminal_sessions.get(&session_id).await else {
        return Err(AppError::NotFound("terminal session not found".into()));
    };
    if !server_visible(&auth, &session.agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let agent = AgentRepository::new(state.db.clone())
        .find_by_id(session.agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    ensure_agent_visible(&auth, &agent)?;

    let io_registry = state.io_registry.clone();
    let terminal_sessions = state.terminal_sessions.clone();
    Ok(ws.on_upgrade(move |socket| async move {
        let _ = handle_terminal_socket(socket, io_registry, terminal_sessions, session).await;
    }))
}

async fn handle_terminal_socket(
    socket: WebSocket,
    io_registry: crate::grpc::IoRegistry,
    terminal_sessions: TerminalSessionRegistry,
    session: TerminalSession,
) -> Result<(), ()> {
    let (mut sender, mut receiver) = socket.split();
    let mut inbound = io_registry.subscribe_stream(session.id.clone()).await;
    let mut next_sequence = 1_u64;

    if io_registry
        .send_to_agent(
            &session.agent_id,
            build_data_frame(
                &session.id,
                &session.agent_id,
                next_sequence,
                &TerminalBridgeMessage::Open {
                    cols: session.cols,
                    rows: session.rows,
                },
            ),
        )
        .await
        .is_err()
    {
        let _ = send_terminal_message(
            &mut sender,
            &TerminalServerMessage::Error {
                message: "agent IO stream unavailable".into(),
            },
        )
        .await;
        terminal_sessions.remove(&session.id).await;
        io_registry.unsubscribe_stream(&session.id).await;
        return Ok(());
    }
    next_sequence += 1;

    let _ = send_terminal_message(
        &mut sender,
        &TerminalServerMessage::Ready {
            session_id: session.id.clone(),
        },
    )
    .await;

    let io_registry_reader = io_registry.clone();
    let session_for_reader = session.clone();
    let mut recv_task = tokio::spawn(async move {
        let mut sequence = next_sequence;
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let parsed = parse_client_message(&text);
                    let bridge = match parsed {
                        Ok(TerminalClientMessage::Input { data }) => {
                            Some(TerminalBridgeMessage::Input { data })
                        }
                        Ok(TerminalClientMessage::Resize { cols, rows }) => {
                            Some(TerminalBridgeMessage::Resize {
                                cols: sanitize_cols(cols),
                                rows: sanitize_rows(rows),
                            })
                        }
                        Ok(TerminalClientMessage::Close) => {
                            let _ = io_registry_reader
                                .send_to_agent(
                                    &session_for_reader.agent_id,
                                    build_close_frame(
                                        &session_for_reader.id,
                                        &session_for_reader.agent_id,
                                        sequence,
                                        "browser closed",
                                    ),
                                )
                                .await;
                            break;
                        }
                        Err(_) => None,
                    };

                    if let Some(bridge) = bridge {
                        let _ = io_registry_reader
                            .send_to_agent(
                                &session_for_reader.agent_id,
                                build_data_frame(
                                    &session_for_reader.id,
                                    &session_for_reader.agent_id,
                                    sequence,
                                    &bridge,
                                ),
                            )
                            .await;
                        sequence += 1;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => {
                    let _ = io_registry_reader
                        .send_to_agent(
                            &session_for_reader.agent_id,
                            build_close_frame(
                                &session_for_reader.id,
                                &session_for_reader.agent_id,
                                sequence,
                                "browser disconnected",
                            ),
                        )
                        .await;
                    break;
                }
                _ => {}
            }
        }
    });

    let mut send_task = tokio::spawn(async move {
        while let Some(frame) = inbound.recv().await {
            if let Some(message) = map_agent_frame(frame) {
                if send_terminal_message(&mut sender, &message).await.is_err() {
                    break;
                }
                if matches!(message, TerminalServerMessage::Closed { .. }) {
                    break;
                }
            }
        }
    });

    tokio::select! {
        _ = &mut recv_task => send_task.abort(),
        _ = &mut send_task => recv_task.abort(),
    }

    terminal_sessions.remove(&session.id).await;
    io_registry.unsubscribe_stream(&session.id).await;
    Ok(())
}

fn parse_client_message(text: &str) -> Result<TerminalClientMessage, serde_json::Error> {
    if let Ok(message) = serde_json::from_str::<TerminalClientMessage>(text) {
        return Ok(message);
    }
    let value = serde_json::from_str::<serde_json::Value>(text)?;
    let ty = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    match ty {
        "terminal.input" | "input" => Ok(TerminalClientMessage::Input {
            data: value
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        }),
        "terminal.resize" | "resize" => Ok(TerminalClientMessage::Resize {
            cols: value.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16,
            rows: value.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16,
        }),
        "terminal.close" | "close" => Ok(TerminalClientMessage::Close),
        _ => serde_json::from_value(value),
    }
}

fn build_data_frame(
    session_id: &str,
    agent_id: &AgentId,
    sequence: u64,
    message: &TerminalBridgeMessage,
) -> IoFrame {
    IoFrame {
        stream_id: session_id.to_string(),
        sequence,
        agent_id: agent_id.0.to_string(),
        payload: Some(io_frame::Payload::Data(IoData {
            data: serde_json::to_vec(message).unwrap_or_default(),
        })),
    }
}

fn build_close_frame(session_id: &str, agent_id: &AgentId, sequence: u64, reason: &str) -> IoFrame {
    IoFrame {
        stream_id: session_id.to_string(),
        sequence,
        agent_id: agent_id.0.to_string(),
        payload: Some(io_frame::Payload::Close(IoClose {
            reason: reason.to_string(),
        })),
    }
}

fn map_agent_frame(frame: IoFrame) -> Option<TerminalServerMessage> {
    match frame.payload {
        Some(io_frame::Payload::Data(data)) => {
            if let Ok(bridge) = serde_json::from_slice::<TerminalBridgeMessage>(&data.data) {
                match bridge {
                    TerminalBridgeMessage::Output { data } => {
                        Some(TerminalServerMessage::Output { data })
                    }
                    TerminalBridgeMessage::Close { reason } => {
                        Some(TerminalServerMessage::Closed {
                            reason: reason.unwrap_or_else(|| "terminal closed".into()),
                        })
                    }
                    TerminalBridgeMessage::Error { message } => {
                        Some(TerminalServerMessage::Error { message })
                    }
                    _ => None,
                }
            } else {
                Some(TerminalServerMessage::Output {
                    data: String::from_utf8_lossy(&data.data).to_string(),
                })
            }
        }
        Some(io_frame::Payload::Close(close)) => Some(TerminalServerMessage::Closed {
            reason: close.reason,
        }),
        Some(io_frame::Payload::Error(IoError { message, .. })) => {
            Some(TerminalServerMessage::Error { message })
        }
        None => None,
    }
}

async fn send_terminal_message(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    message: &TerminalServerMessage,
) -> Result<(), ()> {
    let text = serde_json::to_string(message).map_err(|_| ())?;
    sender.send(Message::Text(text)).await.map_err(|_| ())
}

fn parse_agent_id(id: &str) -> Result<AgentId, AppError> {
    let parsed = uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::BadRequest(format!("invalid agent id: {}", id)))?;
    Ok(AgentId(parsed))
}

fn sanitize_cols(cols: u16) -> u16 {
    cols.clamp(20, 240)
}

fn sanitize_rows(rows: u16) -> u16 {
    rows.clamp(8, 80)
}

impl TerminalSessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn create(
        &self,
        agent_id: AgentId,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalSession, AppError> {
        let session = TerminalSession {
            id: uuid::Uuid::now_v7().to_string(),
            agent_id,
            cols,
            rows,
            created_at: chrono::Utc::now(),
        };
        let mut sessions = self.inner.write().await;
        prune_expired_terminal_sessions(&mut sessions, chrono::Utc::now());
        if sessions.len() >= MAX_PENDING_TERMINAL_SESSIONS {
            return Err(AppError::Forbidden(
                "too many pending terminal sessions".into(),
            ));
        }
        sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    pub async fn get(&self, session_id: &str) -> Option<TerminalSession> {
        let mut sessions = self.inner.write().await;
        prune_expired_terminal_sessions(&mut sessions, chrono::Utc::now());
        sessions.get(session_id).cloned()
    }

    pub async fn remove(&self, session_id: &str) {
        self.inner.write().await.remove(session_id);
    }
}

fn prune_expired_terminal_sessions(
    sessions: &mut HashMap<String, TerminalSession>,
    now: chrono::DateTime<chrono::Utc>,
) {
    sessions.retain(|_, session| {
        now.signed_duration_since(session.created_at).num_seconds() <= TERMINAL_SESSION_TTL_SECONDS
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn terminal_registry_prunes_expired_sessions() {
        let registry = TerminalSessionRegistry::new();
        let expired = TerminalSession {
            id: "expired".into(),
            agent_id: AgentId(uuid::Uuid::now_v7()),
            cols: 80,
            rows: 24,
            created_at: chrono::Utc::now()
                - chrono::Duration::seconds(TERMINAL_SESSION_TTL_SECONDS + 1),
        };
        registry
            .inner
            .write()
            .await
            .insert(expired.id.clone(), expired);

        assert!(registry.get("expired").await.is_none());
    }

    #[tokio::test]
    async fn terminal_registry_limits_pending_sessions() {
        let registry = TerminalSessionRegistry::new();
        for index in 0..MAX_PENDING_TERMINAL_SESSIONS {
            let session = TerminalSession {
                id: format!("session-{index}"),
                agent_id: AgentId(uuid::Uuid::now_v7()),
                cols: 80,
                rows: 24,
                created_at: chrono::Utc::now(),
            };
            registry
                .inner
                .write()
                .await
                .insert(session.id.clone(), session);
        }

        let err = registry
            .create(AgentId(uuid::Uuid::now_v7()), 80, 24)
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }
}
