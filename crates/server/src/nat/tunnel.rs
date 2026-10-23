use crate::db::repository::NatMappingRepository;
use crate::db::Db;
use crate::grpc::IoRegistry;
use crate::nat::policy::{
    nat_public_port_allowed, nat_source_list_allows, validate_nat_mapping_runtime_policy,
};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, RwLock};
use tracing::{error, info, warn};
use xlstatus_proto_gen::xlstatus::v1::{io_frame, IoClose, IoData, IoError, IoFrame};
use xlstatus_shared::nat::{NatMapping, NatTunnelControlMessage, Protocol, TunnelStats};

const DEFAULT_NAT_MAX_ACTIVE_TUNNELS: usize = 128;
pub const PRIVILEGED_PORT_MAX: u16 = 1023;
pub const DEFAULT_RESERVED_CONTROL_PLANE_PORTS: &[u16] = &[3000, 8080, 50051];
pub const MAX_TRACKED_SOURCE_IPS_PER_MAPPING: usize = 4096;
pub const MAX_NAT_FRAME_BYTES: usize = 64 * 1024;
const DEFAULT_NAT_HANDSHAKE_TIMEOUT_SECONDS: u64 = 15;
const DEFAULT_NAT_PUBLIC_WRITE_TIMEOUT_SECONDS: u64 = 30;

