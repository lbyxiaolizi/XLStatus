use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::HeaderMap,
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
use crate::api::v1::auth::{require_sensitive_totp, AppError, AppState};
use crate::api::v1::servers::{ensure_agent_visible, server_visible};
use crate::auth::generate_temporary_transfer_token;
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::auth::rbac::has_scope;
use crate::db::{
    AgentRepository, CreateTemporaryTransferTokenInput, TemporaryTransferTokenRepository,
};
use crate::grpc::{base64_encoded_len, ensure_task_result_text_within};
use crate::mcp::executor::{
    percent_encode, temporary_url_expires_at, temporary_url_expires_in,
    TEMP_URL_DEFAULT_EXPIRES_SECS,
};

const FILE_OP_TIMEOUT_SECS: u64 = 30;
const FILE_READ_MAX_BYTES: u64 = 2 * 1024 * 1024;
const FILE_WRITE_MAX_BYTES: usize = 2 * 1024 * 1024;
const SERVER_OPS_API_MAX_BODY_BYTES: usize = 3 * 1024 * 1024;
const SERVER_OPS_UUID_TEXT_LEN: usize = 36;
const SERVER_OPS_MAX_PATH_BYTES: usize = 4096;
const CONFIG_PATCH_MAX_BYTES: usize = 128 * 1024;
const FORCE_UPDATE_MAX_URL_BYTES: usize = 2048;
const FILE_LIST_RESULT_MAX_BYTES: usize = 2 * 1024 * 1024;
const FILE_SMALL_RESULT_MAX_BYTES: usize = 4096;
const FORCE_UPDATE_REPO_HOST: &str = "github.com";
const FORCE_UPDATE_REPO_PATH_PREFIX: &str = "/lbyxiaolizi/XLStatus/releases/download/";
const REMOTE_CONFIG_LOW_RISK_FIELDS: &[&str] = &[
    "name",
    "report_interval_seconds",
    "ip_report_interval_seconds",
];
const REMOTE_CONFIG_SENSITIVE_FIELDS: &[&str] = &[
    "server",
    "grpc_server",
    "grpc_tls_ca_path",
    "grpc_tls_domain_name",
    "grpc_tls_client_cert_path",
    "grpc_tls_client_key_path",
    "disable_auto_update",
    "disable_force_update",
    "disable_command_execute",
    "disable_nat",
    "disable_send_query",
    "file_allowed_roots",
];
const REMOTE_CONFIG_FORBIDDEN_FIELDS: &[&str] = &["agent_id", "public_key", "private_key"];

