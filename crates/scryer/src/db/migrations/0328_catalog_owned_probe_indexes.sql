-- Find the catalog titles a set of discovery items can name.
--
-- Discovery decides which candidates the catalog already owns by comparing a
-- candidate's target key with each title's external ids, either as the id
-- lowercased or as the number its last segment holds. Without an index on
-- those forms, the only way to answer was to read every title. These index
-- the id lowercased and the number formed by its trailing digits, for both
-- `title_external_ids` and the legacy `titles.imdb_id` column, so the check
-- reads only the titles its candidates name.
CREATE INDEX IF NOT EXISTS idx_title_external_ids_value_nocase
    ON title_external_ids(LOWER(TRIM(external_id)));
CREATE INDEX IF NOT EXISTS idx_title_external_ids_numeric_suffix
    ON title_external_ids(
        CAST(SUBSTR(TRIM(external_id), LENGTH(RTRIM(TRIM(external_id), '0123456789')) + 1) AS INTEGER)
    );
CREATE INDEX IF NOT EXISTS idx_titles_imdb_id_nocase
    ON titles(LOWER(TRIM(imdb_id)))
    WHERE imdb_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_titles_imdb_id_numeric_suffix
    ON titles(
        CAST(SUBSTR(TRIM(imdb_id), LENGTH(RTRIM(TRIM(imdb_id), '0123456789')) + 1) AS INTEGER)
    )
    WHERE imdb_id IS NOT NULL;
