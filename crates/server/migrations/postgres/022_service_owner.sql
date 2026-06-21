ALTER TABLE services ADD COLUMN IF NOT EXISTS owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL;

UPDATE services s
SET owner_user_id = owners.owner_user_id
FROM (
    SELECT
        ss.service_id,
        (ARRAY_AGG(DISTINCT a.owner_user_id))[1] AS owner_user_id
    FROM service_servers ss
    JOIN agents a ON a.id = ss.server_id
    GROUP BY ss.service_id
    HAVING COUNT(DISTINCT a.owner_user_id) = 1
) owners
WHERE s.id = owners.service_id
  AND s.owner_user_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_services_owner ON services(owner_user_id);
