use crate::api::types::*;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::security::validate_outbound_url;
use crate::services::{probe_http, probe_icmp, probe_tcp, ProbeType};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ListServicesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Debug, Deserialize)]
pub struct CreateServiceRequest {
    pub name: String,
    #[serde(alias = "type", alias = "kind")]
    pub service_type: String,
    pub target: String,
    pub interval_seconds: Option<i32>,
    pub timeout_seconds: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub server_ids: Vec<String>,
    #[serde(default)]
    pub cover_mode: Option<String>,
    #[serde(default)]
    pub exclude_server_ids: Vec<String>,
    #[serde(default)]
    pub notification_group_id: Option<String>,
    #[serde(default)]
    pub failure_task_ids: Vec<String>,
    #[serde(default)]
    pub recovery_task_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ServiceListResponse {
    pub services: Vec<ServiceResponse>,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct ServiceResponse {
    pub id: String,
    pub name: String,
    pub service_type: String,
    pub kind: String,
    #[serde(rename = "type")]
    pub service_type_alias: String,
    pub target: String,
    pub interval_seconds: i32,
    pub timeout_seconds: i32,
    pub enabled: bool,
    pub server_id: Option<String>,
    pub server_ids: Vec<String>,
    pub cover_mode: String,
    pub exclude_server_ids: Vec<String>,
    pub notification_group_id: Option<String>,
    pub failure_task_ids: Vec<String>,
    pub recovery_task_ids: Vec<String>,
    pub last_status: Option<String>,
    pub last_check_at: Option<String>,
    pub cert_fingerprint: Option<String>,
    pub cert_not_after: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ProbeTestRequest {
    pub service_type: String,
    pub target: String,
    pub timeout_seconds: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ProbeTestResponse {
    pub success: bool,
    pub latency_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error: Option<String>,
    pub cert_fingerprint: Option<String>,
    pub cert_not_after: Option<String>,
}

pub async fn list_services(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(q): Query<ListServicesQuery>,
) -> Result<Json<ApiResponse<ServiceListResponse>>, AppError> {
    require_scope(&auth, "service:read")?;
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);
    match &state.db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT s.id, s.name, s.type, s.target, s.interval_seconds, s.timeout_seconds,
                       s.enabled, s.server_id, s.notification_group_id, s.created_at, s.updated_at,
                       COALESCE(s.cover_mode, 'local') AS cover_mode,
                       s.exclude_server_ids_json AS exclude_server_ids_json,
                       s.failure_task_ids_json AS failure_task_ids_json,
                       s.recovery_task_ids_json AS recovery_task_ids_json,
                       r.status AS last_status, r.created_at AS last_check_at,
                       r.cert_fingerprint AS cert_fingerprint, r.cert_not_after AS cert_not_after
                FROM services s
                LEFT JOIN service_results r ON r.id = (
                    SELECT sr.id FROM service_results sr
                    WHERE sr.service_id = s.id
                    ORDER BY sr.created_at DESC
                    LIMIT 1
                )
                ORDER BY s.created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM services")
                .fetch_one(pool)
                .await
                .map_err(db_err)?;
            let mut services = rows
                .into_iter()
                .map(service_from_sqlite_row)
                .collect::<Vec<_>>();
            attach_service_server_ids(&state.db, &mut services).await?;
            Ok(Json(ApiResponse::success(ServiceListResponse {
                services,
                total: total.0,
            })))
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT s.id::text AS id, s.name, s.type, s.target, s.interval_seconds, s.timeout_seconds,
                       s.enabled, s.server_id::text AS server_id, s.notification_group_id::text AS notification_group_id,
                       s.created_at::text AS created_at, s.updated_at::text AS updated_at,
                       COALESCE(s.cover_mode, 'local') AS cover_mode,
                       s.exclude_server_ids_json AS exclude_server_ids_json,
                       s.failure_task_ids_json AS failure_task_ids_json,
                       s.recovery_task_ids_json AS recovery_task_ids_json,
                       r.status AS last_status, r.created_at::text AS last_check_at,
                       r.cert_fingerprint AS cert_fingerprint, r.cert_not_after::text AS cert_not_after
                FROM services s
                LEFT JOIN service_results r ON r.id = (
                    SELECT sr.id FROM service_results sr
                    WHERE sr.service_id = s.id
                    ORDER BY sr.created_at DESC
                    LIMIT 1
                )
                ORDER BY s.created_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM services")
                .fetch_one(pool)
                .await
                .map_err(db_err)?;
            let mut services = rows
                .into_iter()
                .map(service_from_postgres_row)
                .collect::<Vec<_>>();
            attach_service_server_ids(&state.db, &mut services).await?;
            Ok(Json(ApiResponse::success(ServiceListResponse {
                services,
                total: total.0,
            })))
        }
    }
}

pub async fn get_service(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ServiceResponse>>, AppError> {
    require_scope(&auth, "service:read")?;
    let service = load_service(&state.db, &id).await?;
    Ok(Json(ApiResponse::success(service)))
}

pub async fn create_service(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<CreateServiceRequest>,
) -> Result<Json<ApiResponse<ServiceResponse>>, AppError> {
    require_scope(&auth, "service:write")?;
    let input = validate_service_request(req).await?;
    ensure_servers_exist(&state.db, &input.server_ids).await?;
    ensure_servers_exist(&state.db, &input.exclude_server_ids).await?;
    let owner = auth.user_id.0.to_string();
    ensure_tasks_owned_by(&state.db, &owner, &input.failure_task_ids).await?;
    ensure_tasks_owned_by(&state.db, &owner, &input.recovery_task_ids).await?;
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    let now_text = now.to_rfc3339();
    let failure_task_ids_json = task_ids_json(&input.failure_task_ids)?;
    let recovery_task_ids_json = task_ids_json(&input.recovery_task_ids)?;
    match &state.db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO services (id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, notification_group_id, failure_task_ids_json, recovery_task_ids_json, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&input.name)
            .bind(input.service_type.as_db())
            .bind(&input.target)
            .bind(input.interval_seconds)
            .bind(input.timeout_seconds)
            .bind(if input.enabled { 1i32 } else { 0i32 })
            .bind(&input.server_id)
            .bind(&input.notification_group_id)
            .bind(&failure_task_ids_json)
            .bind(&recovery_task_ids_json)
            .bind(&now_text)
            .bind(&now_text)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let service_id =
                Uuid::parse_str(&id).map_err(|e| AppError::BadRequest(e.to_string()))?;
            let server_id = input
                .server_id
                .as_deref()
                .map(Uuid::parse_str)
                .transpose()
                .map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))?;
            let group_id = input
                .notification_group_id
                .as_deref()
                .map(Uuid::parse_str)
                .transpose()
                .map_err(|e| AppError::BadRequest(format!("invalid notification_group_id: {e}")))?;
            sqlx::query(
                "INSERT INTO services (id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, notification_group_id, failure_task_ids_json, recovery_task_ids_json, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            )
            .bind(service_id)
            .bind(&input.name)
            .bind(input.service_type.as_db())
            .bind(&input.target)
            .bind(input.interval_seconds)
            .bind(input.timeout_seconds)
            .bind(input.enabled)
            .bind(server_id)
            .bind(group_id)
            .bind(&failure_task_ids_json)
            .bind(&recovery_task_ids_json)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
    }
    replace_service_servers(&state.db, &id, &input.server_ids).await?;
    update_service_cover(&state.db, &id, &input.cover_mode, &input.exclude_server_ids).await?;
    Ok(Json(ApiResponse::success(
        load_service(&state.db, &id).await?,
    )))
}

