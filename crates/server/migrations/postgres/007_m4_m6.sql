-- M4 / M6 / M7 supplementary tables (PostgreSQL variant).
-- See sqlite/007_m4_m6.sql for the full rationale.

CREATE TABLE IF NOT EXISTS alert_rules (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id UUID NOT NULL,
    name VARCHAR(255) NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    trigger_mode VARCHAR(20) NOT NULL DEFAULT 'once',
    rules_json TEXT NOT NULL,
    notification_group_id TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_alert_rules_enabled ON alert_rules(enabled);

CREATE TABLE IF NOT EXISTS alert_events (
    id TEXT PRIMARY KEY NOT NULL,
    rule_id TEXT NOT NULL,
    agent_id UUID,
    service_id UUID,
    kind VARCHAR(20) NOT NULL,
    payload_json TEXT NOT NULL,
    fired_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (rule_id) REFERENCES alert_rules(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_alert_events_rule ON alert_events(rule_id, fired_at DESC);
CREATE INDEX IF NOT EXISTS idx_alert_events_agent ON alert_events(agent_id, fired_at DESC);

CREATE TABLE IF NOT EXISTS ddns_configs (
    id UUID PRIMARY KEY NOT NULL,
    owner_user_id UUID NOT NULL,
    agent_id UUID,
    name VARCHAR(255) NOT NULL,
    provider VARCHAR(50) NOT NULL,
    domain TEXT NOT NULL,
    record_id TEXT,
    zone_id TEXT,
    api_token TEXT,
    api_key TEXT,
    api_secret TEXT,
    webhook_url TEXT,
    current_ip TEXT,
    last_applied_ip TEXT,
    last_applied_at TIMESTAMPTZ,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ddns_configs_owner ON ddns_configs(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_ddns_configs_agent ON ddns_configs(agent_id);

CREATE TABLE IF NOT EXISTS ddns_history (
    id UUID PRIMARY KEY NOT NULL,
    config_id UUID NOT NULL,
    old_ip TEXT,
    new_ip TEXT NOT NULL,
    success BOOLEAN NOT NULL,
    error TEXT,
    applied_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (config_id) REFERENCES ddns_configs(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ddns_history_config ON ddns_history(config_id, applied_at DESC);

CREATE TABLE IF NOT EXISTS nat_tunnels (
    id UUID PRIMARY KEY NOT NULL,
    owner_user_id UUID NOT NULL,
    agent_id UUID NOT NULL,
    name VARCHAR(255) NOT NULL,
    domain TEXT NOT NULL,
    local_host TEXT NOT NULL,
    local_port INTEGER NOT NULL,
    protocol VARCHAR(10) NOT NULL DEFAULT 'tcp',
    enabled BOOLEAN NOT NULL DEFAULT true,
    last_request_at TIMESTAMPTZ,
    last_request_status VARCHAR(50),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_nat_tunnels_agent ON nat_tunnels(agent_id);
CREATE INDEX IF NOT EXISTS idx_nat_tunnels_domain ON nat_tunnels(domain);
