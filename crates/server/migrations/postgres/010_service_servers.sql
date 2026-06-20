-- Many-to-many service probe assignments.
--
-- `services.server_id` is kept as a legacy/compatibility column. The
-- dashboard and monitor now read `service_servers`, and this migration
-- backfills the new relation from existing single-server assignments.

CREATE TABLE IF NOT EXISTS service_servers (
    service_id UUID NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    server_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (service_id, server_id)
);

CREATE INDEX IF NOT EXISTS idx_service_servers_server ON service_servers(server_id);

INSERT INTO service_servers (service_id, server_id)
SELECT id, server_id
FROM services
WHERE server_id IS NOT NULL
ON CONFLICT DO NOTHING;
