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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

pub const MAX_SAMPLE_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_SAMPLE_JSON_DEPTH: usize = 16;
pub const MAX_BATCH_SAMPLES: usize = 256_000;
pub const MAX_SAMPLES_PER_AGENT: usize = 50_000;
pub const MAX_RETENTION_DAYS: i64 = 3650;
pub const MAX_FUTURE_SKEW_SECONDS: i64 = 86_400;

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

    fn export_samples(&self) -> Result<Vec<MetricSample>, MetricError> {
        Err(MetricError::Backend(
            "metric backend does not support sample export".into(),
        ))
    }

    fn compact(&self) -> Result<usize, MetricError> {
        Ok(0)
    }

    fn retention(&self) -> Option<Duration> {
        None
    }

    fn set_retention(&self, _retention: Duration) -> Result<(), MetricError> {
        Err(MetricError::Backend(
            "metric backend does not support dynamic retention".into(),
        ))
    }

    fn health(&self) -> MetricBackendHealth {
        MetricBackendHealth::healthy(self.name())
    }
}

/// Thread-safe, bounded in-memory store. Retention defaults to 30 days.
/// Writes compact only the affected agents; call `compact` to reclaim
/// expired data belonging to inactive agents.
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

fn normalize_retention(retention: Duration) -> Duration {
    retention.clamp(
        Duration::milliseconds(1),
        Duration::days(MAX_RETENTION_DAYS),
    )
}

fn cutoff_for_retention(now: DateTime<Utc>, retention: Duration) -> DateTime<Utc> {
    let clamped = retention.max(Duration::zero());
    now.checked_sub_signed(clamped)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .max(DateTime::<Utc>::UNIX_EPOCH)
}

fn validate_sample_payload(value: &serde_json::Value) -> Result<usize, MetricError> {
    fn add_bytes(accumulated: &mut usize, n: usize) -> Result<(), MetricError> {
        *accumulated = accumulated.saturating_add(n);
        if *accumulated > MAX_SAMPLE_PAYLOAD_BYTES {
            Err(MetricError::Backend(format!(
                "sample payload size exceeds maximum of {} bytes",
                MAX_SAMPLE_PAYLOAD_BYTES
            )))
        } else {
            Ok(())
        }
    }

    fn walk(
        value: &serde_json::Value,
        depth: usize,
        accumulated: &mut usize,
    ) -> Result<(), MetricError> {
        if depth > MAX_SAMPLE_JSON_DEPTH {
            return Err(MetricError::Backend(format!(
                "sample payload JSON depth exceeds maximum of {}",
                MAX_SAMPLE_JSON_DEPTH
            )));
        }

        match value {
            serde_json::Value::Null => add_bytes(accumulated, 4),
            serde_json::Value::Bool(b) => add_bytes(accumulated, if *b { 4 } else { 5 }),
            serde_json::Value::Number(num) => {
                if let Some(f) = num.as_f64() {
                    if !f.is_finite() {
                        return Err(MetricError::Backend(
                            "sample payload contains non-finite number".into(),
                        ));
                    }
                }
                add_bytes(accumulated, 24)
            }
            serde_json::Value::String(s) => add_bytes(accumulated, s.len().saturating_add(2)),
            serde_json::Value::Array(items) => {
                add_bytes(accumulated, 2)?;
                for item in items {
                    add_bytes(accumulated, 1)?;
                    walk(item, depth + 1, accumulated)?;
                }
                Ok(())
            }
            serde_json::Value::Object(map) => {
                add_bytes(accumulated, 2)?;
                for (k, v) in map {
                    add_bytes(accumulated, k.len().saturating_add(4))?;
                    walk(v, depth + 1, accumulated)?;
                }
                Ok(())
            }
        }
    }

    let mut total = 0usize;
    walk(value, 0, &mut total)?;
    Ok(total)
}

fn validate_sample(sample: &MetricSample, now: DateTime<Utc>) -> Result<(), MetricError> {
    if sample.sample_at.timestamp() < 0 {
        return Err(MetricError::InvalidTimestamp(sample.sample_at.to_rfc3339()));
    }
    let max_allowed = now
        .checked_add_signed(Duration::seconds(MAX_FUTURE_SKEW_SECONDS))
        .unwrap_or(DateTime::<Utc>::MAX_UTC);
    if sample.sample_at > max_allowed {
        return Err(MetricError::InvalidTimestamp(sample.sample_at.to_rfc3339()));
    }
    validate_sample_payload(&sample.fields_json)?;
    Ok(())
}

impl InMemoryMetricStore {
    pub fn new() -> Self {
        Self::with_retention(QueryRange::Day30.to_chrono_duration())
    }

