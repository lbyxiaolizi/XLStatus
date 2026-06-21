#![allow(dead_code)]
#![allow(unused_imports)]

use crate::api::v1::settings;
use crate::db::Db;
use crate::security::{secure_reqwest_client_builder, validate_outbound_url_resolved};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{error, info};

use super::provider::{create_provider, DdnsProviderTrait};
use xlstatus_shared::ddns::*;

/// Agent IP state
#[derive(Debug, Clone)]
struct AgentIpState {
    agent_id: String,
    current_ip: Option<String>,
    last_checked: chrono::DateTime<chrono::Utc>,
}

/// DDNS manager
#[allow(dead_code)]
pub struct DdnsManager {
    db: Db,
    // Map of provider_id -> provider instance
    providers: Arc<RwLock<HashMap<String, Box<dyn DdnsProviderTrait>>>>,
    // Map of agent_id -> IP state
    agent_states: Arc<RwLock<HashMap<String, AgentIpState>>>,
}

impl DdnsManager {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            providers: Arc::new(RwLock::new(HashMap::new())),
            agent_states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start DDNS manager
    pub async fn start(self: Arc<Self>) -> Result<()> {
        info!("Starting DDNS manager");

        // Load providers
        if let Err(e) = self.load_providers().await {
            error!("Failed to load DDNS providers: {}", e);
        }

        // Start check loop
        let manager = self.clone();
        tokio::spawn(async move {
            manager.check_loop().await;
        });

        Ok(())
    }

    /// Load DDNS providers from database. M6: build a `Box<dyn
    /// DdnsProviderTrait>` for every enabled `ddns_configs` row so
    /// the check loop can call `update_ip` on them.
    async fn load_providers(&self) -> Result<()> {
        use crate::db::repository::ddns::DdnsConfigRepository;
        let rows = DdnsConfigRepository::list_enabled(&self.db).await?;
        let mut providers = self.providers.write().await;
        providers.clear();
        for row in rows {
            let provider_type = match ProviderType::from_str(&row.provider) {
                Some(t) => t,
                None => {
                    error!("Unknown DDNS provider type: {}", row.provider);
                    continue;
                }
            };
            // Build a per-row JSON config that `create_provider` can
            // parse. We map DB columns onto the shared
            // `*Config` shapes.
            let cfg_json = match provider_type {
                ProviderType::Cloudflare => serde_json::json!({
                    "api_token": row.api_token.clone().unwrap_or_default(),
                    "zone_id": row.zone_id.clone().unwrap_or_default(),
                    "record_name": row.domain,
                    "record_type": "A",
                    "ttl": 60,
                    "proxied": false,
                })
                .to_string(),
                ProviderType::TencentCloud => serde_json::json!({
                    "secret_id": row.api_key.clone().unwrap_or_default(),
                    "secret_key": row.api_secret.clone().unwrap_or_default(),
                    "domain": row.domain,
                    "subdomain": "@",
                    "record_id": row.record_id.as_deref().and_then(|value| value.parse::<u64>().ok()),
                    "record_type": "A",
                    "record_line": "默认",
                    "ttl": 600,
                })
                .to_string(),
                ProviderType::He => serde_json::json!({
                    "hostname": row.domain,
                    "password": row.api_token.clone().or(row.api_secret.clone()).unwrap_or_default(),
                })
                .to_string(),
                ProviderType::Webhook => serde_json::json!({
                    "url": row.webhook_url.clone().unwrap_or_default(),
                    "method": "POST",
                    "headers": serde_json::json!({"content-type":"application/json"}).to_string(),
                    "body_template": serde_json::json!({"hostname":"{{hostname}}","ip":"{{ip}}"}).to_string(),
                })
                .to_string(),
                ProviderType::Dummy => "{}".to_string(),
            };
            match create_provider(provider_type, &cfg_json) {
                Ok(p) => {
                    providers.insert(row.id.clone(), p);
                }
                Err(e) => error!("DDNS provider {} init failed: {}", row.id, e),
            }
        }
        info!("DDNS providers loaded: {}", providers.len());
        Ok(())
    }

    /// Main check loop
    async fn check_loop(&self) {
        let mut tick = interval(Duration::from_secs(60)); // Check every minute

        loop {
            tick.tick().await;

            if let Err(e) = self.check_all_agents().await {
                error!("DDNS check error: {}", e);
            }
        }
    }

