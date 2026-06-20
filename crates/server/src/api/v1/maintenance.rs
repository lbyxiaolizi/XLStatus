//! Maintenance and backup API.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{require_sensitive_totp, AppError, AppState};
use crate::api::v1::settings::{set_tsdb_retention_days, tsdb_retention_days};
use crate::auth::middleware::AuthSession;
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Pool, Row, Sqlite,
};
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;

const RESTORE_MAX_BYTES: usize = 512 * 1024 * 1024;
const RESTORE_SCHEMA_ALIAS: &str = "restore_src";

#[derive(Debug, Serialize)]
pub struct MaintenanceStatus {
    pub database_backend: String,
    pub backup_supported: bool,
    pub archive_supported: bool,
    pub restore_supported: bool,
    pub vacuum_supported: bool,
    pub tsdb_compact_supported: bool,
    pub tsdb_backend: String,
    pub tsdb_status: String,
    pub tsdb_samples: Option<usize>,
    pub tsdb_retention_days: Option<i64>,
    pub tsdb_retention_configurable: bool,
}

#[derive(Debug, Serialize)]
pub struct MaintenanceActionResponse {
    pub action: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
struct MaintenanceArchiveManifest {
    generated_at: String,
    database_backend: String,
    tsdb_backend: String,
    tsdb_status: String,
    tsdb_samples: Option<usize>,
    tsdb_retention_days: Option<i64>,
    files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TsdbCompactResponse {
    pub action: String,
    pub success: bool,
    pub backend: String,
    pub removed_samples: usize,
    pub samples_before: Option<usize>,
    pub samples_after: Option<usize>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct TsdbRetentionRequest {
    pub retention_days: i64,
}

#[derive(Debug, Serialize)]
pub struct TsdbRetentionResponse {
    pub action: String,
    pub success: bool,
    pub backend: String,
    pub retention_days: i64,
    pub samples_before: Option<usize>,
    pub samples_after: Option<usize>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct RestoreQuery {
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct MaintenanceRestoreResponse {
    pub dry_run: bool,
    pub restored: bool,
    pub compatible: bool,
    pub database_backend: String,
    pub user_version: i64,
    pub table_count: usize,
    pub row_count: i64,
    pub message: String,
}

#[derive(Debug, Clone)]
struct TableSchema {
    name: String,
    columns: Vec<String>,
}

pub async fn maintenance_status(
    State(state): State<AppState>,
    auth: AuthSession,
) -> Result<Json<ApiResponse<MaintenanceStatus>>, AppError> {
    require_admin(&auth)?;
    let sqlite = matches!(state.db, crate::db::DatabaseBackend::Sqlite(_));
    let tsdb_health = state.metrics.health();
    let configured_retention = tsdb_retention_days(&state.db).await?;
    let active_retention = state.metrics.retention().map(|duration| {
        let seconds = duration.num_seconds().max(1);
        ((seconds + 86_399) / 86_400).clamp(1, 3650)
    });
    Ok(Json(ApiResponse::success(MaintenanceStatus {
        database_backend: if sqlite { "sqlite" } else { "postgres" }.into(),
        backup_supported: sqlite,
        archive_supported: sqlite,
        restore_supported: sqlite,
        vacuum_supported: sqlite,
        tsdb_compact_supported: true,
        tsdb_backend: tsdb_health.backend,
        tsdb_status: format!("{:?}", tsdb_health.status).to_ascii_lowercase(),
        tsdb_samples: tsdb_health.samples,
        tsdb_retention_days: active_retention.or(Some(configured_retention)),
        tsdb_retention_configurable: state.metrics.retention().is_some(),
    })))
}

pub async fn download_backup(
    State(state): State<AppState>,
    auth: AuthSession,
) -> Result<Response, AppError> {
    require_admin(&auth)?;
    let crate::db::DatabaseBackend::Sqlite(pool) = &state.db else {
        return Err(AppError::BadRequest(
            "backup download currently supports SQLite only".into(),
        ));
    };

    let tmp_path =
        std::env::temp_dir().join(format!("xlstatus-backup-{}.sqlite3", uuid::Uuid::now_v7()));
    let sql = format!("VACUUM INTO '{}'", sql_quote_path(&tmp_path));
    sqlx::query(&sql).execute(pool).await.map_err(db_err)?;
    let bytes = tokio::fs::read(&tmp_path)
        .await
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    let _ = tokio::fs::remove_file(&tmp_path).await;

    let filename = format!(
        "xlstatus-backup-{}.sqlite3",
        Utc::now().format("%Y%m%d%H%M%S")
    );
    let content_disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/vnd.sqlite3"),
            ),
            (header::CONTENT_DISPOSITION, content_disposition),
        ],
        Body::from(bytes),
    )
        .into_response())
}

pub async fn download_archive(
    State(state): State<AppState>,
    auth: AuthSession,
) -> Result<Response, AppError> {
    require_admin(&auth)?;
    let crate::db::DatabaseBackend::Sqlite(pool) = &state.db else {
        return Err(AppError::BadRequest(
            "full archive currently supports SQLite only".into(),
        ));
    };
    let database_bytes = sqlite_backup_bytes(pool).await?;
    let tsdb_health = state.metrics.health();
    let tsdb_samples = state
        .metrics
        .export_samples()
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    let tsdb_samples_json = serde_json::to_vec_pretty(&tsdb_samples)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let retention_days = state.metrics.retention().map(|duration| {
        let seconds = duration.num_seconds().max(1);
        ((seconds + 86_399) / 86_400).clamp(1, 3650)
    });
    let manifest = MaintenanceArchiveManifest {
        generated_at: Utc::now().to_rfc3339(),
        database_backend: "sqlite".into(),
        tsdb_backend: tsdb_health.backend,
        tsdb_status: format!("{:?}", tsdb_health.status).to_ascii_lowercase(),
        tsdb_samples: tsdb_health.samples,
        tsdb_retention_days: retention_days,
        files: vec![
            "manifest.json".into(),
            "database.sqlite3".into(),
            "tsdb_samples.json".into(),
        ],
    };
    let manifest_json =
        serde_json::to_vec_pretty(&manifest).map_err(|e| AppError::BadRequest(e.to_string()))?;

    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer
        .start_file("manifest.json", options)
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    writer
        .write_all(&manifest_json)
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    writer
        .start_file("database.sqlite3", options)
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    writer
        .write_all(&database_bytes)
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    writer
        .start_file("tsdb_samples.json", options)
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    writer
        .write_all(&tsdb_samples_json)
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    let bytes = writer
        .finish()
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?
        .into_inner();

    let filename = format!("xlstatus-archive-{}.zip", Utc::now().format("%Y%m%d%H%M%S"));
    let content_disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/zip"),
            ),
            (header::CONTENT_DISPOSITION, content_disposition),
        ],
        Body::from(bytes),
    )
        .into_response())
}

