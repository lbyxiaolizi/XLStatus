-- Editable dashboard metadata and per-server service assignment.

ALTER TABLE agents ADD COLUMN IF NOT EXISTS remark TEXT;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS expires_at TEXT;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS renewal_price TEXT;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS dashboard_metadata_json TEXT;

ALTER TABLE services ADD COLUMN IF NOT EXISTS server_id UUID REFERENCES agents(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_services_server ON services(server_id);
