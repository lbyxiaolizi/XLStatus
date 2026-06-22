use crate::db::Db;
use anyhow::{Context, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use xlstatus_proto_gen::xlstatus::v1::{
    server_task::Spec, FileListTask, FileReadTask, ServerTask, TaskOutcome, TaskType,
};
use xlstatus_shared::{AgentId, UserId};

use super::tools::{McpToolRequest, McpToolResponse};
use crate::auth::generate_temporary_transfer_token;
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::db::repository::agent::AgentRepository;
use crate::db::{CreateTemporaryTransferTokenInput, TemporaryTransferTokenRepository};
use crate::grpc::{base64_encoded_len, ensure_task_result_text_within};

const MCP_FILE_OP_TIMEOUT_SECS: u64 = 30;
const MCP_FILE_READ_MAX_BYTES: u64 = 1024 * 1024;
const MCP_FILE_LIST_RESULT_MAX_BYTES: usize = 1024 * 1024;
const MCP_SMALL_RESULT_MAX_BYTES: usize = 4096;
const MCP_EXEC_MAX_COMMAND_BYTES: usize = 8192;
const MCP_EXEC_DEFAULT_TIMEOUT_SECS: u32 = 30;
const MCP_EXEC_MAX_TIMEOUT_SECS: u32 = 60;
const MCP_EXEC_RESULT_MAX_BYTES: usize = 64 * 1024;
const MCP_UUID_TEXT_LEN: usize = 36;
pub(crate) const TEMP_URL_DEFAULT_EXPIRES_SECS: i64 = 300;
pub(crate) const TEMP_URL_MAX_EXPIRES_SECS: i64 = 600;

/// MCP tool executor
pub struct McpExecutor {
    db: Db,
    /// M6: handle to the live gRPC session registry so
    /// `server.exec` and `fs.*` can dispatch a task to a real
    /// agent and wait for the result.
    session_registry: crate::grpc::SessionRegistry,
    temp_url_secret: String,
}

impl McpExecutor {
    pub fn new(
        db: Db,
        session_registry: crate::grpc::SessionRegistry,
        temp_url_secret: String,
    ) -> Self {
        Self {
            db,
            session_registry,
            temp_url_secret,
        }
    }

    /// Execute an MCP tool
    pub async fn execute(&self, auth: &AuthSession, request: McpToolRequest) -> McpToolResponse {
        let result = match request.tool.as_str() {
            "meta.whoami" => self.exec_whoami(&auth.user_id.0.to_string()).await,
            "server.list" => self.exec_server_list(auth, &request.arguments).await,
            "server.get" => self.exec_server_get(auth, &request.arguments).await,
            "server.exec" => self.exec_server_exec(auth, &request.arguments).await,
            "fs.list" => self.exec_fs_list(auth, &request.arguments).await,
            "fs.read" => self.exec_fs_read(auth, &request.arguments).await,
            "fs.write" | "fs.delete" => reject_mcp_file_write_tool(),
            "fs.download_url" => self.exec_fs_download_url(auth, &request.arguments).await,
            "fs.upload_url" => reject_mcp_file_write_tool(),
            _ => Err(anyhow::anyhow!("Unknown tool: {}", request.tool)),
        };

        match result {
            Ok(value) => McpToolResponse {
                success: true,
                result: Some(value),
                error: None,
            },
            Err(e) => McpToolResponse {
                success: false,
                result: None,
                error: Some(e.to_string()),
            },
        }
    }

    /// Execute meta.whoami
    async fn exec_whoami(&self, user_id: &str) -> Result<serde_json::Value> {
        Ok(json!({
            "user_id": user_id,
            "system": "XLStatus",
            "version": env!("CARGO_PKG_VERSION"),
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }))
    }

    /// Execute server.list
    async fn exec_server_list(
        &self,
        auth: &AuthSession,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let limit = args["limit"].as_i64().unwrap_or(50).clamp(1, 200);
        let offset = args["offset"].as_i64().unwrap_or(0).max(0);
        let repo = AgentRepository::new(self.db.clone());
        let agents = if let Some(server_ids) = auth.server_ids.as_deref() {
            let owner_filter = if auth.role.is_admin() {
                None
            } else {
                Some(auth.user_id)
            };
            let (agents, _) = repo
                .list_with_state_by_server_ids(owner_filter, server_ids, limit, offset)
                .await?;
            agents.into_iter().map(|row| row.agent).collect()
        } else {
            repo.list_by_owner(auth.user_id, limit, offset).await?
        };
        let servers: Vec<_> = agents
            .into_iter()
            .filter(|agent| agent_visible_to_auth(auth, agent))
            .map(|agent| {
                json!({
                    "server_id": agent.id.0.to_string(),
                    "name": agent.name,
                    "online": agent.last_seen_at
                        .map(|last_seen| chrono::Utc::now().signed_duration_since(last_seen).num_seconds() <= 30)
                        .unwrap_or(false),
                    "last_seen_at": agent.last_seen_at.map(|dt| dt.to_rfc3339()),
                    "created_at": agent.created_at.to_rfc3339(),
                    "revoked": agent.revoked_at.is_some(),
                })
            })
            .collect();

        Ok(json!({
            "servers": servers,
            "total": servers.len(),
            "limit": limit,
            "offset": offset,
        }))
    }

    /// Execute server.get
    async fn exec_server_get(
        &self,
        auth: &AuthSession,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        let agent = self.ensure_agent_visible(auth, server_id).await?;
        let server_id = agent.id.0.to_string();

        Ok(json!({
            "server_id": server_id,
            "name": agent.name,
            "online": agent.last_seen_at
                .map(|last_seen| chrono::Utc::now().signed_duration_since(last_seen).num_seconds() <= 30)
                .unwrap_or(false),
            "last_seen_at": agent.last_seen_at.map(|dt| dt.to_rfc3339()),
            "created_at": agent.created_at.to_rfc3339(),
            "updated_at": agent.updated_at.to_rfc3339(),
            "revoked": agent.revoked_at.is_some(),
        }))
    }

    /// Execute server.exec
    async fn exec_server_exec(
        &self,
        auth: &AuthSession,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        let agent_id = self.ensure_agent_active(auth, server_id).await?.id;
        let server_id = agent_id.0.to_string();
        let command = validate_exec_command(args["command"].as_str().context("Missing command")?)?;
        let timeout = normalize_exec_timeout(args["timeout"].as_i64());

        // M6: dispatch the command to the live agent over gRPC and
        // wait up to `timeout` seconds for the `TaskResult`.
        if !self.session_registry.is_online(&agent_id).await {
            anyhow::bail!("agent {} is offline", server_id);
        }
        let response_registry = crate::current_task_response_registry();
        let run_id = uuid::Uuid::now_v7().to_string();
        let rx = response_registry.register(run_id.clone()).await;
        if let Err(e) = self
            .session_registry
            .send_task(&agent_id, &run_id, command, timeout)
            .await
        {
            response_registry.cancel(&run_id).await;
            return Err(anyhow::anyhow!(e));
        }
        let result =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout as u64), rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => anyhow::bail!("agent disconnected before reply"),
                Err(_) => {
                    response_registry.cancel(&run_id).await;
                    anyhow::bail!("task timeout")
                }
            };
        ensure_mcp_task_result_text(
            &result,
            MCP_EXEC_RESULT_MAX_BYTES,
            MCP_EXEC_RESULT_MAX_BYTES,
            MCP_SMALL_RESULT_MAX_BYTES,
            "server.exec result",
        )?;
        use xlstatus_proto_gen::xlstatus::v1::TaskOutcome;
        let status = match TaskOutcome::try_from(result.status).unwrap_or(TaskOutcome::Unspecified)
        {
            TaskOutcome::Success => "success",
            TaskOutcome::Failure => "failure",
            TaskOutcome::Timeout => "timeout",
            TaskOutcome::Unspecified => "unknown",
        };
        Ok(json!({
            "server_id": server_id,
            "command": command,
            "timeout_seconds": timeout,
            "exit_code": result.exit_code,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "status": status,
            "duration_ms": result.finished_at.saturating_sub(result.started_at).saturating_mul(1000),
        }))
    }

    /// Execute fs.list
    async fn exec_fs_list(
        &self,
        auth: &AuthSession,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        let agent = self.ensure_agent_active(auth, server_id).await?;
        let server_id = agent.id.0.to_string();
        let path = validate_abs_path(args["path"].as_str().context("Missing path")?)?;
        let result = self
            .dispatch_file_task(
                &server_id,
                ServerTask {
                    task_id: String::new(),
                    task_type: TaskType::FileList as i32,
                    spec: Some(Spec::FileList(FileListTask { path: path.clone() })),
                },
                MCP_FILE_OP_TIMEOUT_SECS,
            )
            .await?;
        ensure_mcp_file_result_text(&result, MCP_FILE_LIST_RESULT_MAX_BYTES, "fs.list result")?;
        ensure_task_success(&result)?;
        let entries = serde_json::from_str::<serde_json::Value>(&result.stdout)
            .context("agent returned invalid file list JSON")?;
        Ok(json!({
            "server_id": server_id,
            "path": path,
            "entries": entries,
        }))
    }

    /// Execute fs.read
    async fn exec_fs_read(
        &self,
        auth: &AuthSession,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        let agent = self.ensure_agent_active(auth, server_id).await?;
        let server_id = agent.id.0.to_string();
        let path = validate_abs_path(args["path"].as_str().context("Missing path")?)?;
        let max_size = args["max_size"]
            .as_u64()
            .unwrap_or(MCP_FILE_READ_MAX_BYTES)
            .clamp(1, MCP_FILE_READ_MAX_BYTES);
        let result = self
            .dispatch_file_task(
                &server_id,
                ServerTask {
                    task_id: String::new(),
                    task_type: TaskType::FileRead as i32,
                    spec: Some(Spec::FileRead(FileReadTask {
                        path: path.clone(),
                        offset: 0,
                        length: max_size,
                    })),
                },
                MCP_FILE_OP_TIMEOUT_SECS,
            )
            .await?;
        ensure_mcp_file_result_text(
            &result,
            base64_encoded_len(max_size as usize),
            "fs.read result",
        )?;
        ensure_task_success(&result)?;
        let data = decode_base64(result.stdout.trim())
            .map_err(|e| anyhow::anyhow!("agent returned invalid base64: {e}"))?;
        if data.len() > max_size as usize {
            anyhow::bail!("agent returned more than {max_size} file bytes");
        }
        let content = String::from_utf8(data.clone()).context("file is not valid UTF-8")?;
        Ok(json!({
            "server_id": server_id,
            "path": path,
            "max_size": max_size,
            "content": content,
            "bytes": data.len(),
            "truncated": data.len() as u64 >= max_size,
        }))
    }

    /// Execute fs.download_url
    async fn exec_fs_download_url(
        &self,
        auth: &AuthSession,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        let agent = self.ensure_agent_active(auth, server_id).await?;
        let server_id = agent.id.0.to_string();
        let path = validate_abs_path(args["path"].as_str().context("Missing path")?)?;
        let expires_in = temporary_url_expires_in(
            args["expires_in"]
                .as_i64()
                .unwrap_or(TEMP_URL_DEFAULT_EXPIRES_SECS),
        );

        let expires_at = temporary_url_expires_at(expires_in);
        let token = self
            .create_temporary_transfer_token(
                auth,
                &server_id,
                &path,
                "download",
                "transfer:read",
                expires_at,
            )
            .await?;
        let url = format!(
            "/api/v1/transfers/temp/download?token={}",
            percent_encode(&token)
        );

        Ok(json!({
            "server_id": server_id,
            "path": path,
            "url": url,
            "expires_in": expires_in,
            "expires_at": expires_at,
            "method": "GET",
        }))
    }

    async fn create_temporary_transfer_token(
        &self,
        auth: &AuthSession,
        server_id: &str,
        path: &str,
        op: &str,
        scope: &str,
        expires_at: i64,
    ) -> Result<String> {
        let (token, token_hash) = generate_temporary_transfer_token();
        let expires_at = chrono::DateTime::from_timestamp(expires_at, 0)
            .context("invalid temporary URL expiration")?;
        let auth_kind = match auth.auth_kind {
            AuthKind::Session => "session",
            AuthKind::PersonalAccessToken => "pat",
        }
        .to_string();
        TemporaryTransferTokenRepository::new(self.db.clone())
            .create(CreateTemporaryTransferTokenInput {
                token_hash,
                server_id: parse_mcp_server_id(server_id)?,
                path: path.to_string(),
                op: op.to_string(),
                issued_by_user_id: auth.user_id,
                auth_kind,
                session_id: matches!(auth.auth_kind, AuthKind::Session)
                    .then_some(auth.session_id.clone()),
                api_token_id: auth.pat_id.clone(),
                scope: scope.to_string(),
                expires_at,
                created_ip: None,
            })
            .await?;
        Ok(token)
    }

    async fn ensure_agent_visible(
        &self,
        auth: &AuthSession,
        server_id: &str,
    ) -> Result<crate::db::Agent> {
        let agent_id = parse_mcp_server_id(server_id)?;
        let agent = AgentRepository::new(self.db.clone())
            .find_by_id(agent_id)
            .await?
            .context("Server not found")?;
        if agent_visible_to_auth(auth, &agent) {
            Ok(agent)
        } else {
            anyhow::bail!("Server not allowed")
        }
    }

    async fn ensure_agent_active(
        &self,
        auth: &AuthSession,
        server_id: &str,
    ) -> Result<crate::db::Agent> {
        let agent = self.ensure_agent_visible(auth, server_id).await?;
        if agent.revoked_at.is_some() {
            anyhow::bail!("agent has been revoked");
        }
        Ok(agent)
    }
}