pub async fn update_service(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Json(req): Json<CreateServiceRequest>,
) -> Result<Json<ApiResponse<ServiceResponse>>, AppError> {
    require_scope(&auth, "service:write")?;
    let input = validate_service_request(req).await?;
    ensure_servers_exist(&state.db, &input.server_ids).await?;
    ensure_servers_exist(&state.db, &input.exclude_server_ids).await?;
    let owner = auth.user_id.0.to_string();
    ensure_tasks_owned_by(&state.db, &owner, &input.failure_task_ids).await?;
    ensure_tasks_owned_by(&state.db, &owner, &input.recovery_task_ids).await?;
    let now = Utc::now();
    let now_text = now.to_rfc3339();
    let failure_task_ids_json = task_ids_json(&input.failure_task_ids)?;
    let recovery_task_ids_json = task_ids_json(&input.recovery_task_ids)?;
    let affected = match &state.db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "UPDATE services SET name = ?, type = ?, target = ?, interval_seconds = ?, timeout_seconds = ?, enabled = ?, server_id = ?, notification_group_id = ?, failure_task_ids_json = ?, recovery_task_ids_json = ?, updated_at = ? WHERE id = ?",
            )
            .bind(&input.name)
            .bind(input.service_type.as_db())
            .bind(&input.target)
            .bind(input.interval_seconds)
            .bind(input.timeout_seconds)
            .bind(if input.enabled { 1i32 } else { 0i32 })
            .bind(&input.server_id)
            .bind(&input.notification_group_id)
            .bind(&failure_task_ids_json)
            .bind(&recovery_task_ids_json)
            .bind(&now_text)
            .bind(&id)
            .execute(pool)
            .await
            .map_err(db_err)?
            .rows_affected()
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let service_id = Uuid::parse_str(&id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let server_id = input
                .server_id
                .as_deref()
                .map(Uuid::parse_str)
                .transpose()
                .map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))?;
            let group_id = input
                .notification_group_id
                .as_deref()
                .map(Uuid::parse_str)
                .transpose()
                .map_err(|e| AppError::BadRequest(format!("invalid notification_group_id: {e}")))?;
            sqlx::query(
                "UPDATE services SET name = $1, type = $2, target = $3, interval_seconds = $4, timeout_seconds = $5, enabled = $6, server_id = $7, notification_group_id = $8, failure_task_ids_json = $9, recovery_task_ids_json = $10, updated_at = $11 WHERE id = $12",
            )
            .bind(&input.name)
            .bind(input.service_type.as_db())
            .bind(&input.target)
            .bind(input.interval_seconds)
            .bind(input.timeout_seconds)
            .bind(input.enabled)
            .bind(server_id)
            .bind(group_id)
            .bind(&failure_task_ids_json)
            .bind(&recovery_task_ids_json)
            .bind(now)
            .bind(service_id)
            .execute(pool)
            .await
            .map_err(db_err)?
            .rows_affected()
        }
    };
    if affected == 0 {
        return Err(AppError::NotFound("service not found".into()));
    }
    replace_service_servers(&state.db, &id, &input.server_ids).await?;
    update_service_cover(&state.db, &id, &input.cover_mode, &input.exclude_server_ids).await?;
    Ok(Json(ApiResponse::success(
        load_service(&state.db, &id).await?,
    )))
}

