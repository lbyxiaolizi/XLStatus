use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use xlstatus_proto_gen::xlstatus::v1::{
    server_message::Payload as ServerPayload, server_task::Spec, ConfigUpdate, FileDeleteTask,
    FileListTask, FileReadTask, FileWriteTask, ForceUpdate, ServerMessage, ServerTask, TaskOutcome,
    TaskType,
};
use xlstatus_shared::AgentId;

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::servers::server_visible;
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::AgentRepository;
use crate::mcp::executor::{percent_encode, temporary_url_expires_at, temporary_url_token};

const FILE_OP_TIMEOUT_SECS: u64 = 30;
const FILE_READ_MAX_BYTES: u64 = 2 * 1024 * 1024;
const TEMP_URL_DEFAULT_EXPIRES_SECS: i64 = 3600;

#[derive(Debug, Deserialize)]
pub struct FileListQuery {
    #[serde(default = "default_path")]
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct FileReadQuery {
    pub path: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default = "default_file_read_length")]
    pub length: u64,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

#[derive(Debug, Deserialize)]
pub struct FileWriteRequest {
    pub path: String,
    pub content: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default)]
    pub mode: Option<u32>,
    #[serde(default)]
    pub create_dirs: bool,
}

#[derive(Debug, Deserialize)]
pub struct FileDeleteRequest {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Deserialize)]
pub struct TempUrlQuery {
    pub path: String,
    #[serde(default = "default_temp_url_expires")]
    pub expires_in: i64,
}

