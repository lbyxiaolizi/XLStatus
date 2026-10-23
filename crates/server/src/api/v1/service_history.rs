use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashSet;
use std::net::IpAddr;
use uuid::Uuid;
use xlstatus_shared::AgentId;

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::services::{
    auth_has_global_service_visibility, ensure_service_id_visible_to_auth,
    visible_server_ids_for_auth, SERVICE_MAX_SERVER_IDS,
};
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::{AgentRepository, DatabaseBackend};

const SERVICE_UUID_TEXT_LEN: usize = 36;
const SERVICE_HISTORY_MAX_LIMIT: i64 = 1000;
const SERVICE_HISTORY_MAX_OFFSET: i64 = 10_000;
const SERVICE_HISTORY_MAX_TIMESTAMP_BYTES: usize = 64;
const SERVICE_HISTORY_MAX_RANGE_BYTES: usize = 16;
const SERVICE_RESULT_ERROR_MAX_BYTES: usize = 512;
const SERVICE_RESULT_STATUS_MAX_BYTES: usize = 32;
const SERVICE_RESULT_CERT_FINGERPRINT_MAX_BYTES: usize = 256;
const SERVICE_RESULT_CERT_NOT_AFTER_MAX_BYTES: usize = 64;
const SERVICE_EXCLUDE_SERVER_IDS_JSON_MAX_BYTES: usize = 8 * 1024;
const REDACTED_URL_MARKER: &str = "[redacted]";
const REDACTED_INTERNAL_URL_MARKER: &str = "[redacted-url]";

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub range: Option<String>,
}

impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            limit: default_limit(),
            offset: 0,
            start_time: None,
            end_time: None,
            server_id: None,
            range: None,
        }
    }
}

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Serialize)]
pub struct ServiceHistoryResponse {
    pub results: Vec<ServiceResult>,
    pub total: usize,
    pub uptime_percent: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ServiceResult {
    pub id: String,
    pub service_id: String,
    pub server_id: Option<String>,
    pub status: String,
    pub delay_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error: Option<String>,
    pub cert_fingerprint: Option<String>,
    pub cert_not_after: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ServiceUptimeResponse {
    pub service_id: String,
    pub total_checks: i64,
    pub successful_checks: i64,
    pub uptime_percent: f64,
    pub avg_latency_ms: Option<f64>,
    pub period_start: String,
    pub period_end: String,
}

pub async fn get_service_history(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(service_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<ApiResponse<ServiceHistoryResponse>>, AppError> {
    require_scope(&auth, "service:read")?;
    let service_id = require_canonical_uuid_text(&service_id, "service_id")?;
    ensure_service_history_visible_to_auth(&state.db, &auth, &service_id).await?;
    let limit = normalize_history_limit(query.limit)?;
    let offset = normalize_history_offset(query.offset)?;
    let results =
        list_service_history_for_auth(&state.db, &auth, &service_id, &query, limit, offset).await?;

    let uptime_percent = if results.is_empty() {
        None
    } else {
        let successful = results.iter().filter(|r| r.status == "success").count();
        Some((successful as f64 / results.len() as f64) * 100.0)
    };
    let total = results.len();

    Ok(Json(ApiResponse::success(ServiceHistoryResponse {
        results,
        total,
        uptime_percent,
    })))
}

pub async fn get_service_uptime(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(service_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<ApiResponse<ServiceUptimeResponse>>, AppError> {
    require_scope(&auth, "service:read")?;
    let service_id = require_canonical_uuid_text(&service_id, "service_id")?;
    ensure_service_history_visible_to_auth(&state.db, &auth, &service_id).await?;
    let _ = normalize_history_limit(query.limit)?;
    let _ = normalize_history_offset(query.offset)?;
    if let Some(raw_server_id) = query.server_id.as_deref() {
        ensure_requested_server_visible_to_auth(&state.db, &auth, raw_server_id).await?;
    }
    let (start_time, end_time) = resolve_uptime_window(&query)?;
    let (total_checks, successful_checks, avg_latency) =
        if let Some(raw_server_id) = query.server_id.as_deref() {
            let server_id = require_canonical_uuid_text(raw_server_id, "server_id")?;
            service_uptime_for_auth_with_server_filter(
                &state.db,
                &auth,
                &service_id,
                &start_time,
                &end_time,
                Some(&server_id),
            )
            .await?
        } else {
            service_uptime_for_auth(&state.db, &auth, &service_id, &start_time, &end_time).await?
        };

    let uptime_percent = if total_checks > 0 {
        (successful_checks as f64 / total_checks as f64) * 100.0
    } else {
        0.0
    };

    Ok(Json(ApiResponse::success(ServiceUptimeResponse {
        service_id,
        total_checks,
        successful_checks,
        uptime_percent,
        avg_latency_ms: avg_latency,
        period_start: start_time,
        period_end: end_time,
    })))
}

async fn list_service_history_for_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    service_id: &str,
    query: &HistoryQuery,
    limit: i64,
    offset: i64,
) -> Result<Vec<ServiceResult>, AppError> {
    let service_id = require_canonical_uuid_text(service_id, "service_id")?;
    let service_servers = ensure_service_history_visible_to_auth(db, auth, &service_id).await?;
    let limit = normalize_history_limit(limit)?;
    let offset = normalize_history_offset(offset)?;
    let (start_bound, end_bound) = resolve_optional_history_window(query)?;

    let requested_server_id = match query.server_id.as_deref() {
        Some(raw_server_id) => {
            Some(ensure_requested_server_visible_to_auth(db, auth, raw_server_id).await?)
        }
        None => None,
    };

    let visible_server_ids =
        resolve_query_server_ids(db, auth, &service_servers, requested_server_id.as_deref())
            .await?;
    if let Some(ids) = &visible_server_ids {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
    }

    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let mut sql = String::from(
                r#"
                SELECT id, service_id, server_id, status, delay_ms, status_code, error,
                       cert_fingerprint, cert_not_after, created_at
                FROM service_results
                WHERE service_id = ?
                "#,
            );
            if start_bound.is_some() {
                sql.push_str(" AND created_at >= ?");
            }
            if end_bound.is_some() {
                sql.push_str(" AND created_at <= ?");
            }
            if let Some(ids) = &visible_server_ids {
                sql.push_str(" AND server_id IN (");
                sql.push_str(&sqlite_placeholders(ids.len()));
                sql.push(')');
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");

            let mut q = sqlx::query(&sql).bind(&service_id);
            if let Some((start_text, _)) = &start_bound {
                q = q.bind(start_text);
            }
            if let Some((end_text, _)) = &end_bound {
                q = q.bind(end_text);
            }
            if let Some(ids) = &visible_server_ids {
                for id in ids {
                    q = q.bind(id);
                }
            }
            q.bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
                .map_err(db_err)
                .and_then(|rows| {
                    rows.into_iter()
                        .map(service_result_from_sqlite_row)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(db_err)
                })
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let mut sql = String::from(
                r#"
                SELECT id::text AS id, service_id::text AS service_id, server_id::text AS server_id,
                       status, delay_ms, status_code, error, cert_fingerprint,
                       cert_not_after::text AS cert_not_after, created_at::text AS created_at
                FROM service_results
                WHERE service_id = $1
                "#,
            );
            let mut bind_index = 2;
            if start_bound.is_some() {
                sql.push_str(&format!(" AND created_at >= ${bind_index}"));
                bind_index += 1;
            }
            if end_bound.is_some() {
                sql.push_str(&format!(" AND created_at <= ${bind_index}"));
                bind_index += 1;
            }
            if visible_server_ids.is_some() {
                sql.push_str(&format!(" AND server_id = ANY(${bind_index}::uuid[])"));
                bind_index += 1;
            }
            sql.push_str(&format!(
                " ORDER BY created_at DESC LIMIT ${bind_index} OFFSET ${}",
                bind_index + 1
            ));

            let sid = Uuid::parse_str(&service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let mut q = sqlx::query(&sql).bind(sid);
            if let Some((_, start_dt)) = &start_bound {
                q = q.bind(*start_dt);
            }
            if let Some((_, end_dt)) = &end_bound {
                q = q.bind(*end_dt);
            }
            if let Some(ids) = &visible_server_ids {
                q = q.bind(parse_uuid_ids(ids)?);
            }
            q.bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
                .map_err(db_err)
                .and_then(|rows| {
                    rows.into_iter()
                        .map(service_result_from_postgres_row)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(db_err)
                })
        }
    }
}

async fn service_uptime_for_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    service_id: &str,
    start_time: &str,
    end_time: &str,
) -> Result<(i64, i64, Option<f64>), AppError> {
    service_uptime_for_auth_with_server_filter(db, auth, service_id, start_time, end_time, None)
        .await
}

async fn service_uptime_for_auth_with_server_filter(
    db: &crate::db::Db,
    auth: &AuthSession,
    service_id: &str,
    start_time: &str,
    end_time: &str,
    requested_server_id: Option<&str>,
) -> Result<(i64, i64, Option<f64>), AppError> {
    let service_id = require_canonical_uuid_text(service_id, "service_id")?;
    let service_servers = ensure_service_history_visible_to_auth(db, auth, &service_id).await?;
    let (start_text, start_dt) = parse_history_timestamp(start_time, "start_time")?;
    let (end_text, end_dt) = parse_history_timestamp(end_time, "end_time")?;
    if start_dt > end_dt {
        return Err(AppError::BadRequest(
            "start_time must not be after end_time".into(),
        ));
    }

    let validated_server_id = match requested_server_id {
        Some(raw_server_id) => {
            Some(ensure_requested_server_visible_to_auth(db, auth, raw_server_id).await?)
        }
        None => None,
    };

    let visible_server_ids =
        resolve_query_server_ids(db, auth, &service_servers, validated_server_id.as_deref())
            .await?;
    if let Some(ids) = &visible_server_ids {
        if ids.is_empty() {
            return Ok((0, 0, None));
        }
    }

    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let mut sql = String::from(
                r#"
                SELECT
                    COUNT(*) AS total_checks,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS successful_checks,
                    AVG(CASE WHEN status = 'success' THEN delay_ms ELSE NULL END) AS avg_latency
                FROM service_results
                WHERE service_id = ?
                  AND created_at >= ?
                  AND created_at <= ?
                "#,
            );
            if let Some(ids) = &visible_server_ids {
                sql.push_str(" AND server_id IN (");
                sql.push_str(&sqlite_placeholders(ids.len()));
                sql.push(')');
            }
            let mut q = sqlx::query(&sql)
                .bind(&service_id)
                .bind(&start_text)
                .bind(&end_text);
            if let Some(ids) = &visible_server_ids {
                for id in ids {
                    q = q.bind(id);
                }
            }
            let row = q.fetch_one(pool).await.map_err(db_err)?;
            // Explicit Option decodes: NULL (empty window) maps to None/0,
            // while a column-type mismatch surfaces as an error instead of
            // being silently swallowed into a null latency.
            Ok((
                row.try_get::<Option<i64>, _>("total_checks")
                    .map_err(db_err)?
                    .unwrap_or(0),
                row.try_get::<Option<i64>, _>("successful_checks")
                    .map_err(db_err)?
                    .unwrap_or(0),
                row.try_get::<Option<f64>, _>("avg_latency")
                    .map_err(db_err)?,
            ))
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let sid = Uuid::parse_str(&service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let mut sql = String::from(
                r#"
                SELECT
                    COUNT(*) AS total_checks,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS successful_checks,
                    -- ::float8 because AVG(int4) yields NUMERIC, which this
                    -- build cannot decode into f64 (no decimal feature); the
                    -- .ok() below would silently turn that into a null latency.
                    AVG(CASE WHEN status = 'success' THEN delay_ms ELSE NULL END)::float8 AS avg_latency
                FROM service_results
                WHERE service_id = $1
                  AND created_at >= $2
                  AND created_at <= $3
                "#,
            );
            if visible_server_ids.is_some() {
                sql.push_str(" AND server_id = ANY($4::uuid[])");
            }
            let mut q = sqlx::query(&sql).bind(sid).bind(start_dt).bind(end_dt);
            if let Some(ids) = &visible_server_ids {
                q = q.bind(parse_uuid_ids(ids)?);
            }
            let row = q.fetch_one(pool).await.map_err(db_err)?;
            // Explicit Option decodes: NULL (empty window) maps to None/0,
            // while a column-type mismatch surfaces as an error instead of
            // being silently swallowed into a null latency.
            Ok((
                row.try_get::<Option<i64>, _>("total_checks")
                    .map_err(db_err)?
                    .unwrap_or(0),
                row.try_get::<Option<i64>, _>("successful_checks")
                    .map_err(db_err)?
                    .unwrap_or(0),
                row.try_get::<Option<f64>, _>("avg_latency")
                    .map_err(db_err)?,
            ))
        }
    }
}

struct ServiceSecurityMeta {
    owner_user_id: Option<String>,
    cover_mode: String,
    legacy_server_id: Option<String>,
    exclude_server_ids_json: Option<String>,
    assigned_server_ids: Vec<String>,
}

async fn load_service_security_meta(
    db: &DatabaseBackend,
    service_id: &str,
) -> Result<ServiceSecurityMeta, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query(
                r#"
                SELECT owner_user_id, COALESCE(cover_mode, 'local') AS cover_mode,
                       server_id, exclude_server_ids_json
                FROM services
                WHERE id = ?
                "#,
            )
            .bind(service_id)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?
            .ok_or_else(|| AppError::NotFound("service not found".into()))?;

            let server_rows: Vec<(String,)> = sqlx::query_as(
                "SELECT server_id FROM service_servers WHERE service_id = ? ORDER BY created_at ASC, server_id ASC LIMIT ?",
            )
            .bind(service_id)
            .bind((SERVICE_MAX_SERVER_IDS + 1) as i64)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;

            Ok(ServiceSecurityMeta {
                owner_user_id: row.try_get("owner_user_id").map_err(db_err)?,
                cover_mode: row.try_get("cover_mode").map_err(db_err)?,
                legacy_server_id: row.try_get("server_id").map_err(db_err)?,
                exclude_server_ids_json: row.try_get("exclude_server_ids_json").map_err(db_err)?,
                assigned_server_ids: server_rows.into_iter().map(|(id,)| id).collect(),
            })
        }
        DatabaseBackend::Postgres(pool) => {
            let sid = Uuid::parse_str(service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let row = sqlx::query(
                r#"
                SELECT owner_user_id::text AS owner_user_id,
                       COALESCE(cover_mode, 'local') AS cover_mode,
                       server_id::text AS server_id,
                       exclude_server_ids_json
                FROM services
                WHERE id = $1
                "#,
            )
            .bind(sid)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?
            .ok_or_else(|| AppError::NotFound("service not found".into()))?;

            let server_rows: Vec<(String,)> = sqlx::query_as(
                "SELECT server_id::text FROM service_servers WHERE service_id = $1 ORDER BY created_at ASC, server_id ASC LIMIT $2",
            )
            .bind(sid)
            .bind((SERVICE_MAX_SERVER_IDS + 1) as i64)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;

            Ok(ServiceSecurityMeta {
                owner_user_id: row.try_get("owner_user_id").map_err(db_err)?,
                cover_mode: row.try_get("cover_mode").map_err(db_err)?,
                legacy_server_id: row.try_get("server_id").map_err(db_err)?,
                exclude_server_ids_json: row.try_get("exclude_server_ids_json").map_err(db_err)?,
                assigned_server_ids: server_rows.into_iter().map(|(id,)| id).collect(),
            })
        }
    }
}

pub(crate) async fn ensure_service_history_visible_to_auth(
    db: &DatabaseBackend,
    auth: &AuthSession,
    service_id: &str,
) -> Result<Vec<String>, AppError> {
    let service_id = require_canonical_uuid_text(service_id, "service_id")?;
    let meta = load_service_security_meta(db, &service_id).await?;
    ensure_service_id_visible_to_auth(db, auth, &service_id).await?;

    if meta.assigned_server_ids.len() > SERVICE_MAX_SERVER_IDS {
        return Err(AppError::Forbidden("service not in scope".into()));
    }

    let mut target_server_ids = Vec::new();
    for raw_id in &meta.assigned_server_ids {
        let canonical = require_canonical_uuid_text(raw_id, "server_id")
            .map_err(|_| AppError::Forbidden("service not in scope".into()))?;
        if !target_server_ids
            .iter()
            .any(|existing| existing == &canonical)
        {
            target_server_ids.push(canonical);
        }
    }
    if let Some(legacy_id) = meta.legacy_server_id.as_deref() {
        let trimmed = legacy_id.trim();
        if !trimmed.is_empty() {
            let canonical = require_canonical_uuid_text(trimmed, "server_id")
                .map_err(|_| AppError::Forbidden("service not in scope".into()))?;
            if !target_server_ids
                .iter()
                .any(|existing| existing == &canonical)
            {
                target_server_ids.push(canonical);
            }
        }
    }

    if auth_has_global_service_visibility(auth) {
        return Ok(target_server_ids);
    }

    if !auth.role.is_admin() {
        let owner_user_id = meta
            .owner_user_id
            .as_deref()
            .and_then(|raw| require_canonical_uuid_text(raw.trim(), "owner_user_id").ok())
            .ok_or_else(|| AppError::Forbidden("service not in scope".into()))?;
        if owner_user_id != auth.user_id.0.to_string() {
            return Err(AppError::Forbidden("service not in scope".into()));
        }
    }

    if !meta.cover_mode.trim().eq_ignore_ascii_case("specific") {
        return Err(AppError::Forbidden("service not in scope".into()));
    }

    if !exclude_server_ids_json_is_empty(meta.exclude_server_ids_json.as_deref()) {
        return Err(AppError::Forbidden("service not in scope".into()));
    }

    if target_server_ids.is_empty() {
        return Err(AppError::Forbidden("service not in scope".into()));
    }

    let pat_allowlist = parse_pat_allowlist(auth)?;
    let agent_repo = AgentRepository::new(db.clone());
    for server_id in &target_server_ids {
        if let Some(allowlist) = &pat_allowlist {
            if !allowlist.contains(server_id) {
                return Err(AppError::Forbidden("service not in scope".into()));
            }
        }
        let parsed = Uuid::parse_str(server_id)
            .map_err(|_| AppError::Forbidden("service not in scope".into()))?;
        let agent = agent_repo
            .find_by_id(AgentId(parsed))
            .await?
            .ok_or_else(|| AppError::Forbidden("service not in scope".into()))?;
        if agent.revoked_at.is_some() {
            return Err(AppError::Forbidden("service not in scope".into()));
        }
        if !auth.role.is_admin() && agent.owner_user_id != auth.user_id {
            return Err(AppError::Forbidden("service not in scope".into()));
        }
    }

    Ok(target_server_ids)
}

async fn ensure_requested_server_visible_to_auth(
    db: &DatabaseBackend,
    auth: &AuthSession,
    raw_server_id: &str,
) -> Result<String, AppError> {
    let server_id = require_canonical_uuid_text(raw_server_id, "server_id")?;
    let pat_allowlist = parse_pat_allowlist(auth)?;
    if let Some(allowlist) = &pat_allowlist {
        if !allowlist.contains(&server_id) {
            return Err(AppError::Forbidden("server not in scope".into()));
        }
    }
    let parsed = Uuid::parse_str(&server_id)
        .map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))?;
    let agent = AgentRepository::new(db.clone())
        .find_by_id(AgentId(parsed))
        .await?
        .ok_or_else(|| {
            if auth_has_global_service_visibility(auth) {
                AppError::NotFound("server not found".into())
            } else {
                AppError::Forbidden("server not in scope".into())
            }
        })?;
    if agent.revoked_at.is_some() {
        return Err(AppError::Forbidden("server not in scope".into()));
    }
    if !auth.role.is_admin() && agent.owner_user_id != auth.user_id {
        return Err(AppError::Forbidden("server not in scope".into()));
    }
    Ok(server_id)
}

