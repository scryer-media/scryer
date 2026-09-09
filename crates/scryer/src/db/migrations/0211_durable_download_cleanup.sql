CREATE TABLE download_cleanup (
    download_id TEXT PRIMARY KEY REFERENCES downloads(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    client_type TEXT NOT NULL,
    item_id TEXT NOT NULL,
    title_id TEXT,
    facet TEXT,
    source_title TEXT,
    tracked_state TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'completed')),
    attempts BIGINT NOT NULL DEFAULT 0,
    history_offset BIGINT NOT NULL DEFAULT 0,
    payload_checkpoint TEXT,
    outcome TEXT,
    last_error TEXT,
    next_attempt_at TEXT NOT NULL,
    lease_until TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_download_cleanup_due ON download_cleanup(status, next_attempt_at, client_id);
