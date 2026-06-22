use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{connect_info::ConnectInfo, DefaultBodyLimit, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use xlstatus_proto_gen::xlstatus::v1::{
    server_task::Spec, FileReadTask, FileWriteTask, ServerTask, TaskOutcome, TaskResult, TaskType,
};
use xlstatus_shared::AgentId;

use crate::api::types::ApiResponse;
use crate::api::v1::auth::AppState;
use crate::auth::hash_token;
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::auth::rbac::has_scope;
use crate::auth::SessionRepository;
use crate::db::{
    AgentRepository, PATRepository, TemporaryTransferToken, TemporaryTransferTokenRepository,
    UserRepository,
};
use crate::grpc::{base64_encoded_len, ensure_task_result_text_within};

const TEMP_TRANSFER_MAX_BYTES: usize = 100 * 1024 * 1024;
const TEMP_TRANSFER_MAX_QUERY_BYTES: usize = 512;
const DOWNLOAD_TIMEOUT_SECS: u64 = 60;
const UPLOAD_TIMEOUT_SECS: u64 = 120;
const TEMP_TRANSFER_RATE_LIMIT: u32 = 10;
const TEMP_TRANSFER_RATE_WINDOW_SECS: u64 = 60;
const TEMP_TRANSFER_SMALL_RESULT_MAX_BYTES: usize = 4096;

static TEMP_TRANSFER_RATE_STATE: once_cell::sync::Lazy<
    std::sync::Mutex<HashMap<String, (std::time::Instant, u32)>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub struct TempTransferQuery {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct ListTemporaryTransfersQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Debug, Serialize)]
