use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::db::{AgentRepository, DatabaseBackend};
use axum::{extract::State, Json};
use chrono::Utc;
use serde::Serialize;
use sqlx::Row;

const ONLINE_THRESHOLD_SECS: i64 = 30;

#[derive(Debug, Serialize)]
pub struct PublicStatusResponse {
    pub servers: Vec<PublicServerView>,
    pub services: Vec<PublicServiceView>,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct PublicServerView {
    pub id: String,
    pub name: String,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub cpu_percent: Option<f64>,
    pub memory_used: Option<i64>,
    pub memory_total: Option<i64>,
    pub load_1: Option<f64>,
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
    pub last_status: Option<String>,
    pub last_check_at: Option<String>,
}

pub async fn public_status(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<PublicStatusResponse>>, AppError> {
    let servers = public_servers(&state).await?;
    let services = public_services(&state).await?;
    Ok(Json(ApiResponse::success(PublicStatusResponse {
        servers,
        services,
        updated_at: Utc::now().to_rfc3339(),
    })))
}

async fn public_servers(state: &AppState) -> Result<Vec<PublicServerView>, AppError> {
    let agent_repo = AgentRepository::new(state.db.clone());
    let (rows, _) = agent_repo.list_with_state(100, 0).await?;
    let now = Utc::now();

    Ok(rows
        .into_iter()
        .map(|row| {
            let agent = row.agent;
            let last_seen_age = agent
                .last_seen_at
                .map(|ts| (now - ts).num_seconds())
                .unwrap_or(i64::MAX);
            let status = if agent.revoked_at.is_some() {
                "revoked"
            } else if last_seen_age <= ONLINE_THRESHOLD_SECS {
                "online"
            } else {
                "offline"
            };
            let parsed = row
                .last_state_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());

            PublicServerView {
                id: agent.id.0.to_string(),
                name: agent.name,
                status: status.to_string(),
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
            }
        })
        .collect())
}

async fn public_services(state: &AppState) -> Result<Vec<PublicServiceView>, AppError> {
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT s.id, s.name, s.type, s.target,
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

            Ok(rows
                .into_iter()
                .map(|row| {
                    let service_type: String = row.get("type");
                    PublicServiceView {
                        id: row.get("id"),
                        name: row.get("name"),
                        service_type: service_type.clone(),
                        kind: service_type.clone(),
                        service_type_alias: service_type,
                        target: row.get("target"),
                        last_status: row.try_get("last_status").ok(),
                        last_check_at: row.try_get("last_check_at").ok(),
                    }
                })
                .collect())
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT s.id::text AS id, s.name, s.type, s.target,
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

            Ok(rows
                .into_iter()
                .map(|row| {
                    let service_type: String = row.get("type");
                    PublicServiceView {
                        id: row.get("id"),
                        name: row.get("name"),
                        service_type: service_type.clone(),
                        kind: service_type.clone(),
                        service_type_alias: service_type,
                        target: row.get("target"),
                        last_status: row.try_get("last_status").ok(),
                        last_check_at: row.try_get("last_check_at").ok(),
                    }
                })
                .collect())
        }
    }
}
