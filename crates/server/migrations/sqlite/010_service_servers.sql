-- Many-to-many service probe assignments.
--
-- `services.server_id` is kept as a legacy/compatibility column. The
-- dashboard and monitor now read `service_servers`, and this migration
-- backfills the new relation from existing single-server assignments.

CREATE TABLE IF NOT EXISTS service_servers (
    service_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (service_id, server_id),
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE,
    FOREIGN KEY (server_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_service_servers_server ON service_servers(server_id);

INSERT OR IGNORE INTO service_servers (service_id, server_id)
SELECT id, server_id
FROM services
WHERE server_id IS NOT NULL AND TRIM(server_id) <> '';
