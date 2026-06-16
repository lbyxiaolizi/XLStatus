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
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE INDEX idx_nat_mappings_agent ON nat_mappings(agent_id);
CREATE INDEX idx_nat_mappings_public_port ON nat_mappings(public_port);
CREATE INDEX idx_nat_mappings_enabled ON nat_mappings(enabled);
