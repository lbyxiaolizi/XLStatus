-- M6 NAT 穿透相关表

-- NAT 映射配置
CREATE TABLE IF NOT EXISTS nat_mappings (
    id UUID PRIMARY KEY NOT NULL,
    agent_id UUID NOT NULL,
    local_host VARCHAR(255) NOT NULL,
    local_port INTEGER NOT NULL,
    public_port INTEGER NOT NULL UNIQUE,
    protocol VARCHAR(10) NOT NULL DEFAULT 'tcp',
    enabled BOOLEAN NOT NULL DEFAULT true,
    description TEXT,
    allowed_sources TEXT,
    max_active_tunnels INTEGER,
    idle_timeout_seconds INTEGER,
    max_bytes_per_tunnel BIGINT,
    max_bandwidth_bytes_per_second BIGINT,
    rate_limit_window_seconds INTEGER,
    max_connections_per_window INTEGER,
    max_bytes_per_window BIGINT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_nat_mappings_agent ON nat_mappings(agent_id);
CREATE INDEX IF NOT EXISTS idx_nat_mappings_public_port ON nat_mappings(public_port);
CREATE INDEX IF NOT EXISTS idx_nat_mappings_enabled ON nat_mappings(enabled);

ALTER TABLE nat_mappings ADD COLUMN IF NOT EXISTS allowed_sources TEXT;
ALTER TABLE nat_mappings ADD COLUMN IF NOT EXISTS max_active_tunnels INTEGER;
ALTER TABLE nat_mappings ADD COLUMN IF NOT EXISTS idle_timeout_seconds INTEGER;
ALTER TABLE nat_mappings ADD COLUMN IF NOT EXISTS max_bytes_per_tunnel BIGINT;
ALTER TABLE nat_mappings ADD COLUMN IF NOT EXISTS max_bandwidth_bytes_per_second BIGINT;
ALTER TABLE nat_mappings ADD COLUMN IF NOT EXISTS rate_limit_window_seconds INTEGER;
ALTER TABLE nat_mappings ADD COLUMN IF NOT EXISTS max_connections_per_window INTEGER;
ALTER TABLE nat_mappings ADD COLUMN IF NOT EXISTS max_bytes_per_window BIGINT;

CREATE TABLE IF NOT EXISTS nat_usage_windows (
    mapping_id UUID NOT NULL,
    source_ip TEXT NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    connection_count BIGINT NOT NULL DEFAULT 0,
    bytes_transferred BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (mapping_id, source_ip, window_start),
    FOREIGN KEY (mapping_id) REFERENCES nat_mappings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_nat_usage_windows_expires ON nat_usage_windows(window_start);
