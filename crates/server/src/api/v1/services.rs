use crate::api::types::*;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::notifications::ensure_notification_group_owned_by;
use crate::api::v1::servers::agent_visible;
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::AgentRepository;
use crate::security::{validate_outbound_host, validate_outbound_url};
use crate::services::{probe_http, probe_icmp, probe_tcp, ProbeType};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
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

const SERVICE_LIST_SQLITE: &str = r#"
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
"#;

const SERVICE_LIST_POSTGRES: &str = r#"
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
"#;

pub(crate) const SERVICE_API_MAX_BODY_BYTES: usize = 128 * 1024;
pub(crate) const SERVICE_MAX_NAME_BYTES: usize = 128;
pub(crate) const SERVICE_MAX_TARGET_BYTES: usize = 2048;
pub(crate) const SERVICE_MIN_INTERVAL_SECONDS: i32 = 10;
pub(crate) const SERVICE_MAX_INTERVAL_SECONDS: i32 = 86_400;
pub(crate) const SERVICE_MIN_TIMEOUT_SECONDS: i32 = 1;
pub(crate) const SERVICE_MAX_TIMEOUT_SECONDS: i32 = 30;
pub(crate) const SERVICE_MAX_SERVER_IDS: usize = 64;
pub(crate) const SERVICE_MAX_TASK_IDS: usize = 32;
pub(crate) const SERVICE_MAX_TARGETS_PER_PROBE: usize = 64;

const VISIBLE_SERVICE_FILTER_SQL: &str = r#"
                (
                    EXISTS (
                        SELECT 1 FROM service_servers ss
                        JOIN visible_server_ids vsi ON vsi.id = ss.server_id
                        WHERE ss.service_id = s.id
                    )
                    OR (
                        NOT EXISTS (
                            SELECT 1 FROM service_servers ss
                            WHERE ss.service_id = s.id
                        )
                        AND s.server_id IS NOT NULL
                        AND TRIM(s.server_id) <> ''
                        AND EXISTS (
                            SELECT 1 FROM visible_server_ids vsi
                            WHERE vsi.id = s.server_id
                        )
                    )
                )
                AND NOT EXISTS (
                    SELECT 1 FROM service_servers ss
                    WHERE ss.service_id = s.id
                    AND NOT EXISTS (
                        SELECT 1 FROM visible_server_ids vsi
                        WHERE vsi.id = ss.server_id
                    )
                )
"#;

pub async fn list_services(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(q): Query<ListServicesQuery>,
) -> Result<Json<ApiResponse<ServiceListResponse>>, AppError> {
    require_scope(&auth, "service:read")?;
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);
    Ok(Json(ApiResponse::success(
        list_services_for_auth(&state.db, &auth, limit, offset).await?,
    )))
}