pub async fn delete_service(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_scope(&auth, "service:delete")?;
    let affected = match &state.db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            sqlx::query("DELETE FROM services WHERE id = ?")
                .bind(&id)
                .execute(pool)
                .await
                .map_err(db_err)?
                .rows_affected()
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let service_id = Uuid::parse_str(&id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            sqlx::query("DELETE FROM services WHERE id = $1")
                .bind(service_id)
                .execute(pool)
                .await
                .map_err(db_err)?
                .rows_affected()
        }
    };
    if affected == 0 {
        return Err(AppError::NotFound("service not found".into()));
    }
    Ok(Json(ApiResponse::success(
        serde_json::json!({"id": id, "deleted": true}),
    )))
}

pub async fn test_probe(
    State(_state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<ProbeTestRequest>,
) -> Result<Json<ApiResponse<ProbeTestResponse>>, AppError> {
    require_scope(&auth, "service:read")?;

    let timeout = req.timeout_seconds.unwrap_or(10).max(1) as u64;

    let result = match ProbeType::from_str(&req.service_type) {
        Some(ProbeType::Http) => probe_http(&req.target, timeout).await,
        Some(ProbeType::Tcp) => {
            let (host, port) = parse_tcp_target(&req.target)?;
            probe_tcp(host, port, timeout).await
        }
        Some(ProbeType::Icmp) => probe_icmp(&req.target, timeout).await,
        None => {
            return Err(AppError::BadRequest("Invalid service type".to_string()));
        }
    };

    let probe = result.map_err(|e| AppError::BadRequest(e.to_string()))?;

    Ok(Json(ApiResponse::success(ProbeTestResponse {
        success: probe.success,
        latency_ms: probe.latency_ms,
        status_code: probe.status_code,
        error: probe.error,
        cert_fingerprint: probe.cert_fingerprint,
        cert_not_after: probe.cert_not_after.map(|ts| ts.to_rfc3339()),
    })))
}

#[derive(Debug)]
struct ValidServiceInput {
    name: String,
    service_type: ProbeType,
    target: String,
    interval_seconds: i32,
    timeout_seconds: i32,
    enabled: bool,
    server_id: Option<String>,
    server_ids: Vec<String>,
    cover_mode: String,
    exclude_server_ids: Vec<String>,
    notification_group_id: Option<String>,
    failure_task_ids: Vec<String>,
    recovery_task_ids: Vec<String>,
}

async fn validate_service_request(
    req: CreateServiceRequest,
) -> Result<ValidServiceInput, AppError> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    let service_type = ProbeType::from_str(&req.service_type)
        .ok_or_else(|| AppError::BadRequest("Invalid service type".into()))?;
    let target = req.target.trim().to_string();
    if target.is_empty() {
        return Err(AppError::BadRequest("target is required".into()));
    }
    let interval_seconds = req.interval_seconds.unwrap_or(60);
    if interval_seconds < 10 {
        return Err(AppError::BadRequest(
            "interval_seconds must be at least 10".into(),
        ));
    }
    let timeout_seconds = req.timeout_seconds.unwrap_or(10);
    if timeout_seconds < 1 {
        return Err(AppError::BadRequest(
            "timeout_seconds must be at least 1".into(),
        ));
    }
    match service_type {
        ProbeType::Http => {
            validate_outbound_url(&target, "HTTP monitor")
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
        }
        ProbeType::Tcp => {
            parse_tcp_target(&target)?;
        }
        ProbeType::Icmp => {}
    }
    let mut server_ids = Vec::new();
    if let Some(server_id) = req.server_id {
        let trimmed = server_id.trim();
        if !trimmed.is_empty() {
            server_ids.push(trimmed.to_string());
        }
    }
    for server_id in req.server_ids {
        let trimmed = server_id.trim();
        if !trimmed.is_empty() && !server_ids.iter().any(|existing| existing == trimmed) {
            server_ids.push(trimmed.to_string());
        }
    }
    for server_id in &server_ids {
        Uuid::parse_str(server_id)
            .map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))?;
    }
    let mut exclude_server_ids = Vec::new();
    for server_id in req.exclude_server_ids {
        let trimmed = server_id.trim();
        if !trimmed.is_empty()
            && !exclude_server_ids
                .iter()
                .any(|existing| existing == trimmed)
        {
            Uuid::parse_str(trimmed)
                .map_err(|e| AppError::BadRequest(format!("invalid exclude_server_id: {e}")))?;
            exclude_server_ids.push(trimmed.to_string());
        }
    }
    let cover_mode = normalize_cover_mode(req.cover_mode, !server_ids.is_empty())?;
    let server_id = server_ids.first().cloned();
    Ok(ValidServiceInput {
        name,
        service_type,
        target,
        interval_seconds,
        timeout_seconds,
        enabled: req.enabled.unwrap_or(true),
        server_id,
        server_ids,
        cover_mode,
        exclude_server_ids,
        notification_group_id: req
            .notification_group_id
            .filter(|value| !value.trim().is_empty()),
        failure_task_ids: normalize_id_list(req.failure_task_ids),
        recovery_task_ids: normalize_id_list(req.recovery_task_ids),
    })
}

