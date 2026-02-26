CREATE TABLE IF NOT EXISTS release_download_attempts(
    id TEXT PRIMARY KEY,
    title_id TEXT,
    source_hint TEXT,
    source_title TEXT,
    outcome TEXT NOT NULL,
    error_message TEXT,
    attempted_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_release_download_attempts_outcome_attempted
    ON release_download_attempts (outcome, attempted_at DESC);

CREATE INDEX IF NOT EXISTS idx_release_download_attempts_source_hint
    ON release_download_attempts (source_hint);

CREATE INDEX IF NOT EXISTS idx_release_download_attempts_source_title
    ON release_download_attempts (source_title);