    /// Check all agents for IP changes. M6 implementation: load
    /// `ddns_configs` rows, look up the agent's last IP from the
    /// `agents.last_state_json` payload, compare with
    /// `last_applied_ip`, and call the provider when they differ.
    async fn check_all_agents(&self) -> Result<()> {
        use crate::db::repository::ddns::DdnsConfigRepository;
        use crate::db::AgentRepository;

        let configs = DdnsConfigRepository::list_enabled(&self.db).await?;
        if configs.is_empty() {
            return Ok(());
        }
        let agent_repo = AgentRepository::new(self.db.clone());
        for cfg in configs {
            let agent_id = match &cfg.agent_id {
                Some(a) => a.clone(),
                None => continue,
            };
            let agent = match agent_repo
                .find_by_id_with_state(xlstatus_shared::AgentId(
                    uuid::Uuid::parse_str(&agent_id).unwrap_or_default(),
                ))
                .await
            {
                Ok(Some(a)) => a,
                _ => continue,
            };
            let state_json = match &agent.last_state_json {
                Some(s) => s,
                None => continue,
            };
            let parsed: serde_json::Value = match serde_json::from_str(state_json) {
                Ok(v) => v,
                Err(_) => continue,
            };
            // Try to extract an IP from the host state. The agent
            // does not always report IP; we use the first network
            // interface's address when available.
            let new_ip = parsed["primary_ip"].as_str().map(|s| s.to_string());
            let new_ip = match new_ip {
                Some(s) if !s.is_empty() => s,
                _ => continue,
            };
            if cfg.last_applied_ip.as_deref() == Some(new_ip.as_str()) {
                continue;
            }
            if let Some(resolver_url) = settings::ddns_resolver_url(&self.db)
                .await
                .map_err(|err| anyhow::anyhow!("{err:?}"))?
            {
                match resolver_contains_ip(&resolver_url, &cfg.domain, &new_ip).await {
                    Ok(true) => {
                        let now = chrono::Utc::now().to_rfc3339();
                        DdnsConfigRepository::record_history(
                            &self.db,
                            &uuid::Uuid::now_v7().to_string(),
                            &cfg.id,
                            cfg.last_applied_ip.as_deref(),
                            &new_ip,
                            true,
                            None,
                            &now,
                        )
                        .await?;
                        DdnsConfigRepository::update_after_apply(&self.db, &cfg.id, &new_ip, &now)
                            .await?;
                        continue;
                    }
                    Ok(false) => {}
                    Err(err) => {
                        error!("DDNS resolver check failed for {}: {}", cfg.domain, err);
                    }
                }
            }
            self.apply_update(&cfg, &new_ip).await;
        }
        Ok(())
    }

    /// Apply a single DDNS update + record the history row.
    async fn apply_update(&self, cfg: &crate::db::repository::ddns::DdnsConfigRow, new_ip: &str) {
        use crate::db::repository::ddns::DdnsConfigRepository;
        let providers = self.providers.read().await;
        let provider = match providers.get(&cfg.id) {
            Some(p) => p,
            None => return,
        };
        let old_ip = cfg.last_applied_ip.clone();
        let now = chrono::Utc::now();
        let now_str = now.to_rfc3339();
        let res = provider.update_ip(&cfg.domain, new_ip).await;
        let (success, err_msg) = match res {
            Ok(_) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        };
        if let Err(e) = DdnsConfigRepository::record_history(
            &self.db,
            &uuid::Uuid::now_v7().to_string(),
            &cfg.id,
            old_ip.as_deref(),
            new_ip,
            success,
            err_msg.as_deref(),
            &now_str,
        )
        .await
        {
            error!("ddns history insert failed: {}", e);
        }
        if success {
            if let Err(e) =
                DdnsConfigRepository::update_after_apply(&self.db, &cfg.id, new_ip, &now_str).await
            {
                error!("ddns update_after_apply failed: {}", e);
            }
            // Update in-memory state cache.
            let mut states = self.agent_states.write().await;
            states.insert(
                cfg.agent_id.clone().unwrap_or_default(),
                AgentIpState {
                    agent_id: cfg.agent_id.clone().unwrap_or_default(),
                    current_ip: Some(new_ip.to_string()),
                    last_checked: now,
                },
            );
            info!(
                "DDNS applied for {} via {}: {} -> {}",
                cfg.domain,
                cfg.provider,
                old_ip.as_deref().unwrap_or("?"),
                new_ip
            );
        } else {
            error!(
                "DDNS failed for {}: {}",
                cfg.domain,
                err_msg.unwrap_or_default()
            );
        }
    }

