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
    Ok(ws.on_upgrade(move |socket| async move {
        let _ = handle_socket(socket, hub).await;
    }))
}

async fn handle_socket(socket: WebSocket, hub: BroadcastHub) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = hub.subscribe();

    // Seed the client with whatever we already have so the dashboard
    // does not flash an empty state for three seconds.
    let snap = WsOutbound::Snapshot {
        events: hub.latest_snapshot(),
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
