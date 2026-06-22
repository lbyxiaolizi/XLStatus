use anyhow::Context;
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{lookup_host, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, Duration};
use tokio_stream::wrappers::ReceiverStream;
use tonic::metadata::MetadataValue;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use tonic::Request;
use xlstatus_proto_gen::xlstatus::v1::agent_message::Payload;
use xlstatus_proto_gen::xlstatus::v1::agent_service_client::AgentServiceClient;
use xlstatus_proto_gen::xlstatus::v1::{
    io_frame, AgentMessage, ConfigUpdate, ForceUpdate, GeoIpReport, Heartbeat, IoData, IoError,
    IoFrame,
};
use xlstatus_shared::nat::NatTunnelControlMessage;
use xlstatus_shared::terminal::TerminalBridgeMessage;

mod collector;
mod executor;

const GRPC_MESSAGE_LIMIT: usize = 256 * 1024 * 1024;
const AGENT_SHELL_MIN_TIMEOUT_SECONDS: u64 = 1;
const AGENT_SHELL_DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const AGENT_SHELL_MAX_TIMEOUT_SECONDS: u64 = 60;
const AGENT_SHELL_DEFAULT_OUTPUT_MAX_BYTES: u64 = 64 * 1024;
const AGENT_SHELL_OUTPUT_MAX_BYTES: u64 = 64 * 1024;
const AGENT_PROBE_MIN_TIMEOUT_SECONDS: u64 = 1;
const AGENT_PROBE_DEFAULT_TIMEOUT_SECONDS: u64 = 10;
const AGENT_PROBE_MAX_TIMEOUT_SECONDS: u64 = 30;
const AGENT_PING_PROCESS_TIMEOUT_GRACE_SECONDS: u64 = 2;
const AGENT_PING_OUTPUT_MAX_BYTES: usize = 4096;
const REMOTE_CONFIG_MAX_BYTES: usize = 128 * 1024;
const REMOTE_CONFIG_MAX_NAME_BYTES: usize = 255;
const REMOTE_CONFIG_MAX_URL_BYTES: usize = 2048;
const REMOTE_CONFIG_MAX_PATH_BYTES: usize = 4096;
const REMOTE_CONFIG_MAX_ROOTS: usize = 32;
const REMOTE_CONFIG_MIN_INTERVAL_SECONDS: u64 = 1;
const REMOTE_CONFIG_MAX_INTERVAL_SECONDS: u64 = 86_400;
const FORCE_UPDATE_MAX_VERSION_BYTES: usize = 80;
const FORCE_UPDATE_MAX_URL_BYTES: usize = 2048;
const AGENT_ENROLLMENT_TOKEN_MAX_BYTES: usize = 128;
type TerminalSessionMap =
    Arc<Mutex<std::collections::HashMap<String, Arc<executor::terminal::TerminalSession>>>>;
type NatSessionMap = Arc<Mutex<std::collections::HashMap<String, NatSocketSession>>>;

