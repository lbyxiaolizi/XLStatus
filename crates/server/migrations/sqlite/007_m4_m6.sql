-- M4 alert rules + fired/recovered events.
-- M6 DDNS configs and per-update history.
-- M9 audit log / system records (some of these already exist in 005; we
-- add only the new M4 / M6 shapes here).

CREATE TABLE IF NOT EXISTS alert_rules (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    trigger_mode TEXT NOT NULL DEFAULT 'once', -- 'always' | 'once'
    rules_json TEXT NOT NULL,                  -- Vec<AlertCondition>
    notification_group_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_alert_rules_enabled ON alert_rules(enabled);

CREATE TABLE IF NOT EXISTS alert_events (
    id TEXT PRIMARY KEY NOT NULL,
    rule_id TEXT NOT NULL,
    agent_id TEXT,
    service_id TEXT,
    kind TEXT NOT NULL, -- 'fired' | 'recovered'
    payload_json TEXT NOT NULL,
    fired_at TEXT NOT NULL,
    FOREIGN KEY (rule_id) REFERENCES alert_rules(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_alert_events_rule ON alert_events(rule_id, fired_at DESC);
CREATE INDEX IF NOT EXISTS idx_alert_events_agent ON alert_events(agent_id, fired_at DESC);

CREATE TABLE IF NOT EXISTS ddns_configs (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL,
    agent_id TEXT,
    name TEXT NOT NULL,
    provider TEXT NOT NULL, -- 'cloudflare' | 'tencent' | 'he' | 'webhook' | 'dummy'
    domain TEXT NOT NULL,
    record_id TEXT,         -- cloudflare record id; tencent record id; etc.
    zone_id TEXT,           -- cloudflare zone id
    api_token TEXT,
    api_key TEXT,           -- tencent secret_id
    api_secret TEXT,        -- tencent secret_key
    webhook_url TEXT,       -- 'webhook' provider
    current_ip TEXT,
    last_applied_ip TEXT,
    last_applied_at TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ddns_configs_owner ON ddns_configs(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_ddns_configs_agent ON ddns_configs(agent_id);

CREATE TABLE IF NOT EXISTS ddns_history (
    id TEXT PRIMARY KEY NOT NULL,
    config_id TEXT NOT NULL,
    old_ip TEXT,
    new_ip TEXT NOT NULL,
    success INTEGER NOT NULL,
    error TEXT,
    applied_at TEXT NOT NULL,
    FOREIGN KEY (config_id) REFERENCES ddns_configs(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ddns_history_config ON ddns_history(config_id, applied_at DESC);

CREATE TABLE IF NOT EXISTS nat_tunnels (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    name TEXT NOT NULL,
    domain TEXT NOT NULL,
    local_host TEXT NOT NULL,
    local_port INTEGER NOT NULL,
    protocol TEXT NOT NULL DEFAULT 'tcp',
    enabled INTEGER NOT NULL DEFAULT 1,
    last_request_at TEXT,
    last_request_status TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_nat_tunnels_agent ON nat_tunnels(agent_id);
CREATE INDEX IF NOT EXISTS idx_nat_tunnels_domain ON nat_tunnels(domain);
