use crate::db::Db;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::Row;
use std::collections::BTreeMap;
use xlstatus_proto_gen::xlstatus::v1::{
    server_task::Spec, FileListTask, FileReadTask, ServerTask, TaskOutcome, TaskType,
};
use xlstatus_shared::{AgentId, UserId};
use xlstatus_tsdb::QueryRange;

use super::tools::{McpToolRequest, McpToolResponse};
use crate::api::v1::agent_json::{parse_agent_telemetry_json, parse_dashboard_metadata_json};
use crate::auth::generate_temporary_transfer_token;
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::db::repository::agent::AgentRepository;
use crate::db::repository::alerts::{AlertEventRepository, AlertRepository};
use crate::db::repository::ddns::DdnsConfigRepository;
use crate::db::repository::nat::NatMappingRepository;
use crate::db::repository::tasks::TaskRepository;
use crate::db::{
    AgentWithState, CreateTemporaryTransferTokenInput, TemporaryTransferTokenRepository,
};
use crate::grpc::{base64_encoded_len, ensure_task_result_text_within};

const MCP_FILE_OP_TIMEOUT_SECS: u64 = 30;
const MCP_FILE_READ_MAX_BYTES: u64 = 1024 * 1024;
const MCP_FILE_LIST_RESULT_MAX_BYTES: usize = 1024 * 1024;
const MCP_SMALL_RESULT_MAX_BYTES: usize = 4096;
const MCP_EXEC_MAX_COMMAND_BYTES: usize = 8192;
const MCP_EXEC_DEFAULT_TIMEOUT_SECS: u32 = 30;
const MCP_EXEC_MAX_TIMEOUT_SECS: u32 = 60;
const MCP_EXEC_RESULT_MAX_BYTES: usize = 64 * 1024;
const MCP_BATCH_EXEC_MAX_SERVERS: usize = 10;
const MCP_UUID_TEXT_LEN: usize = 36;
const MCP_ONLINE_THRESHOLD_SECS: i64 = 30;
pub(crate) const TEMP_URL_DEFAULT_EXPIRES_SECS: i64 = 300;
pub(crate) const TEMP_URL_MAX_EXPIRES_SECS: i64 = 600;