pub struct TemporaryTransferTokenView {
    pub id: String,
    pub server_id: String,
    pub path: String,
    pub op: String,
    pub issued_by_user_id: String,
    pub auth_kind: String,
    pub session_id: Option<String>,
    pub api_token_id: Option<String>,
    pub scope: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub used_ip: Option<String>,
    pub used_status: Option<String>,
    pub used_error: Option<String>,
    pub agent_task_id: Option<String>,
    pub revoked_at: Option<String>,
    pub created_at: String,
    pub created_ip: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TemporaryTransferTokenListResponse {
    pub tokens: Vec<TemporaryTransferTokenView>,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct RevokeTemporaryTransferResponse {
    pub id: String,
    pub revoked: bool,
}

pub async fn list_temporary_transfers(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(q): Query<ListTemporaryTransfersQuery>,
) -> Result<Json<ApiResponse<TemporaryTransferTokenListResponse>>, crate::api::v1::auth::AppError> {
    require_transfer_scope_app(&auth, "transfer:read")?;
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);
    let repo = TemporaryTransferTokenRepository::new(state.db.clone());
    let (tokens, total) = if auth.role.is_admin() && auth.server_ids.is_none() {
        repo.list(limit, offset).await?
    } else if let Some(server_ids) = auth.server_ids.as_deref() {
        repo.list_for_owner_server_ids(auth.user_id, server_ids, limit, offset)
            .await?
    } else {
        repo.list_for_owner(auth.user_id, limit, offset).await?
    };
    Ok(Json(ApiResponse::success(
        TemporaryTransferTokenListResponse {
            tokens: tokens
                .into_iter()
                .map(temporary_transfer_token_view)
                .collect(),
            total,
        },
    )))
}

pub async fn revoke_temporary_transfer(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<RevokeTemporaryTransferResponse>>, crate::api::v1::auth::AppError> {
    require_transfer_scope_app(&auth, "transfer:write")?;
    let repo = TemporaryTransferTokenRepository::new(state.db.clone());
    let record = repo
        .find_by_id(&id)
        .await?
        .ok_or(crate::api::v1::auth::AppError::NotFound(
            "temporary transfer token not found".into(),
        ))?;
    if !temporary_transfer_visible_to_auth(&state, &auth, &record).await? {
        return Err(crate::api::v1::auth::AppError::Forbidden(
            "temporary transfer token not in scope".into(),
        ));
    }
    let revoked = repo.revoke(&id).await?;
    Ok(Json(ApiResponse::success(
        RevokeTemporaryTransferResponse { id, revoked },
    )))
}

pub async fn temp_download(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let query = match parse_temp_transfer_query(&uri) {
        Ok(query) => query,
        Err((status, message)) => return json_error(status, message),
    };
    let client_ip = crate::security::client_ip_from_headers_and_peer(&headers, Some(peer_addr));
    match run_temp_download(state, query, client_ip).await {
        Ok(response) => response,
        Err((status, message)) => json_error(status, message),
    }
}

fn temporary_transfer_token_view(token: TemporaryTransferToken) -> TemporaryTransferTokenView {
    TemporaryTransferTokenView {
        id: token.id,
        server_id: token.server_id.0.to_string(),
        path: token.path,
        op: token.op,
        issued_by_user_id: token.issued_by_user_id.0.to_string(),
        auth_kind: token.auth_kind,
        session_id: token.session_id,
        api_token_id: token.api_token_id,
        scope: token.scope,
        expires_at: token.expires_at.to_rfc3339(),
        used_at: token.used_at.map(|ts| ts.to_rfc3339()),
        used_ip: token.used_ip,
        used_status: token.used_status,
        used_error: token.used_error,
        agent_task_id: token.agent_task_id,
        revoked_at: token.revoked_at.map(|ts| ts.to_rfc3339()),
        created_at: token.created_at.to_rfc3339(),
        created_ip: token.created_ip,
    }
}

async fn temporary_transfer_visible_to_auth(
    state: &AppState,
    auth: &AuthSession,
    token: &TemporaryTransferToken,
) -> Result<bool, crate::api::v1::auth::AppError> {
    if auth.role.is_admin() && auth.server_ids.is_none() {
        return Ok(true);
    }
    if token.issued_by_user_id != auth.user_id {
        return Ok(false);
    }
    let agent = AgentRepository::new(state.db.clone())
        .find_by_id(token.server_id)
        .await?
        .ok_or(crate::api::v1::auth::AppError::NotFound(
            "agent not found".into(),
        ))?;
    Ok(crate::api::v1::servers::agent_visible(auth, &agent))
}

fn require_transfer_scope_app(
    auth: &AuthSession,
    scope: &str,
) -> Result<(), crate::api::v1::auth::AppError> {
    if has_scope(auth, scope) {
        Ok(())
    } else {
        Err(crate::api::v1::auth::AppError::Forbidden(format!(
            "missing scope: {scope}"
        )))
    }
}

pub async fn temp_upload(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    uri: Uri,
    body: Body,
) -> Response {
    let query = match parse_temp_transfer_query(&uri) {
        Ok(query) => query,
        Err((status, message)) => return json_error(status, message),
    };
    let client_ip = crate::security::client_ip_from_headers_and_peer(&headers, Some(peer_addr));
    match run_temp_upload(state, query, client_ip, body).await {
        Ok(response) => response,
        Err((status, message)) => json_error(status, message),
    }
}

async fn run_temp_download(
    state: AppState,
    query: TempTransferQuery,
    client_ip: String,
) -> Result<Response, (StatusCode, String)> {
    let transfer = validate_temporary_transfer(&state, &query.token, "download").await?;
    let path = transfer.path;
    let server_id = transfer.server_id;
    check_rate_limit(&query.token)?;
    consume_temporary_transfer(&state, &transfer.token_id, Some(&client_ip)).await?;
    let dispatched = match dispatch_server_task(
        &state,
        &server_id,
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
    .await
    {
        Ok(dispatched) => dispatched,
        Err(err) => {
            let task_id = err.agent_task_id.as_deref();
            record_temporary_transfer_result(
                &state,
                &transfer.token_id,
                "failed",
                task_id,
                Some(&err.message),
            )
            .await;
            return Err(err.into_response_error());
        }
    };
    if let Err(err) = ensure_temporary_transfer_result_text(
        &dispatched.result,
        base64_encoded_len(TEMP_TRANSFER_MAX_BYTES),
        "temporary download agent result",
    ) {
        record_temporary_transfer_result(
            &state,
            &transfer.token_id,
            "failed",
            Some(&dispatched.run_id),
            Some(&err.1),
        )
        .await;
        return Err(err);
    }
    if let Err(err) = ensure_task_success(&dispatched.result) {
        record_temporary_transfer_result(
            &state,
            &transfer.token_id,
            "failed",
            Some(&dispatched.run_id),
            Some(&err.1),
        )
        .await;
        return Err(err);
    }

    let data = match base64_decode(dispatched.result.stdout.trim()) {
        Ok(data) => data,
        Err(e) => {
            let err = (
                StatusCode::BAD_GATEWAY,
                format!("agent returned invalid base64 file data: {e}"),
            );
            record_temporary_transfer_result(
                &state,
                &transfer.token_id,
                "failed",
                Some(&dispatched.run_id),
                Some(&err.1),
            )
            .await;
            return Err(err);
        }
    };
    if data.len() > TEMP_TRANSFER_MAX_BYTES {
        let err = (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("file is larger than {} bytes", TEMP_TRANSFER_MAX_BYTES),
        );
        record_temporary_transfer_result(
            &state,
            &transfer.token_id,
            "failed",
            Some(&dispatched.run_id),
            Some(&err.1),
        )
        .await;
        return Err(err);
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
        match HeaderValue::from_str(&data.len().to_string()) {
            Ok(value) => value,
            Err(e) => {
                let err = (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
                record_temporary_transfer_result(
                    &state,
                    &transfer.token_id,
                    "failed",
                    Some(&dispatched.run_id),
                    Some(&err.1),
                )
                .await;
                return Err(err);
            }
        },
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        match HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            sanitize_filename(filename)
        )) {
            Ok(value) => value,
            Err(e) => {
                let err = (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
                record_temporary_transfer_result(
                    &state,
                    &transfer.token_id,
                    "failed",
                    Some(&dispatched.run_id),
                    Some(&err.1),
                )
                .await;
                return Err(err);
            }
        },
    );
    record_temporary_transfer_result(
        &state,
        &transfer.token_id,
        "success",
        Some(&dispatched.run_id),
        None,
    )
    .await;
    Ok((StatusCode::OK, headers, data).into_response())
}

pub fn upload_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(TEMP_TRANSFER_MAX_BYTES)
}

async fn run_temp_upload(
    state: AppState,
    query: TempTransferQuery,
    client_ip: String,
    body: Body,
) -> Result<Response, (StatusCode, String)> {
    let transfer = validate_temporary_transfer(&state, &query.token, "upload").await?;
    let path = transfer.path;
    let server_id = transfer.server_id;
    check_rate_limit(&query.token)?;
    consume_temporary_transfer(&state, &transfer.token_id, Some(&client_ip)).await?;
    let body = match read_limited_upload_body(body).await {
        Ok(body) => body,
        Err(err) => {
            record_temporary_transfer_result(
                &state,
                &transfer.token_id,
                "failed",
                None,
                Some(&err.1),
            )
            .await;
            return Err(err);
        }
    };
    let dispatched = match dispatch_server_task(
        &state,
        &server_id,
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
    .await
    {
        Ok(dispatched) => dispatched,
        Err(err) => {
            let task_id = err.agent_task_id.as_deref();
            record_temporary_transfer_result(
                &state,
                &transfer.token_id,
                "failed",
                task_id,
                Some(&err.message),
            )
            .await;
            return Err(err.into_response_error());
        }
    };
    if let Err(err) = ensure_temporary_transfer_result_text(
        &dispatched.result,
        TEMP_TRANSFER_SMALL_RESULT_MAX_BYTES,
        "temporary upload agent result",
    ) {
        record_temporary_transfer_result(
            &state,
            &transfer.token_id,
            "failed",
            Some(&dispatched.run_id),
            Some(&err.1),
        )
        .await;
        return Err(err);
    }
    if let Err(err) = ensure_task_success(&dispatched.result) {
        record_temporary_transfer_result(
            &state,
            &transfer.token_id,
            "failed",
            Some(&dispatched.run_id),
            Some(&err.1),
        )
        .await;
        return Err(err);
    }
    let written = match dispatched.result.stdout.trim().parse::<usize>() {
        Ok(written) => written,
        Err(_) => {
            let err = (
                StatusCode::BAD_GATEWAY,
                "agent returned invalid byte count".to_string(),
            );
            record_temporary_transfer_result(
                &state,
                &transfer.token_id,
                "failed",
                Some(&dispatched.run_id),
                Some(&err.1),
            )
            .await;
            return Err(err);
        }
    };
    if written != body.len() {
        let err = (
            StatusCode::BAD_GATEWAY,
            format!("agent wrote {} bytes, expected {}", written, body.len()),
        );
        record_temporary_transfer_result(
            &state,
            &transfer.token_id,
            "failed",
            Some(&dispatched.run_id),
            Some(&err.1),
        )
        .await;
        return Err(err);
    }
    record_temporary_transfer_result(
        &state,
        &transfer.token_id,
        "success",
        Some(&dispatched.run_id),
        None,
    )
    .await;
    Ok(Json(ApiResponse::success(serde_json::json!({
        "server_id": server_id,
        "path": path,
        "bytes_written": written,
    })))
    .into_response())
}

async fn read_limited_upload_body(body: Body) -> Result<Bytes, (StatusCode, String)> {
    read_limited_body(body, TEMP_TRANSFER_MAX_BYTES).await
}

fn parse_temp_transfer_query(uri: &Uri) -> Result<TempTransferQuery, (StatusCode, String)> {
    let raw_query = uri.query().unwrap_or_default();
    if raw_query.len() > TEMP_TRANSFER_MAX_QUERY_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "temporary transfer query must be at most {TEMP_TRANSFER_MAX_QUERY_BYTES} bytes"
            ),
        ));
    }
    Query::<TempTransferQuery>::try_from_uri(uri)
        .map(|Query(query)| query)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "temporary transfer query is invalid".to_string(),
            )
        })
}

