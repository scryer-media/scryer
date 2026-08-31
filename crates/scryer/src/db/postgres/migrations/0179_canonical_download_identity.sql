CREATE TABLE downloads (
    id TEXT PRIMARY KEY,
    origin TEXT NOT NULL CHECK (origin IN ('scryer_submission', 'foreign_observation')),
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    first_observed_at TIMESTAMP WITH TIME ZONE,
    last_observed_at TIMESTAMP WITH TIME ZONE,
    terminal_at TIMESTAMP WITH TIME ZONE
);

CREATE TABLE download_client_bindings (
    download_id TEXT PRIMARY KEY REFERENCES downloads(id),
    client_config_id TEXT,
    client_type_snapshot TEXT,
    client_name_snapshot TEXT,
    native_item_id TEXT,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    last_seen_at TIMESTAMP WITH TIME ZONE,
    ended_at TIMESTAMP WITH TIME ZONE
);

CREATE INDEX idx_download_client_bindings_locator
    ON download_client_bindings(client_config_id, client_type_snapshot, native_item_id);

ALTER TABLE download_submissions
    ALTER COLUMN download_client_item_id DROP NOT NULL;

ALTER TABLE download_identity_states
    ADD COLUMN canonical_download_id TEXT;
ALTER TABLE imports
    ADD COLUMN canonical_download_id TEXT;
ALTER TABLE download_import_artifacts
    ADD COLUMN canonical_download_id TEXT;
ALTER TABLE download_queue_commands
    ADD COLUMN canonical_download_id TEXT;
