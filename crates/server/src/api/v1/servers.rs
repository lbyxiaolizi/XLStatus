//! M3 server listing + metrics endpoints.
//!
//! `/api/v1/servers` returns the rows in the `agents` table, plus
//! the latest `last_state_json` parsed into a flat object. The status
//! is derived from `last_seen_at` (within 30 s = online, otherwise
//! offline). `/api/v1/servers/:id/metrics` returns the MetricStore
//! time series for one agent.
//!
//! Both routes are protected by the standard session middleware and
//! scope-validated through `auth/rbac.rs`. The PAT path requires
//! `server:read` and a matching `server_id` in the PAT allowlist.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::AgentRepository;

pub use crate::realtime::ws::ws_servers;
use axum::{
    extract::{Path, Query, State},
    Json,
};
#[cfg(test)]
use chrono::Duration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use xlstatus_shared::AgentId;
use xlstatus_tsdb::{MetricSeries, QueryRange};

const ONLINE_THRESHOLD_SECS: i64 = 30;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Debug, Serialize)]
pub struct ServerView {
    pub id: String,
    pub name: String,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub cpu_percent: Option<f64>,
    pub memory_used: Option<i64>,
    pub memory_total: Option<i64>,
    pub load_1: Option<f64>,
    pub net_rx_bps: Option<i64>,
    pub net_tx_bps: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ListServersResponse {
    pub servers: Vec<ServerView>,
    pub total: i64,
}

pub async fn list_servers(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(q): Query<ListQuery>,
) -> Result<Json<ApiResponse<ListServersResponse>>, AppError> {
    if !has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);

    let agent_repo = AgentRepository::new(state.db.clone());
    let (rows, total) = agent_repo.list_with_state(limit, offset).await?;
    let now = Utc::now();
    let mut servers = Vec::with_capacity(rows.len());
    for row in rows.into_iter() {
        if !server_visible(&auth, &row.agent.id) {
            continue;
        }
        let agent = row.agent;
        let last_seen_age = agent
            .last_seen_at
            .map(|ts| (now - ts).num_seconds())
            .unwrap_or(i64::MAX);
        let status = if agent.revoked_at.is_some() {
            "revoked"
        } else if last_seen_age <= ONLINE_THRESHOLD_SECS {
            "online"
        } else {
            "offline"
        };
        let parsed = row
            .last_state_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        servers.push(ServerView {
            id: agent.id.0.to_string(),
            name: agent.name,
            status: status.to_string(),
            last_seen_at: agent.last_seen_at.map(|t| t.to_rfc3339()),
            cpu_percent: parsed
                .as_ref()
                .and_then(|v| v.get("cpu_percent"))
                .and_then(|v| v.as_f64()),
            memory_used: parsed
                .as_ref()
                .and_then(|v| v.get("memory_used"))
                .and_then(|v| v.as_i64()),
            memory_total: parsed
                .as_ref()
                .and_then(|v| v.get("memory_total"))
                .and_then(|v| v.as_i64()),
            load_1: parsed
                .as_ref()
                .and_then(|v| v.get("load_1"))
                .and_then(|v| v.as_f64()),
            net_rx_bps: None,
            net_tx_bps: None,
        });
    }
    Ok(Json(ApiResponse::success(ListServersResponse {
        servers,
        total,
    })))
}

#[derive(Debug, Serialize)]
pub struct ServerDetailResponse {
    pub id: String,
    pub name: String,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub last_state: Option<serde_json::Value>,
    pub last_info: Option<serde_json::Value>,
}

