use serde::{Deserialize, Serialize};

/// NAT mapping for port forwarding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatMapping {
    pub id: String,
    pub agent_id: String,
    pub local_host: String,
    pub local_port: u16,
    pub public_port: u16,
    pub protocol: Protocol,
    pub enabled: bool,
    pub description: Option<String>,
    #[serde(default)]
    pub allowed_sources: Option<String>,
    #[serde(default)]
    pub max_active_tunnels: Option<u32>,
    #[serde(default)]
    pub idle_timeout_seconds: Option<u32>,
    #[serde(default)]
    pub max_bytes_per_tunnel: Option<u64>,
    #[serde(default)]
    pub max_bandwidth_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub rate_limit_window_seconds: Option<u32>,
    #[serde(default)]
    pub max_connections_per_window: Option<u32>,
    #[serde(default)]
    pub max_bytes_per_window: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
}

/// Protocol type for NAT mapping
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "tcp" => Some(Protocol::Tcp),
            "udp" => Some(Protocol::Udp),
            _ => None,
        }
    }
}

/// Tunnel connection info
#[derive(Debug, Clone)]
pub struct TunnelInfo {
    pub tunnel_id: String,
    pub agent_id: String,
    pub local_host: String,
    pub local_port: u16,
    pub public_port: u16,
    pub protocol: Protocol,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Tunnel statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelStats {
    pub tunnel_id: String,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub connections: u64,
    pub last_activity: String,
}

/// Initial open request sent over AgentService::IoStream for a NAT
/// reverse-tunnel connection. Subsequent `IoFrame::data` payloads on
/// the same `stream_id` are raw bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatTunnelOpenRequest {
    pub local_host: String,
    pub local_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NatTunnelControlMessage {
    Open { local_host: String, local_port: u16 },
    Ready,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_conversion() {
        assert_eq!(Protocol::Tcp.as_str(), "tcp");
        assert_eq!(Protocol::Udp.as_str(), "udp");

        assert!(matches!(Protocol::from_str("tcp"), Some(Protocol::Tcp)));
        assert!(matches!(Protocol::from_str("TCP"), Some(Protocol::Tcp)));
        assert!(matches!(Protocol::from_str("udp"), Some(Protocol::Udp)));
        assert!(Protocol::from_str("invalid").is_none());
    }

    #[test]
    fn test_nat_mapping_serialization() {
        let mapping = NatMapping {
            id: "test".to_string(),
            agent_id: "agent1".to_string(),
            local_host: "127.0.0.1".to_string(),
            local_port: 8080,
            public_port: 9090,
            protocol: Protocol::Tcp,
            enabled: true,
            description: Some("Test mapping".to_string()),
            allowed_sources: Some("127.0.0.1/32".to_string()),
            max_active_tunnels: Some(2),
            idle_timeout_seconds: Some(60),
            max_bytes_per_tunnel: Some(10 * 1024 * 1024),
            max_bandwidth_bytes_per_second: Some(1024 * 1024),
            rate_limit_window_seconds: Some(60),
            max_connections_per_window: Some(30),
            max_bytes_per_window: Some(100 * 1024 * 1024),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&mapping).unwrap();
        let deserialized: NatMapping = serde_json::from_str(&json).unwrap();

        assert_eq!(mapping.id, deserialized.id);
        assert_eq!(mapping.local_port, deserialized.local_port);
        assert_eq!(mapping.public_port, deserialized.public_port);
        assert_eq!(mapping.allowed_sources, deserialized.allowed_sources);
        assert_eq!(mapping.max_active_tunnels, deserialized.max_active_tunnels);
        assert_eq!(
            mapping.idle_timeout_seconds,
            deserialized.idle_timeout_seconds
        );
        assert_eq!(
            mapping.max_bytes_per_tunnel,
            deserialized.max_bytes_per_tunnel
        );
        assert_eq!(
            mapping.max_bandwidth_bytes_per_second,
            deserialized.max_bandwidth_bytes_per_second
        );
        assert_eq!(
            mapping.rate_limit_window_seconds,
            deserialized.rate_limit_window_seconds
        );
        assert_eq!(
            mapping.max_connections_per_window,
            deserialized.max_connections_per_window
        );
        assert_eq!(
            mapping.max_bytes_per_window,
            deserialized.max_bytes_per_window
        );
    }
}