struct NatSocketSession {
    writer_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    reader_task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct RuntimeContext {
    config_path: PathBuf,
    config: Arc<Mutex<AgentConfig>>,
}

fn default_report_interval_seconds() -> u64 {
    REPORT_INTERVAL_SECS
}

fn default_ip_report_interval_seconds() -> u64 {
    60
}

fn default_file_allowed_roots() -> Vec<String> {
    #[cfg(unix)]
    {
        vec!["/var/lib/xlstatus/files".to_string()]
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir()
            .join("xlstatus-files")
            .to_str()
            .map(|path| vec![path.to_string()])
            .unwrap_or_default()
    }
}

#[derive(Parser)]
#[command(name = "xlstatus-agent")]
#[command(about = "XLStatus monitoring agent", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Enroll this agent with the dashboard
    Enroll {
        /// Dashboard server URL
        #[arg(long)]
        server: String,

        /// gRPC server URL
        #[arg(long)]
        grpc_server: Option<String>,

        /// PEM CA bundle used to verify the gRPC server when using https://
        #[arg(long)]
        grpc_tls_ca_path: Option<String>,

        /// TLS server name override for gRPC certificate verification
        #[arg(long)]
        grpc_tls_domain_name: Option<String>,

        /// PEM client certificate for gRPC mTLS
        #[arg(long)]
        grpc_tls_client_cert_path: Option<String>,

        /// PEM client private key for gRPC mTLS
        #[arg(long)]
        grpc_tls_client_key_path: Option<String>,

        /// Enrollment token. Prefer --token-stdin on shared hosts so the token is not visible in process arguments.
        #[arg(long, conflicts_with = "token_stdin")]
        token: Option<String>,

        /// Read the enrollment token from stdin
        #[arg(long, conflicts_with = "token")]
        token_stdin: bool,

        /// Agent display name
        #[arg(long, default_value = "xlstatus-agent")]
        name: String,

        /// Config file path to write
        #[arg(long, default_value = "agent.yaml")]
        config: String,
    },
    /// Run the agent
    Run {
        /// Config file path
        #[arg(long, default_value = "agent.yaml")]
        config: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentConfig {
    server: String,
    grpc_server: String,
    #[serde(default)]
    grpc_tls_ca_path: Option<String>,
    #[serde(default)]
    grpc_tls_domain_name: Option<String>,
    #[serde(default)]
    grpc_tls_client_cert_path: Option<String>,
    #[serde(default)]
    grpc_tls_client_key_path: Option<String>,
    agent_id: String,
    name: String,
    public_key: String,
    #[serde(default)]
    private_key: String,
    #[serde(default = "default_report_interval_seconds")]
    report_interval_seconds: u64,
    #[serde(default = "default_ip_report_interval_seconds")]
    ip_report_interval_seconds: u64,
    #[serde(default)]
    disable_auto_update: bool,
    #[serde(default)]
    disable_force_update: bool,
    #[serde(default)]
    disable_command_execute: bool,
    #[serde(default)]
    disable_nat: bool,
    #[serde(default)]
    disable_send_query: bool,
    #[serde(default = "default_file_allowed_roots")]
    file_allowed_roots: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct AgentTlsConfigInput {
    ca_path: Option<String>,
    domain_name: Option<String>,
    client_cert_path: Option<String>,
    client_key_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EnrollResponse {
    agent_id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct JwtResponse {
    jwt: String,
}

#[derive(Debug, Deserialize)]
struct JwtChallengeResponse {
    nonce: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Enroll {
            server,
            grpc_server,
            grpc_tls_ca_path,
            grpc_tls_domain_name,
            grpc_tls_client_cert_path,
            grpc_tls_client_key_path,
            token,
            token_stdin,
            name,
            config,
        } => {
            let token = enrollment_token_from_sources(token, token_stdin)?;
            tracing::info!("Enrolling agent with server: {}", server);
            enroll_agent(
                server,
                grpc_server,
                AgentTlsConfigInput {
                    ca_path: grpc_tls_ca_path,
                    domain_name: grpc_tls_domain_name,
                    client_cert_path: grpc_tls_client_cert_path,
                    client_key_path: grpc_tls_client_key_path,
                },
                token,
                name,
                PathBuf::from(config),
            )
            .await
        }
        Commands::Run { config } => {
            tracing::info!("Starting agent with config: {}", config);
            run_agent(PathBuf::from(config)).await
        }
    }
}

fn enrollment_token_from_sources(
    token: Option<String>,
    token_stdin: bool,
) -> anyhow::Result<String> {
    let stdin = std::io::stdin();
    enrollment_token_from_sources_with_reader(token, token_stdin, stdin.lock())
}

fn enrollment_token_from_sources_with_reader<R: Read>(
    token: Option<String>,
    token_stdin: bool,
    reader: R,
) -> anyhow::Result<String> {
    if token_stdin {
        let mut raw = String::new();
        reader
            .take((AGENT_ENROLLMENT_TOKEN_MAX_BYTES + 2) as u64)
            .read_to_string(&mut raw)?;
        return normalize_enrollment_token_input(&raw);
    }

    let Some(token) = token else {
        anyhow::bail!("enrollment token is required; use --token or --token-stdin");
    };
    normalize_enrollment_token_input(&token)
}

fn normalize_enrollment_token_input(raw: &str) -> anyhow::Result<String> {
    let token = raw.trim();
    if token.is_empty() {
        anyhow::bail!("enrollment token is required");
    }
    if token.len() > AGENT_ENROLLMENT_TOKEN_MAX_BYTES {
        anyhow::bail!(
            "enrollment token must be at most {} bytes",
            AGENT_ENROLLMENT_TOKEN_MAX_BYTES
        );
    }
    if token.chars().any(char::is_whitespace) {
        anyhow::bail!("enrollment token must not contain whitespace");
    }
    Ok(token.to_string())
}

async fn enroll_agent(
    server: String,
    grpc_server: Option<String>,
    tls: AgentTlsConfigInput,
    token: String,
    name: String,
    config_path: PathBuf,
) -> anyhow::Result<()> {
    let server = server.trim_end_matches('/').to_string();
    let grpc_server = grpc_server.unwrap_or_else(|| infer_grpc_url(&server));
    let signing_key = generate_signing_key();
    let private_key = hex::encode(signing_key.to_bytes());
    let public_key = hex::encode(signing_key.verifying_key().to_bytes());

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/v1/agents/enroll", server))
        .json(&serde_json::json!({
            "name": name,
            "enrollment_token": token,
            "public_key": public_key,
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<ApiResponse<EnrollResponse>>()
        .await?;

    if !response.success {
        anyhow::bail!(
            "enrollment failed: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }

    let enrolled = response
        .data
        .ok_or_else(|| anyhow::anyhow!("enrollment response did not include data"))?;
    let config = AgentConfig {
        server,
        grpc_server,
        grpc_tls_ca_path: normalize_optional_string(tls.ca_path),
        grpc_tls_domain_name: normalize_optional_string(tls.domain_name),
        grpc_tls_client_cert_path: normalize_optional_string(tls.client_cert_path),
        grpc_tls_client_key_path: normalize_optional_string(tls.client_key_path),
        agent_id: enrolled.agent_id,
        name: enrolled.name,
        public_key,
        private_key,
        report_interval_seconds: default_report_interval_seconds(),
        ip_report_interval_seconds: default_ip_report_interval_seconds(),
        disable_auto_update: false,
        disable_force_update: false,
        disable_command_execute: false,
        disable_nat: false,
        disable_send_query: false,
        file_allowed_roots: default_file_allowed_roots(),
    };

    let serialized = serde_json::to_string_pretty(&config)?;
    write_secure_config(&config_path, serialized.as_bytes())?;
    println!(
        "Agent enrolled and config written to {}",
        config_path.display()
    );
    Ok(())
}

async fn run_agent(config_path: PathBuf) -> anyhow::Result<()> {
    let config_text = std::fs::read_to_string(&config_path)?;
    let config: AgentConfig = serde_json::from_str(&config_text)?;
    let runtime = RuntimeContext {
        config_path: config_path.clone(),
        config: Arc::new(Mutex::new(config)),
    };
    // M2: outer reconnect loop. Each iteration establishes a fresh
    // gRPC stream with a freshly-challenged JWT. Backoff is bounded
    // exponential with full jitter, capped at 60 s.
    let mut attempt: u32 = 0;
    loop {
        match run_agent_session(runtime.clone()).await {
            Ok(SessionExit::Revoked) => {
                tracing::info!("session ended by server ForceDisconnect; not reconnecting");
                return Ok(());
            }
            Ok(SessionExit::StreamClosed) => {
                attempt = attempt.saturating_add(1);
                let delay = backoff_with_jitter(attempt);
                tracing::warn!(
                    "stream closed, reconnecting in {}s (attempt {})",
                    delay.as_secs(),
                    attempt
                );
                tokio::time::sleep(delay).await;
            }
            Err(e) => {
                attempt = attempt.saturating_add(1);
                let delay = backoff_with_jitter(attempt);
                tracing::error!(
                    "session error: {}; reconnecting in {}s (attempt {})",
                    e,
                    delay.as_secs(),
                    attempt
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

async fn connect_grpc_channel(config: &AgentConfig) -> anyhow::Result<Channel> {
    let mut endpoint = Endpoint::new(config.grpc_server.clone())?;
    if let Some(tls_config) = build_grpc_client_tls_config(config).await? {
        endpoint = endpoint
            .tls_config(tls_config)
            .map_err(|e| anyhow::anyhow!("failed to configure gRPC TLS: {}", e))?;
    }
    endpoint
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("failed to connect gRPC server: {}", e))
}

async fn build_grpc_client_tls_config(
    config: &AgentConfig,
) -> anyhow::Result<Option<ClientTlsConfig>> {
    let ca_path = non_empty_config_path(&config.grpc_tls_ca_path);
    let domain_name = non_empty_config_path(&config.grpc_tls_domain_name);
    let client_cert_path = non_empty_config_path(&config.grpc_tls_client_cert_path);
    let client_key_path = non_empty_config_path(&config.grpc_tls_client_key_path);

    if ca_path.is_none()
        && domain_name.is_none()
        && client_cert_path.is_none()
        && client_key_path.is_none()
    {
        return Ok(None);
    }

    if !config.grpc_server.starts_with("https://") {
        anyhow::bail!("custom gRPC TLS settings require grpc_server to use https://");
    }

    let mut tls_config = ClientTlsConfig::new().with_enabled_roots();
    if let Some(domain_name) = domain_name {
        tls_config = tls_config.domain_name(domain_name.to_string());
    }
    if let Some(ca_path) = ca_path {
        let ca = tokio::fs::read(ca_path)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read gRPC TLS CA from {ca_path}: {e}"))?;
        tls_config = tls_config.ca_certificate(Certificate::from_pem(ca));
    }

    match (client_cert_path, client_key_path) {
        (None, None) => {}
        (Some(cert_path), Some(key_path)) => {
            let cert = tokio::fs::read(cert_path).await.map_err(|e| {
                anyhow::anyhow!("failed to read gRPC mTLS client certificate from {cert_path}: {e}")
            })?;
            let key = tokio::fs::read(key_path).await.map_err(|e| {
                anyhow::anyhow!("failed to read gRPC mTLS client private key from {key_path}: {e}")
            })?;
            tls_config = tls_config.identity(Identity::from_pem(cert, key));
        }
        _ => {
            anyhow::bail!(
                "grpc_tls_client_cert_path and grpc_tls_client_key_path must be configured together"
            );
        }
    }

    Ok(Some(tls_config))
}

fn non_empty_config_path(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| non_empty_string(&value))
}

/// One gRPC session: connect -> heartbeat / HostState / HostInfo ->
/// wait for the server to close the stream or send ForceDisconnect.
/// The returned enum tells the outer loop whether to reconnect or exit.
async fn run_agent_session(runtime: RuntimeContext) -> anyhow::Result<SessionExit> {
    let initial = runtime.config.lock().await.clone();
    let jwt = fetch_agent_jwt(&initial).await?;
    let channel = connect_grpc_channel(&initial).await?;
    let mut client = AgentServiceClient::new(channel);
    client = client
        .max_decoding_message_size(GRPC_MESSAGE_LIMIT)
        .max_encoding_message_size(GRPC_MESSAGE_LIMIT);
    let mut io_client = client.clone();
    let (tx, rx) = mpsc::channel(32);
    let (io_tx, io_rx) = mpsc::channel::<IoFrame>(128);
    let mut request = Request::new(ReceiverStream::new(rx));
    let auth_header = format!("bearer {}", jwt);
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(auth_header.as_str())?,
    );
    let mut io_request = Request::new(ReceiverStream::new(io_rx));
    io_request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(auth_header.as_str())?,
    );
    let mut response_stream = client.session(request).await?.into_inner();
    let mut io_response_stream = io_client.io_stream(io_request).await?.into_inner();
    let report_interval = Duration::from_secs(initial.report_interval_seconds.max(1));
    let heartbeat_interval = Duration::from_secs(HEARTBEAT_INTERVAL_SECS);
    let terminal_sessions: TerminalSessionMap =
        Arc::new(Mutex::new(std::collections::HashMap::new()));
    let nat_sessions: NatSessionMap = Arc::new(Mutex::new(std::collections::HashMap::new()));

    // Heartbeat loop (server uses this to refresh `last_seen_at` and
    // detect disconnects quickly; metrics flow on a separate stream).
    let heartbeat_tx = tx.clone();
    tokio::spawn(async move {
        let mut tick = interval(heartbeat_interval);
        loop {
            tick.tick().await;
            let message = AgentMessage {
                payload: Some(Payload::Heartbeat(Heartbeat {
                    timestamp: now_unix_seconds() as i64,
                })),
            };
            if heartbeat_tx.send(message).await.is_err() {
                break;
            }
        }
    });

    // Send HostInfo once on connect, then send HostState every
    // report_interval. Per-collector failures do not abort the loop.
    let metrics_tx = tx.clone();
    let metrics_runtime = runtime.clone();
    tokio::spawn(async move {
        let cfg = metrics_runtime.config.lock().await.clone();
        if let Err(e) = send_host_info(&metrics_tx, &cfg).await {
            tracing::warn!("host_info send failed: {}", e);
        }
        let mut tick = interval(report_interval);
        loop {
            tick.tick().await;
            let cfg = metrics_runtime.config.lock().await.clone();
            if let Err(e) = send_host_state(&metrics_tx, &cfg).await {
                tracing::warn!("host_state send failed: {}", e);
                break;
            }
        }
    });

    // M6: report the agent's outbound IP so the server can apply
    // agent-bound DDNS configs immediately when it changes.
    let ip_report_tx = tx.clone();
    let ip_report_runtime = runtime.clone();
    let ip_report_interval = Duration::from_secs(initial.ip_report_interval_seconds.max(1));
    tokio::spawn(async move {
        let mut tick = interval(ip_report_interval);
        let mut last_ipv4: Option<String> = None;
        loop {
            tick.tick().await;
            let cfg = ip_report_runtime.config.lock().await.clone();
            if cfg.disable_send_query {
                continue;
            }
            match detect_primary_ipv4().await {
                Ok(Some(ipv4)) => {
                    let message = AgentMessage {
                        payload: Some(Payload::GeoIpReport(GeoIpReport {
                            ipv4: ipv4.clone(),
                            ipv6: String::new(),
                        })),
                    };
                    if ip_report_tx.send(message).await.is_err() {
                        break;
                    }
                    if last_ipv4.as_deref() != Some(ipv4.as_str()) {
                        tracing::debug!("reported primary ipv4 {}", ipv4);
                    }
                    last_ipv4 = Some(ipv4);
                }
                Ok(_) => {}
                Err(e) => tracing::debug!("primary ip detect failed: {}", e),
            }
        }
    });

    println!(
        "Agent {} connected (heartbeat {}s, report {}s)",
        initial.agent_id,
        HEARTBEAT_INTERVAL_SECS,
        initial.report_interval_seconds.max(1)
    );

    loop {
        tokio::select! {
            message = response_stream.message() => {
                let Some(message) = message.map_err(|e| anyhow::anyhow!("gRPC recv: {e}"))? else {
                    return Ok(SessionExit::StreamClosed);
                };
                match message.payload {
                    Some(xlstatus_proto_gen::xlstatus::v1::server_message::Payload::ForceDisconnect(_)) => {
                        tracing::info!("server requested force_disconnect, exiting");
                        return Ok(SessionExit::Revoked);
                    }
                    Some(xlstatus_proto_gen::xlstatus::v1::server_message::Payload::ConfigUpdate(update)) => {
                        if let Err(e) = apply_remote_config(&runtime, &update).await {
                            tracing::warn!("config update failed: {}", e);
                        } else {
                            tracing::info!("remote config update applied");
                        }
                    }
                    Some(xlstatus_proto_gen::xlstatus::v1::server_message::Payload::ForceUpdate(update)) => {
                        let cfg = runtime.config.lock().await.clone();
                        if cfg.disable_force_update {
                            tracing::warn!("force update ignored because disable_force_update=true");
                        } else if let Err(e) = record_force_update_request(&runtime, &update).await {
                            tracing::warn!("force update record failed: {}", e);
                        } else {
                            tracing::info!("force update request recorded for version {}", update.version);
                        }
                    }
                    Some(xlstatus_proto_gen::xlstatus::v1::server_message::Payload::Task(task)) => {
                        // M5: server just dispatched a task. Execute
                        // it inline and stream the result back over
                        // the same session.
                        let metrics_tx = tx.clone();
                        let task_id = task.task_id.clone();
                        let runtime = runtime.clone();
                        tokio::spawn(async move {
                            let result = run_server_task(runtime, task).await;
                            let msg = xlstatus_proto_gen::xlstatus::v1::AgentMessage {
                                payload: Some(xlstatus_proto_gen::xlstatus::v1::agent_message::Payload::TaskResult(result)),
                            };
                            if let Err(e) = metrics_tx.send(msg).await {
                                tracing::warn!("task_result send failed: {}", e);
                            } else {
                                tracing::info!("task {} completed", task_id);
                            }
                        });
                    }
                    _ => tracing::debug!("received server message: {:?}", message.payload),
                }
            }
            frame = io_response_stream.message() => {
                let Some(frame) = frame.map_err(|e| anyhow::anyhow!("gRPC io recv: {e}"))? else {
                    return Ok(SessionExit::StreamClosed);
                };
                handle_io_frame(
                    &initial.agent_id,
                    frame,
                    terminal_sessions.clone(),
                    nat_sessions.clone(),
                    runtime.clone(),
                    io_tx.clone(),
                ).await;
            }
        }
    }
}

enum SessionExit {
    /// Stream ended normally; the outer loop should reconnect.
    StreamClosed,
    /// Server asked us to leave; the outer loop should not reconnect.
    Revoked,
}

/// M2: exponential backoff with full jitter, capped at 60 s.
fn backoff_with_jitter(attempt: u32) -> Duration {
    let base_secs: u64 = 2u64.saturating_pow(attempt.min(6)).min(60);
    let jitter_ms: u64 = rand::random::<u64>() % 1000;
    Duration::from_secs(base_secs) + Duration::from_millis(jitter_ms)
}

const HEARTBEAT_INTERVAL_SECS: u64 = 15;
const REPORT_INTERVAL_SECS: u64 = 3;

async fn handle_io_frame(
    agent_id: &str,
    frame: IoFrame,
    terminal_sessions: TerminalSessionMap,
    nat_sessions: NatSessionMap,
    runtime: RuntimeContext,
    io_tx: tokio::sync::mpsc::Sender<IoFrame>,
) {
    match frame.payload {
        Some(io_frame::Payload::Data(data)) => {
            let stream_id = frame.stream_id.clone();
            let sequence = frame.sequence;
            let payload = data.data;
            let existing_terminal = { terminal_sessions.lock().await.get(&stream_id).cloned() };
            if let Some(session) = existing_terminal {
                handle_terminal_message(
                    agent_id,
                    &stream_id,
                    sequence,
                    session,
                    terminal_sessions,
                    runtime.clone(),
                    io_tx,
                    payload,
                )
                .await;
                return;
            }

            let existing_nat_tx = {
                nat_sessions
                    .lock()
                    .await
                    .get(&stream_id)
                    .map(|session| session.writer_tx.clone())
            };
            if let Some(writer_tx) = existing_nat_tx {
                if writer_tx.send(payload).await.is_err() {
                    close_nat_session(
                        agent_id,
                        &stream_id,
                        "NAT writer channel closed".to_string(),
                        sequence,
                        &nat_sessions,
                        &io_tx,
                        true,
                    )
                    .await;
                }
                return;
            }

            if let Ok(message) = serde_json::from_slice::<TerminalBridgeMessage>(&payload) {
                handle_new_terminal_session(
                    agent_id,
                    stream_id,
                    sequence,
                    message,
                    terminal_sessions,
                    runtime.clone(),
                    io_tx,
                )
                .await;
                return;
            }

            if let Ok(message) = serde_json::from_slice::<NatTunnelControlMessage>(&payload) {
                handle_nat_control_message(
                    agent_id,
                    stream_id,
                    sequence,
                    message,
                    nat_sessions,
                    runtime.clone(),
                    io_tx,
                )
                .await;
                return;
            }

            let _ = io_tx
                .send(io_error_frame(
                    &stream_id,
                    agent_id,
                    sequence,
                    "invalid_io_message",
                    "could not decode terminal or NAT control message",
                ))
                .await;
        }
        Some(io_frame::Payload::Close(close)) => {
            let has_terminal = terminal_sessions
                .lock()
                .await
                .contains_key(&frame.stream_id);
            if has_terminal {
                close_terminal_session(
                    agent_id,
                    &frame.stream_id,
                    close.reason,
                    frame.sequence,
                    &terminal_sessions,
                    &io_tx,
                )
                .await;
                return;
            }
            let has_nat = nat_sessions.lock().await.contains_key(&frame.stream_id);
            if has_nat {
                close_nat_session(
                    agent_id,
                    &frame.stream_id,
                    close.reason,
                    frame.sequence,
                    &nat_sessions,
                    &io_tx,
                    true,
                )
                .await;
            }
        }
        Some(io_frame::Payload::Error(_)) | None => {}
    }
}

async fn handle_new_terminal_session(
    agent_id: &str,
    stream_id: String,
    sequence: u64,
    message: TerminalBridgeMessage,
    terminal_sessions: TerminalSessionMap,
    runtime: RuntimeContext,
    io_tx: tokio::sync::mpsc::Sender<IoFrame>,
) {
    match message {
        TerminalBridgeMessage::Open { cols, rows } => {
            let cfg = runtime.config.lock().await.clone();
            if cfg.disable_command_execute {
                let _ = io_tx
                    .send(io_error_frame(
                        &stream_id,
                        agent_id,
                        sequence,
                        "command_execution_disabled",
                        "terminal access is disabled by agent policy",
                    ))
                    .await;
                return;
            }
            match executor::terminal::TerminalSession::new(stream_id.clone(), cols, rows).await {
                Ok(session) => {
                    let session = Arc::new(session);
                    terminal_sessions
                        .lock()
                        .await
                        .insert(stream_id.clone(), session.clone());
                    spawn_terminal_output_loop(
                        agent_id.to_string(),
                        stream_id,
                        sequence.saturating_add(1),
                        session,
                        io_tx.clone(),
                    );
                }
                Err(e) => {
                    let _ = io_tx
                        .send(io_error_frame(
                            &stream_id,
                            agent_id,
                            sequence,
                            "terminal_open_failed",
                            &e.to_string(),
                        ))
                        .await;
                }
            }
        }
        TerminalBridgeMessage::Input { .. }
        | TerminalBridgeMessage::Resize { .. }
        | TerminalBridgeMessage::Close { .. } => {
            let _ = io_tx
                .send(io_error_frame(
                    &stream_id,
                    agent_id,
                    sequence,
                    "terminal_not_open",
                    "terminal session is not open",
                ))
                .await;
        }
        TerminalBridgeMessage::Output { .. } | TerminalBridgeMessage::Error { .. } => {}
    }
}

async fn handle_terminal_message(
    agent_id: &str,
    stream_id: &str,
    sequence: u64,
    session: Arc<executor::terminal::TerminalSession>,
    terminal_sessions: TerminalSessionMap,
    runtime: RuntimeContext,
    io_tx: tokio::sync::mpsc::Sender<IoFrame>,
    payload: Vec<u8>,
) {
    let cfg = runtime.config.lock().await.clone();
    if cfg.disable_command_execute {
        let _ = io_tx
            .send(io_error_frame(
                stream_id,
                agent_id,
                sequence,
                "command_execution_disabled",
                "terminal access is disabled by agent policy",
            ))
            .await;
        return;
    }
    let Ok(message) = serde_json::from_slice::<TerminalBridgeMessage>(&payload) else {
        let _ = io_tx
            .send(io_error_frame(
                stream_id,
                agent_id,
                sequence,
                "invalid_terminal_message",
                "could not decode terminal control message",
            ))
            .await;
        return;
    };

    match message {
        TerminalBridgeMessage::Input { data } => {
            if let Err(e) = session.send_input(data.as_bytes()).await {
                let _ = io_tx
                    .send(io_error_frame(
                        stream_id,
                        agent_id,
                        sequence,
                        "terminal_input_failed",
                        &e.to_string(),
                    ))
                    .await;
            }
        }
        TerminalBridgeMessage::Resize { cols, rows } => {
            if let Err(e) = session.resize(cols, rows) {
                let _ = io_tx
                    .send(io_error_frame(
                        stream_id,
                        agent_id,
                        sequence,
                        "terminal_resize_failed",
                        &e.to_string(),
                    ))
                    .await;
            }
        }
        TerminalBridgeMessage::Close { reason } => {
            close_terminal_session(
                agent_id,
                stream_id,
                reason.unwrap_or_else(|| "terminal closed".to_string()),
                sequence,
                &terminal_sessions,
                &io_tx,
            )
            .await;
        }
        TerminalBridgeMessage::Open { .. }
        | TerminalBridgeMessage::Output { .. }
        | TerminalBridgeMessage::Error { .. } => {}
    }
}

async fn handle_nat_control_message(
    agent_id: &str,
    stream_id: String,
    sequence: u64,
    message: NatTunnelControlMessage,
    nat_sessions: NatSessionMap,
    runtime: RuntimeContext,
    io_tx: tokio::sync::mpsc::Sender<IoFrame>,
) {
    let cfg = runtime.config.lock().await.clone();
    if cfg.disable_nat {
        let _ = io_tx
            .send(io_error_frame(
                &stream_id,
                agent_id,
                sequence,
                "nat_disabled",
                "NAT is disabled by agent policy",
            ))
            .await;
        return;
    }
    match message {
        NatTunnelControlMessage::Open {
            local_host,
            local_port,
        } => {
            let local_addr_label = format_nat_target_label(&local_host, local_port);
            let connect_result = async {
                let addrs = resolve_agent_nat_target(&local_host, local_port).await?;
                connect_agent_nat_addrs(&addrs).await
            }
            .await;
            match connect_result {
                Ok(stream) => {
                    let (mut reader, mut writer) = stream.into_split();
                    let (writer_tx, mut writer_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
                    let reader_stream_id = stream_id.clone();
                    let reader_agent_id = agent_id.to_string();
                    let reader_io_tx = io_tx.clone();
                    let reader_nat_sessions = nat_sessions.clone();
                    let reader_task = tokio::spawn(async move {
                        let mut read_sequence = sequence.saturating_add(2);
                        let mut buf = [0u8; 8192];
                        loop {
                            match reader.read(&mut buf).await {
                                Ok(0) => {
                                    let _ = reader_io_tx
                                        .send(io_close_frame(
                                            &reader_stream_id,
                                            &reader_agent_id,
                                            read_sequence,
                                            "local NAT stream closed",
                                        ))
                                        .await;
                                    let _ =
                                        reader_nat_sessions.lock().await.remove(&reader_stream_id);
                                    break;
                                }
                                Ok(n) => {
                                    if reader_io_tx
                                        .send(io_raw_data_frame(
                                            &reader_stream_id,
                                            &reader_agent_id,
                                            read_sequence,
                                            buf[..n].to_vec(),
                                        ))
                                        .await
                                        .is_err()
                                    {
                                        let _ = reader_nat_sessions
                                            .lock()
                                            .await
                                            .remove(&reader_stream_id);
                                        break;
                                    }
                                    read_sequence = read_sequence.saturating_add(1);
                                }
                                Err(e) => {
                                    let _ = reader_io_tx
                                        .send(io_error_frame(
                                            &reader_stream_id,
                                            &reader_agent_id,
                                            read_sequence,
                                            "nat_read_failed",
                                            &e.to_string(),
                                        ))
                                        .await;
                                    let _ =
                                        reader_nat_sessions.lock().await.remove(&reader_stream_id);
                                    break;
                                }
                            }
                        }
                    });

                    tokio::spawn({
                        let stream_id = stream_id.clone();
                        let writer_agent_id = agent_id.to_string();
                        let writer_io_tx = io_tx.clone();
                        async move {
                            while let Some(chunk) = writer_rx.recv().await {
                                if let Err(e) = writer.write_all(&chunk).await {
                                    let _ = writer_io_tx
                                        .send(io_error_frame(
                                            &stream_id,
                                            &writer_agent_id,
                                            sequence.saturating_add(1),
                                            "nat_write_failed",
                                            &e.to_string(),
                                        ))
                                        .await;
                                    break;
                                }
                            }
                        }
                    });

                    nat_sessions.lock().await.insert(
                        stream_id.clone(),
                        NatSocketSession {
                            writer_tx,
                            reader_task,
                        },
                    );

                    let _ = io_tx
                        .send(io_json_data_frame(
                            &stream_id,
                            agent_id,
                            sequence.saturating_add(1),
                            &NatTunnelControlMessage::Ready,
                        ))
                        .await;
                }
                Err(e) => {
                    let _ = io_tx
                        .send(io_error_frame(
                            &stream_id,
                            agent_id,
                            sequence,
                            "nat_open_failed",
                            &format!("failed to connect to {}: {}", local_addr_label, e),
                        ))
                        .await;
                }
            }
        }
        NatTunnelControlMessage::Ready => {}
    }
}

fn spawn_terminal_output_loop(
    agent_id: String,
    stream_id: String,
    start_sequence: u64,
    session: Arc<executor::terminal::TerminalSession>,
    io_tx: tokio::sync::mpsc::Sender<IoFrame>,
) {
    tokio::spawn(async move {
        let mut sequence = start_sequence;
        while let Some(chunk) = session.recv_output().await {
            let payload = TerminalBridgeMessage::Output {
                data: String::from_utf8_lossy(&chunk).to_string(),
            };
            if io_tx
                .send(io_json_data_frame(
                    &stream_id, &agent_id, sequence, &payload,
                ))
                .await
                .is_err()
            {
                break;
            }
            sequence = sequence.saturating_add(1);
        }
        let _ = io_tx
            .send(io_json_data_frame(
                &stream_id,
                &agent_id,
                sequence,
                &TerminalBridgeMessage::Close {
                    reason: Some("terminal exited".to_string()),
                },
            ))
            .await;
    });
}

async fn close_terminal_session(
    agent_id: &str,
    stream_id: &str,
    reason: String,
    sequence: u64,
    sessions: &Arc<
        Mutex<std::collections::HashMap<String, Arc<executor::terminal::TerminalSession>>>,
    >,
    io_tx: &tokio::sync::mpsc::Sender<IoFrame>,
) {
    let session = sessions.lock().await.remove(stream_id);
    if let Some(session) = session {
        session.close();
    }
    let _ = io_tx
        .send(io_data_frame(
            stream_id,
            agent_id,
            sequence,
            &TerminalBridgeMessage::Close {
                reason: Some(reason),
            },
        ))
        .await;
}

fn io_data_frame(
    stream_id: &str,
    agent_id: &str,
    sequence: u64,
    message: &TerminalBridgeMessage,
) -> IoFrame {
    io_json_data_frame(stream_id, agent_id, sequence, message)
}

fn io_json_data_frame<T: serde::Serialize>(
    stream_id: &str,
    agent_id: &str,
    sequence: u64,
    message: &T,
) -> IoFrame {
    IoFrame {
        stream_id: stream_id.to_string(),
        sequence,
        agent_id: agent_id.to_string(),
        payload: Some(io_frame::Payload::Data(IoData {
            data: serde_json::to_vec(message).unwrap_or_default(),
        })),
    }
}

fn io_raw_data_frame(stream_id: &str, agent_id: &str, sequence: u64, data: Vec<u8>) -> IoFrame {
    IoFrame {
        stream_id: stream_id.to_string(),
        sequence,
        agent_id: agent_id.to_string(),
        payload: Some(io_frame::Payload::Data(IoData { data })),
    }
}

fn io_close_frame(stream_id: &str, agent_id: &str, sequence: u64, reason: &str) -> IoFrame {
    IoFrame {
        stream_id: stream_id.to_string(),
        sequence,
        agent_id: agent_id.to_string(),
        payload: Some(io_frame::Payload::Close(
            xlstatus_proto_gen::xlstatus::v1::IoClose {
                reason: reason.to_string(),
            },
        )),
    }
}

fn io_error_frame(
    stream_id: &str,
    agent_id: &str,
    sequence: u64,
    code: &str,
    message: &str,
) -> IoFrame {
    IoFrame {
        stream_id: stream_id.to_string(),
        sequence,
        agent_id: agent_id.to_string(),
        payload: Some(io_frame::Payload::Error(IoError {
            code: code.to_string(),
            message: message.to_string(),
        })),
    }
}

async fn close_nat_session(
    agent_id: &str,
    stream_id: &str,
    reason: String,
    sequence: u64,
    nat_sessions: &NatSessionMap,
    io_tx: &tokio::sync::mpsc::Sender<IoFrame>,
    echo_close: bool,
) {
    let session = nat_sessions.lock().await.remove(stream_id);
    if let Some(session) = session {
        session.reader_task.abort();
    }
    if echo_close {
        let _ = io_tx
            .send(io_close_frame(stream_id, agent_id, sequence, &reason))
            .await;
    }
}

async fn send_host_info(
    tx: &tokio::sync::mpsc::Sender<AgentMessage>,
    config: &AgentConfig,
) -> anyhow::Result<()> {
    use xlstatus_proto_gen::xlstatus::v1::{DiskInfo, HostInfo as HostInfoMsg, HostInfoUpdate};
    let info = collector::collect_host_info();
    let hostname = info.hostname;
    let os = info.os;
    let os_version = info.os_version;
    let kernel_version = info.kernel_version;
    let arch = info.arch;
    let cpu_cores = info.cpu_cores;
    let total_memory = info.total_memory;
    let total_swap = info.total_swap;
    let agent_version = info.agent_version;
    let disks: Vec<DiskInfo> = info
        .disks
        .into_iter()
        .map(|d| DiskInfo {
            device: d.device,
            mount_point: d.mount_point,
            fs_type: d.fs_type,
            total: d.total,
        })
        .collect();
    let msg = AgentMessage {
        payload: Some(Payload::HostInfoUpdate(HostInfoUpdate {
            host_info: Some(HostInfoMsg {
                hostname,
                os: os.clone(),
                platform: os,
                platform_version: format!(
                    "{} | agent={} | report={}s | ip_report={}s | auto_update_disabled={} | force_update_disabled={} | command_disabled={} | nat_disabled={} | send_query_disabled={}",
                    os_version,
                    agent_version,
                    config.report_interval_seconds,
                    config.ip_report_interval_seconds,
                    config.disable_auto_update,
                    config.disable_force_update,
                    config.disable_command_execute,
                    config.disable_nat,
                    config.disable_send_query
                ),
                kernel_version,
                arch,
                cpu_cores: cpu_cores as u32,
                total_memory,
                total_swap,
                disks,
            }),
        })),
    };
    tx.send(msg)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))
}

async fn send_host_state(
    tx: &tokio::sync::mpsc::Sender<AgentMessage>,
    _config: &AgentConfig,
) -> anyhow::Result<()> {
    use prost_types::Timestamp;
    use xlstatus_proto_gen::xlstatus::v1::{
        DiskState, HostState as HostStateMsg, NetStat, TempSensor,
    };
    let s = collector::collect_host_state();
    let disks: Vec<DiskState> = s
        .disks
        .into_iter()
        .map(|d| DiskState {
            mount_point: d.mount_point,
            used: d.used,
            total: d.total,
        })
        .collect();
    let net_io: Vec<NetStat> = s
        .network_interfaces
        .into_iter()
        .map(|n| NetStat {
            interface: n.name,
            bytes_sent: n.tx_bytes,
            bytes_recv: n.rx_bytes,
        })
        .collect();
    let temperatures: Vec<TempSensor> = s
        .temperatures
        .into_iter()
        .enumerate()
        .map(|(i, t)| TempSensor {
            label: format!("sensor{}", i),
            temperature: t as f64,
        })
        .collect();
    let now = now_unix_seconds() as i64;
    let msg = AgentMessage {
        payload: Some(Payload::HostState(HostStateMsg {
            cpu_percent: s.cpu_percent as f64,
            memory_used: s.memory_used,
            memory_total: s.memory_total,
            swap_used: s.swap_used,
            swap_total: s.swap_total,
            disks,
            net_io,
            load_1: s.load1,
            load_5: s.load5,
            load_15: s.load15,
            tcp_connections: s.tcp_connections as u32,
            udp_connections: s.udp_connections as u32,
            process_count: s.process_count as u32,
            temperatures,
            gpus: vec![],
            timestamp: Some(Timestamp {
                seconds: now,
                nanos: 0,
            }),
            uptime_seconds: s.uptime_seconds,
        })),
    };
    tx.send(msg)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))
}

async fn fetch_agent_jwt(config: &AgentConfig) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let challenge = client
        .post(format!("{}/api/v1/agents/jwt/challenge", config.server))
        .json(&serde_json::json!({ "agent_id": config.agent_id }))
        .send()
        .await?
        .error_for_status()?
        .json::<ApiResponse<JwtChallengeResponse>>()
        .await?;

    if !challenge.success {
        anyhow::bail!(
            "jwt challenge failed: {}",
            challenge
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }

    let nonce = challenge
        .data
        .map(|data| data.nonce)
        .ok_or_else(|| anyhow::anyhow!("jwt challenge response did not include data"))?;
    let signing_key = signing_key_from_config(config)?;
    let signature = signing_key.sign(nonce.as_bytes());

    let response = client
        .post(format!("{}/api/v1/agents/jwt", config.server))
        .json(&serde_json::json!({
            "agent_id": config.agent_id,
            "nonce": nonce,
            "signature": hex::encode(signature.to_bytes()),
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<ApiResponse<JwtResponse>>()
        .await?;

    if !response.success {
        anyhow::bail!(
            "jwt request failed: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }

    response
        .data
        .map(|data| data.jwt)
        .ok_or_else(|| anyhow::anyhow!("jwt response did not include data"))
}

fn generate_signing_key() -> SigningKey {
    let mut private_key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut private_key);
    SigningKey::from_bytes(&private_key)
}

fn signing_key_from_config(config: &AgentConfig) -> anyhow::Result<SigningKey> {
    if config.private_key.is_empty() {
        anyhow::bail!("agent config does not include a private_key; re-run enroll");
    }
    let private_key = hex::decode(&config.private_key)?;
    let private_key: [u8; 32] = private_key
        .try_into()
        .map_err(|_| anyhow::anyhow!("agent private_key must be 32 bytes"))?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let expected_public_key = hex::encode(signing_key.verifying_key().to_bytes());
    if expected_public_key != config.public_key {
        anyhow::bail!("agent private_key does not match public_key");
    }
    Ok(signing_key)
}

fn write_secure_config(config_path: &PathBuf, contents: &[u8]) -> anyhow::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        let mut file = options.open(config_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::set_permissions(config_path, std::fs::Permissions::from_mode(0o600))?;
    }

    #[cfg(not(unix))]
    {
        let mut file = options.open(config_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }

    Ok(())
}

fn infer_grpc_url(server: &str) -> String {
    if let Some(rest) = server.strip_prefix("http://") {
        if let Some(host) = rest.strip_suffix(":8080") {
            return format!("http://{}:50051", host);
        }
    }
    server.to_string()
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn detect_primary_ipv4() -> anyhow::Result<Option<String>> {
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect("1.1.1.1:80").await?;
    let ip = socket.local_addr()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        return Ok(None);
    }
    Ok(Some(ip.to_string()))
}

/// M5: execute a ServerTask on the agent and produce a TaskResult.
async fn run_server_task(
    runtime: RuntimeContext,
    task: xlstatus_proto_gen::xlstatus::v1::ServerTask,
) -> xlstatus_proto_gen::xlstatus::v1::TaskResult {
    use xlstatus_proto_gen::xlstatus::v1::server_task::Spec;
    use xlstatus_proto_gen::xlstatus::v1::TaskOutcome;
    use xlstatus_proto_gen::xlstatus::v1::TaskResult;
    let task_id = task.task_id.clone();
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut result = TaskResult {
        task_id: task_id.clone(),
        agent_id: runtime.config.lock().await.agent_id.clone(),
        status: TaskOutcome::Unspecified as i32,
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
        error: String::new(),
        started_at: started,
        finished_at: 0,
    };
    let finished = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };
    let cfg = runtime.config.lock().await.clone();
    match task.spec {
        Some(Spec::ShellCommand(shell)) => {
            if cfg.disable_command_execute {
                result.status = TaskOutcome::Failure as i32;
                result.exit_code = 126;
                result.error = "command execution disabled by agent policy".to_string();
                result.finished_at = finished();
                return result;
            }
            let timeout = shell_task_timeout_seconds(shell.timeout_seconds);
            let max_output_bytes = shell_task_output_max_bytes(shell.max_output_bytes);
            let working_dir = non_empty_string(&shell.working_dir);
            let env = shell.env.into_iter().collect::<Vec<(String, String)>>();
            match executor::shell::execute_shell_command(
                &shell.command,
                working_dir.as_deref(),
                &env,
                timeout as u32,
                max_output_bytes,
            )
            .await
            {
                Ok(out) => {
                    result.exit_code = out.exit_code;
                    result.stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    result.stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    if out.output_truncated {
                        result.error = format!("output truncated at {} bytes", max_output_bytes);
                    }
                    result.status = if out.exit_code == 0 {
                        TaskOutcome::Success as i32
                    } else {
                        TaskOutcome::Failure as i32
                    };
                }
                Err(e) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.error = e.to_string();
                }
            }
        }
        Some(Spec::HttpGet(spec)) => {
            if reject_probe_task_when_disabled(&cfg, &mut result, finished()) {
                return result;
            }
            match run_http_probe_task(spec).await {
                Ok(output) => {
                    result.status = TaskOutcome::Success as i32;
                    result.stdout = output;
                }
                Err(e) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.exit_code = 1;
                    result.error = e.to_string();
                }
            }
        }
        Some(Spec::TcpPing(spec)) => {
            if reject_probe_task_when_disabled(&cfg, &mut result, finished()) {
                return result;
            }
            match run_tcp_probe_task(spec).await {
                Ok(output) => {
                    result.status = TaskOutcome::Success as i32;
                    result.stdout = output;
                }
                Err(e) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.exit_code = 1;
                    result.error = e.to_string();
                }
            }
        }
        Some(Spec::IcmpPing(spec)) => {
            if reject_probe_task_when_disabled(&cfg, &mut result, finished()) {
                return result;
            }
            match run_icmp_probe_task(spec).await {
                Ok(output) => {
                    result.status = TaskOutcome::Success as i32;
                    result.stdout = output;
                }
                Err(e) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.exit_code = 1;
                    result.error = e.to_string();
                }
            }
        }
        Some(Spec::FileList(spec)) => {
            if cfg.disable_command_execute {
                result.status = TaskOutcome::Failure as i32;
                result.exit_code = 126;
                result.error = "file access disabled by agent policy".to_string();
                result.finished_at = finished();
                return result;
            }
            match executor::files::list_files(&spec.path, &cfg.file_allowed_roots).await {
                Ok(entries) => {
                    result.status = TaskOutcome::Success as i32;
                    let entries = entries
                        .into_iter()
                        .map(|entry| {
                            serde_json::json!({
                                "name": entry.name,
                                "file_type": match entry.file_type {
                                    executor::files::FileType::File => "file",
                                    executor::files::FileType::Dir => "dir",
                                    executor::files::FileType::Symlink => "symlink",
                                },
                                "size": entry.size,
                                "mode": entry.mode,
                                "modified_at": entry.modified_at,
                                "symlink_target": entry.symlink_target,
                            })
                        })
                        .collect::<Vec<_>>();
                    result.stdout = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".into());
                }
                Err(e) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.exit_code = 1;
                    result.error = e.to_string();
                }
            }
        }
        Some(Spec::FileRead(spec)) => {
            if cfg.disable_command_execute {
                result.status = TaskOutcome::Failure as i32;
                result.exit_code = 126;
                result.error = "file access disabled by agent policy".to_string();
                result.finished_at = finished();
                return result;
            }
            match executor::files::read_file(
                &spec.path,
                spec.offset,
                spec.length,
                &cfg.file_allowed_roots,
            )
            .await
            {
                Ok(data) => {
                    result.status = TaskOutcome::Success as i32;
                    result.stdout = base64_encode(&data);
                }
                Err(e) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.exit_code = 1;
                    result.error = e.to_string();
                }
            }
        }
        Some(Spec::FileWrite(spec)) => {
            if cfg.disable_command_execute {
                result.status = TaskOutcome::Failure as i32;
                result.exit_code = 126;
                result.error = "file access disabled by agent policy".to_string();
                result.finished_at = finished();
                return result;
            }
            match executor::files::write_file(
                &spec.path,
                &spec.data,
                if spec.mode == 0 {
                    None
                } else {
                    Some(spec.mode)
                },
                spec.create_dirs,
                &cfg.file_allowed_roots,
            )
            .await
            {
                Ok(written) => {
                    result.status = TaskOutcome::Success as i32;
                    result.stdout = written.to_string();
                }
                Err(e) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.exit_code = 1;
                    result.error = e.to_string();
                }
            }
        }
        Some(Spec::FileDelete(spec)) => {
            if cfg.disable_command_execute {
                result.status = TaskOutcome::Failure as i32;
                result.exit_code = 126;
                result.error = "file access disabled by agent policy".to_string();
                result.finished_at = finished();
                return result;
            }
            match executor::files::delete_path(&spec.path, spec.recursive, &cfg.file_allowed_roots)
                .await
            {
                Ok(()) => {
                    result.status = TaskOutcome::Success as i32;
                    result.stdout = "deleted".to_string();
                }
                Err(e) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.exit_code = 1;
                    result.error = e.to_string();
                }
            }
        }
        Some(_) => {
            result.status = TaskOutcome::Failure as i32;
            result.error = "task type not yet implemented in agent".into();
        }
        None => {
            result.status = TaskOutcome::Failure as i32;
            result.error = "task with no spec".into();
        }
    }
    result.finished_at = finished();
    result
}

async fn run_http_probe_task(
    spec: xlstatus_proto_gen::xlstatus::v1::HttpGetTask,
) -> anyhow::Result<String> {
    let start = std::time::Instant::now();
    let timeout = task_timeout_seconds(spec.timeout_seconds);
    let validated = validate_agent_probe_url_resolved(&spec.url, "Agent HTTP probe target").await?;
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in spec.headers {
        headers.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes())?,
            reqwest::header::HeaderValue::from_str(&value)?,
        );
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout))
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(!spec.verify_tls)
        .resolve_to_addrs(&validated.host, &validated.addrs)
        .build()?;

    let output = match client
        .get(validated.url.clone())
        .headers(headers)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            probe_output_json(
                status.is_success(),
                Some(start.elapsed().as_millis() as i32),
                Some(status.as_u16() as i32),
                if status.is_success() {
                    None
                } else {
                    Some(format!("HTTP {}", status.as_u16()))
                },
            )
        }
        Err(e) => probe_output_json(
            false,
            Some(start.elapsed().as_millis() as i32),
            None,
            Some(e.to_string()),
        ),
    };
    Ok(output)
}

