use serde::{Deserialize, Serialize};

/// DDNS provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdnsProvider {
    pub id: String,
    pub name: String,
    pub provider_type: ProviderType,
    pub enabled: bool,
    pub config_json: String, // Provider-specific configuration
    pub created_at: String,
    pub updated_at: String,
}

/// DDNS provider type
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Cloudflare,
    TencentCloud,
    He,        // Hurricane Electric
    Webhook,
    Dummy,     // For testing
}

impl ProviderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderType::Cloudflare => "cloudflare",
            ProviderType::TencentCloud => "tencent_cloud",
            ProviderType::He => "he",
            ProviderType::Webhook => "webhook",
            ProviderType::Dummy => "dummy",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "cloudflare" => Some(ProviderType::Cloudflare),
            "tencent_cloud" => Some(ProviderType::TencentCloud),
            "he" => Some(ProviderType::He),
            "webhook" => Some(ProviderType::Webhook),
            "dummy" => Some(ProviderType::Dummy),
            _ => None,
        }
    }
}

/// Cloudflare DDNS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudflareConfig {
    pub api_token: String,
    pub zone_id: String,
    pub record_name: String,
    pub record_type: String, // "A" or "AAAA"
    pub proxied: bool,
    pub ttl: u32,
}

/// Tencent Cloud DDNS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TencentCloudConfig {
    pub secret_id: String,
    pub secret_key: String,
    pub domain: String,
    pub subdomain: String,
    pub record_type: String,
    pub record_line: String, // "默认"
    pub ttl: u32,
}

/// Hurricane Electric DDNS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeConfig {
    pub hostname: String,
    pub password: String,
}

/// Webhook DDNS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub method: String, // "GET" or "POST"
    pub headers: Option<String>, // JSON string
    pub body_template: Option<String>, // Template with {{ip}} placeholder
}

/// DDNS update record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdnsRecord {
    pub id: String,
    pub provider_id: String,
    pub agent_id: String,
    pub old_ip: Option<String>,
    pub new_ip: String,
    pub success: bool,
    pub error: Option<String>,
    pub created_at: String,
}

/// DDNS configuration for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDdnsConfig {
    pub id: String,
    pub agent_id: String,
    pub provider_id: String,
    pub enabled: bool,
    pub check_interval_seconds: u64,
    pub last_ip: Option<String>,
    pub last_checked_at: Option<String>,
    pub last_updated_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_type_conversion() {
        assert_eq!(ProviderType::Cloudflare.as_str(), "cloudflare");
        assert_eq!(ProviderType::TencentCloud.as_str(), "tencent_cloud");

        assert!(matches!(
            ProviderType::from_str("cloudflare"),
            Some(ProviderType::Cloudflare)
        ));
        assert!(matches!(
            ProviderType::from_str("webhook"),
            Some(ProviderType::Webhook)
        ));
        assert!(ProviderType::from_str("invalid").is_none());
    }

    #[test]
    fn test_cloudflare_config_serialization() {
        let config = CloudflareConfig {
            api_token: "test_token".to_string(),
            zone_id: "zone123".to_string(),
            record_name: "example.com".to_string(),
            record_type: "A".to_string(),
            proxied: false,
            ttl: 300,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: CloudflareConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.api_token, deserialized.api_token);
        assert_eq!(config.zone_id, deserialized.zone_id);
    }
}
