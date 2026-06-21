use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::settings::{
    public_server_details_enabled, public_site_branding, PublicSiteBranding,
};
use crate::api::v1::themes::{selected_public_theme, ThemeDefinition};
use crate::db::{AgentRepository, DatabaseBackend};
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, HeaderValue, StatusCode},
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
use std::time::Duration as StdDuration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;
use xlstatus_shared::AgentId;
use xlstatus_tsdb::QueryRange;

const ONLINE_THRESHOLD_SECS: i64 = 30;
const MJPEG_BOUNDARY: &str = "xlstatus-status";
const PUBLIC_MJPEG_MAX_CONNECTIONS: usize = 32;
const PUBLIC_METRIC_SAMPLE_LIMIT: usize = 240;

static PUBLIC_MJPEG_CONNECTIONS: once_cell::sync::Lazy<Arc<Semaphore>> =
    once_cell::sync::Lazy::new(|| Arc::new(Semaphore::new(PUBLIC_MJPEG_MAX_CONNECTIONS)));

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
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<PublicServerDetailView>>, AppError> {
    ensure_public_site_enabled(&state).await?;
    let include_server_details = public_server_details_enabled(&state.db).await?;
    let agent_id = parse_agent_id(&id)?;
    let agent_repo = AgentRepository::new(state.db.clone());
    let row = agent_repo
        .find_by_id_with_state(agent_id)
        .await?
        .ok_or(AppError::NotFound("server not found".to_string()))?;
    let dashboard = dashboard_metadata(row.dashboard_metadata_json.as_deref());
    if !visible_to_public(&dashboard) {
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
            let frame = match build_public_status_mjpeg_frame(&state).await {
                Ok(frame) => frame,
                Err(err) => {
                    tracing::warn!("failed to build public status MJPEG frame: {:?}", err);
                    build_mjpeg_frame(&fallback_status_jpeg("XLSTATUS", "ERROR", 0, 0, 0, 0))
                        .unwrap_or_default()
                }
            };
            Some((
                Ok::<Bytes, Infallible>(Bytes::from(frame)),
                (state, false, permit),
            ))
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

async fn build_public_status_mjpeg_frame(state: &AppState) -> Result<Vec<u8>, AppError> {
    let servers = public_servers(state).await?;
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
    let agent_repo = AgentRepository::new(state.db.clone());
    let (rows, _) = agent_repo.list_with_state(100, 0).await?;
    let include_server_details = public_server_details_enabled(&state.db).await?;

    let mut servers = Vec::new();
    for row in rows.into_iter() {
        let agent = row.agent;
        let dashboard = dashboard_metadata(row.dashboard_metadata_json.as_deref());
        if !visible_to_public(&dashboard) {
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
        });
    }
    Ok(servers)
}

async fn ensure_public_site_enabled(state: &AppState) -> Result<(), AppError> {
    if crate::api::v1::settings::public_site_enabled(&state.db).await? {
        Ok(())
    } else {
        Err(AppError::Forbidden("public status page is private".into()))
    }
}

async fn public_services(state: &AppState) -> Result<Vec<PublicServiceView>, AppError> {
    let public_server_ids = public_server_id_set(state).await?;
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT s.id, s.name, s.type, s.server_id,
                       r.status AS last_status, r.created_at AS last_check_at
                FROM services s
                LEFT JOIN service_results r ON r.id = (
                    SELECT sr.id FROM service_results sr
                    WHERE sr.service_id = s.id
                    ORDER BY sr.created_at DESC
                    LIMIT 1
                )
                WHERE s.enabled = 1
                ORDER BY s.created_at DESC
                LIMIT 100
                "#,
            )
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
                let history = filter_public_service_history(
                    public_service_history_sqlite(pool, &service_id).await?,
                    &public_server_ids,
                );
                services.push(PublicServiceView {
                    id: service_id,
                    name: row.get("name"),
                    service_type: service_type.clone(),
                    kind: service_type.clone(),
                    service_type_alias: service_type,
                    server_id: visible_server_ids.first().cloned(),
                    server_ids: visible_server_ids,
                    last_status: row.try_get("last_status").ok(),
                    last_check_at: row.try_get("last_check_at").ok(),
                    history,
                });
            }
            Ok(services)
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT s.id::text AS id, s.name, s.type, s.server_id::text AS server_id,
                       r.status AS last_status, r.created_at::text AS last_check_at
                FROM services s
                LEFT JOIN service_results r ON r.id = (
                    SELECT sr.id FROM service_results sr
                    WHERE sr.service_id = s.id
                    ORDER BY sr.created_at DESC
                    LIMIT 1
                )
                WHERE s.enabled = true
                ORDER BY s.created_at DESC
                LIMIT 100
                "#,
            )
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
                let history = filter_public_service_history(
                    public_service_history_postgres(pool, &service_id).await?,
                    &public_server_ids,
                );
                services.push(PublicServiceView {
                    id: service_id,
                    name: row.get("name"),
                    service_type: service_type.clone(),
                    kind: service_type.clone(),
                    service_type_alias: service_type,
                    server_id: visible_server_ids.first().cloned(),
                    server_ids: visible_server_ids,
                    last_status: row.try_get("last_status").ok(),
                    last_check_at: row.try_get("last_check_at").ok(),
                    history,
                });
            }
            Ok(services)
        }
    }
}

