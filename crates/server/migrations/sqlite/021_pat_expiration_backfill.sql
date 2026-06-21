UPDATE personal_access_tokens
SET expires_at = strftime('%Y-%m-%dT%H:%M:%SZ', created_at, '+90 days')
WHERE expires_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_pat_expires_at
    ON personal_access_tokens(expires_at);
