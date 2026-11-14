-- A download's import artifacts in the order they were recorded, and the
-- child side of the artifact's reference to its download.
CREATE INDEX IF NOT EXISTS idx_download_import_artifacts_canonical_download
    ON download_import_artifacts(canonical_download_id, created_at);

-- The blocklist page, newest first.
CREATE INDEX IF NOT EXISTS idx_blocklist_created_at
    ON blocklist(created_at);

-- The wanted list across every status, most recently changed first.
CREATE INDEX IF NOT EXISTS idx_wanted_items_updated_at
    ON wanted_items(updated_at);

-- A title's ratings by normalized source name. The expression is the one the
-- catalog's rating sorts test, so a sort reads only the ratings of the titles
-- in its scope that come from the sources it wants.
CREATE INDEX IF NOT EXISTS idx_title_metadata_external_ratings_title_source_key
    ON title_metadata_external_ratings(
        title_id,
        LOWER(REPLACE(REPLACE(REPLACE(TRIM(source), ' ', ''), '_', ''), '-', '')),
        normalized
    );

-- One library's catalog page by year, runtime, popularity or status, either
-- direction. Each index holds the sort's ORDER BY terms in order after the
-- library, so a page is read in order and stops at its limit.
CREATE INDEX IF NOT EXISTS idx_titles_library_year_order_asc ON titles (
    library_id,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX IF NOT EXISTS idx_titles_library_year_order_desc ON titles (
    library_id,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year DESC,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX IF NOT EXISTS idx_titles_library_runtime_order_asc ON titles (
    library_id,
    (CASE WHEN runtime_minutes IS NULL THEN 1 ELSE 0 END) ASC,
    runtime_minutes ASC,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX IF NOT EXISTS idx_titles_library_runtime_order_desc ON titles (
    library_id,
    (CASE WHEN runtime_minutes IS NULL THEN 1 ELSE 0 END) ASC,
    runtime_minutes DESC,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX IF NOT EXISTS idx_titles_library_popularity_order_asc ON titles (
    library_id,
    (CASE WHEN popularity IS NULL THEN 1 ELSE 0 END) ASC,
    popularity ASC,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX IF NOT EXISTS idx_titles_library_popularity_order_desc ON titles (
    library_id,
    (CASE WHEN popularity IS NULL THEN 1 ELSE 0 END) ASC,
    popularity DESC,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX IF NOT EXISTS idx_titles_library_status_order_asc ON titles (
    library_id,
    (CASE WHEN CASE LOWER(TRIM(COALESCE(content_status, ''))) WHEN 'returning' THEN 'continuing' WHEN 'finished' THEN 'ended' ELSE LOWER(TRIM(COALESCE(content_status, ''))) END = '' THEN 1 ELSE 0 END) ASC,
    (CASE LOWER(TRIM(COALESCE(content_status, ''))) WHEN 'returning' THEN 'continuing' WHEN 'finished' THEN 'ended' ELSE LOWER(TRIM(COALESCE(content_status, ''))) END) ASC,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX IF NOT EXISTS idx_titles_library_status_order_desc ON titles (
    library_id,
    (CASE WHEN CASE LOWER(TRIM(COALESCE(content_status, ''))) WHEN 'returning' THEN 'continuing' WHEN 'finished' THEN 'ended' ELSE LOWER(TRIM(COALESCE(content_status, ''))) END = '' THEN 1 ELSE 0 END) DESC,
    (CASE LOWER(TRIM(COALESCE(content_status, ''))) WHEN 'returning' THEN 'continuing' WHEN 'finished' THEN 'ended' ELSE LOWER(TRIM(COALESCE(content_status, ''))) END) DESC,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);

-- The requests list is ordered by `updated_at`, then `created_at`, then `id`,
-- newest first, and the dashboard reads pending requests oldest first. Neither
-- order had an index, so every page sorted the matching requests.
CREATE INDEX IF NOT EXISTS idx_media_requests_updated_order
    ON media_requests(updated_at, created_at, id);
CREATE INDEX IF NOT EXISTS idx_media_requests_status_created
    ON media_requests(status, created_at, id);

-- A title's history shows the requests submitted for it. A submission event
-- carries its request only inside the compressed payload, so finding one
-- title's submissions meant decoding every submission on the instance.
-- `media_request_id` is that request as a plain column, like `download_id`.
-- Rows stored before the column existed are filled by the migration's Rust
-- step, which decodes each submission's payload. The index is partial because
-- no other event type carries the key.
ALTER TABLE domain_events ADD COLUMN media_request_id TEXT;

CREATE INDEX IF NOT EXISTS idx_domain_events_media_request_sequence
    ON domain_events(media_request_id, sequence)
    WHERE media_request_id IS NOT NULL;