async fn list_services_for_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    limit: i64,
    offset: i64,
) -> Result<ServiceListResponse, AppError> {
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let visible_server_ids = visible_server_ids_for_auth(db, auth).await?;
            if !auth_has_global_service_visibility(auth) && visible_server_ids.is_empty() {
                return Ok(ServiceListResponse {
                    services: Vec::new(),
                    total: 0,
                });
            }
            let total = if auth_has_global_service_visibility(auth) {
                let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM services")
                    .fetch_one(pool)
                    .await
                    .map_err(db_err)?;
                row.0
            } else {
                fetch_visible_service_count_sqlite(pool, &visible_server_ids).await?
            };
            let rows = if auth_has_global_service_visibility(auth) {
                sqlx::query(&format!(
                    "{SERVICE_LIST_SQLITE} ORDER BY s.created_at DESC LIMIT ? OFFSET ?"
                ))
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
                .map_err(db_err)?
            } else {
                fetch_visible_service_rows_sqlite(pool, &visible_server_ids, limit, offset).await?
            };
            let mut services = rows
                .into_iter()
                .map(service_from_sqlite_row)
                .collect::<Vec<_>>();
            attach_service_server_ids(db, &mut services).await?;
            Ok(ServiceListResponse { services, total })
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let visible_server_ids = visible_server_ids_for_auth(db, auth).await?;
            if !auth_has_global_service_visibility(auth) && visible_server_ids.is_empty() {
                return Ok(ServiceListResponse {
                    services: Vec::new(),
                    total: 0,
                });
            }
            let total = if auth_has_global_service_visibility(auth) {
                let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM services")
                    .fetch_one(pool)
                    .await
                    .map_err(db_err)?;
                row.0
            } else {
                fetch_visible_service_count_postgres(pool, &visible_server_ids).await?
            };
            let rows = if auth_has_global_service_visibility(auth) {
                sqlx::query(&format!(
                    "{SERVICE_LIST_POSTGRES} ORDER BY s.created_at DESC LIMIT $1 OFFSET $2"
                ))
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
                .map_err(db_err)?
            } else {
                fetch_visible_service_rows_postgres(pool, &visible_server_ids, limit, offset)
                    .await?
            };
            let mut services = rows
                .into_iter()
                .map(service_from_postgres_row)
                .collect::<Vec<_>>();
            attach_service_server_ids(db, &mut services).await?;
            Ok(ServiceListResponse { services, total })
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
    ensure_service_visible_to_auth(&state.db, &auth, &service).await?;
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
    ensure_service_input_servers_visible(&state.db, &auth, &input).await?;
    let owner = auth.user_id.0.to_string();
    ensure_notification_group_owned_by(
        &state.db,
        auth.user_id.0,
        input.notification_group_id.as_deref(),
    )
    .await?;
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
                "INSERT INTO services (id, owner_user_id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, notification_group_id, failure_task_ids_json, recovery_task_ids_json, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&owner)
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
                "INSERT INTO services (id, owner_user_id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, notification_group_id, failure_task_ids_json, recovery_task_ids_json, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            )
            .bind(service_id)
            .bind(auth.user_id.0)
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
    let existing = load_service(&state.db, &id).await?;
    ensure_service_visible_to_auth(&state.db, &auth, &existing).await?;
    ensure_servers_exist(&state.db, &input.server_ids).await?;
    ensure_servers_exist(&state.db, &input.exclude_server_ids).await?;
    ensure_service_input_servers_visible(&state.db, &auth, &input).await?;
    let owner = auth.user_id.0.to_string();
    ensure_notification_group_owned_by(
        &state.db,
        auth.user_id.0,
        input.notification_group_id.as_deref(),
    )
    .await?;
    ensure_tasks_owned_by(&state.db, &owner, &input.failure_task_ids).await?;
    ensure_tasks_owned_by(&state.db, &owner, &input.recovery_task_ids).await?;
    let now = Utc::now();
    let now_text = now.to_rfc3339();
    let failure_task_ids_json = task_ids_json(&input.failure_task_ids)?;
    let recovery_task_ids_json = task_ids_json(&input.recovery_task_ids)?;
    let affected = match &state.db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "UPDATE services SET owner_user_id = ?, name = ?, type = ?, target = ?, interval_seconds = ?, timeout_seconds = ?, enabled = ?, server_id = ?, notification_group_id = ?, failure_task_ids_json = ?, recovery_task_ids_json = ?, updated_at = ? WHERE id = ?",
            )
            .bind(&owner)
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
                "UPDATE services SET owner_user_id = $1, name = $2, type = $3, target = $4, interval_seconds = $5, timeout_seconds = $6, enabled = $7, server_id = $8, notification_group_id = $9, failure_task_ids_json = $10, recovery_task_ids_json = $11, updated_at = $12 WHERE id = $13",
            )
            .bind(auth.user_id.0)
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
    let service = load_service(&state.db, &id).await?;
    ensure_service_visible_to_auth(&state.db, &auth, &service).await?;
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
    require_probe_test_scope(&auth)?;

    let timeout = normalize_probe_timeout(req.timeout_seconds)? as u64;

    let result = match ProbeType::from_str(&req.service_type) {
        Some(ProbeType::Http) => probe_http(&req.target, timeout).await,
        Some(ProbeType::Tcp) => {
            let (host, port) = parse_tcp_target(&req.target)?;
            validate_outbound_host(host, port, "TCP probe")
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            probe_tcp(host, port, timeout).await
        }
        Some(ProbeType::Icmp) => {
            validate_outbound_host(&req.target, 0, "ICMP probe")
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            probe_icmp(&req.target, timeout).await
        }
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

pub fn service_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(SERVICE_API_MAX_BODY_BYTES)
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
    ensure_byte_len(&name, SERVICE_MAX_NAME_BYTES, "name")?;
    let service_type = ProbeType::from_str(&req.service_type)
        .ok_or_else(|| AppError::BadRequest("Invalid service type".into()))?;
    let target = req.target.trim().to_string();
    if target.is_empty() {
        return Err(AppError::BadRequest("target is required".into()));
    }
    ensure_byte_len(&target, SERVICE_MAX_TARGET_BYTES, "target")?;
    let interval_seconds = normalize_service_interval(req.interval_seconds)?;
    let timeout_seconds = normalize_probe_timeout(req.timeout_seconds)?;
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
            ensure_byte_len(trimmed, 128, "server_id")?;
            server_ids.push(trimmed.to_string());
        }
    }
    ensure_list_len(&server_ids, SERVICE_MAX_SERVER_IDS, "server_ids")?;
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
            ensure_byte_len(trimmed, 128, "exclude_server_id")?;
            Uuid::parse_str(trimmed)
                .map_err(|e| AppError::BadRequest(format!("invalid exclude_server_id: {e}")))?;
            exclude_server_ids.push(trimmed.to_string());
        }
    }
    ensure_list_len(
        &exclude_server_ids,
        SERVICE_MAX_SERVER_IDS,
        "exclude_server_ids",
    )?;
    let cover_mode = normalize_cover_mode(req.cover_mode, !server_ids.is_empty())?;
    let server_id = server_ids.first().cloned();
    let notification_group_id = req
        .notification_group_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(group_id) = notification_group_id.as_deref() {
        ensure_byte_len(group_id, 128, "notification_group_id")?;
        Uuid::parse_str(group_id)
            .map_err(|e| AppError::BadRequest(format!("invalid notification_group_id: {e}")))?;
    }
    let failure_task_ids = normalize_id_list(req.failure_task_ids);
    let recovery_task_ids = normalize_id_list(req.recovery_task_ids);
    ensure_list_len(&failure_task_ids, SERVICE_MAX_TASK_IDS, "failure_task_ids")?;
    ensure_list_len(
        &recovery_task_ids,
        SERVICE_MAX_TASK_IDS,
        "recovery_task_ids",
    )?;
    validate_task_id_list(&failure_task_ids)?;
    validate_task_id_list(&recovery_task_ids)?;
    match service_type {
        ProbeType::Http => {
            validate_outbound_url(&target, "HTTP monitor")
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
        }
        ProbeType::Tcp => {
            let (host, port) = parse_tcp_target(&target)?;
            validate_outbound_host(host, port, "TCP monitor")
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
        }
        ProbeType::Icmp => {
            validate_outbound_host(&target, 0, "ICMP monitor")
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
        }
    }
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
        notification_group_id,
        failure_task_ids,
        recovery_task_ids,
    })
}

