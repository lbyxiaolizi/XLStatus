use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::settings::{public_site_branding, PublicSiteBranding};
use crate::api::v1::themes::{selected_public_theme, ThemeDefinition};
use crate::db::{AgentRepository, DatabaseBackend};
use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::convert::Infallible;
use std::time::Duration as StdDuration;
use uuid::Uuid;
use xlstatus_shared::AgentId;
use xlstatus_tsdb::{MetricSample, MetricSeries, QueryRange};

const ONLINE_THRESHOLD_SECS: i64 = 30;
const MJPEG_BOUNDARY: &str = "xlstatus-status";

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
    pub expires_at: Option<String>,
    pub renewal_price: Option<String>,
    pub price: Option<String>,
    pub currency: Option<String>,
    pub billing_cycle: Option<String>,
    pub auto_renew: Option<bool>,
    pub traffic_quota_bytes: Option<i64>,
    pub traffic_quota_type: Option<String>,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub plan: Option<String>,
    pub tags: Vec<String>,
    pub accent_color: Option<String>,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub cpu_percent: Option<f64>,
    pub memory_used: Option<i64>,
    pub memory_total: Option<i64>,
    pub load_1: Option<f64>,
    pub net_rx_bps: Option<i64>,
    pub net_tx_bps: Option<i64>,
    pub network_in_total: Option<i64>,
    pub network_out_total: Option<i64>,
    pub uptime_seconds: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PublicServerDetailView {
    pub id: String,
    pub name: String,
    pub remark: Option<String>,
    pub public_note: Option<String>,
    pub expires_at: Option<String>,
    pub renewal_price: Option<String>,
    pub price: Option<String>,
    pub currency: Option<String>,
    pub billing_cycle: Option<String>,
    pub auto_renew: Option<bool>,
    pub traffic_quota_bytes: Option<i64>,
    pub traffic_quota_type: Option<String>,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub plan: Option<String>,
    pub tags: Vec<String>,
    pub accent_color: Option<String>,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub last_state: Option<serde_json::Value>,
    pub last_info: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct PublicMetricsQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "1d".to_string()
}

#[derive(Debug, Serialize)]
pub struct PublicMetricsResponse {
    pub agent_id: String,
    pub range: String,
    pub series: MetricSeries,
}

#[derive(Debug, Serialize)]
pub struct PublicServiceView {
    pub id: String,
    pub name: String,
    pub service_type: String,
    pub kind: String,
    #[serde(rename = "type")]
    pub service_type_alias: String,
    pub target: String,
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
    let agent_id = parse_agent_id(&id)?;
    let agent_repo = AgentRepository::new(state.db.clone());
    let row = agent_repo
        .find_by_id_with_state(agent_id)
        .await?
        .ok_or(AppError::NotFound("server not found".to_string()))?;
    let last_state = parse_json(row.last_state_json.as_deref());
    let last_info = parse_json(row.last_info_json.as_deref());
    let dashboard = dashboard_metadata(
        row.dashboard_metadata_json.as_deref(),
        &[last_info.as_ref(), last_state.as_ref()],
    );
    if hidden_from_public(&dashboard) {
        return Err(AppError::NotFound("server not found".to_string()));
    }
    let public_last_state = sanitize_public_json(last_state.clone());
    let public_last_info = sanitize_public_json(last_info.clone());