async fn run_tcp_probe_task(
    spec: xlstatus_proto_gen::xlstatus::v1::TcpPingTask,
) -> anyhow::Result<String> {
    let start = std::time::Instant::now();
    let timeout = task_timeout_seconds(spec.timeout_seconds);
    let port = agent_probe_port(spec.port, "Agent TCP probe target")?;
    let addrs = resolve_agent_probe_host(&spec.host, port, "Agent TCP probe target").await?;
    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout),
        connect_agent_probe_addrs(&addrs),
    )
    .await
    {
        Ok(Ok(())) => probe_output_json(true, Some(start.elapsed().as_millis() as i32), None, None),
        Ok(Err(e)) => probe_output_json(
            false,
            Some(start.elapsed().as_millis() as i32),
            None,
            Some(e.to_string()),
        ),
        Err(_) => probe_output_json(
            false,
            Some(start.elapsed().as_millis() as i32),
            None,
            Some("connection timeout".to_string()),
        ),
    };
    Ok(output)
}

async fn run_icmp_probe_task(
    spec: xlstatus_proto_gen::xlstatus::v1::IcmpPingTask,
) -> anyhow::Result<String> {
    let start = std::time::Instant::now();
    let timeout = task_timeout_seconds(spec.timeout_seconds);
    let addrs = resolve_agent_probe_host(&spec.host, 0, "Agent ICMP probe target").await?;
    let ping_target = agent_ping_target(&addrs, "Agent ICMP probe target")?;
    let count = spec.count.clamp(1, 10).to_string();
    let timeout_text = timeout.to_string();
    let mut command = if cfg!(target_os = "windows") {
        let mut cmd = tokio::process::Command::new("ping");
        cmd.args([
            "-n",
            &count,
            "-w",
            &(timeout * 1000).to_string(),
            &ping_target,
        ]);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("ping");
        cmd.args(["-c", &count, "-W", &timeout_text, &ping_target]);
        cmd
    };
    command.kill_on_drop(true);

    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(
            timeout.saturating_add(AGENT_PING_PROCESS_TIMEOUT_GRACE_SECONDS),
        ),
        command.output(),
    )
    .await
    {
        Ok(Ok(output)) => {
            let latency_ms = start.elapsed().as_millis() as i32;
            if output.status.success() {
                let stdout = bounded_ping_output_text(&output.stdout);
                probe_output_json(
                    true,
                    Some(parse_ping_latency(stdout.as_str()).unwrap_or(latency_ms)),
                    None,
                    None,
                )
            } else {
                let message = ping_failure_message(&output.stderr, &output.stdout);
                probe_output_json(false, Some(latency_ms), None, Some(message))
            }
        }
        Ok(Err(e)) => probe_output_json(
            false,
            Some(start.elapsed().as_millis() as i32),
            None,
            Some(e.to_string()),
        ),
        Err(_) => probe_output_json(
            false,
            Some(start.elapsed().as_millis() as i32),
            None,
            Some("ping timeout".to_string()),
        ),
    };
    Ok(output)
}

