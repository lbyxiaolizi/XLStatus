use crate::api::types::*;
use crate::api::v1::auth::{AppError, AppState};
use crate::services::{probe_http, probe_tcp, ProbeType};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateServiceRequest {
    pub name: String,
    pub service_type: String, // "http", "tcp", "icmp"
    pub target: String,
    pub interval_seconds: Option<i32>,
    pub timeout_seconds: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ServiceResponse {
    pub id: String,
    pub name: String,
    pub service_type: String,
    pub target: String,
    pub enabled: bool,
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
}

pub async fn test_probe(
    State(_state): State<AppState>,
    Json(req): Json<ProbeTestRequest>,
) -> Result<Json<ApiResponse<ProbeTestResponse>>, AppError> {
    let timeout = req.timeout_seconds.unwrap_or(10) as u64;

    let result = match ProbeType::from_str(&req.service_type) {
        Some(ProbeType::Http) => probe_http(&req.target, timeout).await,
        Some(ProbeType::Tcp) => {
            // Parse host:port
            let parts: Vec<&str> = req.target.split(':').collect();
            if parts.len() != 2 {
                return Err(AppError::BadRequest(
                    "TCP target must be host:port".to_string(),
                ));
            }
            let port = parts[1]
                .parse()
                .map_err(|_| AppError::BadRequest("Invalid port".to_string()))?;
            probe_tcp(parts[0], port, timeout).await
        }
        Some(ProbeType::Icmp) => {
            return Err(AppError::BadRequest("ICMP not implemented".to_string()));
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
    })))
}
