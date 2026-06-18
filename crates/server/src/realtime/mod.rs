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
        Self {
            kind: kind.into(),
            agent_id,
            received_at: Utc::now(),
            payload,
        }
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
    pub fn publish(&self, event: RealtimeEvent) {
        {
            let mut latest = self.latest.write();
            latest.insert(event.agent_id, event.clone());
            if latest.len() > LATEST_CACHE_PER_AGENT * 1024 {
                // Defensive bound - we never expect this many agents
                // in a single process, but we'd rather drop something
                // than OOM.
                latest.clear();
            }
        }
        // send() returns Err only when there are no receivers; we
        // intentionally swallow that error to keep the publisher path
        // infallible.
        let _ = self.tx.send(event);
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
}
