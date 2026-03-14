-- Dedicated blocklist table for fast acquisition-time lookups.
-- When an import fails, the source_title is blocklisted so the same release
-- is not re-grabbed.  Users can remove entries to retry.

CREATE TABLE IF NOT EXISTS blocklist (
    id           TEXT PRIMARY KEY,
    title_id     TEXT NOT NULL,
    source_title TEXT,
    source_hint  TEXT,
    quality      TEXT,
    download_id  TEXT,
    reason       TEXT,
    data_json    TEXT,
    created_at   TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_blocklist_title_id
    ON blocklist (title_id);

CREATE INDEX IF NOT EXISTS idx_blocklist_source_title
    ON blocklist (source_title)
    WHERE source_title IS NOT NULL;
