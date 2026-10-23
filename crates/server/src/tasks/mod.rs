pub mod scheduler;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use cron::Schedule;
use futures::{stream, StreamExt};
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{info, warn};
use xlstatus_shared::tasks::*;

use crate::api::v1::agent_json::{
    parse_agent_telemetry_json, parse_dashboard_metadata_json,
    AGENT_DASHBOARD_METADATA_JSON_MAX_BYTES, AGENT_TELEMETRY_JSON_MAX_BYTES,
};
use crate::db::repository::tasks::{TaskRepository, TaskRunRepository};
use crate::db::Db;
use crate::grpc::{truncate_task_result_text, SessionRegistry, TaskResponseRegistry};
use crate::notifications::policy::notification_channel_from_values;
use crate::notifications::sender::{
    ensure_notification_channel_count_allowed, NotificationChannel, NotificationMessage,
    NotificationSender, NotificationSeverity, NOTIFICATION_MAX_GROUP_CHANNELS,
};

pub(crate) const TASK_API_MAX_BODY_BYTES: usize = 256 * 1024;
pub(crate) const TASK_MAX_NAME_BYTES: usize = 128;
pub(crate) const TASK_MAX_SCHEDULE_BYTES: usize = 128;
pub(crate) const TASK_MAX_COMMAND_BYTES: usize = 8192;
pub(crate) const TASK_MAX_TARGET_BYTES: usize = 2048;
pub(crate) const TASK_MAX_HOST_BYTES: usize = 253;
pub(crate) const TASK_MAX_PAYLOAD_BYTES: usize = 64 * 1024;
pub(crate) const TASK_MAX_TIMEOUT_SECS: u64 = 3600;
pub(crate) const TASK_MAX_PING_COUNT: u64 = 100;
pub(crate) const TASK_MAX_HTTP_HEADERS: usize = 32;
pub(crate) const TASK_MAX_HTTP_HEADER_NAME_BYTES: usize = 128;
pub(crate) const TASK_MAX_HTTP_HEADER_VALUE_BYTES: usize = 1024;
pub(crate) const TASK_MAX_SELECTOR_BYTES: usize = 16 * 1024;
pub(crate) const TASK_MAX_SELECTOR_IDS: usize = 64;
pub(crate) const TASK_MAX_SELECTOR_TAGS: usize = 32;
pub(crate) const TASK_MAX_SELECTOR_TOKEN_BYTES: usize = 128;
pub(crate) const TASK_MAX_DISPATCH_TARGETS: usize = 64;
pub(crate) const TASK_MAX_CONCURRENT_DISPATCHES: usize = 8;
pub(crate) const TASK_SCHEDULED_RUNTIME_SCAN_LIMIT: i64 = 1024;
pub(crate) const TASK_RESULT_STDOUT_MAX_BYTES: usize = 64 * 1024;
pub(crate) const TASK_RESULT_STDERR_MAX_BYTES: usize = 64 * 1024;
pub(crate) const TASK_RESULT_ERROR_MAX_BYTES: usize = 16 * 1024;

static TASK_DISPATCH_PERMITS: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(TASK_MAX_CONCURRENT_DISPATCHES)));

const TASK_RUN_CANCELLATION_ERROR: &str = "task dispatch was cancelled before completion";
const TASK_RUN_CANCELLATION_CLEANUP_ATTEMPTS: usize = 10;

struct PendingTaskRunGuard {
    db: Db,
    run_id: String,
    armed: bool,
}

impl PendingTaskRunGuard {
    fn new(db: Db, run_id: String) -> Self {
        Self {
            db,
            run_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingTaskRunGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            warn!(
                "task run {} cancellation cleanup deferred until Server restart",
                self.run_id
            );
            return;
        };
        let db = self.db.clone();
        let run_id = self.run_id.clone();
        runtime.spawn(async move {
            settle_cancelled_task_run(db, run_id).await;
        });
    }
}

async fn settle_cancelled_task_run(db: Db, run_id: String) {
    let mut last_error = None;
    for attempt in 0..TASK_RUN_CANCELLATION_CLEANUP_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        match TaskRunRepository::settle_pending_after_cancellation(
            &db,
            &run_id,
            TASK_RUN_CANCELLATION_ERROR,
        )
        .await
        {
            Ok(true) => return,
            Ok(false) => last_error = None,
            Err(error) => last_error = Some(error),
        }
    }

    if let Some(error) = last_error {
        warn!(
            "failed to settle cancelled task run {} after retries: {}",
            run_id, error
        );
    }
}

pub(crate) fn parse_task_schedule(schedule: &str) -> Result<Schedule> {
    let trimmed = schedule.trim();
    let field_count = trimmed.split_whitespace().count();
    let normalized = match field_count {
        5 => format!("0 {}", trimmed),
        6 | 7 => trimmed.to_string(),
        _ => return Err(anyhow!("Invalid cron expression")),
    };

    Schedule::from_str(&normalized).map_err(|err| anyhow!(err))
}

pub(crate) fn validate_task_definition(task: &Task) -> Result<()> {
    validate_sized_text(&task.name, 1, TASK_MAX_NAME_BYTES, "task.name")?;
    if task.name.trim().is_empty() {
        anyhow::bail!("task.name must not be empty");
    }
    if let Some(schedule) = task.schedule.as_deref() {
        validate_sized_text(schedule, 1, TASK_MAX_SCHEDULE_BYTES, "task.schedule")?;
        parse_task_schedule(schedule).context("task.schedule is invalid")?;
    }
    if let Some(command) = task.command.as_deref() {
        let min = if task.task_type == TaskType::Shell {
            1
        } else {
            0
        };
        validate_sized_text(command, min, TASK_MAX_COMMAND_BYTES, "task.command")?;
    } else if task.task_type == TaskType::Shell {
        anyhow::bail!("task.command must be between 1 and {TASK_MAX_COMMAND_BYTES} bytes");
    }
    if let Some(payload) = task.payload_json.as_deref() {
        validate_sized_text(payload, 0, TASK_MAX_PAYLOAD_BYTES, "task.payload_json")?;
    }
    validate_task_target_and_payload(task)?;
    validate_sized_text(
        &task.server_selector_json,
        1,
        TASK_MAX_SELECTOR_BYTES,
        "task.server_selector_json",
    )?;
    let selector: ServerSelector = serde_json::from_str(&task.server_selector_json)
        .context("task.server_selector_json is invalid")?;
    validate_task_selector_shape(&selector)?;
    Ok(())
}

