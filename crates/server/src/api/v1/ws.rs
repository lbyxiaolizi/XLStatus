//! Hardened `/ws/servers` WebSocket route with CSWSH Origin validation,
//! strict frame/message size limits, PAT `server_ids` allowlist enforcement,
//! and live per-event ownership & revocation re-validation.

use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::servers::{agent_visible, server_visible};
use crate::auth::middleware::AuthSession;
use crate::db::AgentRepository;
use crate::realtime::{BroadcastHub, RealtimeEvent};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::HeaderMap,
    response::Response,
};
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::HashSet;
use tokio::time::{interval, Duration};
use xlstatus_shared::AgentId;

pub const WS_SERVERS_MAX_MESSAGE_BYTES: usize = 4 * 1024;
pub const WS_SERVERS_MAX_FRAME_BYTES: usize = 4 * 1024;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsOutbound {
    Snapshot { events: Vec<RealtimeEvent> },
    Event { event: RealtimeEvent },
    Ping { ts: chrono::DateTime<chrono::Utc> },
}

pub async fn ws_servers(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let _ = crate::realtime::ws::ws_servers;
    validate_ws_servers_handshake(&state, &auth, &headers)?;
    let hub = state.realtime.clone();
    let visible_agents = realtime_visible_agent_ids(&state, &auth).await?;
    let state_for_socket = state.clone();
    let auth_for_socket = auth.clone();
    Ok(ws
        .max_message_size(WS_SERVERS_MAX_MESSAGE_BYTES)
        .max_frame_size(WS_SERVERS_MAX_FRAME_BYTES)
        .on_upgrade(move |socket| async move {
            let _ = handle_socket(
                socket,
                state_for_socket,
                auth_for_socket,
                hub,
                visible_agents,
            )
            .await;
        }))
}

