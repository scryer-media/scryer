-- TrackedDownloads: scryer-side download lifecycle state machine (plan 055).
--
-- Extend download_submissions with terminal tracked state so scryer can
-- reconstruct its workflow state after restart without re-processing
-- already-handled downloads.
ALTER TABLE download_submissions ADD COLUMN tracked_state TEXT;
ALTER TABLE download_submissions ADD COLUMN tracked_state_at TEXT;

-- Per-file import artifact history.  Append-only audit trail used by
-- verify_import() to prove cumulative completion across multiple import
-- passes (e.g. season pack where episodes arrive in separate runs).
CREATE TABLE download_import_artifacts (
    id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    import_id TEXT,
    relative_path TEXT,
    normalized_file_name TEXT NOT NULL,
    media_kind TEXT NOT NULL,
    title_id TEXT,
    episode_id TEXT,
    season_number INTEGER,
    episode_number INTEGER,
    result TEXT NOT NULL,
    reason_code TEXT,
    imported_media_file_id TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (import_id) REFERENCES imports(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL,
    FOREIGN KEY (imported_media_file_id) REFERENCES media_files(id) ON DELETE SET NULL
);

CREATE INDEX idx_download_import_artifacts_source
    ON download_import_artifacts (source_system, source_ref, created_at);

CREATE INDEX idx_download_import_artifacts_episode
    ON download_import_artifacts (episode_id, result);
