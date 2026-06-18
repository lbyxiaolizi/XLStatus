//! Minimal embedded time-series store and backend facade.
//!
//! The plan allows a real TSDB (VictoriaMetrics / ClickHouse /
//! TimescaleDB) to land behind the same `MetricStore` API. This crate
//! provides the interface every component should depend on, plus a
//! working in-memory implementation that:
//!
//!   * appends `(agent_id, sample_at, fields_json)` rows
//!   * supports batch writes for high-frequency agent samples
//!   * supports 1d / 7d / 30d queries (returns series ordered by `sample_at`)
//!   * is safe to share across axum handlers via `Arc<MetricStore>`
//!
//! An external backend can drop in without changing callers by
//! implementing `MetricBackend` and constructing `MetricStore::from_backend`.

use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum MetricError {
    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),
    #[error("metric backend error: {0}")]
    Backend(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AgentId(pub Uuid);

/// One snapshot of an agent's state. `fields_json` is a free-form
/// `serde_json::Value` so the gRPC layer can pass through the same JSON
/// it persists to `agents.last_state_json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    pub agent_id: AgentId,
    pub sample_at: DateTime<Utc>,
    pub fields_json: serde_json::Value,
}

/// Bounded series returned from a query. `samples` is ordered ascending
/// by `sample_at`. `latest` is just `samples.last()` cached for callers
/// that only need the freshest row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSeries {
    pub agent_id: AgentId,
    pub samples: Vec<MetricSample>,
}

impl MetricSeries {
    pub fn latest(&self) -> Option<&MetricSample> {
        self.samples.last()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricBackendStatus {
    Healthy,
    Degraded,
    Unavailable,
}

/// Lightweight health report exposed by every metric backend. External
/// stores can use `detail` for endpoint names, queue depth, or the last
/// failed flush reason without leaking backend-specific types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetricBackendHealth {
    pub backend: String,
    pub status: MetricBackendStatus,
    pub detail: Option<String>,
    pub samples: Option<usize>,
}

impl MetricBackendHealth {
    pub fn healthy(backend: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            status: MetricBackendStatus::Healthy,
            detail: None,
            samples: None,
        }
    }

    pub fn with_samples(mut self, samples: usize) -> Self {
        self.samples = Some(samples);
        self
    }
}

/// Windowed query range. `1d` keeps the last day of samples, `7d` the
/// last week, and so on. Anything not in this enum is rejected.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QueryRange {
    #[serde(rename = "1d")]
    Day1,
    #[serde(rename = "7d")]
    Day7,
    #[serde(rename = "30d")]
    Day30,
}

impl QueryRange {
    pub fn to_chrono_duration(self) -> Duration {
        match self {
            QueryRange::Day1 => Duration::days(1),
            QueryRange::Day7 => Duration::days(7),
            QueryRange::Day30 => Duration::days(30),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            QueryRange::Day1 => "1d",
            QueryRange::Day7 => "7d",
            QueryRange::Day30 => "30d",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "1d" => Some(QueryRange::Day1),
            "7d" => Some(QueryRange::Day7),
            "30d" => Some(QueryRange::Day30),
            _ => None,
        }
    }
}

/// The trait every backend must implement. The default in-memory
/// implementation in this crate is what the rest of the codebase uses
/// locally; M8 can add a `VictoriaMetricsStore` / `ClickHouseStore`
/// without changing the server call sites.
pub trait MetricBackend: Send + Sync + 'static {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn write(&self, sample: MetricSample) -> Result<(), MetricError>;

    fn write_batch(&self, samples: Vec<MetricSample>) -> Result<usize, MetricError> {
        let count = samples.len();
        for sample in samples {
            self.write(sample)?;
        }
        Ok(count)
    }

    fn query(&self, agent_id: AgentId, range: QueryRange) -> Result<MetricSeries, MetricError>;
    fn latest(&self, agent_id: AgentId) -> Result<Option<MetricSample>, MetricError>;
    fn list_agents(&self) -> Result<Vec<AgentId>, MetricError>;

    fn compact(&self) -> Result<usize, MetricError> {
        Ok(0)
    }

    fn health(&self) -> MetricBackendHealth {
        MetricBackendHealth::healthy(self.name())
    }
}