fn validate_task_target_and_payload(task: &Task) -> Result<()> {
    let payload_obj = match task
        .payload_json
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
    {
        Some(raw) => {
            let value: serde_json::Value =
                serde_json::from_str(raw).context("task.payload_json is not valid JSON")?;
            let Some(obj) = value.as_object() else {
                anyhow::bail!("task.payload_json must be a JSON object");
            };
            validate_payload_common_fields(obj)?;
            Some(obj.clone())
        }
        None => None,
    };

    let command_text = task
        .command
        .as_deref()
        .filter(|value| !value.trim().is_empty());

    match task.task_type {
        TaskType::Shell => {
            let Some(cmd) = command_text else {
                anyhow::bail!("task.command must not be empty for shell task");
            };
            if cmd.trim().is_empty() {
                anyhow::bail!("task.command must not be empty for shell task");
            }
        }
        TaskType::HttpGet => {
            let mut has_target = false;
            if let Some(url) = command_text {
                validate_http_get_task_url(url)?;
                has_target = true;
            }
            if let Some(obj) = &payload_obj {
                if let Some(url_val) = obj.get("url") {
                    let Some(url) = url_val.as_str() else {
                        anyhow::bail!("http_get payload url must be a string");
                    };
                    validate_http_get_task_url(url)?;
                    has_target = true;
                }
                if let Some(headers_val) = obj.get("headers") {
                    validate_http_get_task_headers(headers_val)?;
                }
            }
            if !has_target {
                anyhow::bail!("http_get task requires a valid target URL");
            }
        }
        TaskType::IcmpPing => {
            let mut has_target = false;
            if let Some(host) = command_text {
                validate_task_network_host(host, "icmp_ping")?;
                has_target = true;
            }
            if let Some(obj) = &payload_obj {
                if let Some(host_val) = obj.get("host") {
                    let Some(host) = host_val.as_str() else {
                        anyhow::bail!("icmp_ping payload host must be a string");
                    };
                    validate_task_network_host(host, "icmp_ping")?;
                    has_target = true;
                }
                if let Some(count_val) = obj.get("count") {
                    let Some(count) = count_val.as_u64() else {
                        anyhow::bail!("icmp_ping payload count must be a positive integer");
                    };
                    if count == 0 || count > TASK_MAX_PING_COUNT {
                        anyhow::bail!(
                            "icmp_ping payload count must be between 1 and {TASK_MAX_PING_COUNT}"
                        );
                    }
                }
            }
            if !has_target {
                anyhow::bail!("icmp_ping task requires a valid target host");
            }
        }
        TaskType::TcpPing => {
            let mut has_target = false;
            if let Some(target) = command_text {
                let (host, port) = parse_tcp_ping_host_port(target)?;
                validate_task_network_host(&host, "tcp_ping")?;
                if port == 0 {
                    anyhow::bail!("tcp_ping port must be between 1 and 65535");
                }
                has_target = true;
            }
            if let Some(obj) = &payload_obj {
                let has_host = obj.contains_key("host");
                let has_port = obj.contains_key("port");
                if has_host || has_port {
                    let Some(host) = obj.get("host").and_then(|v| v.as_str()) else {
                        anyhow::bail!("tcp_ping payload host must be a string");
                    };
                    let Some(port) = obj.get("port").and_then(|v| v.as_u64()) else {
                        anyhow::bail!("tcp_ping payload port must be an integer");
                    };
                    if port == 0 || port > u16::MAX as u64 {
                        anyhow::bail!("tcp_ping payload port must be between 1 and 65535");
                    }
                    validate_task_network_host(host, "tcp_ping")?;
                    has_target = true;
                }
            }
            if !has_target {
                anyhow::bail!("tcp_ping task requires a valid host:port target");
            }
        }
    }

    Ok(())
}

fn validate_payload_common_fields(obj: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    for key in ["timeout_secs", "timeout_seconds"] {
        if let Some(val) = obj.get(key) {
            let Some(secs) = val.as_u64() else {
                anyhow::bail!("task.payload_json {key} must be a positive integer");
            };
            if secs == 0 || secs > TASK_MAX_TIMEOUT_SECS {
                anyhow::bail!(
                    "task.payload_json {key} must be between 1 and {TASK_MAX_TIMEOUT_SECS}"
                );
            }
        }
    }
    Ok(())
}

fn validate_http_get_task_url(url: &str) -> Result<()> {
    validate_sized_text(url, 1, TASK_MAX_TARGET_BYTES, "http_get url")?;
    if url.trim() != url || url.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        anyhow::bail!("http_get url contains invalid characters");
    }
    let parsed = reqwest::Url::parse(url).context("http_get url is invalid")?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => anyhow::bail!("http_get url scheme '{scheme}' is not allowed"),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!("http_get url must not include credentials");
    }
    let host = parsed
        .host_str()
        .context("http_get url must include a host")?;
    let port = parsed
        .port_or_known_default()
        .context("http_get url must include a valid port")?;
    if port == 0 {
        anyhow::bail!("http_get url port must be between 1 and 65535");
    }
    validate_task_network_host(host, "http_get")
}

fn validate_http_get_task_headers(headers_val: &serde_json::Value) -> Result<()> {
    let Some(map) = headers_val.as_object() else {
        anyhow::bail!("http_get payload headers must be a JSON object");
    };
    if map.len() > TASK_MAX_HTTP_HEADERS {
        anyhow::bail!("http_get payload headers exceeds {TASK_MAX_HTTP_HEADERS} entries");
    }
    for (name, value) in map {
        validate_sized_text(
            name,
            1,
            TASK_MAX_HTTP_HEADER_NAME_BYTES,
            "http_get header name",
        )?;
        let Some(value_str) = value.as_str() else {
            anyhow::bail!("http_get header value must be a string");
        };
        validate_sized_text(
            value_str,
            0,
            TASK_MAX_HTTP_HEADER_VALUE_BYTES,
            "http_get header value",
        )?;
        if name.chars().any(|ch| ch.is_control() || ch.is_whitespace())
            || value_str.chars().any(char::is_control)
        {
            anyhow::bail!("http_get header contains invalid control characters");
        }
    }
    Ok(())
}

fn parse_tcp_ping_host_port(target: &str) -> Result<(String, u16)> {
    validate_sized_text(target, 1, TASK_MAX_TARGET_BYTES, "tcp_ping target")?;
    if target.trim() != target
        || target
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        anyhow::bail!("tcp_ping target contains invalid characters");
    }
    let (host, port_str) = if let Some(rest) = target.strip_prefix('[') {
        let Some((ipv6_host, suffix)) = rest.split_once("]:") else {
            anyhow::bail!("tcp_ping IPv6 target must use [host]:port format");
        };
        (format!("[{ipv6_host}]"), suffix)
    } else {
        let Some((host_part, port_part)) = target.rsplit_once(':') else {
            anyhow::bail!("tcp_ping target must use host:port format");
        };
        if host_part.contains(':') {
            anyhow::bail!("tcp_ping IPv6 target must use [host]:port format");
        }
        (host_part.to_string(), port_part)
    };
    let port: u16 = port_str
        .parse()
        .context("tcp_ping target port is invalid")?;
    if port == 0 {
        anyhow::bail!("tcp_ping port must be between 1 and 65535");
    }
    Ok((host, port))
}

fn validate_task_network_host(host: &str, purpose: &str) -> Result<()> {
    if host.is_empty() || host.trim() != host {
        anyhow::bail!(
            "{purpose} host is required and must not contain leading/trailing whitespace"
        );
    }
    validate_sized_text(host, 1, TASK_MAX_HOST_BYTES, &format!("{purpose} host"))?;
    crate::services::probe::ensure_safe_probe_host(host, purpose)?;

    let unbracketed = if host.starts_with('[') || host.ends_with(']') {
        let Some(inner) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
            anyhow::bail!("{purpose} host has mismatched IPv6 brackets");
        };
        let ipv6: Ipv6Addr = inner
            .parse()
            .with_context(|| format!("{purpose} bracketed host must be a valid IPv6 literal"))?;
        if !crate::security::allow_private_outbound() && is_blocked_task_ip(IpAddr::V6(ipv6)) {
            anyhow::bail!("{purpose} target resolves to disallowed private address {ipv6}");
        }
        return Ok(());
    } else {
        host
    };

    if let Ok(ip) = unbracketed.parse::<IpAddr>() {
        if !crate::security::allow_private_outbound() && is_blocked_task_ip(ip) {
            anyhow::bail!("{purpose} target resolves to disallowed private address {ip}");
        }
        return Ok(());
    }

    if unbracketed.contains(':') {
        anyhow::bail!("{purpose} host contains invalid characters");
    }
    if !is_valid_task_hostname(unbracketed) {
        anyhow::bail!("{purpose} host is not a valid hostname or IP literal");
    }
    if !crate::security::allow_private_outbound() && is_disallowed_task_hostname(unbracketed) {
        anyhow::bail!("{purpose} target resolves to disallowed local or internal host");
    }
    Ok(())
}

fn is_valid_task_hostname(host: &str) -> bool {
    if host.is_empty() || host.len() > TASK_MAX_HOST_BYTES {
        return false;
    }
    if host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        {
            return false;
        }
    }
    true
}

fn is_disallowed_task_hostname(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    lower == "localhost"
        || lower.ends_with(".localhost")
        || lower == "metadata.google.internal"
        || lower.ends_with(".internal")
        || lower.ends_with(".local")
}

fn is_blocked_task_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_blocked_task_ipv4(ip),
        IpAddr::V6(ip) => is_blocked_task_ipv6(ip),
    }
}