impl ProbeType {
    fn as_db(&self) -> &'static str {
        match self {
            ProbeType::Http => "http",
            ProbeType::Tcp => "tcp",
            ProbeType::Icmp => "icmp",
        }
    }
}

fn parse_tcp_target(target: &str) -> Result<(&str, u16), AppError> {
    let (host, port) = target
        .rsplit_once(':')
        .ok_or_else(|| AppError::BadRequest("TCP target must be host:port".to_string()))?;
    if host.trim().is_empty() {
        return Err(AppError::BadRequest("TCP host is required".into()));
    }
    let port = port
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid port".to_string()))?;
    Ok((host, port))
}

fn normalize_cover_mode(
    cover_mode: Option<String>,
    has_server_ids: bool,
) -> Result<String, AppError> {
    let mode = cover_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(if has_server_ids { "specific" } else { "local" })
        .to_ascii_lowercase();
    match mode.as_str() {
        "local" | "all" | "specific" | "exclude" => Ok(mode),
        _ => Err(AppError::BadRequest(
            "cover_mode must be local, all, specific, or exclude".into(),
        )),
    }
}

async fn load_service(db: &crate::db::Db, id: &str) -> Result<ServiceResponse, AppError> {
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query(
                r#"
                SELECT s.id, s.name, s.type, s.target, s.interval_seconds, s.timeout_seconds,
                       s.enabled, s.server_id, s.notification_group_id, s.created_at, s.updated_at,
                       COALESCE(s.cover_mode, 'local') AS cover_mode,
                       s.exclude_server_ids_json AS exclude_server_ids_json,
                       s.failure_task_ids_json AS failure_task_ids_json,
                       s.recovery_task_ids_json AS recovery_task_ids_json,
                       r.status AS last_status, r.created_at AS last_check_at,
                       r.cert_fingerprint AS cert_fingerprint, r.cert_not_after AS cert_not_after
                FROM services s
                LEFT JOIN service_results r ON r.id = (
                    SELECT sr.id FROM service_results sr
                    WHERE sr.service_id = s.id
                    ORDER BY sr.created_at DESC
                    LIMIT 1
                )
                WHERE s.id = ?
                "#,
            )
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            let mut service = row
                .map(service_from_sqlite_row)
                .ok_or_else(|| AppError::NotFound("service not found".into()))?;
            attach_service_server_ids(db, std::slice::from_mut(&mut service)).await?;
            Ok(service)
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let service_id = Uuid::parse_str(id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let row = sqlx::query(
                r#"
                SELECT s.id::text AS id, s.name, s.type, s.target, s.interval_seconds, s.timeout_seconds,
                       s.enabled, s.server_id::text AS server_id, s.notification_group_id::text AS notification_group_id,
                       s.created_at::text AS created_at, s.updated_at::text AS updated_at,
                       COALESCE(s.cover_mode, 'local') AS cover_mode,
                       s.exclude_server_ids_json AS exclude_server_ids_json,
                       s.failure_task_ids_json AS failure_task_ids_json,
                       s.recovery_task_ids_json AS recovery_task_ids_json,
                       r.status AS last_status, r.created_at::text AS last_check_at,
                       r.cert_fingerprint AS cert_fingerprint, r.cert_not_after::text AS cert_not_after
                FROM services s
                LEFT JOIN service_results r ON r.id = (
                    SELECT sr.id FROM service_results sr
                    WHERE sr.service_id = s.id
                    ORDER BY sr.created_at DESC
                    LIMIT 1
                )
                WHERE s.id = $1
                "#,
            )
            .bind(service_id)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            let mut service = row
                .map(service_from_postgres_row)
                .ok_or_else(|| AppError::NotFound("service not found".into()))?;
            attach_service_server_ids(db, std::slice::from_mut(&mut service)).await?;
            Ok(service)
        }
    }
}

