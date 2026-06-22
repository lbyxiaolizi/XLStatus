use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, Path, State,
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
use crate::api::v1::auth::{require_sensitive_totp, AppError, AppState};
use crate::api::v1::servers::{ensure_agent_visible, server_visible};
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::auth::rbac::has_scope;
use crate::db::AgentRepository;

const TERMINAL_SESSION_TTL_SECONDS: i64 = 300;
const MAX_PENDING_TERMINAL_SESSIONS: usize = 256;
const TERMINAL_CREATE_MAX_BODY_BYTES: usize = 4 * 1024;
const TERMINAL_MAX_CLIENT_TEXT_BYTES: usize = 16 * 1024;
const TERMINAL_MAX_INPUT_BYTES: usize = 8 * 1024;
const TERMINAL_MAX_AGENT_FRAME_BYTES: usize = 64 * 1024;
const TERMINAL_MAX_CLOSE_REASON_BYTES: usize = 1024;
const TERMINAL_MAX_ERROR_BYTES: usize = 4096;
const TERMINAL_UUID_TEXT_LEN: usize = 36;

#[derive(Clone, Default)]
pub struct TerminalSessionRegistry {
    inner: Arc<RwLock<HashMap<String, TerminalSession>>>,
}

#[derive(Debug, Clone)]
pub struct TerminalSession {
    pub id: String,
    pub agent_id: AgentId,
    pub created_by_user_id: xlstatus_shared::UserId,
    pub created_auth_kind: AuthKind,
    pub created_session_id: Option<String>,
    pub created_pat_id: Option<String>,
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

pub fn terminal_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(TERMINAL_CREATE_MAX_BODY_BYTES)
}

