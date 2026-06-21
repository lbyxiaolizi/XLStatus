UPDATE personal_access_tokens
SET expires_at = created_at + INTERVAL '90 days'
WHERE expires_at IS NULL;

ALTER TABLE personal_access_tokens
    ALTER COLUMN expires_at SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_pat_expires_at
    ON personal_access_tokens(expires_at);
