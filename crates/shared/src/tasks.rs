use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Task type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Shell,
    HttpGet,
    IcmpPing,
    TcpPing,
}

/// Task execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Success,
    Failure,
    Timeout,
    Offline,
}

/// Task cover mode - which servers to run on
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverMode {
    /// Run on all servers matching selector
    All,
    /// Run on any one server matching selector
    Any,
    /// Run on specific servers only
    Specific,
}

/// Server selector
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSelector {
    /// Specific server IDs (when cover_mode = specific)
    #[serde(default)]
    pub server_ids: Vec<String>,

    /// Server group IDs
    #[serde(default)]
    pub group_ids: Vec<String>,

    /// Tag filters
    #[serde(default)]
    pub tags: HashMap<String, String>,
}

impl Default for ServerSelector {
    fn default() -> Self {
        Self {
            server_ids: Vec::new(),
            group_ids: Vec::new(),
            tags: HashMap::new(),
        }
    }
}

/// Shell command task payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellTaskPayload {
    pub command: String,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
    #[serde(default = "default_max_output")]
    pub max_output_bytes: u64,
}

/// HTTP GET task payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpGetTaskPayload {
    pub url: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
    #[serde(default = "default_verify_tls")]
    pub verify_tls: bool,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

/// ICMP ping task payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcmpPingTaskPayload {
    pub host: String,
    #[serde(default = "default_ping_count")]
    pub count: u32,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
}

/// TCP ping task payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpPingTaskPayload {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
}

fn default_timeout() -> u32 {
    30
}

fn default_max_output() -> u64 {
    1024 * 1024 // 1 MiB
}

fn default_verify_tls() -> bool {
    true
}

fn default_ping_count() -> u32 {
    4
}

/// Task definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub task_type: TaskType,
    pub schedule: Option<String>,
    pub command: Option<String>,
    pub payload_json: Option<String>,
    pub cover_mode: CoverMode,
    pub server_selector_json: String,
    pub push_successful: bool,
    pub notification_group_id: Option<String>,
    pub last_executed_at: Option<String>,
    pub last_result: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Task run record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub id: String,
    pub task_id: String,
    pub server_id: String,
    pub status: TaskStatus,
    pub delay_ms: Option<i64>,
    pub output: Option<String>,
    pub output_truncated: bool,
    pub error: Option<String>,
    pub created_at: String,
}

/// File transfer operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferOp {
    Upload,
    Download,
}

/// File transfer status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// File transfer record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub id: String,
    pub owner_user_id: String,
    pub server_id: String,
    pub op: TransferOp,
    pub path: String,
    pub size: i64,
    pub status: TransferStatus,
    pub error: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: String,
    pub user_id: Option<String>,
    pub api_token_id: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub server_id: Option<String>,
    pub ip: String,
    pub outcome: String,
    pub metadata_json: Option<String>,
    pub sensitive_hash: Option<String>,
    pub created_at: String,
}
