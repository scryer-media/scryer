-- Index the child side of foreign keys that reference titles, collections,
-- episodes and media files.
--
-- Deleting a parent row makes the engine find every referencing child row to
-- apply ON DELETE (or to enforce NO ACTION). Without an index leading on the
-- child column that lookup scans the whole child table once per deleted
-- parent, so removing a title with many episodes and files grew with the size
-- of the activity and import history. Every index here is additive.

CREATE INDEX IF NOT EXISTS idx_workflow_operations_title
    ON workflow_operations (title_id);
CREATE INDEX IF NOT EXISTS idx_workflow_operations_collection
    ON workflow_operations (collection_id);
CREATE INDEX IF NOT EXISTS idx_workflow_operations_episode
    ON workflow_operations (episode_id);
CREATE INDEX IF NOT EXISTS idx_workflow_operations_media_file
    ON workflow_operations (media_file_id);

CREATE INDEX IF NOT EXISTS idx_discovery_pending_changes_title
    ON discovery_pending_context_changes (title_id);
CREATE INDEX IF NOT EXISTS idx_discovery_titles_resolved_title
    ON discovery_titles (resolved_title_id);

CREATE INDEX IF NOT EXISTS idx_download_import_artifacts_title
    ON download_import_artifacts (title_id);
CREATE INDEX IF NOT EXISTS idx_download_import_artifacts_imported_media_file
    ON download_import_artifacts (imported_media_file_id);

CREATE INDEX IF NOT EXISTS idx_release_download_attempts_title
    ON release_download_attempts (title_id);

CREATE INDEX IF NOT EXISTS idx_series_movie_links_linked_episode
    ON series_movie_links (linked_episode_id);

-- The unique collection index only covers collection-scoped rows since 0255,
-- so episode rows that carry their collection need their own lookup path.
CREATE INDEX IF NOT EXISTS idx_wanted_items_episode
    ON wanted_items (episode_id);
CREATE INDEX IF NOT EXISTS idx_wanted_items_collection
    ON wanted_items (collection_id);