async fn read_limited_body(body: Body, max_bytes: usize) -> Result<Bytes, (StatusCode, String)> {
    let limit = max_bytes.saturating_add(1);
    let body = to_bytes(body, limit).await.map_err(|err| {
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "upload is larger than {} bytes or could not be read: {err}",
                max_bytes
            ),
        )
    })?;
    if body.len() > max_bytes {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("upload is larger than {} bytes", max_bytes),
        ));
    }
    Ok(body)
}

#[derive(Debug)]
struct ValidatedTemporaryTransfer {
    token_id: String,
    server_id: String,
    path: String,
}

async fn validate_temporary_transfer(
    state: &AppState,
    token: &str,
    op: &str,
) -> Result<ValidatedTemporaryTransfer, (StatusCode, String)> {
    if !valid_temporary_token_shape(token) {
        return Err((
            StatusCode::FORBIDDEN,
            "invalid or expired temporary URL".to_string(),
        ));
    }
    let token_hash = hash_token(token);
    let repo = TemporaryTransferTokenRepository::new(state.db.clone());
    let Some(record) = repo
        .find_by_token_hash(&token_hash)
        .await
        .map_err(internal_error)?
    else {
        return Err((
            StatusCode::FORBIDDEN,
            "invalid or expired temporary URL".to_string(),
        ));
    };
    validate_transfer_record_state(&record, op)?;
    validate_transfer_issuer(state, &record).await?;
    Ok(ValidatedTemporaryTransfer {
        token_id: record.id,
        server_id: record.server_id.0.to_string(),
        path: record.path,
    })
}

