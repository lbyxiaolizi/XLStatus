use anyhow::{Context, Result};
use async_trait::async_trait;
use xlstatus_shared::ddns::*;

/// DDNS provider trait
#[async_trait]
pub trait DdnsProviderTrait: Send + Sync {
    /// Update DNS record with new IP
    async fn update_ip(&self, hostname: &str, ip: &str) -> Result<()>;

    /// Get provider name
    fn name(&self) -> &'static str;
}

/// Cloudflare DDNS provider
pub struct CloudflareProvider {
    config: CloudflareConfig,
    client: reqwest::Client,
}

impl CloudflareProvider {
    pub fn new(config: CloudflareConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl DdnsProviderTrait for CloudflareProvider {
    async fn update_ip(&self, hostname: &str, ip: &str) -> Result<()> {
        // Get DNS record ID
        let list_url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records?name={}",
            self.config.zone_id, hostname
        );

        let list_response = self
            .client
            .get(&list_url)
            .header("Authorization", format!("Bearer {}", self.config.api_token))
            .header("Content-Type", "application/json")
            .send()
            .await
            .context("Failed to list DNS records")?;

        let list_json: serde_json::Value = list_response
            .json()
            .await
            .context("Failed to parse list response")?;

        let records = list_json["result"]
            .as_array()
            .context("No result array")?;

        if records.is_empty() {
            anyhow::bail!("DNS record not found: {}", hostname);
        }

        let record_id = records[0]["id"]
            .as_str()
            .context("No record ID")?;

        // Update DNS record
        let update_url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
            self.config.zone_id, record_id
        );

        let update_body = serde_json::json!({
            "type": self.config.record_type,
            "name": hostname,
            "content": ip,
            "ttl": self.config.ttl,
            "proxied": self.config.proxied,
        });

        let update_response = self
            .client
            .put(&update_url)
            .header("Authorization", format!("Bearer {}", self.config.api_token))
            .header("Content-Type", "application/json")
            .json(&update_body)
            .send()
            .await
            .context("Failed to update DNS record")?;

        if !update_response.status().is_success() {
            let error_text = update_response.text().await.unwrap_or_default();
            anyhow::bail!("Cloudflare API error: {}", error_text);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "Cloudflare"
    }
}

/// Hurricane Electric DDNS provider
pub struct HeProvider {
    config: HeConfig,
    client: reqwest::Client,
}

impl HeProvider {
    pub fn new(config: HeConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl DdnsProviderTrait for HeProvider {
    async fn update_ip(&self, _hostname: &str, ip: &str) -> Result<()> {
        let url = format!(
            "https://dyn.dns.he.net/nic/update?hostname={}&password={}&myip={}",
            self.config.hostname, self.config.password, ip
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to update HE DDNS")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("HE DDNS error: {}", error_text);
        }

        let body = response.text().await?;
        if !body.starts_with("good") && !body.starts_with("nochg") {
            anyhow::bail!("HE DDNS update failed: {}", body);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "Hurricane Electric"
    }
}

/// Webhook DDNS provider
pub struct WebhookProvider {
    config: WebhookConfig,
    client: reqwest::Client,
}

impl WebhookProvider {
    pub fn new(config: WebhookConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl DdnsProviderTrait for WebhookProvider {
    async fn update_ip(&self, hostname: &str, ip: &str) -> Result<()> {
        let url = self.config.url.replace("{{ip}}", ip).replace("{{hostname}}", hostname);

        let mut request = match self.config.method.to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            "PUT" => self.client.put(&url),
            _ => self.client.get(&url),
        };

        // Add custom headers
        if let Some(ref headers_json) = self.config.headers {
            if let Ok(headers) = serde_json::from_str::<serde_json::Value>(headers_json) {
                if let Some(obj) = headers.as_object() {
                    for (key, value) in obj {
                        if let Some(val_str) = value.as_str() {
                            request = request.header(key, val_str);
                        }
                    }
                }
            }
        }

        // Add body for POST/PUT
        if let Some(ref body_template) = self.config.body_template {
            let body = body_template.replace("{{ip}}", ip).replace("{{hostname}}", hostname);
            request = request.body(body);
        }

        let response = request
            .send()
            .await
            .context("Failed to send webhook request")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Webhook error: {}", error_text);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "Webhook"
    }
}

/// Dummy DDNS provider for testing
pub struct DummyProvider;

#[async_trait]
impl DdnsProviderTrait for DummyProvider {
    async fn update_ip(&self, hostname: &str, ip: &str) -> Result<()> {
        tracing::info!("Dummy DDNS: {} -> {}", hostname, ip);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "Dummy"
    }
}

/// Create DDNS provider from configuration
pub fn create_provider(
    provider_type: ProviderType,
    config_json: &str,
) -> Result<Box<dyn DdnsProviderTrait>> {
    match provider_type {
        ProviderType::Cloudflare => {
            let config: CloudflareConfig = serde_json::from_str(config_json)
                .context("Failed to parse Cloudflare config")?;
            Ok(Box::new(CloudflareProvider::new(config)))
        }
        ProviderType::He => {
            let config: HeConfig = serde_json::from_str(config_json)
                .context("Failed to parse HE config")?;
            Ok(Box::new(HeProvider::new(config)))
        }
        ProviderType::Webhook => {
            let config: WebhookConfig = serde_json::from_str(config_json)
                .context("Failed to parse Webhook config")?;
            Ok(Box::new(WebhookProvider::new(config)))
        }
        ProviderType::Dummy => Ok(Box::new(DummyProvider)),
        ProviderType::TencentCloud => {
            // TODO: Implement Tencent Cloud provider
            anyhow::bail!("Tencent Cloud provider not yet implemented")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_dummy_provider() {
        let provider = create_provider(ProviderType::Dummy, "{}").unwrap();
        assert_eq!(provider.name(), "Dummy");
    }
}
