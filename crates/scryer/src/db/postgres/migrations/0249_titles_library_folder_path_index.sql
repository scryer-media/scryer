-- Index the folder-ownership lookup. See the SQLite twin for which read this
-- serves and why the existing `titles` indexes do not answer it.
CREATE INDEX IF NOT EXISTS idx_titles_library_folder_path
    ON titles(library_id, folder_path);