fn validate_transfer_record_state(
    record: &TemporaryTransferToken,
    op: &str,
) -> Result<(), (StatusCode, String)> {
    if record.op != op
        || record.expires_at <= Utc::now()
        || record.used_at.is_some()
        || record.revoked_at.is_some()
        || !record.path.starts_with('/')
    {
        return Err((
            StatusCode::FORBIDDEN,
            "invalid or expired temporary URL".to_string(),
        ));
    }
    Ok(())
}

async fn validate_transfer_issuer(
    state: &AppState,
    record: &TemporaryTransferToken,
) -> Result<(), (StatusCode, String)> {
    let user = UserRepository::new(state.db.clone())
        .find_by_id(record.issued_by_user_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "temporary URL issuer is invalid".to_string(),
            )
        })?;
    let auth = match record.auth_kind.as_str() {
        "session" => {
            let session_id = record.session_id.as_deref().ok_or_else(|| {
                (
                    StatusCode::FORBIDDEN,
                    "temporary URL issuer is invalid".to_string(),
                )
            })?;
            let session = SessionRepository::new(state.db.clone())
                .find_by_id(session_id)
                .await
                .map_err(internal_error)?
                .ok_or_else(|| {
                    (
                        StatusCode::FORBIDDEN,
                        "temporary URL issuer is invalid".to_string(),
                    )
                })?;
            if session.user_id != record.issued_by_user_id {
                return Err((
                    StatusCode::FORBIDDEN,
                    "temporary URL issuer is invalid".to_string(),
                ));
            }
            AuthSession {
                session_id: session.id,
                user_id: user.id,
                username: user.username.clone(),
                role: user.role,
                csrf_token: String::new(),
                auth_kind: AuthKind::Session,
                scopes: Vec::new(),
                server_ids: None,
                pat_id: None,
            }
        }
        "pat" => {
            let pat_id = record.api_token_id.as_deref().ok_or_else(|| {
                (
                    StatusCode::FORBIDDEN,
                    "temporary URL issuer is invalid".to_string(),
                )
            })?;
            let pat = PATRepository::new(state.db.clone())
                .find_by_id(pat_id)
                .await
                .map_err(internal_error)?
                .ok_or_else(|| {
                    (
                        StatusCode::FORBIDDEN,
                        "temporary URL issuer is invalid".to_string(),
                    )
                })?;
            if pat.user_id != record.issued_by_user_id
                || pat.revoked_at.is_some()
                || pat.expires_at.map(|ts| ts <= Utc::now()).unwrap_or(true)
                || crate::auth::rbac::validate_pat_runtime(
                    &pat.scopes,
                    user.role.is_admin(),
                    pat.server_ids.as_deref(),
                )
                .is_err()
            {
                return Err((
                    StatusCode::FORBIDDEN,
                    "temporary URL issuer is invalid".to_string(),
                ));
            }
            AuthSession {
                session_id: pat.id.clone(),
                user_id: user.id,
                username: user.username.clone(),
                role: user.role,
                csrf_token: String::new(),
                auth_kind: AuthKind::PersonalAccessToken,
                scopes: pat.scopes,
                server_ids: pat.server_ids,
                pat_id: Some(pat.id),
            }
        }
        _ => {
            return Err((
                StatusCode::FORBIDDEN,
                "temporary URL issuer is invalid".to_string(),
            ));
        }
    };
    if !has_scope(&auth, &record.scope) {
        return Err((
            StatusCode::FORBIDDEN,
            "temporary URL issuer no longer has permission".to_string(),
        ));
    }
    let agent = AgentRepository::new(state.db.clone())
        .find_by_id(record.server_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "temporary URL server is invalid".to_string(),
            )
        })?;
    if agent.revoked_at.is_some() {
        return Err((
            StatusCode::FORBIDDEN,
            "temporary URL server is revoked".to_string(),
        ));
    }
    if !crate::api::v1::servers::agent_visible(&auth, &agent) {
        return Err((
            StatusCode::FORBIDDEN,
            "temporary URL issuer no longer has server access".to_string(),
        ));
    }
    Ok(())
}