fn normalize_service_interval(value: Option<i32>) -> Result<i32, AppError> {
    let seconds = value.unwrap_or(60);
    if !(SERVICE_MIN_INTERVAL_SECONDS..=SERVICE_MAX_INTERVAL_SECONDS).contains(&seconds) {
        return Err(AppError::BadRequest(format!(
            "interval_seconds must be between {SERVICE_MIN_INTERVAL_SECONDS} and {SERVICE_MAX_INTERVAL_SECONDS}"
        )));
    }
    Ok(seconds)
}

fn normalize_probe_timeout(value: Option<i32>) -> Result<i32, AppError> {
    let seconds = value.unwrap_or(10);
    if !(SERVICE_MIN_TIMEOUT_SECONDS..=SERVICE_MAX_TIMEOUT_SECONDS).contains(&seconds) {
        return Err(AppError::BadRequest(format!(
            "timeout_seconds must be between {SERVICE_MIN_TIMEOUT_SECONDS} and {SERVICE_MAX_TIMEOUT_SECONDS}"
        )));
    }
    Ok(seconds)
}

fn ensure_byte_len(value: &str, max_bytes: usize, field: &str) -> Result<(), AppError> {
    if value.len() > max_bytes {
        return Err(AppError::BadRequest(format!(
            "{field} must be at most {max_bytes} bytes"
        )));
    }
    Ok(())
}

