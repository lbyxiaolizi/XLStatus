//! M3: in-process pub/sub used by the gRPC `Session` task to fan out
//! HostState samples to any number of WebSocket subscribers.
//!
//! One `BroadcastHub` is created at startup and shared between the
//! gRPC layer (publisher) and the `/ws/servers` axum route
//! (subscriber). Each subscriber gets its own `tokio::sync::broadcast`
//! receiver; slow consumers drop messages instead of stalling the
//! pipeline. The hub also keeps a small "latest sample" ring so a
//! fresh subscriber can render the current state immediately without
//! waiting for the next HostState tick.

pub mod ws;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

const BROADCAST_CAPACITY: usize = 256;
const LATEST_CACHE_PER_AGENT: usize = 1;
const LATEST_CACHE_MAX_AGENTS: usize = LATEST_CACHE_PER_AGENT * 1024;
const MAX_REALTIME_KIND_CHARS: usize = 64;
const MAX_REALTIME_JSON_DEPTH: usize = 8;
const MAX_REALTIME_OBJECT_KEYS: usize = 128;
const MAX_REALTIME_ARRAY_LEN: usize = 256;
const MAX_REALTIME_STRING_CHARS: usize = 512;

/// One event on the wire. `agent_id` is the source; `payload` is the
/// same JSON the gRPC layer also persisted to `agents.last_state_json`
/// and to the in-memory `MetricStore`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeEvent {
    pub kind: String,
    pub agent_id: Uuid,
    pub received_at: DateTime<Utc>,
    pub payload: serde_json::Value,
}

impl RealtimeEvent {
    pub fn new(kind: impl Into<String>, agent_id: Uuid, payload: serde_json::Value) -> Self {
        let kind = sanitize_realtime_kind(&kind.into());
        let payload = sanitize_realtime_payload(payload, 0);
        Self {
            kind,
            agent_id,
            received_at: Utc::now(),
            payload,
        }
    }
}

fn sanitize_realtime_kind(kind: &str) -> String {
    let cleaned: String = kind
        .chars()
        .filter(|ch| !ch.is_control() && !ch.is_whitespace())
        .take(MAX_REALTIME_KIND_CHARS)
        .collect();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

fn is_sensitive_realtime_key(key: &str) -> bool {
    let lower = key.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "private_key"
            | "public_key"
            | "owner_user_id"
            | "password"
            | "password_hash"
            | "secret"
            | "csrf_token"
            | "enrollment_token"
            | "access_token"
            | "refresh_token"
            | "api_key"
            | "bearer"
            | "authorization"
            | "cookie"
            | "session_id"
            | "totp_secret"
    ) || lower.ends_with("_secret")
        || lower.ends_with("_token")
        || lower.ends_with("_private_key")
}

fn sanitize_realtime_payload(value: serde_json::Value, depth: usize) -> serde_json::Value {
    if depth > MAX_REALTIME_JSON_DEPTH {
        return serde_json::Value::Null;
    }
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len().min(MAX_REALTIME_OBJECT_KEYS));
            for (raw_key, val) in map {
                if out.len() >= MAX_REALTIME_OBJECT_KEYS {
                    break;
                }
                let key: String = raw_key
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(128)
                    .collect();
                if key.is_empty() || is_sensitive_realtime_key(&key) {
                    continue;
                }
                out.insert(key, sanitize_realtime_payload(val, depth + 1));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .take(MAX_REALTIME_ARRAY_LEN)
                .map(|item| sanitize_realtime_payload(item, depth + 1))
                .collect(),
        ),
        serde_json::Value::String(s) => serde_json::Value::String(
            s.chars()
                .filter(|ch| !ch.is_control())
                .take(MAX_REALTIME_STRING_CHARS)
                .collect(),
        ),
        other => other,
    }
}

/// Shared hub. Cheap to clone (Arc inside).
#[derive(Clone)]
pub struct BroadcastHub {
    tx: broadcast::Sender<RealtimeEvent>,
    latest: Arc<RwLock<HashMap<Uuid, RealtimeEvent>>>,
}

impl Default for BroadcastHub {
    fn default() -> Self {
        Self::new()
    }
}

