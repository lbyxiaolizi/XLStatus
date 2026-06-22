use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::settings::{
    public_server_details_enabled, public_site_branding, PublicSiteBranding,
};
use crate::api::v1::themes::{selected_public_theme, ThemeDefinition};
use crate::db::{Agent, AgentRepository, AgentWithState, DatabaseBackend};
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;
use xlstatus_shared::AgentId;
use xlstatus_tsdb::QueryRange;

const ONLINE_THRESHOLD_SECS: i64 = 30;
const MJPEG_BOUNDARY: &str = "xlstatus-status";
const PUBLIC_SITE_PRIVATE_MESSAGE: &str = "public status page is private";
const PUBLIC_MJPEG_MAX_CONNECTIONS: usize = 32;
const PUBLIC_MJPEG_FRAME_CACHE_TTL: StdDuration = StdDuration::from_secs(1);
const PUBLIC_METRIC_SAMPLE_LIMIT: usize = 240;
const PUBLIC_SERVICE_HISTORY_LIMIT: i64 = 240;
const PUBLIC_SERVER_ID_PATH_BYTES: usize = 36;
const PUBLIC_STATUS_SERVER_LIMIT: i64 = 100;
const PUBLIC_STATUS_SERVICE_LIMIT: i64 = 100;

static PUBLIC_MJPEG_CONNECTIONS: once_cell::sync::Lazy<Arc<Semaphore>> =
    once_cell::sync::Lazy::new(|| Arc::new(Semaphore::new(PUBLIC_MJPEG_MAX_CONNECTIONS)));
static PUBLIC_MJPEG_FRAME_CACHE: once_cell::sync::Lazy<Mutex<PublicMjpegFrameCache>> =
    once_cell::sync::Lazy::new(|| Mutex::new(PublicMjpegFrameCache::default()));

#[derive(Debug, Serialize)]
pub struct PublicStatusResponse {
    pub servers: Vec<PublicServerView>,
    pub services: Vec<PublicServiceView>,
    pub updated_at: String,
    pub site: PublicSiteBranding,
    pub theme: Option<ThemeDefinition>,
}

#[derive(Debug, Serialize)]
pub struct PublicServerView {
    pub id: String,
    pub name: String,
    pub remark: Option<String>,
    pub public_note: Option<String>,
    pub accent_color: Option<String>,
    pub status: String,
    pub last_seen_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<PublicServerResourcesView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<PublicServerMetricsView>,
}

