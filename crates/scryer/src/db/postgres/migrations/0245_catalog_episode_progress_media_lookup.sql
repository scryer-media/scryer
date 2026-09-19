-- Cover the media-file lookup the catalog's episode-progress join makes. See
-- the SQLite twin for why the primary-key row fetch was the cost.
CREATE INDEX IF NOT EXISTS idx_media_files_id_role_path
    ON media_files(id, role, file_path);