fn service_from_sqlite_row(row: sqlx::sqlite::SqliteRow) -> ServiceResponse {
    let kind: String = row.try_get("type").unwrap_or_else(|_| "http".into());
    ServiceResponse {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        service_type: kind.clone(),
        kind: kind.clone(),
        service_type_alias: kind,
        target: row.try_get("target").unwrap_or_default(),
        interval_seconds: row.try_get::<i64, _>("interval_seconds").unwrap_or(60) as i32,
        timeout_seconds: row.try_get::<i64, _>("timeout_seconds").unwrap_or(10) as i32,
        enabled: row.try_get::<i64, _>("enabled").unwrap_or(0) != 0,
        server_id: row.try_get("server_id").ok(),
        server_ids: row
            .try_get::<Option<String>, _>("server_id")
            .ok()
            .flatten()
            .into_iter()
            .collect(),
        cover_mode: row
            .try_get::<String, _>("cover_mode")
            .unwrap_or_else(|_| "local".into()),
        exclude_server_ids: parse_server_ids_json(
            row.try_get::<Option<String>, _>("exclude_server_ids_json")
                .ok()
                .flatten(),
        ),
        notification_group_id: row.try_get("notification_group_id").ok(),
        failure_task_ids: parse_task_ids_json(
            row.try_get::<Option<String>, _>("failure_task_ids_json")
                .ok()
                .flatten(),
        ),
        recovery_task_ids: parse_task_ids_json(
            row.try_get::<Option<String>, _>("recovery_task_ids_json")
                .ok()
                .flatten(),
        ),
        last_status: row.try_get("last_status").ok(),
        last_check_at: row.try_get("last_check_at").ok(),
        cert_fingerprint: row.try_get("cert_fingerprint").ok(),
        cert_not_after: row.try_get("cert_not_after").ok(),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

fn service_from_postgres_row(row: sqlx::postgres::PgRow) -> ServiceResponse {
    let kind: String = row.try_get("type").unwrap_or_else(|_| "http".into());
    ServiceResponse {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        service_type: kind.clone(),
        kind: kind.clone(),
        service_type_alias: kind,
        target: row.try_get("target").unwrap_or_default(),
        interval_seconds: row.try_get("interval_seconds").unwrap_or(60),
        timeout_seconds: row.try_get("timeout_seconds").unwrap_or(10),
        enabled: row.try_get("enabled").unwrap_or(false),
        server_id: row.try_get("server_id").ok(),
        server_ids: row
            .try_get::<Option<String>, _>("server_id")
            .ok()
            .flatten()
            .into_iter()
            .collect(),
        cover_mode: row
            .try_get::<String, _>("cover_mode")
            .unwrap_or_else(|_| "local".into()),
        exclude_server_ids: parse_server_ids_json(
            row.try_get::<Option<String>, _>("exclude_server_ids_json")
                .ok()
                .flatten(),
        ),
        notification_group_id: row.try_get("notification_group_id").ok(),
        failure_task_ids: parse_task_ids_json(
            row.try_get::<Option<String>, _>("failure_task_ids_json")
                .ok()
                .flatten(),
        ),
        recovery_task_ids: parse_task_ids_json(
            row.try_get::<Option<String>, _>("recovery_task_ids_json")
                .ok()
                .flatten(),
        ),
        last_status: row.try_get("last_status").ok(),
        last_check_at: row.try_get("last_check_at").ok(),
        cert_fingerprint: row.try_get("cert_fingerprint").ok(),
        cert_not_after: row.try_get("cert_not_after").ok(),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

async fn attach_service_server_ids(
    db: &crate::db::Db,
    services: &mut [ServiceResponse],
) -> Result<(), AppError> {
    for service in services {
        let server_ids = load_service_server_ids(db, &service.id).await?;
        if server_ids.is_empty() {
            service.server_ids = service.server_id.clone().into_iter().collect();
        } else {
            service.server_id = server_ids.first().cloned();
            service.server_ids = server_ids;
        }
    }
    Ok(())
}

async fn load_service_server_ids(
    db: &crate::db::Db,
    service_id: &str,
) -> Result<Vec<String>, AppError> {
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT server_id FROM service_servers WHERE service_id = ? ORDER BY created_at ASC, server_id ASC",
            )
            .bind(service_id)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            Ok(rows.into_iter().map(|(id,)| id).collect())
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let service_id = Uuid::parse_str(service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT server_id::text FROM service_servers WHERE service_id = $1 ORDER BY created_at ASC, server_id ASC",
            )
            .bind(service_id)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            Ok(rows.into_iter().map(|(id,)| id).collect())
        }
    }
}

async fn ensure_servers_exist(db: &crate::db::Db, server_ids: &[String]) -> Result<(), AppError> {
    for server_id in server_ids {
        let exists = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM agents WHERE id = ?")
                    .bind(server_id)
                    .fetch_one(pool)
                    .await
                    .map_err(db_err)?;
                row.0 > 0
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let parsed = Uuid::parse_str(server_id)
                    .map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))?;
                let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM agents WHERE id = $1")
                    .bind(parsed)
                    .fetch_one(pool)
                    .await
                    .map_err(db_err)?;
                row.0 > 0
            }
        };
        if !exists {
            return Err(AppError::BadRequest(format!(
                "server_id not found: {server_id}"
            )));
        }
    }
    Ok(())
}

