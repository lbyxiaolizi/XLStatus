-- M8 high-IO compatibility helpers for SQLite.
--
-- SQLite remains the development/small-deployment backend, so it does not
-- support PostgreSQL-style range partitions. This migration keeps the
-- schema behavior aligned where it matters locally: retention policy
-- metadata plus recent-window indexes for deterministic verification.

CREATE TABLE IF NOT EXISTS xlstatus_metric_retention_policies (
    table_name TEXT PRIMARY KEY,
    retention_days INTEGER NOT NULL CHECK (retention_days > 0),
    partition_interval TEXT NOT NULL DEFAULT 'none',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT OR IGNORE INTO xlstatus_metric_retention_policies
    (table_name, retention_days, partition_interval)
VALUES
    ('service_results', 30, 'none'),
    ('task_runs', 30, 'none'),
    ('audit_logs', 90, 'none'),
    ('transfers', 30, 'none');

CREATE INDEX IF NOT EXISTS idx_service_results_service_created_desc
    ON service_results(service_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_service_results_server_created_desc
    ON service_results(server_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_service_results_status_created_desc
    ON service_results(status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_task_runs_task_created_desc
    ON task_runs(task_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_task_runs_server_created_desc
    ON task_runs(server_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_task_runs_status_created_desc
    ON task_runs(status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_audit_logs_created_desc
    ON audit_logs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_resource_created_desc
    ON audit_logs(resource_type, resource_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_transfers_owner_created_desc
    ON transfers(owner_user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_transfers_server_created_desc
    ON transfers(server_id, created_at DESC);
