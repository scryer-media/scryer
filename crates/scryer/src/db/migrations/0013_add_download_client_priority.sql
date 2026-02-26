ALTER TABLE download_clients
    ADD COLUMN client_priority INTEGER NOT NULL DEFAULT 0;

WITH ranked_clients AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            ORDER BY
                COALESCE(created_at, '1970-01-01T00:00:00Z'),
                id
        ) AS client_priority
    FROM download_clients
)
UPDATE download_clients
SET client_priority = (
    SELECT ranked_clients.client_priority
    FROM ranked_clients
    WHERE ranked_clients.id = download_clients.id
)
WHERE client_priority = 0;

UPDATE download_clients
SET client_priority = 1
WHERE client_priority IS NULL OR client_priority < 1;

CREATE INDEX IF NOT EXISTS idx_download_clients_client_priority
    ON download_clients (client_priority);
