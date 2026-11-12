-- "Searched, nothing found" rows for automatic subtitle searches: one per media
-- file and wanted language. An automatic search skips the language until
-- retry_after, while the file, wanted languages and providers asked are the
-- ones recorded here. Rows go with their media file; nothing else prunes them.
CREATE TABLE IF NOT EXISTS subtitle_search_backoffs (
    media_file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    language TEXT NOT NULL,
    hearing_impaired INTEGER NOT NULL,
    forced INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    wanted_languages TEXT NOT NULL,
    enabled_providers TEXT NOT NULL,
    empty_searches INTEGER NOT NULL,
    searched_at TEXT NOT NULL,
    retry_after TEXT NOT NULL,
    PRIMARY KEY (media_file_id, language, hearing_impaired, forced)
);
