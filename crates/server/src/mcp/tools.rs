use serde::{Deserialize, Serialize};

/// MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub category: String,
    pub input_schema: serde_json::Value,
    #[serde(default)]
    pub annotations: serde_json::Value,
}

/// MCP static resource definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub mime_type: String,
}

/// MCP resource template definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceTemplate {
    pub uri_template: String,
    pub name: String,
    pub description: String,
    pub mime_type: String,
}

/// MCP prompt argument definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// MCP prompt definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPrompt {
    pub name: String,
    pub description: String,
    pub arguments: Vec<McpPromptArgument>,
}

/// MCP tool execution request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolRequest {
    pub tool: String,
    pub arguments: serde_json::Value,
}

/// MCP tool execution response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResponse {
    pub success: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

fn read_only_annotations() -> serde_json::Value {
    serde_json::json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    })
}

fn exec_annotations(destructive: bool) -> serde_json::Value {
    serde_json::json!({
        "readOnlyHint": false,
        "destructiveHint": destructive,
        "idempotentHint": false,
        "openWorldHint": true
    })
}

/// Available MCP tools
pub fn get_available_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "meta.whoami".to_string(),
            description: "Get current authenticated identity, scopes, server allowlist, and XLStatus MCP capabilities".to_string(),
            category: "meta".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "cluster.overview".to_string(),
            description: "Get a comprehensive fleet-wide summary: online/offline/revoked server counts, aggregate CPU/RAM utilization, network RX/TX throughput, service monitor SLA health, active alerts, and regional breakdown".to_string(),
            category: "cluster".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "cluster.diagnose".to_string(),
            description: "Run an AI SRE automated anomaly & health scan across all accessible servers, service probes, and alert events. Returns a cluster health score (0-100), prioritized anomalies with root-cause hints, and recommended follow-up tools".to_string(),
            category: "diagnostics".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "cpu_threshold": {
                        "type": "number",
                        "description": "CPU usage percentage threshold for flagging hotspots",
                        "default": 85.0
                    },
                    "memory_threshold": {
                        "type": "number",
                        "description": "Memory usage percentage threshold for flagging pressure",
                        "default": 85.0
                    },
                    "disk_threshold": {
                        "type": "number",
                        "description": "Disk usage percentage threshold for flagging storage exhaustion",
                        "default": 85.0
                    }
                },
                "required": []
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "server.list".to_string(),
            description: "List all servers accessible to the current user with live CPU, memory, disk, load, network throughput, geo-location, and tags".to_string(),
            category: "server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "description": "Filter by node status: all, online, or offline",
                        "enum": ["all", "online", "offline"],
                        "default": "all"
                    },
                    "search": {
                        "type": "string",
                        "description": "Optional substring filter matching server name, region, country, provider, or tag"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of servers to return",
                        "default": 50
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Number of servers to skip",
                        "default": 0
                    }
                },
                "required": []
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "server.get".to_string(),
            description: "Get detailed hardware specifications, metadata, and real-time telemetry for a specific server by canonical UUID".to_string(),
            category: "server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Canonical Server UUID"
                    }
                },
                "required": ["server_id"]
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "server.resolve".to_string(),
            description: "Fuzzy-resolve a server by name, partial UUID, hostname, region, country, provider, or tag into canonical server_id UUIDs".to_string(),
            category: "server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query (server name, partial ID, region, provider, or tag)"
                    }
                },
                "required": ["query"]
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "server.metrics".to_string(),
            description: "Query time-series telemetry (CPU, memory, load_1, network RX/TX) for a server with statistical min/max/avg/latest aggregates".to_string(),
            category: "server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Canonical Server UUID"
                    },
                    "range": {
                        "type": "string",
                        "description": "Time window to analyze: 15m, 1h, 6h, or 24h",
                        "enum": ["15m", "1h", "6h", "24h"],
                        "default": "1h"
                    }
                },
                "required": ["server_id"]
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "server.exec".to_string(),
            description: "Execute a shell command on a single online server over the mTLS gRPC agent stream".to_string(),
            category: "server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Canonical Server UUID"
                    },
                    "command": {
                        "type": "string",
                        "description": "Command to execute",
                        "minLength": 1,
                        "maxLength": 8192
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds",
                        "default": 30,
                        "minimum": 1,
                        "maximum": 60
                    }
                },
                "required": ["server_id", "command"]
            }),
            annotations: exec_annotations(true),
        },
        McpTool {
            name: "server.batch_exec".to_string(),
            description: "Execute a shell command concurrently across multiple online servers (up to 10 nodes) and aggregate exit codes and stdout/stderr".to_string(),
            category: "server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of canonical Server UUIDs (omit if target='all_online')"
                    },
                    "target": {
                        "type": "string",
                        "description": "Set to 'all_online' to target up to 10 online visible servers",
                        "enum": ["selected", "all_online"],
                        "default": "selected"
                    },
                    "command": {
                        "type": "string",
                        "description": "Shell command to run on each target server",
                        "minLength": 1,
                        "maxLength": 8192
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Per-server timeout in seconds",
                        "default": 20,
                        "minimum": 1,
                        "maximum": 60
                    }
                },
                "required": ["command"]
            }),
            annotations: exec_annotations(true),
        },
        McpTool {
            name: "server.inspect".to_string(),
            description: "Run a safe, read-only SRE diagnostic playbook on a live online server (system vitals, top CPU/RAM processes, disk & inode usage, listening sockets, Docker containers, or recent system logs)".to_string(),
            category: "diagnostics".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Canonical Server UUID"
                    },
                    "playbook": {
                        "type": "string",
                        "description": "Predefined read-only diagnostic playbook to run",
                        "enum": [
                            "system_vital",
                            "top_processes",
                            "disk_and_inodes",
                            "network_sockets",
                            "docker_containers",
                            "system_logs"
                        ],
                        "default": "system_vital"
                    }
                },
                "required": ["server_id"]
            }),
            annotations: exec_annotations(false),
        },
        McpTool {
            name: "service.list".to_string(),
            description: "List HTTP, TCP, and ICMP service monitors with latest probe status, latency, and TLS certificate expiry".to_string(),
            category: "service".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of service monitors to return",
                        "default": 50
                    }
                },
                "required": []
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "service.history".to_string(),
            description: "Get recent probe check results, availability percentage, and average latency for a specific service monitor".to_string(),
            category: "service".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "service_id": {
                        "type": "string",
                        "description": "Service Monitor UUID"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Number of recent probe checks to retrieve",
                        "default": 30
                    }
                },
                "required": ["service_id"]
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "alert.events".to_string(),
            description: "List recent alert events (firing and recovered incidents) across visible servers and services".to_string(),
            category: "alert".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "description": "Optional filter by event kind (e.g. firing, resolved, all)",
                        "default": "all"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of alert events to return",
                        "default": 25
                    }
                },
                "required": []
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "alert.rules".to_string(),
            description: "List configured alert rules, trigger conditions, and automated remediation task bindings".to_string(),
            category: "alert".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of alert rules to return",
                        "default": 50
                    }
                },
                "required": []
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "task.list".to_string(),
            description: "List scheduled cron jobs and automation tasks configured in the cluster".to_string(),
            category: "automation".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of tasks to return",
                        "default": 25
                    }
                },
                "required": []
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "network.topology".to_string(),
            description: "Inspect NAT reverse-tunnel port mappings and Dynamic DNS (DDNS) domain records across visible servers".to_string(),
            category: "network".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "fs.list".to_string(),
            description: "List files in a directory on a server".to_string(),
            category: "filesystem".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory path"
                    }
                },
                "required": ["server_id", "path"]
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "fs.read".to_string(),
            description: "Read a file from a server".to_string(),
            category: "filesystem".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path"
                    },
                    "max_size": {
                        "type": "integer",
                        "description": "Maximum file size in bytes",
                        "default": 1048576
                    }
                },
                "required": ["server_id", "path"]
            }),
            annotations: read_only_annotations(),
        },
        McpTool {
            name: "fs.download_url".to_string(),
            description: "Generate a temporary download URL for a file".to_string(),
            category: "filesystem".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path"
                    },
                    "expires_in": {
                        "type": "integer",
                        "description": "URL expiration time in seconds",
                        "default": 3600
                    }
                },
                "required": ["server_id", "path"]
            }),
            annotations: read_only_annotations(),
        },
    ]
}