impl McpExecutor {
    /// M6 helper: dispatch a shell command to an agent and wait
    /// for the TaskResult, returning a JSON object with
    /// `stdout`, `stderr`, `status` and `error` fields.
    async fn dispatch_shell(
        &self,
        server_id: &str,
        command: &str,
        timeout_seconds: u32,
    ) -> Result<serde_json::Value> {
        let command = validate_exec_command(command)?;
        let timeout_seconds = normalize_exec_timeout(Some(timeout_seconds as i64));
        let agent_id = parse_mcp_server_id(server_id)?;
        if !self.session_registry.is_online(&agent_id).await {
            anyhow::bail!("agent {} is offline", server_id);
        }
        let response_registry = crate::current_task_response_registry();
        let run_id = uuid::Uuid::now_v7().to_string();
        let rx = response_registry.register(run_id.clone()).await;
        if let Err(e) = self
            .session_registry
            .send_task(&agent_id, &run_id, command, timeout_seconds)
            .await
        {
            response_registry.cancel(&run_id).await;
            return Err(anyhow::anyhow!(e));
        }
        let result =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_seconds as u64), rx)
                .await
            {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => anyhow::bail!("agent disconnected before reply"),
                Err(_) => {
                    response_registry.cancel(&run_id).await;
                    anyhow::bail!("task timeout")
                }
            };
        ensure_mcp_task_result_text(
            &result,
            MCP_EXEC_RESULT_MAX_BYTES,
            MCP_EXEC_RESULT_MAX_BYTES,
            MCP_SMALL_RESULT_MAX_BYTES,
            "server.exec result",
        )?;
        use xlstatus_proto_gen::xlstatus::v1::TaskOutcome;
        let status = match TaskOutcome::try_from(result.status).unwrap_or(TaskOutcome::Unspecified)
        {
            TaskOutcome::Success => "success",
            TaskOutcome::Failure => "failure",
            TaskOutcome::Timeout => "timeout",
            TaskOutcome::Unspecified => "unknown",
        };
        Ok(json!({
            "exit_code": result.exit_code,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "status": status,
            "error": if result.error.is_empty() { serde_json::Value::Null } else { json!(result.error) },
        }))
    }

    async fn dispatch_file_task(
        &self,
        server_id: &str,
        mut task: ServerTask,
        timeout_seconds: u64,
    ) -> Result<xlstatus_proto_gen::xlstatus::v1::TaskResult> {
        let agent_id = parse_mcp_server_id(server_id)?;
        if !self.session_registry.is_online(&agent_id).await {
            anyhow::bail!("agent {} is offline", server_id);
        }
        let response_registry = crate::current_task_response_registry();
        let run_id = uuid::Uuid::now_v7().to_string();
        task.task_id = run_id.clone();
        let rx = response_registry.register(run_id.clone()).await;
        if let Err(e) = self
            .session_registry
            .send_server_task(&agent_id, task)
            .await
        {
            response_registry.cancel(&run_id).await;
            return Err(anyhow::anyhow!(e));
        }
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_seconds), rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => anyhow::bail!("agent disconnected before reply"),
            Err(_) => {
                response_registry.cancel(&run_id).await;
                anyhow::bail!("task timeout")
            }
        }
    }
}

