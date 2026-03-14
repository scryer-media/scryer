-- Unified title history table: every lifecycle event (grab, download, import,
-- failure, deletion, rename) for a title is recorded as a single row with an
-- event_type discriminator and a flexible data_json column for event-specific
-- metadata.  Modeled after Sonarr's EpisodeHistory.

CREATE TABLE IF NOT EXISTS title_history (
    id          TEXT PRIMARY KEY,
    title_id    TEXT NOT NULL,
    episode_id  TEXT,
    collection_id TEXT,
    event_type  TEXT NOT NULL,
    source_title TEXT,
    quality     TEXT,
    download_id TEXT,
    data_json   TEXT,
    occurred_at TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_title_history_title_id
    ON title_history (title_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_title_history_event_type
    ON title_history (event_type, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_title_history_download_id
    ON title_history (download_id)
    WHERE download_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_title_history_occurred_at
    ON title_history (occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_title_history_episode_id
    ON title_history (episode_id, occurred_at DESC)
    WHERE episode_id IS NOT NULL;
