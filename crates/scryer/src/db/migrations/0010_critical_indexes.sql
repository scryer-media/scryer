CREATE INDEX IF NOT EXISTS idx_titles_facet_monitored
    ON titles (facet, monitored);

CREATE INDEX IF NOT EXISTS idx_releases_title_scope
    ON releases (title_id, release_scope);

CREATE INDEX IF NOT EXISTS idx_media_files_title_path
    ON media_files (title_id, file_path);

CREATE INDEX IF NOT EXISTS idx_history_title_time
    ON history_events (title_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_history_type_time
    ON history_events (event_type, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_operations_status_time
    ON workflow_operations (status, started_at DESC);