/// MCP tool, resource, and prompt executor
pub struct McpExecutor {
    db: Db,
    /// M6: handle to the live gRPC session registry so
    /// `server.exec` and `fs.*` can dispatch a task to a real
    /// agent and wait for the result.
    session_registry: crate::grpc::SessionRegistry,
    temp_url_secret: String,
    metrics: Option<xlstatus_tsdb::MetricStore>,
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
            metrics: None,
        }
    }

    pub fn with_metrics(mut self, metrics: xlstatus_tsdb::MetricStore) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Execute an MCP tool
    pub async fn execute(&self, auth: &AuthSession, request: McpToolRequest) -> McpToolResponse {
        let result = match request.tool.as_str() {
            "meta.whoami" => self.exec_whoami(auth).await,
            "cluster.overview" => self.exec_cluster_overview(auth).await,
            "cluster.diagnose" => self.exec_cluster_diagnose(auth, &request.arguments).await,
            "server.list" => self.exec_server_list(auth, &request.arguments).await,
            "server.get" => self.exec_server_get(auth, &request.arguments).await,
            "server.resolve" => self.exec_server_resolve(auth, &request.arguments).await,
            "server.metrics" => self.exec_server_metrics(auth, &request.arguments).await,
            "server.exec" => self.exec_server_exec(auth, &request.arguments).await,
            "server.batch_exec" => self.exec_server_batch_exec(auth, &request.arguments).await,
            "server.inspect" => self.exec_server_inspect(auth, &request.arguments).await,
            "service.list" => self.exec_service_list(auth, &request.arguments).await,
            "service.history" => self.exec_service_history(auth, &request.arguments).await,
            "alert.events" => self.exec_alert_events(auth, &request.arguments).await,
            "alert.rules" => self.exec_alert_rules(auth, &request.arguments).await,
            "task.list" => self.exec_task_list(auth, &request.arguments).await,
            "network.topology" => self.exec_network_topology(auth).await,
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

    /// Read an MCP resource by URI (`xlstatus://...`)
    pub async fn read_resource(&self, auth: &AuthSession, uri: &str) -> Result<Value> {
        let trimmed = uri.trim();
        match trimmed {
            "xlstatus://cluster/overview" => self.exec_cluster_overview(auth).await,
            "xlstatus://servers/inventory" => {
                self.exec_server_list(auth, &json!({ "limit": 200, "offset": 0 }))
                    .await
            }
            "xlstatus://services/health" => {
                self.exec_service_list(auth, &json!({ "limit": 200 })).await
            }
            "xlstatus://alerts/active" => {
                let events = self
                    .exec_alert_events(auth, &json!({ "limit": 30 }))
                    .await?;
                let rules = self.exec_alert_rules(auth, &json!({ "limit": 50 })).await?;
                Ok(json!({
                    "uri": trimmed,
                    "events": events["events"],
                    "events_total": events["total"],
                    "rules": rules["rules"],
                    "rules_total": rules["total"],
                    "timestamp": Utc::now().to_rfc3339(),
                }))
            }
            "xlstatus://network/topology" => self.exec_network_topology(auth).await,
            _ => {
                if let Some(rest) = trimmed.strip_prefix("xlstatus://servers/") {
                    if let Some(server_id) = rest.strip_suffix("/telemetry") {
                        let server = self
                            .exec_server_get(auth, &json!({ "server_id": server_id }))
                            .await?;
                        let metrics = self
                            .exec_server_metrics(
                                auth,
                                &json!({ "server_id": server_id, "range": "1h" }),
                            )
                            .await?;
                        return Ok(json!({
                            "uri": trimmed,
                            "server": server,
                            "metrics_1h": metrics,
                            "timestamp": Utc::now().to_rfc3339(),
                        }));
                    }
                }
                anyhow::bail!("Unknown MCP resource URI: {}", trimmed)
            }
        }
    }

    /// Render a dynamic MCP prompt enriched with live cluster telemetry
    pub async fn get_prompt(
        &self,
        auth: &AuthSession,
        name: &str,
        arguments: &Value,
    ) -> Result<Value> {
        match name {
            "sre.cluster_health_audit" => {
                let overview = self.exec_cluster_overview(auth).await?;
                let diagnosis = self.exec_cluster_diagnose(auth, &json!({})).await?;
                let prompt_text = format!(
                    "You are a Principal Site Reliability Engineer auditing an XLStatus cluster.\n\
                     Analyze the following live cluster overview and automated anomaly scan:\n\n\
                     ### 1. Live Cluster Overview\n```json\n{}\n```\n\n\
                     ### 2. Automated Anomaly Scan (`cluster.diagnose`)\n```json\n{}\n```\n\n\
                     Please provide:\n\
                     1. **Executive Fleet Summary**: Health score, online/offline ratio, and overall saturation.\n\
                     2. **Prioritized Findings**: Group anomalies by severity (`critical` first, then `warning`) with probable root causes.\n\
                     3. **Actionable Remediation Plan**: Specific MCP tools (`server.inspect`, `server.exec`, `service.history`) and safe shell commands to run next.",
                    serde_json::to_string_pretty(&overview).unwrap_or_default(),
                    serde_json::to_string_pretty(&diagnosis).unwrap_or_default(),
                );
                Ok(json!({
                    "description": "Comprehensive AI SRE audit of fleet health, resource bottlenecks, offline nodes, and active incidents",
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": prompt_text
                            }
                        }
                    ]
                }))
            }
            "sre.troubleshoot_node" => {
                let raw_query = arguments["server_id"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .context("Missing required prompt argument: server_id")?;
                let symptom = arguments["symptom"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("general performance or availability degradation");

                let resolved_id = if parse_mcp_server_id(raw_query).is_ok() {
                    raw_query.to_string()
                } else {
                    let resolved = self
                        .exec_server_resolve(auth, &json!({ "query": raw_query }))
                        .await?;
                    resolved["best_match"]["server_id"]
                        .as_str()
                        .map(str::to_string)
                        .context(format!("No visible server matched '{}'", raw_query))?
                };

                let server = self
                    .exec_server_get(auth, &json!({ "server_id": resolved_id }))
                    .await?;
                let metrics = self
                    .exec_server_metrics(auth, &json!({ "server_id": resolved_id, "range": "1h" }))
                    .await?;
                let prompt_text = format!(
                    "You are an AI SRE troubleshooting server node `{}` (`{}`).\n\
                     Reported symptom: **{}**\n\n\
                     ### Node Hardware & Live State (`server.get`)\n```json\n{}\n```\n\n\
                     ### 1-Hour Telemetry Summary (`server.metrics`)\n```json\n{}\n```\n\n\
                     Please provide:\n\
                     1. **Telemetry Correlation**: Analyze CPU, memory, swap, disk, load_1/5/15, and network throughput against the reported symptom.\n\
                     2. **Diagnostic Playbook Steps**: Recommend which `server.inspect` playbooks (`system_vital`, `top_processes`, `disk_and_inodes`, `network_sockets`, `docker_containers`, `system_logs`) to run and what indicators to look for.\n\
                     3. **Safe Remediation Commands**: Provide non-destructive remediation steps.",
                    server["name"].as_str().unwrap_or(&resolved_id),
                    resolved_id,
                    symptom,
                    serde_json::to_string_pretty(&server).unwrap_or_default(),
                    serde_json::to_string_pretty(&metrics).unwrap_or_default(),
                );
                Ok(json!({
                    "description": format!("Deep-dive root-cause analysis for server {}", resolved_id),
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": prompt_text
                            }
                        }
                    ]
                }))
            }
            "sre.incident_triage" => {
                let diagnosis = self.exec_cluster_diagnose(auth, &json!({})).await?;
                let events = self
                    .exec_alert_events(auth, &json!({ "limit": 20 }))
                    .await?;
                let services = self
                    .exec_service_list(auth, &json!({ "limit": 50 }))
                    .await?;
                let prompt_text = format!(
                    "You are an Incident Commander triaging active alerts and service probe degradations in XLStatus.\n\n\
                     ### Active Anomalies (`cluster.diagnose`)\n```json\n{}\n```\n\n\
                     ### Recent Alert Events (`alert.events`)\n```json\n{}\n```\n\n\
                     ### Service Monitors (`service.list`)\n```json\n{}\n```\n\n\
                     Please provide:\n\
                     1. **Incident Timeline & Blast Radius**: Correlate alert events with failing service monitors and affected servers.\n\
                     2. **Immediate Triage Actions**: Step-by-step verification and mitigation commands.",
                    serde_json::to_string_pretty(&diagnosis).unwrap_or_default(),
                    serde_json::to_string_pretty(&events).unwrap_or_default(),
                    serde_json::to_string_pretty(&services).unwrap_or_default(),
                );
                Ok(json!({
                    "description": "Triage active alerts and degraded service probes with actionable remediation commands",
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": prompt_text
                            }
                        }
                    ]
                }))
            }
            "sre.capacity_and_security" => {
                let overview = self.exec_cluster_overview(auth).await?;
                let servers = self
                    .exec_server_list(auth, &json!({ "limit": 100, "offset": 0 }))
                    .await?;
                let topology = self.exec_network_topology(auth).await?;
                let prompt_text = format!(
                    "You are an Infrastructure Capacity & Security Architect reviewing an XLStatus cluster.\n\n\
                     ### Cluster Overview\n```json\n{}\n```\n\n\
                     ### Server Fleet Inventory\n```json\n{}\n```\n\n\
                     ### Network Topology (NAT & DDNS)\n```json\n{}\n```\n\n\
                     Please audit:\n\
                     1. **Resource Saturation & Right-Sizing**: Identify over-provisioned or saturated nodes (CPU, RAM, disk).\n\
                     2. **Lifecycle & Billing Risks**: Highlight servers with upcoming `expires_at` dates.\n\
                     3. **Network Exposure**: Review active NAT port mappings (public ports, source restrictions) and DDNS records.",
                    serde_json::to_string_pretty(&overview).unwrap_or_default(),
                    serde_json::to_string_pretty(&servers).unwrap_or_default(),
                    serde_json::to_string_pretty(&topology).unwrap_or_default(),
                );
                Ok(json!({
                    "description": "Audit cluster resource saturation, expiring nodes, and exposed NAT/DDNS endpoints",
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": prompt_text
                            }
                        }
                    ]
                }))
            }
            _ => anyhow::bail!("Unknown MCP prompt: {}", name),
        }
    }

    /// Execute meta.whoami
    async fn exec_whoami(&self, auth: &AuthSession) -> Result<Value> {
        let auth_kind = match auth.auth_kind {
            AuthKind::Session => "session",
            AuthKind::PersonalAccessToken => "pat",
        };
        Ok(json!({
            "user_id": auth.user_id.0.to_string(),
            "username": auth.username,
            "role": format!("{:?}", auth.role).to_lowercase(),
            "auth_kind": auth_kind,
            "scopes": auth.scopes,
            "server_ids": auth.server_ids,
            "system": "XLStatus",
            "version": env!("CARGO_PKG_VERSION"),
            "protocol_version": "2024-11-05",
            "timestamp": Utc::now().to_rfc3339(),
        }))
    }

    /// Execute cluster.overview
    async fn exec_cluster_overview(&self, auth: &AuthSession) -> Result<Value> {
        let rows = self.load_all_visible_agents_with_state(auth, 500).await?;
        let mut total_servers = 0usize;
        let mut online_servers = 0usize;
        let mut offline_servers = 0usize;
        let mut revoked_servers = 0usize;

        let mut cpu_sum = 0.0f64;
        let mut cpu_count = 0usize;
        let mut max_cpu_percent = 0.0f64;

        let mut total_memory_used: i64 = 0;
        let mut total_memory_total: i64 = 0;
        let mut total_disk_used: i64 = 0;
        let mut total_disk_total: i64 = 0;
        let mut total_net_rx_bps: i64 = 0;
        let mut total_net_tx_bps: i64 = 0;

        let mut regions: BTreeMap<String, usize> = BTreeMap::new();
        let mut providers: BTreeMap<String, usize> = BTreeMap::new();

        for row in &rows {
            total_servers += 1;
            let item = self.build_enriched_server_json(row).await;
            let revoked = item["revoked"].as_bool().unwrap_or(false);
            let online = item["online"].as_bool().unwrap_or(false);
            if revoked {
                revoked_servers += 1;
            } else if online {
                online_servers += 1;
            } else {
                offline_servers += 1;
            }

            if let Some(cpu) = item["cpu_percent"].as_f64() {
                cpu_sum += cpu;
                cpu_count += 1;
                if cpu > max_cpu_percent {
                    max_cpu_percent = cpu;
                }
            }
            if let Some(mem_used) = item["memory_used"].as_i64() {
                total_memory_used = total_memory_used.saturating_add(mem_used);
            }
            if let Some(mem_tot) = item["memory_total"].as_i64() {
                total_memory_total = total_memory_total.saturating_add(mem_tot);
            }
            if let Some(d_used) = item["disk_used"].as_i64() {
                total_disk_used = total_disk_used.saturating_add(d_used);
            }
            if let Some(d_tot) = item["disk_total"].as_i64() {
                total_disk_total = total_disk_total.saturating_add(d_tot);
            }
            if let Some(rx) = item["net_rx_bps"].as_i64() {
                total_net_rx_bps = total_net_rx_bps.saturating_add(rx);
            }
            if let Some(tx) = item["net_tx_bps"].as_i64() {
                total_net_tx_bps = total_net_tx_bps.saturating_add(tx);
            }

            let region = item["region"]
                .as_str()
                .or_else(|| item["country"].as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("unassigned")
                .to_string();
            *regions.entry(region).or_insert(0) += 1;

            let provider = item["provider"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("unassigned")
                .to_string();
            *providers.entry(provider).or_insert(0) += 1;
        }

        let avg_cpu_percent = if cpu_count > 0 {
            Some(((cpu_sum / cpu_count as f64) * 10.0).round() / 10.0)
        } else {
            None
        };
        let cluster_memory_percent = if total_memory_total > 0 {
            Some(((total_memory_used as f64 / total_memory_total as f64) * 1000.0).round() / 10.0)
        } else {
            None
        };
        let cluster_disk_percent = if total_disk_total > 0 {
            Some(((total_disk_used as f64 / total_disk_total as f64) * 1000.0).round() / 10.0)
        } else {
            None
        };

        let services_val = self
            .exec_service_list(auth, &json!({ "limit": 200 }))
            .await
            .unwrap_or_else(|_| json!({ "services": [], "total": 0 }));
        let service_list = services_val["services"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let total_services = service_list.len();
        let healthy_services = service_list
            .iter()
            .filter(|s| s["last_status"].as_str() == Some("success"))
            .count();
        let failing_services = service_list
            .iter()
            .filter(|s| {
                s["enabled"].as_bool().unwrap_or(true)
                    && matches!(
                        s["last_status"].as_str(),
                        Some("failure" | "down" | "error" | "timeout")
                    )
            })
            .count();

        let alerts_val = self
            .exec_alert_events(auth, &json!({ "limit": 50 }))
            .await
            .unwrap_or_else(|_| json!({ "events": [], "total": 0 }));
        let alert_list = alerts_val["events"].as_array().cloned().unwrap_or_default();
        let firing_alerts = alert_list
            .iter()
            .filter(|e| {
                let kind = e["kind"].as_str().unwrap_or("");
                kind.contains("fire")
                    || kind.contains("firing")
                    || kind.contains("trigger")
                    || kind == "down"
                    || kind == "alert"
            })
            .count();

        Ok(json!({
            "fleet": {
                "total_servers": total_servers,
                "online_servers": online_servers,
                "offline_servers": offline_servers,
                "revoked_servers": revoked_servers,
                "availability_percent": if total_servers.saturating_sub(revoked_servers) > 0 {
                    ((online_servers as f64 / (total_servers - revoked_servers) as f64) * 1000.0).round() / 10.0
                } else {
                    100.0
                }
            },
            "resources": {
                "avg_cpu_percent": avg_cpu_percent,
                "max_cpu_percent": if cpu_count > 0 { Some((max_cpu_percent * 10.0).round() / 10.0) } else { None },
                "memory_used_bytes": total_memory_used,
                "memory_total_bytes": total_memory_total,
                "memory_percent": cluster_memory_percent,
                "disk_used_bytes": total_disk_used,
                "disk_total_bytes": total_disk_total,
                "disk_percent": cluster_disk_percent,
                "net_rx_bps": total_net_rx_bps,
                "net_tx_bps": total_net_tx_bps,
            },
            "services": {
                "total_monitors": total_services,
                "healthy_monitors": healthy_services,
                "failing_monitors": failing_services,
            },
            "alerts": {
                "recent_events": alert_list.len(),
                "firing_events": firing_alerts,
            },
            "regions": regions,
            "providers": providers,
            "timestamp": Utc::now().to_rfc3339(),
        }))
    }

    /// Execute cluster.diagnose (AI SRE Automated Anomaly & Health Scanner)
    async fn exec_cluster_diagnose(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let cpu_threshold = args["cpu_threshold"]
            .as_f64()
            .unwrap_or(85.0)
            .clamp(10.0, 100.0);
        let memory_threshold = args["memory_threshold"]
            .as_f64()
            .unwrap_or(85.0)
            .clamp(10.0, 100.0);
        let disk_threshold = args["disk_threshold"]
            .as_f64()
            .unwrap_or(85.0)
            .clamp(10.0, 100.0);

        let rows = self.load_all_visible_agents_with_state(auth, 500).await?;
        let mut anomalies: Vec<Value> = Vec::new();
        let mut online_count = 0usize;
        let mut active_nodes = 0usize;

        for row in &rows {
            let item = self.build_enriched_server_json(row).await;
            if item["revoked"].as_bool().unwrap_or(false) {
                continue;
            }
            active_nodes += 1;
            let server_id = item["server_id"].as_str().unwrap_or_default().to_string();
            let name = item["name"].as_str().unwrap_or_default().to_string();
            let online = item["online"].as_bool().unwrap_or(false);

            if !online {
                let last_seen = item["last_seen_at"].as_str().unwrap_or("never connected");
                anomalies.push(json!({
                    "severity": "critical",
                    "category": "availability",
                    "resource_type": "server",
                    "resource_id": server_id,
                    "resource_name": name,
                    "title": format!("Server '{}' is offline", name),
                    "detail": format!("Agent gRPC session is disconnected (last seen: {}).", last_seen),
                    "recommended_action": "Verify host reachability and inspect the xlstatus-agent service on the node.",
                    "suggested_tool": "server.get",
                    "suggested_arguments": { "server_id": server_id }
                }));
                continue;
            }

            online_count += 1;

            if let Some(cpu) = item["cpu_percent"].as_f64() {
                if cpu >= cpu_threshold {
                    let severity = if cpu >= 95.0 { "critical" } else { "warning" };
                    let load_1 = item["load_1"].as_f64().unwrap_or(0.0);
                    anomalies.push(json!({
                        "severity": severity,
                        "category": "compute",
                        "resource_type": "server",
                        "resource_id": server_id,
                        "resource_name": name,
                        "title": format!("High CPU utilization on '{}' ({:.1}%)", name, cpu),
                        "detail": format!("CPU usage is {:.1}% (threshold {:.1}%), load_1={:.2}.", cpu, cpu_threshold, load_1),
                        "recommended_action": "Run the top_processes diagnostic playbook to identify CPU-intensive processes.",
                        "suggested_tool": "server.inspect",
                        "suggested_arguments": { "server_id": server_id, "playbook": "top_processes" }
                    }));
                }
            }

            if let Some(mem_pct) = item["memory_percent"].as_f64() {
                if mem_pct >= memory_threshold {
                    let severity = if mem_pct >= 95.0 {
                        "critical"
                    } else {
                        "warning"
                    };
                    anomalies.push(json!({
                        "severity": severity,
                        "category": "memory",
                        "resource_type": "server",
                        "resource_id": server_id,
                        "resource_name": name,
                        "title": format!("Memory pressure on '{}' ({:.1}%)", name, mem_pct),
                        "detail": format!("Memory utilization is {:.1}% (threshold {:.1}%).", mem_pct, memory_threshold),
                        "recommended_action": "Inspect top memory consumers and check system logs for OOM events.",
                        "suggested_tool": "server.inspect",
                        "suggested_arguments": { "server_id": server_id, "playbook": "top_processes" }
                    }));
                }
            }

            if let Some(disk_pct) = item["disk_percent"].as_f64() {
                if disk_pct >= disk_threshold {
                    let severity = if disk_pct >= 95.0 {
                        "critical"
                    } else {
                        "warning"
                    };
                    anomalies.push(json!({
                        "severity": severity,
                        "category": "storage",
                        "resource_type": "server",
                        "resource_id": server_id,
                        "resource_name": name,
                        "title": format!("Disk capacity warning on '{}' ({:.1}%)", name, disk_pct),
                        "detail": format!("Aggregate disk usage is {:.1}% (threshold {:.1}%).", disk_pct, disk_threshold),
                        "recommended_action": "Run disk_and_inodes playbook to locate full filesystems or exhausted inodes.",
                        "suggested_tool": "server.inspect",
                        "suggested_arguments": { "server_id": server_id, "playbook": "disk_and_inodes" }
                    }));
                }
            }

            if let Some(expires_str) = item["expires_at"].as_str() {
                if let Ok(expires_dt) = DateTime::parse_from_rfc3339(expires_str) {
                    let days_left = expires_dt
                        .with_timezone(&Utc)
                        .signed_duration_since(Utc::now())
                        .num_days();
                    if days_left <= 7 {
                        let severity = if days_left <= 1 {
                            "critical"
                        } else {
                            "warning"
                        };
                        anomalies.push(json!({
                            "severity": severity,
                            "category": "lifecycle",
                            "resource_type": "server",
                            "resource_id": server_id,
                            "resource_name": name,
                            "title": format!("Server '{}' expires in {} day(s)", name, days_left),
                            "detail": format!("Scheduled expiration timestamp: {}.", expires_str),
                            "recommended_action": "Renew server billing or drain workloads before expiration.",
                            "suggested_tool": "server.get",
                            "suggested_arguments": { "server_id": server_id }
                        }));
                    }
                }
            }
        }

        if let Ok(services_val) = self.exec_service_list(auth, &json!({ "limit": 200 })).await {
            if let Some(services) = services_val["services"].as_array() {
                for svc in services {
                    if !svc["enabled"].as_bool().unwrap_or(true) {
                        continue;
                    }
                    let status = svc["last_status"].as_str().unwrap_or("unknown");
                    if matches!(status, "failure" | "down" | "error" | "timeout") {
                        let svc_id = svc["id"].as_str().unwrap_or_default();
                        let svc_name = svc["name"].as_str().unwrap_or_default();
                        let target = svc["target"].as_str().unwrap_or_default();
                        anomalies.push(json!({
                            "severity": "critical",
                            "category": "service_sla",
                            "resource_type": "service",
                            "resource_id": svc_id,
                            "resource_name": svc_name,
                            "title": format!("Service probe '{}' is {}", svc_name, status),
                            "detail": format!("Target '{}' failed its latest health check.", target),
                            "recommended_action": "Inspect probe check history and verify upstream service availability.",
                            "suggested_tool": "service.history",
                            "suggested_arguments": { "service_id": svc_id, "limit": 20 }
                        }));
                    }
                }
            }
        }

        anomalies.sort_by_key(|a| match a["severity"].as_str() {
            Some("critical") => 0,
            Some("warning") => 1,
            _ => 2,
        });

        let critical_count = anomalies
            .iter()
            .filter(|a| a["severity"].as_str() == Some("critical"))
            .count();
        let warning_count = anomalies
            .iter()
            .filter(|a| a["severity"].as_str() == Some("warning"))
            .count();

        let penalty = (critical_count * 20 + warning_count * 8).min(100);
        let health_score = 100usize.saturating_sub(penalty);
        let overall_status = if health_score >= 85 && critical_count == 0 {
            "healthy"
        } else if health_score >= 60 {
            "degraded"
        } else {
            "critical"
        };

        let summary = if anomalies.is_empty() {
            format!(
                "All {} active server(s) and monitored services are operating normally within thresholds.",
                active_nodes
            )
        } else {
            format!(
                "Detected {} anomaly/anomalies ({} critical, {} warning) across {} active server(s) ({} online).",
                anomalies.len(),
                critical_count,
                warning_count,
                active_nodes,
                online_count
            )
        };

        let mut recommendations = Vec::new();
        if critical_count > 0 {
            recommendations.push(
                "Prioritize critical availability and service SLA anomalies immediately using `server.inspect` or `service.history`."
                    .to_string(),
            );
        }
        if warning_count > 0 {
            recommendations.push(
                "Review resource pressure warnings with `server.metrics` (range='6h') to distinguish transient spikes from sustained saturation."
                    .to_string(),
            );
        }
        if recommendations.is_empty() {
            recommendations.push(
                "Cluster is healthy. Consider running `sre.capacity_and_security` to audit long-term capacity and NAT/DDNS posture."
                    .to_string(),
            );
        }

        Ok(json!({
            "health_score": health_score,
            "overall_status": overall_status,
            "summary": summary,
            "thresholds": {
                "cpu_percent": cpu_threshold,
                "memory_percent": memory_threshold,
                "disk_percent": disk_threshold,
            },
            "counts": {
                "active_servers": active_nodes,
                "online_servers": online_count,
                "critical_anomalies": critical_count,
                "warning_anomalies": warning_count,
                "total_anomalies": anomalies.len(),
            },
            "anomalies": anomalies,
            "recommendations": recommendations,
            "timestamp": Utc::now().to_rfc3339(),
        }))
    }

    /// Execute server.list
    async fn exec_server_list(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let limit = args["limit"].as_i64().unwrap_or(50).clamp(1, 200);
        let offset = args["offset"].as_i64().unwrap_or(0).max(0);
        let status_filter = args["status"]
            .as_str()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty() && s != "all");
        let search_filter = args["search"]
            .as_str()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());

        if status_filter.is_none() && search_filter.is_none() {
            let repo = AgentRepository::new(self.db.clone());
            let rows = if let Some(server_ids) = auth.server_ids.as_deref() {
                let owner_filter = if auth.role.is_admin() {
                    None
                } else {
                    Some(auth.user_id)
                };
                let (rows, _) = repo
                    .list_with_state_by_server_ids(owner_filter, server_ids, limit, offset)
                    .await?;
                rows
            } else if auth.role.is_admin() {
                let (rows, _) = repo.list_with_state(limit, offset).await?;
                rows
            } else {
                let (rows, _) = repo
                    .list_with_state_by_owner(auth.user_id, limit, offset)
                    .await?;
                rows
            };

            let mut servers = Vec::new();
            for row in rows {
                if agent_visible_to_auth(auth, &row.agent) {
                    servers.push(self.build_enriched_server_json(&row).await);
                }
            }

            return Ok(json!({
                "servers": servers,
                "total": servers.len(),
                "limit": limit,
                "offset": offset,
            }));
        }

        let all_rows = self.load_all_visible_agents_with_state(auth, 500).await?;
        let mut filtered = Vec::new();
        for row in all_rows {
            let item = self.build_enriched_server_json(&row).await;
            if let Some(ref st) = status_filter {
                let is_online = item["online"].as_bool().unwrap_or(false);
                if st == "online" && !is_online {
                    continue;
                }
                if st == "offline" && is_online {
                    continue;
                }
            }
            if let Some(ref q) = search_filter {
                if !server_json_matches_query(&item, q) {
                    continue;
                }
            }
            filtered.push(item);
        }

        let paged: Vec<Value> = filtered
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();

        Ok(json!({
            "servers": paged,
            "total": paged.len(),
            "limit": limit,
            "offset": offset,
        }))
    }

    /// Execute server.get
    async fn exec_server_get(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        let row = self
            .ensure_agent_with_state_visible(auth, server_id)
            .await?;
        let mut value = self.build_enriched_server_json(&row).await;
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "updated_at".to_string(),
                json!(row.agent.updated_at.to_rfc3339()),
            );
        }
        Ok(value)
    }

    /// Execute server.resolve
    async fn exec_server_resolve(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let query = args["query"]
            .as_str()
            .map(str::trim)
            .context("Missing query")?;
        let q_lower = query.to_ascii_lowercase();
        let rows = self.load_all_visible_agents_with_state(auth, 500).await?;

        let mut scored: Vec<(i32, Value)> = Vec::new();
        for row in rows {
            let item = self.build_enriched_server_json(&row).await;
            let sid = item["server_id"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let name = item["name"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();

            let (score, match_type) = if q_lower.is_empty() || q_lower == "*" || q_lower == "all" {
                (50, "all")
            } else if sid == q_lower || name == q_lower {
                (100, "exact")
            } else if name.starts_with(&q_lower) || sid.starts_with(&q_lower) {
                (85, "prefix")
            } else if name.contains(&q_lower) {
                (70, "name_substring")
            } else if server_json_matches_query(&item, &q_lower) {
                (50, "metadata")
            } else {
                (0, "")
            };

            if score > 0 {
                scored.push((
                    score,
                    json!({
                        "server_id": item["server_id"],
                        "name": item["name"],
                        "online": item["online"],
                        "status": item["status"],
                        "region": item["region"],
                        "country": item["country"],
                        "provider": item["provider"],
                        "tags": item["tags"],
                        "cpu_percent": item["cpu_percent"],
                        "memory_percent": item["memory_percent"],
                        "score": score,
                        "match_type": match_type,
                    }),
                ));
            }
        }

        scored.sort_by_key(|b| std::cmp::Reverse(b.0));
        let matches: Vec<Value> = scored.into_iter().map(|(_, v)| v).collect();
        let best_match = matches.first().cloned().unwrap_or(Value::Null);

        Ok(json!({
            "query": query,
            "total": matches.len(),
            "best_match": best_match,
            "matches": matches,
        }))
    }

    /// Execute server.metrics
    async fn exec_server_metrics(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        let row = self
            .ensure_agent_with_state_visible(auth, server_id)
            .await?;
        let canonical_id = row.agent.id.0.to_string();
        let range_str = args["range"].as_str().unwrap_or("1h").trim();
        let window = match range_str {
            "15m" => Duration::minutes(15),
            "1h" => Duration::hours(1),
            "6h" => Duration::hours(6),
            "24h" | "1d" => Duration::hours(24),
            _ => Duration::hours(1),
        };
        let cutoff = Utc::now() - window;

        let mut samples: Vec<(DateTime<Utc>, Value)> = Vec::new();
        if let Some(ref store) = self.metrics {
            if let Ok(series) =
                store.query(xlstatus_tsdb::AgentId(row.agent.id.0), QueryRange::Day1)
            {
                for s in series.samples {
                    if s.sample_at >= cutoff {
                        samples.push((s.sample_at, s.fields_json));
                    }
                }
            }
        }

        if samples.is_empty() {
            if let Some(parsed) =
                parse_agent_telemetry_json::<Value>(row.last_state_json.as_deref())
            {
                let ts = row.agent.last_seen_at.unwrap_or_else(Utc::now);
                samples.push((ts, parsed));
            }
        }

        let mut cpu_vals = Vec::new();
        let mut mem_pct_vals = Vec::new();
        let mut load_vals = Vec::new();
        let mut series_points = Vec::new();

        let step = (samples.len() / 60).max(1);
        for (idx, (ts, val)) in samples.iter().enumerate() {
            let cpu = val.get("cpu_percent").and_then(Value::as_f64);
            let mem_used = val.get("memory_used").and_then(Value::as_i64);
            let mem_tot = val.get("memory_total").and_then(Value::as_i64);
            let mem_pct = match (mem_used, mem_tot) {
                (Some(u), Some(t)) if t > 0 => Some((u as f64 / t as f64) * 100.0),
                _ => None,
            };
            let load_1 = val.get("load_1").and_then(Value::as_f64);

            if let Some(c) = cpu {
                cpu_vals.push(c);
            }
            if let Some(m) = mem_pct {
                mem_pct_vals.push(m);
            }
            if let Some(l) = load_1 {
                load_vals.push(l);
            }

            if idx % step == 0 || idx + 1 == samples.len() {
                series_points.push(json!({
                    "sample_at": ts.to_rfc3339(),
                    "cpu_percent": cpu,
                    "memory_used": mem_used,
                    "memory_total": mem_tot,
                    "memory_percent": mem_pct.map(|v| (v * 10.0).round() / 10.0),
                    "load_1": load_1,
                }));
            }
        }

        Ok(json!({
            "server_id": canonical_id,
            "name": row.agent.name,
            "range": range_str,
            "sample_count": samples.len(),
            "aggregates": {
                "cpu_percent": compute_f64_stats(&cpu_vals),
                "memory_percent": compute_f64_stats(&mem_pct_vals),
                "load_1": compute_f64_stats(&load_vals),
            },
            "series": series_points,
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
        let rx = response_registry.register(run_id.clone(), agent_id).await;
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

    /// Execute server.batch_exec
    async fn exec_server_batch_exec(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let command = validate_exec_command(args["command"].as_str().context("Missing command")?)?
            .to_string();
        let timeout = normalize_exec_timeout(args["timeout"].as_i64().or(Some(20)));
        let target_mode = args["target"].as_str().unwrap_or("selected");

        let mut target_agents = Vec::new();
        if target_mode == "all_online" {
            let rows = self.load_all_visible_agents_with_state(auth, 200).await?;
            for row in rows {
                if row.agent.revoked_at.is_some() {
                    continue;
                }
                if self.is_agent_online(&row.agent).await {
                    target_agents.push(row.agent);
                    if target_agents.len() >= MCP_BATCH_EXEC_MAX_SERVERS {
                        break;
                    }
                }
            }
        } else {
            let ids = args["server_ids"]
                .as_array()
                .context("Missing server_ids array (or set target='all_online')")?;
            if ids.is_empty() {
                anyhow::bail!("server_ids must not be empty");
            }
            if ids.len() > MCP_BATCH_EXEC_MAX_SERVERS {
                anyhow::bail!(
                    "server.batch_exec supports at most {} servers per call",
                    MCP_BATCH_EXEC_MAX_SERVERS
                );
            }
            for id_val in ids {
                let sid = id_val
                    .as_str()
                    .context("Each item in server_ids must be a canonical UUID string")?;
                let agent = self.ensure_agent_active(auth, sid).await?;
                target_agents.push(agent);
            }
        }

        let mut results = Vec::with_capacity(target_agents.len());
        let mut succeeded = 0usize;
        let mut failed = 0usize;

        for agent in target_agents {
            let sid = agent.id.0.to_string();
            match self.dispatch_shell(&sid, &command, timeout).await {
                Ok(res) => {
                    let ok = res["status"].as_str() == Some("success")
                        && res["exit_code"].as_i64() == Some(0);
                    if ok {
                        succeeded += 1;
                    } else {
                        failed += 1;
                    }
                    results.push(json!({
                        "server_id": sid,
                        "name": agent.name,
                        "success": ok,
                        "status": res["status"],
                        "exit_code": res["exit_code"],
                        "stdout": res["stdout"],
                        "stderr": res["stderr"],
                        "error": res["error"],
                    }));
                }
                Err(err) => {
                    failed += 1;
                    results.push(json!({
                        "server_id": sid,
                        "name": agent.name,
                        "success": false,
                        "status": "error",
                        "exit_code": Value::Null,
                        "stdout": "",
                        "stderr": "",
                        "error": err.to_string(),
                    }));
                }
            }
        }

        Ok(json!({
            "command": command,
            "timeout_seconds": timeout,
            "targeted": results.len(),
            "succeeded": succeeded,
            "failed": failed,
            "results": results,
        }))
    }

    /// Execute server.inspect (Predefined read-only SRE diagnostic playbooks)
    async fn exec_server_inspect(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let server_id = args["server_id"].as_str().context("Missing server_id")?;
        let agent = self.ensure_agent_active(auth, server_id).await?;
        let canonical_id = agent.id.0.to_string();
        let playbook = args["playbook"].as_str().unwrap_or("system_vital").trim();

        let (title, command) = match playbook {
            "system_vital" => (
                "System Vitals, Load & Memory Snapshot",
                "uname -a; echo '=== UPTIME ==='; uptime; echo '=== MEMORY ==='; (free -h 2>/dev/null || vm_stat 2>/dev/null || true); echo '=== FILESYSTEMS ==='; df -h",
            ),
            "top_processes" => (
                "Top CPU & Memory Processes",
                "ps aux 2>/dev/null | head -n 25 || ps -ef | head -n 25",
            ),
            "disk_and_inodes" => (
                "Disk Block & Inode Utilization",
                "echo '=== BLOCK USAGE ==='; df -h; echo '=== INODE USAGE ==='; (df -i 2>/dev/null || true)",
            ),
            "network_sockets" => (
                "Listening Network Sockets & Active Connections",
                "(ss -tulnp 2>/dev/null || netstat -an 2>/dev/null | head -n 45 || true)",
            ),
            "docker_containers" => (
                "Docker Containers & Runtime Status",
                "(docker ps -a --format 'table {{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}' 2>/dev/null || echo 'Docker daemon not installed or not accessible')",
            ),
            "system_logs" => (
                "Recent System & Kernel Logs",
                "(journalctl -n 40 --no-pager 2>/dev/null || tail -n 40 /var/log/syslog 2>/dev/null || tail -n 40 /var/log/messages 2>/dev/null || dmesg 2>/dev/null | tail -n 40 || echo 'No system logs accessible')",
            ),
            _ => anyhow::bail!(
                "Unsupported diagnostic playbook '{}'. Valid playbooks: system_vital, top_processes, disk_and_inodes, network_sockets, docker_containers, system_logs",
                playbook
            ),
        };

        let result = self.dispatch_shell(&canonical_id, command, 25).await?;
        Ok(json!({
            "server_id": canonical_id,
            "server_name": agent.name,
            "playbook": playbook,
            "title": title,
            "command": command,
            "status": result["status"],
            "exit_code": result["exit_code"],
            "stdout": result["stdout"],
            "stderr": result["stderr"],
            "error": result["error"],
        }))
    }

    /// Execute service.list
    async fn exec_service_list(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let limit = args["limit"].as_i64().unwrap_or(50).clamp(1, 200);
        let owner_id = auth.user_id.0.to_string();
        let restrict_by_visible_servers = !auth.role.is_admin() || auth.server_ids.is_some();
        let visible_server_ids: std::collections::HashSet<String> = if restrict_by_visible_servers {
            self.load_all_visible_agents_with_state(auth, 500)
                .await?
                .into_iter()
                .map(|r| r.agent.id.0.to_string())
                .collect()
        } else {
            std::collections::HashSet::new()
        };
        let query_limit = if restrict_by_visible_servers {
            200
        } else {
            limit
        };
        let mut services = Vec::new();

        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = if auth.role.is_admin() {
                    sqlx::query(
                        r#"
                        SELECT s.id, s.owner_user_id, s.name, s.type, s.target, s.interval_seconds,
                               s.timeout_seconds, s.enabled, s.server_id, COALESCE(s.cover_mode, 'local') AS cover_mode,
                               r.status AS last_status, r.delay_ms AS last_delay_ms, r.status_code AS last_status_code,
                               r.error AS last_error, r.created_at AS last_check_at, r.cert_not_after AS cert_not_after
                        FROM services s
                        LEFT JOIN service_results r ON r.id = (
                            SELECT sr.id FROM service_results sr
                            WHERE sr.service_id = s.id
                            ORDER BY sr.created_at DESC
                            LIMIT 1
                        )
                        ORDER BY s.created_at DESC
                        LIMIT ?
                        "#,
                    )
                    .bind(query_limit)
                    .fetch_all(pool)
                    .await?
                } else {
                    sqlx::query(
                        r#"
                        SELECT s.id, s.owner_user_id, s.name, s.type, s.target, s.interval_seconds,
                               s.timeout_seconds, s.enabled, s.server_id, COALESCE(s.cover_mode, 'local') AS cover_mode,
                               r.status AS last_status, r.delay_ms AS last_delay_ms, r.status_code AS last_status_code,
                               r.error AS last_error, r.created_at AS last_check_at, r.cert_not_after AS cert_not_after
                        FROM services s
                        LEFT JOIN service_results r ON r.id = (
                            SELECT sr.id FROM service_results sr
                            WHERE sr.service_id = s.id
                            ORDER BY sr.created_at DESC
                            LIMIT 1
                        )
                        WHERE s.owner_user_id = ?
                        ORDER BY s.created_at DESC
                        LIMIT ?
                        "#,
                    )
                    .bind(&owner_id)
                    .bind(query_limit)
                    .fetch_all(pool)
                    .await?
                };

                for row in rows {
                    let server_id: Option<String> = row
                        .try_get::<Option<String>, _>("server_id")
                        .ok()
                        .flatten()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    if restrict_by_visible_servers {
                        match server_id.as_ref() {
                            Some(sid) if visible_server_ids.contains(sid) => {}
                            None if auth.server_ids.is_none() && !auth.role.is_admin() => {}
                            _ => continue,
                        }
                    }
                    let enabled = row.try_get::<i64, _>("enabled").unwrap_or(1) != 0;
                    services.push(json!({
                        "id": row.try_get::<String, _>("id").unwrap_or_default(),
                        "name": row.try_get::<String, _>("name").unwrap_or_default(),
                        "service_type": row.try_get::<String, _>("type").unwrap_or_default(),
                        "target": row.try_get::<String, _>("target").unwrap_or_default(),
                        "interval_seconds": row.try_get::<i32, _>("interval_seconds").unwrap_or(60),
                        "timeout_seconds": row.try_get::<i32, _>("timeout_seconds").unwrap_or(5),
                        "enabled": enabled,
                        "server_id": server_id,
                        "cover_mode": row.try_get::<String, _>("cover_mode").unwrap_or_else(|_| "local".into()),
                        "last_status": row.try_get::<Option<String>, _>("last_status").ok().flatten(),
                        "last_delay_ms": row.try_get::<Option<i32>, _>("last_delay_ms").ok().flatten(),
                        "last_status_code": row.try_get::<Option<i32>, _>("last_status_code").ok().flatten(),
                        "last_error": row.try_get::<Option<String>, _>("last_error").ok().flatten(),
                        "last_check_at": row.try_get::<Option<String>, _>("last_check_at").ok().flatten(),
                        "cert_not_after": row.try_get::<Option<String>, _>("cert_not_after").ok().flatten(),
                    }));
                    if services.len() >= limit as usize {
                        break;
                    }
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = if auth.role.is_admin() {
                    sqlx::query(
                        r#"
                        SELECT s.id::text AS id, s.name, s.type, s.target, s.interval_seconds,
                               s.timeout_seconds, s.enabled, s.server_id::text AS server_id,
                               COALESCE(s.cover_mode, 'local') AS cover_mode,
                               r.status AS last_status, r.delay_ms AS last_delay_ms, r.status_code AS last_status_code,
                               r.error AS last_error, r.created_at::text AS last_check_at, r.cert_not_after::text AS cert_not_after
                        FROM services s
                        LEFT JOIN service_results r ON r.id = (
                            SELECT sr.id FROM service_results sr
                            WHERE sr.service_id = s.id
                            ORDER BY sr.created_at DESC
                            LIMIT 1
                        )
                        ORDER BY s.created_at DESC
                        LIMIT $1
                        "#,
                    )
                    .bind(query_limit)
                    .fetch_all(pool)
                    .await?
                } else {
                    sqlx::query(
                        r#"
                        SELECT s.id::text AS id, s.name, s.type, s.target, s.interval_seconds,
                               s.timeout_seconds, s.enabled, s.server_id::text AS server_id,
                               COALESCE(s.cover_mode, 'local') AS cover_mode,
                               r.status AS last_status, r.delay_ms AS last_delay_ms, r.status_code AS last_status_code,
                               r.error AS last_error, r.created_at::text AS last_check_at, r.cert_not_after::text AS cert_not_after
                        FROM services s
                        LEFT JOIN service_results r ON r.id = (
                            SELECT sr.id FROM service_results sr
                            WHERE sr.service_id = s.id
                            ORDER BY sr.created_at DESC
                            LIMIT 1
                        )
                        WHERE s.owner_user_id = $1
                        ORDER BY s.created_at DESC
                        LIMIT $2
                        "#,
                    )
                    .bind(auth.user_id.0)
                    .bind(query_limit)
                    .fetch_all(pool)
                    .await?
                };

                for row in rows {
                    let server_id: Option<String> = row
                        .try_get::<Option<String>, _>("server_id")
                        .ok()
                        .flatten()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    if restrict_by_visible_servers {
                        match server_id.as_ref() {
                            Some(sid) if visible_server_ids.contains(sid) => {}
                            None if auth.server_ids.is_none() && !auth.role.is_admin() => {}
                            _ => continue,
                        }
                    }
                    services.push(json!({
                        "id": row.try_get::<String, _>("id").unwrap_or_default(),
                        "name": row.try_get::<String, _>("name").unwrap_or_default(),
                        "service_type": row.try_get::<String, _>("type").unwrap_or_default(),
                        "target": row.try_get::<String, _>("target").unwrap_or_default(),
                        "interval_seconds": row.try_get::<i32, _>("interval_seconds").unwrap_or(60),
                        "timeout_seconds": row.try_get::<i32, _>("timeout_seconds").unwrap_or(5),
                        "enabled": row.try_get::<bool, _>("enabled").unwrap_or(true),
                        "server_id": server_id,
                        "cover_mode": row.try_get::<String, _>("cover_mode").unwrap_or_else(|_| "local".into()),
                        "last_status": row.try_get::<Option<String>, _>("last_status").ok().flatten(),
                        "last_delay_ms": row.try_get::<Option<i32>, _>("last_delay_ms").ok().flatten(),
                        "last_status_code": row.try_get::<Option<i32>, _>("last_status_code").ok().flatten(),
                        "last_error": row.try_get::<Option<String>, _>("last_error").ok().flatten(),
                        "last_check_at": row.try_get::<Option<String>, _>("last_check_at").ok().flatten(),
                        "cert_not_after": row.try_get::<Option<String>, _>("cert_not_after").ok().flatten(),
                    }));
                    if services.len() >= limit as usize {
                        break;
                    }
                }
            }
        }

        Ok(json!({
            "services": services,
            "total": services.len(),
        }))
    }

    /// Execute service.history
    async fn exec_service_history(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let service_id = args["service_id"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .context("Missing service_id")?;
        let _parsed_uuid =
            uuid::Uuid::parse_str(service_id).context("service_id must be a valid UUID")?;
        let limit = args["limit"].as_i64().unwrap_or(30).clamp(1, 200);

        let visible_services = self
            .exec_service_list(auth, &json!({ "limit": 200 }))
            .await?;
        let svc_meta = visible_services["services"]
            .as_array()
            .and_then(|arr| arr.iter().find(|s| s["id"].as_str() == Some(service_id)))
            .cloned()
            .context("Service not found or not accessible")?;

        let restrict_by_visible_servers = !auth.role.is_admin() || auth.server_ids.is_some();
        let visible_server_ids: std::collections::HashSet<String> = if restrict_by_visible_servers {
            self.load_all_visible_agents_with_state(auth, 500)
                .await?
                .into_iter()
                .map(|r| r.agent.id.0.to_string())
                .collect()
        } else {
            std::collections::HashSet::new()
        };

        let mut results = Vec::new();
        match &self.db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(
                    r#"
                    SELECT id, service_id, server_id, status, delay_ms, status_code, error, cert_not_after, created_at
                    FROM service_results
                    WHERE service_id = ?
                    ORDER BY created_at DESC
                    LIMIT ?
                    "#,
                )
                .bind(service_id)
                .bind(limit)
                .fetch_all(pool)
                .await?;
                for row in rows {
                    let row_server_id = row
                        .try_get::<Option<String>, _>("server_id")
                        .ok()
                        .flatten()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    if restrict_by_visible_servers {
                        if let Some(ref sid) = row_server_id {
                            if !visible_server_ids.contains(sid) {
                                continue;
                            }
                        } else if auth.server_ids.is_some() {
                            continue;
                        }
                    }
                    results.push(json!({
                        "id": row.try_get::<String, _>("id").unwrap_or_default(),
                        "service_id": row.try_get::<String, _>("service_id").unwrap_or_default(),
                        "server_id": row_server_id,
                        "status": row.try_get::<String, _>("status").unwrap_or_default(),
                        "delay_ms": row.try_get::<Option<i32>, _>("delay_ms").ok().flatten(),
                        "status_code": row.try_get::<Option<i32>, _>("status_code").ok().flatten(),
                        "error": row.try_get::<Option<String>, _>("error").ok().flatten(),
                        "cert_not_after": row.try_get::<Option<String>, _>("cert_not_after").ok().flatten(),
                        "created_at": row.try_get::<String, _>("created_at").unwrap_or_default(),
                    }));
                }
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let sid = uuid::Uuid::parse_str(service_id)?;
                let rows = sqlx::query(
                    r#"
                    SELECT id::text AS id, service_id::text AS service_id, server_id::text AS server_id,
                           status, delay_ms, status_code, error, cert_not_after::text AS cert_not_after,
                           created_at::text AS created_at
                    FROM service_results
                    WHERE service_id = $1
                    ORDER BY created_at DESC
                    LIMIT $2
                    "#,
                )
                .bind(sid)
                .bind(limit)
                .fetch_all(pool)
                .await?;
                for row in rows {
                    let row_server_id = row
                        .try_get::<Option<String>, _>("server_id")
                        .ok()
                        .flatten()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    if restrict_by_visible_servers {
                        if let Some(ref sid) = row_server_id {
                            if !visible_server_ids.contains(sid) {
                                continue;
                            }
                        } else if auth.server_ids.is_some() {
                            continue;
                        }
                    }
                    results.push(json!({
                        "id": row.try_get::<String, _>("id").unwrap_or_default(),
                        "service_id": row.try_get::<String, _>("service_id").unwrap_or_default(),
                        "server_id": row_server_id,
                        "status": row.try_get::<String, _>("status").unwrap_or_default(),
                        "delay_ms": row.try_get::<Option<i32>, _>("delay_ms").ok().flatten(),
                        "status_code": row.try_get::<Option<i32>, _>("status_code").ok().flatten(),
                        "error": row.try_get::<Option<String>, _>("error").ok().flatten(),
                        "cert_not_after": row.try_get::<Option<String>, _>("cert_not_after").ok().flatten(),
                        "created_at": row.try_get::<String, _>("created_at").unwrap_or_default(),
                    }));
                }
            }
        }

        let total = results.len();
        let success_count = results
            .iter()
            .filter(|r| r["status"].as_str() == Some("success"))
            .count();
        let uptime_percent = if total > 0 {
            Some(((success_count as f64 / total as f64) * 1000.0).round() / 10.0)
        } else {
            None
        };
        let latencies: Vec<f64> = results
            .iter()
            .filter_map(|r| r["delay_ms"].as_i64().map(|v| v as f64))
            .collect();

        Ok(json!({
            "service": svc_meta,
            "total": total,
            "uptime_percent": uptime_percent,
            "latency_ms": compute_f64_stats(&latencies),
            "results": results,
        }))
    }

    /// Execute alert.events
    async fn exec_alert_events(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let limit = args["limit"].as_i64().unwrap_or(25).clamp(1, 200);
        let kind_filter = args["kind"]
            .as_str()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty() && s != "all");
        let repo = AlertEventRepository::new(self.db.clone());
        let owner = auth.user_id.0.to_string();
        let restrict_by_visibility = !auth.role.is_admin() || auth.server_ids.is_some();

        let visible_server_ids: std::collections::HashSet<String> = if restrict_by_visibility {
            self.load_all_visible_agents_with_state(auth, 500)
                .await?
                .into_iter()
                .map(|r| r.agent.id.0.to_string())
                .collect()
        } else {
            std::collections::HashSet::new()
        };

        let visible_service_ids: std::collections::HashSet<String> = if restrict_by_visibility {
            self.exec_service_list(auth, &json!({ "limit": 200 }))
                .await?["services"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s["id"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            std::collections::HashSet::new()
        };

        let rows = if auth.role.is_admin() {
            if let Some(server_ids) = auth.server_ids.as_ref() {
                repo.list_recent_for_server_ids(server_ids, limit).await?
            } else {
                repo.list_recent(limit).await?
            }
        } else if let Some(server_ids) = auth.server_ids.as_ref() {
            repo.list_recent_for_owner_server_ids(&owner, server_ids, limit)
                .await?
        } else {
            repo.list_recent_for_owner(&owner, limit).await?
        };

        let mut events = Vec::new();
        for e in rows {
            if restrict_by_visibility {
                let agent_ok = e
                    .agent_id
                    .as_ref()
                    .filter(|s| !s.is_empty())
                    .is_some_and(|aid| visible_server_ids.contains(aid));
                let service_ok = e.agent_id.is_none()
                    && e.service_id
                        .as_ref()
                        .filter(|s| !s.is_empty())
                        .is_some_and(|sid| visible_service_ids.contains(sid));
                if !agent_ok && !service_ok {
                    continue;
                }
            }
            if let Some(ref kf) = kind_filter {
                if !e.kind.to_ascii_lowercase().contains(kf) {
                    continue;
                }
            }
            let payload = serde_json::from_str::<Value>(&e.payload_json).unwrap_or(Value::Null);
            events.push(json!({
                "id": e.id,
                "rule_id": e.rule_id,
                "agent_id": e.agent_id,
                "service_id": e.service_id,
                "kind": e.kind,
                "payload": payload,
                "fired_at": e.fired_at.to_rfc3339(),
            }));
        }

        Ok(json!({
            "events": events,
            "total": events.len(),
        }))
    }

    /// Execute alert.rules
    async fn exec_alert_rules(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let limit = args["limit"].as_i64().unwrap_or(50).clamp(1, 200);
        let repo = AlertRepository::new(self.db.clone());
        let owner = auth.user_id.0.to_string();
        let restrict_by_visibility = !auth.role.is_admin() || auth.server_ids.is_some();

        let visible_server_ids: std::collections::HashSet<String> = if restrict_by_visibility {
            self.load_all_visible_agents_with_state(auth, 500)
                .await?
                .into_iter()
                .map(|r| r.agent.id.0.to_string())
                .collect()
        } else {
            std::collections::HashSet::new()
        };

        let visible_service_ids: std::collections::HashSet<String> = if restrict_by_visibility {
            self.exec_service_list(auth, &json!({ "limit": 200 }))
                .await?["services"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s["id"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            std::collections::HashSet::new()
        };

        let fetch_limit = if restrict_by_visibility { 200 } else { limit };
        let rows = if auth.role.is_admin() {
            repo.list_page(fetch_limit, 0).await?
        } else {
            repo.list_by_owner_page(&owner, fetch_limit, 0).await?
        };

        let rules: Vec<Value> = rows
            .into_iter()
            .filter(|r| {
                if !restrict_by_visibility {
                    return true;
                }
                if auth.server_ids.is_some() && r.conditions.is_empty() {
                    return false;
                }
                r.conditions.iter().all(|cond| match cond {
                    crate::alerts::engine::AlertCondition::ServiceDown { service_id, .. }
                    | crate::alerts::engine::AlertCondition::ServiceLatency {
                        service_id, ..
                    }
                    | crate::alerts::engine::AlertCondition::CertificateExpiry {
                        service_id, ..
                    } => {
                        let sid = service_id.trim();
                        if sid.is_empty() || sid == "*" {
                            auth.server_ids.is_none()
                        } else {
                            visible_service_ids.contains(sid)
                        }
                    }
                    crate::alerts::engine::AlertCondition::ServerExpiry { agent_id, .. }
                    | crate::alerts::engine::AlertCondition::ServerTrafficQuota {
                        agent_id, ..
                    }
                    | crate::alerts::engine::AlertCondition::ServerOffline { agent_id, .. }
                    | crate::alerts::engine::AlertCondition::ServerResource { agent_id, .. } => {
                        let aid = agent_id.trim();
                        if aid.is_empty() || aid == "*" {
                            auth.server_ids.is_none()
                        } else {
                            visible_server_ids.contains(aid)
                        }
                    }
                })
            })
            .take(limit as usize)
            .map(|r| {
                json!({
                    "id": r.id,
                    "name": r.name,
                    "enabled": r.enabled,
                    "trigger": r.trigger_mode.as_db(),
                    "conditions": r.conditions,
                    "notification_group_id": r.notification_group_id,
                    "failure_task_ids": r.failure_task_ids,
                    "recovery_task_ids": r.recovery_task_ids,
                    "created_at": r.created_at.to_rfc3339(),
                    "updated_at": r.updated_at.to_rfc3339(),
                })
            })
            .collect();

        Ok(json!({
            "rules": rules,
            "total": rules.len(),
        }))
    }

    /// Execute task.list
    async fn exec_task_list(&self, auth: &AuthSession, args: &Value) -> Result<Value> {
        let limit = args["limit"].as_i64().unwrap_or(25).clamp(1, 100);
        let owner = auth.user_id.0.to_string();
        let restrict_by_visibility = !auth.role.is_admin() || auth.server_ids.is_some();
        let visible_server_ids: std::collections::HashSet<String> = if restrict_by_visibility {
            self.load_all_visible_agents_with_state(auth, 500)
                .await?
                .into_iter()
                .map(|r| r.agent.id.0.to_string())
                .collect()
        } else {
            std::collections::HashSet::new()
        };

        let fetch_limit = if restrict_by_visibility { 100 } else { limit };
        let tasks = TaskRepository::list_by_user(&self.db, &owner, fetch_limit, 0).await?;
        let items: Vec<Value> = tasks
            .into_iter()
            .filter(|t| {
                if !restrict_by_visibility {
                    return true;
                }
                let Ok(selector) = serde_json::from_str::<xlstatus_shared::tasks::ServerSelector>(
                    &t.server_selector_json,
                ) else {
                    return false;
                };
                if auth.server_ids.is_some()
                    && (selector.source_server
                        || !selector.group_ids.is_empty()
                        || !selector.tag_names.is_empty()
                        || !selector.tags.is_empty()
                        || selector.server_ids.is_empty())
                {
                    return false;
                }
                selector
                    .server_ids
                    .iter()
                    .chain(selector.exclude_server_ids.iter())
                    .all(|sid| visible_server_ids.contains(sid))
            })
            .take(limit as usize)
            .map(|t| {
                json!({
                    "id": t.id,
                    "name": t.name,
                    "task_type": t.task_type,
                    "schedule": t.schedule,
                    "command": t.command,
                    "cover_mode": t.cover_mode,
                    "enabled": t.enabled,
                    "last_executed_at": t.last_executed_at,
                    "last_result": t.last_result,
                    "created_at": t.created_at,
                })
            })
            .collect();

        Ok(json!({
            "tasks": items,
            "total": items.len(),
        }))
    }

    /// Execute network.topology
    async fn exec_network_topology(&self, auth: &AuthSession) -> Result<Value> {
        let visible_rows = self.load_all_visible_agents_with_state(auth, 500).await?;
        let visible_ids: std::collections::HashSet<String> = visible_rows
            .iter()
            .map(|r| r.agent.id.0.to_string())
            .collect();
        let name_by_id: std::collections::HashMap<String, String> = visible_rows
            .iter()
            .map(|r| (r.agent.id.0.to_string(), r.agent.name.clone()))
            .collect();

        let nat_rows = NatMappingRepository::list_enabled(&self.db).await?;
        let nat_mappings: Vec<Value> = nat_rows
            .into_iter()
            .filter(|m| visible_ids.contains(&m.agent_id))
            .map(|m| {
                json!({
                    "id": m.id,
                    "server_id": m.agent_id,
                    "server_name": name_by_id.get(&m.agent_id).cloned(),
                    "local_host": m.local_host,
                    "local_port": m.local_port,
                    "public_port": m.public_port,
                    "protocol": m.protocol.as_str(),
                    "enabled": m.enabled,
                    "description": m.description,
                    "allowed_sources": m.allowed_sources,
                })
            })
            .collect();

        let ddns_rows = DdnsConfigRepository::list_enabled(&self.db).await?;
        let ddns_records: Vec<Value> = ddns_rows
            .into_iter()
            .filter(|d| {
                let owner_ok =
                    auth.role.is_admin() || d.owner_user_id == auth.user_id.0.to_string();
                let agent_ok = d
                    .agent_id
                    .as_ref()
                    .map(|aid| visible_ids.contains(aid))
                    .unwrap_or(auth.server_ids.is_none());
                owner_ok && agent_ok
            })
            .map(|d| {
                json!({
                    "id": d.id,
                    "name": d.name,
                    "provider": d.provider,
                    "domain": d.domain,
                    "server_id": d.agent_id,
                    "server_name": d.agent_id.as_ref().and_then(|id| name_by_id.get(id).cloned()),
                    "current_ip": d.current_ip,
                    "last_applied_ip": d.last_applied_ip,
                    "last_applied_at": d.last_applied_at,
                    "enabled": d.enabled,
                })
            })
            .collect();

        Ok(json!({
            "nat_mappings": nat_mappings,
            "nat_total": nat_mappings.len(),
            "ddns_records": ddns_records,
            "ddns_total": ddns_records.len(),
            "timestamp": Utc::now().to_rfc3339(),
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

    async fn ensure_agent_with_state_visible(
        &self,
        auth: &AuthSession,
        server_id: &str,
    ) -> Result<AgentWithState> {
        let agent_id = parse_mcp_server_id(server_id)?;
        let row = AgentRepository::new(self.db.clone())
            .find_by_id_with_state(agent_id)
            .await?
            .context("Server not found")?;
        if agent_visible_to_auth(auth, &row.agent) {
            Ok(row)
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

    async fn is_agent_online(&self, agent: &crate::db::Agent) -> bool {
        if agent.revoked_at.is_some() {
            return false;
        }
        if self.session_registry.is_online(&agent.id).await {
            return true;
        }
        agent
            .last_seen_at
            .map(|last_seen| {
                Utc::now().signed_duration_since(last_seen).num_seconds()
                    <= MCP_ONLINE_THRESHOLD_SECS
            })
            .unwrap_or(false)
    }

    async fn load_all_visible_agents_with_state(
        &self,
        auth: &AuthSession,
        limit: i64,
    ) -> Result<Vec<AgentWithState>> {
        let repo = AgentRepository::new(self.db.clone());
        let rows = if let Some(server_ids) = auth.server_ids.as_deref() {
            let owner_filter = if auth.role.is_admin() {
                None
            } else {
                Some(auth.user_id)
            };
            let (rows, _) = repo
                .list_with_state_by_server_ids(owner_filter, server_ids, limit, 0)
                .await?;
            rows
        } else if auth.role.is_admin() {
            let (rows, _) = repo.list_with_state(limit, 0).await?;
            rows
        } else {
            let (rows, _) = repo
                .list_with_state_by_owner(auth.user_id, limit, 0)
                .await?;
            rows
        };

        Ok(rows
            .into_iter()
            .filter(|row| agent_visible_to_auth(auth, &row.agent))
            .collect())
    }

    async fn build_enriched_server_json(&self, row: &AgentWithState) -> Value {
        let agent = &row.agent;
        let online = self.is_agent_online(agent).await;
        let revoked = agent.revoked_at.is_some();
        let status = if revoked {
            "revoked"
        } else if online {
            "online"
        } else {
            "offline"
        };

        let state_val = parse_agent_telemetry_json::<Value>(row.last_state_json.as_deref());
        let info_val = parse_agent_telemetry_json::<Value>(row.last_info_json.as_deref());
        let dash_val =
            parse_dashboard_metadata_json::<Value>(row.dashboard_metadata_json.as_deref());

        let cpu_percent = state_val
            .as_ref()
            .and_then(|v| v.get("cpu_percent"))
            .and_then(Value::as_f64);
        let memory_used = state_val
            .as_ref()
            .and_then(|v| v.get("memory_used"))
            .and_then(Value::as_i64);
        let memory_total = state_val
            .as_ref()
            .and_then(|v| v.get("memory_total"))
            .and_then(Value::as_i64)
            .or_else(|| {
                info_val
                    .as_ref()
                    .and_then(|v| v.get("total_memory"))
                    .and_then(Value::as_i64)
            });
        let memory_percent = match (memory_used, memory_total) {
            (Some(u), Some(t)) if t > 0 => Some(((u as f64 / t as f64) * 1000.0).round() / 10.0),
            _ => None,
        };

        let (disk_used, disk_total) = extract_disk_totals(state_val.as_ref(), info_val.as_ref());
        let disk_percent = match (disk_used, disk_total) {
            (Some(u), Some(t)) if t > 0 => Some(((u as f64 / t as f64) * 1000.0).round() / 10.0),
            _ => None,
        };

        let load_1 = state_val
            .as_ref()
            .and_then(|v| v.get("load_1"))
            .and_then(Value::as_f64);
        let load_5 = state_val
            .as_ref()
            .and_then(|v| v.get("load_5"))
            .and_then(Value::as_f64);
        let load_15 = state_val
            .as_ref()
            .and_then(|v| v.get("load_15"))
            .and_then(Value::as_f64);

        let net_rx_bps = state_val
            .as_ref()
            .and_then(|v| v.get("net_rx_bps").or_else(|| v.get("network_in_speed")))
            .and_then(Value::as_i64);
        let net_tx_bps = state_val
            .as_ref()
            .and_then(|v| v.get("net_tx_bps").or_else(|| v.get("network_out_speed")))
            .and_then(Value::as_i64);
        let network_in_total = state_val
            .as_ref()
            .and_then(|v| v.get("network_in_total"))
            .and_then(Value::as_i64);
        let network_out_total = state_val
            .as_ref()
            .and_then(|v| v.get("network_out_total"))
            .and_then(Value::as_i64);
        let uptime_seconds = state_val
            .as_ref()
            .and_then(|v| v.get("uptime_seconds").or_else(|| v.get("uptime")))
            .and_then(Value::as_i64);

        let os = info_val
            .as_ref()
            .and_then(|v| v.get("os").or_else(|| v.get("platform")))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let arch = info_val
            .as_ref()
            .and_then(|v| v.get("arch"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let kernel = info_val
            .as_ref()
            .and_then(|v| v.get("kernel_version"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let hostname = info_val
            .as_ref()
            .and_then(|v| v.get("hostname"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let cpu_cores = info_val
            .as_ref()
            .and_then(|v| v.get("cpu_cores"))
            .and_then(Value::as_i64);

        let provider = dash_val
            .as_ref()
            .and_then(|v| v.get("provider"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let region = dash_val
            .as_ref()
            .and_then(|v| v.get("region"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let country = dash_val
            .as_ref()
            .and_then(|v| v.get("country"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let tags: Vec<String> = dash_val
            .as_ref()
            .and_then(|v| v.get("tags"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        json!({
            "server_id": agent.id.0.to_string(),
            "name": agent.name,
            "online": online,
            "status": status,
            "last_seen_at": agent.last_seen_at.map(|dt| dt.to_rfc3339()),
            "created_at": agent.created_at.to_rfc3339(),
            "revoked": revoked,
            "cpu_percent": cpu_percent,
            "memory_used": memory_used,
            "memory_total": memory_total,
            "memory_percent": memory_percent,
            "disk_used": disk_used,
            "disk_total": disk_total,
            "disk_percent": disk_percent,
            "load_1": load_1,
            "load_5": load_5,
            "load_15": load_15,
            "net_rx_bps": net_rx_bps,
            "net_tx_bps": net_tx_bps,
            "network_in_total": network_in_total,
            "network_out_total": network_out_total,
            "uptime_seconds": uptime_seconds,
            "os": os,
            "arch": arch,
            "kernel": kernel,
            "hostname": hostname,
            "cpu_cores": cpu_cores,
            "provider": provider,
            "region": region,
            "country": country,
            "tags": tags,
            "remark": row.remark,
            "expires_at": row.expires_at,
        })
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
        let rx = response_registry.register(run_id.clone(), agent_id).await;
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
        let rx = response_registry.register(run_id.clone(), agent_id).await;
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

fn extract_disk_totals(
    state_val: Option<&Value>,
    info_val: Option<&Value>,
) -> (Option<i64>, Option<i64>) {
    if let Some(disks) = state_val
        .and_then(|v| v.get("disks"))
        .and_then(Value::as_array)
    {
        if !disks.is_empty() {
            let mut used = 0i64;
            let mut total = 0i64;
            for d in disks {
                used = used.saturating_add(d.get("used").and_then(Value::as_i64).unwrap_or(0));
                total = total.saturating_add(d.get("total").and_then(Value::as_i64).unwrap_or(0));
            }
            if total > 0 {
                return (Some(used), Some(total));
            }
        }
    }
    if let Some(disks) = info_val
        .and_then(|v| v.get("disks"))
        .and_then(Value::as_array)
    {
        let mut total = 0i64;
        for d in disks {
            total = total.saturating_add(d.get("total").and_then(Value::as_i64).unwrap_or(0));
        }
        if total > 0 {
            return (None, Some(total));
        }
    }
    (None, None)
}

fn server_json_matches_query(item: &Value, q_lower: &str) -> bool {
    for key in [
        "name",
        "server_id",
        "region",
        "country",
        "provider",
        "hostname",
        "os",
        "remark",
    ] {
        if let Some(val) = item[key].as_str() {
            if val.to_ascii_lowercase().contains(q_lower) {
                return true;
            }
        }
    }
    if let Some(tags) = item["tags"].as_array() {
        for tag in tags {
            if let Some(t) = tag.as_str() {
                if t.to_ascii_lowercase().contains(q_lower) {
                    return true;
                }
            }
        }
    }
    false
}

fn compute_f64_stats(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({
            "min": Value::Null,
            "max": Value::Null,
            "avg": Value::Null,
            "latest": Value::Null,
            "samples": 0,
        });
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sum = 0.0;
    for &v in values {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
        sum += v;
    }
    let avg = sum / values.len() as f64;
    let latest = *values.last().unwrap_or(&avg);
    let round1 = |x: f64| ((x * 10.0).round()) / 10.0;
    json!({
        "min": round1(min),
        "max": round1(max),
        "avg": round1(avg),
        "latest": round1(latest),
        "samples": values.len(),
    })
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
    if !bytes.len().is_multiple_of(4) {
        return Err("length is not a multiple of 4".to_string());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.as_chunks::<4>().0 {
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
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
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
        let db = test_db().await;
        let owner = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        seed_user(&db, owner, "owner", "admin").await;
        let executor = test_executor(db);
        let auth = pat_auth(owner, UserRole::Admin, vec![]);
        let res = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "meta.whoami".into(),
                    arguments: json!({}),
                },
            )
            .await;
        assert!(res.success);
        let val = res.result.unwrap();
        assert_eq!(val["user_id"], owner.0.to_string());
        assert_eq!(val["protocol_version"], "2024-11-05");
    }

    #[tokio::test]
    async fn mcp_cluster_overview_and_diagnose_and_resolve_work() {
        let db = test_db().await;
        let owner = UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        seed_user(&db, owner, "owner", "admin").await;
        let repo = AgentRepository::new(db.clone());
        let agent = repo
            .create(CreateAgentInput {
                name: "tokyo-edge-01".into(),
                public_key: "pk".into(),
                owner_user_id: owner,
            })
            .await
            .unwrap();
        repo.update_last_seen(agent.id).await.unwrap();
        repo.update_last_state(
            agent.id,
            &json!({
                "cpu_percent": 92.5,
                "memory_used": 9_000_000_000_i64,
                "memory_total": 10_000_000_000_i64,
                "load_1": 3.4,
            })
            .to_string(),
        )
        .await
        .unwrap();

        let executor = test_executor(db);
        let auth = pat_auth(owner, UserRole::Admin, vec![agent.id.0.to_string()]);

        let overview = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "cluster.overview".into(),
                    arguments: json!({}),
                },
            )
            .await;
        assert!(overview.success);
        assert_eq!(
            overview.result.as_ref().unwrap()["fleet"]["online_servers"],
            1
        );

        let diag = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "cluster.diagnose".into(),
                    arguments: json!({ "cpu_threshold": 80.0 }),
                },
            )
            .await;
        assert!(diag.success);
        let diag_val = diag.result.unwrap();
        assert!(diag_val["anomalies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["category"] == "compute"));

        let resolved = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.resolve".into(),
                    arguments: json!({ "query": "tokyo" }),
                },
            )
            .await;
        assert!(resolved.success);
        assert_eq!(
            resolved.result.unwrap()["best_match"]["server_id"],
            agent.id.0.to_string()
        );

        let res_overview = executor
            .read_resource(&auth, "xlstatus://cluster/overview")
            .await
            .unwrap();
        assert_eq!(res_overview["fleet"]["total_servers"], 1);

        let prompt = executor
            .get_prompt(
                &auth,
                "sre.troubleshoot_node",
                &json!({ "server_id": "tokyo", "symptom": "high_cpu" }),
            )
            .await
            .unwrap();
        assert!(prompt["messages"].as_array().unwrap().len() == 1);
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

    #[tokio::test]
    async fn server_inspect_rejects_unknown_playbook_name() {
        let db = test_db().await;
        let owner = UserId(uuid::Uuid::now_v7());
        seed_user(&db, owner, "owner", "member").await;

        let agent_repo = AgentRepository::new(db.clone());
        let agent = agent_repo
            .create(CreateAgentInput {
                name: "inspect-node".into(),
                public_key: "hash-inspect".into(),
                owner_user_id: owner,
            })
            .await
            .unwrap();

        let executor = test_executor(db);
        let auth = pat_auth(owner, UserRole::Member, vec![agent.id.0.to_string()]);
        let response = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.inspect".into(),
                    arguments: json!({
                        "server_id": agent.id.0.to_string(),
                        "playbook": "custom_shell; uname -a"
                    }),
                },
            )
            .await;
        assert!(!response.success);
        assert!(response
            .error
            .unwrap_or_default()
            .contains("Unsupported diagnostic playbook"));
    }

    #[tokio::test]
    async fn mcp_2_0_tools_enforce_tenant_and_pat_server_allowlist_isolation() {
        let db = test_db().await;
        let owner_a = UserId(uuid::Uuid::now_v7());
        let owner_b = UserId(uuid::Uuid::now_v7());
        let admin_id = UserId(uuid::Uuid::now_v7());
        seed_user(&db, owner_a, "owner-a", "member").await;
        seed_user(&db, owner_b, "owner-b", "member").await;
        seed_user(&db, admin_id, "admin", "admin").await;

        let agent_repo = AgentRepository::new(db.clone());
        let allowed_agent = agent_repo
            .create(CreateAgentInput {
                name: "alpha-allowed-node".into(),
                public_key: "hash-alpha".into(),
                owner_user_id: owner_a,
            })
            .await
            .unwrap();
        let denied_own_agent = agent_repo
            .create(CreateAgentInput {
                name: "beta-unlisted-node".into(),
                public_key: "hash-beta".into(),
                owner_user_id: owner_a,
            })
            .await
            .unwrap();
        let foreign_agent = agent_repo
            .create(CreateAgentInput {
                name: "gamma-foreign-node".into(),
                public_key: "hash-gamma".into(),
                owner_user_id: owner_b,
            })
            .await
            .unwrap();

        let DatabaseBackend::Sqlite(pool) = &db else {
            unreachable!();
        };

        // Seed services: global (no server_id), allowed_agent, denied_own_agent, foreign_agent
        for (svc_id, uid, name, sid) in [
            (
                uuid::Uuid::now_v7().to_string(),
                admin_id.0.to_string(),
                "global-probe",
                None::<String>,
            ),
            (
                uuid::Uuid::now_v7().to_string(),
                owner_a.0.to_string(),
                "allowed-probe",
                Some(allowed_agent.id.0.to_string()),
            ),
            (
                uuid::Uuid::now_v7().to_string(),
                owner_a.0.to_string(),
                "denied-own-probe",
                Some(denied_own_agent.id.0.to_string()),
            ),
            (
                uuid::Uuid::now_v7().to_string(),
                owner_b.0.to_string(),
                "foreign-probe",
                Some(foreign_agent.id.0.to_string()),
            ),
        ] {
            sqlx::query(
                "INSERT INTO services (id, owner_user_id, name, type, target, interval_seconds, timeout_seconds, enabled, server_id, created_at, updated_at) VALUES (?, ?, ?, 'http', 'https://example.com', 60, 5, 1, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            )
            .bind(svc_id)
            .bind(uid)
            .bind(name)
            .bind(sid)
            .execute(pool)
            .await
            .unwrap();
        }

        // Seed NAT mappings & DDNS configs
        for (aid, port, uid, domain) in [
            (
                allowed_agent.id.0.to_string(),
                10001i64,
                owner_a.0.to_string(),
                "allowed.example.com",
            ),
            (
                denied_own_agent.id.0.to_string(),
                10002i64,
                owner_a.0.to_string(),
                "denied.example.com",
            ),
            (
                foreign_agent.id.0.to_string(),
                10003i64,
                owner_b.0.to_string(),
                "foreign.example.com",
            ),
        ] {
            sqlx::query(
                "INSERT INTO nat_mappings (id, agent_id, local_host, local_port, public_port, protocol, enabled, created_at, updated_at) VALUES (?, ?, '127.0.0.1', 8080, ?, 'tcp', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&aid)
            .bind(port)
            .execute(pool)
            .await
            .unwrap();

            sqlx::query(
                "INSERT INTO ddns_configs (id, owner_user_id, name, provider, domain, api_token, agent_id, enabled, created_at, updated_at) VALUES (?, ?, ?, 'cloudflare', ?, 'enc', ?, 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&uid)
            .bind(domain)
            .bind(domain)
            .bind(&aid)
            .execute(pool)
            .await
            .unwrap();
        }

        // Seed tasks for owner_a: one targeting allowed_agent, one targeting denied_own_agent
        for (name, target_sid) in [
            ("allowed-task", allowed_agent.id.0.to_string()),
            ("denied-task", denied_own_agent.id.0.to_string()),
        ] {
            sqlx::query(
                "INSERT INTO tasks (id, owner_user_id, name, task_type, command, cover_mode, server_selector_json, enabled, created_at, updated_at) VALUES (?, ?, ?, '\"shell\"', 'uptime', '\"specific\"', ?, 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(owner_a.0.to_string())
            .bind(name)
            .bind(serde_json::to_string(&json!({ "server_ids": [target_sid] })).unwrap())
            .execute(pool)
            .await
            .unwrap();
        }

        let executor = test_executor(db);
        // PAT belongs to owner_a and includes allowed_agent + foreign_agent (which owner_a does not own)
        let auth = pat_auth(
            owner_a,
            UserRole::Member,
            vec![
                allowed_agent.id.0.to_string(),
                foreign_agent.id.0.to_string(),
            ],
        );

        let overview = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "cluster.overview".into(),
                    arguments: json!({}),
                },
            )
            .await;
        assert!(overview.success);
        let ov = overview.result.unwrap();
        assert_eq!(ov["fleet"]["total_servers"], 1);
        assert_eq!(ov["services"]["total_monitors"], 1);

        let diagnose = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "cluster.diagnose".into(),
                    arguments: json!({}),
                },
            )
            .await;
        assert!(diagnose.success);
        let diag = diagnose.result.unwrap();
        assert_eq!(diag["counts"]["active_servers"], 1);
        assert_eq!(diag["counts"]["total_anomalies"], 1);

        let resolve_foreign = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.resolve".into(),
                    arguments: json!({ "query": "gamma-foreign" }),
                },
            )
            .await;
        assert!(resolve_foreign.success);
        assert_eq!(resolve_foreign.result.unwrap()["total"], 0);

        let resolve_denied = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.resolve".into(),
                    arguments: json!({ "query": "beta-unlisted" }),
                },
            )
            .await;
        assert!(resolve_denied.success);
        assert_eq!(resolve_denied.result.unwrap()["total"], 0);

        let metrics_denied = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.metrics".into(),
                    arguments: json!({ "server_id": denied_own_agent.id.0.to_string() }),
                },
            )
            .await;
        assert!(!metrics_denied.success);

        let metrics_foreign = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "server.metrics".into(),
                    arguments: json!({ "server_id": foreign_agent.id.0.to_string() }),
                },
            )
            .await;
        assert!(!metrics_foreign.success);

        let svc_list = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "service.list".into(),
                    arguments: json!({}),
                },
            )
            .await;
        assert!(svc_list.success);
        let svcs = svc_list.result.unwrap();
        assert_eq!(svcs["total"], 1);
        assert_eq!(svcs["services"][0]["name"], "allowed-probe");

        let topology = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "network.topology".into(),
                    arguments: json!({}),
                },
            )
            .await;
        assert!(topology.success);
        let topo = topology.result.unwrap();
        assert_eq!(topo["nat_total"], 1);
        assert_eq!(
            topo["nat_mappings"][0]["server_id"],
            allowed_agent.id.0.to_string()
        );
        assert_eq!(topo["ddns_total"], 1);
        assert_eq!(topo["ddns_records"][0]["domain"], "allowed.example.com");

        let tasks = executor
            .execute(
                &auth,
                McpToolRequest {
                    tool: "task.list".into(),
                    arguments: json!({}),
                },
            )
            .await;
        assert!(tasks.success);
        let task_res = tasks.result.unwrap();
        assert_eq!(task_res["total"], 1);
        assert_eq!(task_res["tasks"][0]["name"], "allowed-task");
    }
}