async fn ensure_tasks_owned_by(
    db: &crate::db::Db,
    owner_user_id: &str,
    task_ids: &[String],
) -> Result<(), AppError> {
    for task_id in task_ids {
        let exists = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM tasks WHERE id = ? AND owner_user_id = ?")
                        .bind(task_id)
                        .bind(owner_user_id)
                        .fetch_one(pool)
                        .await
                        .map_err(db_err)?;
                row.0 > 0
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let owner = Uuid::parse_str(owner_user_id)
                    .map_err(|e| AppError::BadRequest(format!("invalid owner_user_id: {e}")))?;
                let row: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM tasks WHERE id = $1 AND owner_user_id = $2",
                )
                .bind(task_id)
                .bind(owner)
                .fetch_one(pool)
                .await
                .map_err(db_err)?;
                row.0 > 0
            }
        };
        if !exists {
            return Err(AppError::BadRequest(format!(
                "task {task_id} does not exist or is not owned by current user"
            )));
        }
    }
    Ok(())
}

async fn replace_service_servers(
    db: &crate::db::Db,
    service_id: &str,
    server_ids: &[String],
) -> Result<(), AppError> {
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            sqlx::query("DELETE FROM service_servers WHERE service_id = ?")
                .bind(service_id)
                .execute(pool)
                .await
                .map_err(db_err)?;
            let now = Utc::now().to_rfc3339();
            for server_id in server_ids {
                sqlx::query(
                    "INSERT OR IGNORE INTO service_servers (service_id, server_id, created_at) VALUES (?, ?, ?)",
                )
                .bind(service_id)
                .bind(server_id)
                .bind(&now)
                .execute(pool)
                .await
                .map_err(db_err)?;
            }
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let parsed_service_id = Uuid::parse_str(service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            sqlx::query("DELETE FROM service_servers WHERE service_id = $1")
                .bind(parsed_service_id)
                .execute(pool)
                .await
                .map_err(db_err)?;
            let now = Utc::now();
            for server_id in server_ids {
                let parsed_server_id = Uuid::parse_str(server_id)
                    .map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))?;
                sqlx::query(
                    "INSERT INTO service_servers (service_id, server_id, created_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                )
                .bind(parsed_service_id)
                .bind(parsed_server_id)
                .bind(now)
                .execute(pool)
                .await
                .map_err(db_err)?;
            }
        }
    }
    Ok(())
}