pub async fn get_server(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ServerDetailResponse>>, AppError> {
    if !has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    let agent_id = parse_agent_id(&id)?;
    if !server_visible(&auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let agent_repo = AgentRepository::new(state.db.clone());
    let row = agent_repo
        .find_by_id_with_state(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".to_string()))?;
    let agent = row.agent;
    let now = Utc::now();
    let last_seen_age = agent
        .last_seen_at
        .map(|ts| (now - ts).num_seconds())
        .unwrap_or(i64::MAX);
    let status = if agent.revoked_at.is_some() {
        "revoked"
    } else if last_seen_age <= ONLINE_THRESHOLD_SECS {
        "online"
    } else {
        "offline"
    };
    let last_state = row
        .last_state_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    let last_info = row
        .last_info_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    Ok(Json(ApiResponse::success(ServerDetailResponse {
        id: agent.id.0.to_string(),
        name: agent.name,
        status: status.to_string(),
        last_seen_at: agent.last_seen_at.map(|t| t.to_rfc3339()),
        last_state,
        last_info,
    })))
}

#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "1d".to_string()
}

#[derive(Debug, Serialize)]
pub struct MetricsResponse {
    pub agent_id: String,
    pub range: String,
    pub series: MetricSeries,
}

pub async fn get_server_metrics(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Query(q): Query<MetricsQuery>,
) -> Result<Json<ApiResponse<MetricsResponse>>, AppError> {
    if !has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    let agent_id = parse_agent_id(&id)?;
    if !server_visible(&auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let range = QueryRange::parse(&q.range).ok_or(AppError::BadRequest(format!(
        "unsupported range: {}",
        q.range
    )))?;
    let series = state
        .metrics
        .query(xlstatus_tsdb::AgentId(agent_id.0), range)?;
    Ok(Json(ApiResponse::success(MetricsResponse {
        agent_id: agent_id.0.to_string(),
        range: range.as_str().to_string(),
        series,
    })))
}

fn parse_agent_id(id: &str) -> Result<AgentId, AppError> {
    let parsed = uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::BadRequest(format!("invalid agent id: {id}")))?;
    Ok(AgentId(parsed))
}

/// Helper used by tests and the public docs: build a ServerView from
/// (agent, parsed_state). Pulled out of the route so the offline/online
/// decision is unit-testable in isolation.
pub fn build_server_view(
    agent_id: AgentId,
    name: String,
    last_seen_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    last_state: Option<&serde_json::Value>,
) -> ServerView {
    let now = Utc::now();
    let age = last_seen_at
        .map(|ts| (now - ts).num_seconds())
        .unwrap_or(i64::MAX);
    let status = if revoked_at.is_some() {
        "revoked"
    } else if age <= ONLINE_THRESHOLD_SECS {
        "online"
    } else {
        "offline"
    };
    ServerView {
        id: agent_id.0.to_string(),
        name,
        status: status.to_string(),
        last_seen_at: last_seen_at.map(|t| t.to_rfc3339()),
        cpu_percent: last_state
            .and_then(|v| v.get("cpu_percent"))
            .and_then(|v| v.as_f64()),
        memory_used: last_state
            .and_then(|v| v.get("memory_used"))
            .and_then(|v| v.as_i64()),
        memory_total: last_state
            .and_then(|v| v.get("memory_total"))
            .and_then(|v| v.as_i64()),
        load_1: last_state
            .and_then(|v| v.get("load_1"))
            .and_then(|v| v.as_f64()),
        net_rx_bps: None,
        net_tx_bps: None,
    }
}

/// Per-agent visibility check that respects both admin (always visible)
/// and PAT (must be in the PAT's `server_ids` allowlist). Lives next
/// to the route so the API contract is local and unit-testable.
pub fn server_visible(auth: &AuthSession, agent_id: &AgentId) -> bool {
    match &auth.server_ids {
        None => true,
        Some(allow) => {
            let id_str = agent_id.0.to_string();
            allow.iter().any(|a| a == &id_str)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use xlstatus_shared::AgentId;

    #[test]
    fn online_when_fresh() {
        let view = build_server_view(
            AgentId(uuid::Uuid::from_bytes([1; 16])),
            "n1".into(),
            Some(Utc::now()),
            None,
            None,
        );
        assert_eq!(view.status, "online");
    }

    #[test]
    fn offline_when_stale() {
        let view = build_server_view(
            AgentId(uuid::Uuid::from_bytes([2; 16])),
            "n2".into(),
            Some(Utc::now() - Duration::seconds(120)),
            None,
            None,
        );
        assert_eq!(view.status, "offline");
    }

    #[test]
    fn revoked_overrides_anything() {
        let view = build_server_view(
            AgentId(uuid::Uuid::from_bytes([3; 16])),
            "n3".into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
        );
        assert_eq!(view.status, "revoked");
    }

    #[test]
    fn extracts_metrics_from_state() {
        let state = json!({
            "cpu_percent": 42.0,
            "memory_used": 1000,
            "memory_total": 2000,
            "load_1": 0.5,
        });
        let view = build_server_view(
            AgentId(uuid::Uuid::from_bytes([4; 16])),
            "n4".into(),
            Some(Utc::now()),
            None,
            Some(&state),
        );
        assert_eq!(view.cpu_percent, Some(42.0));
        assert_eq!(view.memory_used, Some(1000));
        assert_eq!(view.memory_total, Some(2000));
        assert_eq!(view.load_1, Some(0.5));
    }
}