fn ensure_list_len<T>(values: &[T], max_len: usize, field: &str) -> Result<(), AppError> {
    if values.len() > max_len {
        return Err(AppError::BadRequest(format!(
            "{field} must contain at most {max_len} entries"
        )));
    }
    Ok(())
}

fn validate_task_id_list(task_ids: &[String]) -> Result<(), AppError> {
    for task_id in task_ids {
        ensure_byte_len(task_id, 128, "task_id")?;
    }
    Ok(())
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

pub(crate) async fn load_service(
    db: &crate::db::Db,
    id: &str,
) -> Result<ServiceResponse, AppError> {
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

pub(crate) async fn visible_server_ids_for_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
) -> Result<Vec<String>, AppError> {
    if auth.role.is_admin() && auth.server_ids.is_none() {
        return Ok(Vec::new());
    }

    let allowlist = auth
        .server_ids
        .as_ref()
        .map(|ids| normalize_id_list(ids.clone()));

    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let mut query = if auth.role.is_admin() {
                "SELECT id FROM agents WHERE 1 = 1".to_string()
            } else {
                "SELECT id FROM agents WHERE owner_user_id = ?".to_string()
            };
            if let Some(ids) = &allowlist {
                if ids.is_empty() {
                    return Ok(Vec::new());
                }
                query.push_str(" AND id IN (");
                query.push_str(&placeholders("?", ids.len()));
                query.push(')');
            }
            query.push_str(" ORDER BY id ASC");
            let mut sql = sqlx::query(&query);
            if !auth.role.is_admin() {
                sql = sql.bind(auth.user_id.0.to_string());
            }
            if let Some(ids) = &allowlist {
                for id in ids {
                    sql = sql.bind(id);
                }
            }
            let rows = sql.fetch_all(pool).await.map_err(db_err)?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("id").ok())
                .collect())
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let mut query = if auth.role.is_admin() {
                "SELECT id::text AS id FROM agents WHERE 1 = 1".to_string()
            } else {
                "SELECT id::text AS id FROM agents WHERE owner_user_id = $1".to_string()
            };
            if let Some(ids) = &allowlist {
                if ids.is_empty() {
                    return Ok(Vec::new());
                }
                query.push_str(" AND id IN (");
                query.push_str(&numbered_placeholders(
                    if auth.role.is_admin() { 1 } else { 2 },
                    ids.len(),
                ));
                query.push(')');
            }
            query.push_str(" ORDER BY id ASC");
            let mut sql = sqlx::query(&query);
            if !auth.role.is_admin() {
                sql = sql.bind(auth.user_id.0);
            }
            if let Some(ids) = &allowlist {
                for id in ids {
                    let parsed = Uuid::parse_str(id)
                        .map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))?;
                    sql = sql.bind(parsed);
                }
            }
            let rows = sql.fetch_all(pool).await.map_err(db_err)?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("id").ok())
                .collect())
        }
    }
}

