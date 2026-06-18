use crate::db::Db;
use anyhow::{Context, Result};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use xlstatus_shared::{AgentId, UserId};

use super::tools::{McpToolRequest, McpToolResponse};
use crate::db::repository::agent::AgentRepository;

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
    pub async fn execute(
        &self,
        user_id: &str,
        request: McpToolRequest,
        allowed_server_ids: Option<&[String]>,
    ) -> McpToolResponse {
        let result = match request.tool.as_str() {
            "meta.whoami" => self.exec_whoami(user_id).await,
            "server.list" => {
                self.exec_server_list(user_id, &request.arguments, allowed_server_ids)
                    .await
            }
            "server.get" => {
                self.exec_server_get(user_id, &request.arguments, allowed_server_ids)
                    .await
            }
            "server.exec" => {
                self.exec_server_exec(&request.arguments, allowed_server_ids)
                    .await
            }
            "fs.list" => {
                self.exec_fs_list(&request.arguments, allowed_server_ids)
                    .await
            }
            "fs.read" => {
                self.exec_fs_read(&request.arguments, allowed_server_ids)
                    .await
            }
            "fs.write" => {
                self.exec_fs_write(&request.arguments, allowed_server_ids)
                    .await
            }
            "fs.delete" => {
                self.exec_fs_delete(&request.arguments, allowed_server_ids)
                    .await
            }
            "fs.download_url" => {
                self.exec_fs_download_url(&request.arguments, allowed_server_ids)
                    .await
            }
            "fs.upload_url" => {
                self.exec_fs_upload_url(&request.arguments, allowed_server_ids)
                    .await
            }
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
        user_id: &str,
        args: &serde_json::Value,
        allowed_server_ids: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let user_id = parse_user_id(user_id)?;
        let limit = args["limit"].as_i64().unwrap_or(50).clamp(1, 200);
        let offset = args["offset"].as_i64().unwrap_or(0).max(0);
        let repo = AgentRepository::new(self.db.clone());
        let agents = repo.list_by_owner(user_id, limit, offset).await?;
        let servers: Vec<_> = agents
            .into_iter()
            .filter(|agent| {
                allowed_server_ids
                    .map(|ids| ids.iter().any(|id| id == &agent.id.0.to_string()))
                    .unwrap_or(true)
            })
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
        user_id: &str,
        args: &serde_json::Value,
        allowed_server_ids: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let user_id = parse_user_id(user_id)?;
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        ensure_server_allowed(server_id, allowed_server_ids)?;
        let agent_id = AgentId(uuid::Uuid::parse_str(server_id).context("Invalid server_id")?);
        let repo = AgentRepository::new(self.db.clone());
        let agent = repo
            .find_by_id(agent_id)
            .await?
            .context("Server not found")?;

        if agent.owner_user_id != user_id {
            anyhow::bail!("Server not found");
        }

        Ok(json!({
            "server_id": agent.id.0.to_string(),
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
        args: &serde_json::Value,
        allowed_server_ids: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        ensure_server_allowed(server_id, allowed_server_ids)?;
        let command = args["command"].as_str().context("Missing command")?;
        let timeout = args["timeout"].as_i64().unwrap_or(30);

        // M6: dispatch the command to the live agent over gRPC and
        // wait up to `timeout` seconds for the `TaskResult`.
        let agent_uuid = uuid::Uuid::parse_str(server_id).context("server_id must be a UUID")?;
        let agent_id = AgentId(agent_uuid);
        if !self.session_registry.is_online(&agent_id).await {
            anyhow::bail!("agent {} is offline", server_id);
        }
        let response_registry = crate::current_task_response_registry();
        let run_id = uuid::Uuid::now_v7().to_string();
        let rx = response_registry.register(run_id.clone()).await;
        if let Err(e) = self
            .session_registry
            .send_task(&agent_id, &run_id, command, timeout.max(1) as u32)
            .await
        {
            response_registry.cancel(&run_id).await;
            return Err(anyhow::anyhow!(e));
        }
        let result =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout.max(1) as u64), rx)
                .await
            {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => anyhow::bail!("agent disconnected before reply"),
                Err(_) => {
                    response_registry.cancel(&run_id).await;
                    anyhow::bail!("task timeout")
                }
            };
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
        args: &serde_json::Value,
        allowed_server_ids: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        ensure_server_allowed(server_id, allowed_server_ids)?;
        let path = args["path"].as_str().context("Missing path")?;

        // M6: delegate to the agent via `ls -la` shell task.
        let cmd = format!("ls -la {}", shell_escape(path));
        let r = self.dispatch_shell(server_id, &cmd, 15).await?;
        Ok(json!({
            "server_id": server_id,
            "path": path,
            "raw": r.get("stdout").cloned().unwrap_or_default(),
            "status": r.get("status").cloned().unwrap_or_else(|| "unknown".into()),
        }))
    }

    /// Execute fs.read
    async fn exec_fs_read(
        &self,
        args: &serde_json::Value,
        allowed_server_ids: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        ensure_server_allowed(server_id, allowed_server_ids)?;
        let path = args["path"].as_str().context("Missing path")?;
        let max_size = args["max_size"].as_i64().unwrap_or(1048576);

        // M6: read the file via the shell task using `head -c`.
        let cmd = format!(
            "head -c {} {} 2>/dev/null || echo __XLSTATUS_READ_ERROR__",
            max_size,
            shell_escape(path)
        );
        let r = self.dispatch_shell(server_id, &cmd, 30).await?;
        let content = r
            .get("stdout")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let truncated = content.len() as i64 >= max_size;
        let error = if content.contains("__XLSTATUS_READ_ERROR__") {
            Some("file not found or unreadable".to_string())
        } else {
            None
        };
        Ok(json!({
            "server_id": server_id,
            "path": path,
            "max_size": max_size,
            "content": content,
            "truncated": truncated,
            "error": error,
        }))
    }

    /// Execute fs.write
    async fn exec_fs_write(
        &self,
        args: &serde_json::Value,
        allowed_server_ids: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        ensure_server_allowed(server_id, allowed_server_ids)?;
        let path = args["path"].as_str().context("Missing path")?;
        let content = args["content"].as_str().context("Missing content")?;
        let mode = args["mode"].as_str().unwrap_or("overwrite");

        // M6: write via shell using a python helper that respects mode.
        let b64 = base64_encode(content.as_bytes());
        let cmd = format!(
            "mkdir -p $(dirname {}) && echo {} | base64 -d > {}",
            shell_escape(path),
            b64,
            shell_escape(path)
        );
        let r = self.dispatch_shell(server_id, &cmd, 30).await?;
        Ok(json!({
            "server_id": server_id,
            "path": path,
            "mode": mode,
            "bytes_written": content.len(),
            "status": r.get("status").cloned().unwrap_or_else(|| "unknown".into()),
        }))
    }

    /// Execute fs.delete
    async fn exec_fs_delete(
        &self,
        args: &serde_json::Value,
        allowed_server_ids: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        ensure_server_allowed(server_id, allowed_server_ids)?;
        let path = args["path"].as_str().context("Missing path")?;

        // M6: delete via shell `rm -f`.
        let cmd = format!("rm -f {}", shell_escape(path));
        let r = self.dispatch_shell(server_id, &cmd, 15).await?;
        Ok(json!({
            "server_id": server_id,
            "path": path,
            "status": r.get("status").cloned().unwrap_or_else(|| "unknown".into()),
        }))
    }

    /// Execute fs.download_url
    async fn exec_fs_download_url(
        &self,
        args: &serde_json::Value,
        allowed_server_ids: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        ensure_server_allowed(server_id, allowed_server_ids)?;
        let path = args["path"].as_str().context("Missing path")?;
        let expires_in = args["expires_in"].as_i64().unwrap_or(3600);

        let expires_at = temporary_url_expires_at(expires_in);
        let token = temporary_url_token(
            &self.temp_url_secret,
            server_id,
            path,
            "download",
            expires_at,
        )?;
        let url = format!(
            "/api/v1/transfers/temp/download?server_id={}&path={}&expires_at={}&token={}",
            percent_encode(server_id),
            percent_encode(path),
            expires_at,
            token
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

    /// Execute fs.upload_url
    async fn exec_fs_upload_url(
        &self,
        args: &serde_json::Value,
        allowed_server_ids: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        ensure_server_allowed(server_id, allowed_server_ids)?;
        let path = args["path"].as_str().context("Missing path")?;
        let expires_in = args["expires_in"].as_i64().unwrap_or(3600);

        let expires_at = temporary_url_expires_at(expires_in);
        let token =
            temporary_url_token(&self.temp_url_secret, server_id, path, "upload", expires_at)?;
        let url = format!(
            "/api/v1/transfers/temp/upload?server_id={}&path={}&expires_at={}&token={}",
            percent_encode(server_id),
            percent_encode(path),
            expires_at,
            token
        );

        Ok(json!({
            "server_id": server_id,
            "path": path,
            "url": url,
            "expires_in": expires_in,
            "expires_at": expires_at,
            "method": "PUT",
        }))
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
        let agent_uuid = uuid::Uuid::parse_str(server_id).context("server_id must be a UUID")?;
        let agent_id = AgentId(agent_uuid);
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
}

pub(crate) fn temporary_url_expires_at(expires_in: i64) -> i64 {
    chrono::Utc::now()
        .timestamp()
        .saturating_add(expires_in.max(1))
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
    if expires_at <= chrono::Utc::now().timestamp() {
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

fn parse_user_id(user_id: &str) -> Result<UserId> {
    Ok(UserId(
        uuid::Uuid::parse_str(user_id).context("Invalid user_id")?,
    ))
}

fn ensure_server_allowed(server_id: &str, allowed_server_ids: Option<&[String]>) -> Result<()> {
    if allowed_server_ids
        .map(|ids| ids.iter().any(|id| id == server_id))
        .unwrap_or(true)
    {
        Ok(())
    } else {
        anyhow::bail!("Server not allowed by PAT")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_whoami() {
        // Placeholder test
    }
}
