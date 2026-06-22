-- M4 Service Monitoring tables.
--
-- Earlier iterations had this migration define `service_history`
-- (with `success` / `latency_ms` / `checked_at`) and a `services`
-- table that no code in the crate actually reads. The current
-- iteration unifies the schema:
--
--   * `services`        - per-row monitor definition
--   * `service_results` - one row per probe attempt
--
-- Both `services` and `service_results` are referenced by the
-- service monitor (`crates/server/src/services/monitor.rs`), the
-- alert engine (`crates/server/src/alerts/engine.rs`), and the
-- service history REST API (`crates/server/src/api/v1/service_history.rs`).

CREATE TABLE IF NOT EXISTS services (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT,
    name TEXT NOT NULL,
    type TEXT NOT NULL, -- 'http' | 'tcp' | 'icmp'
    target TEXT NOT NULL,
    interval_seconds INTEGER NOT NULL DEFAULT 60,
    timeout_seconds INTEGER NOT NULL DEFAULT 10,
    enabled INTEGER NOT NULL DEFAULT 1,
    notification_group_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_services_enabled ON services(enabled);
-- `idx_services_owner` is created from DatabaseBackend::run_migrations after
-- the owner_user_id compatibility column has been added to legacy tables.

CREATE TABLE IF NOT EXISTS service_results (
    id TEXT PRIMARY KEY NOT NULL,
    service_id TEXT NOT NULL,
    server_id TEXT,
    status TEXT NOT NULL,          -- 'success' | 'failure'
    delay_ms INTEGER,
    status_code INTEGER,
    error TEXT,
    cert_fingerprint TEXT,
    cert_not_after TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_service_results_service ON service_results(service_id, created_at);
CREATE INDEX IF NOT EXISTS idx_service_results_checked ON service_results(created_at);

-- Keep the old `service_history` table for backward compatibility
-- with the existing service_history REST API: it is just a synonym
-- view of `service_results`.
CREATE TABLE IF NOT EXISTS service_history (
    id TEXT PRIMARY KEY NOT NULL,
    service_id TEXT NOT NULL,
    success INTEGER NOT NULL,
    latency_ms INTEGER,
    status_code INTEGER,
    error TEXT,
    checked_at TEXT NOT NULL,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_service_history_service ON service_history(service_id, checked_at);
CREATE INDEX IF NOT EXISTS idx_service_history_checked ON service_history(checked_at);
