-- WAF and security operation records.

CREATE TABLE IF NOT EXISTS waf_bans (
    id TEXT PRIMARY KEY,
    ip TEXT NOT NULL UNIQUE,
    reason TEXT NOT NULL,
    failed_count INTEGER NOT NULL DEFAULT 0,
    banned_until TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_waf_bans_until ON waf_bans(banned_until);

CREATE TABLE IF NOT EXISTS waf_events (
    id TEXT PRIMARY KEY,
    ip TEXT NOT NULL,
    username TEXT,
    outcome TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_waf_events_ip ON waf_events(ip, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_waf_events_outcome ON waf_events(outcome, created_at DESC);