async fn fetch_visible_service_count_sqlite(
    pool: &sqlx::SqlitePool,
    visible_server_ids: &[String],
) -> Result<i64, AppError> {
    let visible_cte = sqlite_visible_server_ids_cte(visible_server_ids.len());
    let sql =
        format!("{visible_cte} SELECT COUNT(*) FROM services s WHERE {VISIBLE_SERVICE_FILTER_SQL}");
    let mut query = sqlx::query_as::<_, (i64,)>(&sql);
    for id in visible_server_ids {
        query = query.bind(id);
    }
    let row = query.fetch_one(pool).await.map_err(db_err)?;
    Ok(row.0)
}

async fn fetch_visible_service_rows_sqlite(
    pool: &sqlx::SqlitePool,
    visible_server_ids: &[String],
    limit: i64,
    offset: i64,
) -> Result<Vec<sqlx::sqlite::SqliteRow>, AppError> {
    let visible_cte = sqlite_visible_server_ids_cte(visible_server_ids.len());
    let sql = format!(
        "{visible_cte} {SERVICE_LIST_SQLITE} WHERE {VISIBLE_SERVICE_FILTER_SQL} ORDER BY s.created_at DESC LIMIT ? OFFSET ?"
    );
    let mut query = sqlx::query(&sql);
    for id in visible_server_ids {
        query = query.bind(id);
    }
    query
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(db_err)
}

async fn fetch_visible_service_count_postgres(
    pool: &sqlx::PgPool,
    visible_server_ids: &[String],
) -> Result<i64, AppError> {
    let parsed = parse_uuid_ids(visible_server_ids)?;
    let sql = format!(
        "WITH visible_server_ids(id) AS (SELECT UNNEST($1::uuid[])) SELECT COUNT(*) FROM services s WHERE {VISIBLE_SERVICE_FILTER_SQL}"
    );
    let row: (i64,) = sqlx::query_as(&sql)
        .bind(parsed)
        .fetch_one(pool)
        .await
        .map_err(db_err)?;
    Ok(row.0)
}

async fn fetch_visible_service_rows_postgres(
    pool: &sqlx::PgPool,
    visible_server_ids: &[String],
    limit: i64,
    offset: i64,
) -> Result<Vec<sqlx::postgres::PgRow>, AppError> {
    let parsed = parse_uuid_ids(visible_server_ids)?;
    let sql = format!(
        "WITH visible_server_ids(id) AS (SELECT UNNEST($1::uuid[])) {SERVICE_LIST_POSTGRES} WHERE {VISIBLE_SERVICE_FILTER_SQL} ORDER BY s.created_at DESC LIMIT $2 OFFSET $3"
    );
    sqlx::query(&sql)
        .bind(parsed)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(db_err)
}

fn sqlite_visible_server_ids_cte(len: usize) -> String {
    format!(
        "WITH visible_server_ids(id) AS (VALUES {})",
        (0..len).map(|_| "(?)").collect::<Vec<_>>().join(", ")
    )
}

fn placeholders(token: &str, len: usize) -> String {
    std::iter::repeat_n(token, len)
        .collect::<Vec<_>>()
        .join(", ")
}

fn numbered_placeholders(start: usize, len: usize) -> String {
    (start..start + len)
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_uuid_ids(ids: &[String]) -> Result<Vec<Uuid>, AppError> {
    ids.iter()
        .map(|id| {
            Uuid::parse_str(id).map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))
        })
        .collect()
}

pub(crate) fn auth_has_global_service_visibility(auth: &AuthSession) -> bool {
    auth.role.is_admin() && auth.server_ids.is_none()
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

pub(crate) async fn ensure_service_id_visible_to_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    service_id: &str,
) -> Result<(), AppError> {
    let service = load_service(db, service_id).await?;
    ensure_service_visible_to_auth(db, auth, &service).await
}

async fn ensure_service_visible_to_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    service: &ServiceResponse,
) -> Result<(), AppError> {
    if service_visible_to_auth(db, auth, service).await? {
        Ok(())
    } else {
        Err(AppError::Forbidden("service not in scope".into()))
    }
}