fn probe_output_json(
    success: bool,
    latency_ms: Option<i32>,
    status_code: Option<i32>,
    error: Option<String>,
) -> String {
    serde_json::json!({
        "success": success,
        "latency_ms": latency_ms,
        "status_code": status_code,
        "error": error,
        "cert_fingerprint": null,
        "cert_not_after": null,
    })
    .to_string()
}

fn task_timeout_seconds(value: u32) -> u64 {
    if value == 0 {
        AGENT_PROBE_DEFAULT_TIMEOUT_SECONDS
    } else {
        u64::from(value).clamp(
            AGENT_PROBE_MIN_TIMEOUT_SECONDS,
            AGENT_PROBE_MAX_TIMEOUT_SECONDS,
        )
    }
}

fn shell_task_timeout_seconds(value: u32) -> u64 {
    if value == 0 {
        AGENT_SHELL_DEFAULT_TIMEOUT_SECONDS
    } else {
        u64::from(value).clamp(
            AGENT_SHELL_MIN_TIMEOUT_SECONDS,
            AGENT_SHELL_MAX_TIMEOUT_SECONDS,
        )
    }
}

fn shell_task_output_max_bytes(value: u64) -> u64 {
    if value == 0 {
        AGENT_SHELL_DEFAULT_OUTPUT_MAX_BYTES
    } else {
        value.min(AGENT_SHELL_OUTPUT_MAX_BYTES)
    }
}