    let agent = row.agent;
    let public_note =
        public_note_from_metadata(&dashboard, &[last_info.as_ref(), last_state.as_ref()]);
    Ok(Json(ApiResponse::success(PublicServerDetailView {
        id: agent.id.0.to_string(),
        name: agent.name,
        remark: public_note.clone(),
        public_note,
        expires_at: row.expires_at.or_else(|| {
            metadata_string(
                &[last_info.as_ref(), last_state.as_ref()],
                &["expires_at", "expired_at", "expire_at", "due_at", "end_at"],
            )
        }),
        renewal_price: row.renewal_price.or_else(|| {
            metadata_string(
                &[last_info.as_ref(), last_state.as_ref()],
                &[
                    "renewal_price",
                    "renew_price",
                    "renewal",
                    "price",
                    "billing_price",
                ],
            )
        }),
        price: dashboard.price,
        currency: dashboard.currency,
        billing_cycle: dashboard.billing_cycle,
        auto_renew: dashboard.auto_renew,
        traffic_quota_bytes: dashboard.traffic_quota_bytes,
        traffic_quota_type: dashboard.traffic_quota_type,
        provider: dashboard.provider,
        region: dashboard.region,
        plan: dashboard.plan,
        tags: dashboard.tags,
        accent_color: dashboard.accent_color,
        status: server_status(agent.last_seen_at, agent.revoked_at).to_string(),
        last_seen_at: agent.last_seen_at.map(|t| t.to_rfc3339()),
        last_state: public_last_state,
        last_info: public_last_info,
    })))
}

pub async fn public_server_metrics(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PublicMetricsQuery>,
) -> Result<Json<ApiResponse<PublicMetricsResponse>>, AppError> {
    ensure_public_site_enabled(&state).await?;
    let agent_id = parse_agent_id(&id)?;
    ensure_public_server_visible(&state, agent_id).await?;
    let range = QueryRange::parse(&q.range).ok_or(AppError::BadRequest(format!(
        "unsupported range: {}",
        q.range
    )))?;
    let series = sanitize_public_metric_series(
        state
            .metrics
            .query(xlstatus_tsdb::AgentId(agent_id.0), range)?,
    );
    Ok(Json(ApiResponse::success(PublicMetricsResponse {
        agent_id: agent_id.0.to_string(),
        range: range.as_str().to_string(),
        series,
    })))
}

pub async fn public_status_mjpeg(State(state): State<AppState>) -> Result<Response, AppError> {
    ensure_public_site_enabled(&state).await?;
    let stream = futures::stream::unfold((state, true), |(state, first)| async move {
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
        Some((Ok::<Bytes, Infallible>(Bytes::from(frame)), (state, false)))
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

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let agent = row.agent;
            let parsed = parse_json(row.last_state_json.as_deref());
            let parsed_info = parse_json(row.last_info_json.as_deref());
            let dashboard = dashboard_metadata(
                row.dashboard_metadata_json.as_deref(),
                &[parsed_info.as_ref(), parsed.as_ref()],
            );
            if hidden_from_public(&dashboard) {
                return None;
            }
            let public_note =
                public_note_from_metadata(&dashboard, &[parsed_info.as_ref(), parsed.as_ref()]);

            Some(PublicServerView {
                id: agent.id.0.to_string(),
                name: agent.name,
                remark: public_note.clone(),
                public_note,
                expires_at: row.expires_at.or_else(|| {
                    metadata_string(
                        &[parsed_info.as_ref(), parsed.as_ref()],
                        &["expires_at", "expired_at", "expire_at", "due_at", "end_at"],
                    )
                }),
                renewal_price: row.renewal_price.or_else(|| {
                    metadata_string(
                        &[parsed_info.as_ref(), parsed.as_ref()],
                        &[
                            "renewal_price",
                            "renew_price",
                            "renewal",
                            "price",
                            "billing_price",
                        ],
                    )
                }),
                price: dashboard.price,
                currency: dashboard.currency,
                billing_cycle: dashboard.billing_cycle,
                auto_renew: dashboard.auto_renew,
                traffic_quota_bytes: dashboard.traffic_quota_bytes,
                traffic_quota_type: dashboard.traffic_quota_type,
                provider: dashboard.provider,
                region: dashboard.region,
                plan: dashboard.plan,
                tags: dashboard.tags,
                accent_color: dashboard.accent_color,
                status: server_status(agent.last_seen_at, agent.revoked_at).to_string(),
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
                net_rx_bps: parsed
                    .as_ref()
                    .and_then(|v| json_i64_by_keys(v, &["net_rx_bps", "network_in_speed"])),
                net_tx_bps: parsed
                    .as_ref()
                    .and_then(|v| json_i64_by_keys(v, &["net_tx_bps", "network_out_speed"])),
                network_in_total: parsed.as_ref().and_then(|v| network_total(v, "bytes_recv")),
                network_out_total: parsed.as_ref().and_then(|v| network_total(v, "bytes_sent")),
                uptime_seconds: parsed
                    .as_ref()
                    .and_then(|v| json_i64_by_keys(v, &["uptime_seconds", "uptime"])),
            })
        })
        .collect())
}