async fn resolve_query_server_ids(
    db: &DatabaseBackend,
    auth: &AuthSession,
    service_server_ids: &[String],
    requested_server_id: Option<&str>,
) -> Result<Option<Vec<String>>, AppError> {
    if auth_has_global_service_visibility(auth) {
        return Ok(requested_server_id.map(|id| vec![id.to_string()]));
    }
    let visible = visible_server_ids_for_auth(db, auth).await?;
    let mut allowed: Vec<String> = service_server_ids
        .iter()
        .filter(|server_id| visible.iter().any(|vis| vis == *server_id))
        .cloned()
        .collect();
    if let Some(req_id) = requested_server_id {
        if !allowed.iter().any(|id| id == req_id) {
            return Err(AppError::Forbidden("server not in scope".into()));
        }
        allowed.retain(|id| id == req_id);
    }
    Ok(Some(allowed))
}

fn exclude_server_ids_json_is_empty(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return true;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.len() > SERVICE_EXCLUDE_SERVER_IDS_JSON_MAX_BYTES {
        return false;
    }
    serde_json::from_str::<Vec<String>>(trimmed)
        .map(|items| items.iter().all(|item| item.trim().is_empty()))
        .unwrap_or(false)
}

fn parse_pat_allowlist(auth: &AuthSession) -> Result<Option<HashSet<String>>, AppError> {
    let Some(raw_ids) = auth.server_ids.as_ref() else {
        return Ok(None);
    };
    if raw_ids.is_empty() || raw_ids.len() > SERVICE_MAX_SERVER_IDS {
        return Err(AppError::Forbidden("service not in scope".into()));
    }
    let mut set = HashSet::with_capacity(raw_ids.len());
    for raw in raw_ids {
        let canonical = require_canonical_uuid_text(raw.trim(), "server_id")
            .map_err(|_| AppError::Forbidden("service not in scope".into()))?;
        set.insert(canonical);
    }
    Ok(Some(set))
}

