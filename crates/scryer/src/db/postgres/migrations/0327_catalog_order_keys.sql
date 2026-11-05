ALTER TABLE titles ADD COLUMN catalog_name_tie_key TEXT NOT NULL DEFAULT '';
DROP INDEX IF EXISTS idx_titles_catalog_sort_key;
