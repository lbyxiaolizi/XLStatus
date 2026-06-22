use crate::db::repository::NatMappingRepository;
use crate::db::Db;
use crate::grpc::IoRegistry;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, RwLock};
use tracing::{error, info, warn};
use xlstatus_proto_gen::xlstatus::v1::{io_frame, IoClose, IoData, IoError, IoFrame};
use xlstatus_shared::nat::{NatMapping, NatTunnelControlMessage, Protocol, TunnelStats};

const DEFAULT_NAT_MAX_ACTIVE_TUNNELS: usize = 128;
const DEFAULT_NAT_MIN_PUBLIC_PORT: u16 = 1024;

/// Active tunnel connection
#[derive(Debug)]
struct ActiveTunnel {
    tunnel_id: String,
    mapping_id: String,
    agent_id: String,
    created_at: chrono::DateTime<chrono::Utc>,
    bytes_sent: Arc<RwLock<u64>>,
    bytes_received: Arc<RwLock<u64>>,
}

/// NAT tunnel manager
pub struct NatTunnelManager {
    db: Db,
    io_registry: IoRegistry,
    listeners: Arc<RwLock<HashMap<u16, NatListenerHandle>>>,
    active_tunnels: Arc<RwLock<HashMap<String, ActiveTunnel>>>,
}

#[derive(Clone)]
struct NatListenerHandle {
    shutdown: watch::Sender<bool>,
}

