-- Reversible bad-media identity blocks and the reports that created them.
--
-- The identity is deliberately sampled, not a full-file or metadata identity:
-- size plus the existing 1 MiB head/tail ImportContentProof semantics. Blocks
-- are independent of media_files so recycling or deleting a file cannot erase
-- the block. Scope is library-scoped, not instance-scoped.
-- Media issues are permanent audit records; they are never pruned or archived
-- automatically, and the details field is bounded by the application.
CREATE TABLE IF NOT EXISTS library_content_blocks (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    scheme TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    sample_bytes INTEGER NOT NULL,
    sample_blake3 TEXT NOT NULL,
    reason TEXT,
    approving_actor_id TEXT,
    source_report_id TEXT,
    source_media_file_id TEXT,
    provenance_json TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (library_id, scheme, size_bytes, sample_bytes, sample_blake3)
);

CREATE INDEX IF NOT EXISTS idx_library_content_blocks_match
    ON library_content_blocks(library_id, scheme, size_bytes, sample_blake3);
CREATE INDEX IF NOT EXISTS idx_library_content_blocks_created
    ON library_content_blocks(library_id, created_at, id);

ALTER TABLE media_files ADD COLUMN content_block_id TEXT
    REFERENCES library_content_blocks(id) ON DELETE SET NULL;
CREATE INDEX idx_media_files_content_block ON media_files(content_block_id)
    WHERE content_block_id IS NOT NULL;
CREATE INDEX idx_media_files_content_block_candidate ON media_files(size_bytes, title_id);

CREATE TABLE IF NOT EXISTS media_issues (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL,
    media_file_id TEXT NOT NULL,
    file_path_snapshot TEXT NOT NULL,
    file_version TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    scheme TEXT NOT NULL,
    sample_bytes INTEGER NOT NULL,
    sample_blake3 TEXT NOT NULL,
    reporter_user_id TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    details TEXT,
    remediation TEXT NOT NULL CHECK (remediation IN ('mark_bad', 'recycle_and_search')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'rejected', 'canceled')),
    affected_episode_ids_json TEXT NOT NULL DEFAULT '[]',
    affected_series_movie_link_ids_json TEXT NOT NULL DEFAULT '[]',
    rule_decision_id TEXT,
    reviewed_by_user_id TEXT,
    reviewed_at TEXT,
    execution_status TEXT NOT NULL CHECK (execution_status IN ('none', 'blocked', 'recycling', 'recycle_failed', 'no_eligible_release', 'download_queued', 'replacement_imported', 'searching', 'complete', 'failed')),
    execution_error_code TEXT,
    recycle_preview_fingerprint TEXT,
    recycle_receipt_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_media_issues_library_status
    ON media_issues(library_id, status, created_at, id);
CREATE INDEX IF NOT EXISTS idx_media_issues_title
    ON media_issues(title_id, created_at, id);
CREATE INDEX IF NOT EXISTS idx_media_issues_file
    ON media_issues(media_file_id, created_at, id);
CREATE INDEX IF NOT EXISTS idx_media_issues_reporter
    ON media_issues(reporter_user_id, created_at, id);
CREATE INDEX idx_media_issues_recycle_recovery ON media_issues(updated_at, id)
    WHERE execution_status = 'recycling';
CREATE UNIQUE INDEX idx_media_issues_report_identity
    ON media_issues(library_id, media_file_id, file_version, reporter_user_id, remediation)
    WHERE status <> 'canceled';
