CREATE TABLE IF NOT EXISTS temporary_transfer_tokens (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    server_id TEXT NOT NULL,
    path TEXT NOT NULL,
    op TEXT NOT NULL CHECK(op IN ('download', 'upload')),
    issued_by_user_id TEXT NOT NULL,
    auth_kind TEXT NOT NULL CHECK(auth_kind IN ('session', 'pat')),
    session_id TEXT,
    api_token_id TEXT,
    scope TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at TEXT,
    used_ip TEXT,
    used_status TEXT,
    used_error TEXT,
    agent_task_id TEXT,
    revoked_at TEXT,
    created_at TEXT NOT NULL,
    created_ip TEXT,
    FOREIGN KEY (server_id) REFERENCES agents(id) ON DELETE CASCADE,
    FOREIGN KEY (issued_by_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL,
    FOREIGN KEY (api_token_id) REFERENCES personal_access_tokens(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_temporary_transfer_tokens_hash
    ON temporary_transfer_tokens(token_hash);
CREATE INDEX IF NOT EXISTS idx_temporary_transfer_tokens_expires
    ON temporary_transfer_tokens(expires_at);
CREATE INDEX IF NOT EXISTS idx_temporary_transfer_tokens_server
    ON temporary_transfer_tokens(server_id, created_at DESC);
