CREATE TABLE pending_releases (
    id TEXT PRIMARY KEY,
    wanted_item_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    release_title TEXT NOT NULL,
    release_url TEXT,
    release_size_bytes INTEGER,
    release_score INTEGER NOT NULL,
    scoring_log_json TEXT,
    indexer_source TEXT,
    release_guid TEXT,
    added_at TEXT NOT NULL,
    delay_until TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'waiting',
    grabbed_at TEXT
);

CREATE INDEX idx_pending_releases_status ON pending_releases(status);
CREATE INDEX idx_pending_releases_wanted ON pending_releases(wanted_item_id, status);