#[derive(Debug, Deserialize)]
pub struct FileListRequest {
    #[serde(default = "default_path")]
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct FileReadRequest {
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
pub struct TempUrlRequest {
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

pub fn server_ops_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(SERVER_OPS_API_MAX_BODY_BYTES)
}

pub async fn list_files(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Json(req): Json<FileListRequest>,
) -> Result<Json<ApiResponse<FileListResponse>>, AppError> {
    require_transfer_scope(&auth, "transfer:read")?;
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    let path = validate_abs_path(&req.path)?;
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
    ensure_file_task_result_text(
        &result,
        FILE_LIST_RESULT_MAX_BYTES,
        "agent file list result",
    )?;
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
    Json(req): Json<FileReadRequest>,
) -> Result<Json<ApiResponse<FileReadResponse>>, AppError> {
    require_transfer_scope(&auth, "transfer:read")?;
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    let path = validate_abs_path(&req.path)?;
    let length = req.length.clamp(1, FILE_READ_MAX_BYTES);
    let result = dispatch_file_task(
        &state,
        agent_id,
        ServerTask {
            task_id: String::new(),
            task_type: TaskType::FileRead as i32,
            spec: Some(Spec::FileRead(FileReadTask {
                path: path.clone(),
                offset: req.offset,
                length,
            })),
        },
        FILE_OP_TIMEOUT_SECS,
    )
    .await?;
    ensure_file_task_result_text(
        &result,
        base64_encoded_len(length as usize),
        "agent file read result",
    )?;
    ensure_task_success(&result)?;
    let bytes = decode_base64(result.stdout.trim())
        .map_err(|e| AppError::BadRequest(format!("agent returned invalid base64: {e}")))?;
    if bytes.len() > length as usize {
        return Err(AppError::BadRequest(format!(
            "agent returned more than {length} file bytes"
        )));
    }
    let encoding = normalize_encoding(&req.encoding)?;
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
    ensure_file_write_size(&data)?;
    let bytes_len = data.len();
    let result = dispatch_file_task(
        &state,
        agent_id,
        ServerTask {
            task_id: String::new(),
            task_type: TaskType::FileWrite as i32,
            spec: Some(Spec::FileWrite(FileWriteTask {
                path: path.clone(),
                data,
                mode: req.mode.unwrap_or(0),
                create_dirs: req.create_dirs,
            })),
        },
        FILE_OP_TIMEOUT_SECS,
    )
    .await?;
    ensure_file_task_result_text(
        &result,
        FILE_SMALL_RESULT_MAX_BYTES,
        "agent file write result",
    )?;
    ensure_task_success(&result)?;
    Ok(Json(ApiResponse::success(serde_json::json!({
        "server_id": server_id,
        "path": path,
        "bytes_written": result.stdout.trim().parse::<u64>().unwrap_or(bytes_len as u64),
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
    ensure_file_task_result_text(
        &result,
        FILE_SMALL_RESULT_MAX_BYTES,
        "agent file delete result",
    )?;
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
    Json(req): Json<TempUrlRequest>,
) -> Result<Json<ApiResponse<TempUrlResponse>>, AppError> {
    build_temp_url(state, auth, server_id, req, "download", "GET").await
}

pub async fn upload_url(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(server_id): Path<String>,
    Json(req): Json<TempUrlRequest>,
) -> Result<Json<ApiResponse<TempUrlResponse>>, AppError> {
    build_temp_url(state, auth, server_id, req, "upload", "PUT").await
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
            "disable_send_query",
            "file_allowed_roots"
        ]
    }))))
}

pub async fn apply_config(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    Path(server_id): Path<String>,
    Json(req): Json<ApplyConfigRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !has_scope(&auth, "server:write") {
        return Err(AppError::Forbidden("missing scope: server:write".into()));
    }
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
    require_remote_config_patch_auth(&state.db, &auth, &headers, &req.config).await?;
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    let payload = serialize_config_patch(&req.config)?;
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
    headers: HeaderMap,
    Path(server_id): Path<String>,
    Json(req): Json<ForceUpdateRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_force_update_auth(&state.db, &auth, &headers).await?;
    let agent_id = ensure_server_online(&state, &auth, &server_id).await?;
    let force_update = validate_force_update_request(req)?;
    state
        .session_registry
        .send(
            &agent_id,
            ServerMessage {
                payload: Some(ServerPayload::ForceUpdate(force_update.clone())),
            },
        )
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(ApiResponse::success(serde_json::json!({
        "server_id": server_id,
        "version": force_update.version,
        "sent": true,
    }))))
}

