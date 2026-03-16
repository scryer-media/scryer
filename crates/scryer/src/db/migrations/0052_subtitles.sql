-- Subtitle management tables

CREATE TABLE subtitle_downloads (
    id TEXT PRIMARY KEY,
    media_file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    episode_id TEXT,
    language TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_file_id TEXT,
    file_path TEXT NOT NULL,
    score INTEGER,
    hearing_impaired INTEGER NOT NULL DEFAULT 0,
    forced INTEGER NOT NULL DEFAULT 0,
    ai_translated INTEGER NOT NULL DEFAULT 0,
    machine_translated INTEGER NOT NULL DEFAULT 0,
    uploader TEXT,
    release_info TEXT,
    synced INTEGER NOT NULL DEFAULT 0,
    downloaded_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_subtitle_downloads_media_file ON subtitle_downloads(media_file_id);
CREATE INDEX idx_subtitle_downloads_title ON subtitle_downloads(title_id);
CREATE INDEX idx_subtitle_downloads_language ON subtitle_downloads(language);

CREATE TABLE subtitle_blacklist (
    id TEXT PRIMARY KEY,
    media_file_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_file_id TEXT NOT NULL,
    language TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(media_file_id, provider, provider_file_id)
);

CREATE INDEX idx_subtitle_blacklist_media_file ON subtitle_blacklist(media_file_id);
