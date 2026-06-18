use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::collections::HashMap;
use xlstatus_proto_gen::xlstatus::v1::{
    server_task::Spec, FileReadTask, FileWriteTask, ServerTask, TaskOutcome, TaskType,
};
use xlstatus_shared::AgentId;

use crate::api::types::ApiResponse;
use crate::api::v1::auth::AppState;
use crate::mcp::executor::{percent_decode, validate_temporary_url_token};

const TEMP_TRANSFER_MAX_BYTES: usize = 100 * 1024 * 1024;
const DOWNLOAD_TIMEOUT_SECS: u64 = 60;
const UPLOAD_TIMEOUT_SECS: u64 = 120;
const TEMP_TRANSFER_RATE_LIMIT: u32 = 10;
const TEMP_TRANSFER_RATE_WINDOW_SECS: u64 = 60;

static TEMP_TRANSFER_RATE_STATE: once_cell::sync::Lazy<
    std::sync::Mutex<HashMap<String, (std::time::Instant, u32)>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub struct TempTransferQuery {
    pub server_id: String,
    pub path: String,
    pub expires_at: i64,
    pub token: String,
}

pub async fn temp_download(
    State(state): State<AppState>,
    Query(query): Query<TempTransferQuery>,
) -> Response {
    match run_temp_download(state, query).await {
        Ok(response) => response,
        Err((status, message)) => json_error(status, message),
    }
}

pub async fn temp_upload(
    State(state): State<AppState>,
    Query(query): Query<TempTransferQuery>,
    body: Bytes,
) -> Response {
    match run_temp_upload(state, query, body).await {
        Ok(response) => response,
        Err((status, message)) => json_error(status, message),
    }
}

async fn run_temp_download(
    state: AppState,
    query: TempTransferQuery,
) -> Result<Response, (StatusCode, String)> {
    let path = decode_path(&query.path)?;
    validate_query(
        &state,
        &query.server_id,
        &path,
        "download",
        query.expires_at,
        &query.token,
    )?;
    check_rate_limit(&query.token)?;
    let result = dispatch_server_task(
        &state,
        &query.server_id,
        ServerTask {
            task_id: String::new(),
            task_type: TaskType::FileRead as i32,
            spec: Some(Spec::FileRead(FileReadTask {
                path: path.clone(),
                offset: 0,
                length: TEMP_TRANSFER_MAX_BYTES as u64,
            })),
        },
        DOWNLOAD_TIMEOUT_SECS,
    )
    .await?;
    ensure_task_success(&result)?;

    let data = base64_decode(result.stdout.trim()).map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("agent returned invalid base64 file data: {e}"),
        )
    })?;
    if data.len() > TEMP_TRANSFER_MAX_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("file is larger than {} bytes", TEMP_TRANSFER_MAX_BYTES),
        ));
    }

    let filename = path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("download.bin");
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&data.len().to_string())
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            sanitize_filename(filename)
        ))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
    );
    Ok((StatusCode::OK, headers, data).into_response())
}

pub fn upload_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(TEMP_TRANSFER_MAX_BYTES)
}

async fn run_temp_upload(
    state: AppState,
    query: TempTransferQuery,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let path = decode_path(&query.path)?;
    validate_query(
        &state,
        &query.server_id,
        &path,
        "upload",
        query.expires_at,
        &query.token,
    )?;
    check_rate_limit(&query.token)?;
    if body.len() > TEMP_TRANSFER_MAX_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("upload is larger than {} bytes", TEMP_TRANSFER_MAX_BYTES),
        ));
    }
    let result = dispatch_server_task(
        &state,
        &query.server_id,
        ServerTask {
            task_id: String::new(),
            task_type: TaskType::FileWrite as i32,
            spec: Some(Spec::FileWrite(FileWriteTask {
                path: path.clone(),
                data: body.to_vec(),
                mode: 0,
                create_dirs: true,
            })),
        },
        UPLOAD_TIMEOUT_SECS,
    )
    .await?;
    ensure_task_success(&result)?;
    let written = result.stdout.trim().parse::<usize>().map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            "agent returned invalid byte count".to_string(),
        )
    })?;
    if written != body.len() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("agent wrote {} bytes, expected {}", written, body.len()),
        ));
    }
    Ok(Json(ApiResponse::success(serde_json::json!({
        "server_id": query.server_id,
        "path": path,
        "bytes_written": written,
    })))
    .into_response())
}

