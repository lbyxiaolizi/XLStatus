CREATE TABLE IF NOT EXISTS temporary_transfer_tokens (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    server_id UUID NOT NULL,
    path TEXT NOT NULL,
    op TEXT NOT NULL CHECK(op IN ('download', 'upload')),
    issued_by_user_id UUID NOT NULL,
    auth_kind TEXT NOT NULL CHECK(auth_kind IN ('session', 'pat')),
    session_id UUID,
    api_token_id UUID,
    scope TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    used_ip TEXT,
    used_status TEXT,
    used_error TEXT,
    agent_task_id TEXT,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    created_ip TEXT,
    FOREIGN KEY (server_id) REFERENCES agents(id) ON DELETE CASCADE,
    FOREIGN KEY (issued_by_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL,
    FOREIGN KEY (api_token_id) REFERENCES personal_access_tokens(id) ON DELETE SET NULL
);

ALTER TABLE temporary_transfer_tokens ADD COLUMN IF NOT EXISTS used_ip TEXT;
ALTER TABLE temporary_transfer_tokens ADD COLUMN IF NOT EXISTS used_status TEXT;
ALTER TABLE temporary_transfer_tokens ADD COLUMN IF NOT EXISTS used_error TEXT;
ALTER TABLE temporary_transfer_tokens ADD COLUMN IF NOT EXISTS agent_task_id TEXT;

CREATE INDEX IF NOT EXISTS idx_temporary_transfer_tokens_hash
    ON temporary_transfer_tokens(token_hash);
CREATE INDEX IF NOT EXISTS idx_temporary_transfer_tokens_expires
    ON temporary_transfer_tokens(expires_at);
CREATE INDEX IF NOT EXISTS idx_temporary_transfer_tokens_server
    ON temporary_transfer_tokens(server_id, created_at DESC);