impl BroadcastHub {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            tx,
            latest: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Publish a HostState or HostInfo event. The hub also remembers
    /// the freshest event per agent so a new dashboard tab can render
    /// current state without waiting for the next tick.
    pub fn publish(&self, mut event: RealtimeEvent) {
        event.kind = sanitize_realtime_kind(&event.kind);
        event.payload = sanitize_realtime_payload(event.payload, 0);
        {
            let mut latest = self.latest.write();
            if latest.len() >= LATEST_CACHE_MAX_AGENTS && !latest.contains_key(&event.agent_id) {
                // Defensive bound - evict the oldest cached agent entry rather than
                // wiping the entire snapshot map on overflow.
                if let Some(oldest_id) = latest
                    .iter()
                    .min_by_key(|(_, cached)| cached.received_at)
                    .map(|(id, _)| *id)
                {
                    latest.remove(&oldest_id);
                }
            }
            latest.insert(event.agent_id, event.clone());
        }
        // send() returns Err only when there are no receivers; we
        // intentionally swallow that error to keep the publisher path
        // infallible.
        let _ = self.tx.send(event);
    }

    /// Evict a revoked or deleted agent from the latest snapshot cache.
    pub fn remove_agent(&self, agent_id: &Uuid) {
        self.latest.write().remove(agent_id);
    }

    /// Subscribe to all events. Returns a receiver that yields the
    /// most recent cached snapshot first, then forwards live events.
    pub fn subscribe(&self) -> broadcast::Receiver<RealtimeEvent> {
        self.tx.subscribe()
    }

    /// Snapshot of the latest event per agent, used by the WS handler
    /// to seed a new dashboard tab.
    pub fn latest_snapshot(&self) -> Vec<RealtimeEvent> {
        self.latest.read().values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn uid(b: u8) -> Uuid {
        Uuid::from_bytes([b; 16])
    }

    #[tokio::test]
    async fn publish_then_subscribe_receives() {
        let hub = BroadcastHub::new();
        let mut rx = hub.subscribe();
        hub.publish(RealtimeEvent::new("host_state", uid(1), json!({"cpu":1.0})));
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.kind, "host_state");
        assert_eq!(ev.agent_id, uid(1));
        assert_eq!(ev.payload["cpu"], 1.0);
    }

    #[tokio::test]
    async fn latest_snapshot_keeps_most_recent() {
        let hub = BroadcastHub::new();
        hub.publish(RealtimeEvent::new("host_state", uid(2), json!({"v":1})));
        hub.publish(RealtimeEvent::new("host_state", uid(2), json!({"v":2})));
        let snap = hub.latest_snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].payload["v"], 2);
    }

    #[tokio::test]
    async fn multiple_subscribers_each_get_event() {
        let hub = BroadcastHub::new();
        let mut a = hub.subscribe();
        let mut b = hub.subscribe();
        hub.publish(RealtimeEvent::new("host_state", uid(3), json!({})));
        a.recv().await.unwrap();
        b.recv().await.unwrap();
    }

    #[tokio::test]
    async fn no_subscribers_does_not_panic() {
        let hub = BroadcastHub::new();
        // publish with zero subscribers must not error or panic.
        hub.publish(RealtimeEvent::new("host_state", uid(4), json!({})));
    }

    #[tokio::test]
    async fn realtime_event_strips_sensitive_keys_and_control_characters() {
        let hub = BroadcastHub::new();
        let mut rx = hub.subscribe();
        hub.publish(RealtimeEvent::new(
            "host_info\u{0007}\n",
            uid(9),
            json!({
                "hostname": "edge-\u{0000}node\u{001b}[31m",
                "cpu_percent": 15.5,
                "public_key": "ed25519-secret-key",
                "private_key": "never-broadcast",
                "owner_user_id": "00000000-0000-0000-0000-000000000001",
                "enrollment_token": "tok_secret",
                "nested": {
                    "api_token": "secret",
                    "safe_label": "ok\u{0007}"
                }
            }),
        ));

        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.kind, "host_info");
        assert_eq!(ev.payload["hostname"], "edge-node[31m");
        assert_eq!(ev.payload["cpu_percent"], 15.5);
        assert!(ev.payload.get("public_key").is_none());
        assert!(ev.payload.get("private_key").is_none());
        assert!(ev.payload.get("owner_user_id").is_none());
        assert!(ev.payload.get("enrollment_token").is_none());
        assert!(ev.payload["nested"].get("api_token").is_none());
        assert_eq!(ev.payload["nested"]["safe_label"], "ok");
    }
}