/// Thread-safe, bounded in-memory store. Retention defaults to 30 days
/// and is enforced on every `write` to keep memory bounded during long
/// runs.
#[derive(Debug)]
pub struct InMemoryMetricStore {
    inner: RwLock<InMemoryState>,
}

#[derive(Debug)]
struct InMemoryState {
    series: HashMap<AgentId, BTreeMap<DateTime<Utc>, serde_json::Value>>,
    retention: Duration,
}

impl Default for InMemoryMetricStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryMetricStore {
    pub fn new() -> Self {
        Self::with_retention(QueryRange::Day30.to_chrono_duration())
    }

    pub fn with_retention(retention: Duration) -> Self {
        Self {
            inner: RwLock::new(InMemoryState {
                series: HashMap::new(),
                retention,
            }),
        }
    }

    /// Drop samples older than `cutoff` for every agent. Called
    /// opportunistically inside `write`; can also be called manually.
    pub fn compact(&self) -> usize {
        let retention = self.inner.read().retention;
        let cutoff = Utc::now() - retention;
        let mut state = self.inner.write();
        let mut dropped = 0;
        for entries in state.series.values_mut() {
            // BTreeMap is ordered by key, so split_off is O(log n) plus
            // the size of the dropped prefix.
            let before = entries.len();
            let prefix = entries.split_off(&cutoff);
            *entries = prefix;
            dropped += before.saturating_sub(entries.len());
        }
        dropped
    }