fn ensure_task_success(result: &xlstatus_proto_gen::xlstatus::v1::TaskResult) -> Result<()> {
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
    anyhow::bail!(detail)
}

fn ensure_mcp_file_result_text(
    result: &xlstatus_proto_gen::xlstatus::v1::TaskResult,
    stdout_max: usize,
    context: &str,
) -> Result<()> {
    ensure_mcp_task_result_text(
        result,
        stdout_max,
        MCP_SMALL_RESULT_MAX_BYTES,
        MCP_SMALL_RESULT_MAX_BYTES,
        context,
    )
}

fn ensure_mcp_task_result_text(
    result: &xlstatus_proto_gen::xlstatus::v1::TaskResult,
    stdout_max: usize,
    stderr_max: usize,
    error_max: usize,
    context: &str,
) -> Result<()> {
    ensure_task_result_text_within(result, stdout_max, stderr_max, error_max, context)
        .map_err(anyhow::Error::msg)
}

pub(crate) fn temporary_url_expires_in(expires_in: i64) -> i64 {
    expires_in.clamp(1, TEMP_URL_MAX_EXPIRES_SECS)
}

pub(crate) fn temporary_url_expires_at(expires_in: i64) -> i64 {
    chrono::Utc::now()
        .timestamp()
        .saturating_add(temporary_url_expires_in(expires_in))
}

