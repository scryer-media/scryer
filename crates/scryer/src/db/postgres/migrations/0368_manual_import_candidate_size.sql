-- The size of each manual-import candidate as the preview found it, so the
-- dialog's estimated upgrade verdict can score a file against a target the
-- operator picks without reading the file again. NULL on selections built
-- before the column existed; those rows show no verdict.
ALTER TABLE manual_import_selection_candidates ADD COLUMN size_bytes BIGINT;