async fn consume_temporary_transfer(
    state: &AppState,
    token_id: &str,
    used_ip: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if TemporaryTransferTokenRepository::new(state.db.clone())
        .mark_used_once(token_id, used_ip)
        .await
        .map_err(internal_error)?
    {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "invalid or already used temporary URL".to_string(),
        ))
    }
}

async fn record_temporary_transfer_result(
    state: &AppState,
    token_id: &str,
    status: &str,
    agent_task_id: Option<&str>,
    error: Option<&str>,
) {
    if let Err(err) = TemporaryTransferTokenRepository::new(state.db.clone())
        .record_use_result(token_id, status, agent_task_id, error)
        .await
    {
        tracing::warn!(
            token_id = %token_id,
            status = %status,
            "failed to record temporary transfer usage result: {err}"
        );
    }
}

fn valid_temporary_token_shape(token: &str) -> bool {
    token
        .strip_prefix("xlt_")
        .map(|body| body.len() == 64 && body.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or(false)
}

fn internal_error(err: anyhow::Error) -> (StatusCode, String) {
    tracing::warn!("temporary transfer validation failed: {}", err);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "temporary transfer validation failed".to_string(),
    )
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
) -> Result<DispatchedServerTaskResult, DispatchServerTaskError> {
    let agent_uuid = uuid::Uuid::parse_str(server_id).map_err(|_| {
        DispatchServerTaskError::new(StatusCode::BAD_REQUEST, "invalid server_id", None)
    })?;
    let agent_id = AgentId(agent_uuid);
    if !state.session_registry.is_online(&agent_id).await {
        return Err(DispatchServerTaskError::new(
            StatusCode::BAD_GATEWAY,
            "agent is offline",
            None,
        ));
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
        return Err(DispatchServerTaskError::new(
            StatusCode::BAD_GATEWAY,
            e,
            Some(run_id),
        ));
    }
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_seconds), rx).await {
        Ok(Ok(result)) => Ok(DispatchedServerTaskResult { run_id, result }),
        Ok(Err(_)) => Err(DispatchServerTaskError::new(
            StatusCode::BAD_GATEWAY,
            "agent disconnected before reply",
            Some(run_id),
        )),
        Err(_) => {
            response_registry.cancel(&run_id).await;
            Err(DispatchServerTaskError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "temporary transfer timed out",
                Some(run_id),
            ))
        }
    }
}