#[derive(Debug, Serialize)]
pub struct PublicServerDetailView {
    pub id: String,
    pub name: String,
    pub remark: Option<String>,
    pub public_note: Option<String>,
    pub accent_color: Option<String>,
    pub status: String,
    pub last_seen_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<PublicServerResourcesView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<PublicServerMetricsView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicServerResourcesView {
    pub cpu_percent: Option<f64>,
    pub memory_used: Option<i64>,
    pub memory_total: Option<i64>,
    pub memory_percent: Option<f64>,
    pub disk_used: Option<i64>,
    pub disk_total: Option<i64>,
    pub disk_percent: Option<f64>,
    pub load_1: Option<f64>,
    pub net_rx_bps: Option<i64>,
    pub net_tx_bps: Option<i64>,
    pub network_in_total: Option<i64>,
    pub network_out_total: Option<i64>,
    pub uptime_seconds: Option<i64>,
    pub tcp_connections: Option<i64>,
    pub udp_connections: Option<i64>,
    pub process_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicServerMetricsView {
    pub range: String,
    pub samples: Vec<PublicMetricSampleView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicMetricSampleView {
    pub sample_at: String,
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
    pub disk_percent: Option<f64>,
    pub load_1: Option<f64>,
    pub net_rx_bps: Option<i64>,
    pub net_tx_bps: Option<i64>,
    pub network_in_total: Option<i64>,
    pub network_out_total: Option<i64>,
    pub tcp_connections: Option<i64>,
    pub udp_connections: Option<i64>,
    pub process_count: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PublicServiceView {
    pub id: String,
    pub name: String,
    pub service_type: String,
    pub kind: String,
    #[serde(rename = "type")]
    pub service_type_alias: String,
    pub server_id: Option<String>,
    pub server_ids: Vec<String>,
    pub last_status: Option<String>,
    pub last_check_at: Option<String>,
    pub history: Vec<PublicServiceResultView>,
}

#[derive(Debug, Serialize)]
pub struct PublicServiceResultView {
    pub server_id: Option<String>,
    pub status: String,
    pub delay_ms: Option<i32>,
    pub created_at: String,
}

pub async fn public_status(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<PublicStatusResponse>>, AppError> {
    ensure_public_site_enabled(&state).await?;
    let servers = public_servers(&state).await?;
    let services = public_services(&state).await?;
    Ok(Json(ApiResponse::success(PublicStatusResponse {
        servers,
        services,
        updated_at: Utc::now().to_rfc3339(),
        site: public_site_branding(&state.db).await?,
        theme: selected_public_theme(&state.db).await?,
    })))
}

pub async fn public_server_detail(
    State(state): State<AppState>,
    uri: Uri,
) -> Result<Json<ApiResponse<PublicServerDetailView>>, AppError> {
    ensure_public_site_enabled(&state).await?;
    let include_server_details = public_server_details_enabled(&state.db).await?;
    let agent_id = parse_public_server_id_path(&uri)?;
    let agent_repo = AgentRepository::new(state.db.clone());
    let row = agent_repo
        .find_by_id_with_state(agent_id)
        .await?
        .ok_or(AppError::NotFound("server not found".to_string()))?;
    let dashboard = dashboard_metadata(row.dashboard_metadata_json.as_deref());
    if !agent_visible_to_public(&row.agent, &dashboard) {
        return Err(AppError::NotFound("server not found".to_string()));
    }

    let agent = row.agent;
    let public_note = public_note_from_metadata(&dashboard);
    let parsed = row
        .last_state_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    let network_rates = network_rates_from_store(&state.metrics, agent.id.0);
    Ok(Json(ApiResponse::success(PublicServerDetailView {
        id: agent.id.0.to_string(),
        name: agent.name,
        remark: public_note.clone(),
        public_note,
        accent_color: dashboard.accent_color,
        status: server_status(agent.last_seen_at, agent.revoked_at).to_string(),
        last_seen_at: agent.last_seen_at.map(|t| t.to_rfc3339()),
        resources: include_server_details
            .then(|| public_resources_from_state(parsed.as_ref(), network_rates))
            .flatten(),
        metrics: if include_server_details {
            public_server_metrics(&state, agent.id.0)
        } else {
            None
        },
    })))
}

pub async fn public_status_mjpeg(State(state): State<AppState>) -> Result<Response, AppError> {
    ensure_public_site_enabled(&state).await?;
    let permit = acquire_public_mjpeg_permit()?;
    let stream =
        futures::stream::unfold((state, true, permit), |(state, first, permit)| async move {
            if !first {
                tokio::time::sleep(StdDuration::from_secs(5)).await;
            }
            let frame = match cached_public_status_mjpeg_frame(&state).await {
                Ok(frame) => frame,
                Err(err) if public_site_disabled_error(&err) => return None,
                Err(err) => {
                    tracing::warn!("failed to build public status MJPEG frame: {:?}", err);
                    Bytes::from(
                        build_mjpeg_frame(&fallback_status_jpeg("XLSTATUS", "ERROR", 0, 0, 0, 0))
                            .unwrap_or_default(),
                    )
                }
            };
            Some((Ok::<Bytes, Infallible>(frame), (state, false, permit)))
        })
        .map(|item| item);
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("multipart/x-mixed-replace; boundary=xlstatus-status"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store, no-cache, must-revalidate"),
            ),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

fn acquire_public_mjpeg_permit() -> Result<OwnedSemaphorePermit, AppError> {
    acquire_public_mjpeg_permit_from(PUBLIC_MJPEG_CONNECTIONS.clone())
}

fn acquire_public_mjpeg_permit_from(
    semaphore: Arc<Semaphore>,
) -> Result<OwnedSemaphorePermit, AppError> {
    semaphore
        .try_acquire_owned()
        .map_err(|_| AppError::TooManyRequests("too many public MJPEG streams".to_string()))
}

async fn cached_public_status_mjpeg_frame(state: &AppState) -> Result<Bytes, AppError> {
    cached_public_status_mjpeg_frame_from(state, &PUBLIC_MJPEG_FRAME_CACHE).await
}

async fn cached_public_status_mjpeg_frame_from(
    state: &AppState,
    cache: &Mutex<PublicMjpegFrameCache>,
) -> Result<Bytes, AppError> {
    let now = Instant::now();
    let mut cache = cache.lock().await;
    ensure_public_site_enabled(state).await?;
    if let Some(frame) = cache.get(now) {
        return Ok(frame);
    }

    let frame = Bytes::from(build_public_status_mjpeg_frame(state).await?);
    ensure_public_site_enabled(state).await?;
    Ok(cache.store(Instant::now(), frame))
}

#[derive(Default)]
struct PublicMjpegFrameCache {
    frame: Option<Bytes>,
    expires_at: Option<Instant>,
}

impl PublicMjpegFrameCache {
    fn get(&self, now: Instant) -> Option<Bytes> {
        match (self.frame.as_ref(), self.expires_at) {
            (Some(frame), Some(expires_at)) if expires_at > now => Some(frame.clone()),
            _ => None,
        }
    }

    fn store(&mut self, now: Instant, frame: Bytes) -> Bytes {
        self.expires_at = now.checked_add(PUBLIC_MJPEG_FRAME_CACHE_TTL);
        self.frame = Some(frame.clone());
        frame
    }
}

fn public_site_disabled_error(err: &AppError) -> bool {
    matches!(err, AppError::Forbidden(message) if message == PUBLIC_SITE_PRIVATE_MESSAGE)
}

async fn build_public_status_mjpeg_frame(state: &AppState) -> Result<Vec<u8>, AppError> {
    let servers = public_servers_summary(state).await?;
    let services = public_services(state).await?;
    let site = public_site_branding(&state.db).await?;
    let online_servers = servers
        .iter()
        .filter(|server| server.status == "online")
        .count();
    let failing_services = services
        .iter()
        .filter(|service| {
            matches!(
                service.last_status.as_deref(),
                Some("failure" | "down" | "failed")
            )
        })
        .count();
    let status = if servers.is_empty() && services.is_empty() {
        "NO DATA"
    } else if online_servers < servers.len() || failing_services > 0 {
        "DEGRADED"
    } else {
        "OK"
    };
    let jpeg = fallback_status_jpeg(
        &site.site_name,
        status,
        online_servers,
        servers.len(),
        services.len().saturating_sub(failing_services),
        services.len(),
    );
    build_mjpeg_frame(&jpeg)
}

async fn public_servers(state: &AppState) -> Result<Vec<PublicServerView>, AppError> {
    public_servers_with_metric_option(state, true).await
}

async fn public_servers_summary(state: &AppState) -> Result<Vec<PublicServerView>, AppError> {
    public_servers_with_metric_option(state, false).await
}

async fn public_servers_with_metric_option(
    state: &AppState,
    include_metrics_when_details_enabled: bool,
) -> Result<Vec<PublicServerView>, AppError> {
    let rows = public_agent_rows(state, PUBLIC_STATUS_SERVER_LIMIT).await?;
    let include_server_details = public_server_details_enabled(&state.db).await?;

    let mut servers = Vec::new();
    for row in rows.into_iter() {
        let agent = row.agent;
        let dashboard = dashboard_metadata(row.dashboard_metadata_json.as_deref());
        if !agent_visible_to_public(&agent, &dashboard) {
            continue;
        }
        let public_note = public_note_from_metadata(&dashboard);
        let parsed = row
            .last_state_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        let network_rates = network_rates_from_store(&state.metrics, agent.id.0);

        servers.push(PublicServerView {
            id: agent.id.0.to_string(),
            name: agent.name,
            remark: public_note.clone(),
            public_note,
            accent_color: dashboard.accent_color,
            status: server_status(agent.last_seen_at, agent.revoked_at).to_string(),
            last_seen_at: agent.last_seen_at.map(|t| t.to_rfc3339()),
            resources: include_server_details
                .then(|| public_resources_from_state(parsed.as_ref(), network_rates))
                .flatten(),
            metrics: if include_server_details && include_metrics_when_details_enabled {
                public_server_metrics(state, agent.id.0)
            } else {
                None
            },
        });
    }
    Ok(servers)
}

async fn ensure_public_site_enabled(state: &AppState) -> Result<(), AppError> {
    if crate::api::v1::settings::public_site_enabled(&state.db).await? {
        Ok(())
    } else {
        Err(AppError::Forbidden(PUBLIC_SITE_PRIVATE_MESSAGE.into()))
    }
}

async fn public_services(state: &AppState) -> Result<Vec<PublicServiceView>, AppError> {
    let public_server_ids = public_server_id_set(state).await?;
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT s.id, s.name, s.type, s.server_id
                FROM services s
                WHERE s.enabled = 1
                  AND (
                    EXISTS (
                      SELECT 1
                      FROM service_servers ss
                      JOIN agents a ON a.id = ss.server_id
                      WHERE ss.service_id = s.id
                        AND a.revoked_at IS NULL
                        AND json_valid(a.dashboard_metadata_json)
                        AND json_extract(a.dashboard_metadata_json, '$.dashboard_visible') = 1
                        AND COALESCE(json_extract(a.dashboard_metadata_json, '$.hide_for_guest'), 0) != 1
                    )
                    OR (
                      NOT EXISTS (SELECT 1 FROM service_servers ss WHERE ss.service_id = s.id)
                      AND EXISTS (
                        SELECT 1
                        FROM agents a
                        WHERE a.id = s.server_id
                          AND a.revoked_at IS NULL
                          AND json_valid(a.dashboard_metadata_json)
                          AND json_extract(a.dashboard_metadata_json, '$.dashboard_visible') = 1
                          AND COALESCE(json_extract(a.dashboard_metadata_json, '$.hide_for_guest'), 0) != 1
                      )
                    )
                  )
                ORDER BY s.created_at DESC
                LIMIT ?
                "#,
            )
            .bind(PUBLIC_STATUS_SERVICE_LIMIT)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.into()))?;

            let mut services = Vec::with_capacity(rows.len());
            for row in rows {
                let service_id: String = row.get("id");
                let service_type: String = row.get("type");
                let server_ids = public_service_server_ids(state, &service_id).await?;
                let legacy_server_id: Option<String> = row.try_get("server_id").ok();
                let visible_server_ids = visible_public_service_server_ids(
                    server_ids,
                    legacy_server_id,
                    &public_server_ids,
                );
                if visible_server_ids.is_empty() {
                    continue;
                }
                let history =
                    public_service_history_sqlite(pool, &service_id, &visible_server_ids).await?;
                let (last_status, last_check_at) = public_service_last_from_history(&history);
                services.push(PublicServiceView {
                    id: service_id,
                    name: row.get("name"),
                    service_type: service_type.clone(),
                    kind: service_type.clone(),
                    service_type_alias: service_type,
                    server_id: visible_server_ids.first().cloned(),
                    server_ids: visible_server_ids,
                    last_status,
                    last_check_at,
                    history,
                });
            }
            Ok(services)
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT s.id::text AS id, s.name, s.type, s.server_id::text AS server_id
                FROM services s
                WHERE s.enabled = true
                  AND (
                    EXISTS (
                      SELECT 1
                      FROM service_servers ss
                      JOIN agents a ON a.id = ss.server_id
                      WHERE ss.service_id = s.id
                        AND a.revoked_at IS NULL
                        AND (
                          a.dashboard_metadata_json LIKE '%"dashboard_visible":true%'
                          OR a.dashboard_metadata_json LIKE '%"dashboard_visible": true%'
                        )
                        AND a.dashboard_metadata_json NOT LIKE '%"hide_for_guest":true%'
                        AND a.dashboard_metadata_json NOT LIKE '%"hide_for_guest": true%'
                    )
                    OR (
                      NOT EXISTS (SELECT 1 FROM service_servers ss WHERE ss.service_id = s.id)
                      AND EXISTS (
                        SELECT 1
                        FROM agents a
                        WHERE a.id = s.server_id
                          AND a.revoked_at IS NULL
                          AND (
                            a.dashboard_metadata_json LIKE '%"dashboard_visible":true%'
                            OR a.dashboard_metadata_json LIKE '%"dashboard_visible": true%'
                          )
                          AND a.dashboard_metadata_json NOT LIKE '%"hide_for_guest":true%'
                          AND a.dashboard_metadata_json NOT LIKE '%"hide_for_guest": true%'
                      )
                    )
                  )
                ORDER BY s.created_at DESC
                LIMIT $1
                "#,
            )
            .bind(PUBLIC_STATUS_SERVICE_LIMIT)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.into()))?;