async fn service_visible_to_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    service: &ServiceResponse,
) -> Result<bool, AppError> {
    if auth_has_global_service_visibility(auth) {
        return Ok(true);
    }
    let server_ids = service_effective_server_ids(service);
    if server_ids.is_empty() {
        return Ok(false);
    }
    for server_id in server_ids {
        if !server_visible_to_auth(db, auth, &server_id).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn ensure_service_input_servers_visible(
    db: &crate::db::Db,
    auth: &AuthSession,
    input: &ValidServiceInput,
) -> Result<(), AppError> {
    if auth_has_global_service_visibility(auth) {
        return Ok(());
    }
    if input.cover_mode != "specific" {
        return Err(AppError::Forbidden(
            "non-global service monitors must use specific servers".into(),
        ));
    }
    if input.server_ids.is_empty() {
        return Err(AppError::Forbidden(
            "non-global services must be scoped to owned servers".into(),
        ));
    }
    let mut server_ids = input.server_ids.clone();
    server_ids.extend(input.exclude_server_ids.clone());
    for server_id in server_ids {
        if !server_visible_to_auth(db, auth, &server_id).await? {
            return Err(AppError::Forbidden("server not in scope".into()));
        }
    }
    Ok(())
}

async fn server_visible_to_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    server_id: &str,
) -> Result<bool, AppError> {
    let agent_id = Uuid::parse_str(server_id)
        .map(xlstatus_shared::AgentId)
        .map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))?;
    let agent = AgentRepository::new(db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("server not found".into()))?;
    Ok(agent_visible(auth, &agent))
}