fn validate_ws_servers_handshake(
    state: &AppState,
    auth: &AuthSession,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    crate::security::validate_websocket_origin(headers, &state.config.server.cors_allowed_origins)
        .map_err(AppError::Forbidden)?;
    if !crate::auth::rbac::has_scope(auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    Ok(())
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    auth: AuthSession,
    hub: BroadcastHub,
    visible_agents: HashSet<uuid::Uuid>,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = hub.subscribe();

    let snap = WsOutbound::Snapshot {
        events: filter_realtime_events(hub.latest_snapshot(), &visible_agents),
    };
    if let Ok(text) = serde_json::to_string(&snap) {
        if sender.send(Message::Text(text)).await.is_err() {
            return;
        }
    }

    let mut send_task = tokio::spawn(async move {
        let mut ping_tick = interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                biased;
                _ = ping_tick.tick() => {
                    let msg = WsOutbound::Ping {
                        ts: chrono::Utc::now(),
                    };
                    let text = match serde_json::to_string(&msg) {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    if sender.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                event = rx.recv() => {
                    match event {
                        Ok(event) => {
                            if !realtime_event_visible(&state, &auth, &event).await {
                                continue;
                            }
                            let msg = WsOutbound::Event { event };
                            let text = match serde_json::to_string(&msg) {
                                Ok(s) => s,
                                Err(_) => continue,
                            };
                            if sender.send(Message::Text(text)).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    };
                }
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) if text.len() > WS_SERVERS_MAX_MESSAGE_BYTES => break,
                Ok(Message::Binary(bytes)) if bytes.len() > WS_SERVERS_MAX_MESSAGE_BYTES => break,
                Ok(Message::Close(_)) | Err(_) => break,
                _ => continue,
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

pub(crate) async fn realtime_visible_agent_ids(
    state: &AppState,
    auth: &AuthSession,
) -> Result<HashSet<uuid::Uuid>, AppError> {
    let agent_repo = AgentRepository::new(state.db.clone());
    let agents = if let Some(server_ids) = auth.server_ids.as_deref() {
        let owner_filter = (!auth.role.is_admin()).then_some(auth.user_id);
        let (rows, _) = agent_repo
            .list_with_state_by_server_ids(owner_filter, server_ids, i64::MAX, 0)
            .await?;
        rows.into_iter().map(|row| row.agent).collect()
    } else if auth.role.is_admin() {
        let (rows, _) = agent_repo.list_with_state(i64::MAX, 0).await?;
        rows.into_iter().map(|row| row.agent).collect()
    } else {
        agent_repo.list_by_owner(auth.user_id, i64::MAX, 0).await?
    };

    Ok(agents
        .into_iter()
        .filter(|agent| agent.revoked_at.is_none() && agent_visible(auth, agent))
        .map(|agent| agent.id.0)
        .collect())
}

pub(crate) async fn realtime_event_visible(
    state: &AppState,
    auth: &AuthSession,
    event: &RealtimeEvent,
) -> bool {
    let agent_id = AgentId(event.agent_id);
    if !server_visible(auth, &agent_id) {
        return false;
    }
    let Ok(Some(agent)) = AgentRepository::new(state.db.clone())
        .find_by_id(agent_id)
        .await
    else {
        return false;
    };
    agent.revoked_at.is_none() && agent_visible(auth, &agent)
}

fn filter_realtime_events(
    events: Vec<RealtimeEvent>,
    visible_agents: &HashSet<uuid::Uuid>,
) -> Vec<RealtimeEvent> {
    events
        .into_iter()
        .filter(|event| visible_agents.contains(&event.agent_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::middleware::AuthKind;
    use crate::db::DatabaseBackend;
    use axum::http::header;
    use std::sync::Arc;
    use uuid::Uuid;
    use xlstatus_shared::{UserId, UserRole};

    fn uid(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    fn auth_session(user_id: Uuid, role: UserRole, server_ids: Option<Vec<String>>) -> AuthSession {
        AuthSession {
            session_id: "session".into(),
            user_id: UserId(user_id),
            username: "user".into(),
            role,
            csrf_token: "csrf".into(),
            auth_kind: if server_ids.is_some() {
                AuthKind::PersonalAccessToken
            } else {
                AuthKind::Session
            },
            scopes: vec!["server:read".into()],
            server_ids,
            pat_id: None,
        }
    }

    async fn test_state() -> AppState {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        AppState {
            db,
            config: Arc::new(crate::config::Config::default()),
            agent_jwt_challenges: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            metrics: xlstatus_tsdb::MetricStore::in_memory(),
            realtime: BroadcastHub::new(),
            session_registry: crate::grpc::SessionRegistry::new(),
            terminal_sessions: crate::api::v1::terminal::TerminalSessionRegistry::new(),
            io_registry: crate::grpc::IoRegistry::new(),
        }
    }

    async fn seed_user(db: &DatabaseBackend, id: Uuid, username: &str, role: UserRole) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'x', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(role.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_agent(db: &DatabaseBackend, id: Uuid, owner: Uuid, name: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at) VALUES (?, ?, 'pk', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(owner.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    #[test]
    fn ws_servers_resource_limits_are_explicit() {
        assert_eq!(WS_SERVERS_MAX_MESSAGE_BYTES, 4 * 1024);
        assert_eq!(WS_SERVERS_MAX_FRAME_BYTES, 4 * 1024);
    }

    #[tokio::test]
    async fn ws_servers_handshake_validates_origin_and_scope() {
        let state = test_state().await;
        let user = uid(1);
        let auth = auth_session(user, UserRole::Member, None);

        let mut evil_headers = HeaderMap::new();
        evil_headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        evil_headers.insert(header::HOST, "status.example.com".parse().unwrap());
        let err = validate_ws_servers_handshake(&state, &auth, &evil_headers).unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));

        let mut same_origin_headers = HeaderMap::new();
        same_origin_headers.insert(
            header::ORIGIN,
            "https://status.example.com".parse().unwrap(),
        );
        same_origin_headers.insert(header::HOST, "status.example.com".parse().unwrap());
        assert!(validate_ws_servers_handshake(&state, &auth, &same_origin_headers).is_ok());

        let mut no_scope_auth = auth_session(user, UserRole::Member, Some(vec![]));
        no_scope_auth.scopes.clear();
        let err = validate_ws_servers_handshake(&state, &no_scope_auth, &same_origin_headers)
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn ws_servers_excludes_revoked_and_transferred_agents_live() {
        let state = test_state().await;
        let owner = uid(10);
        let other = uid(11);
        let server_a = uid(20);
        let server_b = uid(21);
        seed_user(&state.db, owner, "owner", UserRole::Member).await;
        seed_user(&state.db, other, "other", UserRole::Member).await;
        seed_agent(&state.db, server_a, owner, "server-a").await;
        seed_agent(&state.db, server_b, owner, "server-b").await;

        let auth = auth_session(owner, UserRole::Member, None);
        let event_a = RealtimeEvent::new("host_state", server_a, serde_json::json!({"cpu": 10}));
        let event_b = RealtimeEvent::new("host_state", server_b, serde_json::json!({"cpu": 20}));

        assert!(realtime_event_visible(&state, &auth, &event_a).await);
        assert!(realtime_event_visible(&state, &auth, &event_b).await);

        // Revoke server_a while WebSocket session is open
        AgentRepository::new(state.db.clone())
            .revoke(AgentId(server_a))
            .await
            .unwrap();
        assert!(!realtime_event_visible(&state, &auth, &event_a).await);
        let visible = realtime_visible_agent_ids(&state, &auth).await.unwrap();
        assert!(!visible.contains(&server_a));
        assert!(visible.contains(&server_b));

        // Transfer server_b to another user while WebSocket session is open
        let DatabaseBackend::Sqlite(pool) = &state.db else {
            unreachable!();
        };
        sqlx::query("UPDATE agents SET owner_user_id = ? WHERE id = ?")
            .bind(other.to_string())
            .bind(server_b.to_string())
            .execute(pool)
            .await
            .unwrap();
        assert!(!realtime_event_visible(&state, &auth, &event_b).await);
    }

    #[tokio::test]
    async fn ws_servers_pat_allowlist_blocks_unlisted_and_cross_owner_events() {
        let state = test_state().await;
        let owner = uid(30);
        let other = uid(31);
        let allowed_server = uid(40);
        let unlisted_owned_server = uid(41);
        let foreign_server = uid(42);
        seed_user(&state.db, owner, "owner", UserRole::Member).await;
        seed_user(&state.db, other, "other", UserRole::Member).await;
        seed_agent(&state.db, allowed_server, owner, "allowed").await;
        seed_agent(&state.db, unlisted_owned_server, owner, "unlisted").await;
        seed_agent(&state.db, foreign_server, other, "foreign").await;

        let auth = auth_session(
            owner,
            UserRole::Member,
            Some(vec![allowed_server.to_string(), foreign_server.to_string()]),
        );

        let allowed_ev = RealtimeEvent::new(
            "host_state",
            allowed_server,
            serde_json::json!({"ok": true}),
        );
        let unlisted_ev = RealtimeEvent::new(
            "host_state",
            unlisted_owned_server,
            serde_json::json!({"ok": false}),
        );
        let foreign_ev = RealtimeEvent::new(
            "host_state",
            foreign_server,
            serde_json::json!({"ok": false}),
        );

        assert!(realtime_event_visible(&state, &auth, &allowed_ev).await);
        assert!(!realtime_event_visible(&state, &auth, &unlisted_ev).await);
        assert!(!realtime_event_visible(&state, &auth, &foreign_ev).await);
    }
}
