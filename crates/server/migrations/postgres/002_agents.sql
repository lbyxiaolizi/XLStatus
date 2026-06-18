-- Add agent-related tables for M2

-- Enrollment tokens (one-time use)
CREATE TABLE IF NOT EXISTS enrollment_tokens (
    id UUID PRIMARY KEY NOT NULL,
    token_hash VARCHAR(255) NOT NULL UNIQUE,
    created_by_user_id UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    used_by_agent_id UUID,
    created_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_enrollment_tokens_hash ON enrollment_tokens(token_hash);
CREATE INDEX IF NOT EXISTS idx_enrollment_tokens_expires ON enrollment_tokens(expires_at);

-- Agents
CREATE TABLE IF NOT EXISTS agents (
    id UUID PRIMARY KEY NOT NULL,
    name VARCHAR(255) NOT NULL,
    public_key TEXT NOT NULL,
    owner_user_id UUID NOT NULL,
    last_seen_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_agents_owner ON agents(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_agents_revoked ON agents(revoked_at);

-- Update servers table to link to agents
ALTER TABLE servers ADD COLUMN IF NOT EXISTS agent_id UUID REFERENCES agents(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_servers_agent ON servers(agent_id);
