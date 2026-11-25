-- Reversible bad-media identity blocks and the reports that created them.
-- Media issues are permanent audit records; they are never pruned or archived
-- automatically, and the details field is bounded by the application.
CREATE TABLE IF NOT EXISTS library_content_blocks (
    id text PRIMARY KEY NOT NULL,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    scheme text NOT NULL,
    size_bytes bigint NOT NULL,
    sample_bytes bigint NOT NULL,
    sample_blake3 text NOT NULL,
    reason text,
    approving_actor_id text,
    source_report_id text,
    source_media_file_id text,
    provenance_json text,
    created_at timestamp with time zone NOT NULL,
    UNIQUE (library_id, scheme, size_bytes, sample_bytes, sample_blake3)
);

CREATE INDEX IF NOT EXISTS idx_library_content_blocks_match
    ON library_content_blocks(library_id, scheme, size_bytes, sample_blake3);
CREATE INDEX IF NOT EXISTS idx_library_content_blocks_created
    ON library_content_blocks(library_id, created_at, id);

ALTER TABLE media_files ADD COLUMN content_block_id text
    REFERENCES library_content_blocks(id) ON DELETE SET NULL;
CREATE INDEX idx_media_files_content_block ON media_files(content_block_id)
    WHERE content_block_id IS NOT NULL;
CREATE INDEX idx_media_files_content_block_candidate ON media_files(size_bytes, title_id);

CREATE TABLE IF NOT EXISTS media_issues (
    id text PRIMARY KEY NOT NULL,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    title_id text NOT NULL,
    media_file_id text NOT NULL,
    file_path_snapshot text NOT NULL,
    file_version text NOT NULL,
    size_bytes bigint NOT NULL,
    scheme text NOT NULL,
    sample_bytes bigint NOT NULL,
    sample_blake3 text NOT NULL,
    reporter_user_id text NOT NULL,
    reason_code text NOT NULL,
    details text,
    remediation text NOT NULL CHECK (remediation IN ('mark_bad', 'recycle_and_search')),
    status text NOT NULL CHECK (status IN ('pending', 'approved', 'rejected', 'canceled')),
    affected_episode_ids_json text NOT NULL DEFAULT '[]',
    affected_series_movie_link_ids_json text NOT NULL DEFAULT '[]',
    rule_decision_id text,
    reviewed_by_user_id text,
    reviewed_at timestamp with time zone,
    execution_status text NOT NULL CHECK (execution_status IN ('none', 'blocked', 'recycling', 'recycle_failed', 'no_eligible_release', 'download_queued', 'replacement_imported', 'searching', 'complete', 'failed')),
    execution_error_code text,
    recycle_preview_fingerprint text,
    recycle_receipt_json text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
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