/// Available MCP static resources
pub fn get_available_resources() -> Vec<McpResource> {
    vec![
        McpResource {
            uri: "xlstatus://cluster/overview".to_string(),
            name: "Cluster Health & Anomaly Overview".to_string(),
            description: "Real-time fleet summary, aggregate resource saturation, network bandwidth, service probe SLAs, and active incidents".to_string(),
            mime_type: "application/json".to_string(),
        },
        McpResource {
            uri: "xlstatus://servers/inventory".to_string(),
            name: "Live Server Fleet Inventory & Telemetry".to_string(),
            description: "Full list of accessible servers enriched with CPU, RAM, disk, load average, network RX/TX, geo-location, and tags".to_string(),
            mime_type: "application/json".to_string(),
        },
        McpResource {
            uri: "xlstatus://services/health".to_string(),
            name: "Service Probes & SLA Status".to_string(),
            description: "HTTP, TCP, and ICMP service monitors with latest check status, latency, and TLS certificate expiration".to_string(),
            mime_type: "application/json".to_string(),
        },
        McpResource {
            uri: "xlstatus://alerts/active".to_string(),
            name: "Active Incidents, Alert Events & Rules".to_string(),
            description: "Recent alert events and configured threshold rules across the cluster".to_string(),
            mime_type: "application/json".to_string(),
        },
        McpResource {
            uri: "xlstatus://network/topology".to_string(),
            name: "NAT Port Mappings & DDNS Records".to_string(),
            description: "Active reverse-tunnel NAT port mappings and Dynamic DNS domain bindings".to_string(),
            mime_type: "application/json".to_string(),
        },
    ]
}