            let mut services = Vec::with_capacity(rows.len());
            for row in rows {
                let service_id: String = row.get("id");
                let service_type: String = row.get("type");
                let server_ids = public_service_server_ids(state, &service_id).await?;
                let legacy_server_id: Option<String> = row.try_get("server_id").ok();
                let visible_server_ids = visible_public_service_server_ids(
                    server_ids,
                    legacy_server_id,
                    &public_server_ids,
                );
                if visible_server_ids.is_empty() {
                    continue;
                }
                let history =
                    public_service_history_postgres(pool, &service_id, &visible_server_ids).await?;
                let (last_status, last_check_at) = public_service_last_from_history(&history);
                services.push(PublicServiceView {
                    id: service_id,
                    name: row.get("name"),
                    service_type: service_type.clone(),
                    kind: service_type.clone(),
                    service_type_alias: service_type,
                    server_id: visible_server_ids.first().cloned(),
                    server_ids: visible_server_ids,
                    last_status,
                    last_check_at,
                    history,
                });
            }
            Ok(services)
        }
    }
}

async fn public_server_id_set(state: &AppState) -> Result<HashSet<String>, AppError> {
    let rows = public_agent_rows(state, PUBLIC_STATUS_SERVER_LIMIT).await?;
    let mut ids = HashSet::new();
    for row in rows {
        let dashboard = dashboard_metadata(row.dashboard_metadata_json.as_deref());
        if visible_to_public(&dashboard) {
            ids.insert(row.agent.id.0.to_string());
        }
    }
    Ok(ids)
}

async fn public_agent_rows(state: &AppState, limit: i64) -> Result<Vec<AgentWithState>, AppError> {
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let rows: Vec<PublicAgentRow> = sqlx::query_as(
                r#"
                SELECT id, name, public_key, owner_user_id, last_seen_at, revoked_at,
                       created_at, updated_at, remark, expires_at, renewal_price,
                       dashboard_metadata_json, last_state_json, last_state_at,
                       last_info_json, last_info_at
                FROM agents
                WHERE revoked_at IS NULL
                  AND json_valid(dashboard_metadata_json)
                  AND json_extract(dashboard_metadata_json, '$.dashboard_visible') = 1
                  AND COALESCE(json_extract(dashboard_metadata_json, '$.hide_for_guest'), 0) != 1
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(limit)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.into()))?;
            Ok(public_agent_rows_to_state(rows))
        }
        DatabaseBackend::Postgres(pool) => {
            let rows: Vec<PublicAgentRow> = sqlx::query_as(
                r#"
                SELECT id::text, name, public_key, owner_user_id::text,
                       to_char(last_seen_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                       to_char(revoked_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                       to_char(created_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                       to_char(updated_at,  'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
                       remark, expires_at, renewal_price, dashboard_metadata_json,
                       last_state_json, last_state_at::text, last_info_json, last_info_at::text
                FROM agents
                WHERE revoked_at IS NULL
                  AND (
                    dashboard_metadata_json LIKE '%"dashboard_visible":true%'
                    OR dashboard_metadata_json LIKE '%"dashboard_visible": true%'
                  )
                  AND dashboard_metadata_json NOT LIKE '%"hide_for_guest":true%'
                  AND dashboard_metadata_json NOT LIKE '%"hide_for_guest": true%'
                ORDER BY created_at DESC
                LIMIT $1
                "#,
            )
            .bind(limit)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.into()))?;
            Ok(public_agent_rows_to_state(rows))
        }
    }
}

fn visible_public_service_server_ids(
    server_ids: Vec<String>,
    legacy_server_id: Option<String>,
    public_server_ids: &HashSet<String>,
) -> Vec<String> {
    let mut ids = if server_ids.is_empty() {
        legacy_server_id.into_iter().collect::<Vec<_>>()
    } else {
        server_ids
    };
    ids.retain(|id| public_server_ids.contains(id));
    ids.sort();
    ids.dedup();
    ids
}

type PublicAgentRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn public_agent_rows_to_state(rows: Vec<PublicAgentRow>) -> Vec<AgentWithState> {
    rows.into_iter()
        .filter_map(|row| {
            let agent_id = row.0.clone();
            match public_agent_row_to_state(row) {
                Ok(agent) => Some(agent),
                Err(err) => {
                    tracing::warn!(
                        agent_id = %agent_id,
                        "skipping invalid public agent row: {err}"
                    );
                    None
                }
            }
        })
        .collect()
}

fn public_agent_row_to_state(row: PublicAgentRow) -> Result<AgentWithState, anyhow::Error> {
    let (
        id,
        name,
        public_key,
        owner_user_id,
        last_seen_at,
        revoked_at,
        created_at,
        updated_at,
        remark,
        expires_at,
        renewal_price,
        dashboard_metadata_json,
        last_state_json,
        _last_state_at,
        last_info_json,
        _last_info_at,
    ) = row;
    let id = Uuid::parse_str(&id).map_err(|e| anyhow::anyhow!("invalid public agent id: {e}"))?;
    let owner_user_id = Uuid::parse_str(&owner_user_id)
        .map_err(|e| anyhow::anyhow!("invalid public agent owner: {e}"))?;
    Ok(AgentWithState {
        agent: Agent {
            id: AgentId(id),
            name,
            public_key,
            owner_user_id: xlstatus_shared::UserId(owner_user_id),
            last_seen_at: parse_optional_public_rfc3339(last_seen_at.as_deref(), "last_seen_at")?,
            revoked_at: parse_optional_public_rfc3339(revoked_at.as_deref(), "revoked_at")?,
            created_at: parse_public_rfc3339(&created_at, "created_at")?,
            updated_at: parse_public_rfc3339(&updated_at, "updated_at")?,
        },
        remark,
        expires_at,
        renewal_price,
        dashboard_metadata_json,
        last_state_json,
        last_info_json,
    })
}

fn parse_public_rfc3339(value: &str, field: &str) -> Result<DateTime<Utc>, anyhow::Error> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|e| anyhow::anyhow!("invalid public agent {field}: {e}"))
}

fn parse_optional_public_rfc3339(
    value: Option<&str>,
    field: &str,
) -> Result<Option<DateTime<Utc>>, anyhow::Error> {
    value
        .map(|value| parse_public_rfc3339(value, field))
        .transpose()
}

fn public_service_last_from_history(
    history: &[PublicServiceResultView],
) -> (Option<String>, Option<String>) {
    let Some(latest) = history.first() else {
        return (None, None);
    };
    (Some(latest.status.clone()), Some(latest.created_at.clone()))
}

async fn public_service_history_sqlite(
    pool: &sqlx::SqlitePool,
    service_id: &str,
    visible_server_ids: &[String],
) -> Result<Vec<PublicServiceResultView>, AppError> {
    if visible_server_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", visible_server_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        r#"
        SELECT server_id, status, delay_ms, created_at
        FROM service_results
        WHERE service_id = ? AND server_id IN ({placeholders})
        ORDER BY created_at DESC
        LIMIT ?
        "#
    );
    let mut query = sqlx::query(&sql).bind(service_id);
    for server_id in visible_server_ids {
        query = query.bind(server_id);
    }
    let rows = query
        .bind(PUBLIC_SERVICE_HISTORY_LIMIT)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.into()))?;

    let history = rows
        .into_iter()
        .filter_map(|row| {
            let server_id = row.try_get("server_id").ok();
            visible_server_ids
                .contains(server_id.as_ref()?)
                .then(|| PublicServiceResultView {
                    server_id,
                    status: row.try_get("status").unwrap_or_default(),
                    delay_ms: row.try_get("delay_ms").ok(),
                    created_at: row.try_get("created_at").unwrap_or_default(),
                })
        })
        .collect();
    Ok(history)
}

