-- Record where an imported file sat inside the archive workspace it came from.
--
-- Extracted files live in a workspace staged in the title folder, outside the
-- download folder that `relative_path` is measured against, so nothing tied
-- an extracted video back to its import. Releasing held sources removes a
-- workspace only when every video in it is recorded here as imported by the
-- import that owns it. Rows stored before this column existed stay NULL and
-- their workspaces are kept.

ALTER TABLE download_import_artifacts ADD COLUMN IF NOT EXISTS workspace_relative_path TEXT;
