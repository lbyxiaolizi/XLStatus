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
use crate::db::{Agent, AgentRepository};
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
use tokio::time::{interval, timeout, Duration};
use xlstatus_shared::AgentId;

const WS_SERVERS_MAX_INBOUND_BYTES: usize = 4 * 1024;
const WS_SERVERS_SEND_TIMEOUT: Duration = Duration::from_secs(10);
const WS_SERVERS_PING_INTERVAL: Duration = Duration::from_secs(30);

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
    Ok(ws
        .max_message_size(WS_SERVERS_MAX_INBOUND_BYTES)
        .max_frame_size(WS_SERVERS_MAX_INBOUND_BYTES)
        .on_upgrade(move |socket| async move {
            let _ = handle_socket(socket, state, auth, hub, visible_agents).await;
        }))
}

async fn send_ws_outbound(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    outbound: &WsOutbound,
) -> bool {
    let Ok(text) = serde_json::to_string(outbound) else {
        return false;
    };
    matches!(
        timeout(WS_SERVERS_SEND_TIMEOUT, sender.send(Message::Text(text))).await,
        Ok(Ok(()))
    )
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    auth: AuthSession,
    hub: BroadcastHub,
    mut visible_agents: HashSet<uuid::Uuid>,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = hub.subscribe();

    // Seed the client with whatever we already have so the dashboard
    // does not flash an empty state for three seconds.
    let snap = WsOutbound::Snapshot {
        events: filter_realtime_events(hub.latest_snapshot(), &visible_agents),
    };
    if !send_ws_outbound(&mut sender, &snap).await {
        return;
    }

    // Spawn a task that drains broadcast -> ws, and select on either
    // a client close or a broadcast error so we tear down cleanly.
    let mut send_task = tokio::spawn(async move {
        let mut ping_tick = interval(WS_SERVERS_PING_INTERVAL);
        loop {
            tokio::select! {
                biased;
                _ = ping_tick.tick() => {
                    let msg = WsOutbound::Ping {
                        ts: chrono::Utc::now(),
                    };
                    if !send_ws_outbound(&mut sender, &msg).await {
                        break;
                    }
                }
                event = rx.recv() => {
                    match event {
                        Ok(event) => {
                            if !realtime_event_visible(&state, &auth, &mut visible_agents, event.agent_id).await {
                                continue;
                            }
                            let msg = WsOutbound::Event { event };
                            if !send_ws_outbound(&mut sender, &msg).await {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // Slow consumer: refresh the visible agent set and send a fresh snapshot.
                            let Ok(refreshed) = realtime_visible_agent_ids(&state, &auth).await else {
                                break;
                            };
                            visible_agents = refreshed;
                            let snap = WsOutbound::Snapshot {
                                events: filter_realtime_events(hub.latest_snapshot(), &visible_agents),
                            };
                            if !send_ws_outbound(&mut sender, &snap).await {
                                break;
                            }
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
    // the client sends is ignored except for explicit Close or oversized frames.
    let mut recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(Message::Text(text)) if text.len() > WS_SERVERS_MAX_INBOUND_BYTES => break,
                Ok(Message::Binary(bytes)) if bytes.len() > WS_SERVERS_MAX_INBOUND_BYTES => break,
                _ => continue,
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

fn agent_visible(auth: &AuthSession, agent: &Agent) -> bool {
    if agent.revoked_at.is_some() {
        return false;
    }
    if !auth.role.is_admin() && agent.owner_user_id != auth.user_id {
        return false;
    }
    if let Some(allowed) = auth.server_ids.as_ref() {
        let id = agent.id.0.to_string();
        if !allowed.iter().any(|candidate| candidate == &id) {
            return false;
        }
    }
    true
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

    Ok(agents
        .into_iter()
        .filter(|agent| agent.revoked_at.is_none() && agent_visible(auth, agent))
        .map(|agent| agent.id.0)
        .collect())
}

async fn realtime_event_visible(
    state: &AppState,
    auth: &AuthSession,
    visible_agents: &mut HashSet<uuid::Uuid>,
    agent_id: uuid::Uuid,
) -> bool {
    if let Some(allowed) = auth.server_ids.as_ref() {
        let id_str = agent_id.to_string();
        if !allowed.iter().any(|candidate| candidate == &id_str) {
            visible_agents.remove(&agent_id);
            return false;
        }
    }

    let agent_repo = AgentRepository::new(state.db.clone());
    let Ok(Some(agent)) = agent_repo.find_by_id(AgentId(agent_id)).await else {
        visible_agents.remove(&agent_id);
        return false;
    };
    if agent.revoked_at.is_some() {
        state.realtime.remove_agent(&agent_id);
        visible_agents.remove(&agent_id);
        return false;
    }
    let visible = agent_visible(auth, &agent);
    if visible {
        visible_agents.insert(agent_id);
    } else {
        visible_agents.remove(&agent_id);
    }
    visible
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

    #[tokio::test]
    async fn realtime_excludes_revoked_agents_and_drops_visibility_after_revoke_or_transfer() {
        let state = test_state().await;
        let owner = uid(71);
        let new_owner = uid(72);
        let active_agent = uid(81);
        let revoked_agent = uid(82);
        seed_user(&state.db, owner, "owner", UserRole::Member).await;
        seed_user(&state.db, new_owner, "new-owner", UserRole::Member).await;
        seed_agent(&state.db, active_agent, owner, "active").await;
        seed_agent(&state.db, revoked_agent, owner, "revoked").await;

        let DatabaseBackend::Sqlite(pool) = &state.db else {
            unreachable!();
        };
        sqlx::query("UPDATE agents SET revoked_at = '2026-06-22T00:00:00Z' WHERE id = ?")
            .bind(revoked_agent.to_string())
            .execute(pool)
            .await
            .unwrap();

        let auth = auth_session(owner, UserRole::Member, None);
        let mut visible = realtime_visible_agent_ids(&state, &auth).await.unwrap();
        assert!(visible.contains(&active_agent));
        assert!(!visible.contains(&revoked_agent));

        state.realtime.publish(RealtimeEvent::new(
            "host_state",
            active_agent,
            serde_json::json!({"cpu": 10.0}),
        ));
        assert!(realtime_event_visible(&state, &auth, &mut visible, active_agent).await);

        // Transfer ownership to new_owner while WebSocket session is open:
        sqlx::query("UPDATE agents SET owner_user_id = ? WHERE id = ?")
            .bind(new_owner.to_string())
            .bind(active_agent.to_string())
            .execute(pool)
            .await
            .unwrap();
        assert!(!realtime_event_visible(&state, &auth, &mut visible, active_agent).await);
        assert!(!visible.contains(&active_agent));

        // Transfer back, then revoke while WebSocket session is open:
        sqlx::query(
            "UPDATE agents SET owner_user_id = ?, revoked_at = '2026-06-22T01:00:00Z' WHERE id = ?",
        )
        .bind(owner.to_string())
        .bind(active_agent.to_string())
        .execute(pool)
        .await
        .unwrap();
        visible.insert(active_agent);
        assert!(!realtime_event_visible(&state, &auth, &mut visible, active_agent).await);
        assert!(!visible.contains(&active_agent));
        assert!(state.realtime.latest_snapshot().is_empty());
    }
}