async fn ensure_public_site_enabled(state: &AppState) -> Result<(), AppError> {
    if crate::api::v1::settings::public_site_enabled(&state.db).await? {
        Ok(())
    } else {
        Err(AppError::Forbidden("public status page is private".into()))
    }
}

async fn public_services(state: &AppState) -> Result<Vec<PublicServiceView>, AppError> {
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT s.id, s.name, s.type, s.target, s.server_id,
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
                let history = public_service_history_sqlite(pool, &service_id).await?;
                let service_type: String = row.get("type");
                let server_ids = public_service_server_ids(state, &service_id).await?;
                let legacy_server_id: Option<String> = row.try_get("server_id").ok();
                services.push(PublicServiceView {
                    id: service_id,
                    name: row.get("name"),
                    service_type: service_type.clone(),
                    kind: service_type.clone(),
                    service_type_alias: service_type,
                    target: row.get("target"),
                    server_id: server_ids
                        .first()
                        .cloned()
                        .or_else(|| legacy_server_id.clone()),
                    server_ids: if server_ids.is_empty() {
                        legacy_server_id.into_iter().collect()
                    } else {
                        server_ids
                    },
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
                SELECT s.id::text AS id, s.name, s.type, s.target, s.server_id::text AS server_id,
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
                let history = public_service_history_postgres(pool, &service_id).await?;
                let service_type: String = row.get("type");
                let server_ids = public_service_server_ids(state, &service_id).await?;
                let legacy_server_id: Option<String> = row.try_get("server_id").ok();
                services.push(PublicServiceView {
                    id: service_id,
                    name: row.get("name"),
                    service_type: service_type.clone(),
                    kind: service_type.clone(),
                    service_type_alias: service_type,
                    target: row.get("target"),
                    server_id: server_ids
                        .first()
                        .cloned()
                        .or_else(|| legacy_server_id.clone()),
                    server_ids: if server_ids.is_empty() {
                        legacy_server_id.into_iter().collect()
                    } else {
                        server_ids
                    },
                    last_status: row.try_get("last_status").ok(),
                    last_check_at: row.try_get("last_check_at").ok(),
                    history,
                });
            }
            Ok(services)
        }
    }
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

async fn ensure_public_server_visible(state: &AppState, id: AgentId) -> Result<(), AppError> {
    let agent_repo = AgentRepository::new(state.db.clone());
    let row = agent_repo
        .find_by_id_with_state(id)
        .await?
        .ok_or(AppError::NotFound("server not found".to_string()))?;
    let last_state = parse_json(row.last_state_json.as_deref());
    let last_info = parse_json(row.last_info_json.as_deref());
    let dashboard = dashboard_metadata(
        row.dashboard_metadata_json.as_deref(),
        &[last_info.as_ref(), last_state.as_ref()],
    );
    if hidden_from_public(&dashboard) {
        return Err(AppError::NotFound("server not found".to_string()));
    }
    Ok(())
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

fn parse_json(value: Option<&str>) -> Option<serde_json::Value> {
    value.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
}

fn sanitize_public_json(value: Option<serde_json::Value>) -> Option<serde_json::Value> {
    value.map(sanitize_public_json_value)
}

fn sanitize_public_metric_series(series: MetricSeries) -> MetricSeries {
    MetricSeries {
        agent_id: series.agent_id,
        samples: series
            .samples
            .into_iter()
            .map(|sample| {
                let MetricSample {
                    agent_id,
                    sample_at,
                    fields_json,
                } = sample;
                MetricSample {
                    agent_id,
                    sample_at,
                    fields_json: sanitize_public_json_value(fields_json),
                }
            })
            .collect(),
    }
}

fn sanitize_public_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (key, value) in map {
                if is_sensitive_public_key(&key) {
                    continue;
                }
                sanitized.insert(key, sanitize_public_json_value(value));
            }
            serde_json::Value::Object(sanitized)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sanitize_public_json_value).collect())
        }
        other => other,
    }
}