    pub fn with_retention(retention: Duration) -> Self {
        Self {
            inner: RwLock::new(InMemoryState {
                series: HashMap::new(),
                retention: normalize_retention(retention),
            }),
        }
    }

    fn compact_series(
        entries: &mut BTreeMap<DateTime<Utc>, serde_json::Value>,
        cutoff: DateTime<Utc>,
    ) -> usize {
        // BTreeMap is ordered by key, so split_off is O(log n) plus
        // the size of the dropped prefix.
        let before = entries.len();
        *entries = entries.split_off(&cutoff);
        if entries.len() > MAX_SAMPLES_PER_AGENT {
            let excess = entries.len() - MAX_SAMPLES_PER_AGENT;
            for _ in 0..excess {
                let _ = entries.pop_first();
            }
        }
        before.saturating_sub(entries.len())
    }

    fn compact_agent(
        series: &mut HashMap<AgentId, BTreeMap<DateTime<Utc>, serde_json::Value>>,
        agent_id: &AgentId,
        cutoff: DateTime<Utc>,
    ) -> usize {
        let Some(entries) = series.get_mut(agent_id) else {
            return 0;
        };

        let dropped = Self::compact_series(entries, cutoff);
        if entries.is_empty() {
            series.remove(agent_id);
        }
        dropped
    }

    fn compact_all(state: &mut InMemoryState, cutoff: DateTime<Utc>) -> usize {
        let mut dropped: usize = 0;
        state.series.retain(|_, entries| {
            dropped = dropped.saturating_add(Self::compact_series(entries, cutoff));
            !entries.is_empty()
        });
        dropped
    }

    /// Drop samples outside retention for every agent and remove empty
    /// series. Writes use the narrower per-agent helper instead.
    pub fn compact(&self) -> usize {
        let mut state = self.inner.write();
        let cutoff = cutoff_for_retention(Utc::now(), state.retention);
        Self::compact_all(&mut state, cutoff)
    }

    pub fn retention(&self) -> Duration {
        self.inner.read().retention
    }

    pub fn set_retention(&self, retention: Duration) -> usize {
        let retention = normalize_retention(retention);
        let mut state = self.inner.write();
        state.retention = retention;
        let cutoff = cutoff_for_retention(Utc::now(), retention);
        Self::compact_all(&mut state, cutoff)
    }