struct DispatchedServerTaskResult {
    run_id: String,
    result: TaskResult,
}

struct DispatchServerTaskError {
    status: StatusCode,
    message: String,
    agent_task_id: Option<String>,
}

impl DispatchServerTaskError {
    fn new(status: StatusCode, message: impl Into<String>, agent_task_id: Option<String>) -> Self {
        Self {
            status,
            message: message.into(),
            agent_task_id,
        }
    }

    fn into_response_error(self) -> (StatusCode, String) {
        (self.status, self.message)
    }
}

fn ensure_task_success(result: &TaskResult) -> Result<(), (StatusCode, String)> {
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

fn ensure_temporary_transfer_result_text(
    result: &TaskResult,
    stdout_max: usize,
    context: &str,
) -> Result<(), (StatusCode, String)> {
    ensure_task_result_text_within(
        result,
        stdout_max,
        TEMP_TRANSFER_SMALL_RESULT_MAX_BYTES,
        TEMP_TRANSFER_SMALL_RESULT_MAX_BYTES,
        context,
    )
    .map_err(|message| (StatusCode::BAD_GATEWAY, message))
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::sync::Arc;
    use xlstatus_shared::{UserId, UserRole};

    use crate::db::{
        CreateAgentInput, CreateSessionInput, CreateTemporaryTransferTokenInput, CreateUserInput,
        DatabaseBackend, UserRepository,
    };

    #[test]
    fn temporary_transfer_view_does_not_expose_token_hash() {
        let now = Utc::now();
        let view = temporary_transfer_token_view(TemporaryTransferToken {
            id: "tok-id".into(),
            token_hash: "secret-hash".into(),
            server_id: AgentId(uuid::Uuid::from_bytes([1; 16])),
            path: "/tmp/file.txt".into(),
            op: "download".into(),
            issued_by_user_id: UserId(uuid::Uuid::from_bytes([2; 16])),
            auth_kind: "session".into(),
            session_id: Some("sess".into()),
            api_token_id: None,
            scope: "transfer:read".into(),
            expires_at: now + Duration::minutes(5),
            used_at: None,
            used_ip: Some("203.0.113.10".into()),
            used_status: Some("success".into()),
            used_error: None,
            agent_task_id: Some("task-1".into()),
            revoked_at: None,
            created_at: now,
            created_ip: Some("127.0.0.1".into()),
        });

        let serialized = serde_json::to_value(view).unwrap();
        assert!(serialized.get("token_hash").is_none());
        assert_eq!(serialized["id"], "tok-id");
        assert_eq!(serialized["path"], "/tmp/file.txt");
        assert_eq!(serialized["used_ip"], "203.0.113.10");
        assert_eq!(serialized["used_status"], "success");
        assert_eq!(serialized["agent_task_id"], "task-1");
    }

    #[test]
    fn temporary_transfer_query_resource_budget_is_bounded_before_deserialize() {
        assert_eq!(TEMP_TRANSFER_MAX_QUERY_BYTES, 512);
        assert_eq!(TEMP_TRANSFER_SMALL_RESULT_MAX_BYTES, 4096);

        let uri: Uri = format!(
            "/api/v1/transfers/temp/download?token={}",
            "a".repeat(TEMP_TRANSFER_MAX_QUERY_BYTES)
        )
        .parse()
        .unwrap();
        let err = parse_temp_transfer_query(&uri).unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("temporary transfer query"));

        let token = format!("xlt_{}", "a".repeat(64));
        let uri: Uri = format!("/api/v1/transfers/temp/download?token={token}")
            .parse()
            .unwrap();
        let query = parse_temp_transfer_query(&uri).unwrap();
        assert_eq!(query.token, token);
    }

    #[tokio::test]
    async fn upload_body_reader_allows_exact_limit_and_rejects_larger_body() {
        let exact = Body::from(vec![0u8; 16]);
        assert_eq!(read_limited_body(exact, 16).await.unwrap().len(), 16);

        let too_large = Body::from(vec![0u8; 17]);
        let err = read_limited_body(too_large, 16).await.unwrap_err();
        assert_eq!(err.0, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(err.1.contains("upload is larger than"));
    }

    #[test]
    fn temporary_transfer_agent_result_text_is_bounded() {
        let mut result = TaskResult {
            stdout: "x".repeat(16),
            stderr: String::new(),
            error: String::new(),
            ..Default::default()
        };
        assert!(ensure_temporary_transfer_result_text(&result, 16, "temporary result").is_ok());

        result.stdout.push('x');
        let err =
            ensure_temporary_transfer_result_text(&result, 16, "temporary result").unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_GATEWAY);
        assert!(err.1.contains("stdout exceeds 16 bytes"));

        result.stdout = String::new();
        result.error = "x".repeat(TEMP_TRANSFER_SMALL_RESULT_MAX_BYTES + 1);
        let err =
            ensure_temporary_transfer_result_text(&result, 16, "temporary result").unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_GATEWAY);
        assert!(err.1.contains("error exceeds"));
    }

    #[tokio::test]
    async fn temporary_transfer_rejects_revoked_server_before_consuming_token() {
        let state = test_state().await;
        let token = format!("xlt_{}", "a".repeat(64));
        let token_hash = hash_token(&token);
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
                name: "revoked-agent".into(),
                public_key: "public".into(),
                owner_user_id: user.id,
            })
            .await
            .unwrap();
        let session = SessionRepository::new(state.db.clone())
            .create(
                CreateSessionInput {
                    user_id: user.id,
                    ip: Some("127.0.0.1".into()),
                    user_agent: Some("test".into()),
                    expires_at: Utc::now() + Duration::minutes(30),
                },
                "session-token-hash".into(),
            )
            .await
            .unwrap();
        let record = TemporaryTransferTokenRepository::new(state.db.clone())
            .create(CreateTemporaryTransferTokenInput {
                token_hash: token_hash.clone(),
                server_id: agent.id,
                path: "/tmp/file.txt".into(),
                op: "download".into(),
                issued_by_user_id: user.id,
                auth_kind: "session".into(),
                session_id: Some(session.id),
                api_token_id: None,
                scope: "transfer:read".into(),
                expires_at: Utc::now() + Duration::minutes(5),
                created_ip: Some("127.0.0.1".into()),
            })
            .await
            .unwrap();

        assert!(agent_repo.revoke(agent.id).await.unwrap());

        let err = validate_temporary_transfer(&state, &token, "download")
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1, "temporary URL server is revoked");

        let stored = TemporaryTransferTokenRepository::new(state.db.clone())
            .find_by_id(&record.id)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.used_at.is_none());
        assert!(stored.used_ip.is_none());
        assert!(stored.used_status.is_none());
    }

    async fn test_state() -> AppState {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-temp-transfer-api-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
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
}