fn is_sensitive_public_key(key: &str) -> bool {
    let normalized = key
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' ', '.'], "_");

    if matches!(
        normalized.as_str(),
        "ip" | "ipv4"
            | "ipv6"
            | "ips"
            | "ip_address"
            | "ip_addresses"
            | "primary_ip"
            | "public_ip"
            | "public_ips"
            | "private_ip"
            | "private_ips"
            | "listen_ip"
            | "host_ip"
            | "mac"
            | "mac_address"
            | "mac_addresses"
            | "hwaddr"
            | "hostname"
            | "host_name"
            | "machine_id"
            | "remark"
            | "note"
            | "private_note"
            | "admin_note"
            | "token"
            | "secret"
            | "password"
            | "passwd"
            | "authorization"
            | "cookie"
            | "credential"
            | "credentials"
            | "env"
            | "environment"
            | "public_key"
            | "private_key"
    ) {
        return true;
    }

    normalized.ends_with("_ip")
        || normalized.ends_with("_ipv4")
        || normalized.ends_with("_ipv6")
        || normalized.ends_with("_ips")
        || normalized.ends_with("_mac")
        || normalized.ends_with("_macs")
        || normalized.ends_with("_private_note")
        || normalized.ends_with("_admin_note")
        || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("password")
        || normalized.contains("passwd")
        || normalized.contains("authorization")
        || normalized.contains("credential")
        || normalized.contains("private_key")
        || normalized.contains("public_key")
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
    provider: Option<String>,
    region: Option<String>,
    plan: Option<String>,
    price: Option<String>,
    currency: Option<String>,
    billing_cycle: Option<String>,
    auto_renew: Option<bool>,
    traffic_quota_bytes: Option<i64>,
    traffic_quota_type: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    accent_color: Option<String>,
    dashboard_visible: Option<bool>,
    hide_for_guest: Option<bool>,
    display_order: Option<i64>,
}

fn hidden_from_public(dashboard: &DashboardMetadata) -> bool {
    dashboard.dashboard_visible == Some(false) || dashboard.hide_for_guest == Some(true)
}

fn public_note_from_metadata(
    dashboard: &DashboardMetadata,
    fallback_sources: &[Option<&serde_json::Value>],
) -> Option<String> {
    dashboard
        .public_note
        .clone()
        .or_else(|| metadata_string(fallback_sources, &["public_note", "public_description"]))
}