impl NatTunnelManager {
    pub fn new(db: Db, io_registry: IoRegistry) -> Self {
        Self {
            db,
            io_registry,
            listeners: Arc::new(RwLock::new(HashMap::new())),
            active_tunnels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start NAT tunnel manager
    pub async fn start(self: Arc<Self>) -> Result<()> {
        info!("Starting NAT tunnel manager");
        self.reload().await?;

        info!(
            "NAT tunnel manager started with {} mappings",
            self.listeners.read().await.len()
        );
        Ok(())
    }

    pub async fn reload(&self) -> Result<()> {
        let mappings = NatMappingRepository::list_enabled_for_active_agents(&self.db)
            .await
            .context("Failed to load NAT mappings")?;
        let mut tcp_mappings = Vec::new();
        for mapping in mappings {
            if matches!(mapping.protocol, Protocol::Tcp) {
                tcp_mappings.push(mapping);
            } else {
                warn!(
                    "UDP tunnels not yet supported, skipping mapping {}",
                    mapping.id
                );
            }
        }
        let desired_ports: std::collections::HashSet<u16> = tcp_mappings
            .iter()
            .map(|mapping| mapping.public_port)
            .collect();

        let mut stale_listeners = Vec::new();
        {
            let mut listeners = self.listeners.write().await;
            listeners.retain(|port, handle| {
                let keep = desired_ports.contains(port);
                if !keep {
                    stale_listeners.push(handle.shutdown.clone());
                }
                keep
            });
        }
        for shutdown in stale_listeners {
            let _ = shutdown.send(true);
        }

        for mapping in tcp_mappings {
            if self
                .listeners
                .read()
                .await
                .contains_key(&mapping.public_port)
            {
                continue;
            }
            if let Err(e) = self.start_listener(mapping).await {
                error!("Failed to start listener: {}", e);
            }
        }
        Ok(())
    }

    async fn start_listener(&self, mapping: NatMapping) -> Result<()> {
        if !matches!(mapping.protocol, Protocol::Tcp) {
            warn!(
                "UDP tunnels not yet supported, skipping mapping {}",
                mapping.id
            );
            return Ok(());
        }

        let addr = nat_bind_addr(mapping.public_port);
        let listener = TcpListener::bind(&addr)
            .await
            .context(format!("Failed to bind to {}", addr))?;

        info!(
            "NAT listener started on port {} -> {}:{} via agent {}",
            mapping.public_port, mapping.local_host, mapping.local_port, mapping.agent_id
        );

        let listener = Arc::new(listener);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        self.listeners.write().await.insert(
            mapping.public_port,
            NatListenerHandle {
                shutdown: shutdown_tx,
            },
        );

        let manager = Arc::new(self.clone());
        let public_port = mapping.public_port;
        tokio::spawn(async move {
            if let Err(e) = manager
                .accept_loop(listener, public_port, shutdown_rx)
                .await
            {
                error!("Accept loop error: {}", e);
            }
        });

        Ok(())
    }

    async fn accept_loop(
        self: Arc<Self>,
        listener: Arc<TcpListener>,
        public_port: u16,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<()> {
        loop {
            let accepted = tokio::select! {
                result = listener.accept() => result,
                changed = shutdown.changed() => {
                    if changed.is_ok() && *shutdown.borrow() {
                        info!("NAT listener on port {} stopped", public_port);
                        return Ok(());
                    }
                    continue;
                }
            };
            match accepted {
                Ok((stream, peer_addr)) => {
                    if !nat_source_allowed(peer_addr) {
                        warn!(
                            "Rejected NAT connection on port {} from {}: source not allowed",
                            public_port, peer_addr
                        );
                        drop(stream);
                        continue;
                    }
                    let mapping = match self.current_mapping_for_port(public_port).await {
                        Ok(Some(mapping)) => mapping,
                        Ok(None) => {
                            warn!(
                                "Rejected NAT connection on port {} from {}: mapping is disabled or missing",
                                public_port, peer_addr
                            );
                            drop(stream);
                            continue;
                        }
                        Err(e) => {
                            warn!(
                                "Rejected NAT connection on port {} from {}: failed to load mapping policy: {}",
                                public_port, peer_addr, e
                            );
                            drop(stream);
                            continue;
                        }
                    };
                    if !nat_mapping_source_allowed(&mapping, peer_addr) {
                        warn!(
                            "Rejected NAT connection on port {} from {}: source not allowed by mapping policy",
                            mapping.public_port, peer_addr
                        );
                        drop(stream);
                        continue;
                    }
                    if self.active_tunnel_count().await >= nat_max_active_tunnels() {
                        warn!(
                            "Rejected NAT connection on port {} from {}: active tunnel limit reached",
                            mapping.public_port, peer_addr
                        );
                        drop(stream);
                        continue;
                    }
                    if let Some(limit) = mapping.max_active_tunnels {
                        if self.active_tunnel_count_for_mapping(&mapping.id).await >= limit as usize
                        {
                            warn!(
                                "Rejected NAT connection on port {} from {}: mapping active tunnel limit reached",
                                mapping.public_port, peer_addr
                            );
                            drop(stream);
                            continue;
                        }
                    }
                    let usage_window = match self.record_nat_connection(&mapping, peer_addr).await {
                        Ok(window) => window,
                        Err(e) => {
                            warn!(
                                "Rejected NAT connection on port {} from {}: {}",
                                mapping.public_port, peer_addr, e
                            );
                            drop(stream);
                            continue;
                        }
                    };
                    info!(
                        "New NAT connection on port {} from {}",
                        mapping.public_port, peer_addr
                    );

                    let manager = self.clone();
                    let mapping_clone = mapping.clone();
                    tokio::spawn(async move {
                        if let Err(e) = manager
                            .handle_connection(stream, mapping_clone, peer_addr, usage_window)
                            .await
                        {
                            error!("NAT connection handling error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error on port {}: {}", public_port, e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn current_mapping_for_port(&self, public_port: u16) -> Result<Option<NatMapping>> {
        let Some(mapping) =
            NatMappingRepository::get_active_by_public_port(&self.db, public_port).await?
        else {
            return Ok(None);
        };
        if matches!(mapping.protocol, Protocol::Tcp) {
            Ok(Some(mapping))
        } else {
            Ok(None)
        }
    }

    async fn handle_connection(
        &self,
        public_stream: TcpStream,
        mapping: NatMapping,
        peer_addr: SocketAddr,
        usage_window: Option<NatUsageWindowKey>,
    ) -> Result<()> {
        let tunnel_id = uuid::Uuid::now_v7().to_string();
        let agent_uuid = uuid::Uuid::parse_str(&mapping.agent_id)
            .map_err(|_| anyhow!("invalid agent id on mapping {}", mapping.id))?;
        let agent_id = xlstatus_shared::AgentId(agent_uuid);

        info!(
            "Creating NAT tunnel {} for mapping {} -> {}:{}",
            tunnel_id, mapping.id, mapping.local_host, mapping.local_port
        );

        if !self.io_registry.is_agent_online(&agent_id).await {
            return Err(anyhow!(
                "agent {} has no active IO stream",
                mapping.agent_id
            ));
        }

        let tunnel = ActiveTunnel {
            tunnel_id: tunnel_id.clone(),
            mapping_id: mapping.id.clone(),
            agent_id: mapping.agent_id.clone(),
            created_at: chrono::Utc::now(),
            bytes_sent: Arc::new(RwLock::new(0)),
            bytes_received: Arc::new(RwLock::new(0)),
        };
        self.active_tunnels
            .write()
            .await
            .insert(tunnel_id.clone(), tunnel);

        let mut inbound = self.io_registry.subscribe_stream(tunnel_id.clone()).await;
        self.io_registry
            .send_to_agent(
                &agent_id,
                IoFrame {
                    stream_id: tunnel_id.clone(),
                    sequence: 1,
                    agent_id: mapping.agent_id.clone(),
                    payload: Some(io_frame::Payload::Data(IoData {
                        data: serde_json::to_vec(&NatTunnelControlMessage::Open {
                            local_host: mapping.local_host.clone(),
                            local_port: mapping.local_port,
                        })
                        .unwrap_or_default(),
                    })),
                },
            )
            .await
            .map_err(|e| anyhow!("failed to send NAT open request: {}", e))?;

        let bytes_sent = self
            .active_tunnels
            .read()
            .await
            .get(&tunnel_id)
            .expect("active tunnel missing")
            .bytes_sent
            .clone();
        let bytes_received = self
            .active_tunnels
            .read()
            .await
            .get(&tunnel_id)
            .expect("active tunnel missing")
            .bytes_received
            .clone();

        let result = Self::bridge_over_iostream(
            public_stream,
            agent_id,
            mapping.agent_id.clone(),
            tunnel_id.clone(),
            self.io_registry.clone(),
            &mut inbound,
            bytes_sent,
            bytes_received,
            mapping.idle_timeout_seconds,
            mapping.max_bytes_per_tunnel,
            mapping.max_bandwidth_bytes_per_second,
        )
        .await;

        self.io_registry.unsubscribe_stream(&tunnel_id).await;
        self.active_tunnels.write().await.remove(&tunnel_id);

        match &result {
            Ok((sent, received)) => {
                if let Some(window) = usage_window {
                    if let Err(e) = self
                        .record_nat_window_bytes(&mapping, peer_addr, window, *sent + *received)
                        .await
                    {
                        warn!(
                            "failed to record NAT usage for mapping {} source {}: {}",
                            mapping.id, peer_addr, e
                        );
                    }
                }
                info!(
                    "NAT tunnel {} closed: {} bytes sent, {} bytes received",
                    tunnel_id, sent, received
                );
            }
            Err(e) => {
                error!("NAT tunnel {} error: {}", tunnel_id, e);
            }
        }
        result.map(|_| ())
    }

    async fn record_nat_connection(
        &self,
        mapping: &NatMapping,
        peer_addr: SocketAddr,
    ) -> Result<Option<NatUsageWindowKey>> {
        let Some(window_seconds) = nat_rate_limit_window_seconds(mapping) else {
            return Ok(None);
        };
        let window = current_nat_usage_window(window_seconds);
        let source_ip = peer_addr.ip().to_string();
        NatMappingRepository::prune_usage_windows(
            &self.db,
            Utc::now() - Duration::seconds((window_seconds as i64).saturating_mul(4)),
        )
        .await
        .ok();
        let usage = NatMappingRepository::record_connection_window(
            &self.db,
            &mapping.id,
            &source_ip,
            window.window_start,
        )
        .await?;
        if let Some(limit) = mapping.max_connections_per_window {
            if usage.connection_count > limit as i64 {
                return Err(anyhow!(
                    "NAT mapping connection rate limit exceeded for source {}",
                    source_ip
                ));
            }
        }
        if let Some(limit) = mapping.max_bytes_per_window {
            if usage.bytes_transferred > limit as i64 {
                return Err(anyhow!(
                    "NAT mapping byte window limit exceeded for source {}",
                    source_ip
                ));
            }
        }
        Ok(Some(NatUsageWindowKey {
            window_start: window.window_start,
            source_ip,
        }))
    }

    async fn record_nat_window_bytes(
        &self,
        mapping: &NatMapping,
        peer_addr: SocketAddr,
        window: NatUsageWindowKey,
        bytes: u64,
    ) -> Result<()> {
        if mapping.max_bytes_per_window.is_none() || bytes == 0 {
            return Ok(());
        }
        let usage = NatMappingRepository::record_window_bytes(
            &self.db,
            &mapping.id,
            &window.source_ip,
            window.window_start,
            bytes,
        )
        .await?;
        if let Some(limit) = mapping.max_bytes_per_window {
            if usage.bytes_transferred > limit as i64 {
                warn!(
                    "NAT mapping {} source {} exceeded byte window after tunnel close: {} > {}",
                    mapping.id,
                    peer_addr.ip(),
                    usage.bytes_transferred,
                    limit
                );
            }
        }
        Ok(())
    }

    async fn bridge_over_iostream(
        mut public_stream: TcpStream,
        agent_id: xlstatus_shared::AgentId,
        agent_id_str: String,
        tunnel_id: String,
        io_registry: IoRegistry,
        inbound: &mut tokio::sync::mpsc::Receiver<IoFrame>,
        bytes_sent: Arc<RwLock<u64>>,
        bytes_received: Arc<RwLock<u64>>,
        idle_timeout_seconds: Option<u32>,
        max_bytes_per_tunnel: Option<u64>,
        max_bandwidth_bytes_per_second: Option<u64>,
    ) -> Result<(u64, u64)> {
        let (mut public_read, mut public_write) = public_stream.split();
        let mut sequence = 2_u64;
        let mut sent_total = 0_u64;
        let mut received_total = 0_u64;
        let mut saw_ready = false;
        let mut buf = [0_u8; 8192];
        let idle_timeout =
            idle_timeout_seconds.map(|seconds| tokio::time::Duration::from_secs(seconds as u64));
        let started_at = tokio::time::Instant::now();

        loop {
            let event = if let Some(timeout) = idle_timeout {
                tokio::select! {
                    result = public_read.read(&mut buf) => NatBridgeEvent::PublicRead(result),
                    frame = inbound.recv() => NatBridgeEvent::AgentFrame(frame),
                    _ = tokio::time::sleep(timeout) => NatBridgeEvent::IdleTimeout,
                }
            } else {
                tokio::select! {
                    result = public_read.read(&mut buf) => NatBridgeEvent::PublicRead(result),
                    frame = inbound.recv() => NatBridgeEvent::AgentFrame(frame),
                }
            };

            match event {
                NatBridgeEvent::PublicRead(result) => {
                    let n = result?;
                    if n == 0 {
                        io_registry
                            .send_to_agent(
                                &agent_id,
                                IoFrame {
                                    stream_id: tunnel_id.clone(),
                                    sequence,
                                    agent_id: agent_id_str.clone(),
                                    payload: Some(io_frame::Payload::Close(IoClose {
                                        reason: "public stream closed".to_string(),
                                    })),
                                },
                            )
                            .await
                            .map_err(|e| anyhow!(e))?;
                        break;
                    }
                    io_registry
                        .send_to_agent(
                            &agent_id,
                            IoFrame {
                                stream_id: tunnel_id.clone(),
                                sequence,
                                agent_id: agent_id_str.clone(),
                                payload: Some(io_frame::Payload::Data(IoData {
                                    data: buf[..n].to_vec(),
                                })),
                            },
                        )
                        .await
                        .map_err(|e| anyhow!(e))?;
                    sent_total += n as u64;
                    *bytes_sent.write().await += n as u64;
                    sequence = sequence.saturating_add(1);
                    ensure_nat_byte_limit(
                        sent_total,
                        received_total,
                        max_bytes_per_tunnel,
                        &io_registry,
                        &agent_id,
                        &agent_id_str,
                        &tunnel_id,
                        sequence,
                    )
                    .await?;
                    apply_nat_bandwidth_limit(
                        sent_total,
                        received_total,
                        max_bandwidth_bytes_per_second,
                        started_at,
                    )
                    .await;
                }
                NatBridgeEvent::AgentFrame(frame) => {
                    let Some(frame) = frame else {
                        break;
                    };
                    match frame.payload {
                        Some(io_frame::Payload::Data(data)) => {
                            if !saw_ready {
                                if let Ok(control) =
                                    serde_json::from_slice::<NatTunnelControlMessage>(&data.data)
                                {
                                    match control {
                                        NatTunnelControlMessage::Ready => {
                                            saw_ready = true;
                                            continue;
                                        }
                                        NatTunnelControlMessage::Open { .. } => continue,
                                    }
                                } else {
                                    saw_ready = true;
                                }
                            }
                            public_write.write_all(&data.data).await?;
                            received_total += data.data.len() as u64;
                            *bytes_received.write().await += data.data.len() as u64;
                            ensure_nat_byte_limit(
                                sent_total,
                                received_total,
                                max_bytes_per_tunnel,
                                &io_registry,
                                &agent_id,
                                &agent_id_str,
                                &tunnel_id,
                                sequence,
                            )
                            .await?;
                            apply_nat_bandwidth_limit(
                                sent_total,
                                received_total,
                                max_bandwidth_bytes_per_second,
                                started_at,
                            )
                            .await;
                        }
                        Some(io_frame::Payload::Close(close)) => {
                            info!("agent closed NAT tunnel {}: {}", tunnel_id, close.reason);
                            break;
                        }
                        Some(io_frame::Payload::Error(IoError { message, .. })) => {
                            return Err(anyhow!("agent NAT tunnel error: {}", message));
                        }
                        None => {}
                    }
                }
                NatBridgeEvent::IdleTimeout => {
                    io_registry
                        .send_to_agent(
                            &agent_id,
                            IoFrame {
                                stream_id: tunnel_id.clone(),
                                sequence,
                                agent_id: agent_id_str.clone(),
                                payload: Some(io_frame::Payload::Close(IoClose {
                                    reason: "NAT tunnel idle timeout".to_string(),
                                })),
                            },
                        )
                        .await
                        .map_err(|e| anyhow!(e))?;
                    return Err(anyhow!("NAT tunnel idle timeout"));
                }
            }
        }

        Ok((sent_total, received_total))
    }

    pub async fn active_tunnel_count(&self) -> usize {
        self.active_tunnels.read().await.len()
    }

    pub async fn active_tunnel_count_for_mapping(&self, mapping_id: &str) -> usize {
        self.active_tunnels
            .read()
            .await
            .values()
            .filter(|tunnel| tunnel.mapping_id == mapping_id)
            .count()
    }

    pub async fn get_statistics(&self) -> Vec<TunnelStats> {
        let tunnels = self.active_tunnels.read().await;
        let mut stats = Vec::new();
        for tunnel in tunnels.values() {
            stats.push(TunnelStats {
                tunnel_id: tunnel.tunnel_id.clone(),
                bytes_sent: *tunnel.bytes_sent.read().await,
                bytes_received: *tunnel.bytes_received.read().await,
                connections: 1,
                last_activity: tunnel.created_at.to_rfc3339(),
            });
        }
        stats
    }
}

enum NatBridgeEvent {
    PublicRead(std::io::Result<usize>),
    AgentFrame(Option<IoFrame>),
    IdleTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NatUsageWindow {
    window_start: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NatUsageWindowKey {
    window_start: DateTime<Utc>,
    source_ip: String,
}

fn nat_rate_limit_window_seconds(mapping: &NatMapping) -> Option<u32> {
    if mapping.max_connections_per_window.is_none() && mapping.max_bytes_per_window.is_none() {
        return None;
    }
    mapping.rate_limit_window_seconds.or(Some(60))
}

fn current_nat_usage_window(window_seconds: u32) -> NatUsageWindow {
    let window_seconds = i64::from(window_seconds.max(1));
    let now = Utc::now();
    let timestamp = now.timestamp();
    let window_start = timestamp - timestamp.rem_euclid(window_seconds);
    NatUsageWindow {
        window_start: DateTime::<Utc>::from_timestamp(window_start, 0).unwrap_or(now),
    }
}

async fn ensure_nat_byte_limit(
    sent_total: u64,
    received_total: u64,
    max_bytes_per_tunnel: Option<u64>,
    io_registry: &IoRegistry,
    agent_id: &xlstatus_shared::AgentId,
    agent_id_str: &str,
    tunnel_id: &str,
    sequence: u64,
) -> Result<()> {
    let Some(limit) = max_bytes_per_tunnel else {
        return Ok(());
    };
    if sent_total.saturating_add(received_total) <= limit {
        return Ok(());
    }
    io_registry
        .send_to_agent(
            agent_id,
            IoFrame {
                stream_id: tunnel_id.to_string(),
                sequence,
                agent_id: agent_id_str.to_string(),
                payload: Some(io_frame::Payload::Close(IoClose {
                    reason: "NAT tunnel byte limit exceeded".to_string(),
                })),
            },
        )
        .await
        .map_err(|e| anyhow!(e))?;
    Err(anyhow!("NAT tunnel byte limit exceeded"))
}

async fn apply_nat_bandwidth_limit(
    sent_total: u64,
    received_total: u64,
    max_bandwidth_bytes_per_second: Option<u64>,
    started_at: tokio::time::Instant,
) {
    let Some(limit) = max_bandwidth_bytes_per_second else {
        return;
    };
    let total = sent_total.saturating_add(received_total);
    if total == 0 {
        return;
    }
    let expected_elapsed = tokio::time::Duration::from_secs_f64(total as f64 / limit as f64);
    let elapsed = started_at.elapsed();
    if expected_elapsed > elapsed {
        tokio::time::sleep(expected_elapsed - elapsed).await;
    }
}

fn nat_bind_addr(port: u16) -> String {
    let host = std::env::var("XLSTATUS_NAT_BIND_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    format!("{host}:{port}")
}

pub fn nat_public_port_allowed(port: u16) -> bool {
    let min = nat_public_port_min();
    port >= min
}

fn nat_public_port_min() -> u16 {
    std::env::var("XLSTATUS_NAT_PUBLIC_PORT_MIN")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_NAT_MIN_PUBLIC_PORT)
}

fn nat_max_active_tunnels() -> usize {
    std::env::var("XLSTATUS_NAT_MAX_ACTIVE_TUNNELS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_NAT_MAX_ACTIVE_TUNNELS)
}

fn nat_source_allowed(peer_addr: SocketAddr) -> bool {
    let peer_ip = peer_addr.ip();
    let raw = std::env::var("XLSTATUS_NAT_ALLOWED_SOURCES")
        .unwrap_or_else(|_| "127.0.0.0/8,::1/128".to_string());
    nat_source_list_allows(&raw, peer_ip)
}

fn nat_mapping_source_allowed(mapping: &NatMapping, peer_addr: SocketAddr) -> bool {
    let Some(raw) = mapping.allowed_sources.as_deref() else {
        return true;
    };
    nat_source_list_allows(raw, peer_addr.ip())
}

fn nat_source_list_allows(raw: &str, peer_ip: IpAddr) -> bool {
    raw.split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .any(|item| nat_source_entry_matches(item, peer_ip))
}

pub fn nat_source_entry_valid(entry: &str) -> bool {
    if entry.parse::<IpAddr>().is_ok() {
        return true;
    }
    let Some((network, prefix)) = entry.split_once('/') else {
        return false;
    };
    let Ok(network) = network.parse::<IpAddr>() else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    matches!(
        network,
        IpAddr::V4(_) if prefix <= 32
    ) || matches!(
        network,
        IpAddr::V6(_) if prefix <= 128
    )
}

fn nat_source_entry_matches(entry: &str, ip: IpAddr) -> bool {
    if let Ok(exact) = entry.parse::<IpAddr>() {
        return exact == ip;
    }
    let Some((network, prefix)) = entry.split_once('/') else {
        return false;
    };
    let Ok(network) = network.parse::<IpAddr>() else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    nat_ip_in_cidr(ip, network, prefix)
}

fn nat_ip_in_cidr(ip: IpAddr, network: IpAddr, prefix: u8) -> bool {
    match (ip, network) {
        (IpAddr::V4(ip), IpAddr::V4(network)) if prefix <= 32 => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            (u32::from(ip) & mask) == (u32::from(network) & mask)
        }
        (IpAddr::V6(ip), IpAddr::V6(network)) if prefix <= 128 => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            (u128::from(ip) & mask) == (u128::from(network) & mask)
        }
        _ => false,
    }
}

impl Clone for NatTunnelManager {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            io_registry: self.io_registry.clone(),
            listeners: self.listeners.clone(),
            active_tunnels: self.active_tunnels.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nat_listener_binds_loopback_by_default() {
        std::env::remove_var("XLSTATUS_NAT_BIND_HOST");
        assert_eq!(nat_bind_addr(18080), "127.0.0.1:18080");
    }

    #[test]
    fn nat_listener_bind_host_can_be_overridden() {
        std::env::set_var("XLSTATUS_NAT_BIND_HOST", "0.0.0.0");
        assert_eq!(nat_bind_addr(18080), "0.0.0.0:18080");
        std::env::remove_var("XLSTATUS_NAT_BIND_HOST");
    }

    #[test]
    fn nat_source_allowlist_defaults_to_loopback() {
        std::env::remove_var("XLSTATUS_NAT_ALLOWED_SOURCES");
        assert!(nat_source_allowed("127.0.0.1:12345".parse().unwrap()));
        assert!(nat_source_allowed("[::1]:12345".parse().unwrap()));
        assert!(!nat_source_allowed("192.0.2.10:12345".parse().unwrap()));
    }

    #[test]
    fn nat_source_allowlist_supports_cidrs() {
        std::env::set_var("XLSTATUS_NAT_ALLOWED_SOURCES", "203.0.113.0/24");
        assert!(nat_source_allowed("203.0.113.10:12345".parse().unwrap()));
        assert!(!nat_source_allowed("198.51.100.10:12345".parse().unwrap()));
        std::env::remove_var("XLSTATUS_NAT_ALLOWED_SOURCES");
    }

    #[test]
    fn nat_mapping_source_allowlist_is_additional_policy() {
        let mut mapping = test_mapping();
        mapping.allowed_sources = Some("203.0.113.0/24".into());

        assert!(nat_mapping_source_allowed(
            &mapping,
            "203.0.113.10:12345".parse().unwrap()
        ));
        assert!(!nat_mapping_source_allowed(
            &mapping,
            "198.51.100.10:12345".parse().unwrap()
        ));
    }

    #[test]
    fn nat_source_entry_validation_rejects_bad_cidrs() {
        assert!(nat_source_entry_valid("203.0.113.10"));
        assert!(nat_source_entry_valid("203.0.113.0/24"));
        assert!(nat_source_entry_valid("::1/128"));
        assert!(!nat_source_entry_valid("203.0.113.0/33"));
        assert!(!nat_source_entry_valid("not-a-cidr"));
    }

    #[test]
    fn nat_public_ports_reject_privileged_ports_by_default() {
        std::env::remove_var("XLSTATUS_NAT_PUBLIC_PORT_MIN");
        assert!(!nat_public_port_allowed(80));
        assert!(nat_public_port_allowed(1024));
    }

    #[test]
    fn nat_rate_limit_window_defaults_when_limits_configured() {
        let mut mapping = test_mapping();
        assert_eq!(nat_rate_limit_window_seconds(&mapping), None);

        mapping.max_connections_per_window = Some(10);
        assert_eq!(nat_rate_limit_window_seconds(&mapping), Some(60));

        mapping.rate_limit_window_seconds = Some(300);
        assert_eq!(nat_rate_limit_window_seconds(&mapping), Some(300));
    }

    #[test]
    fn test_tunnel_manager_creation() {
        // Placeholder: integration covered by verify scripts.
    }

    fn test_mapping() -> NatMapping {
        NatMapping {
            id: "mapping-1".into(),
            agent_id: "agent-1".into(),
            local_host: "127.0.0.1".into(),
            local_port: 80,
            public_port: 10080,
            protocol: Protocol::Tcp,
            enabled: true,
            description: None,
            allowed_sources: None,
            max_active_tunnels: None,
            idle_timeout_seconds: None,
            max_bytes_per_tunnel: None,
            max_bandwidth_bytes_per_second: None,
            rate_limit_window_seconds: None,
            max_connections_per_window: None,
            max_bytes_per_window: None,
            created_at: "2026-06-21T00:00:00Z".into(),
            updated_at: "2026-06-21T00:00:00Z".into(),
        }
    }
}