fn normalize_history_limit(limit: i64) -> Result<i64, AppError> {
    if limit <= 0 || limit > SERVICE_HISTORY_MAX_LIMIT {
        return Err(AppError::BadRequest(format!(
            "limit must be between 1 and {SERVICE_HISTORY_MAX_LIMIT}"
        )));
    }
    Ok(limit)
}

fn normalize_history_offset(offset: i64) -> Result<i64, AppError> {
    if !(0..=SERVICE_HISTORY_MAX_OFFSET).contains(&offset) {
        return Err(AppError::BadRequest(format!(
            "offset must be between 0 and {SERVICE_HISTORY_MAX_OFFSET}"
        )));
    }
    Ok(offset)
}

fn parse_history_range(range: Option<&str>) -> Result<Option<Duration>, AppError> {
    let Some(raw) = range else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > SERVICE_HISTORY_MAX_RANGE_BYTES || trimmed.chars().any(|ch| ch.is_control())
    {
        return Err(AppError::BadRequest("invalid range".into()));
    }
    let duration = match trimmed.to_ascii_lowercase().as_str() {
        "1h" => Duration::hours(1),
        "6h" => Duration::hours(6),
        "12h" => Duration::hours(12),
        "24h" | "1d" => Duration::days(1),
        "7d" => Duration::days(7),
        "30d" => Duration::days(30),
        _ => {
            return Err(AppError::BadRequest(
                "range must be 1h, 6h, 12h, 24h, 1d, 7d, or 30d".into(),
            ))
        }
    };
    Ok(Some(duration))
}

