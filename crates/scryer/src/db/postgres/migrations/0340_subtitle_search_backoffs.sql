-- "Searched, nothing found" rows for automatic subtitle searches: one per media
-- file and wanted language. An automatic search skips the language until
-- retry_after, while the file, wanted languages and providers asked are the
-- ones recorded here. Rows go with their media file; nothing else prunes them.
CREATE TABLE IF NOT EXISTS subtitle_search_backoffs (
    media_file_id text NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    language text NOT NULL,
    hearing_impaired boolean NOT NULL,
    forced boolean NOT NULL,
    file_path text NOT NULL,
    size_bytes bigint NOT NULL,
    wanted_languages text NOT NULL,
    enabled_providers text NOT NULL,
    empty_searches integer NOT NULL,
    searched_at timestamp with time zone NOT NULL,
    retry_after timestamp with time zone NOT NULL,
    PRIMARY KEY (media_file_id, language, hearing_impaired, forced)
);
