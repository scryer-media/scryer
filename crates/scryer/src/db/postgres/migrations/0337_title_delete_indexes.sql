-- Indexes for the statements that remove a deleted title's rows by title id.
-- Each of these tables grows with the library or with activity, and no index
-- led with title_id, so every title deletion read the whole table. Adding an
-- index removes and changes no rows.

-- Manual import selections a deleted title leaves behind.
CREATE INDEX IF NOT EXISTS idx_manual_import_selections_title_id
    ON manual_import_selections(title_id);

-- Pending import rows that name a title.
CREATE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_title_id
    ON library_scan_unmatched_items(title_id);

-- Search learning per title; the key leads with the indexer.
CREATE INDEX IF NOT EXISTS idx_indexer_search_learning_title_id
    ON indexer_search_learning(title_id);