fn parse_history_timestamp(value: &str, field: &str) -> Result<(String, DateTime<Utc>), AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > SERVICE_HISTORY_MAX_TIMESTAMP_BYTES
        || trimmed.chars().any(|ch| ch.is_control())
    {
        return Err(AppError::BadRequest(format!("invalid {field}")));
    }
    let parsed = DateTime::parse_from_rfc3339(trimmed)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| AppError::BadRequest(format!("{field} must be a valid RFC3339 timestamp")))?;
    Ok((trimmed.to_string(), parsed))
}

fn resolve_optional_history_window(
    query: &HistoryQuery,
) -> Result<
    (
        Option<(String, DateTime<Utc>)>,
        Option<(String, DateTime<Utc>)>,
    ),
    AppError,
> {
    let range_duration = parse_history_range(query.range.as_deref())?;
    let end_bound = query
        .end_time
        .as_deref()
        .map(|raw| parse_history_timestamp(raw, "end_time"))
        .transpose()?;
    let start_bound = match query.start_time.as_deref() {
        Some(raw) => Some(parse_history_timestamp(raw, "start_time")?),
        None => range_duration.map(|dur| {
            let anchor = end_bound
                .as_ref()
                .map(|(_, dt)| *dt)
                .unwrap_or_else(Utc::now);
            let start_dt = anchor - dur;
            (start_dt.to_rfc3339(), start_dt)
        }),
    };
    if let (Some((_, start_dt)), Some((_, end_dt))) = (&start_bound, &end_bound) {
        if start_dt > end_dt {
            return Err(AppError::BadRequest(
                "start_time must not be after end_time".into(),
            ));
        }
    }
    Ok((start_bound, end_bound))
}

fn resolve_uptime_window(query: &HistoryQuery) -> Result<(String, String), AppError> {
    let range_duration =
        parse_history_range(query.range.as_deref())?.unwrap_or_else(|| Duration::days(30));
    let (end_text, end_dt) = match query.end_time.as_deref() {
        Some(raw) => parse_history_timestamp(raw, "end_time")?,
        None => {
            let now = Utc::now();
            (now.to_rfc3339(), now)
        }
    };
    let (start_text, start_dt) = match query.start_time.as_deref() {
        Some(raw) => parse_history_timestamp(raw, "start_time")?,
        None => {
            let start = end_dt - range_duration;
            (start.to_rfc3339(), start)
        }
    };
    if start_dt > end_dt {
        return Err(AppError::BadRequest(
            "start_time must not be after end_time".into(),
        ));
    }
    Ok((start_text, end_text))
}