fn normalize_exec_timeout(timeout: Option<i64>) -> u32 {
    timeout
        .unwrap_or(MCP_EXEC_DEFAULT_TIMEOUT_SECS as i64)
        .clamp(1, MCP_EXEC_MAX_TIMEOUT_SECS as i64) as u32
}

fn validate_exec_command(command: &str) -> Result<&str> {
    if command.trim().is_empty() {
        anyhow::bail!("command must not be empty");
    }
    if command.len() > MCP_EXEC_MAX_COMMAND_BYTES {
        anyhow::bail!(
            "command is too large; maximum is {} bytes",
            MCP_EXEC_MAX_COMMAND_BYTES
        );
    }
    Ok(command)
}

fn parse_mcp_server_id(server_id: &str) -> Result<AgentId> {
    if server_id.is_empty() {
        anyhow::bail!("server_id is required");
    }
    if server_id.len() != MCP_UUID_TEXT_LEN {
        anyhow::bail!("server_id must be a canonical UUID");
    }
    let parsed = uuid::Uuid::parse_str(server_id).context("server_id must be a canonical UUID")?;
    if parsed.to_string() != server_id {
        anyhow::bail!("server_id must be a canonical UUID");
    }
    Ok(AgentId(parsed))
}

pub(crate) fn temporary_url_token(
    secret: &str,
    server_id: &str,
    path: &str,
    op: &str,
    expires_at: i64,
) -> Result<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .context("failed to initialize temp-url HMAC")?;
    mac.update(b"xlstatus-temp-url-v2");
    mac.update(b"\0");
    mac.update(server_id.as_bytes());
    mac.update(b"\0");
    mac.update(path.as_bytes());
    mac.update(b"\0");
    mac.update(op.as_bytes());
    mac.update(b"\0");
    mac.update(expires_at.to_string().as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

pub(crate) fn validate_temporary_url_token(
    secret: &str,
    server_id: &str,
    path: &str,
    op: &str,
    expires_at: i64,
    token: &str,
) -> bool {
    let now = chrono::Utc::now().timestamp();
    if expires_at <= now || expires_at.saturating_sub(now) > TEMP_URL_MAX_EXPIRES_SECS {
        return false;
    }
    temporary_url_token(secret, server_id, path, op, expires_at)
        .map(|expected| constant_time_eq(expected.as_bytes(), token.as_bytes()))
        .unwrap_or(false)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub(crate) fn percent_decode(input: &str) -> Result<String> {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])?;
                let value = u8::from_str_radix(hex, 16)?;
                out.push(value);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Ok(String::from_utf8(out)?)
}