fn service_effective_server_ids(service: &ServiceResponse) -> Vec<String> {
    let mut server_ids = service.server_ids.clone();
    if let Some(server_id) = &service.server_id {
        if !server_ids.iter().any(|existing| existing == server_id) {
            server_ids.push(server_id.clone());
        }
    }
    server_ids
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

fn require_probe_test_scope(auth: &AuthSession) -> Result<(), AppError> {
    require_scope(auth, "service:write")
}

fn db_err(err: sqlx::Error) -> AppError {
    AppError::Database(anyhow::anyhow!(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::middleware::AuthKind;
    use crate::db::DatabaseBackend;
    use xlstatus_shared::{UserId, UserRole};

    #[tokio::test]
    async fn list_services_filters_visibility_before_pagination() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let own_server = Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();
        let other_server = Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(&db, own_server, owner, "own").await;
        seed_agent(&db, other_server, other, "other").await;

        seed_service(
            &db,
            "00000000-0000-0000-0000-000000000301",
            "other-newer",
            other_server,
            "2026-01-03T00:00:00Z",
        )
        .await;
        seed_service(
            &db,
            "00000000-0000-0000-0000-000000000302",
            "own-older",
            own_server,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_service_with_servers(
            &db,
            "00000000-0000-0000-0000-000000000303",
            "mixed",
            &[own_server, other_server],
            "2026-01-01T00:00:00Z",
        )
        .await;

        let response = list_services_for_auth(&db, &member_session(owner), 1, 0)
            .await
            .unwrap();

        assert_eq!(response.total, 1);
        assert_eq!(response.services.len(), 1);
        assert_eq!(response.services[0].name, "own-older");
        assert_eq!(
            response.services[0].server_ids,
            vec![own_server.to_string()]
        );
    }

    #[tokio::test]
    async fn admin_pat_service_list_respects_server_allowlist() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let own_server = Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();
        let other_server = Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();

        seed_user(&db, owner, "owner", "admin").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(&db, own_server, owner, "own").await;
        seed_agent(&db, other_server, other, "other").await;
        seed_service(
            &db,
            "00000000-0000-0000-0000-000000000301",
            "own",
            own_server,
            "2026-01-02T00:00:00Z",
        )
        .await;
        seed_service(
            &db,
            "00000000-0000-0000-0000-000000000302",
            "other",
            other_server,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let response = list_services_for_auth(
            &db,
            &admin_pat_session(owner, vec![other_server.to_string()]),
            10,
            0,
        )
        .await
        .unwrap();

        assert_eq!(response.total, 1);
        assert_eq!(response.services.len(), 1);
        assert_eq!(response.services[0].name, "other");
    }

    #[tokio::test]
    async fn service_history_visibility_rejects_other_owner_service() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let other_server = Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();
        let other_service = "00000000-0000-0000-0000-000000000302";

        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(&db, other_server, other, "other").await;
        seed_service(
            &db,
            other_service,
            "other",
            other_server,
            "2026-01-03T00:00:00Z",
        )
        .await;

        let err = ensure_service_id_visible_to_auth(&db, &member_session(owner), other_service)
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn admin_pat_service_write_respects_server_allowlist() {
        let db = test_db().await;
        let admin = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let allowed_server = Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();
        let blocked_server = Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();

        seed_user(&db, admin, "admin", "admin").await;
        seed_user(&db, other, "other", "member").await;
        seed_agent(&db, allowed_server, admin, "allowed").await;
        seed_agent(&db, blocked_server, other, "blocked").await;

        let mut input = valid_service_input();
        input.server_ids = vec![blocked_server.to_string()];
        input.server_id = Some(blocked_server.to_string());

        let err = ensure_service_input_servers_visible(
            &db,
            &admin_pat_session(admin, vec![allowed_server.to_string()]),
            &input,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn non_global_service_write_rejects_expanding_cover_modes() {
        let db = test_db().await;
        let admin = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let server = Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();

        seed_user(&db, admin, "admin", "admin").await;
        seed_agent(&db, server, admin, "allowed").await;

        let mut input = valid_service_input();
        input.cover_mode = "exclude".into();
        input.exclude_server_ids = vec![server.to_string()];

        let err = ensure_service_input_servers_visible(
            &db,
            &admin_pat_session(admin, vec![server.to_string()]),
            &input,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn non_global_specific_service_requires_explicit_server_ids() {
        let db = test_db().await;
        let admin = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let server = Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();

        seed_user(&db, admin, "admin", "admin").await;
        seed_agent(&db, server, admin, "allowed").await;

        let mut input = valid_service_input();
        input.cover_mode = "specific".into();
        input.server_ids = Vec::new();
        input.server_id = None;
        input.exclude_server_ids = vec![server.to_string()];

        let err = ensure_service_input_servers_visible(
            &db,
            &admin_pat_session(admin, vec![server.to_string()]),
            &input,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn probe_test_requires_service_write_scope() {
        let admin = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let read_only = admin_pat_session_with_scopes(admin, vec!["service:read".into()]);
        let writer = admin_pat_session_with_scopes(admin, vec!["service:write".into()]);

        let err = require_probe_test_scope(&read_only).unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
        assert!(require_probe_test_scope(&writer).is_ok());
    }

    #[test]
    fn service_interval_and_timeout_are_bounded() {
        assert_eq!(normalize_service_interval(None).unwrap(), 60);
        assert_eq!(
            normalize_service_interval(Some(SERVICE_MIN_INTERVAL_SECONDS)).unwrap(),
            SERVICE_MIN_INTERVAL_SECONDS
        );
        assert_eq!(normalize_probe_timeout(None).unwrap(), 10);
        assert_eq!(
            normalize_probe_timeout(Some(SERVICE_MAX_TIMEOUT_SECONDS)).unwrap(),
            SERVICE_MAX_TIMEOUT_SECONDS
        );
        assert!(matches!(
            normalize_service_interval(Some(SERVICE_MIN_INTERVAL_SECONDS - 1)),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            normalize_service_interval(Some(SERVICE_MAX_INTERVAL_SECONDS + 1)),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            normalize_probe_timeout(Some(SERVICE_MAX_TIMEOUT_SECONDS + 1)),
            Err(AppError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn service_request_rejects_oversized_server_and_task_lists() {
        let mut req = CreateServiceRequest {
            name: "svc".into(),
            service_type: "http".into(),
            target: "https://example.com".into(),
            interval_seconds: Some(60),
            timeout_seconds: Some(10),
            enabled: Some(true),
            server_id: None,
            server_ids: (0..=SERVICE_MAX_SERVER_IDS)
                .map(|idx| format!("00000000-0000-0000-0000-{idx:012}"))
                .collect(),
            cover_mode: Some("specific".into()),
            exclude_server_ids: Vec::new(),
            notification_group_id: None,
            failure_task_ids: Vec::new(),
            recovery_task_ids: Vec::new(),
        };
        let err = validate_service_request(req).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        req = CreateServiceRequest {
            name: "svc".into(),
            service_type: "http".into(),
            target: "https://example.com".into(),
            interval_seconds: Some(60),
            timeout_seconds: Some(10),
            enabled: Some(true),
            server_id: None,
            server_ids: Vec::new(),
            cover_mode: Some("local".into()),
            exclude_server_ids: Vec::new(),
            notification_group_id: None,
            failure_task_ids: (0..=SERVICE_MAX_TASK_IDS)
                .map(|idx| format!("task-{idx}"))
                .collect(),
            recovery_task_ids: Vec::new(),
        };
        let err = validate_service_request(req).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    fn member_session(user_id: Uuid) -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: UserId(user_id),
            username: "member".into(),
            role: UserRole::Member,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: vec!["service:read".into()],
            server_ids: None,
            pat_id: None,
        }
    }

    fn admin_pat_session(user_id: Uuid, server_ids: Vec<String>) -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: UserId(user_id),
            username: "admin".into(),
            role: UserRole::Admin,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::PersonalAccessToken,
            scopes: vec!["service:read".into()],
            server_ids: Some(server_ids),
            pat_id: Some("pat".into()),
        }
    }

    fn admin_pat_session_with_scopes(user_id: Uuid, scopes: Vec<String>) -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: UserId(user_id),
            username: "admin".into(),
            role: UserRole::Admin,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::PersonalAccessToken,
            scopes,
            server_ids: None,
            pat_id: Some("pat".into()),
        }
    }

    fn valid_service_input() -> ValidServiceInput {
        ValidServiceInput {
            name: "svc".into(),
            service_type: ProbeType::Http,
            target: "https://example.com".into(),
            interval_seconds: 60,
            timeout_seconds: 10,
            enabled: true,
            server_id: None,
            server_ids: Vec::new(),
            cover_mode: "specific".into(),
            exclude_server_ids: Vec::new(),
            notification_group_id: None,
            failure_task_ids: Vec::new(),
            recovery_task_ids: Vec::new(),
        }
    }

    async fn seed_user(db: &DatabaseBackend, id: Uuid, username: &str, role: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'x', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_agent(db: &DatabaseBackend, id: Uuid, owner: Uuid, name: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at) VALUES (?, ?, 'pk', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(owner.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_service(
        db: &DatabaseBackend,
        id: &str,
        name: &str,
        server_id: Uuid,
        created_at: &str,
    ) {
        seed_service_with_servers(db, id, name, &[server_id], created_at).await;
    }

    async fn seed_service_with_servers(
        db: &DatabaseBackend,
        id: &str,
        name: &str,
        server_ids: &[Uuid],
        created_at: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        let primary = server_ids.first().expect("service must have a server");
        sqlx::query(
            "INSERT INTO services (id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, cover_mode, created_at, updated_at) VALUES (?, ?, 'http', 'https://example.com', 60, 10, 1, ?, 'specific', ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(primary.to_string())
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
        for server_id in server_ids {
            sqlx::query(
                "INSERT INTO service_servers (service_id, server_id, created_at) VALUES (?, ?, ?)",
            )
            .bind(id)
            .bind(server_id.to_string())
            .bind(created_at)
            .execute(pool)
            .await
            .unwrap();
        }
    }
}
