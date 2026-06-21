-- M6 NAT 穿透相关表

-- NAT 映射配置
CREATE TABLE IF NOT EXISTS nat_mappings (
    id TEXT PRIMARY KEY NOT NULL,
    agent_id TEXT NOT NULL,
    local_host TEXT NOT NULL,
    local_port INTEGER NOT NULL,
    public_port INTEGER NOT NULL UNIQUE,
    protocol TEXT NOT NULL DEFAULT 'tcp', -- 'tcp' or 'udp'
    enabled BOOLEAN NOT NULL DEFAULT 1,
    description TEXT,
    allowed_sources TEXT,
    max_active_tunnels INTEGER,
    idle_timeout_seconds INTEGER,
    max_bytes_per_tunnel INTEGER,
    max_bandwidth_bytes_per_second INTEGER,
    rate_limit_window_seconds INTEGER,
    max_connections_per_window INTEGER,
    max_bytes_per_window INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_nat_mappings_agent ON nat_mappings(agent_id);
CREATE INDEX IF NOT EXISTS idx_nat_mappings_public_port ON nat_mappings(public_port);
CREATE INDEX IF NOT EXISTS idx_nat_mappings_enabled ON nat_mappings(enabled);

CREATE TABLE IF NOT EXISTS nat_usage_windows (
    mapping_id TEXT NOT NULL,
    source_ip TEXT NOT NULL,
    window_start TEXT NOT NULL,
    connection_count INTEGER NOT NULL DEFAULT 0,
    bytes_transferred INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (mapping_id, source_ip, window_start),
    FOREIGN KEY (mapping_id) REFERENCES nat_mappings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_nat_usage_windows_expires ON nat_usage_windows(window_start);