#[derive(Debug, Deserialize)]
pub struct ApplyConfigRequest {
    #[serde(default)]
    pub config: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct ForceUpdateRequest {
    pub version: String,
    pub download_url: String,
    #[serde(default)]
    pub checksum: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FileEntryView {
    pub name: String,
    pub file_type: String,
    pub size: u64,
    pub mode: u32,
    pub modified_at: i64,
    pub symlink_target: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileEntryViewJson {
    name: String,
    file_type: String,
    size: u64,
    mode: u32,
    modified_at: i64,
    symlink_target: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FileListResponse {
    pub server_id: String,
    pub path: String,
    pub entries: Vec<FileEntryView>,
}

#[derive(Debug, Serialize)]
pub struct FileReadResponse {
    pub server_id: String,
    pub path: String,
    pub encoding: String,
    pub content: String,
    pub bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct TempUrlResponse {
    pub server_id: String,
    pub path: String,
    pub url: String,
    pub method: String,
    pub expires_at: i64,
}

pub async fn list_files(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Query(query): Query<FileListQuery>,
) -> Result<Json<ApiResponse<FileListResponse>>, AppError> {
    require_transfer_scope(&auth, "transfer:read")?;
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    let path = validate_abs_path(&query.path)?;
    let result = dispatch_file_task(
        &state,
        agent_id,
        ServerTask {
            task_id: String::new(),
            task_type: TaskType::FileList as i32,
            spec: Some(Spec::FileList(FileListTask { path: path.clone() })),
        },
        FILE_OP_TIMEOUT_SECS,
    )
    .await?;
    ensure_task_success(&result)?;
    let entries = serde_json::from_str::<Vec<FileEntryViewJson>>(&result.stdout)
        .map_err(|e| AppError::BadRequest(format!("agent returned invalid file list: {e}")))?;
    Ok(Json(ApiResponse::success(FileListResponse {
        server_id,
        path,
        entries: entries
            .into_iter()
            .map(|entry| FileEntryView {
                name: entry.name,
                file_type: entry.file_type,
                size: entry.size,
                mode: entry.mode,
                modified_at: entry.modified_at,
                symlink_target: entry.symlink_target,
            })
            .collect(),
    })))
}

pub async fn read_file(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Query(query): Query<FileReadQuery>,
) -> Result<Json<ApiResponse<FileReadResponse>>, AppError> {
    require_transfer_scope(&auth, "transfer:read")?;
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    let path = validate_abs_path(&query.path)?;
    let length = query.length.clamp(1, FILE_READ_MAX_BYTES);
    let result = dispatch_file_task(
        &state,
        agent_id,
        ServerTask {
            task_id: String::new(),
            task_type: TaskType::FileRead as i32,
            spec: Some(Spec::FileRead(FileReadTask {
                path: path.clone(),
                offset: query.offset,
                length,
            })),
        },
        FILE_OP_TIMEOUT_SECS,
    )
    .await?;
    ensure_task_success(&result)?;
    let bytes = decode_base64(result.stdout.trim())
        .map_err(|e| AppError::BadRequest(format!("agent returned invalid base64: {e}")))?;
    let encoding = normalize_encoding(&query.encoding)?;
    let content = if encoding == "base64" {
        result.stdout.trim().to_string()
    } else {
        String::from_utf8(bytes.clone())
            .map_err(|_| AppError::BadRequest("file is not valid UTF-8".into()))?
    };
    Ok(Json(ApiResponse::success(FileReadResponse {
        server_id,
        path,
        encoding,
        content,
        bytes: bytes.len(),
    })))
}

pub async fn write_file(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Json(req): Json<FileWriteRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_transfer_scope(&auth, "transfer:write")?;
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    let path = validate_abs_path(&req.path)?;
    let encoding = normalize_encoding(&req.encoding)?;
    let data = if encoding == "base64" {
        decode_base64(req.content.trim())
            .map_err(|e| AppError::BadRequest(format!("invalid base64 content: {e}")))?
    } else {
        req.content.into_bytes()
    };
    let result = dispatch_file_task(
        &state,
        agent_id,
        ServerTask {
            task_id: String::new(),
            task_type: TaskType::FileWrite as i32,
            spec: Some(Spec::FileWrite(FileWriteTask {
                path: path.clone(),
                data: data.clone(),
                mode: req.mode.unwrap_or(0),
                create_dirs: req.create_dirs,
            })),
        },
        FILE_OP_TIMEOUT_SECS,
    )
    .await?;
    ensure_task_success(&result)?;
    Ok(Json(ApiResponse::success(serde_json::json!({
        "server_id": server_id,
        "path": path,
        "bytes_written": result.stdout.trim().parse::<u64>().unwrap_or(data.len() as u64),
    }))))
}

pub async fn delete_file(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Json(req): Json<FileDeleteRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_transfer_scope(&auth, "transfer:write")?;
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    let path = validate_abs_path(&req.path)?;
    if path == "/" {
        return Err(AppError::BadRequest(
            "refusing to delete filesystem root".into(),
        ));
    }
    let result = dispatch_file_task(
        &state,
        agent_id,
        ServerTask {
            task_id: String::new(),
            task_type: TaskType::FileDelete as i32,
            spec: Some(Spec::FileDelete(FileDeleteTask {
                path: path.clone(),
                recursive: req.recursive,
            })),
        },
        FILE_OP_TIMEOUT_SECS,
    )
    .await?;
    ensure_task_success(&result)?;
    Ok(Json(ApiResponse::success(serde_json::json!({
        "server_id": server_id,
        "path": path,
        "deleted": true,
    }))))
}

pub async fn download_url(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Query(query): Query<TempUrlQuery>,
) -> Result<Json<ApiResponse<TempUrlResponse>>, AppError> {
    build_temp_url(state, auth, server_id, query, "download", "GET").await
}

pub async fn upload_url(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Query(query): Query<TempUrlQuery>,
) -> Result<Json<ApiResponse<TempUrlResponse>>, AppError> {
    build_temp_url(state, auth, server_id, query, "upload", "PUT").await
}

pub async fn get_config(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !has_scope(&auth, "server:read") {
        return Err(AppError::Forbidden("missing scope: server:read".into()));
    }
    let agent_id = ensure_server_visible(&state, &auth, &server_id).await?;
    let row = AgentRepository::new(state.db.clone())
        .find_by_id_with_state(agent_id)
        .await?
        .ok_or_else(|| AppError::NotFound("agent not found".into()))?;
    let last_info = row
        .last_info_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok());
    Ok(Json(ApiResponse::success(serde_json::json!({
        "server_id": server_id,
        "agent_name": row.agent.name,
        "online": state.session_registry.is_online(&agent_id).await,
        "last_info": last_info,
        "editable_fields": [
            "server",
            "grpc_server",
            "name",
            "report_interval_seconds",
            "ip_report_interval_seconds",
            "disable_auto_update",
            "disable_force_update",
            "disable_command_execute",
            "disable_nat",
            "disable_send_query"
        ]
    }))))
}

pub async fn apply_config(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Json(req): Json<ApplyConfigRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    if req
        .config
        .as_object()
        .map(|obj| obj.is_empty())
        .unwrap_or(true)
    {
        return Err(AppError::BadRequest(
            "config patch must not be empty".into(),
        ));
    }
    let payload = serde_json::to_vec(&req.config)
        .map_err(|e| AppError::BadRequest(format!("invalid config patch: {e}")))?;
    state
        .session_registry
        .send(
            &agent_id,
            ServerMessage {
                payload: Some(ServerPayload::ConfigUpdate(ConfigUpdate {
                    config_yaml: payload,
                })),
            },
        )
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(ApiResponse::success(serde_json::json!({
        "server_id": server_id,
        "sent": true,
    }))))
}

pub async fn force_update(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Json(req): Json<ForceUpdateRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    if req.version.trim().is_empty() || req.download_url.trim().is_empty() {
        return Err(AppError::BadRequest(
            "version and download_url are required".into(),
        ));
    }
    state
        .session_registry
        .send(
            &agent_id,
            ServerMessage {
                payload: Some(ServerPayload::ForceUpdate(ForceUpdate {
                    version: req.version.clone(),
                    download_url: req.download_url.clone(),
                    checksum: req.checksum.unwrap_or_default(),
                })),
            },
        )
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(ApiResponse::success(serde_json::json!({
        "server_id": server_id,
        "version": req.version,
        "sent": true,
    }))))
}

async fn build_temp_url(
    state: AppState,
    auth: AuthSession,
    server_id: String,
    query: TempUrlQuery,
    op: &str,
    method: &str,
) -> Result<Json<ApiResponse<TempUrlResponse>>, AppError> {
    let scope = if op == "download" {
        "transfer:read"
    } else {
        "transfer:write"
    };
    require_transfer_scope(&auth, scope)?;
    ensure_server_visible(&state, &auth, &server_id).await?;
    let path = validate_abs_path(&query.path)?;
    let expires_at = temporary_url_expires_at(query.expires_in);
    let token = temporary_url_token(
        &state.config.security.session_secret,
        &server_id,
        &path,
        op,
        expires_at,
    )
    .map_err(|e| AppError::BadRequest(format!("failed to sign temporary URL: {e}")))?;
    let route = if op == "download" {
        "/api/v1/transfers/temp/download"
    } else {
        "/api/v1/transfers/temp/upload"
    };
    let url = format!(
        "{}?server_id={}&path={}&expires_at={}&token={}",
        route,
        percent_encode(&server_id),
        percent_encode(&path),
        expires_at,
        token
    );
    Ok(Json(ApiResponse::success(TempUrlResponse {
        server_id,
        path,
        url,
        method: method.to_string(),
        expires_at,
    })))
}

async fn ensure_server_visible(
    state: &AppState,
    auth: &AuthSession,
    server_id: &str,
) -> Result<AgentId, AppError> {
    let parsed = uuid::Uuid::parse_str(server_id)
        .map_err(|_| AppError::BadRequest(format!("invalid server id: {server_id}")))?;
    let agent_id = AgentId(parsed);
    if !server_visible(auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let exists = AgentRepository::new(state.db.clone())
        .find_by_id(agent_id)
        .await?
        .is_some();
    if !exists {
        return Err(AppError::NotFound("agent not found".into()));
    }
    Ok(agent_id)
}

async fn ensure_server_online(
    state: &AppState,
    auth: &AuthSession,
    server_id: &str,
) -> Result<AgentId, AppError> {
    let agent_id = ensure_server_visible(state, auth, server_id).await?;
    if !state.session_registry.is_online(&agent_id).await {
        return Err(AppError::BadRequest("agent is offline".into()));
    }
    Ok(agent_id)
}

async fn dispatch_file_task(
    state: &AppState,
    agent_id: AgentId,
    mut task: ServerTask,
    timeout_seconds: u64,
) -> Result<xlstatus_proto_gen::xlstatus::v1::TaskResult, AppError> {
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
        return Err(AppError::BadRequest(e));
    }
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_seconds), rx).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(_)) => Err(AppError::BadRequest(
            "agent disconnected before replying".into(),
        )),
        Err(_) => {
            response_registry.cancel(&run_id).await;
            Err(AppError::BadRequest("agent operation timed out".into()))
        }
    }
}

