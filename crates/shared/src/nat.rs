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
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&mapping).unwrap();
        let deserialized: NatMapping = serde_json::from_str(&json).unwrap();

        assert_eq!(mapping.id, deserialized.id);
        assert_eq!(mapping.local_port, deserialized.local_port);
        assert_eq!(mapping.public_port, deserialized.public_port);
    }
}
