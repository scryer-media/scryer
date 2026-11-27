-- Bad-file reports and the title-scoped content block they confirm.
--
-- A confirmed bad file is a row in the title's blocklist. Besides the release
-- identity it may carry a sampled content identity: the file size plus a
-- BLAKE3 digest of the first and last MiB (the import content proof). A bad
-- file from a known grab is one row carrying both identities; a file of
-- unknown provenance is a content-only row with no release name.
ALTER TABLE blocklist ADD COLUMN content_scheme text;
ALTER TABLE blocklist ADD COLUMN content_size_bytes bigint;
ALTER TABLE blocklist ADD COLUMN content_sample_bytes bigint;
ALTER TABLE blocklist ADD COLUMN content_sample_blake3 text;

-- Release uniqueness covers rows that name a release. A content-only row has
-- no release name and the empty indexer, so without this predicate a second
-- content-only row on one title would collide with the first.
DROP INDEX IF EXISTS idx_blocklist_release_unique;
CREATE UNIQUE INDEX idx_blocklist_release_unique
    ON blocklist (title_id, indexer_id, normalized_release_name)
    WHERE info_hash IS NULL AND release_name IS NOT NULL;

-- One content identity per title. It also serves the import gate's read of a
-- title's content identities, which repeats the predicate.
CREATE UNIQUE INDEX idx_blocklist_content_identity_unique
    ON blocklist (title_id, content_scheme, content_size_bytes, content_sample_bytes, content_sample_blake3)
    WHERE content_sample_blake3 IS NOT NULL;

-- The title's files that match a confirmed bad file. Only presence and
-- cutoff ignore a stamped file. The partial index keeps the clear path, and
-- the foreign key action when a blocklist row is deleted, from scanning
-- media_files.
ALTER TABLE media_files ADD COLUMN bad_file_blocklist_id text
    REFERENCES blocklist(id) ON DELETE SET NULL;
CREATE INDEX idx_media_files_bad_file_blocklist
    ON media_files (bad_file_blocklist_id)
    WHERE bad_file_blocklist_id IS NOT NULL;

-- Reports are kept as audit records. Nothing prunes them; the details text is
-- bounded by the application.
CREATE TABLE media_issues (
    id text PRIMARY KEY NOT NULL,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    title_id text NOT NULL,
    media_file_id text NOT NULL,
    file_path_snapshot text NOT NULL,
    file_version text NOT NULL,
    content_scheme text NOT NULL,
    size_bytes bigint NOT NULL,
    sample_bytes bigint NOT NULL,
    sample_blake3 text NOT NULL,
    reporter_user_id text NOT NULL,
    reason text NOT NULL CHECK (reason IN ('corrupted', 'wrong_content', 'wrong_language', 'audio_out_of_sync', 'poor_quality', 'other')),
    details text,
    remediation text NOT NULL CHECK (remediation IN ('mark_bad', 'recycle_and_search')),
    authorized_remediation text CHECK (authorized_remediation IN ('mark_bad', 'recycle_and_search')),
    status text NOT NULL CHECK (status IN ('pending', 'approved', 'rejected', 'canceled')),
    affected_episode_ids_json text NOT NULL DEFAULT '[]',
    affected_series_movie_link_ids_json text NOT NULL DEFAULT '[]',
    rule_decision_id text,
    blocklist_id text REFERENCES blocklist(id) ON DELETE SET NULL,
    reviewed_by_user_id text,
    reviewed_at timestamp with time zone,
    execution_status text NOT NULL CHECK (execution_status IN ('none', 'blocked', 'recycling', 'recycle_failed', 'no_eligible_release', 'download_queued', 'searching')),
    execution_error_code text,
    recycle_preview_fingerprint text,
    recycle_receipt_json text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE INDEX idx_media_issues_library_status
    ON media_issues (library_id, status, created_at, id);
CREATE INDEX idx_media_issues_title
    ON media_issues (title_id, created_at, id);
CREATE INDEX idx_media_issues_blocklist
    ON media_issues (blocklist_id)
    WHERE blocklist_id IS NOT NULL;
CREATE INDEX idx_media_issues_recycle_recovery
    ON media_issues (updated_at, id)
    WHERE execution_status = 'recycling';
CREATE UNIQUE INDEX idx_media_issues_report_identity
    ON media_issues (media_file_id, file_version, reporter_user_id, remediation)
    WHERE status <> 'canceled';
