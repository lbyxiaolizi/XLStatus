-- Add agent-related tables for M2

-- Enrollment tokens (one-time use)
CREATE TABLE IF NOT EXISTS enrollment_tokens (
    id TEXT PRIMARY KEY NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_by_user_id TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at TEXT,
    used_by_agent_id TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_enrollment_tokens_hash ON enrollment_tokens(token_hash);
CREATE INDEX IF NOT EXISTS idx_enrollment_tokens_expires ON enrollment_tokens(expires_at);

-- Agents
CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    public_key TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    last_seen_at TEXT,
    revoked_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_agents_owner ON agents(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_agents_revoked ON agents(revoked_at);

-- Update servers table to link to agents.
-- SQLite does not support `ADD COLUMN IF NOT EXISTS`, so the column is
-- added idempotently from `DatabaseBackend::run_migrations`, followed by
-- the `idx_servers_agent` index.
