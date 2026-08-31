ALTER TABLE manual_import_selections
    ADD COLUMN trusted_source_root TEXT NOT NULL DEFAULT '';
ALTER TABLE manual_import_selections
    ADD COLUMN archive_workspace_root TEXT;