async fn public_server_id_set(state: &AppState) -> Result<HashSet<String>, AppError> {
    let agent_repo = AgentRepository::new(state.db.clone());
    let (rows, _) = agent_repo.list_with_state(100, 0).await?;
    let mut ids = HashSet::new();
    for row in rows {
        let dashboard = dashboard_metadata(row.dashboard_metadata_json.as_deref());
        if visible_to_public(&dashboard) {
            ids.insert(row.agent.id.0.to_string());
        }
    }
    Ok(ids)
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

fn filter_public_service_history(
    history: Vec<PublicServiceResultView>,
    public_server_ids: &HashSet<String>,
) -> Vec<PublicServiceResultView> {
    history
        .into_iter()
        .filter(|item| {
            item.server_id
                .as_ref()
                .map(|id| public_server_ids.contains(id))
                .unwrap_or(true)
        })
        .collect()
}

async fn public_service_history_sqlite(
    pool: &sqlx::SqlitePool,
    service_id: &str,
) -> Result<Vec<PublicServiceResultView>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT server_id, status, delay_ms, created_at
        FROM service_results
        WHERE service_id = ?
        ORDER BY created_at DESC
        LIMIT 1200
        "#,
    )
    .bind(service_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e.into()))?;

    Ok(rows
        .into_iter()
        .map(|row| PublicServiceResultView {
            server_id: row.try_get("server_id").ok(),
            status: row.try_get("status").unwrap_or_default(),
            delay_ms: row.try_get("delay_ms").ok(),
            created_at: row.try_get("created_at").unwrap_or_default(),
        })
        .collect())
}

async fn public_service_history_postgres(
    pool: &sqlx::PgPool,
    service_id: &str,
) -> Result<Vec<PublicServiceResultView>, AppError> {
    let sid = Uuid::parse_str(service_id)
        .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
    let rows = sqlx::query(
        r#"
        SELECT server_id::text AS server_id, status, delay_ms, created_at::text AS created_at
        FROM service_results
        WHERE service_id = $1
        ORDER BY created_at DESC
        LIMIT 1200
        "#,
    )
    .bind(sid)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e.into()))?;

    Ok(rows
        .into_iter()
        .map(|row| PublicServiceResultView {
            server_id: row.try_get("server_id").ok(),
            status: row.try_get("status").unwrap_or_default(),
            delay_ms: row.try_get("delay_ms").ok(),
            created_at: row.try_get("created_at").unwrap_or_default(),
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

fn parse_agent_id(id: &str) -> Result<AgentId, AppError> {
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
            "metrics",
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