fn is_blocked_task_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || o[0] == 0
        || o[0] >= 224
        || (o[0] == 100 && (64..=127).contains(&o[1]))
        || (o[0] == 169 && o[1] == 254)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (18..=19).contains(&o[1]))
        || o == [255, 255, 255, 255]
}

fn is_blocked_task_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    let first = segments[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || ip
            .to_ipv4_mapped()
            .map(is_blocked_task_ipv4)
            .unwrap_or(false)
}

fn validate_sized_text(value: &str, min_bytes: usize, max_bytes: usize, field: &str) -> Result<()> {
    let len = value.len();
    if len < min_bytes || len > max_bytes {
        anyhow::bail!("{field} must be between {min_bytes} and {max_bytes} bytes");
    }
    Ok(())
}

fn validate_task_selector_shape(selector: &ServerSelector) -> Result<()> {
    ensure_selector_vec_allowed(&selector.server_ids, "server_ids")?;
    ensure_selector_vec_allowed(&selector.exclude_server_ids, "exclude_server_ids")?;
    ensure_selector_vec_allowed(&selector.group_ids, "group_ids")?;
    if selector.tag_names.len() > TASK_MAX_SELECTOR_TAGS {
        anyhow::bail!("tag_names contains too many entries");
    }
    for tag in &selector.tag_names {
        validate_sized_text(tag, 1, TASK_MAX_SELECTOR_TOKEN_BYTES, "tag_names entry")?;
    }
    if selector.tags.len() > TASK_MAX_SELECTOR_TAGS {
        anyhow::bail!("tags contains too many entries");
    }
    for (key, value) in &selector.tags {
        validate_sized_text(key, 1, TASK_MAX_SELECTOR_TOKEN_BYTES, "tags key")?;
        validate_sized_text(value, 0, TASK_MAX_SELECTOR_TOKEN_BYTES, "tags value")?;
    }
    Ok(())
}

fn ensure_selector_vec_allowed(values: &[String], field: &str) -> Result<()> {
    if values.len() > TASK_MAX_SELECTOR_IDS {
        anyhow::bail!("{field} contains too many entries");
    }
    for value in values {
        validate_sized_text(value, 1, TASK_MAX_SELECTOR_TOKEN_BYTES, field)?;
    }
    Ok(())
}

