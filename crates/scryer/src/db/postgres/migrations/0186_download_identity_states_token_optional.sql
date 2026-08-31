-- PostgreSQL twin of migrations/0186_download_identity_states_token_optional.sql.
-- Drop the legacy wire-token requirement so token-less (plugin client)
-- downloads can persist durable tracked state, keep canonical_download_id
-- mandatory, and add the downloads(id) foreign key the other canonical
-- dependents already carry.
ALTER TABLE download_identity_states
    DROP CONSTRAINT IF EXISTS download_identity_states_download_id_check;
ALTER TABLE download_identity_states
    DROP CONSTRAINT IF EXISTS download_identity_states_check;

ALTER TABLE download_identity_states
    ADD CONSTRAINT download_identity_states_canonical_download_id_fkey
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id);

CREATE INDEX idx_download_identity_states_canonical_download_id
    ON download_identity_states(canonical_download_id);
