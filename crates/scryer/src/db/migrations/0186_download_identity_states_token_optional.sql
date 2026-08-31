-- Durable tracked state is keyed by the canonical download, not by the legacy
-- wire token. Download clients that legally omit the token (plugin clients)
-- could not persist a terminal/blocked marker at all while the table still
-- required `download_id`, so those items lost their outcome across a restart
-- and re-entered processing. Drop the legacy CHECK, keep
-- `canonical_download_id` mandatory, and give it the same downloads(id)
-- foreign key the other canonical dependents already carry.
CREATE TABLE download_identity_states_0186 (
    id TEXT PRIMARY KEY,
    identity_key TEXT NOT NULL UNIQUE,
    canonical_download_id TEXT NOT NULL,
    download_id TEXT,
    client_id TEXT,
    client_type TEXT,
    download_client_item_id TEXT,
    tracked_state TEXT NOT NULL,
    reason TEXT,
    detail TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id)
);

INSERT INTO download_identity_states_0186 (
    id,
    identity_key,
    canonical_download_id,
    download_id,
    client_id,
    client_type,
    download_client_item_id,
    tracked_state,
    reason,
    detail,
    created_at,
    updated_at
)
SELECT
    id,
    identity_key,
    canonical_download_id,
    download_id,
    client_id,
    client_type,
    download_client_item_id,
    tracked_state,
    reason,
    detail,
    created_at,
    updated_at
FROM download_identity_states;

DROP TABLE download_identity_states;
ALTER TABLE download_identity_states_0186 RENAME TO download_identity_states;

CREATE INDEX idx_download_identity_states_download_id
    ON download_identity_states(client_id, client_type, download_id);
CREATE INDEX idx_download_identity_states_canonical_download_id
    ON download_identity_states(canonical_download_id);
