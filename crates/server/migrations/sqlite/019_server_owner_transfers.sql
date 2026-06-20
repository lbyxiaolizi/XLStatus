CREATE TABLE IF NOT EXISTS server_owner_transfers (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    from_user_id TEXT,
    to_user_id TEXT NOT NULL,
    requested_by_user_id TEXT,
    api_token_id TEXT,
    status TEXT NOT NULL CHECK(status IN ('completed', 'failed', 'cancelled')),
    attempts INTEGER NOT NULL DEFAULT 1,
    error TEXT,
    completed_at TEXT,
    cancelled_at TEXT,
    last_attempt_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
    FOREIGN KEY (from_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (to_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (requested_by_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (api_token_id) REFERENCES personal_access_tokens(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_server_owner_transfers_agent
    ON server_owner_transfers(agent_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_server_owner_transfers_status
    ON server_owner_transfers(status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_server_owner_transfers_requested_by
    ON server_owner_transfers(requested_by_user_id, created_at DESC);