async fn build_temp_url(
    state: AppState,
    auth: AuthSession,
    server_id: String,
    req: TempUrlRequest,
    op: &str,
    method: &str,
) -> Result<Json<ApiResponse<TempUrlResponse>>, AppError> {
    let scope = if op == "download" {
        "transfer:read"
    } else {
        "transfer:write"
    };
    require_transfer_scope(&auth, scope)?;
    let agent_id = ensure_server_active(&state, &auth, &server_id).await?;
    let path = validate_abs_path(&req.path)?;
    let expires_in = temporary_url_expires_in(req.expires_in);
    let expires_at = temporary_url_expires_at(expires_in);
    let (token, token_hash) = generate_temporary_transfer_token();
    let expires_at_dt = chrono::DateTime::from_timestamp(expires_at, 0)
        .ok_or_else(|| AppError::BadRequest("invalid temporary URL expiration".into()))?;
    let auth_kind = match auth.auth_kind {
        AuthKind::Session => "session",
        AuthKind::PersonalAccessToken => "pat",
    }
    .to_string();
    TemporaryTransferTokenRepository::new(state.db.clone())
        .create(CreateTemporaryTransferTokenInput {
            token_hash,
            server_id: agent_id,
            path: path.clone(),
            op: op.to_string(),
            issued_by_user_id: auth.user_id,
            auth_kind,
            session_id: matches!(auth.auth_kind, AuthKind::Session)
                .then_some(auth.session_id.clone()),
            api_token_id: auth.pat_id.clone(),
            scope: scope.to_string(),
            expires_at: expires_at_dt,
            created_ip: None,
        })
        .await?;
    let route = if op == "download" {
        "/api/v1/transfers/temp/download"
    } else {
        "/api/v1/transfers/temp/upload"
    };
    let url = format!("{}?token={}", route, percent_encode(&token),);
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
    let parsed = parse_server_ops_agent_id(server_id)?;
    let agent_id = AgentId(parsed);
    if !server_visible(auth, &agent_id) {
        return Err(AppError::Forbidden("agent not in scope".into()));
    }
    let agent = AgentRepository::new(state.db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    ensure_agent_visible(auth, &agent)?;
    Ok(agent_id)
}

fn parse_server_ops_agent_id(server_id: &str) -> Result<uuid::Uuid, AppError> {
    if server_id.len() != SERVER_OPS_UUID_TEXT_LEN {
        return Err(AppError::BadRequest(
            "server_id must be a canonical UUID".into(),
        ));
    }
    let parsed = uuid::Uuid::parse_str(server_id)
        .map_err(|_| AppError::BadRequest("server_id must be a canonical UUID".into()))?;
    if parsed.to_string() != server_id {
        return Err(AppError::BadRequest(
            "server_id must be a canonical UUID".into(),
        ));
    }
    Ok(parsed)
}

async fn ensure_server_online(
    state: &AppState,
    auth: &AuthSession,
    server_id: &str,
) -> Result<AgentId, AppError> {
    let agent_id = ensure_server_active(state, auth, server_id).await?;
    if !state.session_registry.is_online(&agent_id).await {
        return Err(AppError::BadRequest("agent is offline".into()));
    }
    Ok(agent_id)
}

async fn ensure_server_active(
    state: &AppState,
    auth: &AuthSession,
    server_id: &str,
) -> Result<AgentId, AppError> {
    let agent_id = ensure_server_visible(state, auth, server_id).await?;
    let agent = AgentRepository::new(state.db.clone())
        .find_by_id(agent_id)
        .await?
        .ok_or(AppError::NotFound("agent not found".into()))?;
    if agent.revoked_at.is_some() {
        return Err(AppError::Forbidden("agent has been revoked".into()));
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
    if trimmed.len() > SERVER_OPS_MAX_PATH_BYTES {
        return Err(AppError::BadRequest(format!(
            "path exceeds {SERVER_OPS_MAX_PATH_BYTES} bytes"
        )));
    }
    if trimmed.contains('\0') {
        return Err(AppError::BadRequest("path contains NUL byte".into()));
    }
    Ok(trimmed.to_string())
}

fn ensure_file_write_size(data: &[u8]) -> Result<(), AppError> {
    if data.len() > FILE_WRITE_MAX_BYTES {
        return Err(AppError::BadRequest(format!(
            "file write content exceeds {FILE_WRITE_MAX_BYTES} bytes"
        )));
    }
    Ok(())
}

fn ensure_file_task_result_text(
    result: &xlstatus_proto_gen::xlstatus::v1::TaskResult,
    stdout_max: usize,
    context: &str,
) -> Result<(), AppError> {
    ensure_task_result_text_within(
        result,
        stdout_max,
        FILE_SMALL_RESULT_MAX_BYTES,
        FILE_SMALL_RESULT_MAX_BYTES,
        context,
    )
    .map_err(AppError::BadRequest)
}

async fn require_remote_config_patch_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    headers: &HeaderMap,
    config: &serde_json::Value,
) -> Result<(), AppError> {
    if remote_config_patch_contains_sensitive_fields(config)? {
        require_admin_cookie_session(auth)?;
        require_sensitive_totp(db, auth.user_id, headers).await?;
    }
    Ok(())
}

fn remote_config_patch_contains_sensitive_fields(
    config: &serde_json::Value,
) -> Result<bool, AppError> {
    let Some(object) = config.as_object() else {
        return Err(AppError::BadRequest(
            "config patch must be an object".into(),
        ));
    };
    let mut sensitive = false;
    for key in object.keys() {
        if REMOTE_CONFIG_FORBIDDEN_FIELDS.contains(&key.as_str()) {
            return Err(AppError::BadRequest(format!(
                "remote config field {key} cannot be changed from the dashboard"
            )));
        }
        if REMOTE_CONFIG_SENSITIVE_FIELDS.contains(&key.as_str()) {
            sensitive = true;
            continue;
        }
        if !REMOTE_CONFIG_LOW_RISK_FIELDS.contains(&key.as_str()) {
            return Err(AppError::BadRequest(format!(
                "unknown remote config field: {key}"
            )));
        }
    }
    Ok(sensitive)
}

fn require_admin_cookie_session(auth: &AuthSession) -> Result<(), AppError> {
    if !auth.role.is_admin() {
        return Err(AppError::Forbidden("admin role required".into()));
    }
    if matches!(auth.auth_kind, AuthKind::PersonalAccessToken) {
        return Err(AppError::Forbidden("Cookie session required".into()));
    }
    Ok(())
}

fn serialize_config_patch(config: &serde_json::Value) -> Result<Vec<u8>, AppError> {
    let payload = serde_json::to_vec(config)
        .map_err(|e| AppError::BadRequest(format!("invalid config patch: {e}")))?;
    if payload.len() > CONFIG_PATCH_MAX_BYTES {
        return Err(AppError::BadRequest(format!(
            "config patch exceeds {CONFIG_PATCH_MAX_BYTES} bytes"
        )));
    }
    Ok(payload)
}

fn require_force_update_scope(auth: &AuthSession) -> Result<(), AppError> {
    if has_scope(auth, "server:exec") {
        Ok(())
    } else {
        Err(AppError::Forbidden("missing scope: server:exec".into()))
    }
}

async fn require_force_update_auth(
    db: &crate::db::Db,
    auth: &AuthSession,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    require_force_update_scope(auth)?;
    require_sensitive_totp(db, auth.user_id, headers).await
}

fn validate_force_update_request(req: ForceUpdateRequest) -> Result<ForceUpdate, AppError> {
    validate_force_update_request_with_custom_source(req, force_update_custom_source_allowed())
}

fn validate_force_update_request_with_custom_source(
    req: ForceUpdateRequest,
    allow_custom_source: bool,
) -> Result<ForceUpdate, AppError> {
    let version = validate_force_update_version(&req.version)?;
    let checksum = validate_force_update_checksum(req.checksum.as_deref())?;
    let download_url =
        validate_force_update_download_url(&req.download_url, &version, allow_custom_source)?;
    Ok(ForceUpdate {
        version,
        download_url,
        checksum,
    })
}

fn validate_force_update_version(version: &str) -> Result<String, AppError> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("version is required".into()));
    }
    if trimmed == "latest" {
        return Err(AppError::BadRequest(
            "force update requires an explicit release version".into(),
        ));
    }
    if trimmed.len() > 80
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(AppError::BadRequest(
            "version contains unsupported characters".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_force_update_checksum(checksum: Option<&str>) -> Result<String, AppError> {
    let checksum = checksum.map(str::trim).filter(|value| !value.is_empty());
    let Some(checksum) = checksum else {
        return Err(AppError::BadRequest(
            "sha256 checksum is required for force update".into(),
        ));
    };
    let normalized = checksum
        .strip_prefix("sha256:")
        .unwrap_or(checksum)
        .to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest(
            "checksum must be a sha256 hex digest".into(),
        ));
    }
    Ok(normalized)
}