fn ensure_task_target_count_allowed(count: usize) -> Result<()> {
    if count > TASK_MAX_DISPATCH_TARGETS {
        anyhow::bail!(
            "task resolves to {count} target servers; maximum is {TASK_MAX_DISPATCH_TARGETS}"
        );
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct TaskDispatchSummary {
    pub success: usize,
    pub failure: usize,
    pub offline: usize,
    pub timeout: usize,
    pub total: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct TaskDispatchReport {
    pub task_id: String,
    pub summary: TaskDispatchSummary,
    pub runs: Vec<TaskRun>,
}

pub async fn dispatch_task_to_agents(
    db: &Db,
    session_registry: &SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    task: &Task,
) -> Result<TaskDispatchReport> {
    dispatch_task_to_agents_with_source(db, session_registry, response_registry, task, None).await
}

pub async fn dispatch_task_to_agents_with_source(
    db: &Db,
    session_registry: &SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    task: &Task,
    source_agent_id: Option<&str>,
) -> Result<TaskDispatchReport> {
    validate_task_definition(task).context("task definition is outside allowed safety limits")?;
    let selector: ServerSelector = serde_json::from_str(&task.server_selector_json)
        .context("task.server_selector_json is invalid")?;

    let shell_command = match task.task_type {
        TaskType::Shell => task.command.clone().unwrap_or_default(),
        _ => anyhow::bail!("Only Shell tasks can be dispatched today"),
    };
    if shell_command.trim().is_empty() {
        anyhow::bail!("Shell command is empty");
    }

    let target_agent_ids = resolve_server_ids(
        db,
        &selector,
        task.cover_mode,
        &task.owner_user_id,
        source_agent_id,
    )
    .await?;
    if target_agent_ids.is_empty() {
        anyhow::bail!("No target servers resolved from selector");
    }
    ensure_task_target_count_allowed(target_agent_ids.len())?;

    let dispatches = stream::iter(target_agent_ids.iter().cloned().enumerate())
        .map(|(index, agent_id)| {
            let response_registry = response_registry.clone();
            let task_id = task.id.as_str();
            let shell_command = shell_command.as_str();
            async move {
                let _permit = TASK_DISPATCH_PERMITS
                    .clone()
                    .acquire_owned()
                    .await
                    .context("task dispatch limiter closed")?;
                dispatch_task_to_agent(
                    db,
                    session_registry,
                    response_registry.as_ref(),
                    task_id,
                    shell_command,
                    &agent_id,
                )
                .await
                .map(|run| (index, run))
            }
        })
        .buffer_unordered(TASK_MAX_CONCURRENT_DISPATCHES)
        .collect::<Vec<Result<(usize, TaskRun)>>>()
        .await;

    let mut indexed_runs = Vec::with_capacity(dispatches.len());
    for dispatch in dispatches {
        indexed_runs.push(dispatch?);
    }
    indexed_runs.sort_unstable_by_key(|(index, _)| *index);
    let runs = indexed_runs
        .into_iter()
        .map(|(_, run)| run)
        .collect::<Vec<_>>();

    let mut success = 0usize;
    let mut failure = 0usize;
    let mut offline = 0usize;
    let mut timeout = 0usize;
    for run in &runs {
        match run.status {
            TaskStatus::Success => success += 1,
            TaskStatus::Failure => failure += 1,
            TaskStatus::Offline => offline += 1,
            TaskStatus::Timeout => timeout += 1,
        }
    }

    let total = target_agent_ids.len();
    let last_result = format!(
        "success={}, failure={}, offline={}, timeout={}, total={}",
        success, failure, offline, timeout, total
    );
    TaskRepository::update_last_execution(db, &task.id, &Utc::now().to_rfc3339(), &last_result)
        .await?;

    let report = TaskDispatchReport {
        task_id: task.id.clone(),
        summary: TaskDispatchSummary {
            success,
            failure,
            offline,
            timeout,
            total,
        },
        runs,
    };

    if let Err(err) = send_task_dispatch_notification(db, task, &report).await {
        warn!("task {} notification failed: {}", task.id, err);
    }

    Ok(report)
}

async fn dispatch_task_to_agent(
    db: &Db,
    session_registry: &SessionRegistry,
    response_registry: &TaskResponseRegistry,
    task_id: &str,
    shell_command: &str,
    agent_id_str: &str,
) -> Result<TaskRun> {
    let agent_uuid = match uuid::Uuid::parse_str(agent_id_str) {
        Ok(agent_uuid) => agent_uuid,
        Err(_) => {
            return create_run(
                db,
                task_id,
                agent_id_str,
                TaskStatus::Failure,
                0,
                None,
                false,
                Some("invalid agent id".to_string()),
            )
            .await;
        }
    };
    let agent_id = xlstatus_shared::AgentId(agent_uuid);
    let run_id = uuid::Uuid::now_v7().to_string();
    let now = Utc::now().to_rfc3339();

    if !session_registry.is_online(&agent_id).await {
        let run = TaskRun {
            id: run_id,
            task_id: task_id.to_string(),
            server_id: agent_id_str.to_string(),
            status: TaskStatus::Offline,
            delay_ms: Some(0),
            output: None,
            output_truncated: false,
            error: Some("agent offline".to_string()),
            created_at: now,
        };
        TaskRunRepository::create(db, &run).await?;
        return Ok(run);
    }

    let rx = response_registry.register(run_id.clone(), agent_id).await;
    if let Err(error) = session_registry
        .send_task(&agent_id, &run_id, shell_command, 30)
        .await
    {
        response_registry.cancel(&run_id).await;
        let run = TaskRun {
            id: run_id,
            task_id: task_id.to_string(),
            server_id: agent_id_str.to_string(),
            status: TaskStatus::Failure,
            delay_ms: Some(0),
            output: None,
            output_truncated: false,
            error: Some(error),
            created_at: now,
        };
        TaskRunRepository::create(db, &run).await?;
        return Ok(run);
    }

    let pending_run = TaskRun {
        id: run_id.clone(),
        task_id: task_id.to_string(),
        server_id: agent_id_str.to_string(),
        status: TaskStatus::Offline,
        delay_ms: Some(0),
        output: None,
        output_truncated: false,
        error: None,
        created_at: now.clone(),
    };
    // Arm before awaiting the insert. If cancellation races with the database
    // commit, the cleanup retries until the pending row becomes visible.
    let mut pending_guard = PendingTaskRunGuard::new(db.clone(), run_id.clone());
    if let Err(error) = TaskRunRepository::create(db, &pending_run).await {
        response_registry.cancel(&run_id).await;
        return Err(error);
    }

    let started = std::time::Instant::now();
    let outcome = match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
        Ok(Ok(mut result)) => {
            let elapsed = started.elapsed().as_millis() as i64;
            let text_truncation = truncate_task_result_text(
                &mut result,
                TASK_RESULT_STDOUT_MAX_BYTES,
                TASK_RESULT_STDERR_MAX_BYTES,
                TASK_RESULT_ERROR_MAX_BYTES,
            );
            use xlstatus_proto_gen::xlstatus::v1::TaskOutcome as Outcome;
            let status = match Outcome::try_from(result.status).unwrap_or(Outcome::Unspecified) {
                Outcome::Success => TaskStatus::Success,
                Outcome::Timeout => TaskStatus::Timeout,
                Outcome::Failure | Outcome::Unspecified => TaskStatus::Failure,
            };
            let stderr = (!result.stderr.is_empty()).then_some(result.stderr);
            let error = if result.error.is_empty() {
                stderr
            } else {
                Some(result.error)
            };
            let output = (!result.stdout.is_empty()).then_some(result.stdout);
            let output_truncated = text_truncation.any();

            TaskRunRepository::update_result(
                db,
                &run_id,
                status,
                elapsed,
                output.clone(),
                output_truncated,
                error.clone(),
            )
            .await?;

            Ok(TaskRun {
                id: run_id,
                task_id: task_id.to_string(),
                server_id: agent_id_str.to_string(),
                status,
                delay_ms: Some(elapsed),
                output,
                output_truncated,
                error,
                created_at: now,
            })
        }
        Ok(Err(_)) => {
            response_registry.cancel(&run_id).await;
            update_and_build_run(
                db,
                &run_id,
                task_id,
                agent_id_str,
                TaskStatus::Failure,
                0,
                None,
                false,
                Some("agent disconnected before reply".to_string()),
                now,
            )
            .await
        }
        Err(_) => {
            response_registry.cancel(&run_id).await;
            update_and_build_run(
                db,
                &run_id,
                task_id,
                agent_id_str,
                TaskStatus::Timeout,
                started.elapsed().as_millis() as i64,
                None,
                false,
                Some("server-side timeout".to_string()),
                now,
            )
            .await
        }
    };
    if outcome.is_ok() {
        pending_guard.disarm();
    }
    outcome
}

pub fn spawn_triggered_tasks(
    db: Db,
    session_registry: SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    task_ids: Vec<String>,
    source: String,
    source_agent_id: Option<String>,
    owner_user_id: Option<String>,
) {
    let mut seen = std::collections::HashSet::new();
    for task_id in task_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
    {
        if !seen.insert(task_id.clone()) {
            continue;
        }
        let db = db.clone();
        let session_registry = session_registry.clone();
        let response_registry = response_registry.clone();
        let source = source.clone();
        let source_agent_id = source_agent_id.clone();
        let owner_user_id = owner_user_id.clone();
        tokio::spawn(async move {
            if let Err(err) = run_triggered_task(
                db,
                session_registry,
                response_registry,
                &task_id,
                &source,
                source_agent_id.as_deref(),
                owner_user_id.as_deref(),
            )
            .await
            {
                warn!("triggered task {} from {} failed: {}", task_id, source, err);
            }
        });
    }
}

async fn run_triggered_task(
    db: Db,
    session_registry: SessionRegistry,
    response_registry: Arc<TaskResponseRegistry>,
    task_id: &str,
    source: &str,
    source_agent_id: Option<&str>,
    owner_user_id: Option<&str>,
) -> Result<()> {
    let task_id = crate::api::v1::tasks::require_task_uuid_text(task_id, "task_id")
        .map_err(|_| anyhow!("invalid triggered task_id"))?;
    let owner_user_id = owner_user_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("Trigger owner_user_id is required")?;
    let task = TaskRepository::get_by_id(&db, &task_id)
        .await?
        .context("Task not found")?;
    if task.owner_user_id != owner_user_id {
        anyhow::bail!("Task owner does not match trigger owner");
    }
    if !task.enabled {
        info!(
            "triggered task {} from {} skipped: disabled",
            task.id, source
        );
        return Ok(());
    }
    let report = dispatch_task_to_agents_with_source(
        &db,
        &session_registry,
        response_registry,
        &task,
        source_agent_id,
    )
    .await?;
    info!(
        "triggered task {} from {} completed: success={}, failure={}, offline={}, timeout={}, total={}",
        task.id,
        source,
        report.summary.success,
        report.summary.failure,
        report.summary.offline,
        report.summary.timeout,
        report.summary.total
    );
    Ok(())
}

async fn resolve_server_ids(
    db: &Db,
    selector: &ServerSelector,
    cover_mode: CoverMode,
    owner_user_id: &str,
    source_agent_id: Option<&str>,
) -> Result<Vec<String>> {
    let records = list_owned_agent_records(db, owner_user_id).await?;
    let owned_ids: HashSet<String> = records.iter().map(|record| record.id.clone()).collect();
    let mut candidates = Vec::new();
    let has_include_selector =
        selector.source_server || !selector.server_ids.is_empty() || !selector.group_ids.is_empty();

    if selector.source_server {
        if let Some(agent_id) = source_agent_id {
            if owned_ids.contains(agent_id) {
                candidates.push(agent_id.to_string());
            }
        }
    }

    for agent_id in &selector.server_ids {
        if owned_ids.contains(agent_id) {
            candidates.push(agent_id.clone());
        }
    }

    candidates.extend(list_group_agent_ids(db, owner_user_id, &selector.group_ids).await?);

    let tag_filters = normalized_selector_tags(selector);
    if !tag_filters.is_empty() {
        let tag_matched: HashSet<String> = records
            .iter()
            .filter(|record| agent_matches_tag_filters(record, &tag_filters))
            .map(|record| record.id.clone())
            .collect();
        if has_include_selector {
            candidates.retain(|agent_id| tag_matched.contains(agent_id));
        } else {
            candidates.extend(tag_matched);
        }
    } else if !has_include_selector && matches!(cover_mode, CoverMode::All | CoverMode::Any) {
        candidates.extend(records.iter().map(|record| record.id.clone()));
    }

    let excluded: HashSet<String> = selector.exclude_server_ids.iter().cloned().collect();
    let mut seen = HashSet::new();
    let mut resolved = Vec::new();
    for agent_id in candidates {
        if excluded.contains(&agent_id) || !owned_ids.contains(&agent_id) {
            continue;
        }
        if seen.insert(agent_id.clone()) {
            resolved.push(agent_id);
        }
    }

    if matches!(cover_mode, CoverMode::Any) {
        resolved.truncate(1);
    }

    Ok(resolved)
}

#[derive(Debug, Clone)]
struct AgentSelectorRecord {
    id: String,
    name: String,
    dashboard_metadata_json: Option<String>,
    last_info_json: Option<String>,
    last_state_json: Option<String>,
}

async fn list_owned_agent_records(
    db: &Db,
    owner_user_id: &str,
) -> Result<Vec<AgentSelectorRecord>> {
    use sqlx::Row;
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT id, name,
                       CASE WHEN dashboard_metadata_json IS NOT NULL AND length(CAST(dashboard_metadata_json AS BLOB)) <= ? THEN dashboard_metadata_json ELSE NULL END AS dashboard_metadata_json,
                       CASE WHEN last_info_json IS NOT NULL AND length(CAST(last_info_json AS BLOB)) <= ? THEN last_info_json ELSE NULL END AS last_info_json,
                       CASE WHEN last_state_json IS NOT NULL AND length(CAST(last_state_json AS BLOB)) <= ? THEN last_state_json ELSE NULL END AS last_state_json
                FROM agents
                WHERE owner_user_id = ? AND revoked_at IS NULL
                ORDER BY created_at ASC
                "#,
            )
            .bind(AGENT_DASHBOARD_METADATA_JSON_MAX_BYTES as i64)
            .bind(AGENT_TELEMETRY_JSON_MAX_BYTES as i64)
            .bind(AGENT_TELEMETRY_JSON_MAX_BYTES as i64)
            .bind(owner_user_id)
            .fetch_all(pool)
            .await?;
            rows.into_iter()
                .map(|row| {
                    Ok(AgentSelectorRecord {
                        id: row.try_get("id")?,
                        name: row.try_get("name")?,
                        dashboard_metadata_json: row.try_get("dashboard_metadata_json")?,
                        last_info_json: row.try_get("last_info_json")?,
                        last_state_json: row.try_get("last_state_json")?,
                    })
                })
                .collect()
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let owner_id = uuid::Uuid::parse_str(owner_user_id)?;
            let rows = sqlx::query(
                r#"
                SELECT id::text AS id, name,
                       CASE WHEN dashboard_metadata_json IS NOT NULL AND octet_length(dashboard_metadata_json) <= $2 THEN dashboard_metadata_json ELSE NULL END AS dashboard_metadata_json,
                       CASE WHEN last_info_json IS NOT NULL AND octet_length(last_info_json) <= $3 THEN last_info_json ELSE NULL END AS last_info_json,
                       CASE WHEN last_state_json IS NOT NULL AND octet_length(last_state_json) <= $3 THEN last_state_json ELSE NULL END AS last_state_json
                FROM agents
                WHERE owner_user_id = $1 AND revoked_at IS NULL
                ORDER BY created_at ASC
                "#,
            )
            .bind(owner_id)
            .bind(AGENT_DASHBOARD_METADATA_JSON_MAX_BYTES as i64)
            .bind(AGENT_TELEMETRY_JSON_MAX_BYTES as i64)
            .fetch_all(pool)
            .await?;
            rows.into_iter()
                .map(|row| {
                    Ok(AgentSelectorRecord {
                        id: row.try_get("id")?,
                        name: row.try_get("name")?,
                        dashboard_metadata_json: row.try_get("dashboard_metadata_json")?,
                        last_info_json: row.try_get("last_info_json")?,
                        last_state_json: row.try_get("last_state_json")?,
                    })
                })
                .collect()
        }
    }
}