async fn public_service_history_postgres(
    pool: &sqlx::PgPool,
    service_id: &str,
    visible_server_ids: &[String],
) -> Result<Vec<PublicServiceResultView>, AppError> {
    if visible_server_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sid = Uuid::parse_str(service_id)
        .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
    let server_ids = visible_server_ids
        .iter()
        .map(|server_id| {
            Uuid::parse_str(server_id)
                .map_err(|e| AppError::BadRequest(format!("invalid server id: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rows = sqlx::query(
        r#"
        SELECT server_id::text AS server_id, status, delay_ms, created_at::text AS created_at
        FROM service_results
        WHERE service_id = $1 AND server_id = ANY($2)
        ORDER BY created_at DESC
        LIMIT $3
        "#,
    )
    .bind(sid)
    .bind(&server_ids)
    .bind(PUBLIC_SERVICE_HISTORY_LIMIT)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e.into()))?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let server_id = row.try_get("server_id").ok();
            visible_server_ids
                .contains(server_id.as_ref()?)
                .then(|| PublicServiceResultView {
                    server_id,
                    status: row.try_get("status").unwrap_or_default(),
                    delay_ms: row.try_get("delay_ms").ok(),
                    created_at: row.try_get("created_at").unwrap_or_default(),
                })
        })
        .collect())
}

async fn public_service_server_ids(
    state: &AppState,
    service_id: &str,
) -> Result<Vec<String>, AppError> {
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query("SELECT server_id FROM service_servers WHERE service_id = ?")
                .bind(service_id)
                .fetch_all(pool)
                .await
                .map_err(|e| AppError::Database(e.into()))?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("server_id").ok())
                .collect())
        }
        DatabaseBackend::Postgres(pool) => {
            let sid = Uuid::parse_str(service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let rows = sqlx::query(
                "SELECT server_id::text AS server_id FROM service_servers WHERE service_id = $1",
            )
            .bind(sid)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.into()))?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("server_id").ok())
                .collect())
        }
    }
}

fn build_mjpeg_frame(jpeg: &[u8]) -> Result<Vec<u8>, AppError> {
    let header = format!(
        "\r\n--{MJPEG_BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        jpeg.len()
    );
    let mut frame = Vec::with_capacity(header.len() + jpeg.len() + 2);
    frame.extend_from_slice(header.as_bytes());
    frame.extend_from_slice(jpeg);
    frame.extend_from_slice(b"\r\n");
    Ok(frame)
}

fn fallback_status_jpeg(
    site_name: &str,
    status: &str,
    online_servers: usize,
    total_servers: usize,
    ok_services: usize,
    total_services: usize,
) -> Vec<u8> {
    let width = 640_usize;
    let height = 180_usize;
    let mut pixels = vec![248_u8; width * height * 3];
    fill_rect(&mut pixels, width, 0, 0, width, height, [248, 250, 252]);
    fill_rect(&mut pixels, width, 0, 0, width, 16, status_color(status));
    fill_rect(&mut pixels, width, 26, 40, 588, 2, [15, 23, 42]);
    let title = ascii_label(site_name, "XLSTATUS", 22);
    draw_text(&mut pixels, width, 28, 56, &title, 3, [15, 23, 42]);
    draw_text(&mut pixels, width, 30, 112, status, 4, status_color(status));
    draw_text(
        &mut pixels,
        width,
        310,
        70,
        &format!("SERVERS {online_servers}/{total_servers}"),
        2,
        [15, 23, 42],
    );
    draw_text(
        &mut pixels,
        width,
        310,
        108,
        &format!("SERVICES {ok_services}/{total_services}"),
        2,
        [15, 23, 42],
    );
    draw_text(
        &mut pixels,
        width,
        310,
        146,
        &Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        1,
        [71, 85, 105],
    );

    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, 85);
    if encoder
        .encode(
            &pixels,
            width as u16,
            height as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .is_err()
    {
        return Vec::new();
    }
    out
}

fn status_color(status: &str) -> [u8; 3] {
    match status {
        "OK" => [22, 163, 74],
        "DEGRADED" => [234, 179, 8],
        "ERROR" => [220, 38, 38],
        _ => [100, 116, 139],
    }
}

fn ascii_label(value: &str, fallback: &str, max_chars: usize) -> String {
    let label: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '-' | '_'))
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_ascii_uppercase();
    if label.is_empty() {
        fallback.to_string()
    } else {
        label
    }
}

fn fill_rect(
    pixels: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: [u8; 3],
) {
    for yy in y..y.saturating_add(h) {
        for xx in x..x.saturating_add(w) {
            let idx = (yy * width + xx) * 3;
            if idx + 2 < pixels.len() {
                pixels[idx] = color[0];
                pixels[idx + 1] = color[1];
                pixels[idx + 2] = color[2];
            }
        }
    }
}

fn draw_text(
    pixels: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    text: &str,
    scale: usize,
    color: [u8; 3],
) {
    let mut cursor = x;
    for ch in text.chars() {
        draw_char(pixels, width, cursor, y, ch, scale, color);
        cursor += 6 * scale;
    }
}

fn draw_char(
    pixels: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    ch: char,
    scale: usize,
    color: [u8; 3],
) {
    let glyph = glyph(ch);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) != 0 {
                fill_rect(
                    pixels,
                    width,
                    x + col * scale,
                    y + row * scale,
                    scale,
                    scale,
                    color,
                );
            }
        }
    }
}

fn glyph(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        ':' => [
            0b00000, 0b00100, 0b00100, 0b00000, 0b00100, 0b00100, 0b00000,
        ],
        '/' => [
            0b00001, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b10000,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        '_' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111,
        ],
        ' ' => [0; 7],
        _ => [0; 7],
    }
}

fn parse_public_server_id_path(uri: &Uri) -> Result<AgentId, AppError> {
    let path = uri.path();
    let Some(id) = path.strip_prefix("/api/v1/public/servers/") else {
        return Err(AppError::BadRequest(
            "public server path is invalid".to_string(),
        ));
    };
    if id.is_empty() || id.contains('/') {
        return Err(AppError::BadRequest(
            "public server path is invalid".to_string(),
        ));
    }
    parse_public_server_id(id)
}

fn parse_public_server_id(id: &str) -> Result<AgentId, AppError> {
    if id.len() > PUBLIC_SERVER_ID_PATH_BYTES {
        return Err(AppError::BadRequest(format!(
            "public server id must be at most {PUBLIC_SERVER_ID_PATH_BYTES} bytes"
        )));
    }
    Uuid::parse_str(id)
        .map(AgentId)
        .map_err(|e| AppError::BadRequest(format!("invalid server id: {e}")))
}

