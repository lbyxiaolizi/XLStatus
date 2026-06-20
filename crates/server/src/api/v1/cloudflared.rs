//! Cloudflare Tunnel process management.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{require_sensitive_totp, AppError, AppState};
use crate::api::v1::settings::{
    cloudflared_token, cloudflared_token_configured, set_cloudflared_token,
};
use crate::auth::middleware::AuthSession;
use axum::{extract::State, http::HeaderMap, Json};
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

static CLOUDFLARED: Lazy<Mutex<CloudflaredProcess>> =
    Lazy::new(|| Mutex::new(CloudflaredProcess::default()));

#[derive(Default)]
struct CloudflaredProcess {
    child: Option<Child>,
    started_at: Option<String>,
    last_error: Option<String>,
    logs: VecDeque<String>,
}

#[derive(Debug, Serialize)]
pub struct CloudflaredStatusResponse {
    pub token_configured: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub last_error: Option<String>,
    pub logs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CloudflaredTokenRequest {
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CloudflaredActionResponse {
    pub action: String,
    pub success: bool,
    pub status: CloudflaredStatusResponse,
}

pub async fn cloudflared_status(
    State(state): State<AppState>,
    auth: AuthSession,
) -> Result<Json<ApiResponse<CloudflaredStatusResponse>>, AppError> {
    require_admin(&auth)?;
    let mut process = CLOUDFLARED.lock().await;
    Ok(Json(ApiResponse::success(
        current_status(&state, &mut process).await?,
    )))
}

pub async fn save_cloudflared_token(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    Json(req): Json<CloudflaredTokenRequest>,
) -> Result<Json<ApiResponse<CloudflaredActionResponse>>, AppError> {
    require_admin(&auth)?;
    require_sensitive_totp(&state.db, auth.user_id, &headers).await?;
    set_cloudflared_token(&state.db, req.token).await?;
    let mut process = CLOUDFLARED.lock().await;
    push_log(&mut process, "cloudflared token updated");
    let status = current_status(&state, &mut process).await?;
    Ok(Json(ApiResponse::success(CloudflaredActionResponse {
        action: "token".into(),
        success: true,
        status,
    })))
}

pub async fn start_cloudflared(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<CloudflaredActionResponse>>, AppError> {
    require_admin(&auth)?;
    require_sensitive_totp(&state.db, auth.user_id, &headers).await?;
    let token = cloudflared_token(&state.db)
        .await?
        .ok_or(AppError::BadRequest(
            "cloudflared token is not configured".into(),
        ))?;
    let mut process = CLOUDFLARED.lock().await;
    refresh_process(&mut process);
    if process.child.is_some() {
        let status = current_status(&state, &mut process).await?;
        return Ok(Json(ApiResponse::success(CloudflaredActionResponse {
            action: "start".into(),
            success: true,
            status,
        })));
    }

    let mut child = Command::new("cloudflared")
        .arg("tunnel")
        .arg("--no-autoupdate")
        .arg("run")
        .arg("--token")
        .arg(token)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::BadRequest(format!("failed to start cloudflared: {e}")))?;

    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(read_cloudflared_logs("stdout", stdout));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(read_cloudflared_logs("stderr", stderr));
    }
    process.started_at = Some(Utc::now().to_rfc3339());
    process.last_error = None;
    push_log(&mut process, "cloudflared started");
    process.child = Some(child);
    let status = current_status(&state, &mut process).await?;
    Ok(Json(ApiResponse::success(CloudflaredActionResponse {
        action: "start".into(),
        success: true,
        status,
    })))
}

pub async fn stop_cloudflared(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<CloudflaredActionResponse>>, AppError> {
    require_admin(&auth)?;
    require_sensitive_totp(&state.db, auth.user_id, &headers).await?;
    let mut process = CLOUDFLARED.lock().await;
    if let Some(mut child) = process.child.take() {
        if let Err(err) = child.kill().await {
            process.last_error = Some(format!("failed to stop cloudflared: {err}"));
        } else {
            push_log(&mut process, "cloudflared stopped");
        }
    }
    process.started_at = None;
    let status = current_status(&state, &mut process).await?;
    Ok(Json(ApiResponse::success(CloudflaredActionResponse {
        action: "stop".into(),
        success: true,
        status,
    })))
}

async fn current_status(
    state: &AppState,
    process: &mut CloudflaredProcess,
) -> Result<CloudflaredStatusResponse, AppError> {
    refresh_process(process);
    Ok(CloudflaredStatusResponse {
        token_configured: cloudflared_token_configured(&state.db).await?,
        running: process.child.is_some(),
        pid: process.child.as_ref().and_then(Child::id),
        started_at: process.started_at.clone(),
        last_error: process.last_error.clone(),
        logs: process.logs.iter().cloned().collect(),
    })
}

fn refresh_process(process: &mut CloudflaredProcess) {
    let Some(child) = process.child.as_mut() else {
        return;
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            process.last_error = Some(format!("cloudflared exited with {status}"));
            process.child = None;
            process.started_at = None;
        }
        Ok(None) => {}
        Err(err) => {
            process.last_error = Some(format!("cloudflared status check failed: {err}"));
            process.child = None;
            process.started_at = None;
        }
    }
}

async fn read_cloudflared_logs<R>(stream_name: &'static str, stream: R)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut process = CLOUDFLARED.lock().await;
        push_log(
            &mut process,
            &format!("{stream_name}: {}", redact_token(&line)),
        );
    }
}

fn push_log(process: &mut CloudflaredProcess, line: &str) {
    while process.logs.len() >= 200 {
        process.logs.pop_front();
    }
    process
        .logs
        .push_back(format!("{} {line}", Utc::now().to_rfc3339()));
}

fn redact_token(line: &str) -> String {
    line.replace("token=", "token=<redacted>")
}

fn require_admin(auth: &AuthSession) -> Result<(), AppError> {
    if auth.role.is_admin() {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin role required".into()))
    }
}
