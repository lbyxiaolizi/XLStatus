-- M4 Service Monitoring tables (PostgreSQL variant).
-- See sqlite/003_services.sql for the full rationale.

CREATE TABLE IF NOT EXISTS services (
    id UUID PRIMARY KEY NOT NULL,
    owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    name VARCHAR(255) NOT NULL,
    type VARCHAR(50) NOT NULL,
    target TEXT NOT NULL,
    interval_seconds INTEGER NOT NULL DEFAULT 60,
    timeout_seconds INTEGER NOT NULL DEFAULT 10,
    enabled BOOLEAN NOT NULL DEFAULT true,
    notification_group_id UUID,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_services_enabled ON services(enabled);
CREATE INDEX IF NOT EXISTS idx_services_owner ON services(owner_user_id);

CREATE TABLE IF NOT EXISTS service_results (
    id UUID PRIMARY KEY NOT NULL,
    service_id UUID NOT NULL,
    server_id UUID,
    status VARCHAR(20) NOT NULL,
    delay_ms INTEGER,
    status_code INTEGER,
    error TEXT,
    cert_fingerprint TEXT,
    cert_not_after TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_service_results_service ON service_results(service_id, created_at);
CREATE INDEX IF NOT EXISTS idx_service_results_checked ON service_results(created_at);

CREATE TABLE IF NOT EXISTS service_history (
    id UUID PRIMARY KEY NOT NULL,
    service_id UUID NOT NULL,
    success BOOLEAN NOT NULL,
    latency_ms INTEGER,
    status_code INTEGER,
    error TEXT,
    checked_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_service_history_service ON service_history(service_id, checked_at);
CREATE INDEX IF NOT EXISTS idx_service_history_checked ON service_history(checked_at);