fn server_status(
    last_seen_at: Option<chrono::DateTime<Utc>>,
    revoked_at: Option<chrono::DateTime<Utc>>,
) -> &'static str {
    if revoked_at.is_some() {
        return "revoked";
    }
    let last_seen_age = last_seen_at
        .map(|ts| (Utc::now() - ts).num_seconds())
        .unwrap_or(i64::MAX);
    if last_seen_age <= ONLINE_THRESHOLD_SECS {
        "online"
    } else {
        "offline"
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DashboardMetadata {
    public_note: Option<String>,
    accent_color: Option<String>,
    dashboard_visible: Option<bool>,
    hide_for_guest: Option<bool>,
}

fn visible_to_public(dashboard: &DashboardMetadata) -> bool {
    dashboard.dashboard_visible == Some(true) && dashboard.hide_for_guest != Some(true)
}

fn agent_visible_to_public(agent: &Agent, dashboard: &DashboardMetadata) -> bool {
    agent.revoked_at.is_none() && visible_to_public(dashboard)
}

fn public_note_from_metadata(dashboard: &DashboardMetadata) -> Option<String> {
    dashboard.public_note.clone()
}

fn public_resources_from_state(
    state: Option<&serde_json::Value>,
    network_rates: (Option<i64>, Option<i64>),
) -> Option<PublicServerResourcesView> {
    let state = state?;
    let memory_used = json_i64_by_keys(state, &["memory_used"]);
    let memory_total = json_i64_by_keys(state, &["memory_total"]);
    let (disk_used, disk_total) = disk_totals(state);
    let resources = PublicServerResourcesView {
        cpu_percent: json_f64_by_keys(state, &["cpu_percent"]),
        memory_used,
        memory_total,
        memory_percent: percent_from_used_total(memory_used, memory_total),
        disk_used,
        disk_total,
        disk_percent: json_f64_by_keys(state, &["disk_percent"])
            .or_else(|| percent_from_used_total(disk_used, disk_total)),
        load_1: json_f64_by_keys(state, &["load_1", "load1"]),
        net_rx_bps: json_i64_by_keys(state, &["net_rx_bps", "network_in_speed"])
            .or(network_rates.0),
        net_tx_bps: json_i64_by_keys(state, &["net_tx_bps", "network_out_speed"])
            .or(network_rates.1),
        network_in_total: network_total(state, "bytes_recv"),
        network_out_total: network_total(state, "bytes_sent"),
        uptime_seconds: json_i64_by_keys(state, &["uptime_seconds", "uptime"]),
        tcp_connections: json_i64_by_keys(
            state,
            &["tcp_connections", "tcp_conn_count", "tcp_count"],
        ),
        udp_connections: json_i64_by_keys(
            state,
            &["udp_connections", "udp_conn_count", "udp_count"],
        ),
        process_count: json_i64_by_keys(state, &["process_count", "processes", "processes_count"]),
    };
    public_resources_has_data(&resources).then_some(resources)
}

fn public_resources_has_data(resources: &PublicServerResourcesView) -> bool {
    resources.cpu_percent.is_some()
        || resources.memory_used.is_some()
        || resources.memory_total.is_some()
        || resources.disk_used.is_some()
        || resources.disk_total.is_some()
        || resources.load_1.is_some()
        || resources.net_rx_bps.is_some()
        || resources.net_tx_bps.is_some()
        || resources.network_in_total.is_some()
        || resources.network_out_total.is_some()
        || resources.uptime_seconds.is_some()
        || resources.tcp_connections.is_some()
        || resources.udp_connections.is_some()
        || resources.process_count.is_some()
}

fn public_server_metrics(state: &AppState, agent_id: Uuid) -> Option<PublicServerMetricsView> {
    let range = QueryRange::Day1;
    let series = state
        .metrics
        .query(xlstatus_tsdb::AgentId(agent_id), range)
        .ok()?;
    let samples = public_metric_samples(&series.samples);
    (!samples.is_empty()).then(|| PublicServerMetricsView {
        range: range.as_str().to_string(),
        samples,
    })
}

fn public_metric_samples(samples: &[xlstatus_tsdb::MetricSample]) -> Vec<PublicMetricSampleView> {
    let thinned = thin_metric_samples(samples, PUBLIC_METRIC_SAMPLE_LIMIT);
    let mut out = Vec::with_capacity(thinned.len());
    let mut previous: Option<(DateTime<Utc>, Option<i64>, Option<i64>)> = None;
    for sample in thinned {
        let fields = &sample.fields_json;
        let memory_used = json_i64_by_keys(fields, &["memory_used"]);
        let memory_total = json_i64_by_keys(fields, &["memory_total"]);
        let (_, disk_total) = disk_totals(fields);
        let (disk_used, _) = disk_totals(fields);
        let network_in_total = network_total(fields, "bytes_recv");
        let network_out_total = network_total(fields, "bytes_sent");
        let net_rx_bps =
            json_i64_by_keys(fields, &["net_rx_bps", "network_in_speed"]).or_else(|| {
                previous.and_then(|(at, rx, _)| {
                    rate_from_total_delta(sample.sample_at, network_in_total, at, rx)
                })
            });
        let net_tx_bps =
            json_i64_by_keys(fields, &["net_tx_bps", "network_out_speed"]).or_else(|| {
                previous.and_then(|(at, _, tx)| {
                    rate_from_total_delta(sample.sample_at, network_out_total, at, tx)
                })
            });
        let row = PublicMetricSampleView {
            sample_at: sample.sample_at.to_rfc3339(),
            cpu_percent: json_f64_by_keys(fields, &["cpu_percent"]),
            memory_percent: percent_from_used_total(memory_used, memory_total),
            disk_percent: json_f64_by_keys(fields, &["disk_percent"])
                .or_else(|| percent_from_used_total(disk_used, disk_total)),
            load_1: json_f64_by_keys(fields, &["load_1", "load1"]),
            net_rx_bps,
            net_tx_bps,
            network_in_total,
            network_out_total,
            tcp_connections: json_i64_by_keys(
                fields,
                &["tcp_connections", "tcp_conn_count", "tcp_count"],
            ),
            udp_connections: json_i64_by_keys(
                fields,
                &["udp_connections", "udp_conn_count", "udp_count"],
            ),
            process_count: json_i64_by_keys(
                fields,
                &["process_count", "processes", "processes_count"],
            ),
        };
        previous = Some((sample.sample_at, network_in_total, network_out_total));
        if public_metric_sample_has_data(&row) {
            out.push(row);
        }
    }
    out
}

fn public_metric_sample_has_data(sample: &PublicMetricSampleView) -> bool {
    sample.cpu_percent.is_some()
        || sample.memory_percent.is_some()
        || sample.disk_percent.is_some()
        || sample.load_1.is_some()
        || sample.net_rx_bps.is_some()
        || sample.net_tx_bps.is_some()
        || sample.network_in_total.is_some()
        || sample.network_out_total.is_some()
        || sample.tcp_connections.is_some()
        || sample.udp_connections.is_some()
        || sample.process_count.is_some()
}

fn thin_metric_samples(
    samples: &[xlstatus_tsdb::MetricSample],
    limit: usize,
) -> Vec<&xlstatus_tsdb::MetricSample> {
    if samples.len() <= limit || limit <= 1 {
        return samples.iter().collect();
    }
    let last_index = samples.len() - 1;
    let step = last_index as f64 / (limit - 1) as f64;
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(limit);
    for index in 0..limit {
        let sample_index = (index as f64 * step).round() as usize;
        if seen.insert(sample_index) {
            if let Some(sample) = samples.get(sample_index) {
                out.push(sample);
            }
        }
    }
    if seen.insert(last_index) {
        if let Some(sample) = samples.get(last_index) {
            out.push(sample);
        }
    }
    out
}

fn rate_from_total_delta(
    current_at: DateTime<Utc>,
    current_total: Option<i64>,
    previous_at: DateTime<Utc>,
    previous_total: Option<i64>,
) -> Option<i64> {
    let current_total = current_total?;
    let previous_total = previous_total?;
    let elapsed_ms = (current_at - previous_at).num_milliseconds();
    let delta = current_total.checked_sub(previous_total)?;
    if elapsed_ms <= 0 || delta < 0 {
        return None;
    }
    Some(((delta as f64) / (elapsed_ms as f64 / 1000.0)).round() as i64)
}

fn percent_from_used_total(used: Option<i64>, total: Option<i64>) -> Option<f64> {
    let used = used?;
    let total = total?;
    (total > 0).then(|| (used as f64 / total as f64) * 100.0)
}

fn disk_totals(value: &serde_json::Value) -> (Option<i64>, Option<i64>) {
    let direct_used = json_i64_by_keys(value, &["disk_used"]);
    let direct_total = json_i64_by_keys(value, &["disk_total"]);
    if direct_used.is_some() || direct_total.is_some() {
        return (direct_used, direct_total);
    }

    let Some(disks) = value.get("disks").and_then(|v| v.as_array()) else {
        return (None, None);
    };
    let mut used = 0_i64;
    let mut total = 0_i64;
    let mut found = false;
    for disk in disks {
        if let Some(value) = json_i64_by_keys(disk, &["used"]) {
            used = used.saturating_add(value);
            found = true;
        }
        if let Some(value) = json_i64_by_keys(disk, &["total"]) {
            total = total.saturating_add(value);
            found = true;
        }
    }
    found
        .then_some((Some(used), Some(total)))
        .unwrap_or((None, None))
}

fn network_rates_from_store(
    metrics: &xlstatus_tsdb::MetricStore,
    agent_id: Uuid,
) -> (Option<i64>, Option<i64>) {
    let Ok(series) = metrics.query(xlstatus_tsdb::AgentId(agent_id), QueryRange::Day1) else {
        return (None, None);
    };

    (
        network_rate_from_series(&series.samples, "bytes_recv"),
        network_rate_from_series(&series.samples, "bytes_sent"),
    )
}

fn network_rate_from_series(samples: &[xlstatus_tsdb::MetricSample], field: &str) -> Option<i64> {
    let mut latest: Option<(DateTime<Utc>, i64)> = None;
    for sample in samples.iter().rev() {
        let Some(total) = network_total(&sample.fields_json, field) else {
            continue;
        };
        if let Some((latest_at, latest_total)) = latest {
            return rate_from_total_delta(
                latest_at,
                Some(latest_total),
                sample.sample_at,
                Some(total),
            );
        }
        latest = Some((sample.sample_at, total));
    }
    None
}

fn network_total(value: &serde_json::Value, field: &str) -> Option<i64> {
    let direct_keys = match field {
        "bytes_recv" => &["network_in_total", "net_rx_bytes", "bytes_recv_total"][..],
        "bytes_sent" => &["network_out_total", "net_tx_bytes", "bytes_sent_total"][..],
        _ => &[][..],
    };
    if let Some(value) = json_i64_by_keys(value, direct_keys) {
        return Some(value);
    }

    let net_io = value
        .get("net_io")
        .or_else(|| value.get("network_interfaces"))
        .and_then(|v| v.as_array())?;
    let mut total = 0_i64;
    let mut found = false;
    for item in net_io {
        if let Some(value) = item.get(field).and_then(json_i64) {
            total = total.saturating_add(value);
            found = true;
        }
    }
    found.then_some(total)
}

fn json_i64_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(json_i64) {
            return Some(value);
        }
    }
    None
}

