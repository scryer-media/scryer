-- Indexes no query uses: nothing filters, joins or sorts on their leading
-- columns, and none backs a key, a foreign key or an upsert target.
DROP INDEX IF EXISTS idx_subtitle_downloads_language;
DROP INDEX IF EXISTS idx_pending_releases_indexer_id;
DROP INDEX IF EXISTS idx_release_download_attempts_source_hint;
DROP INDEX IF EXISTS idx_release_download_attempts_source_title;
DROP INDEX IF EXISTS idx_history_events_type_time;
DROP INDEX IF EXISTS idx_download_submission_episode_links_episode;
DROP INDEX IF EXISTS idx_download_identity_states_download_id;
DROP INDEX IF EXISTS idx_media_files_full_blake3;

-- Duplicates of a wider key on the same leading columns: the unique
-- target-scope index, and the table's own UNIQUE(legacy_collection_id).
DROP INDEX IF EXISTS idx_notification_subscriptions_target;
DROP INDEX IF EXISTS idx_series_movie_links_legacy_collection;

-- Recording a release decision looks up the scope's existing decision for
-- the same release and code, once per candidate of every search batch.
CREATE INDEX IF NOT EXISTS idx_release_decisions_identity
    ON release_decisions (wanted_item_id, release_title, decision_code);

-- A recovery pass reads the submissions with a password retry pending.
CREATE INDEX IF NOT EXISTS idx_download_submissions_password_retry
    ON download_submissions (id)
    WHERE password_retry_state IS NOT NULL;

-- Resolving a client's item to its download starts from the item id; the
-- client columns are compared after normalisation and cannot be sought.
CREATE INDEX IF NOT EXISTS idx_download_client_bindings_active_native_item
    ON download_client_bindings (native_item_id)
    WHERE native_item_id IS NOT NULL AND ended_at IS NULL;

-- Mismatch recovery pages one title's scopes in id order.
CREATE INDEX IF NOT EXISTS idx_wanted_items_title_id_id
    ON wanted_items (title_id, id);