    /// Update IP for an agent
    pub async fn update_agent_ip(
        &self,
        agent_id: &str,
        provider_id: &str,
        hostname: &str,
        new_ip: &str,
    ) -> Result<()> {
        let providers = self.providers.read().await;
        let provider = providers.get(provider_id).context("Provider not found")?;

        info!(
            "Updating DDNS for agent {} via {}: {} -> {}",
            agent_id,
            provider.name(),
            hostname,
            new_ip
        );

        provider
            .update_ip(hostname, new_ip)
            .await
            .context("Failed to update IP")?;

        // Update agent state
        let mut states = self.agent_states.write().await;
        states.insert(
            agent_id.to_string(),
            AgentIpState {
                agent_id: agent_id.to_string(),
                current_ip: Some(new_ip.to_string()),
                last_checked: chrono::Utc::now(),
            },
        );

        info!("DDNS updated successfully for agent {}", agent_id);
        Ok(())
    }

    /// Register a DDNS provider
    pub async fn register_provider(
        &self,
        provider_id: String,
        provider_type: ProviderType,
        config_json: &str,
    ) -> Result<()> {
        let provider = create_provider(provider_type, config_json)?;

        let mut providers = self.providers.write().await;
        providers.insert(provider_id.clone(), provider);

        info!("DDNS provider {} registered", provider_id);
        Ok(())
    }

    /// M6: hot-reload providers from the database. Useful when an
    /// admin adds a new DDNS config and wants it picked up
    /// without restarting the server.
    pub async fn reload_providers(&self) -> Result<()> {
        self.load_providers().await
    }

    /// M6: run one DDNS check immediately. The background loop still
    /// ticks every 60 seconds, but tests and admin-triggered updates
    /// need a deterministic entrypoint.
    pub async fn check_now(&self) -> Result<()> {
        self.check_all_agents().await
    }

    /// M6: apply DDNS immediately when an agent reports an IP change.
    pub async fn check_agent_ip_report(
        &self,
        agent_id: &str,
        ipv4: Option<&str>,
        ipv6: Option<&str>,
    ) -> Result<()> {
        use crate::db::repository::ddns::DdnsConfigRepository;

        let new_ip = ipv4
            .filter(|value| !value.trim().is_empty())
            .or_else(|| ipv6.filter(|value| !value.trim().is_empty()));
        let Some(new_ip) = new_ip else {
            return Ok(());
        };

        let configs = DdnsConfigRepository::list_enabled(&self.db).await?;
        for cfg in configs
            .into_iter()
            .filter(|cfg| cfg.agent_id.as_deref() == Some(agent_id))
        {
            if cfg.last_applied_ip.as_deref() == Some(new_ip) {
                continue;
            }
            self.apply_update(&cfg, new_ip).await;
        }

        Ok(())
    }

    /// Get statistics
    pub async fn get_statistics(&self) -> DdnsStatistics {
        let providers = self.providers.read().await;
        let states = self.agent_states.read().await;

        DdnsStatistics {
            total_providers: providers.len(),
            total_agents: states.len(),
            last_check: chrono::Utc::now().to_rfc3339(),
        }
    }
}

async fn resolver_contains_ip(resolver_url: &str, domain: &str, ip: &str) -> Result<bool> {
    let record_type = if ip.parse::<std::net::Ipv6Addr>().is_ok() {
        "AAAA"
    } else {
        "A"
    };
    let mut url = reqwest::Url::parse(resolver_url).context("DDNS resolver URL is invalid")?;
    url.query_pairs_mut()
        .append_pair("name", domain)
        .append_pair("type", record_type);
    let validated = validate_outbound_url_resolved(url.as_str(), "DDNS resolver").await?;
    let raw = secure_reqwest_client_builder(&validated)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to build DDNS resolver client")?
        .get(validated.url.clone())
        .send()
        .await
        .context("DDNS resolver request failed")?
        .json::<serde_json::Value>()
        .await
        .context("DDNS resolver response is invalid")?;
    let answers = raw
        .get("Answer")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten();
    Ok(answers
        .filter_map(|item| item.get("data").and_then(|value| value.as_str()))
        .any(|value| value == ip))
}

#[derive(Debug, serde::Serialize)]
pub struct DdnsStatistics {
    pub total_providers: usize,
    pub total_agents: usize,
    pub last_check: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ddns_manager_creation() {
        // Placeholder test
    }
}
