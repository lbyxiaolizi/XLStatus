use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
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
    let limit = query.limit.clamp(1, 1000);
    let offset = query.offset.max(0);
    let results: Vec<ServiceResult> = match &state.db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let mut sql = String::from(
                r#"
                SELECT id, service_id, server_id, status, delay_ms, status_code, error,
                       cert_fingerprint, cert_not_after, created_at
                FROM service_results
                WHERE service_id = ?
                "#,
            );
            let mut params = vec![service_id.clone()];
            if let Some(start) = &query.start_time {
                sql.push_str(" AND created_at >= ?");
                params.push(start.clone());
            }
            if let Some(end) = &query.end_time {
                sql.push_str(" AND created_at <= ?");
                params.push(end.clone());
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");

            let mut q = sqlx::query(&sql);
            for param in &params {
                q = q.bind(param);
            }
            q.bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
                .map_err(db_err)?
                .into_iter()
                .map(service_result_from_sqlite_row)
                .collect()
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
            if query.start_time.is_some() {
                sql.push_str(&format!(" AND created_at >= ${bind_index}"));
                bind_index += 1;
            }
            if query.end_time.is_some() {
                sql.push_str(&format!(" AND created_at <= ${bind_index}"));
                bind_index += 1;
            }
            sql.push_str(&format!(
                " ORDER BY created_at DESC LIMIT ${bind_index} OFFSET ${}",
                bind_index + 1
            ));

            let sid = Uuid::parse_str(&service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let mut q = sqlx::query(&sql).bind(sid);
            if let Some(start) = &query.start_time {
                q = q.bind(start);
            }
            if let Some(end) = &query.end_time {
                q = q.bind(end);
            }
            q.bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
                .map_err(db_err)?
                .into_iter()
                .map(service_result_from_postgres_row)
                .collect()
        }
    };

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
    let start_time = query
        .start_time
        .unwrap_or_else(|| (Utc::now() - Duration::days(30)).to_rfc3339());
    let end_time = query.end_time.unwrap_or_else(|| Utc::now().to_rfc3339());

    let (total_checks, successful_checks, avg_latency) = match &state.db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query(
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
            )
            .bind(&service_id)
            .bind(&start_time)
            .bind(&end_time)
            .fetch_one(pool)
            .await
            .map_err(db_err)?;
            (
                row.try_get("total_checks").unwrap_or(0),
                row.try_get("successful_checks").unwrap_or(0),
                row.try_get("avg_latency").ok(),
            )
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let sid = Uuid::parse_str(&service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let row = sqlx::query(
                r#"
                SELECT
                    COUNT(*) AS total_checks,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS successful_checks,
                    AVG(CASE WHEN status = 'success' THEN delay_ms ELSE NULL END) AS avg_latency
                FROM service_results
                WHERE service_id = $1
                  AND created_at >= $2
                  AND created_at <= $3
                "#,
            )
            .bind(sid)
            .bind(&start_time)
            .bind(&end_time)
            .fetch_one(pool)
            .await
            .map_err(db_err)?;
            (
                row.try_get("total_checks").unwrap_or(0),
                row.try_get("successful_checks").unwrap_or(0),
                row.try_get("avg_latency").ok(),
            )
        }
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

fn service_result_from_sqlite_row(row: sqlx::sqlite::SqliteRow) -> ServiceResult {
    ServiceResult {
        id: row.try_get("id").unwrap_or_default(),
        service_id: row.try_get("service_id").unwrap_or_default(),
        server_id: row.try_get("server_id").ok(),
        status: row.try_get("status").unwrap_or_default(),
        delay_ms: row.try_get("delay_ms").ok(),
        status_code: row.try_get("status_code").ok(),
        error: row.try_get("error").ok(),
        cert_fingerprint: row.try_get("cert_fingerprint").ok(),
        cert_not_after: row.try_get("cert_not_after").ok(),
        created_at: row.try_get("created_at").unwrap_or_default(),
    }
}

fn service_result_from_postgres_row(row: sqlx::postgres::PgRow) -> ServiceResult {
    ServiceResult {
        id: row.try_get("id").unwrap_or_default(),
        service_id: row.try_get("service_id").unwrap_or_default(),
        server_id: row.try_get("server_id").ok(),
        status: row.try_get("status").unwrap_or_default(),
        delay_ms: row.try_get("delay_ms").ok(),
        status_code: row.try_get("status_code").ok(),
        error: row.try_get("error").ok(),
        cert_fingerprint: row.try_get("cert_fingerprint").ok(),
        cert_not_after: row.try_get("cert_not_after").ok(),
        created_at: row.try_get("created_at").unwrap_or_default(),
    }
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