pub async fn create_terminal_session(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    Json(req): Json<CreateTerminalSessionRequest>,
) -> Result<Json<ApiResponse<CreateTerminalSessionResponse>>, AppError> {
    require_terminal_session_scope(&state.db, &auth, &headers).await?;
    let agent_id = parse_agent_id(&req.agent_id)?;
    if !server_visible(&auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let agent = AgentRepository::new(state.db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    ensure_terminal_agent_active(&auth, &agent)?;
    if !state.session_registry.is_online(&agent_id).await
        || !state.io_registry.is_agent_online(&agent_id).await
    {
        return Err(AppError::BadRequest("agent is offline".into()));
    }
    let session = state
        .terminal_sessions
        .create(
            &auth,
            agent_id,
            sanitize_cols(req.cols),
            sanitize_rows(req.rows),
        )
        .await?;
    Ok(Json(ApiResponse::success(CreateTerminalSessionResponse {
        session_id: session.id.clone(),
        agent_id: session.agent_id.0.to_string(),
        cols: session.cols,
        rows: session.rows,
        created_at: session.created_at.to_rfc3339(),
    })))
}

async fn require_terminal_session_scope(
    db: &crate::db::DatabaseBackend,
    auth: &AuthSession,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    if !has_scope(auth, "server:exec") {
        return Err(AppError::Forbidden("missing scope: server:exec".into()));
    }
    if matches!(auth.auth_kind, AuthKind::PersonalAccessToken) {
        return Err(AppError::Forbidden(
            "terminal sessions require a cookie session".into(),
        ));
    }
    require_sensitive_totp(db, auth.user_id, headers).await
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
    let session_id = require_terminal_uuid_text(&session_id, "terminal_session_id")?;
    let session = match state
        .terminal_sessions
        .take_for_auth(&session_id, &auth)
        .await?
    {
        Some(session) => session,
        None => return Err(AppError::NotFound("terminal session not found".into())),
    };
    if !server_visible(&auth, &session.agent_id) {
        state.terminal_sessions.restore(session).await;
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let agent = match AgentRepository::new(state.db.clone())
        .find_by_id(session.agent_id)
        .await?
    {
        Some(agent) => agent,
        None => {
            state.terminal_sessions.restore(session).await;
            return Err(AppError::NotFound("agent not found".into()));
        }
    };
    if let Err(err) = ensure_terminal_agent_active(&auth, &agent) {
        state.terminal_sessions.restore(session).await;
        return Err(err);
    }

    let io_registry = state.io_registry.clone();
    let terminal_sessions = state.terminal_sessions.clone();
    Ok(ws
        .max_message_size(TERMINAL_MAX_CLIENT_TEXT_BYTES)
        .max_frame_size(TERMINAL_MAX_CLIENT_TEXT_BYTES)
        .on_upgrade(move |socket| async move {
            let _ = handle_terminal_socket(socket, io_registry, terminal_sessions, session).await;
        }))
}

fn terminal_session_created_by_auth(session: &TerminalSession, auth: &AuthSession) -> bool {
    session.created_by_user_id == auth.user_id
        && matches!(
            (&session.created_auth_kind, &auth.auth_kind),
            (AuthKind::Session, AuthKind::Session)
                | (AuthKind::PersonalAccessToken, AuthKind::PersonalAccessToken)
        )
        && match auth.auth_kind {
            AuthKind::Session => session.created_session_id.as_deref() == Some(&auth.session_id),
            AuthKind::PersonalAccessToken => {
                let pat_id = auth.pat_id.as_deref().unwrap_or(&auth.session_id);
                session.created_pat_id.as_deref() == Some(pat_id)
            }
        }
}

fn ensure_terminal_agent_active(
    auth: &AuthSession,
    agent: &crate::db::Agent,
) -> Result<(), AppError> {
    ensure_agent_visible(auth, agent)?;
    if agent.revoked_at.is_some() {
        return Err(AppError::Forbidden("agent has been revoked".into()));
    }
    Ok(())
}

async fn handle_terminal_socket(
    socket: WebSocket,
    io_registry: crate::grpc::IoRegistry,
    terminal_sessions: TerminalSessionRegistry,
    session: TerminalSession,
) -> Result<(), ()> {
    let (mut sender, mut receiver) = socket.split();
    let mut inbound = io_registry
        .subscribe_stream(session.id.clone(), session.agent_id)
        .await;
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
    if text.len() > TERMINAL_MAX_CLIENT_TEXT_BYTES {
        return Err(serde_json::from_str::<TerminalClientMessage>("").unwrap_err());
    }
    if let Ok(message) = serde_json::from_str::<TerminalClientMessage>(text) {
        return normalize_terminal_client_message(message);
    }
    let value = serde_json::from_str::<serde_json::Value>(text)?;
    let ty = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    match ty {
        "terminal.input" | "input" => Ok(TerminalClientMessage::Input {
            data: limit_terminal_text(
                value
                    .get("data")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
                TERMINAL_MAX_INPUT_BYTES,
            ),
        }),
        "terminal.resize" | "resize" => Ok(TerminalClientMessage::Resize {
            cols: value.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16,
            rows: value.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16,
        }),
        "terminal.close" | "close" => Ok(TerminalClientMessage::Close),
        _ => serde_json::from_value(value).and_then(normalize_terminal_client_message),
    }
}

fn normalize_terminal_client_message(
    message: TerminalClientMessage,
) -> Result<TerminalClientMessage, serde_json::Error> {
    match message {
        TerminalClientMessage::Input { data } => Ok(TerminalClientMessage::Input {
            data: limit_terminal_text(&data, TERMINAL_MAX_INPUT_BYTES),
        }),
        TerminalClientMessage::Resize { cols, rows } => {
            Ok(TerminalClientMessage::Resize { cols, rows })
        }
        TerminalClientMessage::Close => Ok(TerminalClientMessage::Close),
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
            if data.data.len() > TERMINAL_MAX_AGENT_FRAME_BYTES {
                return Some(TerminalServerMessage::Error {
                    message: "terminal output frame is too large".into(),
                });
            }
            if let Ok(bridge) = serde_json::from_slice::<TerminalBridgeMessage>(&data.data) {
                match bridge {
                    TerminalBridgeMessage::Output { data } => Some(TerminalServerMessage::Output {
                        data: limit_terminal_text(&data, TERMINAL_MAX_AGENT_FRAME_BYTES),
                    }),
                    TerminalBridgeMessage::Close { reason } => {
                        Some(TerminalServerMessage::Closed {
                            reason: reason
                                .as_deref()
                                .map(|value| {
                                    limit_terminal_text(value, TERMINAL_MAX_CLOSE_REASON_BYTES)
                                })
                                .unwrap_or_else(|| "terminal closed".into()),
                        })
                    }
                    TerminalBridgeMessage::Error { message } => {
                        Some(TerminalServerMessage::Error {
                            message: limit_terminal_text(&message, TERMINAL_MAX_ERROR_BYTES),
                        })
                    }
                    _ => None,
                }
            } else {
                Some(TerminalServerMessage::Output {
                    data: limit_terminal_text(
                        &String::from_utf8_lossy(&data.data),
                        TERMINAL_MAX_AGENT_FRAME_BYTES,
                    ),
                })
            }
        }
        Some(io_frame::Payload::Close(close)) => Some(TerminalServerMessage::Closed {
            reason: limit_terminal_text(&close.reason, TERMINAL_MAX_CLOSE_REASON_BYTES),
        }),
        Some(io_frame::Payload::Error(IoError { message, .. })) => {
            Some(TerminalServerMessage::Error {
                message: limit_terminal_text(&message, TERMINAL_MAX_ERROR_BYTES),
            })
        }
        None => None,
    }
}

fn limit_terminal_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

async fn send_terminal_message(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    message: &TerminalServerMessage,
) -> Result<(), ()> {
    let text = serde_json::to_string(message).map_err(|_| ())?;
    sender.send(Message::Text(text)).await.map_err(|_| ())
}

fn parse_agent_id(id: &str) -> Result<AgentId, AppError> {
    let id = require_terminal_uuid_text(id, "agent_id")?;
    let parsed = uuid::Uuid::parse_str(&id).expect("canonical UUID must parse after validation");
    Ok(AgentId(parsed))
}

fn require_terminal_uuid_text(value: &str, field: &str) -> Result<String, AppError> {
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if value.len() != TERMINAL_UUID_TEXT_LEN {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|_| AppError::BadRequest(format!("{field} must be a canonical UUID")))?;
    if parsed.to_string() != value {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    Ok(value.to_string())
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
        auth: &AuthSession,
        agent_id: AgentId,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalSession, AppError> {
        let session = TerminalSession {
            id: uuid::Uuid::now_v7().to_string(),
            agent_id,
            created_by_user_id: auth.user_id,
            created_auth_kind: auth.auth_kind.clone(),
            created_session_id: matches!(auth.auth_kind, AuthKind::Session)
                .then_some(auth.session_id.clone()),
            created_pat_id: matches!(auth.auth_kind, AuthKind::PersonalAccessToken).then(|| {
                auth.pat_id
                    .clone()
                    .unwrap_or_else(|| auth.session_id.clone())
            }),
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

    #[cfg(test)]
    pub async fn get(&self, session_id: &str) -> Option<TerminalSession> {
        let mut sessions = self.inner.write().await;
        prune_expired_terminal_sessions(&mut sessions, chrono::Utc::now());
        sessions.get(session_id).cloned()
    }

    pub async fn take_for_auth(
        &self,
        session_id: &str,
        auth: &AuthSession,
    ) -> Result<Option<TerminalSession>, AppError> {
        let mut sessions = self.inner.write().await;
        prune_expired_terminal_sessions(&mut sessions, chrono::Utc::now());
        let Some(session) = sessions.get(session_id) else {
            return Ok(None);
        };
        if !terminal_session_created_by_auth(session, auth) {
            return Err(AppError::Forbidden(
                "terminal session was created by another credential".into(),
            ));
        }
        Ok(sessions.remove(session_id))
    }

    pub async fn restore(&self, session: TerminalSession) {
        let mut sessions = self.inner.write().await;
        prune_expired_terminal_sessions(&mut sessions, chrono::Utc::now());
        if chrono::Utc::now()
            .signed_duration_since(session.created_at)
            .num_seconds()
            <= TERMINAL_SESSION_TTL_SECONDS
        {
            sessions.insert(session.id.clone(), session);
        }
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
    use crate::db::{CreateAgentInput, CreateUserInput, DatabaseBackend, UserRepository};
    use xlstatus_shared::{UserId, UserRole};

    fn test_auth_session(session_id: &str) -> AuthSession {
        AuthSession {
            session_id: session_id.into(),
            user_id: UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
            username: "admin".into(),
            role: UserRole::Admin,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: Vec::new(),
            server_ids: None,
            pat_id: None,
        }
    }

    fn test_pat_auth(pat_id: &str) -> AuthSession {
        AuthSession {
            session_id: pat_id.into(),
            user_id: UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
            username: "pat".into(),
            role: UserRole::Admin,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::PersonalAccessToken,
            scopes: vec!["server:exec".into()],
            server_ids: None,
            pat_id: Some(pat_id.into()),
        }
    }

    fn terminal_session(id: &str, auth: &AuthSession) -> TerminalSession {
        TerminalSession {
            id: id.into(),
            agent_id: AgentId(uuid::Uuid::now_v7()),
            created_by_user_id: auth.user_id,
            created_auth_kind: auth.auth_kind.clone(),
            created_session_id: matches!(auth.auth_kind, AuthKind::Session)
                .then_some(auth.session_id.clone()),
            created_pat_id: matches!(auth.auth_kind, AuthKind::PersonalAccessToken).then(|| {
                auth.pat_id
                    .clone()
                    .unwrap_or_else(|| auth.session_id.clone())
            }),
            cols: 80,
            rows: 24,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn terminal_registry_prunes_expired_sessions() {
        let registry = TerminalSessionRegistry::new();
        let auth = test_auth_session("sess-1");
        let mut expired = terminal_session("expired", &auth);
        expired.created_at =
            chrono::Utc::now() - chrono::Duration::seconds(TERMINAL_SESSION_TTL_SECONDS + 1);
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
        let auth = test_auth_session("sess-1");
        for index in 0..MAX_PENDING_TERMINAL_SESSIONS {
            let session = terminal_session(&format!("session-{index}"), &auth);
            registry
                .inner
                .write()
                .await
                .insert(session.id.clone(), session);
        }

        let err = registry
            .create(&auth, AgentId(uuid::Uuid::now_v7()), 80, 24)
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn terminal_session_creation_rejects_pat_session() {
        let db = test_db().await;
        let auth = test_pat_auth("pat-id");

        let err = require_terminal_session_scope(&db, &auth, &HeaderMap::new())
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(message) if message.contains("cookie session")));
    }

    #[tokio::test]
    async fn terminal_session_creation_requires_sensitive_totp_when_enabled() {
        let db = test_db().await;
        let auth = test_auth_session("sess-1");
        seed_user_with_id(&db, auth.user_id).await;
        seed_totp_enabled_user(&db, auth.user_id).await;

        let err = require_terminal_session_scope(&db, &auth, &HeaderMap::new())
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(message) if message.contains("TOTP")));
    }

    #[tokio::test]
    async fn terminal_session_creation_allows_cookie_session_without_totp() {
        let db = test_db().await;
        let auth = test_auth_session("sess-1");
        seed_user_with_id(&db, auth.user_id).await;

        assert!(
            require_terminal_session_scope(&db, &auth, &HeaderMap::new())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn terminal_session_take_is_bound_to_creator_and_one_time() {
        let registry = TerminalSessionRegistry::new();
        let creator = test_auth_session("creator-session");
        let other_session = test_auth_session("other-session");
        let pat = test_pat_auth("pat-id");
        let created = registry
            .create(&creator, AgentId(uuid::Uuid::now_v7()), 80, 24)
            .await
            .unwrap();

        let err = registry
            .take_for_auth(&created.id, &other_session)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
        assert!(registry.get(&created.id).await.is_some());

        let err = registry.take_for_auth(&created.id, &pat).await.unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
        assert!(registry.get(&created.id).await.is_some());

        let taken = registry
            .take_for_auth(&created.id, &creator)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(taken.id, created.id);
        assert!(registry
            .take_for_auth(&created.id, &creator)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn terminal_session_pat_binding_uses_pat_id() {
        // Registry-level compatibility for legacy in-memory entries; the HTTP
        // terminal session creation entrypoint rejects PAT before this layer.
        let registry = TerminalSessionRegistry::new();
        let creator = test_pat_auth("pat-1");
        let same_session_different_pat = test_pat_auth("pat-2");
        let created = registry
            .create(&creator, AgentId(uuid::Uuid::now_v7()), 80, 24)
            .await
            .unwrap();

        assert!(registry
            .take_for_auth(&created.id, &same_session_different_pat)
            .await
            .is_err());
        assert!(registry.get(&created.id).await.is_some());
        assert!(registry
            .take_for_auth(&created.id, &creator)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn terminal_agent_active_check_rejects_revoked_agent() {
        let db = test_db().await;
        let owner = seed_user(&db).await;
        let agent_repo = AgentRepository::new(db.clone());
        let agent = agent_repo
            .create(CreateAgentInput {
                name: "terminal-agent".into(),
                public_key: "pk".into(),
                owner_user_id: owner,
            })
            .await
            .unwrap();
        let auth = test_auth_session("sess-1");

        let active_agent = agent_repo.find_by_id(agent.id).await.unwrap().unwrap();
        assert!(ensure_terminal_agent_active(&auth, &active_agent).is_ok());

        assert!(agent_repo.revoke(agent.id).await.unwrap());
        let revoked_agent = agent_repo.find_by_id(agent.id).await.unwrap().unwrap();
        let err = ensure_terminal_agent_active(&auth, &revoked_agent).unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn terminal_resource_limits_are_explicit() {
        assert_eq!(TERMINAL_CREATE_MAX_BODY_BYTES, 4 * 1024);
        assert_eq!(TERMINAL_MAX_CLIENT_TEXT_BYTES, 16 * 1024);
        assert_eq!(TERMINAL_MAX_INPUT_BYTES, 8 * 1024);
        assert_eq!(TERMINAL_MAX_AGENT_FRAME_BYTES, 64 * 1024);
        assert_eq!(TERMINAL_MAX_CLOSE_REASON_BYTES, 1024);
        assert_eq!(TERMINAL_MAX_ERROR_BYTES, 4096);
        assert_eq!(TERMINAL_UUID_TEXT_LEN, 36);
    }

    #[test]
    fn terminal_resource_ids_require_canonical_uuid_text() {
        let id = uuid::Uuid::parse_str("018f7e34-1234-4abc-8def-abcdefabcdef").unwrap();
        let canonical = id.to_string();

        assert_eq!(parse_agent_id(&canonical).unwrap().0, id);
        assert_eq!(
            require_terminal_uuid_text(&canonical, "terminal_session_id").unwrap(),
            canonical
        );

        assert!(require_terminal_uuid_text("session-a", "terminal_session_id").is_err());
        assert!(require_terminal_uuid_text(&format!(" {id} "), "terminal_session_id").is_err());
        assert!(
            require_terminal_uuid_text(&id.simple().to_string(), "terminal_session_id").is_err()
        );
        assert!(
            require_terminal_uuid_text(&canonical.to_uppercase(), "terminal_session_id").is_err()
        );
        assert!(require_terminal_uuid_text(
            &"a".repeat(TERMINAL_UUID_TEXT_LEN + 1),
            "terminal_session_id",
        )
        .is_err());
    }

    #[test]
    fn terminal_client_input_is_bounded() {
        let message = serde_json::json!({
            "type": "input",
            "data": "a".repeat(TERMINAL_MAX_INPUT_BYTES + 32)
        })
        .to_string();

        let parsed = parse_client_message(&message).unwrap();

        match parsed {
            TerminalClientMessage::Input { data } => {
                assert_eq!(data.len(), TERMINAL_MAX_INPUT_BYTES);
            }
            _ => panic!("expected input message"),
        }
    }

    #[test]
    fn terminal_rejects_oversized_client_text() {
        let oversized = "a".repeat(TERMINAL_MAX_CLIENT_TEXT_BYTES + 1);

        assert!(parse_client_message(&oversized).is_err());
    }

    #[test]
    fn terminal_agent_output_and_errors_are_bounded() {
        let session_id = uuid::Uuid::now_v7().to_string();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        let frame = build_data_frame(
            &session_id,
            &agent_id,
            1,
            &TerminalBridgeMessage::Output {
                data: "a".repeat(TERMINAL_MAX_AGENT_FRAME_BYTES + 1),
            },
        );

        let mapped = map_agent_frame(frame).unwrap();

        assert!(matches!(mapped, TerminalServerMessage::Error { .. }));

        let raw_frame = IoFrame {
            stream_id: session_id.clone(),
            sequence: 2,
            agent_id: agent_id.0.to_string(),
            payload: Some(io_frame::Payload::Data(IoData {
                data: vec![b'a'; TERMINAL_MAX_AGENT_FRAME_BYTES],
            })),
        };
        let mapped = map_agent_frame(raw_frame).unwrap();
        match mapped {
            TerminalServerMessage::Output { data } => {
                assert_eq!(data.len(), TERMINAL_MAX_AGENT_FRAME_BYTES);
            }
            _ => panic!("expected output message"),
        }

        let error_frame = IoFrame {
            stream_id: session_id,
            sequence: 3,
            agent_id: agent_id.0.to_string(),
            payload: Some(io_frame::Payload::Error(IoError {
                code: "terminal".into(),
                message: "e".repeat(TERMINAL_MAX_ERROR_BYTES + 1),
            })),
        };
        let mapped = map_agent_frame(error_frame).unwrap();
        match mapped {
            TerminalServerMessage::Error { message } => {
                assert_eq!(message.len(), TERMINAL_MAX_ERROR_BYTES);
            }
            _ => panic!("expected error message"),
        }
    }

    #[test]
    fn terminal_close_reason_is_bounded() {
        let frame = IoFrame {
            stream_id: uuid::Uuid::now_v7().to_string(),
            sequence: 1,
            agent_id: uuid::Uuid::now_v7().to_string(),
            payload: Some(io_frame::Payload::Close(IoClose {
                reason: "r".repeat(TERMINAL_MAX_CLOSE_REASON_BYTES + 1),
            })),
        };

        let mapped = map_agent_frame(frame).unwrap();

        match mapped {
            TerminalServerMessage::Closed { reason } => {
                assert_eq!(reason.len(), TERMINAL_MAX_CLOSE_REASON_BYTES);
            }
            _ => panic!("expected closed message"),
        }
    }

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_user(db: &DatabaseBackend) -> UserId {
        UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "password123".into(),
                role: UserRole::Member,
            })
            .await
            .unwrap()
            .id
    }

    async fn seed_user_with_id(db: &DatabaseBackend, user_id: UserId) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'x', 'admin', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(user_id.0.to_string())
        .bind(format!("terminal-user-{}", user_id.0))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_totp_enabled_user(db: &DatabaseBackend, user_id: UserId) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query("UPDATE users SET totp_secret = ?, totp_enabled = 1 WHERE id = ?")
            .bind("totp-secret")
            .bind(user_id.0.to_string())
            .execute(pool)
            .await
            .unwrap();
    }
}