pub async fn restore_backup(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    Query(query): Query<RestoreQuery>,
    body: Bytes,
) -> Result<Json<ApiResponse<MaintenanceRestoreResponse>>, AppError> {
    require_admin(&auth)?;
    if !query.dry_run {
        require_sensitive_totp(&state.db, auth.user_id, &headers).await?;
    }
    let crate::db::DatabaseBackend::Sqlite(pool) = &state.db else {
        return Err(AppError::BadRequest(
            "backup restore currently supports SQLite only".into(),
        ));
    };
    if body.is_empty() {
        return Err(AppError::BadRequest("backup file is empty".into()));
    }

    let tmp_path =
        std::env::temp_dir().join(format!("xlstatus-restore-{}.sqlite3", uuid::Uuid::now_v7()));
    tokio::fs::write(&tmp_path, &body)
        .await
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;

    let result = async {
        let validation = validate_restore_candidate(pool, &tmp_path).await?;
        if !query.dry_run {
            apply_sqlite_restore(pool, &tmp_path, &validation.schema).await?;
        }
        Ok(Json(ApiResponse::success(MaintenanceRestoreResponse {
            dry_run: query.dry_run,
            restored: !query.dry_run,
            compatible: true,
            database_backend: "sqlite".into(),
            user_version: validation.user_version,
            table_count: validation.schema.len(),
            row_count: validation.row_count,
            message: if query.dry_run {
                "SQLite backup validation completed".into()
            } else {
                "SQLite backup restored".into()
            },
        })))
    }
    .await;

    let _ = tokio::fs::remove_file(&tmp_path).await;
    result
}

pub fn restore_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(RESTORE_MAX_BYTES)
}