fn ensure_task_success(
    result: &xlstatus_proto_gen::xlstatus::v1::TaskResult,
) -> Result<(), AppError> {
    let outcome = TaskOutcome::try_from(result.status).unwrap_or(TaskOutcome::Unspecified);
    if outcome == TaskOutcome::Success && result.exit_code == 0 {
        return Ok(());
    }
    let detail = if !result.error.trim().is_empty() {
        result.error.trim().to_string()
    } else if !result.stderr.trim().is_empty() {
        result.stderr.trim().to_string()
    } else {
        format!("agent operation failed with exit code {}", result.exit_code)
    };
    Err(AppError::BadRequest(detail))
}

fn require_transfer_scope(auth: &AuthSession, scope: &str) -> Result<(), AppError> {
    if has_scope(auth, scope) {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!("missing scope: {scope}")))
    }
}

fn validate_abs_path(path: &str) -> Result<String, AppError> {
    let trimmed = path.trim();
    if !trimmed.starts_with('/') {
        return Err(AppError::BadRequest("path must be absolute".into()));
    }
    if trimmed.contains('\0') {
        return Err(AppError::BadRequest("path contains NUL byte".into()));
    }
    Ok(trimmed.to_string())
}

fn normalize_encoding(value: &str) -> Result<String, AppError> {
    match value {
        "" | "utf8" | "text" => Ok("utf8".into()),
        "base64" => Ok("base64".into()),
        other => Err(AppError::BadRequest(format!(
            "unsupported encoding: {other}"
        ))),
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
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

fn default_path() -> String {
    "/".into()
}

fn default_file_read_length() -> u64 {
    64 * 1024
}

fn default_encoding() -> String {
    "utf8".into()
}

fn default_temp_url_expires() -> i64 {
    TEMP_URL_DEFAULT_EXPIRES_SECS
}