fn validate_query(
    state: &AppState,
    server_id: &str,
    path: &str,
    op: &str,
    expires_at: i64,
    token: &str,
) -> Result<(), (StatusCode, String)> {
    uuid::Uuid::parse_str(server_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid server_id".to_string()))?;
    if !path.starts_with('/') {
        return Err((StatusCode::BAD_REQUEST, "path must be absolute".to_string()));
    }
    if !validate_temporary_url_token(
        &state.config.security.session_secret,
        server_id,
        path,
        op,
        expires_at,
        token,
    ) {
        return Err((
            StatusCode::FORBIDDEN,
            "invalid or expired temporary URL".to_string(),
        ));
    }
    Ok(())
}

fn check_rate_limit(token: &str) -> Result<(), (StatusCode, String)> {
    let mut state = TEMP_TRANSFER_RATE_STATE.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "rate limiter unavailable".to_string(),
        )
    })?;
    let entry = state
        .entry(token.to_string())
        .or_insert_with(|| (std::time::Instant::now(), 0));
    if entry.0.elapsed() >= std::time::Duration::from_secs(TEMP_TRANSFER_RATE_WINDOW_SECS) {
        entry.0 = std::time::Instant::now();
        entry.1 = 0;
    }
    entry.1 = entry.1.saturating_add(1);
    if entry.1 > TEMP_TRANSFER_RATE_LIMIT {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "temporary URL rate limit exceeded".to_string(),
        ));
    }
    Ok(())
}

async fn dispatch_server_task(
    state: &AppState,
    server_id: &str,
    mut task: ServerTask,
    timeout_seconds: u64,
) -> Result<xlstatus_proto_gen::xlstatus::v1::TaskResult, (StatusCode, String)> {
    let agent_uuid = uuid::Uuid::parse_str(server_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid server_id".to_string()))?;
    let agent_id = AgentId(agent_uuid);
    if !state.session_registry.is_online(&agent_id).await {
        return Err((StatusCode::BAD_GATEWAY, "agent is offline".to_string()));
    }
    let response_registry = crate::current_task_response_registry();
    let run_id = uuid::Uuid::now_v7().to_string();
    task.task_id = run_id.clone();
    let rx = response_registry.register(run_id.clone()).await;
    if let Err(e) = state
        .session_registry
        .send_server_task(&agent_id, task)
        .await
    {
        response_registry.cancel(&run_id).await;
        return Err((StatusCode::BAD_GATEWAY, e));
    }
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_seconds), rx).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(_)) => Err((
            StatusCode::BAD_GATEWAY,
            "agent disconnected before reply".to_string(),
        )),
        Err(_) => {
            response_registry.cancel(&run_id).await;
            Err((
                StatusCode::GATEWAY_TIMEOUT,
                "temporary transfer timed out".to_string(),
            ))
        }
    }
}

fn ensure_task_success(
    result: &xlstatus_proto_gen::xlstatus::v1::TaskResult,
) -> Result<(), (StatusCode, String)> {
    let outcome = TaskOutcome::try_from(result.status).unwrap_or(TaskOutcome::Unspecified);
    if outcome == TaskOutcome::Success && result.exit_code == 0 {
        return Ok(());
    }
    let detail = if !result.stderr.trim().is_empty() {
        result.stderr.trim().to_string()
    } else if !result.error.trim().is_empty() {
        result.error.trim().to_string()
    } else {
        format!("agent command failed with exit code {}", result.exit_code)
    };
    Err((StatusCode::BAD_GATEWAY, detail))
}

fn decode_path(path: &str) -> Result<String, (StatusCode, String)> {
    percent_decode(path).map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid path: {e}")))
}

fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .map(|ch| match ch {
            '"' | '\\' | '/' | '\0' => '_',
            _ => ch,
        })
        .collect()
}

fn json_error(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(ApiResponse::<serde_json::Value> {
            success: false,
            data: None,
            error: Some(message),
        }),
    )
        .into_response()
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let bytes = input.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("length is not a multiple of 4".to_string());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks_exact(4) {
        let mut vals = [0u8; 4];
        let mut padding = 0;
        for (idx, b) in chunk.iter().copied().enumerate() {
            vals[idx] = match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => {
                    padding += 1;
                    0
                }
                _ => return Err(format!("invalid base64 byte: {b}")),
            };
        }
        let n = ((vals[0] as u32) << 18)
            | ((vals[1] as u32) << 12)
            | ((vals[2] as u32) << 6)
            | vals[3] as u32;
        out.push(((n >> 16) & 0xff) as u8);
        if padding < 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if padding < 1 {
            out.push((n & 0xff) as u8);
        }
    }
    Ok(out)
}