fn require_canonical_uuid_text(value: &str, field: &str) -> Result<String, AppError> {
    if value.len() != SERVICE_UUID_TEXT_LEN {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    let parsed = Uuid::parse_str(value)
        .map_err(|_| AppError::BadRequest(format!("{field} must be a canonical UUID")))?;
    if parsed.to_string() != value {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    Ok(value.to_string())
}

fn sqlite_placeholders(len: usize) -> String {
    std::iter::repeat_n("?", len).collect::<Vec<_>>().join(", ")
}

fn parse_uuid_ids(ids: &[String]) -> Result<Vec<Uuid>, AppError> {
    ids.iter()
        .map(|id| {
            Uuid::parse_str(id).map_err(|e| AppError::BadRequest(format!("invalid server_id: {e}")))
        })
        .collect()
}

pub(crate) fn sanitize_service_result_error(error: Option<String>) -> Option<String> {
    let raw = error?;
    let redacted = redact_service_error_urls(&raw);
    let mut cleaned = String::with_capacity(redacted.len().min(SERVICE_RESULT_ERROR_MAX_BYTES));
    for ch in redacted.chars() {
        let normalized = match ch {
            '\r' | '\n' | '\t' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if cleaned.len() + normalized.len_utf8() > SERVICE_RESULT_ERROR_MAX_BYTES {
            break;
        }
        cleaned.push(normalized);
    }
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn redact_service_error_urls(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while rest
        .find("http://")
        .or_else(|| rest.find("https://"))
        .is_some()
    {
        let http_pos = rest.find("http://");
        let https_pos = rest.find("https://");
        let idx = match (http_pos, https_pos) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };
        out.push_str(&rest[..idx]);
        let after = &rest[idx..];
        let end = after
            .find(|ch: char| {
                ch.is_whitespace() || matches!(ch, ')' | '}' | '>' | '"' | '\'' | ',' | ';')
            })
            .unwrap_or(after.len());
        let candidate = &after[..end];
        out.push_str(&redact_single_service_error_url(candidate));
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn redact_single_service_error_url(candidate: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(candidate) else {
        return REDACTED_INTERNAL_URL_MARKER.to_string();
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return REDACTED_INTERNAL_URL_MARKER.to_string();
    }
    let Some(host) = parsed.host_str() else {
        return REDACTED_INTERNAL_URL_MARKER.to_string();
    };
    if !is_public_service_error_host(host) {
        return REDACTED_INTERNAL_URL_MARKER.to_string();
    }
    let clean_host = host.trim_start_matches('[').trim_end_matches(']');
    let formatted_host = if clean_host.contains(':') {
        format!("[{clean_host}]")
    } else {
        clean_host.to_string()
    };
    let authority = match parsed.port() {
        Some(port) => format!("{formatted_host}:{port}"),
        None => formatted_host,
    };
    let has_credentials = !parsed.username().is_empty() || parsed.password().is_some();
    if !has_credentials
        && parsed.path() == "/"
        && parsed.query().is_none()
        && parsed.fragment().is_none()
    {
        format!("{}://{authority}/", parsed.scheme())
    } else {
        format!("{}://{authority}/{REDACTED_URL_MARKER}", parsed.scheme())
    }
}

fn is_public_service_error_host(host: &str) -> bool {
    let normalized = host
        .trim_end_matches('.')
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if normalized.is_empty()
        || normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized == "metadata.google.internal"
        || normalized.ends_with(".internal")
        || normalized.ends_with(".local")
    {
        return false;
    }
    if let Ok(ip) = normalized.parse::<IpAddr>() {
        return is_public_service_error_ip(ip);
    }
    true
}

fn is_public_service_error_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            let shared_cgnat = octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000;
            let benchmark = octets[0] == 198 && (octets[1] == 18 || octets[1] == 19);
            let ietf_protocol = octets[0] == 192 && octets[1] == 0 && octets[2] == 0;
            let reserved = octets[0] >= 240;
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || shared_cgnat
                || benchmark
                || ietf_protocol
                || reserved)
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public_service_error_ip(IpAddr::V4(mapped));
            }
            let segments = v6.segments();
            let discard_only = segments[0] == 0x0100 && segments[1..4] == [0, 0, 0];
            let documentation = segments[0] == 0x2001 && segments[1] == 0x0db8;
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || discard_only
                || documentation)
        }
    }
}