fn validate_abs_path(path: &str) -> Result<String> {
    let trimmed = path.trim();
    if !trimmed.starts_with('/') {
        anyhow::bail!("path must be absolute");
    }
    if trimmed.contains('\0') {
        anyhow::bail!("path contains NUL byte");
    }
    Ok(trimmed.to_string())
}

pub(crate) fn shell_escape(s: &str) -> String {
    // Wrap in single quotes and escape any embedded single quote.
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
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

#[cfg(test)]
fn base64_encode(data: &[u8]) -> String {
    const ALPH: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(ALPH[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPH[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPH[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPH[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = (data[i] as u32) << 16;
        out.push(ALPH[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPH[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        out.push(ALPH[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPH[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPH[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

fn agent_visible_to_auth(auth: &AuthSession, agent: &crate::db::Agent) -> bool {
    let allowed_by_pat = auth
        .server_ids
        .as_ref()
        .map(|ids| ids.iter().any(|id| id == &agent.id.0.to_string()))
        .unwrap_or(true);
    allowed_by_pat && (auth.role.is_admin() || agent.owner_user_id == auth.user_id)
}

fn reject_mcp_file_write_tool() -> Result<serde_json::Value> {
    anyhow::bail!("file write operations require a cookie session")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CreateAgentInput, DatabaseBackend, TemporaryTransferTokenRepository};
    use serde_json::json;
    use xlstatus_shared::UserRole;

    #[tokio::test]
    async fn test_whoami() {
        // Placeholder test
    }

    #[tokio::test]
    async fn mcp_agent_visibility_rejects_other_owner_even_if_pat_allowlisted() {
        let db = test_db().await;
        let owner = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        let other = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap());
        seed_user(&db, owner, "owner", "member").await;
        seed_user(&db, other, "other", "member").await;
        let other_agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "other".into(),
                public_key: "pk".into(),
                owner_user_id: other,
            })
            .await
            .unwrap();
        let executor = test_executor(db);
        let auth = pat_auth(owner, UserRole::Member, vec![other_agent.id.0.to_string()]);

        let denied = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.get".into(),
                    arguments: json!({ "server_id": other_agent.id.0.to_string() }),
                },
            )
            .await;

        assert!(!denied.success);
        assert!(denied
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("Server not allowed"));
    }

    #[tokio::test]
    async fn mcp_agent_visibility_allows_admin_pat_when_allowlisted() {
        let db = test_db().await;
        let admin = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        let other = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap());
        seed_user(&db, admin, "admin", "admin").await;
        seed_user(&db, other, "other", "member").await;
        let other_agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "other".into(),
                public_key: "pk".into(),
                owner_user_id: other,
            })
            .await
            .unwrap();
        let executor = test_executor(db);
        let auth = pat_auth(admin, UserRole::Admin, vec![other_agent.id.0.to_string()]);

        let allowed = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.get".into(),
                    arguments: json!({ "server_id": other_agent.id.0.to_string() }),
                },
            )
            .await;

        assert!(allowed.success);
    }

    #[tokio::test]
    async fn mcp_server_list_filters_allowlist_before_pagination() {
        let db = test_db().await;
        let owner = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        seed_user(&db, owner, "owner", "member").await;
        let blocked_agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "blocked".into(),
                public_key: "pk-blocked".into(),
                owner_user_id: owner,
            })
            .await
            .unwrap();
        let allowed_agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "allowed".into(),
                public_key: "pk-allowed".into(),
                owner_user_id: owner,
            })
            .await
            .unwrap();
        let executor = test_executor(db);
        let auth = pat_auth(
            owner,
            UserRole::Member,
            vec![allowed_agent.id.0.to_string()],
        );

        let response = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.list".into(),
                    arguments: json!({ "limit": 1, "offset": 0 }),
                },
            )
            .await;

        assert!(response.success);
        let result = response.result.unwrap();
        let servers = result["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["server_id"], allowed_agent.id.0.to_string());
        assert_ne!(servers[0]["server_id"], blocked_agent.id.0.to_string());
        assert_eq!(result["total"], 1);
    }

    #[tokio::test]
    async fn mcp_admin_pat_server_list_can_span_allowlisted_owners() {
        let db = test_db().await;
        let admin = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        let other = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap());
        seed_user(&db, admin, "admin", "admin").await;
        seed_user(&db, other, "other", "member").await;
        let other_agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "other".into(),
                public_key: "pk-other".into(),
                owner_user_id: other,
            })
            .await
            .unwrap();
        let executor = test_executor(db);
        let auth = pat_auth(admin, UserRole::Admin, vec![other_agent.id.0.to_string()]);

        let response = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.list".into(),
                    arguments: json!({ "limit": 10, "offset": 0 }),
                },
            )
            .await;

        assert!(response.success);
        let result = response.result.unwrap();
        let servers = result["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["server_id"], other_agent.id.0.to_string());
        assert_eq!(result["total"], 1);
    }

    #[tokio::test]
    async fn mcp_temp_url_signing_rejects_revoked_agent_before_creating_token() {
        let db = test_db().await;
        let owner = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        seed_user(&db, owner, "owner", "admin").await;
        let agent_repo = AgentRepository::new(db.clone());
        let agent = agent_repo
            .create(CreateAgentInput {
                name: "revoked-temp-url-agent".into(),
                public_key: "pk".into(),
                owner_user_id: owner,
            })
            .await
            .unwrap();
        assert!(agent_repo.revoke(agent.id).await.unwrap());
        let executor = test_executor(db.clone());
        let auth = pat_auth(owner, UserRole::Admin, vec![agent.id.0.to_string()]);

        let response = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "fs.download_url".into(),
                    arguments: json!({
                        "server_id": agent.id.0.to_string(),
                        "path": "/tmp/file.txt"
                    }),
                },
            )
            .await;

        assert!(!response.success);
        assert!(response
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("agent has been revoked"));
        let (_, total) = TemporaryTransferTokenRepository::new(db)
            .list(10, 0)
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn mcp_file_write_tools_are_disabled_for_pat_mcp() {
        let db = test_db().await;
        let owner = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        seed_user(&db, owner, "owner", "admin").await;
        let agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "file-write-disabled-agent".into(),
                public_key: "pk".into(),
                owner_user_id: owner,
            })
            .await
            .unwrap();
        let executor = test_executor(db.clone());
        let auth = pat_auth(owner, UserRole::Admin, vec![agent.id.0.to_string()]);

        for (tool, arguments) in [
            (
                "fs.write",
                json!({
                    "server_id": agent.id.0.to_string(),
                    "path": "/tmp/file.txt",
                    "content": "owned"
                }),
            ),
            (
                "fs.delete",
                json!({
                    "server_id": agent.id.0.to_string(),
                    "path": "/tmp/file.txt"
                }),
            ),
            (
                "fs.upload_url",
                json!({
                    "server_id": agent.id.0.to_string(),
                    "path": "/tmp/file.txt"
                }),
            ),
        ] {
            let response = executor
                .execute(
                    &auth,
                    McpToolRequest {
                        tool: tool.into(),
                        arguments,
                    },
                )
                .await;

            assert!(!response.success, "{tool} unexpectedly succeeded");
            assert!(response
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("cookie session"));
        }

        let (_, total) = TemporaryTransferTokenRepository::new(db)
            .list(10, 0)
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn mcp_server_exec_rejects_revoked_agent_before_dispatch() {
        let db = test_db().await;
        let owner = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        seed_user(&db, owner, "owner", "admin").await;
        let agent_repo = AgentRepository::new(db.clone());
        let agent = agent_repo
            .create(CreateAgentInput {
                name: "revoked-exec-agent".into(),
                public_key: "pk".into(),
                owner_user_id: owner,
            })
            .await
            .unwrap();
        assert!(agent_repo.revoke(agent.id).await.unwrap());
        let executor = test_executor(db);
        let auth = pat_auth(owner, UserRole::Admin, vec![agent.id.0.to_string()]);

        let response = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.exec".into(),
                    arguments: json!({
                        "server_id": agent.id.0.to_string(),
                        "command": "whoami"
                    }),
                },
            )
            .await;

        assert!(!response.success);
        assert!(response
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("agent has been revoked"));
    }

    #[tokio::test]
    async fn mcp_server_tools_require_canonical_server_id() {
        let db = test_db().await;
        let owner = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        seed_user(&db, owner, "owner", "admin").await;
        let agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "canonical-agent".into(),
                public_key: "pk".into(),
                owner_user_id: owner,
            })
            .await
            .unwrap();
        let executor = test_executor(db);
        let auth = pat_auth(owner, UserRole::Admin, vec![agent.id.0.to_string()]);
        let simple = agent.id.0.simple().to_string();
        let uppercase = agent.id.0.to_string().to_uppercase();

        for server_id in [
            "server-a".to_string(),
            format!(" {} ", agent.id.0),
            simple,
            uppercase,
            "a".repeat(MCP_UUID_TEXT_LEN + 1),
        ] {
            let response = executor
                .execute(
                    &auth,
                    McpToolRequest {
                        tool: "server.get".into(),
                        arguments: json!({ "server_id": server_id }),
                    },
                )
                .await;

            assert!(!response.success);
            assert!(response
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("canonical UUID"));
        }

        assert_eq!(
            parse_mcp_server_id(&agent.id.0.to_string()).unwrap(),
            agent.id
        );
    }

    #[test]
    fn temporary_url_expires_in_is_capped() {
        assert_eq!(temporary_url_expires_in(-10), 1);
        assert_eq!(temporary_url_expires_in(300), 300);
        assert_eq!(
            temporary_url_expires_in(60 * 60 * 24),
            TEMP_URL_MAX_EXPIRES_SECS
        );
    }

    #[test]
    fn server_exec_timeout_is_bounded() {
        assert_eq!(normalize_exec_timeout(None), MCP_EXEC_DEFAULT_TIMEOUT_SECS);
        assert_eq!(normalize_exec_timeout(Some(-10)), 1);
        assert_eq!(normalize_exec_timeout(Some(0)), 1);
        assert_eq!(normalize_exec_timeout(Some(45)), 45);
        assert_eq!(
            normalize_exec_timeout(Some(60 * 60)),
            MCP_EXEC_MAX_TIMEOUT_SECS
        );
    }

    #[test]
    fn server_exec_command_rejects_empty_or_oversized_values() {
        assert!(validate_exec_command("echo ok").is_ok());
        assert!(validate_exec_command("   ").is_err());
        let oversized = "x".repeat(MCP_EXEC_MAX_COMMAND_BYTES + 1);
        assert!(validate_exec_command(&oversized).is_err());
    }

    #[test]
    fn mcp_agent_result_text_has_business_bounds() {
        assert_eq!(MCP_FILE_LIST_RESULT_MAX_BYTES, 1024 * 1024);
        assert_eq!(MCP_EXEC_RESULT_MAX_BYTES, 64 * 1024);

        let mut result = xlstatus_proto_gen::xlstatus::v1::TaskResult {
            stdout: "ok".into(),
            stderr: String::new(),
            error: String::new(),
            ..Default::default()
        };
        assert!(ensure_mcp_file_result_text(&result, 2, "fs.read result").is_ok());

        result.stdout = "x".repeat(3);
        let err = ensure_mcp_file_result_text(&result, 2, "fs.read result").unwrap_err();
        assert!(err.to_string().contains("stdout exceeds 2 bytes"));

        result.stdout = String::new();
        result.stderr = "x".repeat(MCP_SMALL_RESULT_MAX_BYTES + 1);
        let err = ensure_mcp_file_result_text(&result, 2, "fs.read result").unwrap_err();
        assert!(err.to_string().contains("stderr exceeds"));
    }

    #[test]
    fn temporary_url_token_rejects_expired_or_overlong_lifetime() {
        let secret = "test-secret";
        let server_id = "018f7dc7-9db7-7c00-9b63-582c6b2a1184";
        let path = "/var/lib/xlstatus/files/report.txt";
        let now = chrono::Utc::now().timestamp();
        let valid_expires_at = now + TEMP_URL_MAX_EXPIRES_SECS;
        let valid =
            temporary_url_token(secret, server_id, path, "download", valid_expires_at).unwrap();
        assert!(validate_temporary_url_token(
            secret,
            server_id,
            path,
            "download",
            valid_expires_at,
            &valid
        ));

        let overlong_expires_at = now + TEMP_URL_MAX_EXPIRES_SECS + 1;
        let overlong =
            temporary_url_token(secret, server_id, path, "download", overlong_expires_at).unwrap();
        assert!(!validate_temporary_url_token(
            secret,
            server_id,
            path,
            "download",
            overlong_expires_at,
            &overlong
        ));

        let expired_at = now - 1;
        let expired = temporary_url_token(secret, server_id, path, "download", expired_at).unwrap();
        assert!(!validate_temporary_url_token(
            secret, server_id, path, "download", expired_at, &expired
        ));
    }

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    fn test_executor(db: DatabaseBackend) -> McpExecutor {
        McpExecutor::new(db, crate::grpc::SessionRegistry::new(), "secret".into())
    }

    async fn seed_user(db: &DatabaseBackend, id: UserId, username: &str, role: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'x', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.0.to_string())
        .bind(username)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }

    fn pat_auth(user_id: UserId, role: UserRole, server_ids: Vec<String>) -> AuthSession {
        AuthSession {
            session_id: "pat-session".into(),
            user_id,
            username: "pat".into(),
            role,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::PersonalAccessToken,
            scopes: vec![
                "server:read".into(),
                "server:exec".into(),
                "transfer:read".into(),
            ],
            server_ids: Some(server_ids),
            pat_id: Some("pat".into()),
        }
    }
}