fn ping_failure_message(stderr: &[u8], stdout: &[u8]) -> String {
    let stderr = bounded_ping_output_text(stderr);
    let stdout = bounded_ping_output_text(stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    if detail.is_empty() {
        "Ping failed".to_string()
    } else {
        format!("Ping failed: {detail}")
    }
}

fn bounded_ping_output_text(bytes: &[u8]) -> String {
    if bytes.len() <= AGENT_PING_OUTPUT_MAX_BYTES {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut end = AGENT_PING_OUTPUT_MAX_BYTES;
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    let mut text = String::from_utf8_lossy(&bytes[..end]).into_owned();
    text.push_str("... [truncated]");
    text
}

fn reject_probe_task_when_disabled(
    config: &AgentConfig,
    result: &mut xlstatus_proto_gen::xlstatus::v1::TaskResult,
    finished_at: i64,
) -> bool {
    if !config.disable_send_query {
        return false;
    }
    result.status = xlstatus_proto_gen::xlstatus::v1::TaskOutcome::Failure as i32;
    result.exit_code = 126;
    result.error = "probe task disabled by agent policy".to_string();
    result.finished_at = finished_at;
    true
}

async fn resolve_agent_nat_target(host: &str, port: u16) -> anyhow::Result<Vec<SocketAddr>> {
    resolve_agent_nat_target_with_policy(host, port, agent_private_nat_targets_allowed()).await
}

async fn resolve_agent_nat_target_with_policy(
    host: &str,
    port: u16,
    allow_private: bool,
) -> anyhow::Result<Vec<SocketAddr>> {
    let host = host.trim();
    if host.is_empty() {
        anyhow::bail!("Agent NAT target host must not be empty");
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        ensure_agent_nat_ip_allowed(ip, allow_private)?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    let mut addrs = lookup_host((host, port))
        .await
        .with_context(|| format!("failed to resolve Agent NAT target host '{host}'"))?;
    let mut resolved = Vec::new();
    for addr in &mut addrs {
        ensure_agent_nat_ip_allowed(addr.ip(), allow_private)?;
        resolved.push(addr);
    }
    if resolved.is_empty() {
        anyhow::bail!("Agent NAT target host '{host}' did not resolve to any address");
    }
    Ok(resolved)
}

fn ensure_agent_nat_ip_allowed(ip: IpAddr, allow_private: bool) -> anyhow::Result<()> {
    if allow_private || ip.is_loopback() {
        return Ok(());
    }
    anyhow::bail!("Agent NAT target resolves to disallowed non-loopback address {ip}");
}

async fn connect_agent_nat_addrs(addrs: &[SocketAddr]) -> anyhow::Result<TcpStream> {
    let mut last_error = None;
    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                last_error = Some(anyhow::anyhow!("failed to connect to {addr}: {e}"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no Agent NAT target address resolved")))
}

fn agent_private_nat_targets_allowed() -> bool {
    [
        "XLSTATUS_AGENT_ALLOW_PRIVATE_NAT_TARGETS",
        "XLSTATUS_ALLOW_PRIVATE_NAT_TARGETS",
        "XLSTATUS_ALLOW_PRIVATE_OUTBOUND",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    })
}

fn format_nat_target_label(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[derive(Debug, Clone)]
struct ValidatedAgentProbeUrl {
    url: reqwest::Url,
    host: String,
    addrs: Vec<SocketAddr>,
}

async fn validate_agent_probe_url_resolved(
    url: &str,
    purpose: &str,
) -> anyhow::Result<ValidatedAgentProbeUrl> {
    validate_agent_probe_url_resolved_with_policy(url, purpose, agent_private_probes_allowed())
        .await
}

async fn validate_agent_probe_url_resolved_with_policy(
    url: &str,
    purpose: &str,
    allow_private: bool,
) -> anyhow::Result<ValidatedAgentProbeUrl> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("{purpose} URL is invalid"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => anyhow::bail!("{purpose} URL scheme '{scheme}' is not allowed"),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!("{purpose} URL must not include credentials");
    }

    let host = parsed
        .host_str()
        .with_context(|| format!("{purpose} URL must include a host"))?
        .to_string();
    let port = parsed
        .port_or_known_default()
        .with_context(|| format!("{purpose} URL must include a port or known scheme"))?;
    let addrs = resolve_agent_probe_host_with_policy(&host, port, purpose, allow_private).await?;

    Ok(ValidatedAgentProbeUrl {
        url: parsed,
        host,
        addrs,
    })
}

async fn resolve_agent_probe_host(
    host: &str,
    port: u16,
    purpose: &str,
) -> anyhow::Result<Vec<SocketAddr>> {
    resolve_agent_probe_host_with_policy(host, port, purpose, agent_private_probes_allowed()).await
}

async fn resolve_agent_probe_host_with_policy(
    host: &str,
    port: u16,
    purpose: &str,
    allow_private: bool,
) -> anyhow::Result<Vec<SocketAddr>> {
    let host = host.trim();
    if host.is_empty() {
        anyhow::bail!("{purpose} host must not be empty");
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        ensure_agent_probe_ip_allowed(ip, purpose, allow_private)?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    let mut addrs = lookup_host((host, port))
        .await
        .with_context(|| format!("failed to resolve {purpose} host '{host}'"))?;
    let mut resolved = Vec::new();
    for addr in &mut addrs {
        ensure_agent_probe_ip_allowed(addr.ip(), purpose, allow_private)?;
        resolved.push(addr);
    }
    if resolved.is_empty() {
        anyhow::bail!("{purpose} host '{host}' did not resolve to any address");
    }
    Ok(resolved)
}

fn ensure_agent_probe_ip_allowed(
    ip: IpAddr,
    purpose: &str,
    allow_private: bool,
) -> anyhow::Result<()> {
    if !allow_private && is_agent_blocked_ip(ip) {
        anyhow::bail!("{purpose} resolves to disallowed private address {ip}");
    }
    Ok(())
}

fn agent_probe_port(value: u32, purpose: &str) -> anyhow::Result<u16> {
    let port =
        u16::try_from(value).with_context(|| format!("{purpose} port is out of range: {value}"))?;
    if port == 0 {
        anyhow::bail!("{purpose} port must be greater than 0");
    }
    Ok(port)
}

async fn connect_agent_probe_addrs(addrs: &[SocketAddr]) -> anyhow::Result<()> {
    let mut last_error = None;
    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_error = Some(anyhow::anyhow!("failed to connect to {addr}: {e}"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no address resolved")))
}

fn agent_ping_target(addrs: &[SocketAddr], purpose: &str) -> anyhow::Result<String> {
    let addr = addrs
        .first()
        .with_context(|| format!("{purpose} did not resolve to any address"))?;
    Ok(addr.ip().to_string())
}

fn agent_private_probes_allowed() -> bool {
    [
        "XLSTATUS_AGENT_ALLOW_PRIVATE_PROBES",
        "XLSTATUS_ALLOW_PRIVATE_OUTBOUND",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    })
}

fn is_agent_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_agent_blocked_ipv4(ip),
        IpAddr::V6(ip) => is_agent_blocked_ipv6(ip),
    }
}

fn is_agent_blocked_ipv4(ip: Ipv4Addr) -> bool {
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

fn is_agent_blocked_ipv6(ip: Ipv6Addr) -> bool {
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
            .map(is_agent_blocked_ipv4)
            .unwrap_or(false)
}

fn parse_ping_latency(output: &str) -> Option<i32> {
    for line in output.lines() {
        if line.contains("min/avg/max") || line.contains("rtt min/avg/max") {
            if let Some(stats_part) = line.split('=').nth(1) {
                let values: Vec<&str> = stats_part.trim().split('/').collect();
                if values.len() >= 2 {
                    if let Ok(avg) = values[1].trim().parse::<f64>() {
                        return Some(avg as i32);
                    }
                }
            }
        }
    }
    None
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

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

async fn apply_remote_config(
    runtime: &RuntimeContext,
    update: &ConfigUpdate,
) -> anyhow::Result<()> {
    if update.config_yaml.len() > REMOTE_CONFIG_MAX_BYTES {
        anyhow::bail!("remote config payload exceeds {REMOTE_CONFIG_MAX_BYTES} bytes");
    }
    let patch: serde_json::Value = serde_json::from_slice(&update.config_yaml)
        .map_err(|e| anyhow::anyhow!("invalid remote config payload: {}", e))?;
    let object = patch
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("remote config patch must be an object"))?;
    validate_remote_config_fields(object)?;
    let mut guard = runtime.config.lock().await;
    let mut current = guard.clone();

    if let Some(value) = object.get("server") {
        current.server = validate_remote_config_url(value, "server")?;
    }
    if let Some(value) = object.get("grpc_server") {
        current.grpc_server = validate_remote_config_url(value, "grpc_server")?;
    }
    if let Some(value) = object.get("grpc_tls_ca_path") {
        current.grpc_tls_ca_path = validate_optional_remote_config_path(value, "grpc_tls_ca_path")?;
    }
    if let Some(value) = object.get("grpc_tls_domain_name") {
        current.grpc_tls_domain_name =
            validate_optional_remote_config_text(value, "grpc_tls_domain_name", 253)?;
    }
    if let Some(value) = object.get("grpc_tls_client_cert_path") {
        current.grpc_tls_client_cert_path =
            validate_optional_remote_config_path(value, "grpc_tls_client_cert_path")?;
    }
    if let Some(value) = object.get("grpc_tls_client_key_path") {
        current.grpc_tls_client_key_path =
            validate_optional_remote_config_path(value, "grpc_tls_client_key_path")?;
    }
    if let Some(value) = object.get("name") {
        current.name = validate_remote_config_name(value)?;
    }
    if let Some(value) = object.get("report_interval_seconds") {
        current.report_interval_seconds =
            validate_remote_config_interval(value, "report_interval_seconds")?;
    }
    if let Some(value) = object.get("ip_report_interval_seconds") {
        current.ip_report_interval_seconds =
            validate_remote_config_interval(value, "ip_report_interval_seconds")?;
    }
    if let Some(value) = object.get("disable_auto_update") {
        current.disable_auto_update = validate_remote_config_bool(value, "disable_auto_update")?;
    }
    if let Some(value) = object.get("disable_force_update") {
        current.disable_force_update = validate_remote_config_bool(value, "disable_force_update")?;
    }
    if let Some(value) = object.get("disable_command_execute") {
        current.disable_command_execute =
            validate_remote_config_bool(value, "disable_command_execute")?;
    }
    if let Some(value) = object.get("disable_nat") {
        current.disable_nat = validate_remote_config_bool(value, "disable_nat")?;
    }
    if let Some(value) = object.get("disable_send_query") {
        current.disable_send_query = validate_remote_config_bool(value, "disable_send_query")?;
    }
    if let Some(value) = object.get("file_allowed_roots") {
        current.file_allowed_roots = validate_remote_config_roots(value)?;
    }
    current.private_key = read_private_key(&runtime.config_path)?;
    persist_agent_config(&runtime.config_path, &current)?;
    *guard = current;
    Ok(())
}

fn validate_remote_config_fields(
    object: &serde_json::Map<String, serde_json::Value>,
) -> anyhow::Result<()> {
    for key in object.keys() {
        match key.as_str() {
            "server"
            | "grpc_server"
            | "grpc_tls_ca_path"
            | "grpc_tls_domain_name"
            | "grpc_tls_client_cert_path"
            | "grpc_tls_client_key_path"
            | "name"
            | "report_interval_seconds"
            | "ip_report_interval_seconds"
            | "disable_auto_update"
            | "disable_force_update"
            | "disable_command_execute"
            | "disable_nat"
            | "disable_send_query"
            | "file_allowed_roots" => {}
            "agent_id" | "public_key" | "private_key" => {
                anyhow::bail!("remote config field {key} cannot be changed")
            }
            _ => anyhow::bail!("unknown remote config field: {key}"),
        }
    }
    Ok(())
}

fn validate_remote_config_url(value: &serde_json::Value, field: &str) -> anyhow::Result<String> {
    let text = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{field} must be a string"))?
        .trim();
    validate_sized_remote_text(text, 1, REMOTE_CONFIG_MAX_URL_BYTES, field)?;
    let parsed = reqwest::Url::parse(text).with_context(|| format!("{field} is invalid"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => anyhow::bail!("{field} scheme '{scheme}' is not allowed"),
    }
    if parsed.host_str().is_none() {
        anyhow::bail!("{field} must include a host");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!("{field} must not include credentials");
    }
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn validate_optional_remote_config_path(
    value: &serde_json::Value,
    field: &str,
) -> anyhow::Result<Option<String>> {
    let Some(text) =
        validate_optional_remote_config_text(value, field, REMOTE_CONFIG_MAX_PATH_BYTES)?
    else {
        return Ok(None);
    };
    if text.contains('\0') {
        anyhow::bail!("{field} contains NUL byte");
    }
    Ok(Some(text))
}

fn validate_optional_remote_config_text(
    value: &serde_json::Value,
    field: &str,
    max_bytes: usize,
) -> anyhow::Result<Option<String>> {
    let text = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{field} must be a string"))?
        .trim();
    if text.is_empty() {
        return Ok(None);
    }
    validate_sized_remote_text(text, 1, max_bytes, field)?;
    if text.chars().any(char::is_control) {
        anyhow::bail!("{field} contains control characters");
    }
    Ok(Some(text.to_string()))
}

fn validate_remote_config_name(value: &serde_json::Value) -> anyhow::Result<String> {
    let text = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("name must be a string"))?
        .trim();
    validate_sized_remote_text(text, 1, REMOTE_CONFIG_MAX_NAME_BYTES, "name")?;
    if text.chars().any(char::is_control) {
        anyhow::bail!("name contains control characters");
    }
    Ok(text.to_string())
}

fn validate_remote_config_interval(value: &serde_json::Value, field: &str) -> anyhow::Result<u64> {
    let value = value
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("{field} must be an unsigned integer"))?;
    if !(REMOTE_CONFIG_MIN_INTERVAL_SECONDS..=REMOTE_CONFIG_MAX_INTERVAL_SECONDS).contains(&value) {
        anyhow::bail!(
            "{field} must be between {REMOTE_CONFIG_MIN_INTERVAL_SECONDS} and {REMOTE_CONFIG_MAX_INTERVAL_SECONDS} seconds"
        );
    }
    Ok(value)
}

fn validate_remote_config_bool(value: &serde_json::Value, field: &str) -> anyhow::Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("{field} must be a boolean"))
}

fn validate_remote_config_roots(value: &serde_json::Value) -> anyhow::Result<Vec<String>> {
    let roots = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("file_allowed_roots must be an array"))?;
    if roots.is_empty() || roots.len() > REMOTE_CONFIG_MAX_ROOTS {
        anyhow::bail!("file_allowed_roots must contain 1 to {REMOTE_CONFIG_MAX_ROOTS} entries");
    }
    let mut normalized = Vec::with_capacity(roots.len());
    for root in roots {
        let text = root
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("file_allowed_roots entries must be strings"))?
            .trim();
        validate_sized_remote_text(text, 1, REMOTE_CONFIG_MAX_PATH_BYTES, "file_allowed_roots")?;
        if text.contains('\0') {
            anyhow::bail!("file_allowed_roots contains NUL byte");
        }
        normalized.push(text.to_string());
    }
    Ok(normalized)
}

fn validate_sized_remote_text(
    value: &str,
    min_bytes: usize,
    max_bytes: usize,
    field: &str,
) -> anyhow::Result<()> {
    let len = value.len();
    if len < min_bytes || len > max_bytes {
        anyhow::bail!("{field} must be between {min_bytes} and {max_bytes} bytes");
    }
    Ok(())
}

async fn record_force_update_request(
    runtime: &RuntimeContext,
    update: &ForceUpdate,
) -> anyhow::Result<()> {
    let version = validate_force_update_version(&update.version)?;
    let download_url = validate_force_update_download_url(&update.download_url)?;
    let checksum = validate_force_update_checksum(&update.checksum)?;
    let payload = serde_json::json!({
        "version": version,
        "download_url": download_url,
        "checksum": checksum,
        "recorded_at": now_unix_seconds(),
    });
    let path = runtime
        .config_path
        .parent()
        .map(|p| p.join("last-force-update.json"))
        .unwrap_or_else(|| PathBuf::from("last-force-update.json"));
    std::fs::write(path, serde_json::to_vec_pretty(&payload)?)?;
    Ok(())
}

fn validate_force_update_version(version: &str) -> anyhow::Result<String> {
    let version = version.trim();
    if version.is_empty() {
        anyhow::bail!("force update version is required");
    }
    if version == "latest" {
        anyhow::bail!("force update requires an explicit version");
    }
    if version.len() > FORCE_UPDATE_MAX_VERSION_BYTES
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        anyhow::bail!("force update version contains unsupported characters");
    }
    Ok(version.to_string())
}

