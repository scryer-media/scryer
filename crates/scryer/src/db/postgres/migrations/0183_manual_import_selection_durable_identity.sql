ALTER TABLE manual_import_selections ADD COLUMN canonical_download_id text;

CREATE INDEX idx_manual_import_selections_canonical_download
    ON manual_import_selections (canonical_download_id, actor_user_id, title_id, updated_at DESC);