fn validate_force_update_download_url(
    download_url: &str,
    version: &str,
    allow_custom_source: bool,
) -> Result<String, AppError> {
    let trimmed = download_url.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("download_url is required".into()));
    }
    if trimmed.len() > FORCE_UPDATE_MAX_URL_BYTES {
        return Err(AppError::BadRequest(format!(
            "download_url exceeds {FORCE_UPDATE_MAX_URL_BYTES} bytes"
        )));
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|e| AppError::BadRequest(format!("download_url is invalid: {e}")))?;
    if url.scheme() != "https" {
        return Err(AppError::BadRequest("download_url must use https".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::BadRequest(
            "download_url must not contain credentials".into(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::BadRequest(
            "download_url must not contain query or fragment".into(),
        ));
    }
    if !allow_custom_source {
        validate_default_force_update_download_url(&url, version)?;
    }
    Ok(url.to_string())
}

fn validate_default_force_update_download_url(
    url: &reqwest::Url,
    version: &str,
) -> Result<(), AppError> {
    if url.host_str() != Some(FORCE_UPDATE_REPO_HOST) {
        return Err(AppError::BadRequest(
            "download_url must use the XLStatus GitHub release host".into(),
        ));
    }
    if url.port().is_some() {
        return Err(AppError::BadRequest(
            "download_url must use the default https port".into(),
        ));
    }
    let path = url.path();
    let expected_prefix = format!("{FORCE_UPDATE_REPO_PATH_PREFIX}{version}/");
    if !path.starts_with(&expected_prefix) {
        return Err(AppError::BadRequest(
            "download_url must point to the requested XLStatus release version".into(),
        ));
    }
    let filename = path
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| AppError::BadRequest("download_url missing release asset name".into()))?;
    if filename.contains('/') || filename.contains('\\') || !filename.starts_with("xlstatus-agent-")
    {
        return Err(AppError::BadRequest(
            "download_url must point to an XLStatus Agent release asset".into(),
        ));
    }
    Ok(())
}

fn force_update_custom_source_allowed() -> bool {
    matches!(
        std::env::var("XLSTATUS_ALLOW_CUSTOM_FORCE_UPDATE_URL").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::db::{
        CreateAgentInput, CreateUserInput, DatabaseBackend, TemporaryTransferTokenRepository,
        UserRepository,
    };
    use xlstatus_shared::UserRole;

    fn valid_checksum() -> String {
        "a".repeat(64)
    }

    fn force_update_req(download_url: &str) -> ForceUpdateRequest {
        ForceUpdateRequest {
            version: "v0.1.0-alpha.3".into(),
            download_url: download_url.into(),
            checksum: Some(valid_checksum()),
        }
    }

    fn auth_session(
        auth_kind: AuthKind,
        role: xlstatus_shared::UserRole,
        scopes: Vec<&str>,
    ) -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: xlstatus_shared::UserId(
                uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            ),
            username: "user".into(),
            role,
            csrf_token: "csrf".into(),
            auth_kind,
            scopes: scopes.into_iter().map(str::to_string).collect(),
            server_ids: None,
            pat_id: None,
        }
    }

    #[test]
    fn force_update_requires_server_exec_scope() {
        let server_write_pat = auth_session(
            AuthKind::PersonalAccessToken,
            xlstatus_shared::UserRole::Admin,
            vec!["server:write"],
        );
        let err = require_force_update_scope(&server_write_pat).unwrap_err();
        assert!(app_error_message(&err).contains("server:exec"));

        let server_exec_pat = auth_session(
            AuthKind::PersonalAccessToken,
            xlstatus_shared::UserRole::Admin,
            vec!["server:exec"],
        );
        assert!(require_force_update_scope(&server_exec_pat).is_ok());

        let admin_cookie = auth_session(
            AuthKind::Session,
            xlstatus_shared::UserRole::Admin,
            Vec::new(),
        );
        assert!(require_force_update_scope(&admin_cookie).is_ok());
    }

    #[tokio::test]
    async fn force_update_requires_sensitive_totp_when_enabled() {
        let state = test_state().await;
        let user = UserRepository::new(state.db.clone())
            .create(CreateUserInput {
                username: "admin".into(),
                password: "secret-password".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        seed_totp_enabled_user(&state.db, user.id.0).await;
        let auth = AuthSession {
            session_id: "sess".into(),
            user_id: user.id,
            username: user.username,
            role: user.role,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: Vec::new(),
            server_ids: None,
            pat_id: None,
        };
        let err = require_force_update_auth(&state.db, &auth, &HeaderMap::new())
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn server_ops_resource_limits_are_explicit() {
        assert_eq!(SERVER_OPS_API_MAX_BODY_BYTES, 3 * 1024 * 1024);
        assert_eq!(SERVER_OPS_UUID_TEXT_LEN, 36);
        assert_eq!(FILE_WRITE_MAX_BYTES, 2 * 1024 * 1024);
        assert_eq!(CONFIG_PATCH_MAX_BYTES, 128 * 1024);
        assert_eq!(FILE_LIST_RESULT_MAX_BYTES, 2 * 1024 * 1024);
        assert_eq!(FILE_SMALL_RESULT_MAX_BYTES, 4096);
    }

    #[test]
    fn remote_config_patch_fields_are_classified() {
        assert!(
            !remote_config_patch_contains_sensitive_fields(&serde_json::json!({
                "name": "agent",
                "report_interval_seconds": 30,
            }))
            .unwrap()
        );
        assert!(
            remote_config_patch_contains_sensitive_fields(&serde_json::json!({
                "disable_command_execute": true,
            }))
            .unwrap()
        );
        assert!(
            remote_config_patch_contains_sensitive_fields(&serde_json::json!({
                "file_allowed_roots": ["/var/lib/xlstatus/files"],
            }))
            .unwrap()
        );
        assert!(
            remote_config_patch_contains_sensitive_fields(&serde_json::json!({
                "agent_id": "00000000-0000-0000-0000-000000000001",
            }))
            .is_err()
        );
        assert!(
            remote_config_patch_contains_sensitive_fields(&serde_json::json!({
                "private_key": "secret",
            }))
            .is_err()
        );
        assert!(
            remote_config_patch_contains_sensitive_fields(&serde_json::json!({
                "unexpected": true,
            }))
            .is_err()
        );
        assert!(remote_config_patch_contains_sensitive_fields(&serde_json::json!(true)).is_err());
    }

    #[tokio::test]
    async fn sensitive_remote_config_patch_rejects_pat() {
        let state = test_state().await;
        let auth = auth_session(
            AuthKind::PersonalAccessToken,
            xlstatus_shared::UserRole::Admin,
            vec!["server:write"],
        );
        let err = require_remote_config_patch_auth(
            &state.db,
            &auth,
            &HeaderMap::new(),
            &serde_json::json!({ "disable_command_execute": true }),
        )
        .await
        .unwrap_err();

        assert!(app_error_message(&err).contains("Cookie session required"));
    }

    #[tokio::test]
    async fn sensitive_remote_config_patch_requires_totp_when_enabled() {
        let state = test_state().await;
        let user = UserRepository::new(state.db.clone())
            .create(CreateUserInput {
                username: "admin".into(),
                password: "secret-password".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        seed_totp_enabled_user(&state.db, user.id.0).await;
        let auth = AuthSession {
            session_id: "sess".into(),
            user_id: user.id,
            username: user.username,
            role: user.role,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: Vec::new(),
            server_ids: None,
            pat_id: None,
        };
        let err = require_remote_config_patch_auth(
            &state.db,
            &auth,
            &HeaderMap::new(),
            &serde_json::json!({ "disable_nat": true }),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn low_risk_remote_config_patch_allows_pat() {
        let state = test_state().await;
        let auth = auth_session(
            AuthKind::PersonalAccessToken,
            xlstatus_shared::UserRole::Admin,
            vec!["server:write"],
        );

        require_remote_config_patch_auth(
            &state.db,
            &auth,
            &HeaderMap::new(),
            &serde_json::json!({ "report_interval_seconds": 30 }),
        )
        .await
        .unwrap();
    }

    #[test]
    fn server_ops_path_ids_require_canonical_uuid_text() {
        let server_id = uuid::Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();

        assert_eq!(
            parse_server_ops_agent_id(&server_id.to_string()).unwrap(),
            server_id
        );
        assert!(parse_server_ops_agent_id("server-a").is_err());
        assert!(parse_server_ops_agent_id(&format!(" {} ", server_id)).is_err());
        assert!(parse_server_ops_agent_id(&server_id.simple().to_string()).is_err());
        assert!(parse_server_ops_agent_id(&server_id.to_string().to_uppercase()).is_err());
        assert!(parse_server_ops_agent_id(&"a".repeat(SERVER_OPS_UUID_TEXT_LEN + 1)).is_err());
    }

    #[test]
    fn file_path_and_write_content_have_resource_bounds() {
        assert!(validate_abs_path("/var/log/app.log").is_ok());
        assert!(validate_abs_path(&format!("/{}", "a".repeat(SERVER_OPS_MAX_PATH_BYTES))).is_err());
        assert!(ensure_file_write_size(&vec![0_u8; FILE_WRITE_MAX_BYTES]).is_ok());
        assert!(ensure_file_write_size(&vec![0_u8; FILE_WRITE_MAX_BYTES + 1]).is_err());
    }

    #[test]
    fn config_patch_has_resource_bounds() {
        let config = serde_json::json!({ "name": "agent-1" });
        assert!(serialize_config_patch(&config).is_ok());

        let oversized = serde_json::json!({ "blob": "a".repeat(CONFIG_PATCH_MAX_BYTES) });
        let err = serialize_config_patch(&oversized).unwrap_err();
        assert!(app_error_message(&err).contains("config patch exceeds"));
    }

    #[test]
    fn file_task_result_text_has_business_bounds() {
        let mut result = xlstatus_proto_gen::xlstatus::v1::TaskResult {
            stdout: "ok".into(),
            stderr: String::new(),
            error: String::new(),
            ..Default::default()
        };
        assert!(ensure_file_task_result_text(&result, 2, "file op").is_ok());

        result.stdout = "x".repeat(3);
        let err = ensure_file_task_result_text(&result, 2, "file op").unwrap_err();
        assert!(app_error_message(&err).contains("stdout exceeds 2 bytes"));

        result.stdout = "ok".into();
        result.stderr = "x".repeat(FILE_SMALL_RESULT_MAX_BYTES + 1);
        let err = ensure_file_task_result_text(&result, 2, "file op").unwrap_err();
        assert!(app_error_message(&err).contains("stderr exceeds"));
    }

    #[test]
    fn force_update_accepts_project_agent_release_asset() {
        let update = validate_force_update_request_with_custom_source(force_update_req(
            "https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/xlstatus-agent-linux-amd64.tar.gz",
        ), false)
        .unwrap();

        assert_eq!(update.version, "v0.1.0-alpha.3");
        assert_eq!(update.checksum, valid_checksum());
    }

    #[test]
    fn force_update_requires_sha256_checksum() {
        let err = validate_force_update_request_with_custom_source(ForceUpdateRequest {
            version: "v0.1.0-alpha.3".into(),
            download_url: "https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/xlstatus-agent-linux-amd64.tar.gz".into(),
            checksum: None,
        }, false)
        .unwrap_err();

        assert!(app_error_message(&err).contains("sha256 checksum is required"));
    }

    #[test]
    fn force_update_rejects_latest_version() {
        let err = validate_force_update_request_with_custom_source(ForceUpdateRequest {
            version: "latest".into(),
            download_url: "https://github.com/lbyxiaolizi/XLStatus/releases/download/latest/xlstatus-agent-linux-amd64.tar.gz".into(),
            checksum: Some(valid_checksum()),
        }, false)
        .unwrap_err();

        assert!(app_error_message(&err).contains("explicit release version"));
    }

    #[test]
    fn force_update_rejects_oversized_download_url() {
        let err = validate_force_update_download_url(
            &format!(
                "https://updates.example.net/{}",
                "a".repeat(FORCE_UPDATE_MAX_URL_BYTES)
            ),
            "v0.1.0-alpha.3",
            true,
        )
        .unwrap_err();

        assert!(app_error_message(&err).contains("download_url exceeds"));
    }

    #[test]
    fn force_update_rejects_non_project_release_host_by_default() {
        let err = validate_force_update_request_with_custom_source(force_update_req(
            "https://example.com/releases/download/v0.1.0-alpha.3/xlstatus-agent-linux-amd64.tar.gz",
        ), false)
        .unwrap_err();

        assert!(app_error_message(&err).contains("GitHub release host"));
    }

    #[test]
    fn force_update_rejects_wrong_release_version_in_url() {
        let err = validate_force_update_request_with_custom_source(force_update_req(
            "https://github.com/lbyxiaolizi/XLStatus/releases/download/v9.9.9/xlstatus-agent-linux-amd64.tar.gz",
        ), false)
        .unwrap_err();

        assert!(app_error_message(&err).contains("requested XLStatus release version"));
    }

    #[test]
    fn force_update_rejects_non_agent_release_asset() {
        let err = validate_force_update_request_with_custom_source(force_update_req(
            "https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/install-agent.sh",
        ), false)
        .unwrap_err();

        assert!(app_error_message(&err).contains("XLStatus Agent release asset"));
    }

    #[test]
    fn force_update_rejects_url_credentials_query_and_fragment() {
        for url in [
            "https://user@github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/xlstatus-agent-linux-amd64.tar.gz",
            "https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/xlstatus-agent-linux-amd64.tar.gz?token=secret",
            "https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/xlstatus-agent-linux-amd64.tar.gz#sha256",
        ] {
            assert!(
                validate_force_update_request_with_custom_source(force_update_req(url), false)
                    .is_err(),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn force_update_custom_source_escape_hatch_still_requires_https_and_checksum() {
        let update = validate_force_update_request_with_custom_source(
            force_update_req("https://updates.example.net/xlstatus-agent-linux-amd64.tar.gz"),
            true,
        )
        .unwrap();
        assert_eq!(
            update.download_url,
            "https://updates.example.net/xlstatus-agent-linux-amd64.tar.gz"
        );

        let err = validate_force_update_request_with_custom_source(
            force_update_req("http://updates.example.net/xlstatus-agent-linux-amd64.tar.gz"),
            true,
        )
        .unwrap_err();
        assert!(app_error_message(&err).contains("https"));
    }

    #[tokio::test]
    async fn temp_url_signing_rejects_revoked_agent_before_creating_token() {
        let state = test_state().await;
        let user = UserRepository::new(state.db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret-password".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let agent_repo = AgentRepository::new(state.db.clone());
        let agent = agent_repo
            .create(CreateAgentInput {
                name: "revoked-temp-url-agent".into(),
                public_key: "public".into(),
                owner_user_id: user.id,
            })
            .await
            .unwrap();
        assert!(agent_repo.revoke(agent.id).await.unwrap());
        let auth = AuthSession {
            session_id: "sess".into(),
            user_id: user.id,
            username: user.username,
            role: user.role,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: Vec::new(),
            server_ids: None,
            pat_id: None,
        };

        let err = build_temp_url(
            state.clone(),
            auth,
            agent.id.0.to_string(),
            TempUrlRequest {
                path: "/tmp/file.txt".into(),
                expires_in: 300,
            },
            "download",
            "GET",
        )
        .await
        .unwrap_err();

        assert!(app_error_message(&err).contains("agent has been revoked"));
        let (_, total) = TemporaryTransferTokenRepository::new(state.db.clone())
            .list(10, 0)
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn online_agent_operations_reject_revoked_agent_before_online_check() {
        let state = test_state().await;
        let user = UserRepository::new(state.db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret-password".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let agent_repo = AgentRepository::new(state.db.clone());
        let agent = agent_repo
            .create(CreateAgentInput {
                name: "revoked-online-agent".into(),
                public_key: "public".into(),
                owner_user_id: user.id,
            })
            .await
            .unwrap();
        assert!(agent_repo.revoke(agent.id).await.unwrap());
        let auth = AuthSession {
            session_id: "sess".into(),
            user_id: user.id,
            username: user.username,
            role: user.role,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: Vec::new(),
            server_ids: None,
            pat_id: None,
        };

        let err = ensure_server_online(&state, &auth, &agent.id.0.to_string())
            .await
            .unwrap_err();

        assert!(app_error_message(&err).contains("agent has been revoked"));
    }

    async fn test_state() -> AppState {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();

        AppState {
            db,
            config: Arc::new(crate::config::Config::default()),
            agent_jwt_challenges: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            metrics: xlstatus_tsdb::MetricStore::in_memory(),
            realtime: crate::realtime::BroadcastHub::new(),
            session_registry: crate::grpc::SessionRegistry::new(),
            terminal_sessions: crate::api::v1::terminal::TerminalSessionRegistry::new(),
            io_registry: crate::grpc::IoRegistry::new(),
        }
    }

    async fn seed_totp_enabled_user(db: &DatabaseBackend, id: uuid::Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query("UPDATE users SET totp_secret = ?, totp_enabled = 1 WHERE id = ?")
            .bind("totp-secret")
            .bind(id.to_string())
            .execute(pool)
            .await
            .unwrap();
    }

    fn app_error_message(err: &AppError) -> String {
        match err {
            AppError::BadRequest(message)
            | AppError::Forbidden(message)
            | AppError::Unauthorized(message)
            | AppError::NotFound(message)
            | AppError::TooManyRequests(message) => message.clone(),
            AppError::Database(err) => err.to_string(),
        }
    }
}