async fn sqlite_backup_bytes(pool: &Pool<Sqlite>) -> Result<Vec<u8>, AppError> {
    let tmp_path =
        std::env::temp_dir().join(format!("xlstatus-backup-{}.sqlite3", uuid::Uuid::now_v7()));
    let sql = format!("VACUUM INTO '{}'", sql_quote_path(&tmp_path));
    sqlx::query(&sql).execute(pool).await.map_err(db_err)?;
    let bytes = tokio::fs::read(&tmp_path)
        .await
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    let _ = tokio::fs::remove_file(&tmp_path).await;
    Ok(bytes)
}

pub async fn vacuum_sqlite(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<MaintenanceActionResponse>>, AppError> {
    require_admin(&auth)?;
    require_sensitive_totp(&state.db, auth.user_id, &headers).await?;
    let crate::db::DatabaseBackend::Sqlite(pool) = &state.db else {
        return Err(AppError::BadRequest(
            "SQLite VACUUM is only available for SQLite databases".into(),
        ));
    };
    sqlx::query("VACUUM").execute(pool).await.map_err(db_err)?;
    Ok(Json(ApiResponse::success(MaintenanceActionResponse {
        action: "sqlite_vacuum".into(),
        success: true,
        message: "SQLite VACUUM completed".into(),
    })))
}

pub async fn compact_tsdb(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<TsdbCompactResponse>>, AppError> {
    require_admin(&auth)?;
    require_sensitive_totp(&state.db, auth.user_id, &headers).await?;
    let before = state.metrics.health();
    let removed = state
        .metrics
        .compact()
        .map_err(|e| AppError::Database(anyhow::anyhow!(e)))?;
    let after = state.metrics.health();
    Ok(Json(ApiResponse::success(TsdbCompactResponse {
        action: "tsdb_compact".into(),
        success: true,
        backend: after.backend,
        removed_samples: removed,
        samples_before: before.samples,
        samples_after: after.samples,
        message: format!("TSDB compact completed, removed {removed} samples"),
    })))
}

pub async fn update_tsdb_retention(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    Json(req): Json<TsdbRetentionRequest>,
) -> Result<Json<ApiResponse<TsdbRetentionResponse>>, AppError> {
    require_admin(&auth)?;
    require_sensitive_totp(&state.db, auth.user_id, &headers).await?;
    let days = req.retention_days.clamp(1, 3650);
    let before = state.metrics.health();
    state
        .metrics
        .set_retention(Duration::days(days))
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    set_tsdb_retention_days(&state.db, days).await?;
    let after = state.metrics.health();
    Ok(Json(ApiResponse::success(TsdbRetentionResponse {
        action: "tsdb_retention".into(),
        success: true,
        backend: after.backend,
        retention_days: days,
        samples_before: before.samples,
        samples_after: after.samples,
        message: format!("TSDB retention updated to {days} day(s)"),
    })))
}

struct RestoreValidation {
    schema: Vec<TableSchema>,
    user_version: i64,
    row_count: i64,
}

async fn validate_restore_candidate(
    current_pool: &Pool<Sqlite>,
    restore_path: &Path,
) -> Result<RestoreValidation, AppError> {
    let options = SqliteConnectOptions::new()
        .filename(restore_path)
        .read_only(true)
        .create_if_missing(false);
    let restore_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(db_err)?;

    let integrity: (String,) = sqlx::query_as("PRAGMA integrity_check")
        .fetch_one(&restore_pool)
        .await
        .map_err(db_err)?;
    if integrity.0 != "ok" {
        return Err(AppError::BadRequest(format!(
            "SQLite integrity check failed: {}",
            integrity.0
        )));
    }

    let current_schema = load_sqlite_schema(current_pool).await?;
    let restore_schema = load_sqlite_schema(&restore_pool).await?;
    ensure_schema_compatible(&current_schema, &restore_schema)?;
    let user_version: (i64,) = sqlx::query_as("PRAGMA user_version")
        .fetch_one(&restore_pool)
        .await
        .map_err(db_err)?;
    let row_count = count_rows(&restore_pool, &current_schema).await?;

    Ok(RestoreValidation {
        schema: current_schema,
        user_version: user_version.0,
        row_count,
    })
}

async fn apply_sqlite_restore(
    pool: &Pool<Sqlite>,
    restore_path: &Path,
    schema: &[TableSchema],
) -> Result<(), AppError> {
    let mut conn = pool.acquire().await.map_err(db_err)?;
    let result = async {
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *conn)
            .await
            .map_err(db_err)?;
        sqlx::query(&format!(
            "ATTACH DATABASE '{}' AS {}",
            sql_quote_path(restore_path),
            sqlite_quote_identifier(RESTORE_SCHEMA_ALIAS)
        ))
        .execute(&mut *conn)
        .await
        .map_err(db_err)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(db_err)?;

        for table in schema.iter().rev() {
            sqlx::query(&format!(
                "DELETE FROM {}",
                sqlite_quote_identifier(&table.name)
            ))
            .execute(&mut *conn)
            .await
            .map_err(db_err)?;
        }
        for table in schema {
            let columns = table
                .columns
                .iter()
                .map(|column| sqlite_quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ");
            sqlx::query(&format!(
                "INSERT INTO {table_name} ({columns}) SELECT {columns} FROM {schema_name}.{table_name}",
                table_name = sqlite_quote_identifier(&table.name),
                schema_name = sqlite_quote_identifier(RESTORE_SCHEMA_ALIAS),
            ))
            .execute(&mut *conn)
            .await
            .map_err(db_err)?;
        }

        let fk_errors = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *conn)
            .await
            .map_err(db_err)?;
        if !fk_errors.is_empty() {
            return Err(AppError::BadRequest(format!(
                "restored database failed foreign key check: {} issue(s)",
                fk_errors.len()
            )));
        }

        sqlx::query("COMMIT")
            .execute(&mut *conn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
    .await;

    if result.is_err() {
        let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
    }
    let detach_result = sqlx::query(&format!(
        "DETACH DATABASE {}",
        sqlite_quote_identifier(RESTORE_SCHEMA_ALIAS)
    ))
    .execute(&mut *conn)
    .await;
    let _ = sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await;

    result?;
    detach_result.map_err(db_err)?;
    Ok(())
}

async fn load_sqlite_schema(pool: &Pool<Sqlite>) -> Result<Vec<TableSchema>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT name
        FROM sqlite_master
        WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
        ORDER BY name ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    let mut schema = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.get("name");
        schema.push(TableSchema {
            columns: load_sqlite_columns(pool, &name).await?,
            name,
        });
    }
    Ok(schema)
}

async fn load_sqlite_columns(pool: &Pool<Sqlite>, table: &str) -> Result<Vec<String>, AppError> {
    let rows = sqlx::query(&format!(
        "PRAGMA table_info({})",
        sqlite_quote_identifier(table)
    ))
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect())
}

