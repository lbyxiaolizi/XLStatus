use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, Duration};
use tokio_stream::wrappers::ReceiverStream;
use tonic::metadata::MetadataValue;
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

        /// Enrollment token
        #[arg(long)]
        token: String,

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
            token,
            name,
            config,
        } => {
            tracing::info!("Enrolling agent with server: {}", server);
            enroll_agent(server, grpc_server, token, name, PathBuf::from(config)).await
        }
        Commands::Run { config } => {
            tracing::info!("Starting agent with config: {}", config);
            run_agent(PathBuf::from(config)).await
        }
    }
}

async fn enroll_agent(
    server: String,
    grpc_server: Option<String>,
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

/// One gRPC session: connect -> heartbeat / HostState / HostInfo ->
/// JWT auto-refresh every 4 minutes -> wait for the server to close
/// the stream or send ForceDisconnect. The returned enum tells the
/// outer loop whether to reconnect or exit.
async fn run_agent_session(runtime: RuntimeContext) -> anyhow::Result<SessionExit> {
    let initial = runtime.config.lock().await.clone();
    let mut jwt = fetch_agent_jwt(&initial).await?;
    let mut client = AgentServiceClient::connect(initial.grpc_server.clone()).await?;
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

    // M2: JWT auto-refresh every `JWT_REFRESH_SECS` (4 min), well
    // before the 5 min server-side expiry. We just re-run the
    // challenge/signature flow; the in-memory `jwt` is reused on the
    // next reconnect cycle (the gRPC stream itself is not
    // re-authenticated, server-side JWT verification happens on the
    // initial connect).
    let mut refresh_tick = interval(Duration::from_secs(JWT_REFRESH_SECS));
    refresh_tick.tick().await; // skip the immediate first tick

    println!(
        "Agent {} connected (heartbeat {}s, report {}s, jwt refresh {}s)",
        initial.agent_id,
        HEARTBEAT_INTERVAL_SECS,
        initial.report_interval_seconds.max(1),
        JWT_REFRESH_SECS
    );

    loop {
        tokio::select! {
            biased;
            _ = refresh_tick.tick() => {
                let cfg = runtime.config.lock().await.clone();
                match fetch_agent_jwt(&cfg).await {
                    Ok(fresh) => {
                        tracing::debug!("agent jwt refreshed");
                        jwt = fresh;
                    }
                    Err(e) => {
                        tracing::warn!("agent jwt refresh failed: {}", e);
                    }
                }
            }
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
/// M2: refresh the agent JWT every 4 min, well inside the
/// server-issued 5 min window. See plan/02-architecture.md.
const JWT_REFRESH_SECS: u64 = 240;

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
            let local_addr = format!("{}:{}", local_host, local_port);
            match TcpStream::connect(&local_addr).await {
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
                            &format!("failed to connect to {}: {}", local_addr, e),
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
            let cmd = shell.command.clone();
            let timeout = if shell.timeout_seconds > 0 {
                shell.timeout_seconds as u64
            } else {
                30
            };
            let max_output_bytes = shell.max_output_bytes as usize;
            match tokio::time::timeout(
                std::time::Duration::from_secs(timeout),
                tokio::process::Command::new("/bin/sh")
                    .arg("-c")
                    .arg(&cmd)
                    .output(),
            )
            .await
            {
                Ok(Ok(out)) => {
                    result.exit_code = out.status.code().unwrap_or(-1);
                    let mut truncated = false;
                    let stdout = truncate_output(&out.stdout, max_output_bytes, &mut truncated);
                    let stderr = truncate_output(&out.stderr, max_output_bytes, &mut truncated);
                    result.stdout = String::from_utf8_lossy(stdout).to_string();
                    result.stderr = String::from_utf8_lossy(stderr).to_string();
                    if truncated {
                        result.error = format!("output truncated at {} bytes", max_output_bytes);
                    }
                    result.status = if out.status.success() {
                        TaskOutcome::Success as i32
                    } else {
                        TaskOutcome::Failure as i32
                    };
                }
                Ok(Err(e)) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.error = format!("spawn failed: {e}");
                }
                Err(_) => {
                    result.status = TaskOutcome::Failure as i32;
                    result.error = format!("timeout after {timeout}s");
                }
            }
        }
        Some(Spec::HttpGet(spec)) => match run_http_probe_task(spec).await {
            Ok(output) => {
                result.status = TaskOutcome::Success as i32;
                result.stdout = output;
            }
            Err(e) => {
                result.status = TaskOutcome::Failure as i32;
                result.exit_code = 1;
                result.error = e.to_string();
            }
        },
        Some(Spec::TcpPing(spec)) => match run_tcp_probe_task(spec).await {
            Ok(output) => {
                result.status = TaskOutcome::Success as i32;
                result.stdout = output;
            }
            Err(e) => {
                result.status = TaskOutcome::Failure as i32;
                result.exit_code = 1;
                result.error = e.to_string();
            }
        },
        Some(Spec::IcmpPing(spec)) => match run_icmp_probe_task(spec).await {
            Ok(output) => {
                result.status = TaskOutcome::Success as i32;
                result.stdout = output;
            }
            Err(e) => {
                result.status = TaskOutcome::Failure as i32;
                result.exit_code = 1;
                result.error = e.to_string();
            }
        },
        Some(Spec::FileList(spec)) => {
            if cfg.disable_command_execute {
                result.status = TaskOutcome::Failure as i32;
                result.exit_code = 126;
                result.error = "file access disabled by agent policy".to_string();
                result.finished_at = finished();
                return result;
            }
            match executor::files::list_files(&spec.path).await {
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
            match executor::files::read_file(&spec.path, spec.offset, spec.length).await {
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
            match executor::files::delete_path(&spec.path, spec.recursive).await {
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
        .build()?;

    let output = match client.get(&spec.url).headers(headers).send().await {
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
    let addr = socket_addr(&spec.host, spec.port);
    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    {
        Ok(Ok(_)) => probe_output_json(true, Some(start.elapsed().as_millis() as i32), None, None),
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
    let count = spec.count.clamp(1, 10).to_string();
    let timeout_text = timeout.to_string();
    let mut command = if cfg!(target_os = "windows") {
        let mut cmd = tokio::process::Command::new("ping");
        cmd.args([
            "-n",
            &count,
            "-w",
            &(timeout * 1000).to_string(),
            &spec.host,
        ]);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("ping");
        cmd.args(["-c", &count, "-W", &timeout_text, &spec.host]);
        cmd
    };

    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout + 2),
        command.output(),
    )
    .await
    {
        Ok(Ok(output)) => {
            let latency_ms = start.elapsed().as_millis() as i32;
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                probe_output_json(
                    true,
                    Some(parse_ping_latency(&stdout).unwrap_or(latency_ms)),
                    None,
                    None,
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let message = if stderr.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    stderr.trim().to_string()
                };
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
        10
    } else {
        u64::from(value)
    }
}

fn socket_addr(host: &str, port: u32) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
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

fn truncate_output<'a>(data: &'a [u8], max_output_bytes: usize, truncated: &mut bool) -> &'a [u8] {
    if max_output_bytes == 0 || data.len() <= max_output_bytes {
        return data;
    }
    *truncated = true;
    &data[..max_output_bytes]
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

async fn apply_remote_config(
    runtime: &RuntimeContext,
    update: &ConfigUpdate,
) -> anyhow::Result<()> {
    let patch: serde_json::Value = serde_json::from_slice(&update.config_yaml)
        .map_err(|e| anyhow::anyhow!("invalid remote config payload: {}", e))?;
    let mut guard = runtime.config.lock().await;
    let mut current = guard.clone();

    if let Some(v) = patch.get("server").and_then(|v| v.as_str()) {
        current.server = v.to_string();
    }
    if let Some(v) = patch.get("grpc_server").and_then(|v| v.as_str()) {
        current.grpc_server = v.to_string();
    }
    if let Some(v) = patch.get("agent_id").and_then(|v| v.as_str()) {
        current.agent_id = v.to_string();
    }
    if let Some(v) = patch.get("name").and_then(|v| v.as_str()) {
        current.name = v.to_string();
    }
    if let Some(v) = patch.get("public_key").and_then(|v| v.as_str()) {
        current.public_key = v.to_string();
    }
    if let Some(v) = patch
        .get("report_interval_seconds")
        .and_then(|v| v.as_u64())
    {
        current.report_interval_seconds = v.max(1);
    }
    if let Some(v) = patch
        .get("ip_report_interval_seconds")
        .and_then(|v| v.as_u64())
    {
        current.ip_report_interval_seconds = v.max(1);
    }
    if let Some(v) = patch.get("disable_auto_update").and_then(|v| v.as_bool()) {
        current.disable_auto_update = v;
    }
    if let Some(v) = patch.get("disable_force_update").and_then(|v| v.as_bool()) {
        current.disable_force_update = v;
    }
    if let Some(v) = patch
        .get("disable_command_execute")
        .and_then(|v| v.as_bool())
    {
        current.disable_command_execute = v;
    }
    if let Some(v) = patch.get("disable_nat").and_then(|v| v.as_bool()) {
        current.disable_nat = v;
    }
    if let Some(v) = patch.get("disable_send_query").and_then(|v| v.as_bool()) {
        current.disable_send_query = v;
    }
    current.private_key = read_private_key(&runtime.config_path)?;
    persist_agent_config(&runtime.config_path, &current)?;
    *guard = current;
    Ok(())
}

async fn record_force_update_request(
    runtime: &RuntimeContext,
    update: &ForceUpdate,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "version": update.version,
        "download_url": update.download_url,
        "checksum": update.checksum,
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

fn persist_agent_config(config_path: &PathBuf, config: &AgentConfig) -> anyhow::Result<()> {
    let serialized = serde_json::to_string_pretty(config)?;
    write_secure_config(config_path, serialized.as_bytes())
}

fn read_private_key(config_path: &PathBuf) -> anyhow::Result<String> {
    let config_text = std::fs::read_to_string(config_path)?;
    let config: AgentConfig = serde_json::from_str(&config_text)?;
    Ok(config.private_key)
}