async fn update_service_cover(
    db: &crate::db::Db,
    service_id: &str,
    cover_mode: &str,
    exclude_server_ids: &[String],
) -> Result<(), AppError> {
    let exclude_json = if exclude_server_ids.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(exclude_server_ids)
                .map_err(|e| AppError::BadRequest(e.to_string()))?,
        )
    };
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "UPDATE services SET cover_mode = ?, exclude_server_ids_json = ? WHERE id = ?",
            )
            .bind(cover_mode)
            .bind(&exclude_json)
            .bind(service_id)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let parsed_service_id = Uuid::parse_str(service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            sqlx::query(
                "UPDATE services SET cover_mode = $1, exclude_server_ids_json = $2 WHERE id = $3",
            )
            .bind(cover_mode)
            .bind(&exclude_json)
            .bind(parsed_service_id)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
    }
    Ok(())
}

fn parse_server_ids_json(value: Option<String>) -> Vec<String> {
    value
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn parse_task_ids_json(value: Option<String>) -> Vec<String> {
    value
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn normalize_id_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn task_ids_json(values: &[String]) -> Result<String, AppError> {
    serde_json::to_string(values).map_err(|e| AppError::BadRequest(e.to_string()))
}

fn require_scope(auth: &AuthSession, scope: &str) -> Result<(), AppError> {
    if has_scope(auth, scope) {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!("missing scope: {scope}")))
    }
}

fn db_err(err: sqlx::Error) -> AppError {
    AppError::Database(anyhow::anyhow!(err))
}
