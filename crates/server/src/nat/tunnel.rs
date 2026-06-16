use crate::db::repository::NatMappingRepository;
use crate::db::Db;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use xlstatus_shared::nat::*;

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
    // Map of public_port -> listener
    listeners: Arc<RwLock<HashMap<u16, Arc<TcpListener>>>>,
    // Map of tunnel_id -> tunnel info
    active_tunnels: Arc<RwLock<HashMap<String, ActiveTunnel>>>,
}

impl NatTunnelManager {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            listeners: Arc::new(RwLock::new(HashMap::new())),
            active_tunnels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start NAT tunnel manager
    pub async fn start(self: Arc<Self>) -> Result<()> {
        info!("Starting NAT tunnel manager");

        // Load all enabled NAT mappings and start listeners
        let mappings = NatMappingRepository::list_enabled(&self.db)
            .await
            .context("Failed to load NAT mappings")?;

        for mapping in mappings {
            if let Err(e) = self.start_listener(mapping).await {
                error!("Failed to start listener: {}", e);
            }
        }

        info!("NAT tunnel manager started with {} mappings", self.listeners.read().await.len());
        Ok(())
    }

    /// Start listener for a NAT mapping
    async fn start_listener(&self, mapping: NatMapping) -> Result<()> {
        // Only support TCP for now
        if !matches!(mapping.protocol, Protocol::Tcp) {
            warn!("UDP tunnels not yet supported, skipping mapping {}", mapping.id);
            return Ok(());
        }

        let addr = format!("0.0.0.0:{}", mapping.public_port);
        let listener = TcpListener::bind(&addr)
            .await
            .context(format!("Failed to bind to {}", addr))?;

        info!(
            "NAT listener started on port {} -> {}:{}",
            mapping.public_port, mapping.local_host, mapping.local_port
        );

        let listener = Arc::new(listener);
        self.listeners
            .write()
            .await
            .insert(mapping.public_port, listener.clone());

        // Spawn accept loop
        let manager = Arc::new(self.clone());
        let mapping_clone = mapping.clone();
        tokio::spawn(async move {
            if let Err(e) = manager.accept_loop(listener, mapping_clone).await {
                error!("Accept loop error: {}", e);
            }
        });

        Ok(())
    }

    /// Accept incoming connections
    async fn accept_loop(
        self: Arc<Self>,
        listener: Arc<TcpListener>,
        mapping: NatMapping,
    ) -> Result<()> {
        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    info!(
                        "New connection on port {} from {}",
                        mapping.public_port, peer_addr
                    );

                    let manager = self.clone();
                    let mapping_clone = mapping.clone();
                    tokio::spawn(async move {
                        if let Err(e) = manager.handle_connection(stream, mapping_clone).await {
                            error!("Connection handling error: {}", e);
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

    /// Handle a single connection
    async fn handle_connection(&self, mut public_stream: TcpStream, mapping: NatMapping) -> Result<()> {
        let tunnel_id = uuid::Uuid::now_v7().to_string();

        info!(
            "Creating tunnel {} for mapping {} -> {}:{}",
            tunnel_id, mapping.id, mapping.local_host, mapping.local_port
        );

        // TODO: Send tunnel request to agent via gRPC
        // For now, we'll try to connect directly to the local service
        // This is a simplified implementation for demonstration
        let local_addr = format!("{}:{}", mapping.local_host, mapping.local_port);
        let mut local_stream = match TcpStream::connect(&local_addr).await {
            Ok(stream) => stream,
            Err(e) => {
                error!("Failed to connect to local service {}: {}", local_addr, e);
                return Err(e.into());
            }
        };

        info!("Connected to local service {}", local_addr);

        // Track tunnel
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

        // Bidirectional copy
        let bytes_sent = self.active_tunnels.read().await
            .get(&tunnel_id)
            .unwrap()
            .bytes_sent
            .clone();
        let bytes_received = self.active_tunnels.read().await
            .get(&tunnel_id)
            .unwrap()
            .bytes_received
            .clone();

        let result = tokio::select! {
            res = Self::copy_bidirectional(&mut public_stream, &mut local_stream, bytes_sent, bytes_received) => res,
            _ = tokio::signal::ctrl_c() => {
                info!("Shutting down tunnel {}", tunnel_id);
                Ok((0, 0))
            }
        };

        // Clean up
        self.active_tunnels.write().await.remove(&tunnel_id);

        match result {
            Ok((sent, received)) => {
                info!(
                    "Tunnel {} closed: {} bytes sent, {} bytes received",
                    tunnel_id, sent, received
                );
            }
            Err(e) => {
                error!("Tunnel {} error: {}", tunnel_id, e);
            }
        }

        Ok(())
    }

    /// Bidirectional copy between two streams
    async fn copy_bidirectional(
        stream_a: &mut TcpStream,
        stream_b: &mut TcpStream,
        bytes_sent: Arc<RwLock<u64>>,
        bytes_received: Arc<RwLock<u64>>,
    ) -> Result<(u64, u64)> {
        let (mut a_read, mut a_write) = stream_a.split();
        let (mut b_read, mut b_write) = stream_b.split();

        let a_to_b = async {
            let mut buf = vec![0u8; 8192];
            let mut total = 0u64;
            loop {
                let n = a_read.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                b_write.write_all(&buf[..n]).await?;
                total += n as u64;
                *bytes_sent.write().await += n as u64;
            }
            Ok::<_, std::io::Error>(total)
        };

        let b_to_a = async {
            let mut buf = vec![0u8; 8192];
            let mut total = 0u64;
            loop {
                let n = b_read.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                a_write.write_all(&buf[..n]).await?;
                total += n as u64;
                *bytes_received.write().await += n as u64;
            }
            Ok::<_, std::io::Error>(total)
        };

        tokio::try_join!(a_to_b, b_to_a).map_err(|e| e.into())
    }

    /// Get active tunnel count
    pub async fn active_tunnel_count(&self) -> usize {
        self.active_tunnels.read().await.len()
    }

    /// Get statistics for all active tunnels
    pub async fn get_statistics(&self) -> Vec<TunnelStats> {
        let tunnels = self.active_tunnels.read().await;
        let mut stats = Vec::new();

        for tunnel in tunnels.values() {
            stats.push(TunnelStats {
                tunnel_id: tunnel.tunnel_id.clone(),
                bytes_sent: *tunnel.bytes_sent.read().await,
                bytes_received: *tunnel.bytes_received.read().await,
                connections: 1, // Each tunnel is one connection for now
                last_activity: tunnel.created_at.to_rfc3339(),
            });
        }

        stats
    }
}

// Implement Clone manually for testing
impl Clone for NatTunnelManager {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
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
        // This is a placeholder test
        // Real tests would require a database connection
    }
}