    /// Total number of samples currently held. Useful in tests and
    /// admin debug endpoints.
    pub fn len(&self) -> usize {
        self.inner.read().series.values().map(|m| m.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl MetricBackend for InMemoryMetricStore {
    fn name(&self) -> &'static str {
        "in_memory"
    }

    fn write(&self, sample: MetricSample) -> Result<(), MetricError> {
        if sample.sample_at.timestamp() < 0 {
            return Err(MetricError::InvalidTimestamp(sample.sample_at.to_rfc3339()));
        }
        let agent_id = sample.agent_id.clone();
        let ts = sample.sample_at;
        let payload = sample.fields_json.clone();
        {
            let mut state = self.inner.write();
            state
                .series
                .entry(agent_id)
                .or_default()
                .insert(ts, payload);
        }
        // Compact occasionally; cheap because most calls hit the early
        // return when nothing is outside the retention window.
        self.compact();
        Ok(())
    }

    fn write_batch(&self, samples: Vec<MetricSample>) -> Result<usize, MetricError> {
        for sample in &samples {
            if sample.sample_at.timestamp() < 0 {
                return Err(MetricError::InvalidTimestamp(sample.sample_at.to_rfc3339()));
            }
        }

        let count = samples.len();
        {
            let mut state = self.inner.write();
            for sample in samples {
                state
                    .series
                    .entry(sample.agent_id)
                    .or_default()
                    .insert(sample.sample_at, sample.fields_json);
            }
        }
        self.compact();
        Ok(count)
    }

    fn query(&self, agent_id: AgentId, range: QueryRange) -> Result<MetricSeries, MetricError> {
        let cutoff = Utc::now() - range.to_chrono_duration();
        let state = self.inner.read();
        let samples = state
            .series
            .get(&agent_id)
            .map(|entries| {
                entries
                    .range(cutoff..)
                    .map(|(ts, payload)| MetricSample {
                        agent_id: agent_id.clone(),
                        sample_at: *ts,
                        fields_json: payload.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(MetricSeries { agent_id, samples })
    }

    fn latest(&self, agent_id: AgentId) -> Result<Option<MetricSample>, MetricError> {
        let state = self.inner.read();
        Ok(state
            .series
            .get(&agent_id)
            .and_then(|entries| entries.iter().next_back())
            .map(|(ts, payload)| MetricSample {
                agent_id: agent_id.clone(),
                sample_at: *ts,
                fields_json: payload.clone(),
            }))
    }

    fn list_agents(&self) -> Result<Vec<AgentId>, MetricError> {
        Ok(self.inner.read().series.keys().cloned().collect())
    }

    fn compact(&self) -> Result<usize, MetricError> {
        Ok(InMemoryMetricStore::compact(self))
    }

    fn health(&self) -> MetricBackendHealth {
        MetricBackendHealth::healthy(self.name()).with_samples(self.len())
    }
}

/// Public facade so callers can pass around a cloneable `MetricStore`
/// without caring about the active backend.
#[derive(Clone)]
pub struct MetricStore {
    inner: Arc<dyn MetricBackend>,
}

impl MetricStore {
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(InMemoryMetricStore::new()),
        }
    }

    pub fn from_backend<B>(backend: B) -> Self
    where
        B: MetricBackend,
    {
        Self {
            inner: Arc::new(backend),
        }
    }

    pub fn from_arc_backend(inner: Arc<dyn MetricBackend>) -> Self {
        Self { inner }
    }

    pub fn backend_name(&self) -> &'static str {
        self.inner.name()
    }

    pub fn write(&self, sample: MetricSample) -> Result<(), MetricError> {
        self.inner.write(sample)
    }

    pub fn write_batch(&self, samples: Vec<MetricSample>) -> Result<usize, MetricError> {
        self.inner.write_batch(samples)
    }

    pub fn query(&self, agent_id: AgentId, range: QueryRange) -> Result<MetricSeries, MetricError> {
        self.inner.query(agent_id, range)
    }

    pub fn latest(&self, agent_id: AgentId) -> Result<Option<MetricSample>, MetricError> {
        self.inner.latest(agent_id)
    }

    pub fn list_agents(&self) -> Result<Vec<AgentId>, MetricError> {
        self.inner.list_agents()
    }

    pub fn compact(&self) -> Result<usize, MetricError> {
        self.inner.compact()
    }

    pub fn health(&self) -> MetricBackendHealth {
        self.inner.health()
    }

    /// Helper for the gRPC path: parse `(agent_id, sample_at, json)`
    /// and call `write`.
    pub fn write_json(
        &self,
        agent_id: Uuid,
        sample_at: DateTime<Utc>,
        fields_json: serde_json::Value,
    ) -> Result<(), MetricError> {
        self.write(MetricSample {
            agent_id: AgentId(agent_id),
            sample_at,
            fields_json,
        })
    }

    pub fn write_json_batch<I>(&self, samples: I) -> Result<usize, MetricError>
    where
        I: IntoIterator<Item = (Uuid, DateTime<Utc>, serde_json::Value)>,
    {
        let samples = samples
            .into_iter()
            .map(|(agent_id, sample_at, fields_json)| MetricSample {
                agent_id: AgentId(agent_id),
                sample_at,
                fields_json,
            })
            .collect();
        self.write_batch(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(byte: u8) -> AgentId {
        AgentId(Uuid::from_bytes([byte; 16]))
    }

    #[test]
    fn write_then_latest_round_trip() {
        let store = InMemoryMetricStore::new();
        let agent = id(1);
        let now = Utc::now();
        store
            .write(MetricSample {
                agent_id: agent.clone(),
                sample_at: now,
                fields_json: json!({ "cpu": 12.5 }),
            })
            .unwrap();
        let latest = store.latest(agent.clone()).unwrap().unwrap();
        assert_eq!(latest.fields_json["cpu"], 12.5);
        assert_eq!(latest.sample_at, now);
    }

    #[test]
    fn query_respects_window() {
        let store = InMemoryMetricStore::new();
        let agent = id(2);
        let now = Utc::now();
        let old = now - Duration::days(2);
        let recent = now - Duration::hours(1);
        store
            .write(MetricSample {
                agent_id: agent.clone(),
                sample_at: old,
                fields_json: json!({ "cpu": 1.0 }),
            })
            .unwrap();
        store
            .write(MetricSample {
                agent_id: agent.clone(),
                sample_at: recent,
                fields_json: json!({ "cpu": 2.0 }),
            })
            .unwrap();
        let series = store.query(agent.clone(), QueryRange::Day1).unwrap();
        assert_eq!(series.samples.len(), 1);
        assert_eq!(series.samples[0].fields_json["cpu"], 2.0);
    }

    #[test]
    fn retention_drops_old_samples() {
        let store = InMemoryMetricStore::with_retention(Duration::seconds(1));
        let agent = id(3);
        let old = Utc::now() - Duration::seconds(10);
        store
            .write(MetricSample {
                agent_id: agent.clone(),
                sample_at: old,
                fields_json: json!({ "cpu": 9.0 }),
            })
            .unwrap();
        assert!(store.latest(agent).unwrap().is_none());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn compact_reports_dropped_samples() {
        let store = InMemoryMetricStore::with_retention(Duration::seconds(1));
        let agent = id(8);
        store
            .write_batch(vec![
                MetricSample {
                    agent_id: agent.clone(),
                    sample_at: Utc::now() - Duration::seconds(10),
                    fields_json: json!({ "cpu": 1.0 }),
                },
                MetricSample {
                    agent_id: agent,
                    sample_at: Utc::now(),
                    fields_json: json!({ "cpu": 2.0 }),
                },
            ])
            .unwrap();

        assert_eq!(store.len(), 1);
        assert_eq!(store.compact(), 0);
    }

    #[test]
    fn list_agents_returns_distinct_ids() {
        let store = InMemoryMetricStore::new();
        let a = id(4);
        let b = id(5);
        let now = Utc::now();
        store
            .write(MetricSample {
                agent_id: a.clone(),
                sample_at: now,
                fields_json: json!({}),
            })
            .unwrap();
        store
            .write(MetricSample {
                agent_id: b.clone(),
                sample_at: now,
                fields_json: json!({}),
            })
            .unwrap();
        let mut agents = store.list_agents().unwrap();
        agents.sort_by_key(|x| x.0);
        assert_eq!(agents, vec![a, b]);
    }

    #[test]
    fn metric_store_helper_write_json() {
        let store = MetricStore::in_memory();
        let uid = Uuid::from_bytes([7; 16]);
        store
            .write_json(uid, Utc::now(), json!({ "mem": 42 }))
            .unwrap();
        let sample = store.latest(AgentId(uid)).unwrap().unwrap();
        assert_eq!(sample.fields_json["mem"], 42);
    }

    #[test]
    fn metric_store_batch_write_json() {
        let store = MetricStore::in_memory();
        let uid = Uuid::from_bytes([9; 16]);
        let now = Utc::now();

        let written = store
            .write_json_batch([
                (uid, now, json!({ "cpu": 10 })),
                (uid, now + Duration::seconds(1), json!({ "cpu": 11 })),
            ])
            .unwrap();

        assert_eq!(written, 2);
        let series = store.query(AgentId(uid), QueryRange::Day1).unwrap();
        assert_eq!(series.samples.len(), 2);
    }

    #[derive(Default)]
    struct RecordingBackend {
        samples: RwLock<Vec<MetricSample>>,
    }

    impl MetricBackend for RecordingBackend {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn write(&self, sample: MetricSample) -> Result<(), MetricError> {
            self.samples.write().push(sample);
            Ok(())
        }

        fn query(
            &self,
            agent_id: AgentId,
            _range: QueryRange,
        ) -> Result<MetricSeries, MetricError> {
            let samples = self
                .samples
                .read()
                .iter()
                .filter(|sample| sample.agent_id == agent_id)
                .cloned()
                .collect();
            Ok(MetricSeries { agent_id, samples })
        }

        fn latest(&self, agent_id: AgentId) -> Result<Option<MetricSample>, MetricError> {
            Ok(self
                .samples
                .read()
                .iter()
                .rev()
                .find(|sample| sample.agent_id == agent_id)
                .cloned())
        }

        fn list_agents(&self) -> Result<Vec<AgentId>, MetricError> {
            let mut agents = self
                .samples
                .read()
                .iter()
                .map(|sample| sample.agent_id.clone())
                .collect::<Vec<_>>();
            agents.sort_by_key(|agent| agent.0);
            agents.dedup();
            Ok(agents)
        }

        fn health(&self) -> MetricBackendHealth {
            MetricBackendHealth::healthy(self.name()).with_samples(self.samples.read().len())
        }
    }

    #[test]
    fn metric_store_wraps_external_backend() {
        let store = MetricStore::from_backend(RecordingBackend::default());
        let agent = id(10);
        store
            .write(MetricSample {
                agent_id: agent.clone(),
                sample_at: Utc::now(),
                fields_json: json!({ "cpu": 50 }),
            })
            .unwrap();

        assert_eq!(store.backend_name(), "recording");
        assert_eq!(store.health().samples, Some(1));
        assert_eq!(store.list_agents().unwrap(), vec![agent]);
    }
}