fn sanitize_bounded_result_field(value: String, max_bytes: usize) -> String {
    let mut out = String::with_capacity(value.len().min(max_bytes));
    for ch in value.chars() {
        if ch.is_control() {
            continue;
        }
        if out.len() + ch.len_utf8() > max_bytes {
            break;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn sanitize_optional_result_field(value: Option<String>, max_bytes: usize) -> Option<String> {
    value
        .map(|raw| sanitize_bounded_result_field(raw, max_bytes))
        .filter(|cleaned| !cleaned.is_empty())
}

pub(crate) fn service_result_from_sqlite_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<ServiceResult, sqlx::Error> {
    Ok(ServiceResult {
        id: row.try_get("id")?,
        service_id: row.try_get("service_id")?,
        server_id: row.try_get("server_id")?,
        status: sanitize_bounded_result_field(
            row.try_get("status")?,
            SERVICE_RESULT_STATUS_MAX_BYTES,
        ),
        delay_ms: row.try_get("delay_ms")?,
        status_code: row.try_get("status_code")?,
        error: sanitize_service_result_error(row.try_get("error")?),
        cert_fingerprint: sanitize_optional_result_field(
            row.try_get("cert_fingerprint")?,
            SERVICE_RESULT_CERT_FINGERPRINT_MAX_BYTES,
        ),
        cert_not_after: sanitize_optional_result_field(
            row.try_get("cert_not_after")?,
            SERVICE_RESULT_CERT_NOT_AFTER_MAX_BYTES,
        ),
        created_at: row.try_get("created_at")?,
    })
}

pub(crate) fn service_result_from_postgres_row(
    row: sqlx::postgres::PgRow,
) -> Result<ServiceResult, sqlx::Error> {
    Ok(ServiceResult {
        id: row.try_get("id")?,
        service_id: row.try_get("service_id")?,
        server_id: row.try_get("server_id")?,
        status: sanitize_bounded_result_field(
            row.try_get("status")?,
            SERVICE_RESULT_STATUS_MAX_BYTES,
        ),
        delay_ms: row.try_get("delay_ms")?,
        status_code: row.try_get("status_code")?,
        error: sanitize_service_result_error(row.try_get("error")?),
        cert_fingerprint: sanitize_optional_result_field(
            row.try_get("cert_fingerprint")?,
            SERVICE_RESULT_CERT_FINGERPRINT_MAX_BYTES,
        ),
        cert_not_after: sanitize_optional_result_field(
            row.try_get("cert_not_after")?,
            SERVICE_RESULT_CERT_NOT_AFTER_MAX_BYTES,
        ),
        created_at: row.try_get("created_at")?,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::middleware::AuthKind;
    use crate::db::DatabaseBackend;
    use xlstatus_shared::{UserId, UserRole};

    #[tokio::test]
    async fn scoped_pat_service_history_filters_server_rows_before_limit() {
        let db = test_db().await;
        let admin = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let allowed_server = Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();
        let blocked_server = Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();
        let service_id = "00000000-0000-0000-0000-000000000301";

        seed_user(&db, admin, "admin", "admin").await;
        seed_agent(&db, allowed_server, admin, "allowed").await;
        seed_agent(&db, blocked_server, admin, "blocked").await;
        seed_service(&db, service_id, admin, allowed_server).await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000401",
            service_id,
            blocked_server,
            "failure",
            Some(900),
            "2026-01-03T00:00:00Z",
        )
        .await;
        seed_service_result(
            &db,
            "00000000-0000-0000-0000-000000000402",
            service_id,
            allowed_server,
            "success",
            Some(20),
            "2026-01-02T00:00:00Z",
        )
        .await;

        let auth = admin_pat_session(admin, vec![allowed_server.to_string()]);
        ensure_service_id_visible_to_auth(&db, &auth, service_id)
            .await
            .unwrap();
        let history = list_service_history_for_auth(
            &db,
            &auth,
            service_id,
            &HistoryQuery {
                limit: 1,
                offset: 0,
                start_time: None,
                end_time: None,
                server_id: None,
                range: None,
            },
            1,
            0,
        )
        .await
        .unwrap();

        assert_eq!(history.len(), 1);
        let allowed_server_id = allowed_server.to_string();
        assert_eq!(
            history[0].server_id.as_deref(),
            Some(allowed_server_id.as_str())
        );
        assert_eq!(history[0].status, "success");

        let (total_checks, successful_checks, avg_latency) = service_uptime_for_auth(
            &db,
            &auth,
            service_id,
            "2026-01-01T00:00:00Z",
            "2026-01-04T00:00:00Z",
        )
        .await
        .unwrap();
        assert_eq!(total_checks, 1);
        assert_eq!(successful_checks, 1);
        assert_eq!(avg_latency, Some(20.0));
    }

    #[tokio::test]
    async fn member_service_history_enforces_service_and_server_owner_isolation() {
        let db = test_db().await;
        let owner_a = Uuid::parse_str("00000000-0000-0000-0000-000000000011").unwrap();
        let owner_b = Uuid::parse_str("00000000-0000-0000-0000-000000000012").unwrap();
        let server_a = Uuid::parse_str("00000000-0000-0000-0000-000000000111").unwrap();
        let server_b = Uuid::parse_str("00000000-0000-0000-0000-000000000112").unwrap();
        let foreign_service = "00000000-0000-0000-0000-000000000311";
        let mismatched_owner_service = "00000000-0000-0000-0000-000000000312";
        let own_service = "00000000-0000-0000-0000-000000000313";

        seed_user(&db, owner_a, "owner_a", "member").await;
        seed_user(&db, owner_b, "owner_b", "member").await;
        seed_agent(&db, server_a, owner_a, "server-a").await;
        seed_agent(&db, server_b, owner_b, "server-b").await;

        seed_service(&db, foreign_service, owner_b, server_b).await;
        // Service row owned by owner_b even though it references server_a
        seed_service(&db, mismatched_owner_service, owner_b, server_a).await;
        seed_service(&db, own_service, owner_a, server_a).await;

        let member_a = member_session(owner_a);
        assert!(matches!(
            ensure_service_history_visible_to_auth(&db, &member_a, foreign_service).await,
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            ensure_service_history_visible_to_auth(&db, &member_a, mismatched_owner_service).await,
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            list_service_history_for_auth(
                &db,
                &member_a,
                own_service,
                &HistoryQuery {
                    server_id: Some(server_b.to_string()),
                    ..HistoryQuery::default()
                },
                10,
                0,
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
    }

    #[tokio::test]
    async fn scoped_pat_rejects_non_specific_cover_mode_exclude_list_and_unlisted_servers() {
        let db = test_db().await;
        let admin = Uuid::parse_str("00000000-0000-0000-0000-000000000021").unwrap();
        let allowed_server = Uuid::parse_str("00000000-0000-0000-0000-000000000121").unwrap();
        let blocked_server = Uuid::parse_str("00000000-0000-0000-0000-000000000122").unwrap();
        let svc_all = "00000000-0000-0000-0000-000000000321";
        let svc_with_exclude = "00000000-0000-0000-0000-000000000322";
        let svc_mixed_servers = "00000000-0000-0000-0000-000000000323";

        seed_user(&db, admin, "admin", "admin").await;
        seed_agent(&db, allowed_server, admin, "allowed").await;
        seed_agent(&db, blocked_server, admin, "blocked").await;

        seed_service(&db, svc_all, admin, allowed_server).await;
        seed_service(&db, svc_with_exclude, admin, allowed_server).await;
        seed_service(&db, svc_mixed_servers, admin, allowed_server).await;

        let DatabaseBackend::Sqlite(pool) = &db else {
            unreachable!();
        };
        sqlx::query("UPDATE services SET cover_mode = 'all' WHERE id = ?")
            .bind(svc_all)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE services SET exclude_server_ids_json = ? WHERE id = ?")
            .bind(serde_json::json!([blocked_server.to_string()]).to_string())
            .bind(svc_with_exclude)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO service_servers (service_id, server_id, created_at) VALUES (?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(svc_mixed_servers)
        .bind(blocked_server.to_string())
        .execute(pool)
        .await
        .unwrap();

        let pat = admin_pat_session(admin, vec![allowed_server.to_string()]);
        for denied_service in [svc_all, svc_with_exclude, svc_mixed_servers] {
            assert!(
                matches!(
                    ensure_service_history_visible_to_auth(&db, &pat, denied_service).await,
                    Err(AppError::Forbidden(_))
                ),
                "expected {denied_service} to be rejected for scoped PAT"
            );
        }
    }

    #[test]
    fn service_history_validates_canonical_uuids_query_bounds_and_sanitizes_errors() {
        let canonical = "00000000-0000-0000-0000-000000000301";
        assert_eq!(
            require_canonical_uuid_text(canonical, "service_id").unwrap(),
            canonical
        );
        for invalid in [
            "not-a-uuid",
            "00000000000000000000000000000301",
            "00000000-0000-0000-0000-00000000030A",
            " 00000000-0000-0000-0000-000000000301 ",
        ] {
            assert!(matches!(
                require_canonical_uuid_text(invalid, "service_id"),
                Err(AppError::BadRequest(_))
            ));
        }

        assert!(normalize_history_limit(0).is_err());
        assert!(normalize_history_limit(SERVICE_HISTORY_MAX_LIMIT + 1).is_err());
        assert_eq!(normalize_history_limit(100).unwrap(), 100);
        assert!(normalize_history_offset(-1).is_err());
        assert!(normalize_history_offset(SERVICE_HISTORY_MAX_OFFSET + 1).is_err());

        assert!(parse_history_range(Some("24h")).is_ok());
        assert!(parse_history_range(Some("invalid-range")).is_err());
        assert!(parse_history_timestamp("not-a-timestamp", "start_time").is_err());

        let sanitized_internal = sanitize_service_result_error(Some(
            "probe failed: Get \"http://127.0.0.1:8080/internal/secret?token=abc\": connection refused\r\n\x07"
                .into(),
        ))
        .unwrap();
        assert!(!sanitized_internal.contains("127.0.0.1:8080/internal"));
        assert!(!sanitized_internal.contains("token=abc"));
        assert!(!sanitized_internal.chars().any(|ch| ch.is_control()));
        assert!(sanitized_internal.contains(REDACTED_INTERNAL_URL_MARKER));

        let sanitized_public = sanitize_service_result_error(Some(
            "request to https://user:pass@hooks.example.com/v1/probe?api_key=SECRET123 failed"
                .into(),
        ))
        .unwrap();
        assert!(!sanitized_public.contains("SECRET123"));
        assert!(!sanitized_public.contains("user:pass"));
        assert!(sanitized_public.contains("https://hooks.example.com/[redacted]"));

        let oversized = sanitize_service_result_error(Some("e".repeat(2048))).unwrap();
        assert_eq!(oversized.len(), SERVICE_RESULT_ERROR_MAX_BYTES);
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

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
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

    async fn seed_service(db: &DatabaseBackend, id: &str, owner: Uuid, server_id: Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO services (id, owner_user_id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, cover_mode, created_at, updated_at) VALUES (?, ?, 'svc', 'http', 'https://example.com', 60, 10, 1, ?, 'specific', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(server_id.to_string())
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO service_servers (service_id, server_id, created_at) VALUES (?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(server_id.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_service_result(
        db: &DatabaseBackend,
        id: &str,
        service_id: &str,
        server_id: Uuid,
        status: &str,
        delay_ms: Option<i32>,
        created_at: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO service_results (id, service_id, server_id, status, delay_ms, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(service_id)
        .bind(server_id.to_string())
        .bind(status)
        .bind(delay_ms)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }
}
