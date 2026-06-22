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
use crate::api::v1::services::{
    auth_has_global_service_visibility, ensure_service_id_visible_to_auth,
    visible_server_ids_for_auth,
};
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
    ensure_service_id_visible_to_auth(&state.db, &auth, &service_id).await?;
    let limit = query.limit.clamp(1, 1000);
    let offset = query.offset.max(0);
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
    ensure_service_id_visible_to_auth(&state.db, &auth, &service_id).await?;
    let start_time = query
        .start_time
        .unwrap_or_else(|| (Utc::now() - Duration::days(30)).to_rfc3339());
    let end_time = query.end_time.unwrap_or_else(|| Utc::now().to_rfc3339());
    let (total_checks, successful_checks, avg_latency) =
        service_uptime_for_auth(&state.db, &auth, &service_id, &start_time, &end_time).await?;

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
    let visible_server_ids = if auth_has_global_service_visibility(auth) {
        None
    } else {
        let ids = visible_server_ids_for_auth(db, auth).await?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        Some(ids)
    };

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
            if query.start_time.is_some() {
                sql.push_str(" AND created_at >= ?");
            }
            if query.end_time.is_some() {
                sql.push_str(" AND created_at <= ?");
            }
            if let Some(ids) = &visible_server_ids {
                sql.push_str(" AND server_id IN (");
                sql.push_str(&sqlite_placeholders(ids.len()));
                sql.push(')');
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");

            let mut q = sqlx::query(&sql).bind(service_id);
            if let Some(start) = &query.start_time {
                q = q.bind(start);
            }
            if let Some(end) = &query.end_time {
                q = q.bind(end);
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
                .map(|rows| {
                    rows.into_iter()
                        .map(service_result_from_sqlite_row)
                        .collect()
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
            if query.start_time.is_some() {
                sql.push_str(&format!(" AND created_at >= ${bind_index}"));
                bind_index += 1;
            }
            if query.end_time.is_some() {
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

            let sid = Uuid::parse_str(service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let mut q = sqlx::query(&sql).bind(sid);
            if let Some(start) = &query.start_time {
                q = q.bind(start);
            }
            if let Some(end) = &query.end_time {
                q = q.bind(end);
            }
            if let Some(ids) = &visible_server_ids {
                q = q.bind(parse_uuid_ids(ids)?);
            }
            q.bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
                .map_err(db_err)
                .map(|rows| {
                    rows.into_iter()
                        .map(service_result_from_postgres_row)
                        .collect()
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
    let visible_server_ids = if auth_has_global_service_visibility(auth) {
        None
    } else {
        let ids = visible_server_ids_for_auth(db, auth).await?;
        if ids.is_empty() {
            return Ok((0, 0, None));
        }
        Some(ids)
    };

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
                .bind(service_id)
                .bind(start_time)
                .bind(end_time);
            if let Some(ids) = &visible_server_ids {
                for id in ids {
                    q = q.bind(id);
                }
            }
            let row = q.fetch_one(pool).await.map_err(db_err)?;
            Ok((
                row.try_get("total_checks").unwrap_or(0),
                row.try_get("successful_checks").unwrap_or(0),
                row.try_get("avg_latency").ok(),
            ))
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let sid = Uuid::parse_str(service_id)
                .map_err(|e| AppError::BadRequest(format!("invalid service id: {e}")))?;
            let mut sql = String::from(
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
            );
            if visible_server_ids.is_some() {
                sql.push_str(" AND server_id = ANY($4::uuid[])");
            }
            let mut q = sqlx::query(&sql).bind(sid).bind(start_time).bind(end_time);
            if let Some(ids) = &visible_server_ids {
                q = q.bind(parse_uuid_ids(ids)?);
            }
            let row = q.fetch_one(pool).await.map_err(db_err)?;
            Ok((
                row.try_get("total_checks").unwrap_or(0),
                row.try_get("successful_checks").unwrap_or(0),
                row.try_get("avg_latency").ok(),
            ))
        }
    }
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
