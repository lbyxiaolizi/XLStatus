-- Service monitor coverage mode.

ALTER TABLE services ADD COLUMN IF NOT EXISTS cover_mode TEXT NOT NULL DEFAULT 'local';
ALTER TABLE services ADD COLUMN IF NOT EXISTS exclude_server_ids_json TEXT;

UPDATE services
SET cover_mode = 'specific'
WHERE server_id IS NOT NULL AND cover_mode = 'local';
