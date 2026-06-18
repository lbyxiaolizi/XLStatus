use crate::db::repository::NatMappingRepository;
use crate::db::Db;
use crate::grpc::IoRegistry;
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use xlstatus_proto_gen::xlstatus::v1::{io_frame, IoClose, IoData, IoError, IoFrame};
use xlstatus_shared::nat::{NatMapping, NatTunnelControlMessage, Protocol, TunnelStats};

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
    listeners: Arc<RwLock<HashMap<u16, Arc<TcpListener>>>>,
    active_tunnels: Arc<RwLock<HashMap<String, ActiveTunnel>>>,
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
        let mappings = NatMappingRepository::list_enabled(&self.db)
            .await
            .context("Failed to load NAT mappings")?;
        let desired_ports: std::collections::HashSet<u16> =
            mappings.iter().map(|mapping| mapping.public_port).collect();

        {
            let mut listeners = self.listeners.write().await;
            listeners.retain(|port, _| desired_ports.contains(port));
        }

        for mapping in mappings {
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

        let addr = format!("0.0.0.0:{}", mapping.public_port);
        let listener = TcpListener::bind(&addr)
            .await
            .context(format!("Failed to bind to {}", addr))?;

        info!(
            "NAT listener started on port {} -> {}:{} via agent {}",
            mapping.public_port, mapping.local_host, mapping.local_port, mapping.agent_id
        );

        let listener = Arc::new(listener);
        self.listeners
            .write()
            .await
            .insert(mapping.public_port, listener.clone());

        let manager = Arc::new(self.clone());
        let mapping_clone = mapping.clone();
        tokio::spawn(async move {
            if let Err(e) = manager.accept_loop(listener, mapping_clone).await {
                error!("Accept loop error: {}", e);
            }
        });

        Ok(())
    }

    async fn accept_loop(
        self: Arc<Self>,
        listener: Arc<TcpListener>,
        mapping: NatMapping,
    ) -> Result<()> {
        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    info!(
                        "New NAT connection on port {} from {}",
                        mapping.public_port, peer_addr
                    );

                    let manager = self.clone();
                    let mapping_clone = mapping.clone();
                    tokio::spawn(async move {
                        if let Err(e) = manager.handle_connection(stream, mapping_clone).await {
                            error!("NAT connection handling error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error on port {}: {}", mapping.public_port, e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn handle_connection(&self, public_stream: TcpStream, mapping: NatMapping) -> Result<()> {
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
        )
        .await;

        self.io_registry.unsubscribe_stream(&tunnel_id).await;
        self.active_tunnels.write().await.remove(&tunnel_id);

        match &result {
            Ok((sent, received)) => {
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

    async fn bridge_over_iostream(
        mut public_stream: TcpStream,
        agent_id: xlstatus_shared::AgentId,
        agent_id_str: String,
        tunnel_id: String,
        io_registry: IoRegistry,
        inbound: &mut tokio::sync::mpsc::Receiver<IoFrame>,
        bytes_sent: Arc<RwLock<u64>>,
        bytes_received: Arc<RwLock<u64>>,
    ) -> Result<(u64, u64)> {
        let (mut public_read, mut public_write) = public_stream.split();

        let to_agent = async {
            let mut sequence = 2_u64;
            let mut total = 0_u64;
            let mut buf = [0_u8; 8192];
            loop {
                let n = public_read.read(&mut buf).await?;
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
                total += n as u64;
                *bytes_sent.write().await += n as u64;
                sequence = sequence.saturating_add(1);
            }
            Ok::<u64, anyhow::Error>(total)
        };

        let from_agent = async {
            let mut total = 0_u64;
            let mut saw_ready = false;
            while let Some(frame) = inbound.recv().await {
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
                        total += data.data.len() as u64;
                        *bytes_received.write().await += data.data.len() as u64;
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
            Ok::<u64, anyhow::Error>(total)
        };

        tokio::try_join!(to_agent, from_agent)
    }

    pub async fn active_tunnel_count(&self) -> usize {
        self.active_tunnels.read().await.len()
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
    fn test_tunnel_manager_creation() {
        // Placeholder: integration covered by verify scripts.
    }
}