/// Available MCP resource templates
pub fn get_available_resource_templates() -> Vec<McpResourceTemplate> {
    vec![McpResourceTemplate {
        uri_template: "xlstatus://servers/{server_id}/telemetry".to_string(),
        name: "Deep Server Hardware, Live State & 1h TSDB Summary".to_string(),
        description: "Hardware specifications, live host telemetry, and 1-hour time-series statistical aggregates for a specific server UUID".to_string(),
        mime_type: "application/json".to_string(),
    }]
}

/// Available MCP prompts
pub fn get_available_prompts() -> Vec<McpPrompt> {
    vec![
        McpPrompt {
            name: "sre.cluster_health_audit".to_string(),
            description: "Comprehensive AI SRE audit of fleet health, resource bottlenecks, offline nodes, and active incidents".to_string(),
            arguments: vec![],
        },
        McpPrompt {
            name: "sre.troubleshoot_node".to_string(),
            description: "Deep-dive root-cause analysis for a specific server node with live telemetry and diagnostic playbook guidance".to_string(),
            arguments: vec![
                McpPromptArgument {
                    name: "server_id".to_string(),
                    description: "Canonical Server UUID or server name to troubleshoot".to_string(),
                    required: true,
                },
                McpPromptArgument {
                    name: "symptom".to_string(),
                    description: "Observed symptom (e.g. high_cpu, memory_pressure, disk_full, network_spike, offline)".to_string(),
                    required: false,
                },
            ],
        },
        McpPrompt {
            name: "sre.incident_triage".to_string(),
            description: "Triage active alerts and degraded service probes with actionable remediation commands".to_string(),
            arguments: vec![],
        },
        McpPrompt {
            name: "sre.capacity_and_security".to_string(),
            description: "Audit cluster resource saturation, expiring nodes, and exposed NAT/DDNS endpoints".to_string(),
            arguments: vec![],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_available_tools() {
        let tools = get_available_tools();
        assert_eq!(tools.len(), 19);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(tool_names.contains(&"meta.whoami"));
        assert!(tool_names.contains(&"cluster.overview"));
        assert!(tool_names.contains(&"cluster.diagnose"));
        assert!(tool_names.contains(&"server.list"));
        assert!(tool_names.contains(&"server.get"));
        assert!(tool_names.contains(&"server.resolve"));
        assert!(tool_names.contains(&"server.metrics"));
        assert!(tool_names.contains(&"server.exec"));
        assert!(tool_names.contains(&"server.batch_exec"));
        assert!(tool_names.contains(&"server.inspect"));
        assert!(tool_names.contains(&"service.list"));
        assert!(tool_names.contains(&"service.history"));
        assert!(tool_names.contains(&"alert.events"));
        assert!(tool_names.contains(&"alert.rules"));
        assert!(tool_names.contains(&"task.list"));
        assert!(tool_names.contains(&"network.topology"));
        assert!(tool_names.contains(&"fs.list"));
        assert!(tool_names.contains(&"fs.read"));
        assert!(tool_names.contains(&"fs.download_url"));
        assert!(!tool_names.contains(&"fs.write"));
        assert!(!tool_names.contains(&"fs.delete"));
        assert!(!tool_names.contains(&"fs.upload_url"));
    }

    #[test]
    fn test_resources_and_prompts_catalog() {
        let resources = get_available_resources();
        assert_eq!(resources.len(), 5);
        let templates = get_available_resource_templates();
        assert_eq!(templates.len(), 1);
        let prompts = get_available_prompts();
        assert_eq!(prompts.len(), 4);
    }
}
