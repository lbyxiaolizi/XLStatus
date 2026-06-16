use crate::db::Db;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

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

    /// Load DDNS providers from database
    async fn load_providers(&self) -> Result<()> {
        // TODO: Load from database
        // For now, this is a placeholder
        info!("DDNS providers loaded");
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

    /// Check all agents for IP changes
    async fn check_all_agents(&self) -> Result<()> {
        // TODO: Load agent DDNS configurations from database
        // For each configuration:
        // 1. Get agent's current IP
        // 2. Compare with last known IP
        // 3. If changed, update DNS via provider
        // 4. Record the update

        Ok(())
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
        let provider = providers
            .get(provider_id)
            .context("Provider not found")?;

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