fn validate_force_update_download_url(download_url: &str) -> anyhow::Result<String> {
    let download_url = download_url.trim();
    validate_sized_remote_text(download_url, 1, FORCE_UPDATE_MAX_URL_BYTES, "download_url")?;
    let parsed = reqwest::Url::parse(download_url).context("download_url is invalid")?;
    if parsed.scheme() != "https" {
        anyhow::bail!("download_url must use https");
    }
    if parsed.host_str().is_none() {
        anyhow::bail!("download_url must include a host");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!("download_url must not include credentials");
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        anyhow::bail!("download_url must not include query or fragment");
    }
    Ok(parsed.to_string())
}

fn validate_force_update_checksum(checksum: &str) -> anyhow::Result<String> {
    let checksum = checksum.trim();
    let checksum = checksum.strip_prefix("sha256:").unwrap_or(checksum);
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("checksum must be a sha256 hex digest");
    }
    Ok(checksum.to_ascii_lowercase())
}

fn persist_agent_config(config_path: &PathBuf, config: &AgentConfig) -> anyhow::Result<()> {
    let serialized = serde_json::to_string_pretty(config)?;
    write_secure_config(config_path, serialized.as_bytes())
}

fn read_private_key(config_path: &PathBuf) -> anyhow::Result<String> {
    let config_text = std::fs::read_to_string(config_path)?;
    let config: AgentConfig = serde_json::from_str(&config_text)?;
    Ok(config.private_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_agent_config() -> AgentConfig {
        AgentConfig {
            server: "http://dashboard.example".to_string(),
            grpc_server: "http://dashboard.example:50051".to_string(),
            grpc_tls_ca_path: None,
            grpc_tls_domain_name: None,
            grpc_tls_client_cert_path: None,
            grpc_tls_client_key_path: None,
            agent_id: "agent-1".to_string(),
            name: "agent-1".to_string(),
            public_key: String::new(),
            private_key: "private-key".to_string(),
            report_interval_seconds: 3,
            ip_report_interval_seconds: 60,
            disable_auto_update: false,
            disable_force_update: false,
            disable_command_execute: false,
            disable_nat: false,
            disable_send_query: false,
            file_allowed_roots: default_file_allowed_roots(),
        }
    }

    fn test_runtime_with_config(config: AgentConfig) -> (tempfile::TempDir, RuntimeContext) {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("agent.json");
        persist_agent_config(&config_path, &config).unwrap();
        let runtime = RuntimeContext {
            config_path,
            config: Arc::new(Mutex::new(config)),
        };
        (temp_dir, runtime)
    }

    #[test]
    fn enrollment_token_can_be_read_from_stdin_without_using_argv() {
        let token = format!("xle_{}", "a".repeat(64));

        assert_eq!(
            enrollment_token_from_sources_with_reader(None, true, token.as_bytes()).unwrap(),
            token
        );
        assert_eq!(
            enrollment_token_from_sources_with_reader(Some(format!(" {token}\n")), false, &b""[..])
                .unwrap(),
            token
        );
        assert!(
            enrollment_token_from_sources_with_reader(None, false, &b""[..])
                .unwrap_err()
                .to_string()
                .contains("use --token or --token-stdin")
        );
        assert!(
            enrollment_token_from_sources_with_reader(None, true, "xle_bad token".as_bytes())
                .unwrap_err()
                .to_string()
                .contains("must not contain whitespace")
        );
    }

    #[test]
    fn install_agent_script_keeps_enrollment_token_out_of_agent_argv() {
        let script = include_str!("../../../deploy/install-agent.sh");

        assert!(script.contains("--token-stdin"));
        assert!(
            script.contains("printf '%s' \"$ENROLLMENT_TOKEN\" | /usr/local/bin/xlstatus-agent")
        );
        assert!(!script.contains("--token \"$ENROLLMENT_TOKEN\""));
    }

    #[test]
    fn agent_probe_blocks_private_ip_ranges() {
        assert!(is_agent_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_agent_blocked_ip("10.1.2.3".parse().unwrap()));
        assert!(is_agent_blocked_ip("172.16.0.1".parse().unwrap()));
        assert!(is_agent_blocked_ip("192.168.1.1".parse().unwrap()));
        assert!(is_agent_blocked_ip("169.254.169.254".parse().unwrap()));
        assert!(is_agent_blocked_ip("100.64.0.1".parse().unwrap()));
        assert!(is_agent_blocked_ip("::1".parse().unwrap()));
        assert!(is_agent_blocked_ip("fc00::1".parse().unwrap()));
        assert!(is_agent_blocked_ip("fe80::1".parse().unwrap()));
    }

    #[test]
    fn agent_probe_allows_public_ip_ranges() {
        assert!(!is_agent_blocked_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_agent_blocked_ip(
            "2606:4700:4700::1111".parse().unwrap()
        ));
    }

    #[tokio::test]
    async fn agent_probe_rejects_private_http_ip_literal() {
        let err = validate_agent_probe_url_resolved_with_policy(
            "http://127.0.0.1:8080/status",
            "test",
            false,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("disallowed private address"));
    }

    #[tokio::test]
    async fn agent_probe_pins_public_http_ip_literal() {
        let validated = validate_agent_probe_url_resolved_with_policy(
            "https://1.1.1.1/dns-query",
            "test",
            false,
        )
        .await
        .unwrap();
        assert_eq!(validated.host, "1.1.1.1");
        assert_eq!(validated.addrs.len(), 1);
        assert_eq!(
            validated.addrs[0].ip(),
            "1.1.1.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(validated.addrs[0].port(), 443);
    }

    #[tokio::test]
    async fn agent_probe_rejects_http_url_credentials() {
        let err = validate_agent_probe_url_resolved_with_policy(
            "https://user:pass@example.com/status",
            "test",
            false,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("must not include credentials"));
    }

    #[tokio::test]
    async fn agent_probe_private_policy_escape_hatch_allows_literal() {
        let resolved = resolve_agent_probe_host_with_policy("127.0.0.1", 8080, "test", true)
            .await
            .unwrap();
        assert_eq!(resolved, vec!["127.0.0.1:8080".parse().unwrap()]);
    }

    #[test]
    fn agent_probe_ping_target_uses_resolved_ip_literal() {
        let addrs = vec![
            "[2606:4700:4700::1111]:0".parse::<SocketAddr>().unwrap(),
            "1.1.1.1:0".parse().unwrap(),
        ];
        assert_eq!(
            agent_ping_target(&addrs, "test").unwrap(),
            "2606:4700:4700::1111"
        );
    }

    #[test]
    fn agent_probe_rejects_invalid_tcp_port() {
        let err = agent_probe_port(65_536, "test").unwrap_err();
        assert!(err.to_string().contains("out of range"));
        let err = agent_probe_port(0, "test").unwrap_err();
        assert!(err.to_string().contains("greater than 0"));
    }

    #[test]
    fn agent_shell_task_limits_are_bounded() {
        assert_eq!(
            shell_task_timeout_seconds(0),
            AGENT_SHELL_DEFAULT_TIMEOUT_SECONDS
        );
        assert_eq!(
            shell_task_timeout_seconds(1),
            AGENT_SHELL_MIN_TIMEOUT_SECONDS
        );
        assert_eq!(
            shell_task_timeout_seconds(u32::MAX),
            AGENT_SHELL_MAX_TIMEOUT_SECONDS
        );
        assert_eq!(
            shell_task_output_max_bytes(0),
            AGENT_SHELL_DEFAULT_OUTPUT_MAX_BYTES
        );
        assert_eq!(
            shell_task_output_max_bytes(u64::MAX),
            AGENT_SHELL_OUTPUT_MAX_BYTES
        );
    }

    #[test]
    fn agent_probe_task_timeout_is_bounded() {
        assert_eq!(task_timeout_seconds(0), AGENT_PROBE_DEFAULT_TIMEOUT_SECONDS);
        assert_eq!(task_timeout_seconds(1), AGENT_PROBE_MIN_TIMEOUT_SECONDS);
        assert_eq!(
            task_timeout_seconds(u32::MAX),
            AGENT_PROBE_MAX_TIMEOUT_SECONDS
        );
    }

    #[test]
    fn agent_ping_output_text_is_bounded_and_utf8_safe() {
        let oversized = format!("{}é", "x".repeat(AGENT_PING_OUTPUT_MAX_BYTES - 1));
        assert!(oversized.len() > AGENT_PING_OUTPUT_MAX_BYTES);

        let bounded = bounded_ping_output_text(oversized.as_bytes());
        assert!(bounded.ends_with("... [truncated]"));
        assert!(bounded.len() <= AGENT_PING_OUTPUT_MAX_BYTES + "... [truncated]".len());
        assert!(bounded.is_char_boundary(bounded.len()));
    }

    #[test]
    fn agent_ping_failure_message_uses_bounded_stderr_or_stdout() {
        let stderr = "e".repeat(AGENT_PING_OUTPUT_MAX_BYTES + 64);
        let message = ping_failure_message(stderr.as_bytes(), b"stdout detail");
        assert!(message.starts_with("Ping failed: "));
        assert!(message.contains("[truncated]"));
        assert!(!message.contains("stdout detail"));

        let fallback = ping_failure_message(b"   ", b"stdout detail");
        assert_eq!(fallback, "Ping failed: stdout detail");

        let empty = ping_failure_message(b"", b"");
        assert_eq!(empty, "Ping failed");
    }

    #[tokio::test]
    async fn agent_nat_target_allows_loopback_by_default() {
        let addrs = resolve_agent_nat_target_with_policy("127.0.0.1", 8080, false)
            .await
            .unwrap();
        assert_eq!(addrs, vec!["127.0.0.1:8080".parse().unwrap()]);

        let addrs = resolve_agent_nat_target_with_policy("::1", 8080, false)
            .await
            .unwrap();
        assert_eq!(addrs, vec!["[::1]:8080".parse().unwrap()]);
    }

    #[tokio::test]
    async fn agent_nat_target_rejects_private_non_loopback_by_default() {
        let err = resolve_agent_nat_target_with_policy("192.168.1.10", 8080, false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("disallowed non-loopback address"));
    }

    #[tokio::test]
    async fn agent_nat_target_escape_hatch_allows_private_literal() {
        let addrs = resolve_agent_nat_target_with_policy("192.168.1.10", 8080, true)
            .await
            .unwrap();
        assert_eq!(addrs, vec!["192.168.1.10:8080".parse().unwrap()]);
    }

    #[test]
    fn agent_nat_target_label_formats_ipv6() {
        assert_eq!(format_nat_target_label("::1", 8080), "[::1]:8080");
        assert_eq!(format_nat_target_label("127.0.0.1", 8080), "127.0.0.1:8080");
    }

    #[tokio::test]
    async fn agent_probe_task_policy_rejects_when_send_query_disabled() {
        use xlstatus_proto_gen::xlstatus::v1::server_task::Spec;
        use xlstatus_proto_gen::xlstatus::v1::{HttpGetTask, ServerTask, TaskOutcome, TaskType};

        let config = AgentConfig {
            disable_send_query: true,
            ..test_agent_config()
        };
        let runtime = RuntimeContext {
            config_path: PathBuf::from("agent.json"),
            config: Arc::new(Mutex::new(config)),
        };
        let task = ServerTask {
            task_id: "task-1".to_string(),
            task_type: TaskType::HttpGet as i32,
            spec: Some(Spec::HttpGet(HttpGetTask {
                url: "https://1.1.1.1/dns-query".to_string(),
                timeout_seconds: 1,
                verify_tls: true,
                headers: Default::default(),
            })),
        };

        let result = run_server_task(runtime, task).await;
        assert_eq!(result.status, TaskOutcome::Failure as i32);
        assert_eq!(result.exit_code, 126);
        assert_eq!(result.error, "probe task disabled by agent policy");
    }

    #[tokio::test]
    async fn agent_remote_config_applies_allowed_fields_and_preserves_identity() {
        let (_temp_dir, runtime) = test_runtime_with_config(test_agent_config());
        let update = ConfigUpdate {
            config_yaml: serde_json::to_vec(&serde_json::json!({
                "name": " edge-1 ",
                "server": "https://dashboard.example/",
                "report_interval_seconds": 30,
                "disable_send_query": true,
                "file_allowed_roots": ["/var/lib/xlstatus/files"]
            }))
            .unwrap(),
        };

        apply_remote_config(&runtime, &update).await.unwrap();

        let guard = runtime.config.lock().await;
        assert_eq!(guard.name, "edge-1");
        assert_eq!(guard.server, "https://dashboard.example");
        assert_eq!(guard.report_interval_seconds, 30);
        assert!(guard.disable_send_query);
        assert_eq!(guard.file_allowed_roots, vec!["/var/lib/xlstatus/files"]);
        assert_eq!(guard.private_key, "private-key");
    }

    #[tokio::test]
    async fn agent_remote_config_rejects_forbidden_unknown_and_oversized_payloads() {
        let (_temp_dir, runtime) = test_runtime_with_config(test_agent_config());
        for patch in [
            serde_json::json!({ "agent_id": "other" }),
            serde_json::json!({ "private_key": "secret" }),
            serde_json::json!({ "unexpected": true }),
            serde_json::json!({ "server": "https://user:pass@example.com" }),
            serde_json::json!({ "name": "x".repeat(REMOTE_CONFIG_MAX_NAME_BYTES + 1) }),
            serde_json::json!({ "report_interval_seconds": REMOTE_CONFIG_MAX_INTERVAL_SECONDS + 1 }),
            serde_json::json!({ "file_allowed_roots": [] }),
        ] {
            let update = ConfigUpdate {
                config_yaml: serde_json::to_vec(&patch).unwrap(),
            };
            assert!(
                apply_remote_config(&runtime, &update).await.is_err(),
                "patch should be rejected: {patch}"
            );
        }

        let update = ConfigUpdate {
            config_yaml: vec![b' '; REMOTE_CONFIG_MAX_BYTES + 1],
        };
        assert!(apply_remote_config(&runtime, &update).await.is_err());
    }

    #[tokio::test]
    async fn agent_force_update_record_validates_shape_before_writing() {
        let (_temp_dir, runtime) = test_runtime_with_config(test_agent_config());
        let valid = ForceUpdate {
            version: "v0.1.0".into(),
            download_url: "https://updates.example.com/xlstatus-agent-linux-amd64.tar.gz".into(),
            checksum: format!("sha256:{}", "a".repeat(64)),
        };

        record_force_update_request(&runtime, &valid).await.unwrap();
        let recorded = std::fs::read_to_string(
            runtime
                .config_path
                .parent()
                .unwrap()
                .join("last-force-update.json"),
        )
        .unwrap();
        assert!(recorded.contains("\"checksum\": \""));
        assert!(recorded.contains(&"a".repeat(64)));

        for invalid in [
            ForceUpdate {
                version: "latest".into(),
                ..valid.clone()
            },
            ForceUpdate {
                download_url: "http://updates.example.com/agent.tar.gz".into(),
                ..valid.clone()
            },
            ForceUpdate {
                download_url: "https://user@updates.example.com/agent.tar.gz".into(),
                ..valid.clone()
            },
            ForceUpdate {
                download_url: "https://updates.example.com/agent.tar.gz?token=secret".into(),
                ..valid.clone()
            },
            ForceUpdate {
                checksum: "not-a-sha256".into(),
                ..valid.clone()
            },
        ] {
            assert!(record_force_update_request(&runtime, &invalid)
                .await
                .is_err());
        }
    }
}
