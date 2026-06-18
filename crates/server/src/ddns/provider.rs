use crate::security::validate_outbound_url;
use anyhow::{Context, Result};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
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

        let records = list_json["result"].as_array().context("No result array")?;

        if records.is_empty() {
            anyhow::bail!("DNS record not found: {}", hostname);
        }

        let record_id = records[0]["id"].as_str().context("No record ID")?;

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

/// Tencent Cloud DNSPod DDNS provider.
///
/// Uses Tencent Cloud API v3 `dnspod` `ModifyDynamicDNS`. The DB/API layer
/// supplies `record_id`; discovering record IDs can be added later as a
/// UX helper, but update itself is fully implemented here.
pub struct TencentCloudProvider {
    config: TencentCloudConfig,
    client: reqwest::Client,
}

impl TencentCloudProvider {
    pub fn new(config: TencentCloudConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    fn subdomain_for(&self, hostname: &str) -> String {
        if !self.config.subdomain.trim().is_empty() && self.config.subdomain != "@" {
            return self.config.subdomain.clone();
        }
        let suffix = format!(".{}", self.config.domain);
        hostname
            .strip_suffix(&suffix)
            .filter(|value| !value.is_empty())
            .unwrap_or("@")
            .to_string()
    }
}

#[async_trait]
impl DdnsProviderTrait for TencentCloudProvider {
    async fn update_ip(&self, hostname: &str, ip: &str) -> Result<()> {
        let record_id = self
            .config
            .record_id
            .context("Tencent Cloud record_id is required")?;
        if self.config.secret_id.trim().is_empty() || self.config.secret_key.trim().is_empty() {
            anyhow::bail!("Tencent Cloud secret_id and secret_key are required");
        }

        let endpoint = "dnspod.tencentcloudapi.com";
        let timestamp = chrono::Utc::now().timestamp();
        let body = serde_json::json!({
            "Domain": self.config.domain,
            "SubDomain": self.subdomain_for(hostname),
            "RecordLine": self.config.record_line,
            "Value": ip,
            "RecordId": record_id,
            "TTL": self.config.ttl,
        })
        .to_string();
        let authorization = tencent_authorization(
            &self.config.secret_id,
            &self.config.secret_key,
            "dnspod",
            endpoint,
            "ModifyDynamicDNS",
            "/",
            &body,
            timestamp,
        )?;

        let response = self
            .client
            .post(format!("https://{}", endpoint))
            .header("Authorization", authorization)
            .header("Content-Type", "application/json")
            .header("Host", endpoint)
            .header("X-TC-Action", "ModifyDynamicDNS")
            .header("X-TC-Version", "2021-03-23")
            .header("X-TC-Timestamp", timestamp.to_string())
            .body(body)
            .send()
            .await
            .context("Failed to update Tencent Cloud DNS record")?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Tencent Cloud API HTTP {}: {}", status, text);
        }
        let parsed: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("Failed to parse Tencent Cloud response: {}", text))?;
        if let Some(error) = parsed
            .get("Response")
            .and_then(|response| response.get("Error"))
        {
            anyhow::bail!("Tencent Cloud API error: {}", error);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "Tencent Cloud"
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
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("failed to build DDNS webhook HTTP client"),
        }
    }
}

#[async_trait]
impl DdnsProviderTrait for WebhookProvider {
    async fn update_ip(&self, hostname: &str, ip: &str) -> Result<()> {
        let url = self
            .config
            .url
            .replace("{{ip}}", ip)
            .replace("{{hostname}}", hostname);
        let url = validate_outbound_url(&url, "DDNS webhook").await?;

        let mut request = match self.config.method.to_uppercase().as_str() {
            "POST" => self.client.post(url.clone()),
            "PUT" => self.client.put(url.clone()),
            _ => self.client.get(url.clone()),
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
            let body = body_template
                .replace("{{ip}}", ip)
                .replace("{{hostname}}", hostname);
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
            let config: CloudflareConfig =
                serde_json::from_str(config_json).context("Failed to parse Cloudflare config")?;
            Ok(Box::new(CloudflareProvider::new(config)))
        }
        ProviderType::He => {
            let config: HeConfig =
                serde_json::from_str(config_json).context("Failed to parse HE config")?;
            Ok(Box::new(HeProvider::new(config)))
        }
        ProviderType::Webhook => {
            let config: WebhookConfig =
                serde_json::from_str(config_json).context("Failed to parse Webhook config")?;
            Ok(Box::new(WebhookProvider::new(config)))
        }
        ProviderType::Dummy => Ok(Box::new(DummyProvider)),
        ProviderType::TencentCloud => {
            let config: TencentCloudConfig = serde_json::from_str(config_json)
                .context("Failed to parse Tencent Cloud config")?;
            Ok(Box::new(TencentCloudProvider::new(config)))
        }
    }
}

fn tencent_authorization(
    secret_id: &str,
    secret_key: &str,
    service: &str,
    host: &str,
    action: &str,
    canonical_uri: &str,
    payload: &str,
    timestamp: i64,
) -> Result<String> {
    let date = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)
        .context("invalid Tencent Cloud timestamp")?
        .format("%Y-%m-%d")
        .to_string();
    let hashed_payload = hex::encode(Sha256::digest(payload.as_bytes()));
    let canonical_headers = format!(
        "content-type:application/json\nhost:{}\nx-tc-action:{}\n",
        host, action
    );
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        "POST",
        canonical_uri,
        "",
        canonical_headers,
        "content-type;host;x-tc-action",
        hashed_payload
    );
    let credential_scope = format!("{}/{}/tc3_request", date, service);
    let string_to_sign = format!(
        "TC3-HMAC-SHA256\n{}\n{}\n{}",
        timestamp,
        credential_scope,
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );

    let secret_date = hmac_sha256(format!("TC3{}", secret_key).as_bytes(), date.as_bytes())?;
    let secret_service = hmac_sha256(&secret_date, service.as_bytes())?;
    let secret_signing = hmac_sha256(&secret_service, b"tc3_request")?;
    let signature = hex::encode(hmac_sha256(&secret_signing, string_to_sign.as_bytes())?);

    Ok(format!(
        "TC3-HMAC-SHA256 Credential={}/{}, SignedHeaders=content-type;host;x-tc-action, Signature={}",
        secret_id, credential_scope, signature
    ))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).context("invalid HMAC key")?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_dummy_provider() {
        let provider = create_provider(ProviderType::Dummy, "{}").unwrap();
        assert_eq!(provider.name(), "Dummy");
    }

    #[test]
    fn test_tencent_authorization_generates_signature() {
        let auth = tencent_authorization(
            "akid",
            "secret",
            "dnspod",
            "dnspod.tencentcloudapi.com",
            "ModifyDynamicDNS",
            "/",
            "{\"Domain\":\"example.com\"}",
            1_700_000_000,
        )
        .unwrap();
        assert!(auth.starts_with("TC3-HMAC-SHA256 Credential=akid/"));
        assert!(auth.contains("SignedHeaders=content-type;host;x-tc-action"));
        assert!(auth.contains("Signature="));
    }
}