async fn list_group_agent_ids(
    db: &Db,
    owner_user_id: &str,
    group_ids: &[String],
) -> Result<Vec<String>> {
    if group_ids.is_empty() {
        return Ok(Vec::new());
    }

    use sqlx::Row;
    let mut out = Vec::new();
    match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            for group_id in group_ids {
                let rows = sqlx::query(
                    r#"
                    SELECT sgm.agent_id
                    FROM server_group_members sgm
                    JOIN server_groups sg ON sg.id = sgm.group_id
                    JOIN agents a ON a.id = sgm.agent_id
                    WHERE sg.id = ?
                      AND sg.owner_user_id = ?
                      AND a.owner_user_id = ?
                      AND a.revoked_at IS NULL
                    ORDER BY sgm.created_at ASC
                    "#,
                )
                .bind(group_id)
                .bind(owner_user_id)
                .bind(owner_user_id)
                .fetch_all(pool)
                .await?;
                out.extend(
                    rows.into_iter()
                        .map(|row| row.try_get::<String, _>("agent_id"))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let owner_id = uuid::Uuid::parse_str(owner_user_id)?;
            for group_id in group_ids {
                let Ok(group_uuid) = uuid::Uuid::parse_str(group_id) else {
                    continue;
                };
                let rows = sqlx::query(
                    r#"
                    SELECT sgm.agent_id::text AS agent_id
                    FROM server_group_members sgm
                    JOIN server_groups sg ON sg.id = sgm.group_id
                    JOIN agents a ON a.id = sgm.agent_id
                    WHERE sg.id = $1
                      AND sg.owner_user_id = $2
                      AND a.owner_user_id = $2
                      AND a.revoked_at IS NULL
                    ORDER BY sgm.created_at ASC
                    "#,
                )
                .bind(group_uuid)
                .bind(owner_id)
                .fetch_all(pool)
                .await?;
                out.extend(
                    rows.into_iter()
                        .map(|row| row.try_get::<String, _>("agent_id"))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
        }
    }
    Ok(out)
}

fn normalized_selector_tags(selector: &ServerSelector) -> Vec<String> {
    let mut tags = selector
        .tag_names
        .iter()
        .filter_map(|tag| normalize_tag_filter(tag))
        .collect::<Vec<_>>();

    for (key, value) in &selector.tags {
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() || value == "*" || value.eq_ignore_ascii_case("true") {
            if let Some(tag) = normalize_tag_filter(key) {
                tags.push(tag);
            }
            continue;
        }
        if let Some(tag) = normalize_tag_filter(value) {
            tags.push(tag);
        }
        if let Some(tag) = normalize_tag_filter(&format!("{key}:{value}")) {
            tags.push(tag);
        }
    }

    let mut seen = HashSet::new();
    tags.into_iter()
        .filter(|tag| seen.insert(tag.clone()))
        .collect()
}

fn normalize_tag_filter(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_start_matches('#').to_lowercase();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn agent_matches_tag_filters(record: &AgentSelectorRecord, filters: &[String]) -> bool {
    let tags = agent_tags(record);
    filters.iter().all(|filter| tags.contains(filter))
}

fn agent_tags(record: &AgentSelectorRecord) -> HashSet<String> {
    let mut out = HashSet::new();
    let sources = [
        parse_dashboard_metadata_json::<serde_json::Value>(
            record.dashboard_metadata_json.as_deref(),
        ),
        parse_agent_telemetry_json::<serde_json::Value>(record.last_info_json.as_deref()),
        parse_agent_telemetry_json::<serde_json::Value>(record.last_state_json.as_deref()),
    ];
    for tag in sources.iter().flatten().flat_map(tags_from_json_source) {
        if let Some(tag) = normalize_tag_filter(&tag) {
            out.insert(tag);
        }
    }
    if let Some(tag) = normalize_tag_filter(&record.name) {
        out.insert(tag);
    }
    out
}

fn tags_from_json_source(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["tags", "labels"] {
        if let Some(value) = value.get(key) {
            out.extend(tags_from_json_value(value));
        }
    }
    for container in ["metadata", "custom"] {
        if let Some(child) = value.get(container) {
            for key in ["tags", "labels"] {
                if let Some(value) = child.get(key) {
                    out.extend(tags_from_json_value(value));
                }
            }
        }
    }
    out
}

fn tags_from_json_value(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        serde_json::Value::String(value) => value
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        serde_json::Value::Object(map) => map
            .iter()
            .flat_map(|(key, value)| {
                if let Some(label) = value.as_str() {
                    vec![label.to_string(), format!("{key}:{label}")]
                } else if value.as_bool() == Some(true) {
                    vec![key.to_string()]
                } else {
                    Vec::new()
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

async fn send_task_dispatch_notification(
    db: &Db,
    task: &Task,
    report: &TaskDispatchReport,
) -> Result<()> {
    let Some(group_id) = task.notification_group_id.as_deref() else {
        return Ok(());
    };
    let has_failure =
        report.summary.failure > 0 || report.summary.offline > 0 || report.summary.timeout > 0;
    if !has_failure && !task.push_successful {
        return Ok(());
    }

    let channels = notification_channels_for_group(db, group_id, &task.owner_user_id).await?;
    if channels.is_empty() {
        return Ok(());
    }

    let result = if has_failure { "failure" } else { "success" };
    let severity = if has_failure {
        NotificationSeverity::Error
    } else {
        NotificationSeverity::Info
    };
    let mut metadata = HashMap::new();
    metadata.insert("task_id".to_string(), task.id.clone());
    metadata.insert("task_name".to_string(), task.name.clone());
    metadata.insert("result".to_string(), result.to_string());
    metadata.insert("success".to_string(), report.summary.success.to_string());
    metadata.insert("failure".to_string(), report.summary.failure.to_string());
    metadata.insert("offline".to_string(), report.summary.offline.to_string());
    metadata.insert("timeout".to_string(), report.summary.timeout.to_string());
    metadata.insert("total".to_string(), report.summary.total.to_string());

    let message = NotificationMessage {
        title: format!(
            "任务执行{}：{}",
            if has_failure { "异常" } else { "成功" },
            task.name
        ),
        message: format!(
            "任务 {} 执行完成：success={}, failure={}, offline={}, timeout={}, total={}",
            task.name,
            report.summary.success,
            report.summary.failure,
            report.summary.offline,
            report.summary.timeout,
            report.summary.total
        ),
        severity,
        timestamp: Utc::now().to_rfc3339(),
        metadata,
    };

    let sender = Arc::new(NotificationSender::new());
    for channel in channels {
        let sender = sender.clone();
        let message = message.clone();
        tokio::spawn(async move {
            if let Err(err) = sender.send(&channel, &message).await {
                warn!("task notification send failed: {}", err);
            }
        });
    }
    Ok(())
}

async fn notification_channels_for_group(
    db: &Db,
    group_id: &str,
    owner_user_id: &str,
) -> Result<Vec<NotificationChannel>> {
    let rows: Vec<(
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        bool,
    )> = match db {
        crate::db::DatabaseBackend::Sqlite(pool) => {
            sqlx::query_as("SELECT n.id, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls FROM notifications n JOIN notification_group_members ngm ON ngm.notification_id = n.id JOIN notification_groups ng ON ng.id = ngm.group_id WHERE ngm.group_id = ? AND ng.owner_user_id = ? AND n.owner_user_id = ? ORDER BY n.created_at ASC LIMIT ?")
                .bind(group_id)
                .bind(owner_user_id)
                .bind(owner_user_id)
                .bind((NOTIFICATION_MAX_GROUP_CHANNELS + 1) as i64)
                .fetch_all(pool)
                .await?
        }
        crate::db::DatabaseBackend::Postgres(pool) => {
            let group_uuid = uuid::Uuid::parse_str(group_id)?;
            let owner_uuid = uuid::Uuid::parse_str(owner_user_id)?;
            sqlx::query_as("SELECT n.id::text, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls FROM notifications n JOIN notification_group_members ngm ON ngm.notification_id = n.id JOIN notification_groups ng ON ng.id = ngm.group_id WHERE ngm.group_id = $1 AND ng.owner_user_id = $2 AND n.owner_user_id = $2 ORDER BY n.created_at ASC LIMIT $3")
                .bind(group_uuid)
                .bind(owner_uuid)
                .bind((NOTIFICATION_MAX_GROUP_CHANNELS + 1) as i64)
                .fetch_all(pool)
                .await?
        }
    };
    ensure_notification_channel_count_allowed(rows.len())?;

    let mut out = Vec::new();
    for (id, name, url, request_method, request_type, headers_json, body_template, verify_tls) in
        rows
    {
        match notification_channel_from_values(
            id.clone(),
            name,
            url,
            request_method,
            request_type,
            headers_json,
            body_template.unwrap_or_default(),
            verify_tls,
        ) {
            Ok(channel) => out.push(channel),
            Err(err) => warn!("historical task notification channel {id} skipped: {err}"),
        }
    }
    Ok(out)
}

async fn create_run(
    db: &Db,
    task_id: &str,
    server_id: &str,
    status: TaskStatus,
    delay_ms: i64,
    output: Option<String>,
    output_truncated: bool,
    error: Option<String>,
) -> Result<TaskRun> {
    let run = TaskRun {
        id: uuid::Uuid::now_v7().to_string(),
        task_id: task_id.to_string(),
        server_id: server_id.to_string(),
        status,
        delay_ms: Some(delay_ms),
        output,
        output_truncated,
        error,
        created_at: Utc::now().to_rfc3339(),
    };
    TaskRunRepository::create(db, &run).await?;
    Ok(run)
}

async fn update_and_build_run(
    db: &Db,
    run_id: &str,
    task_id: &str,
    server_id: &str,
    status: TaskStatus,
    delay_ms: i64,
    output: Option<String>,
    output_truncated: bool,
    error: Option<String>,
    created_at: String,
) -> Result<TaskRun> {
    TaskRunRepository::update_result(
        db,
        run_id,
        status,
        delay_ms,
        output.clone(),
        output_truncated,
        error.clone(),
    )
    .await?;

    Ok(TaskRun {
        id: run_id.to_string(),
        task_id: task_id.to_string(),
        server_id: server_id.to_string(),
        status,
        delay_ms: Some(delay_ms),
        output,
        output_truncated,
        error,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlstatus_shared::tasks::{CoverMode, ServerSelector, Task, TaskType};
    use xlstatus_shared::AgentId;

    #[tokio::test]
    async fn triggered_task_requires_matching_owner_when_provided() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_task(
            &db,
            "00000000-0000-0000-0000-000000000301",
            other,
            "other-task",
        )
        .await;

        let err = run_triggered_task(
            db,
            crate::grpc::SessionRegistry::new(),
            crate::current_task_response_registry(),
            "00000000-0000-0000-0000-000000000301",
            "alert:dirty",
            None,
            Some(owner),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("owner does not match"));
    }

    #[tokio::test]
    async fn triggered_task_rejects_missing_or_empty_owner() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let task_id = "00000000-0000-0000-0000-000000000301";
        seed_user(&db, owner, "owner").await;
        seed_task(&db, task_id, owner, "owner-task").await;

        for missing_owner in [None, Some(""), Some("   ")] {
            let err = run_triggered_task(
                db.clone(),
                crate::grpc::SessionRegistry::new(),
                crate::current_task_response_registry(),
                task_id,
                "alert:dirty",
                None,
                missing_owner,
            )
            .await
            .unwrap_err();
            assert!(err
                .to_string()
                .contains("Trigger owner_user_id is required"));
        }
    }

    #[test]
    fn http_get_task_validates_url_scheme_credentials_and_private_targets() {
        let mut valid = sample_task();
        valid.task_type = TaskType::HttpGet;
        valid.command = Some("https://example.com/health".to_string());
        valid.payload_json = Some(r#"{"timeout_secs":30}"#.to_string());
        assert!(validate_task_definition(&valid).is_ok());

        let mut payload_only = sample_task();
        payload_only.task_type = TaskType::HttpGet;
        payload_only.command = None;
        payload_only.payload_json =
            Some(r#"{"url":"https://example.com/health","timeout_secs":15}"#.to_string());
        assert!(validate_task_definition(&payload_only).is_ok());

        for bad_url in [
            "file:///etc/passwd",
            "ftp://example.com/file",
            "https://user:pass@example.com/health",
            "http://127.0.0.1:8080/admin",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/internal",
            "http://[::1]:8080/internal",
            "http://localhost:8080/internal",
            "https://example.com/path with spaces",
        ] {
            let mut bad = sample_task();
            bad.task_type = TaskType::HttpGet;
            bad.command = Some(bad_url.to_string());
            assert!(
                validate_task_definition(&bad).is_err(),
                "expected http_get URL {bad_url:?} to be rejected"
            );
        }
    }

    #[test]
    fn icmp_ping_task_rejects_option_injection_whitespace_and_private_hosts() {
        let mut valid = sample_task();
        valid.task_type = TaskType::IcmpPing;
        valid.command = Some("1.1.1.1".to_string());
        valid.payload_json = Some(r#"{"count":4,"timeout_secs":10}"#.to_string());
        assert!(validate_task_definition(&valid).is_ok());

        for bad_host in [
            "-W 999999",
            "-f",
            "--flood",
            "example.com;id",
            "example.com/path",
            "host with space",
            "127.0.0.1",
            "169.254.169.254",
            "::1",
            "localhost",
            &format!("{}.example.com", "a".repeat(250)),
        ] {
            let mut bad = sample_task();
            bad.task_type = TaskType::IcmpPing;
            bad.command = Some(bad_host.to_string());
            assert!(
                validate_task_definition(&bad).is_err(),
                "expected icmp_ping host {bad_host:?} to be rejected"
            );
        }
    }

    #[test]
    fn tcp_ping_task_validates_host_port_and_blocks_private_targets() {
        let mut valid = sample_task();
        valid.task_type = TaskType::TcpPing;
        valid.command = Some("example.com:443".to_string());
        assert!(validate_task_definition(&valid).is_ok());

        let mut valid_ipv6 = sample_task();
        valid_ipv6.task_type = TaskType::TcpPing;
        valid_ipv6.command = Some("[2606:4700:4700::1111]:443".to_string());
        assert!(validate_task_definition(&valid_ipv6).is_ok());

        for bad_target in [
            "example.com",
            "example.com:0",
            "example.com:70000",
            "-flag:80",
            "127.0.0.1:22",
            "10.0.0.1:6379",
            "[::1]:8080",
            "localhost:5432",
        ] {
            let mut bad = sample_task();
            bad.task_type = TaskType::TcpPing;
            bad.command = Some(bad_target.to_string());
            assert!(
                validate_task_definition(&bad).is_err(),
                "expected tcp_ping target {bad_target:?} to be rejected"
            );
        }
    }

    #[test]
    fn task_definition_bounds_command_and_selector() {
        let mut task = sample_task();
        task.command = Some("x".repeat(TASK_MAX_COMMAND_BYTES + 1));
        assert!(validate_task_definition(&task)
            .unwrap_err()
            .to_string()
            .contains("task.command"));

        let mut task = sample_task();
        task.server_selector_json = serde_json::to_string(&ServerSelector {
            server_ids: (0..=TASK_MAX_SELECTOR_IDS)
                .map(|idx| format!("server-{idx}"))
                .collect(),
            ..ServerSelector::default()
        })
        .unwrap();
        assert!(validate_task_definition(&task)
            .unwrap_err()
            .to_string()
            .contains("too many entries"));
    }

    #[test]
    fn task_dispatch_target_count_is_bounded() {
        assert!(ensure_task_target_count_allowed(TASK_MAX_DISPATCH_TARGETS).is_ok());
        let err = ensure_task_target_count_allowed(TASK_MAX_DISPATCH_TARGETS + 1).unwrap_err();
        assert!(err.to_string().contains("maximum"));
    }

    #[tokio::test]
    async fn task_dispatches_agents_concurrently_and_preserves_target_order() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let first_agent = "00000000-0000-0000-0000-000000000201";
        let second_agent = "00000000-0000-0000-0000-000000000202";
        seed_user(&db, owner, "owner").await;
        seed_agent(&db, first_agent, owner, None).await;
        seed_agent(&db, second_agent, owner, None).await;

        let mut task = sample_task();
        task.server_selector_json = serde_json::to_string(&ServerSelector {
            server_ids: vec![first_agent.to_string(), second_agent.to_string()],
            ..ServerSelector::default()
        })
        .unwrap();
        TaskRepository::create(&db, &task).await.unwrap();

        let sessions = SessionRegistry::new();
        let first_id = AgentId(uuid::Uuid::parse_str(first_agent).unwrap());
        let second_id = AgentId(uuid::Uuid::parse_str(second_agent).unwrap());
        let (first_tx, mut first_rx) = tokio::sync::mpsc::channel(1);
        let (second_tx, mut second_rx) = tokio::sync::mpsc::channel(1);
        assert!(sessions.register(first_id, first_tx).await);
        assert!(sessions.register(second_id, second_tx).await);
        let responses = Arc::new(TaskResponseRegistry::new());

        let dispatch_db = db.clone();
        let dispatch_sessions = sessions.clone();
        let dispatch_responses = responses.clone();
        let dispatch_task = task.clone();
        let dispatch = tokio::spawn(async move {
            dispatch_task_to_agents(
                &dispatch_db,
                &dispatch_sessions,
                dispatch_responses,
                &dispatch_task,
            )
            .await
        });

        let (first_message, second_message) =
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                tokio::join!(first_rx.recv(), second_rx.recv())
            })
            .await
            .expect("both Agents should receive work before either replies");
        assert!(!dispatch.is_finished());
        let first_task_id = dispatched_task_id(first_message.unwrap().unwrap());
        let second_task_id = dispatched_task_id(second_message.unwrap().unwrap());

        for (agent_id, task_id) in [(first_id, first_task_id), (second_id, second_task_id)] {
            let result_task_id = task_id.clone();
            assert!(
                responses
                    .deliver(
                        &agent_id,
                        &task_id,
                        xlstatus_proto_gen::xlstatus::v1::TaskResult {
                            task_id: result_task_id,
                            status: xlstatus_proto_gen::xlstatus::v1::TaskOutcome::Success as i32,
                            ..Default::default()
                        },
                    )
                    .await
            );
        }

        let report = tokio::time::timeout(std::time::Duration::from_secs(1), dispatch)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(report.summary.success, 2);
        assert_eq!(
            report
                .runs
                .iter()
                .map(|run| run.server_id.as_str())
                .collect::<Vec<_>>(),
            vec![first_agent, second_agent]
        );
    }

    #[tokio::test]
    async fn aborting_task_dispatch_cleans_pending_response_waiters() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let first_agent = "00000000-0000-0000-0000-000000000211";
        let second_agent = "00000000-0000-0000-0000-000000000212";
        seed_user(&db, owner, "owner").await;
        seed_agent(&db, first_agent, owner, None).await;
        seed_agent(&db, second_agent, owner, None).await;

        let mut task = sample_task();
        task.server_selector_json = serde_json::to_string(&ServerSelector {
            server_ids: vec![first_agent.to_string(), second_agent.to_string()],
            ..ServerSelector::default()
        })
        .unwrap();
        TaskRepository::create(&db, &task).await.unwrap();

        let sessions = SessionRegistry::new();
        let first_id = AgentId(uuid::Uuid::parse_str(first_agent).unwrap());
        let second_id = AgentId(uuid::Uuid::parse_str(second_agent).unwrap());
        let (first_tx, mut first_rx) = tokio::sync::mpsc::channel(1);
        let (second_tx, mut second_rx) = tokio::sync::mpsc::channel(1);
        assert!(sessions.register(first_id, first_tx).await);
        assert!(sessions.register(second_id, second_tx).await);
        let responses = Arc::new(TaskResponseRegistry::new());

        let dispatch = tokio::spawn({
            let db = db.clone();
            let sessions = sessions.clone();
            let responses = responses.clone();
            let task = task.clone();
            async move { dispatch_task_to_agents(&db, &sessions, responses, &task).await }
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(first_rx.recv(), second_rx.recv())
        })
        .await
        .expect("both Agents should receive work before dispatch cancellation");
        assert_eq!(responses.pending_count(), 2);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let rows = task_run_status_rows(&db, &task.id).await;
                if rows.len() == 2 && rows.iter().all(|(status, _)| status == "pending") {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("both task runs should be persisted as pending before cancellation");

        dispatch.abort();
        assert!(dispatch.await.unwrap_err().is_cancelled());
        assert_eq!(responses.pending_count(), 0);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let rows = task_run_status_rows(&db, &task.id).await;
                if rows.len() == 2 && rows.iter().all(|(status, _)| status != "pending") {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled task runs should leave pending state");
        let rows = task_run_status_rows(&db, &task.id).await;
        assert!(rows.iter().all(|(status, error)| {
            status == "failed" && error.as_deref() == Some(TASK_RUN_CANCELLATION_ERROR)
        }));
    }

    fn dispatched_task_id(message: xlstatus_proto_gen::xlstatus::v1::ServerMessage) -> String {
        match message.payload {
            Some(xlstatus_proto_gen::xlstatus::v1::server_message::Payload::Task(task)) => {
                task.task_id
            }
            _ => panic!("expected dispatched task message"),
        }
    }

    #[test]
    fn task_result_output_is_truncated_before_persistence() {
        let mut result = xlstatus_proto_gen::xlstatus::v1::TaskResult {
            stdout: "x".repeat(TASK_RESULT_STDOUT_MAX_BYTES + 1),
            stderr: "y".repeat(TASK_RESULT_STDERR_MAX_BYTES + 1),
            error: "z".repeat(TASK_RESULT_ERROR_MAX_BYTES + 1),
            ..Default::default()
        };

        let truncated = truncate_task_result_text(
            &mut result,
            TASK_RESULT_STDOUT_MAX_BYTES,
            TASK_RESULT_STDERR_MAX_BYTES,
            TASK_RESULT_ERROR_MAX_BYTES,
        );

        assert!(truncated.any());
        assert_eq!(result.stdout.len(), TASK_RESULT_STDOUT_MAX_BYTES);
        assert_eq!(result.stderr.len(), TASK_RESULT_STDERR_MAX_BYTES);
        assert_eq!(result.error.len(), TASK_RESULT_ERROR_MAX_BYTES);
    }

    #[tokio::test]
    async fn task_group_selector_ignores_cross_owner_and_revoked_dirty_members() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let other = "00000000-0000-0000-0000-000000000002";
        let group_id = "00000000-0000-0000-0000-000000000101";
        let owner_agent = "00000000-0000-0000-0000-000000000201";
        let other_agent = "00000000-0000-0000-0000-000000000202";
        let revoked_agent = "00000000-0000-0000-0000-000000000203";

        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_agent(&db, owner_agent, owner, None).await;
        seed_agent(&db, other_agent, other, None).await;
        seed_agent(&db, revoked_agent, owner, Some("2026-01-02T00:00:00Z")).await;
        seed_server_group(&db, group_id, owner).await;
        seed_server_group_member(&db, group_id, owner_agent).await;
        seed_server_group_member(&db, group_id, other_agent).await;
        seed_server_group_member(&db, group_id, revoked_agent).await;

        let resolved = list_group_agent_ids(&db, owner, &[group_id.to_string()])
            .await
            .unwrap();

        assert_eq!(resolved, vec![owner_agent.to_string()]);
    }

    #[tokio::test]
    async fn task_tag_selector_ignores_oversized_historical_agent_metadata() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let valid_agent = "00000000-0000-0000-0000-000000000201";
        let dirty_agent = "00000000-0000-0000-0000-000000000202";
        seed_user(&db, owner, "owner").await;
        seed_agent(&db, valid_agent, owner, None).await;
        seed_agent(&db, dirty_agent, owner, None).await;
        set_agent_dashboard_metadata(&db, valid_agent, r#"{"tags":["prod"]}"#).await;
        let oversized = format!(
            r#"{{"tags":["prod"],"padding":"{}"}}"#,
            "x".repeat(AGENT_DASHBOARD_METADATA_JSON_MAX_BYTES)
        );
        set_agent_dashboard_metadata(&db, dirty_agent, &oversized).await;
        let selector = ServerSelector {
            tag_names: vec!["prod".into()],
            ..Default::default()
        };

        let resolved = resolve_server_ids(&db, &selector, CoverMode::All, owner, None)
            .await
            .unwrap();

        assert_eq!(resolved, vec![valid_agent.to_string()]);
    }

    #[tokio::test]
    async fn notification_channels_skip_invalid_historical_rows() {
        let db = test_db().await;
        let owner = "00000000-0000-0000-0000-000000000001";
        let group_id = "00000000-0000-0000-0000-000000000101";
        let valid_id = "00000000-0000-0000-0000-000000000201";
        let dirty_headers_id = "00000000-0000-0000-0000-000000000202";
        let dirty_method_id = "00000000-0000-0000-0000-000000000203";

        seed_user(&db, owner, "owner").await;
        seed_notification_group(&db, group_id, owner).await;
        seed_notification(
            &db,
            valid_id,
            owner,
            "valid",
            Some(r#"{"Authorization":"secret"}"#),
            "POST",
            "json",
        )
        .await;
        seed_notification(
            &db,
            dirty_headers_id,
            owner,
            "dirty-headers",
            Some("{not-json"),
            "POST",
            "json",
        )
        .await;
        seed_notification(
            &db,
            dirty_method_id,
            owner,
            "dirty-method",
            None,
            "TRACE",
            "json",
        )
        .await;
        for id in [valid_id, dirty_headers_id, dirty_method_id] {
            seed_notification_group_member(&db, group_id, id).await;
        }

        let channels = notification_channels_for_group(&db, group_id, owner)
            .await
            .unwrap();

        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].id, valid_id);
        assert_eq!(
            channels[0].headers.get("Authorization").map(String::as_str),
            Some("secret")
        );
    }

    async fn test_db() -> Db {
        let db = Db::connect("sqlite::memory:", true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn task_run_status_rows(db: &Db, task_id: &str) -> Vec<(String, Option<String>)> {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query_as("SELECT status, error FROM task_runs WHERE task_id = ? ORDER BY id")
            .bind(task_id)
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn seed_user(db: &Db, id: &str, username: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'hash', 'member', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(username)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_agent(db: &Db, id: &str, owner: &str, revoked_at: Option<&str>) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, revoked_at, created_at, updated_at) VALUES (?, ?, 'pk', ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("agent-{id}"))
        .bind(owner)
        .bind(revoked_at)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO servers (id, owner_user_id, name, created_at, updated_at, agent_id) VALUES (?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?)",
        )
        .bind(id)
        .bind(owner)
        .bind(format!("server-{id}"))
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_server_group(db: &Db, id: &str, owner: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO server_groups (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, 'group', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_server_group_member(db: &Db, group_id: &str, agent_id: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO server_group_members (group_id, agent_id, created_at) VALUES (?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(group_id)
        .bind(agent_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn set_agent_dashboard_metadata(db: &Db, agent_id: &str, metadata: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query("UPDATE agents SET dashboard_metadata_json = ? WHERE id = ?")
            .bind(metadata)
            .bind(agent_id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_notification_group(db: &Db, id: &str, owner: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notification_groups (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, 'group', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_notification(
        db: &Db,
        id: &str,
        owner: &str,
        name: &str,
        headers_json: Option<&str>,
        request_method: &str,
        request_type: &str,
    ) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notifications (id, owner_user_id, name, url, request_method, request_type, headers_json, verify_tls, format_metric_units, created_at, updated_at) VALUES (?, ?, ?, 'https://example.com/hook', ?, ?, ?, 1, 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner)
        .bind(name)
        .bind(request_method)
        .bind(request_type)
        .bind(headers_json)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_notification_group_member(db: &Db, group_id: &str, notification_id: &str) {
        let Db::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notification_group_members (group_id, notification_id) VALUES (?, ?)",
        )
        .bind(group_id)
        .bind(notification_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_task(db: &Db, id: &str, owner: &str, name: &str) {
        let mut task = sample_task();
        task.id = id.to_string();
        task.owner_user_id = owner.to_string();
        task.name = name.to_string();
        TaskRepository::create(db, &task).await.unwrap();
    }

    fn sample_task() -> Task {
        Task {
            id: "00000000-0000-0000-0000-000000000101".to_string(),
            owner_user_id: "00000000-0000-0000-0000-000000000001".to_string(),
            name: "task".to_string(),
            task_type: TaskType::Shell,
            schedule: None,
            command: Some("true".to_string()),
            payload_json: None,
            cover_mode: CoverMode::Specific,
            server_selector_json: serde_json::to_string(&ServerSelector {
                server_ids: Vec::new(),
                group_ids: Vec::new(),
                tag_names: Vec::new(),
                tags: HashMap::new(),
                exclude_server_ids: Vec::new(),
                source_server: false,
            })
            .unwrap(),
            push_successful: false,
            notification_group_id: None,
            last_executed_at: None,
            last_result: None,
            enabled: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }
}
