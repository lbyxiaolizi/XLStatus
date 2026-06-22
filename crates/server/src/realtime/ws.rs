//! M3: WebSocket route that streams `RealtimeEvent` JSON to the
//! dashboard. `/ws/servers` accepts a session cookie (same as the REST
//! API), seeds the response with the cached latest snapshot, then
//! forwards every published event until the client disconnects or the
//! server is shut down.
//!
//! Frame format (text JSON):
//!   {"type":"snapshot","events":[ ... RealtimeEvent ... ]}
//!   {"type":"event","event":{ ... RealtimeEvent ... }}
//!   {"type":"ping","ts":"..."}

use crate::api::v1::auth::{AppError, AppState};
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
    crate::security::validate_websocket_origin(&headers, &state.config.server.cors_allowed_origins)
        .map_err(AppError::Forbidden)?;
    // Reuse the same scope check as the REST read endpoint.
    if !crate::auth::rbac::has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    let hub = state.realtime.clone();
    let visible_agents = realtime_visible_agent_ids(&state, &auth).await?;
    Ok(ws.on_upgrade(move |socket| async move {
        let _ = handle_socket(socket, hub, visible_agents).await;
    }))
}

async fn handle_socket(socket: WebSocket, hub: BroadcastHub, visible_agents: HashSet<uuid::Uuid>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = hub.subscribe();

    // Seed the client with whatever we already have so the dashboard
    // does not flash an empty state for three seconds.
    let snap = WsOutbound::Snapshot {
        events: filter_realtime_events(hub.latest_snapshot(), &visible_agents),
    };
    if let Ok(text) = serde_json::to_string(&snap) {
        if sender.send(Message::Text(text)).await.is_err() {
            return;
        }
    }

    // Spawn a task that drains broadcast -> ws, and select on either
    // a client close or a broadcast error so we tear down cleanly.
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
                            if !visible_agents.contains(&event.agent_id) {
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
                            // Slow consumer: skip missed messages and keep going.
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

    // Drain incoming messages (mostly pings / close frames). Anything
    // the client sends is ignored except for explicit Close.
    let mut recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
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

async fn realtime_visible_agent_ids(
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

    Ok(agents.into_iter().map(|agent| agent.id.0).collect())
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
    fn realtime_snapshot_filters_invisible_events() {
        let allowed = uid(1);
        let denied = uid(2);
        let visible_agents = HashSet::from([allowed]);
        let events = vec![
            RealtimeEvent::new("host_state", allowed, serde_json::json!({"ok": true})),
            RealtimeEvent::new("host_state", denied, serde_json::json!({"secret": true})),
        ];

        let filtered = filter_realtime_events(events, &visible_agents);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].agent_id, allowed);
    }

    #[tokio::test]
    async fn realtime_member_session_sees_only_owned_agents() {
        let state = test_state().await;
        let owner = uid(11);
        let other = uid(12);
        let owned_agent = uid(21);
        let other_agent = uid(22);
        seed_user(&state.db, owner, "owner", UserRole::Member).await;
        seed_user(&state.db, other, "other", UserRole::Member).await;
        seed_agent(&state.db, owned_agent, owner, "owned").await;
        seed_agent(&state.db, other_agent, other, "other").await;

        let auth = auth_session(owner, UserRole::Member, None);
        let visible = realtime_visible_agent_ids(&state, &auth).await.unwrap();
        assert!(visible.contains(&owned_agent));
        assert!(!visible.contains(&other_agent));
    }

    #[tokio::test]
    async fn realtime_non_admin_pat_allowlist_is_still_owner_scoped() {
        let state = test_state().await;
        let owner = uid(31);
        let other = uid(32);
        let owned_agent = uid(41);
        let other_agent = uid(42);
        seed_user(&state.db, owner, "owner", UserRole::Member).await;
        seed_user(&state.db, other, "other", UserRole::Member).await;
        seed_agent(&state.db, owned_agent, owner, "owned").await;
        seed_agent(&state.db, other_agent, other, "other").await;

        let auth = auth_session(
            owner,
            UserRole::Member,
            Some(vec![owned_agent.to_string(), other_agent.to_string()]),
        );
        let visible = realtime_visible_agent_ids(&state, &auth).await.unwrap();
        assert!(visible.contains(&owned_agent));
        assert!(!visible.contains(&other_agent));
    }

    #[tokio::test]
    async fn realtime_admin_pat_allowlist_can_span_owners() {
        let state = test_state().await;
        let owner = uid(51);
        let other = uid(52);
        let owned_agent = uid(61);
        let other_agent = uid(62);
        seed_user(&state.db, owner, "owner", UserRole::Admin).await;
        seed_user(&state.db, other, "other", UserRole::Member).await;
        seed_agent(&state.db, owned_agent, owner, "owned").await;
        seed_agent(&state.db, other_agent, other, "other").await;

        let auth = auth_session(owner, UserRole::Admin, Some(vec![other_agent.to_string()]));
        let visible = realtime_visible_agent_ids(&state, &auth).await.unwrap();
        assert!(!visible.contains(&owned_agent));
        assert!(visible.contains(&other_agent));
    }
}