fn ensure_schema_compatible(
    current_schema: &[TableSchema],
    restore_schema: &[TableSchema],
) -> Result<(), AppError> {
    for current_table in current_schema {
        let Some(restore_table) = restore_schema
            .iter()
            .find(|table| table.name == current_table.name)
        else {
            return Err(AppError::BadRequest(format!(
                "backup is missing table {}",
                current_table.name
            )));
        };
        if restore_table.columns != current_table.columns {
            return Err(AppError::BadRequest(format!(
                "backup table {} schema does not match current database",
                current_table.name
            )));
        }
    }
    Ok(())
}

async fn count_rows(pool: &Pool<Sqlite>, schema: &[TableSchema]) -> Result<i64, AppError> {
    let mut total = 0_i64;
    for table in schema {
        let row: (i64,) = sqlx::query_as(&format!(
            "SELECT COUNT(*) FROM {}",
            sqlite_quote_identifier(&table.name)
        ))
        .fetch_one(pool)
        .await
        .map_err(db_err)?;
        total += row.0;
    }
    Ok(total)
}

fn require_admin(auth: &AuthSession) -> Result<(), AppError> {
    if auth.role.is_admin() {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin role required".into()))
    }
}

fn sql_quote_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn sqlite_quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn db_err(err: sqlx::Error) -> AppError {
    AppError::Database(anyhow::anyhow!(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn escapes_sql_path_literal() {
        let path = PathBuf::from("/tmp/xl'status.sqlite3");
        assert_eq!(sql_quote_path(&path), "/tmp/xl''status.sqlite3");
    }

    #[test]
    fn quotes_sqlite_identifier() {
        assert_eq!(sqlite_quote_identifier("users"), "\"users\"");
        assert_eq!(sqlite_quote_identifier("a\"b"), "\"a\"\"b\"");
    }
}