    /// Total number of samples currently held. Useful in tests and
    /// admin debug endpoints.
    pub fn len(&self) -> usize {
        self.inner.read().series.values().map(|m| m.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn export_samples(&self) -> Vec<MetricSample> {
        let state = self.inner.read();
        let cutoff = cutoff_for_retention(Utc::now(), state.retention);
        let mut samples = Vec::new();
        for (agent_id, entries) in &state.series {
            samples.extend(
                entries
                    .range(cutoff..)
                    .map(|(sample_at, fields_json)| MetricSample {
                        agent_id: agent_id.clone(),
                        sample_at: *sample_at,
                        fields_json: fields_json.clone(),
                    }),
            );
        }
        samples.sort_by_key(|sample| (sample.agent_id.0, sample.sample_at));
        samples
    }
}

impl MetricBackend for InMemoryMetricStore {
    fn name(&self) -> &'static str {
        "in_memory"
    }

    fn write(&self, sample: MetricSample) -> Result<(), MetricError> {
        let now = Utc::now();
        validate_sample(&sample, now)?;
        let agent_id = sample.agent_id.clone();
        let ts = sample.sample_at;
        let payload = sample.fields_json.clone();
        let mut state = self.inner.write();
        let cutoff = cutoff_for_retention(now, state.retention);
        state
            .series
            .entry(agent_id.clone())
            .or_default()
            .insert(ts, payload);
        Self::compact_agent(&mut state.series, &agent_id, cutoff);
        Ok(())
    }

    fn write_batch(&self, samples: Vec<MetricSample>) -> Result<usize, MetricError> {
        if samples.len() > MAX_BATCH_SAMPLES {
            return Err(MetricError::Backend(format!(
                "batch size {} exceeds maximum of {}",
                samples.len(),
                MAX_BATCH_SAMPLES
            )));
        }
        let now = Utc::now();
        for sample in &samples {
            validate_sample(sample, now)?;
        }

        let count = samples.len();
        let mut state = self.inner.write();
        let cutoff = cutoff_for_retention(now, state.retention);
        let mut affected_agents = HashSet::with_capacity(count);
        for sample in samples {
            affected_agents.insert(sample.agent_id.clone());
            state
                .series
                .entry(sample.agent_id)
                .or_default()
                .insert(sample.sample_at, sample.fields_json);
        }
        for agent_id in affected_agents {
            Self::compact_agent(&mut state.series, &agent_id, cutoff);
        }
        Ok(count)
    }

    fn query(&self, agent_id: AgentId, range: QueryRange) -> Result<MetricSeries, MetricError> {
        let state = self.inner.read();
        let now = Utc::now();
        let range_cutoff = cutoff_for_retention(now, range.to_chrono_duration());
        let retention_cutoff = cutoff_for_retention(now, state.retention);
        let cutoff = std::cmp::max(range_cutoff, retention_cutoff);
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
        let cutoff = cutoff_for_retention(Utc::now(), state.retention);
        Ok(state
            .series
            .get(&agent_id)
            .and_then(|entries| entries.iter().next_back())
            .filter(|(ts, _)| **ts >= cutoff)
            .map(|(ts, payload)| MetricSample {
                agent_id: agent_id.clone(),
                sample_at: *ts,
                fields_json: payload.clone(),
            }))
    }

    fn list_agents(&self) -> Result<Vec<AgentId>, MetricError> {
        let state = self.inner.read();
        let cutoff = cutoff_for_retention(Utc::now(), state.retention);
        Ok(state
            .series
            .iter()
            .filter(|(_, entries)| {
                entries
                    .last_key_value()
                    .is_some_and(|(sample_at, _)| *sample_at >= cutoff)
            })
            .map(|(agent_id, _)| agent_id.clone())
            .collect())
    }

    fn export_samples(&self) -> Result<Vec<MetricSample>, MetricError> {
        Ok(InMemoryMetricStore::export_samples(self))
    }

    fn compact(&self) -> Result<usize, MetricError> {
        Ok(InMemoryMetricStore::compact(self))
    }

    fn retention(&self) -> Option<Duration> {
        Some(InMemoryMetricStore::retention(self))
    }

    fn set_retention(&self, retention: Duration) -> Result<(), MetricError> {
        if retention <= Duration::zero() {
            return Err(MetricError::Backend(
                "retention must be greater than zero".into(),
            ));
        }
        if retention > Duration::days(MAX_RETENTION_DAYS) {
            return Err(MetricError::Backend(format!(
                "retention must not exceed {} days",
                MAX_RETENTION_DAYS
            )));
        }
        InMemoryMetricStore::set_retention(self, retention);
        Ok(())
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

    pub fn export_samples(&self) -> Result<Vec<MetricSample>, MetricError> {
        self.inner.export_samples()
    }

    pub fn compact(&self) -> Result<usize, MetricError> {
        self.inner.compact()
    }

    pub fn retention(&self) -> Option<Duration> {
        self.inner.retention()
    }

    pub fn set_retention(&self, retention: Duration) -> Result<(), MetricError> {
        self.inner.set_retention(retention)
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

    fn seed_uncompacted(store: &InMemoryMetricStore, agent_id: AgentId, sample_at: DateTime<Utc>) {
        store
            .inner
            .write()
            .series
            .entry(agent_id)
            .or_default()
            .insert(sample_at, json!({ "seeded": true }));
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
    fn write_only_compacts_the_touched_agent() {
        let store = InMemoryMetricStore::with_retention(Duration::hours(1));
        let touched = id(11);
        let untouched = id(12);
        let now = Utc::now();
        let expired = now - Duration::hours(2);
        seed_uncompacted(&store, touched.clone(), expired);
        seed_uncompacted(&store, untouched.clone(), expired);

        store
            .write(MetricSample {
                agent_id: touched.clone(),
                sample_at: now,
                fields_json: json!({ "cpu": 3.0 }),
            })
            .unwrap();

        let state = store.inner.read();
        assert_eq!(state.series[&touched].len(), 1);
        assert_eq!(state.series[&untouched].len(), 1);
        drop(state);

        assert!(store.latest(untouched.clone()).unwrap().is_none());
        assert_eq!(store.list_agents().unwrap(), vec![touched.clone()]);
        assert!(store
            .query(untouched.clone(), QueryRange::Day30)
            .unwrap()
            .samples
            .is_empty());
        assert!(store
            .export_samples()
            .into_iter()
            .all(|sample| sample.agent_id == touched));

        assert_eq!(store.compact(), 1);
        assert!(!store.inner.read().series.contains_key(&untouched));
    }

    #[test]
    fn batch_write_compacts_each_affected_agent_once() {
        let store = InMemoryMetricStore::with_retention(Duration::hours(1));
        let first = id(13);
        let second = id(14);
        let untouched = id(15);
        let now = Utc::now();
        let expired = now - Duration::hours(2);
        for agent_id in [&first, &second, &untouched] {
            seed_uncompacted(&store, agent_id.clone(), expired);
        }

        store
            .write_batch(vec![
                MetricSample {
                    agent_id: first.clone(),
                    sample_at: now,
                    fields_json: json!({ "cpu": 1.0 }),
                },
                MetricSample {
                    agent_id: first.clone(),
                    sample_at: now + Duration::seconds(1),
                    fields_json: json!({ "cpu": 2.0 }),
                },
                MetricSample {
                    agent_id: second.clone(),
                    sample_at: now,
                    fields_json: json!({ "cpu": 3.0 }),
                },
            ])
            .unwrap();

        let state = store.inner.read();
        assert_eq!(state.series[&first].len(), 2);
        assert_eq!(state.series[&second].len(), 1);
        assert_eq!(state.series[&untouched].len(), 1);
        drop(state);

        let mut agents = store.list_agents().unwrap();
        agents.sort_by_key(|agent| agent.0);
        assert_eq!(agents, vec![first, second]);
        assert_eq!(store.compact(), 1);
        assert!(!store.inner.read().series.contains_key(&untouched));
    }

    #[test]
    fn retention_can_be_changed_at_runtime() {
        let store = InMemoryMetricStore::with_retention(Duration::days(30));
        let agent = id(10);
        store
            .write_batch(vec![
                MetricSample {
                    agent_id: agent.clone(),
                    sample_at: Utc::now() - Duration::days(2),
                    fields_json: json!({ "cpu": 1.0 }),
                },
                MetricSample {
                    agent_id: agent,
                    sample_at: Utc::now(),
                    fields_json: json!({ "cpu": 2.0 }),
                },
            ])
            .unwrap();

        assert_eq!(store.len(), 2);
        assert_eq!(store.set_retention(Duration::days(1)), 1);
        assert_eq!(store.retention(), Duration::days(1));
        assert_eq!(store.len(), 1);
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

    #[test]
    fn rejects_oversized_and_deeply_nested_sample_payloads() {
        let store = InMemoryMetricStore::new();
        let agent = id(21);
        let now = Utc::now();

        let oversized_str = "x".repeat(MAX_SAMPLE_PAYLOAD_BYTES + 128);
        let err = store
            .write(MetricSample {
                agent_id: agent.clone(),
                sample_at: now,
                fields_json: json!({ "blob": oversized_str }),
            })
            .unwrap_err();
        assert!(err.to_string().contains("exceeds maximum"));

        let mut nested = json!(1);
        for _ in 0..=MAX_SAMPLE_JSON_DEPTH {
            nested = json!([nested]);
        }
        let err = store
            .write(MetricSample {
                agent_id: agent,
                sample_at: now,
                fields_json: nested,
            })
            .unwrap_err();
        assert!(err.to_string().contains("depth exceeds maximum"));
    }

    #[test]
    fn rejects_pre_epoch_and_far_future_timestamps() {
        let store = InMemoryMetricStore::new();
        let agent = id(22);
        let now = Utc::now();

        let pre_epoch = DateTime::<Utc>::from_timestamp(-10, 0).unwrap();
        assert!(matches!(
            store.write(MetricSample {
                agent_id: agent.clone(),
                sample_at: pre_epoch,
                fields_json: json!({ "cpu": 1.0 }),
            }),
            Err(MetricError::InvalidTimestamp(_))
        ));

        let far_future = now + Duration::days(30);
        assert!(matches!(
            store.write(MetricSample {
                agent_id: agent,
                sample_at: far_future,
                fields_json: json!({ "cpu": 1.0 }),
            }),
            Err(MetricError::InvalidTimestamp(_))
        ));
    }

    #[test]
    fn extreme_retention_values_do_not_overflow_or_panic() {
        let store = InMemoryMetricStore::with_retention(Duration::MAX);
        let agent = id(23);
        let now = Utc::now();

        store
            .write(MetricSample {
                agent_id: agent.clone(),
                sample_at: now,
                fields_json: json!({ "cpu": 10.0 }),
            })
            .unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.compact(), 0);

        // Negative/zero retention via InMemoryMetricStore::set_retention is safely clamped
        store.set_retention(Duration::MIN);
        assert!(store.retention() >= Duration::milliseconds(1));

        // MetricBackend::set_retention rejects non-positive or > MAX_RETENTION_DAYS
        let facade = MetricStore::in_memory();
        assert!(facade.set_retention(Duration::zero()).is_err());
        assert!(facade
            .set_retention(Duration::days(MAX_RETENTION_DAYS + 1))
            .is_err());
    }
}
