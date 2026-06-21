use crate::security::{secure_reqwest_client_builder, validate_outbound_url_resolved};
use anyhow::{Context, Result};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use reqwest::{RequestBuilder, Url};
use sha2::{Digest, Sha256};
use std::time::Duration;
use xlstatus_shared::ddns::*;

const DDNS_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";
const HE_DDNS_UPDATE_URL: &str = "https://dyn.dns.he.net/nic/update";
const TENCENT_DNSPOD_API_HOST: &str = "dnspod.tencentcloudapi.com";
const TENCENT_DNSPOD_API_URL: &str = "https://dnspod.tencentcloudapi.com/";

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
}

impl CloudflareProvider {
    pub fn new(config: CloudflareConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl DdnsProviderTrait for CloudflareProvider {
    async fn update_ip(&self, hostname: &str, ip: &str) -> Result<()> {
        // Get DNS record ID
        let list_url = cloudflare_list_url(&self.config.zone_id, hostname)?;
        let (client, list_url) = ddns_http_client_for_url(&list_url, "Cloudflare DDNS").await?;

        let list_response = client
            .get(list_url)
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
        let update_url = cloudflare_update_url(&self.config.zone_id, record_id)?;
        let (client, update_url) = ddns_http_client_for_url(&update_url, "Cloudflare DDNS").await?;

        let update_body = serde_json::json!({
            "type": self.config.record_type,
            "name": hostname,
            "content": ip,
            "ttl": self.config.ttl,
            "proxied": self.config.proxied,
        });

        let update_response = client
            .put(update_url)
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
}

impl HeProvider {
    pub fn new(config: HeConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl DdnsProviderTrait for HeProvider {
    async fn update_ip(&self, _hostname: &str, ip: &str) -> Result<()> {
        let url = he_update_url(&self.config.hostname, ip)?;
        let (client, url) = ddns_http_client_for_url(&url, "HE DDNS").await?;

        let response = self
            .he_update_request(&client, url)
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

impl HeProvider {
    fn he_update_request(&self, client: &reqwest::Client, url: Url) -> RequestBuilder {
        client.get(url).basic_auth(
            self.config.hostname.trim(),
            Some(self.config.password.trim()),
        )
    }
}

/// Tencent Cloud DNSPod DDNS provider.
///
/// Uses Tencent Cloud API v3 `dnspod` `ModifyDynamicDNS`. The DB/API layer
/// supplies `record_id`; discovering record IDs can be added later as a
/// UX helper, but update itself is fully implemented here.
pub struct TencentCloudProvider {
    config: TencentCloudConfig,
}

impl TencentCloudProvider {
    pub fn new(config: TencentCloudConfig) -> Self {
        Self { config }
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

        let endpoint = TENCENT_DNSPOD_API_HOST;
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
            .tencent_request(&body, timestamp, &authorization)
            .await?
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

impl TencentCloudProvider {
    async fn tencent_request(
        &self,
        body: &str,
        timestamp: i64,
        authorization: &str,
    ) -> Result<RequestBuilder> {
        let url = Url::parse(TENCENT_DNSPOD_API_URL).context("invalid Tencent Cloud DDNS URL")?;
        let (client, url) = ddns_http_client_for_url(&url, "Tencent Cloud DDNS").await?;
        Ok(client
            .post(url)
            .header("Authorization", authorization)
            .header("Content-Type", "application/json")
            .header("Host", TENCENT_DNSPOD_API_HOST)
            .header("X-TC-Action", "ModifyDynamicDNS")
            .header("X-TC-Version", "2021-03-23")
            .header("X-TC-Timestamp", timestamp.to_string())
            .body(body.to_string()))
    }
}

async fn ddns_http_client_for_url(url: &Url, purpose: &str) -> Result<(reqwest::Client, Url)> {
    let validated = validate_outbound_url_resolved(url.as_str(), purpose).await?;
    let client = secure_reqwest_client_builder(&validated)
        .timeout(DDNS_HTTP_TIMEOUT)
        .build()
        .with_context(|| format!("failed to build {purpose} HTTP client"))?;
    Ok((client, validated.url))
}

fn cloudflare_list_url(zone_id: &str, hostname: &str) -> Result<Url> {
    let mut url = cloudflare_api_url(["zones", zone_id.trim(), "dns_records"])?;
    url.query_pairs_mut().append_pair("name", hostname.trim());
    Ok(url)
}

fn cloudflare_update_url(zone_id: &str, record_id: &str) -> Result<Url> {
    cloudflare_api_url(["zones", zone_id.trim(), "dns_records", record_id.trim()])
}

fn cloudflare_api_url<'a>(segments: impl IntoIterator<Item = &'a str>) -> Result<Url> {
    let mut url = Url::parse(CLOUDFLARE_API_BASE).context("invalid Cloudflare DDNS URL")?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Cloudflare DDNS URL cannot be a base"))?;
        path.extend(segments);
    }
    Ok(url)
}

fn he_update_url(hostname: &str, ip: &str) -> Result<Url> {
    let mut url = Url::parse(HE_DDNS_UPDATE_URL).context("invalid HE DDNS URL")?;
    url.query_pairs_mut()
        .append_pair("hostname", hostname.trim())
        .append_pair("myip", ip.trim());
    Ok(url)
}

/// Webhook DDNS provider
pub struct WebhookProvider {
    config: WebhookConfig,
}

impl WebhookProvider {
    pub fn new(config: WebhookConfig) -> Self {
        Self { config }
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
        let validated = validate_outbound_url_resolved(&url, "DDNS webhook").await?;
        let client = secure_reqwest_client_builder(&validated)
            .timeout(DDNS_HTTP_TIMEOUT)
            .build()
            .context("failed to build DDNS webhook HTTP client")?;
        let url = validated.url.clone();

        let mut request = match self.config.method.to_uppercase().as_str() {
            "POST" => client.post(url.clone()),
            "PUT" => client.put(url.clone()),
            _ => client.get(url.clone()),
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

    #[test]
    fn he_update_url_does_not_put_password_in_query() {
        let url = he_update_url(" host.example.com ", " 203.0.113.10 ").unwrap();

        assert_eq!(
            url.as_str(),
            "https://dyn.dns.he.net/nic/update?hostname=host.example.com&myip=203.0.113.10"
        );
        assert!(url.query_pairs().all(|(key, _)| key != "password"));
    }

    #[test]
    fn he_update_request_uses_basic_authorization_header() {
        let provider = HeProvider::new(HeConfig {
            hostname: " host.example.com ".into(),
            password: " dynamic-key ".into(),
        });
        let client = reqwest::Client::builder().build().unwrap();
        let request = provider
            .he_update_request(
                &client,
                he_update_url("host.example.com", "203.0.113.10").unwrap(),
            )
            .build()
            .unwrap();

        assert_eq!(
            request.url().query(),
            Some("hostname=host.example.com&myip=203.0.113.10")
        );
        assert_eq!(
            request
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Basic aG9zdC5leGFtcGxlLmNvbTpkeW5hbWljLWtleQ==")
        );
    }

    #[test]
    fn cloudflare_urls_encode_path_segments_and_query_values() {
        let list = cloudflare_list_url("zone/id", "www.example.com?x=1").unwrap();
        let update = cloudflare_update_url("zone/id", "record/id").unwrap();

        assert_eq!(
            list.as_str(),
            "https://api.cloudflare.com/client/v4/zones/zone%2Fid/dns_records?name=www.example.com%3Fx%3D1"
        );
        assert_eq!(
            update.as_str(),
            "https://api.cloudflare.com/client/v4/zones/zone%2Fid/dns_records/record%2Fid"
        );
    }
}