/// Per-source-IP rate-limiting state within a time window
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackedSourceRateState {
    pub window_start: DateTime<Utc>,
    pub connection_count: u64,
    pub bytes_transferred: u64,
    pub updated_at: DateTime<Utc>,
}

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
    rate_limits: Arc<RwLock<HashMap<String, HashMap<String, TrackedSourceRateState>>>>,
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
            rate_limits: Arc::new(RwLock::new(HashMap::new())),
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
            let mapping = match validate_nat_mapping_runtime_policy(mapping) {
                Ok(valid) => valid,
                Err(err) => {
                    warn!("Skipping NAT mapping with invalid runtime policy: {}", err);
                    continue;
                }
            };
            if let Err(err) = validate_nat_bind_port(mapping.public_port) {
                warn!(
                    "Skipping NAT mapping {} on disallowed public port {}: {}",
                    mapping.id, mapping.public_port, err
                );
                continue;
            }
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
        let active_mapping_ids: std::collections::HashSet<&str> = tcp_mappings
            .iter()
            .map(|mapping| mapping.id.as_str())
            .collect();

        {
            let mut rate_limits = self.rate_limits.write().await;
            rate_limits.retain(|mapping_id, _| active_mapping_ids.contains(mapping_id.as_str()));
        }

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
        let mapping = validate_nat_mapping_runtime_policy(mapping).map_err(anyhow::Error::msg)?;
        validate_nat_bind_port(mapping.public_port)?;

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
        if validate_nat_bind_port(public_port).is_err() {
            return Ok(None);
        }
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

    /// Verify that `tunnel_id` is an active NAT tunnel owned by `sender_agent_id`.
    pub async fn verify_tunnel_ownership(
        &self,
        tunnel_id: &str,
        sender_agent_id: &xlstatus_shared::AgentId,
    ) -> Result<()> {
        if tunnel_id.is_empty() || tunnel_id.chars().any(char::is_control) {
            return Err(anyhow!("invalid NAT tunnel id"));
        }
        let active_tunnels = self.active_tunnels.read().await;
        let Some(tunnel) = active_tunnels.get(tunnel_id) else {
            return Err(anyhow!("unknown NAT tunnel {}", tunnel_id));
        };
        if !verify_agent_id_matches(sender_agent_id, &tunnel.agent_id) {
            return Err(anyhow!(
                "agent {} is not the owner of NAT tunnel {}",
                sender_agent_id,
                tunnel_id
            ));
        }
        Ok(())
    }

    /// Validate agent ownership and frame bounds before delivering an `IoFrame` to an active NAT tunnel.
    pub async fn deliver_agent_frame(
        &self,
        sender_agent_id: &xlstatus_shared::AgentId,
        frame: IoFrame,
    ) -> Result<()> {
        self.verify_tunnel_ownership(&frame.stream_id, sender_agent_id)
            .await?;
        if !frame.agent_id.is_empty() && !verify_agent_id_matches(sender_agent_id, &frame.agent_id)
        {
            return Err(anyhow!(
                "frame agent_id {} does not match authenticated sender {}",
                frame.agent_id,
                sender_agent_id
            ));
        }
        if let Some(io_frame::Payload::Data(ref data)) = frame.payload {
            if data.data.len() > MAX_NAT_FRAME_BYTES {
                return Err(anyhow!(
                    "NAT frame payload ({} bytes) exceeds maximum ({} bytes)",
                    data.data.len(),
                    MAX_NAT_FRAME_BYTES
                ));
            }
        }
        if !self
            .io_registry
            .deliver_from_agent(sender_agent_id, frame)
            .await
        {
            return Err(anyhow!("failed to deliver NAT frame to active tunnel"));
        }
        Ok(())
    }

    async fn handle_connection(
        &self,
        public_stream: TcpStream,
        mapping: NatMapping,
        peer_addr: SocketAddr,
        usage_window: Option<NatUsageWindowKey>,
    ) -> Result<()> {
        validate_nat_bind_port(mapping.public_port)?;
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
        let bytes_sent = tunnel.bytes_sent.clone();
        let bytes_received = tunnel.bytes_received.clone();
        {
            let mut active_tunnels = self.active_tunnels.write().await;
            reserve_active_tunnel(
                &mut active_tunnels,
                tunnel,
                nat_max_active_tunnels(),
                mapping.max_active_tunnels.map(|limit| limit as usize),
            )?;
        }

        // Everything after the reservation lives inside this result block so
        // the cleanup below runs for open-send failures as well as bridge exits.
        let result = async {
            let mut inbound = self
                .io_registry
                .subscribe_stream(tunnel_id.clone(), agent_id)
                .await;
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

            Self::bridge_over_iostream(
                public_stream,
                agent_id,
                mapping.agent_id.clone(),
                tunnel_id.clone(),
                self.active_tunnels.clone(),
                self.io_registry.clone(),
                &mut inbound,
                bytes_sent,
                bytes_received,
                mapping.idle_timeout_seconds,
                mapping.max_bytes_per_tunnel,
                mapping.max_bandwidth_bytes_per_second,
            )
            .await
        }
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
        let source_ip = peer_addr.ip().to_string();
        let effective_window_seconds = nat_rate_limit_window_seconds(mapping).unwrap_or(60);
        let window = current_nat_usage_window(effective_window_seconds);
        let now = Utc::now();

        {
            let mut rate_limits = self.rate_limits.write().await;
            let mapping_sources = rate_limits.entry(mapping.id.clone()).or_default();
            record_source_connection_in_map(
                mapping_sources,
                &source_ip,
                window.window_start,
                now,
                MAX_TRACKED_SOURCE_IPS_PER_MAPPING,
                mapping.max_connections_per_window,
                mapping.max_bytes_per_window,
            )?;
        }

        let Some(window_seconds) = nat_rate_limit_window_seconds(mapping) else {
            return Ok(None);
        };
        NatMappingRepository::prune_usage_windows(
            &self.db,
            now - Duration::seconds((window_seconds as i64).saturating_mul(4)),
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
        {
            let mut rate_limits = self.rate_limits.write().await;
            if let Some(mapping_sources) = rate_limits.get_mut(&mapping.id) {
                record_source_bytes_in_map(
                    mapping_sources,
                    &window.source_ip,
                    window.window_start,
                    bytes,
                    Utc::now(),
                    MAX_TRACKED_SOURCE_IPS_PER_MAPPING,
                );
            }
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

    #[allow(clippy::too_many_arguments)]
    async fn bridge_over_iostream(
        mut public_stream: TcpStream,
        agent_id: xlstatus_shared::AgentId,
        agent_id_str: String,
        tunnel_id: String,
        active_tunnels: Arc<RwLock<HashMap<String, ActiveTunnel>>>,
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
        let handshake_timeout = idle_timeout.unwrap_or_else(|| {
            tokio::time::Duration::from_secs(DEFAULT_NAT_HANDSHAKE_TIMEOUT_SECONDS)
        });
        let public_write_timeout = idle_timeout.unwrap_or_else(|| {
            tokio::time::Duration::from_secs(DEFAULT_NAT_PUBLIC_WRITE_TIMEOUT_SECONDS)
        });
        let started_at = tokio::time::Instant::now();

        loop {
            let effective_timeout = if !saw_ready {
                Some(handshake_timeout)
            } else {
                idle_timeout
            };
            let event = if let Some(timeout) = effective_timeout {
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
                    if let Err(reason) = verify_inbound_tunnel_frame(&tunnel_id, &agent_id, &frame)
                    {
                        warn!(
                            "Dropping unauthorized or mismatched NAT frame on tunnel {}: {}",
                            tunnel_id, reason
                        );
                        continue;
                    }
                    let owns_active_tunnel = {
                        let tunnels = active_tunnels.read().await;
                        tunnels.get(&tunnel_id).is_some_and(|tunnel| {
                            verify_agent_id_matches(&agent_id, &tunnel.agent_id)
                        })
                    };
                    if !owns_active_tunnel {
                        warn!(
                            "Dropping NAT frame for unowned or inactive tunnel {} from agent {}",
                            tunnel_id, agent_id
                        );
                        break;
                    }
                    match frame.payload {
                        Some(io_frame::Payload::Data(data)) => {
                            if data.data.len() > MAX_NAT_FRAME_BYTES {
                                let _ = io_registry
                                    .send_to_agent(
                                        &agent_id,
                                        IoFrame {
                                            stream_id: tunnel_id.clone(),
                                            sequence,
                                            agent_id: agent_id_str.clone(),
                                            payload: Some(io_frame::Payload::Close(IoClose {
                                                reason: "NAT tunnel frame size limit exceeded"
                                                    .to_string(),
                                            })),
                                        },
                                    )
                                    .await;
                                return Err(anyhow!(
                                    "NAT tunnel frame size {} exceeds maximum {}",
                                    data.data.len(),
                                    MAX_NAT_FRAME_BYTES
                                ));
                            }
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
                            tokio::time::timeout(
                                public_write_timeout,
                                public_write.write_all(&data.data),
                            )
                            .await
                            .map_err(|_| anyhow!("NAT public stream write timeout"))??;
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

fn verify_agent_id_matches(
    expected_agent_id: &xlstatus_shared::AgentId,
    candidate_agent_id: &str,
) -> bool {
    let candidate = candidate_agent_id.trim();
    if candidate.is_empty() {
        return false;
    }
    uuid::Uuid::parse_str(candidate)
        .map(|parsed| parsed == expected_agent_id.0)
        .unwrap_or(false)
}

fn verify_inbound_tunnel_frame(
    expected_tunnel_id: &str,
    expected_agent_id: &xlstatus_shared::AgentId,
    frame: &IoFrame,
) -> Result<()> {
    if frame.stream_id != expected_tunnel_id {
        return Err(anyhow!(
            "frame stream_id '{}' does not match tunnel_id '{}'",
            frame.stream_id,
            expected_tunnel_id
        ));
    }
    if !verify_agent_id_matches(expected_agent_id, &frame.agent_id) {
        return Err(anyhow!(
            "frame agent_id '{}' does not match tunnel owner '{}'",
            frame.agent_id,
            expected_agent_id
        ));
    }
    Ok(())
}

pub fn is_reserved_control_plane_port(port: u16) -> bool {
    if DEFAULT_RESERVED_CONTROL_PLANE_PORTS.contains(&port) {
        return true;
    }
    for env_name in [
        "HTTP_BIND",
        "GRPC_BIND",
        "XLSTATUS_HTTP_BIND",
        "XLSTATUS_GRPC_BIND",
    ] {
        if let Ok(bind_value) = std::env::var(env_name) {
            if let Some(bound_port) = parse_port_from_bind_spec(&bind_value) {
                if bound_port == port {
                    return true;
                }
            }
        }
    }
    if let Ok(raw_ports) = std::env::var("XLSTATUS_NAT_RESERVED_PORTS") {
        for token in raw_ports.split([',', ' ', '\t', '\n']) {
            if let Ok(reserved) = token.trim().parse::<u16>() {
                if reserved == port {
                    return true;
                }
            }
        }
    }
    false
}

fn parse_port_from_bind_spec(spec: &str) -> Option<u16> {
    let spec = spec.trim();
    if let Ok(addr) = spec.parse::<SocketAddr>() {
        return Some(addr.port());
    }
    spec.rsplit_once(':')
        .and_then(|(_, port_str)| port_str.trim().parse::<u16>().ok())
}

pub fn validate_nat_bind_port(port: u16) -> Result<()> {
    if port <= PRIVILEGED_PORT_MAX {
        return Err(anyhow!(
            "NAT public_port {} is a privileged port (<= {})",
            port,
            PRIVILEGED_PORT_MAX
        ));
    }
    if !nat_public_port_allowed(port) {
        return Err(anyhow!(
            "NAT public_port {} is below the configured NAT public port minimum",
            port
        ));
    }
    if is_reserved_control_plane_port(port) {
        return Err(anyhow!(
            "NAT public_port {} collides with a reserved control plane port",
            port
        ));
    }
    Ok(())
}

pub(crate) fn prune_expired_source_rate_windows(
    sources: &mut HashMap<String, TrackedSourceRateState>,
    min_window_start: DateTime<Utc>,
) {
    sources.retain(|_, state| state.window_start >= min_window_start);
}

pub(crate) fn record_source_connection_in_map(
    sources: &mut HashMap<String, TrackedSourceRateState>,
    source_ip: &str,
    window_start: DateTime<Utc>,
    now: DateTime<Utc>,
    max_tracked_ips: usize,
    max_connections_per_window: Option<u32>,
    max_bytes_per_window: Option<u64>,
) -> Result<TrackedSourceRateState> {
    prune_expired_source_rate_windows(sources, window_start);

    let source_ip_norm = source_ip.trim();
    if source_ip_norm.is_empty() {
        return Err(anyhow!("NAT source IP must not be empty"));
    }

    if !sources.contains_key(source_ip_norm) && sources.len() >= max_tracked_ips {
        return Err(anyhow!(
            "NAT mapping tracked source IP limit ({}) exceeded for source {}",
            max_tracked_ips,
            source_ip_norm
        ));
    }

    let existing = sources.get(source_ip_norm).cloned();
    let (mut conn_count, byte_count) = match existing {
        Some(ref state) if state.window_start == window_start => {
            (state.connection_count, state.bytes_transferred)
        }
        _ => (0, 0),
    };

    conn_count = conn_count.saturating_add(1);
    if let Some(limit) = max_connections_per_window {
        if conn_count > u64::from(limit) {
            let state = TrackedSourceRateState {
                window_start,
                connection_count: conn_count,
                bytes_transferred: byte_count,
                updated_at: now,
            };
            sources.insert(source_ip_norm.to_string(), state);
            return Err(anyhow!(
                "NAT mapping connection rate limit exceeded for source {}",
                source_ip_norm
            ));
        }
    }
    if let Some(limit) = max_bytes_per_window {
        if byte_count > limit {
            return Err(anyhow!(
                "NAT mapping byte window limit exceeded for source {}",
                source_ip_norm
            ));
        }
    }

    let updated = TrackedSourceRateState {
        window_start,
        connection_count: conn_count,
        bytes_transferred: byte_count,
        updated_at: now,
    };
    sources.insert(source_ip_norm.to_string(), updated.clone());
    Ok(updated)
}

pub(crate) fn record_source_bytes_in_map(
    sources: &mut HashMap<String, TrackedSourceRateState>,
    source_ip: &str,
    window_start: DateTime<Utc>,
    bytes: u64,
    now: DateTime<Utc>,
    max_tracked_ips: usize,
) -> Option<TrackedSourceRateState> {
    prune_expired_source_rate_windows(sources, window_start);
    let source_ip_norm = source_ip.trim();
    if source_ip_norm.is_empty() {
        return None;
    }
    if !sources.contains_key(source_ip_norm) && sources.len() >= max_tracked_ips {
        return None;
    }
    let entry = sources
        .entry(source_ip_norm.to_string())
        .or_insert_with(|| TrackedSourceRateState {
            window_start,
            connection_count: 0,
            bytes_transferred: 0,
            updated_at: now,
        });
    if entry.window_start != window_start {
        entry.window_start = window_start;
        entry.connection_count = 0;
        entry.bytes_transferred = 0;
    }
    entry.bytes_transferred = entry.bytes_transferred.saturating_add(bytes);
    entry.updated_at = now;
    Some(entry.clone())
}

fn reserve_active_tunnel(
    active_tunnels: &mut HashMap<String, ActiveTunnel>,
    tunnel: ActiveTunnel,
    global_limit: usize,
    mapping_limit: Option<usize>,
) -> Result<()> {
    if active_tunnels.len() >= global_limit {
        return Err(anyhow!("active tunnel limit reached"));
    }
    if let Some(limit) = mapping_limit {
        let mapping_count = active_tunnels
            .values()
            .filter(|active| active.mapping_id == tunnel.mapping_id)
            .count();
        if mapping_count >= limit {
            return Err(anyhow!("mapping active tunnel limit reached"));
        }
    }
    if active_tunnels.contains_key(&tunnel.tunnel_id) {
        return Err(anyhow!("duplicate tunnel id"));
    }
    active_tunnels.insert(tunnel.tunnel_id.clone(), tunnel);
    Ok(())
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

impl Clone for NatTunnelManager {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            io_registry: self.io_registry.clone(),
            listeners: self.listeners.clone(),
            active_tunnels: self.active_tunnels.clone(),
            rate_limits: self.rate_limits.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nat::policy::{nat_public_port_allowed, nat_source_entry_valid};

    static NAT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn nat_listener_binds_loopback_by_default() {
        let _lock = NAT_ENV_LOCK.lock().unwrap();
        std::env::remove_var("XLSTATUS_NAT_BIND_HOST");
        assert_eq!(nat_bind_addr(18080), "127.0.0.1:18080");
    }

    #[test]
    fn nat_listener_bind_host_can_be_overridden() {
        let _lock = NAT_ENV_LOCK.lock().unwrap();
        std::env::set_var("XLSTATUS_NAT_BIND_HOST", "0.0.0.0");
        assert_eq!(nat_bind_addr(18080), "0.0.0.0:18080");
        std::env::remove_var("XLSTATUS_NAT_BIND_HOST");
    }

    #[test]
    fn nat_source_allowlist_defaults_to_loopback() {
        let _lock = NAT_ENV_LOCK.lock().unwrap();
        std::env::remove_var("XLSTATUS_NAT_ALLOWED_SOURCES");
        assert!(nat_source_allowed("127.0.0.1:12345".parse().unwrap()));
        assert!(nat_source_allowed("[::1]:12345".parse().unwrap()));
        assert!(!nat_source_allowed("192.0.2.10:12345".parse().unwrap()));
    }

    #[test]
    fn nat_source_allowlist_supports_cidrs() {
        let _lock = NAT_ENV_LOCK.lock().unwrap();
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
        let _lock = NAT_ENV_LOCK.lock().unwrap();
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

    #[test]
    fn active_tunnel_reservation_enforces_mapping_and_global_limits_atomically() {
        let mut active = HashMap::new();
        reserve_active_tunnel(
            &mut active,
            test_active_tunnel("one", "mapping-1"),
            2,
            Some(1),
        )
        .unwrap();

        let mapping_error = reserve_active_tunnel(
            &mut active,
            test_active_tunnel("two", "mapping-1"),
            2,
            Some(1),
        )
        .unwrap_err();
        assert!(mapping_error.to_string().contains("mapping active"));

        reserve_active_tunnel(&mut active, test_active_tunnel("two", "mapping-2"), 2, None)
            .unwrap();
        let global_error = reserve_active_tunnel(
            &mut active,
            test_active_tunnel("three", "mapping-3"),
            2,
            None,
        )
        .unwrap_err();
        assert!(global_error.to_string().contains("active tunnel limit"));
        assert_eq!(active.len(), 2);
    }

    #[tokio::test]
    async fn nat_open_send_failure_releases_tunnel_and_subscriber() {
        let db = Db::connect("sqlite::memory:", true).await.unwrap();
        let registry = IoRegistry::new();
        let agent_id = xlstatus_shared::AgentId(uuid::Uuid::now_v7());
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(rx);
        assert!(registry.register_agent(agent_id, tx).await);
        let manager = NatTunnelManager::new(db, registry.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, peer_addr) = listener.accept().await.unwrap();

        let mut mapping = test_mapping();
        mapping.agent_id = agent_id.0.to_string();
        let error = manager
            .handle_connection(server, mapping, peer_addr, None)
            .await
            .unwrap_err();
        drop(client);

        assert!(error
            .to_string()
            .contains("failed to send NAT open request"));
        assert_eq!(manager.active_tunnel_count().await, 0);
        assert_eq!(registry.subscriber_count().await, 0);
    }

    #[test]
    fn nat_bind_port_rejects_privileged_and_reserved_control_plane_ports() {
        let _lock = NAT_ENV_LOCK.lock().unwrap();
        std::env::remove_var("XLSTATUS_NAT_PUBLIC_PORT_MIN");
        std::env::remove_var("XLSTATUS_NAT_RESERVED_PORTS");

        for privileged in [0_u16, 22, 80, 443, 1023] {
            let err = validate_nat_bind_port(privileged).unwrap_err();
            assert!(err.to_string().contains("privileged"));
        }

        for reserved in [3000_u16, 8080, 50051] {
            let err = validate_nat_bind_port(reserved).unwrap_err();
            assert!(err.to_string().contains("reserved control plane port"));
        }

        std::env::set_var("XLSTATUS_NAT_RESERVED_PORTS", "19090, 19091");
        assert!(validate_nat_bind_port(19090).is_err());
        assert!(validate_nat_bind_port(19091).is_err());
        std::env::remove_var("XLSTATUS_NAT_RESERVED_PORTS");

        assert!(validate_nat_bind_port(10080).is_ok());
    }

    #[tokio::test]
    async fn nat_start_listener_rejects_privileged_and_in_use_ports_without_panic() {
        let db = Db::connect("sqlite::memory:", true).await.unwrap();
        let registry = IoRegistry::new();
        let manager = NatTunnelManager::new(db, registry);

        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();

        let mut bad_port_mapping = test_mapping();
        bad_port_mapping.id = uuid::Uuid::now_v7().to_string();
        bad_port_mapping.agent_id = uuid::Uuid::now_v7().to_string();
        bad_port_mapping.public_port = 80;
        assert!(manager.start_listener(bad_port_mapping).await.is_err());

        let mut conflict_mapping = test_mapping();
        conflict_mapping.id = uuid::Uuid::now_v7().to_string();
        conflict_mapping.agent_id = uuid::Uuid::now_v7().to_string();
        conflict_mapping.public_port = occupied_port;
        let bind_err = manager.start_listener(conflict_mapping).await.unwrap_err();
        assert!(bind_err.to_string().contains("Failed to bind"));
        assert!(!manager.listeners.read().await.contains_key(&occupied_port));
    }

    #[test]
    fn rate_limit_map_bounds_tracked_source_ips_and_prunes_expired_windows() {
        let mut sources = HashMap::new();
        let window_1 = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let now_1 = window_1 + Duration::seconds(5);
        let cap = 16;

        for idx in 0..cap {
            let ip = format!("203.0.113.{idx}");
            record_source_connection_in_map(&mut sources, &ip, window_1, now_1, cap, Some(3), None)
                .unwrap();
        }
        assert_eq!(sources.len(), cap);

        // Existing tracked IP within the cap can still increment up to its per-window limit
        record_source_connection_in_map(
            &mut sources,
            "203.0.113.0",
            window_1,
            now_1,
            cap,
            Some(3),
            None,
        )
        .unwrap();

        // A new distinct source IP beyond the cap in the same window is rejected (DoS protection)
        let overflow_err = record_source_connection_in_map(
            &mut sources,
            "198.51.100.99",
            window_1,
            now_1,
            cap,
            Some(3),
            None,
        )
        .unwrap_err();
        assert!(overflow_err.to_string().contains("tracked source IP limit"));
        assert_eq!(sources.len(), cap);

        // Advancing to a new window prunes all expired entries and allows new source IPs
        let window_2 = window_1 + Duration::seconds(60);
        let now_2 = window_2 + Duration::seconds(1);
        let state = record_source_connection_in_map(
            &mut sources,
            "198.51.100.99",
            window_2,
            now_2,
            cap,
            Some(3),
            None,
        )
        .unwrap();
        assert_eq!(state.connection_count, 1);
        assert_eq!(sources.len(), 1);
    }

    #[tokio::test]
    async fn cross_agent_tunnel_id_hijack_is_rejected_for_data_and_close_frames() {
        let db = Db::connect("sqlite::memory:", true).await.unwrap();
        let registry = IoRegistry::new();
        let owner_agent = xlstatus_shared::AgentId(uuid::Uuid::now_v7());
        let attacker_agent = xlstatus_shared::AgentId(uuid::Uuid::now_v7());
        let (owner_tx, _owner_rx) = tokio::sync::mpsc::channel(8);
        let (attacker_tx, _attacker_rx) = tokio::sync::mpsc::channel(8);
        assert!(registry.register_agent(owner_agent, owner_tx).await);
        assert!(registry.register_agent(attacker_agent, attacker_tx).await);

        let manager = NatTunnelManager::new(db, registry.clone());
        let tunnel_id = "tunnel-owned-by-b".to_string();
        manager.active_tunnels.write().await.insert(
            tunnel_id.clone(),
            ActiveTunnel {
                tunnel_id: tunnel_id.clone(),
                mapping_id: "mapping-1".into(),
                agent_id: owner_agent.0.to_string(),
                created_at: Utc::now(),
                bytes_sent: Arc::new(RwLock::new(0)),
                bytes_received: Arc::new(RwLock::new(0)),
            },
        );
        let mut inbound = registry
            .subscribe_stream(tunnel_id.clone(), owner_agent)
            .await;

        // Attacker Agent A cannot verify ownership or deliver Data / Close frames to Agent B's tunnel
        assert!(manager
            .verify_tunnel_ownership(&tunnel_id, &attacker_agent)
            .await
            .is_err());
        assert!(manager
            .deliver_agent_frame(
                &attacker_agent,
                IoFrame {
                    stream_id: tunnel_id.clone(),
                    sequence: 2,
                    agent_id: attacker_agent.0.to_string(),
                    payload: Some(io_frame::Payload::Data(IoData {
                        data: b"injected".to_vec(),
                    })),
                },
            )
            .await
            .is_err());
        assert!(manager
            .deliver_agent_frame(
                &attacker_agent,
                IoFrame {
                    stream_id: tunnel_id.clone(),
                    sequence: 3,
                    agent_id: attacker_agent.0.to_string(),
                    payload: Some(io_frame::Payload::Close(IoClose {
                        reason: "spoofed close".into(),
                    })),
                },
            )
            .await
            .is_err());
        assert!(inbound.try_recv().is_err());

        // Direct inbound frame validation also rejects mismatched agent_id or stream_id
        assert!(verify_inbound_tunnel_frame(
            &tunnel_id,
            &owner_agent,
            &IoFrame {
                stream_id: tunnel_id.clone(),
                sequence: 2,
                agent_id: attacker_agent.0.to_string(),
                payload: Some(io_frame::Payload::Close(IoClose {
                    reason: "spoofed".into(),
                })),
            }
        )
        .is_err());

        // Legitimate owning Agent B succeeds and oversized frames are rejected
        assert!(manager
            .deliver_agent_frame(
                &owner_agent,
                IoFrame {
                    stream_id: tunnel_id.clone(),
                    sequence: 2,
                    agent_id: owner_agent.0.to_string(),
                    payload: Some(io_frame::Payload::Data(IoData {
                        data: vec![0u8; MAX_NAT_FRAME_BYTES + 1],
                    })),
                },
            )
            .await
            .is_err());
        assert!(manager
            .deliver_agent_frame(
                &owner_agent,
                IoFrame {
                    stream_id: tunnel_id.clone(),
                    sequence: 2,
                    agent_id: owner_agent.0.to_string(),
                    payload: Some(io_frame::Payload::Data(IoData {
                        data: b"legitimate".to_vec(),
                    })),
                },
            )
            .await
            .is_ok());
        let delivered = inbound.try_recv().unwrap();
        assert_eq!(delivered.agent_id, owner_agent.0.to_string());
    }

    fn test_active_tunnel(tunnel_id: &str, mapping_id: &str) -> ActiveTunnel {
        ActiveTunnel {
            tunnel_id: tunnel_id.into(),
            mapping_id: mapping_id.into(),
            agent_id: "agent-1".into(),
            created_at: Utc::now(),
            bytes_sent: Arc::new(RwLock::new(0)),
            bytes_received: Arc::new(RwLock::new(0)),
        }
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