fn json_i64(value: &serde_json::Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    if let Some(value) = value.as_u64() {
        return i64::try_from(value).ok();
    }
    if let Some(value) = value.as_f64() {
        if value.is_finite() && value >= 0.0 && value <= i64::MAX as f64 {
            return Some(value.round() as i64);
        }
    }
    None
}

fn json_f64_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(json_f64) {
            return Some(value);
        }
    }
    None
}

fn json_f64(value: &serde_json::Value) -> Option<f64> {
    if let Some(value) = value.as_f64() {
        return value.is_finite().then_some(value);
    }
    value
        .as_i64()
        .map(|value| value as f64)
        .or_else(|| value.as_u64().map(|value| value as f64))
}

fn dashboard_metadata(stored: Option<&str>) -> DashboardMetadata {
    let mut out = stored
        .and_then(|value| serde_json::from_str::<DashboardMetadata>(value).ok())
        .unwrap_or_default();

    out.public_note = out
        .public_note
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    out.accent_color = out.accent_color.and_then(normalize_accent_color);
    out
}

fn normalize_accent_color(value: String) -> Option<String> {
    let value = value.trim();
    let is_hex = value.len() == 7
        && value.starts_with('#')
        && value.chars().skip(1).all(|ch| ch.is_ascii_hexdigit());
    is_hex.then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_visibility_requires_explicit_opt_in() {
        let mut metadata = DashboardMetadata::default();
        assert!(!visible_to_public(&metadata));

        metadata.dashboard_visible = Some(true);
        assert!(visible_to_public(&metadata));

        metadata.hide_for_guest = Some(true);
        assert!(!visible_to_public(&metadata));
    }

    #[test]
    fn public_server_detail_path_bounds_id_before_uuid_parse() {
        assert_eq!(PUBLIC_SERVER_ID_PATH_BYTES, 36);

        let id = Uuid::now_v7();
        let uri: Uri = format!("/api/v1/public/servers/{id}").parse().unwrap();
        assert_eq!(parse_public_server_id_path(&uri).unwrap().0, id);

        let uri: Uri = format!(
            "/api/v1/public/servers/{}",
            "a".repeat(PUBLIC_SERVER_ID_PATH_BYTES + 1)
        )
        .parse()
        .unwrap();
        let err = parse_public_server_id_path(&uri).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(message) if message.contains("at most")));

        let uri: Uri = format!("/api/v1/public/servers/{id}/metrics")
            .parse()
            .unwrap();
        let err = parse_public_server_id_path(&uri).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(message) if message.contains("invalid")));
    }

    #[test]
    fn public_server_view_uses_minimal_allowlist() {
        let serialized = serde_json::to_value(PublicServerView {
            id: Uuid::nil().to_string(),
            name: "public".into(),
            remark: Some("note".into()),
            public_note: Some("note".into()),
            accent_color: Some("#db2777".into()),
            status: "online".into(),
            last_seen_at: None,
            resources: None,
            metrics: None,
        })
        .expect("public server view serializes");

        for hidden_field in [
            "provider",
            "region",
            "plan",
            "tags",
            "location",
            "cpu_percent",
            "memory_used",
            "network_in_total",
            "last_state",
            "last_info",
        ] {
            assert!(
                serialized.get(hidden_field).is_none(),
                "{hidden_field} must not be serialized in public server view"
            );
        }
    }

    #[test]
    fn public_resources_use_aggregated_allowlist() {
        let state = serde_json::json!({
            "cpu_percent": 42.5,
            "memory_used": 1024,
            "memory_total": 4096,
            "load_1": 0.8,
            "uptime_seconds": 3600,
            "tcp_connections": 12,
            "udp_connections": 4,
            "process_count": 88,
            "disks": [
                { "mount_point": "/", "used": 1000, "total": 4000 },
                { "mount_point": "/data", "used": 2000, "total": 6000 }
            ],
            "net_io": [
                { "interface": "eth0", "bytes_recv": 1000, "bytes_sent": 2000 }
            ]
        });

        let resources = public_resources_from_state(Some(&state), (Some(50), Some(25)))
            .expect("resource summary");
        assert_eq!(resources.cpu_percent, Some(42.5));
        assert_eq!(resources.memory_percent, Some(25.0));
        assert_eq!(resources.disk_used, Some(3000));
        assert_eq!(resources.disk_total, Some(10000));
        assert_eq!(resources.disk_percent, Some(30.0));
        assert_eq!(resources.net_rx_bps, Some(50));
        assert_eq!(resources.net_tx_bps, Some(25));

        let serialized = serde_json::to_value(resources).expect("resources serialize");
        assert!(serialized.get("mount_point").is_none());
        assert!(serialized.get("interface").is_none());
        assert!(serialized.get("net_io").is_none());
        assert!(serialized.get("disks").is_none());
    }

    #[test]
    fn public_metric_samples_derive_network_rates_and_hide_raw_json() {
        let agent_id = xlstatus_tsdb::AgentId(Uuid::nil());
        let now = Utc::now();
        let samples = public_metric_samples(&[
            xlstatus_tsdb::MetricSample {
                agent_id: agent_id.clone(),
                sample_at: now - chrono::Duration::seconds(10),
                fields_json: serde_json::json!({
                    "cpu_percent": 10,
                    "memory_used": 1000,
                    "memory_total": 2000,
                    "network_in_total": 1000,
                    "network_out_total": 3000
                }),
            },
            xlstatus_tsdb::MetricSample {
                agent_id,
                sample_at: now,
                fields_json: serde_json::json!({
                    "cpu_percent": 20,
                    "memory_used": 1500,
                    "memory_total": 2000,
                    "network_in_total": 6000,
                    "network_out_total": 8000
                }),
            },
        ]);

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[1].net_rx_bps, Some(500));
        assert_eq!(samples[1].net_tx_bps, Some(500));

        let serialized = serde_json::to_value(&samples[1]).expect("metric serializes");
        assert!(serialized.get("fields_json").is_none());
        assert!(serialized.get("agent_id").is_none());
    }

    #[test]
    fn public_service_last_status_uses_first_public_history_row() {
        let public_server = Uuid::now_v7().to_string();
        let history = vec![PublicServiceResultView {
            server_id: Some(public_server),
            status: "success".into(),
            delay_ms: Some(20),
            created_at: "2026-06-22T10:00:00Z".into(),
        }];

        let (last_status, last_check_at) = public_service_last_from_history(&history);
        assert_eq!(last_status.as_deref(), Some("success"));
        assert_eq!(last_check_at.as_deref(), Some("2026-06-22T10:00:00Z"));
    }

    #[tokio::test]
    async fn public_service_history_query_filters_servers_and_bounds_rows() {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        let service_id = Uuid::now_v7().to_string();
        let public_server = Uuid::now_v7().to_string();
        let private_server = Uuid::now_v7().to_string();

        sqlx::query(
            "INSERT INTO services (id, name, type, target, interval_seconds, timeout_seconds, enabled, created_at, updated_at) VALUES (?, 'svc', 'http', 'https://example.com', 60, 10, 1, '2026-06-22T00:00:00Z', '2026-06-22T00:00:00Z')",
        )
        .bind(&service_id)
        .execute(&pool)
        .await
        .unwrap();

        for idx in 0..300 {
            sqlx::query(
                "INSERT INTO service_results (id, service_id, server_id, status, delay_ms, created_at) VALUES (?, ?, ?, 'success', 20, ?)",
            )
            .bind(Uuid::now_v7().to_string())
            .bind(&service_id)
            .bind(&public_server)
            .bind(format!("2026-06-22T00:{:02}:{:02}Z", idx / 60, idx % 60))
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO service_results (id, service_id, server_id, status, delay_ms, created_at) VALUES (?, ?, ?, 'failure', 900, ?)",
            )
            .bind(Uuid::now_v7().to_string())
            .bind(&service_id)
            .bind(&private_server)
            .bind(format!("2026-06-22T01:{:02}:{:02}Z", idx / 60, idx % 60))
            .execute(&pool)
            .await
            .unwrap();
        }

        let history = public_service_history_sqlite(&pool, &service_id, &[public_server.clone()])
            .await
            .unwrap();

        assert_eq!(history.len(), PUBLIC_SERVICE_HISTORY_LIMIT as usize);
        assert!(history
            .iter()
            .all(|item| item.server_id.as_deref() == Some(public_server.as_str())));
        assert!(history.iter().all(|item| item.status == "success"));
    }

    #[tokio::test]
    async fn public_servers_filter_visibility_before_limit() {
        let state = test_state_with_public_site(true).await;
        let DatabaseBackend::Sqlite(pool) = &state.db else {
            unreachable!();
        };
        let owner = Uuid::now_v7().to_string();
        seed_public_user(pool, &owner).await;
        let public_server = Uuid::now_v7().to_string();
        seed_public_agent(
            pool,
            &public_server,
            &owner,
            "public-server",
            true,
            false,
            "2026-01-01T00:00:00Z",
        )
        .await;
        for idx in 0..120 {
            seed_public_agent(
                pool,
                &Uuid::now_v7().to_string(),
                &owner,
                &format!("private-{idx}"),
                false,
                false,
                &format!("2026-01-02T00:{:02}:{:02}Z", idx / 60, idx % 60),
            )
            .await;
        }

        let servers = public_servers(&state).await.unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].id, public_server);
    }

    #[tokio::test]
    async fn public_servers_skip_invalid_historical_agent_rows() {
        let state = test_state_with_public_site(true).await;
        let DatabaseBackend::Sqlite(pool) = &state.db else {
            unreachable!();
        };
        let owner = Uuid::now_v7().to_string();
        seed_public_user(pool, &owner).await;
        let public_server = Uuid::now_v7().to_string();
        seed_public_agent(
            pool,
            &public_server,
            &owner,
            "public-server",
            true,
            false,
            "2026-01-01T00:00:00Z",
        )
        .await;
        seed_public_agent(
            pool,
            &Uuid::now_v7().to_string(),
            &owner,
            "dirty-public-server",
            true,
            false,
            "not-a-timestamp",
        )
        .await;

        let servers = public_servers(&state).await.unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].id, public_server);
    }

    #[tokio::test]
    async fn public_server_detail_rejects_revoked_public_agent() {
        let state = test_state_with_public_site(true).await;
        let DatabaseBackend::Sqlite(pool) = &state.db else {
            unreachable!();
        };
        let owner = Uuid::now_v7().to_string();
        seed_public_user(pool, &owner).await;
        let server_id = Uuid::now_v7().to_string();
        seed_public_agent(
            pool,
            &server_id,
            &owner,
            "revoked-public-server",
            true,
            false,
            "2026-01-01T00:00:00Z",
        )
        .await;
        sqlx::query(
            "UPDATE agents SET revoked_at = '2026-06-22T00:00:00Z', last_state_json = ? WHERE id = ?",
        )
        .bind(
            serde_json::json!({
                "cpu_percent": 91.0,
                "memory_used": 1024,
                "memory_total": 2048
            })
            .to_string(),
        )
        .bind(&server_id)
        .execute(pool)
        .await
        .unwrap();

        let uri: Uri = format!("/api/v1/public/servers/{server_id}")
            .parse()
            .unwrap();
        let err = match public_server_detail(axum::extract::State(state), uri).await {
            Ok(_) => panic!("revoked public agent detail must be hidden"),
            Err(err) => err,
        };

        assert!(matches!(err, AppError::NotFound(message) if message == "server not found"));
    }

    #[tokio::test]
    async fn public_status_defaults_to_anonymous_with_server_details() {
        let state = test_state_without_public_settings().await;
        let DatabaseBackend::Sqlite(pool) = &state.db else {
            unreachable!();
        };
        let owner = Uuid::now_v7().to_string();
        let server_id = Uuid::now_v7().to_string();
        seed_public_user(pool, &owner).await;
        seed_public_agent(
            pool,
            &server_id,
            &owner,
            "public-server",
            true,
            false,
            "2026-01-01T00:00:00Z",
        )
        .await;
        sqlx::query(
            "UPDATE agents SET last_state_json = ?, last_state_at = '2026-06-22T00:00:00Z' WHERE id = ?",
        )
        .bind(
            serde_json::json!({
                "cpu_percent": 42.0,
                "memory_used": 1024,
                "memory_total": 2048,
                "net_rx_bps": 128,
                "net_tx_bps": 64
            })
            .to_string(),
        )
        .bind(&server_id)
        .execute(pool)
        .await
        .unwrap();
        state
            .metrics
            .write(xlstatus_tsdb::MetricSample {
                agent_id: xlstatus_tsdb::AgentId(Uuid::parse_str(&server_id).unwrap()),
                sample_at: "2026-06-22T00:00:00Z".parse::<DateTime<Utc>>().unwrap(),
                fields_json: serde_json::json!({
                    "cpu_percent": 38.0,
                    "memory_used": 1536,
                    "memory_total": 2048,
                    "network_in_total": 1000,
                    "network_out_total": 2000
                }),
            })
            .unwrap();
        state
            .metrics
            .write(xlstatus_tsdb::MetricSample {
                agent_id: xlstatus_tsdb::AgentId(Uuid::parse_str(&server_id).unwrap()),
                sample_at: "2026-06-22T00:01:00Z".parse::<DateTime<Utc>>().unwrap(),
                fields_json: serde_json::json!({
                    "cpu_percent": 42.0,
                    "memory_used": 1024,
                    "memory_total": 2048,
                    "network_in_total": 7000,
                    "network_out_total": 5000
                }),
            })
            .unwrap();

        let response = public_status(axum::extract::State(state)).await.unwrap();
        let data = response.0.data.expect("public status response data");

        assert_eq!(data.servers.len(), 1);
        let resources = data.servers[0]
            .resources
            .as_ref()
            .expect("public server details default to visible");
        assert_eq!(resources.cpu_percent, Some(42.0));
        assert_eq!(resources.memory_percent, Some(50.0));
        assert_eq!(resources.net_rx_bps, Some(128));
        assert_eq!(resources.net_tx_bps, Some(64));
        let metrics = data.servers[0]
            .metrics
            .as_ref()
            .expect("public server metrics default to visible");
        assert_eq!(metrics.range, QueryRange::Day1.as_str());
        assert_eq!(metrics.samples.len(), 2);
        assert_eq!(metrics.samples[1].cpu_percent, Some(42.0));
        assert_eq!(metrics.samples[1].net_rx_bps, Some(100));
    }

    #[tokio::test]
    async fn public_status_hides_server_details_when_setting_is_disabled() {
        let state = test_state_with_public_site(true).await;
        let DatabaseBackend::Sqlite(pool) = &state.db else {
            unreachable!();
        };
        disable_public_server_details(pool).await;
        let owner = Uuid::now_v7().to_string();
        let server_id = Uuid::now_v7().to_string();
        seed_public_user(pool, &owner).await;
        seed_public_agent(
            pool,
            &server_id,
            &owner,
            "public-server",
            true,
            false,
            "2026-01-01T00:00:00Z",
        )
        .await;
        sqlx::query(
            "UPDATE agents SET last_state_json = ?, last_state_at = '2026-06-22T00:00:00Z' WHERE id = ?",
        )
        .bind(serde_json::json!({ "cpu_percent": 42.0 }).to_string())
        .bind(&server_id)
        .execute(pool)
        .await
        .unwrap();

        let response = public_status(axum::extract::State(state)).await.unwrap();
        let data = response.0.data.expect("public status response data");

        assert_eq!(data.servers.len(), 1);
        assert!(data.servers[0].resources.is_none());
        assert!(data.servers[0].metrics.is_none());
    }

    #[tokio::test]
    async fn public_services_filter_public_server_scope_before_limit() {
        let state = test_state_with_public_site(true).await;
        let DatabaseBackend::Sqlite(pool) = &state.db else {
            unreachable!();
        };
        let owner = Uuid::now_v7().to_string();
        seed_public_user(pool, &owner).await;
        let public_server = Uuid::now_v7().to_string();
        let private_server = Uuid::now_v7().to_string();
        seed_public_agent(
            pool,
            &public_server,
            &owner,
            "public-server",
            true,
            false,
            "2026-01-01T00:00:00Z",
        )
        .await;
        seed_public_agent(
            pool,
            &private_server,
            &owner,
            "private-server",
            false,
            false,
            "2026-01-02T00:00:00Z",
        )
        .await;
        let public_service = Uuid::now_v7().to_string();
        seed_public_service(
            pool,
            &public_service,
            &owner,
            "public-service",
            &public_server,
            "2026-01-01T00:00:00Z",
        )
        .await;
        for idx in 0..120 {
            seed_public_service(
                pool,
                &Uuid::now_v7().to_string(),
                &owner,
                &format!("private-service-{idx}"),
                &private_server,
                &format!("2026-01-02T00:{:02}:{:02}Z", idx / 60, idx % 60),
            )
            .await;
        }

        let services = public_services(&state).await.unwrap();

        assert_eq!(services.len(), 1);
        assert_eq!(services[0].id, public_service);
        assert_eq!(services[0].server_ids, vec![public_server]);
    }

    async fn test_state_without_public_settings() -> AppState {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        app_state_from_db(db)
    }

    async fn test_state_with_public_site(enabled: bool) -> AppState {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let DatabaseBackend::Sqlite(pool) = &db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO system_settings (key, value_json, updated_at) VALUES ('public_site_enabled', ?, '2026-06-22T00:00:00Z')",
        )
        .bind(if enabled { "true" } else { "false" })
        .execute(pool)
        .await
        .unwrap();

        app_state_from_db(db)
    }

    fn app_state_from_db(db: DatabaseBackend) -> AppState {
        AppState {
            db,
            config: Arc::new(crate::config::Config::default()),
            agent_jwt_challenges: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            metrics: xlstatus_tsdb::MetricStore::in_memory(),
            realtime: crate::realtime::BroadcastHub::new(),
            session_registry: crate::grpc::SessionRegistry::new(),
            terminal_sessions: crate::api::v1::terminal::TerminalSessionRegistry::new(),
            io_registry: crate::grpc::IoRegistry::new(),
        }
    }

    async fn disable_public_server_details(pool: &sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO system_settings (key, value_json, updated_at) VALUES ('public_server_details_enabled', 'false', '2026-06-22T00:00:00Z')",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_public_user(pool: &sqlx::SqlitePool, user_id: &str) {
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'hash', 'admin', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(user_id)
        .bind(format!("user-{user_id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_public_agent(
        pool: &sqlx::SqlitePool,
        agent_id: &str,
        owner_id: &str,
        name: &str,
        dashboard_visible: bool,
        hide_for_guest: bool,
        created_at: &str,
    ) {
        let metadata = serde_json::json!({
            "dashboard_visible": dashboard_visible,
            "hide_for_guest": hide_for_guest,
            "public_note": format!("{name} note"),
        })
        .to_string();
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at, dashboard_metadata_json) VALUES (?, ?, 'pk', ?, ?, ?, ?)",
        )
        .bind(agent_id)
        .bind(name)
        .bind(owner_id)
        .bind(created_at)
        .bind(created_at)
        .bind(metadata)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_public_service(
        pool: &sqlx::SqlitePool,
        service_id: &str,
        owner_id: &str,
        name: &str,
        server_id: &str,
        created_at: &str,
    ) {
        sqlx::query(
            "INSERT INTO services (id, owner_user_id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, cover_mode, created_at, updated_at) VALUES (?, ?, ?, 'http', 'https://example.com', 60, 10, 1, ?, 'specific', ?, ?)",
        )
        .bind(service_id)
        .bind(owner_id)
        .bind(name)
        .bind(server_id)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO service_servers (service_id, server_id, created_at) VALUES (?, ?, ?)",
        )
        .bind(service_id)
        .bind(server_id)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn public_mjpeg_cache_hit_reuses_frame_when_site_is_enabled() {
        let state = test_state_with_public_site(true).await;
        let cache = Mutex::new(PublicMjpegFrameCache {
            frame: Some(Bytes::from_static(b"cached-frame")),
            expires_at: Some(Instant::now() + PUBLIC_MJPEG_FRAME_CACHE_TTL),
        });

        let frame = cached_public_status_mjpeg_frame_from(&state, &cache)
            .await
            .unwrap();

        assert_eq!(&frame[..], b"cached-frame");
    }

    #[tokio::test]
    async fn public_mjpeg_cache_is_not_served_after_site_is_disabled() {
        let state = test_state_with_public_site(false).await;
        let cache = Mutex::new(PublicMjpegFrameCache {
            frame: Some(Bytes::from_static(b"cached-frame")),
            expires_at: Some(Instant::now() + PUBLIC_MJPEG_FRAME_CACHE_TTL),
        });

        let err = cached_public_status_mjpeg_frame_from(&state, &cache)
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            AppError::Forbidden(message) if message == PUBLIC_SITE_PRIVATE_MESSAGE
        ));
    }

    #[test]
    fn public_mjpeg_connection_limit_returns_429_error() {
        let semaphore = Arc::new(Semaphore::new(2));
        let first = acquire_public_mjpeg_permit_from(semaphore.clone()).unwrap();
        let second = acquire_public_mjpeg_permit_from(semaphore.clone()).unwrap();

        assert!(matches!(
            acquire_public_mjpeg_permit_from(semaphore.clone()),
            Err(AppError::TooManyRequests(_))
        ));

        drop(first);
        assert!(acquire_public_mjpeg_permit_from(semaphore).is_ok());
        drop(second);
    }
}
