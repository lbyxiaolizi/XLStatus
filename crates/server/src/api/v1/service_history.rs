use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::api::types::ApiResponse;
use crate::auth::middleware::AuthUser;
use crate::db::Db;

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
    pub error: Option<String>,
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

/// Get service history
pub async fn get_service_history(
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let mut sql = String::from(
        r#"
        SELECT id, service_id, server_id, status, delay_ms, error, created_at
        FROM service_results
        WHERE service_id = ?
    "#,
    );

    let mut params: Vec<String> = vec![service_id.clone()];

    if let Some(start) = &query.start_time {
        sql.push_str(" AND created_at >= ?");
        params.push(start.clone());
    }

    if let Some(end) = &query.end_time {
        sql.push_str(" AND created_at <= ?");
        params.push(end.clone());
    }

    sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");

    let results = match &db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let mut q = sqlx::query(&sql);
            for param in &params {
                q = q.bind(param);
            }
            q = q.bind(query.limit).bind(query.offset);

            let rows = q.fetch_all(pool).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        success: false,
                        data: None,
                        error: Some(format!("Failed to fetch history: {}", e)),
                    }),
                )
            })?;

            let mut results = Vec::new();
            for row in rows {
                results.push(ServiceResult {
                    id: row.try_get("id").unwrap_or_default(),
                    service_id: row.try_get("service_id").unwrap_or_default(),
                    server_id: row.try_get("server_id").ok(),
                    status: row.try_get("status").unwrap_or_default(),
                    delay_ms: row.try_get("delay_ms").ok(),
                    error: row.try_get("error").ok(),
                    created_at: row.try_get("created_at").unwrap_or_default(),
                });
            }
            results
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let mut q = sqlx::query(&sql);
            for param in &params {
                q = q.bind(param);
            }
            q = q.bind(query.limit).bind(query.offset);

            let rows = q.fetch_all(pool).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        success: false,
                        data: None,
                        error: Some(format!("Failed to fetch history: {}", e)),
                    }),
                )
            })?;

            let mut results = Vec::new();
            for row in rows {
                results.push(ServiceResult {
                    id: row.try_get("id").unwrap_or_default(),
                    service_id: row.try_get("service_id").unwrap_or_default(),
                    server_id: row.try_get("server_id").ok(),
                    status: row.try_get("status").unwrap_or_default(),
                    delay_ms: row.try_get("delay_ms").ok(),
                    error: row.try_get("error").ok(),
                    created_at: row.try_get("created_at").unwrap_or_default(),
                });
            }
            results
        }
    };

    // Calculate uptime
    let uptime_percent = if !results.is_empty() {
        let successful = results.iter().filter(|r| r.status == "success").count();
        Some((successful as f64 / results.len() as f64) * 100.0)
    } else {
        None
    };

    let total = results.len();

    Ok(Json(ApiResponse {
        success: true,
        data: Some(ServiceHistoryResponse {
            results,
            total,
            uptime_percent,
        }),
        error: None,
    }))
}

/// Get service uptime statistics
pub async fn get_service_uptime(
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let start_time = query.start_time.as_deref().unwrap_or("1970-01-01T00:00:00Z");
    let end_time = query.end_time.as_deref().unwrap_or("2099-12-31T23:59:59Z");

    let count_query = r#"
        SELECT
            COUNT(*) as total_checks,
            SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) as successful_checks,
            AVG(CASE WHEN status = 'success' THEN delay_ms ELSE NULL END) as avg_latency
        FROM service_results
        WHERE service_id = ?
          AND created_at >= ?
          AND created_at <= ?
    "#;

    let (total_checks, successful_checks, avg_latency) = match &db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query(count_query)
                .bind(&service_id)
                .bind(start_time)
                .bind(end_time)
                .fetch_one(pool)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            success: false,
                            data: None,
                            error: Some(format!("Failed to calculate uptime: {}", e)),
                        }),
                    )
                })?;

            let total_checks: i64 = row.try_get("total_checks").unwrap_or(0);
            let successful_checks: i64 = row.try_get("successful_checks").unwrap_or(0);
            let avg_latency: Option<f64> = row.try_get("avg_latency").ok();

            (total_checks, successful_checks, avg_latency)
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let row = sqlx::query(count_query)
                .bind(&service_id)
                .bind(start_time)
                .bind(end_time)
                .fetch_one(pool)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            success: false,
                            data: None,
                            error: Some(format!("Failed to calculate uptime: {}", e)),
                        }),
                    )
                })?;

            let total_checks: i64 = row.try_get("total_checks").unwrap_or(0);
            let successful_checks: i64 = row.try_get("successful_checks").unwrap_or(0);
            let avg_latency: Option<f64> = row.try_get("avg_latency").ok();

            (total_checks, successful_checks, avg_latency)
        }
    };

    let uptime_percent = if total_checks > 0 {
        (successful_checks as f64 / total_checks as f64) * 100.0
    } else {
        0.0
    };

    Ok(Json(ApiResponse {
        success: true,
        data: Some(ServiceUptimeResponse {
            service_id,
            total_checks,
            successful_checks,
            uptime_percent,
            avg_latency_ms: avg_latency,
            period_start: start_time.to_string(),
            period_end: end_time.to_string(),
        }),
        error: None,
    }))
}
