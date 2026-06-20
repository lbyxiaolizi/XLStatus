CREATE TABLE IF NOT EXISTS agent_ip_events (
    id UUID PRIMARY KEY NOT NULL,
    agent_id UUID NOT NULL,
    old_ipv4 TEXT,
    new_ipv4 TEXT,
    old_ipv6 TEXT,
    new_ipv6 TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_agent_ip_events_agent ON agent_ip_events(agent_id, created_at DESC);