fn dashboard_metadata(
    stored: Option<&str>,
    fallback_sources: &[Option<&serde_json::Value>],
) -> DashboardMetadata {
    let mut out = stored
        .and_then(|value| serde_json::from_str::<DashboardMetadata>(value).ok())
        .unwrap_or_default();

    if out.public_note.is_none() {
        out.public_note = metadata_string(fallback_sources, &["public_note", "public_description"]);
    }
    if out.provider.is_none() {
        out.provider = metadata_string(
            fallback_sources,
            &["provider", "vendor", "datacenter", "isp"],
        );
    }
    if out.region.is_none() {
        out.region = metadata_string(fallback_sources, &["region", "location", "country", "city"]);
    }
    if out.plan.is_none() {
        out.plan = metadata_string(
            fallback_sources,
            &["plan", "package", "sku", "product", "instance_type"],
        );
    }
    if out.price.is_none() {
        out.price = metadata_string(
            fallback_sources,
            &["price", "billing_price", "monthly_price", "amount"],
        );
    }
    if out.currency.is_none() {
        out.currency = metadata_string(fallback_sources, &["currency", "currency_code"]);
    }
    if out.billing_cycle.is_none() {
        out.billing_cycle = metadata_string(
            fallback_sources,
            &["billing_cycle", "cycle", "billing_period", "period"],
        );
    }
    if out.traffic_quota_bytes.is_none() {
        out.traffic_quota_bytes = metadata_i64(
            fallback_sources,
            &[
                "traffic_quota_bytes",
                "traffic_quota",
                "quota_bytes",
                "bandwidth_quota_bytes",
                "monthly_traffic_bytes",
            ],
        );
    }
    if out.traffic_quota_type.is_none() {
        out.traffic_quota_type = metadata_string(
            fallback_sources,
            &[
                "traffic_quota_type",
                "quota_type",
                "traffic_type",
                "bandwidth_type",
            ],
        );
    }
    if out.accent_color.is_none() {
        out.accent_color = metadata_string(fallback_sources, &["accent_color", "color"])
            .and_then(normalize_accent_color);
    }
    if out.tags.is_empty() {
        out.tags = metadata_tags(fallback_sources);
    }
    out.tags = normalize_tags(out.tags);
    out
}

fn metadata_string(sources: &[Option<&serde_json::Value>], keys: &[&str]) -> Option<String> {
    for source in sources.iter().flatten() {
        if let Some(value) = metadata_string_from_value(source, keys) {
            return Some(value);
        }
    }
    None
}

fn metadata_string_from_value(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(json_label) {
            return Some(value);
        }
    }

    for container in ["billing", "plan", "metadata", "custom", "traffic", "limits"] {
        if let Some(child) = value.get(container) {
            for key in keys {
                if let Some(value) = child.get(*key).and_then(json_label) {
                    return Some(value);
                }
            }
        }
    }

    None
}

fn metadata_i64(sources: &[Option<&serde_json::Value>], keys: &[&str]) -> Option<i64> {
    for source in sources.iter().flatten() {
        if let Some(value) = metadata_i64_from_value(source, keys) {
            return Some(value);
        }
    }
    None
}

fn metadata_i64_from_value(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(json_i64) {
            return Some(value);
        }
    }

    for container in ["billing", "plan", "metadata", "custom", "traffic", "limits"] {
        if let Some(child) = value.get(container) {
            for key in keys {
                if let Some(value) = child.get(*key).and_then(json_i64) {
                    return Some(value);
                }
            }
        }
    }

    None
}

fn metadata_tags(sources: &[Option<&serde_json::Value>]) -> Vec<String> {
    for source in sources.iter().flatten() {
        for key in ["tags", "labels"] {
            if let Some(value) = source.get(key) {
                let tags = tags_from_json(value);
                if !tags.is_empty() {
                    return tags;
                }
            }
        }
        for container in ["metadata", "custom"] {
            if let Some(child) = source.get(container) {
                for key in ["tags", "labels"] {
                    if let Some(value) = child.get(key) {
                        let tags = tags_from_json(value);
                        if !tags.is_empty() {
                            return tags;
                        }
                    }
                }
            }
        }
    }
    Vec::new()
}

fn tags_from_json(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => {
            normalize_tags(items.iter().filter_map(json_label).collect::<Vec<_>>())
        }
        serde_json::Value::String(value) => normalize_tags(
            value
                .split([',', ';', '，', '、'])
                .map(str::to_string)
                .collect::<Vec<_>>(),
        ),
        _ => Vec::new(),
    }
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for tag in tags {
        let cleaned = tag.trim();
        if cleaned.is_empty() || out.iter().any(|existing| existing == cleaned) {
            continue;
        }
        out.push(cleaned.chars().take(24).collect());
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn normalize_accent_color(value: String) -> Option<String> {
    let value = value.trim();
    let is_hex = value.len() == 7
        && value.starts_with('#')
        && value.chars().skip(1).all(|ch| ch.is_ascii_hexdigit());
    is_hex.then(|| value.to_string())
}

fn json_label(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
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
