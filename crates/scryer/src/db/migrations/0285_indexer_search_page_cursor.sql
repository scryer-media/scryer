-- A paged indexer reads a background strategy a few provider pages per pass.
-- The run that stopped with pages left records where the next pass resumes;
-- NULL means the strategy started afresh or the provider ran out of pages.
ALTER TABLE indexer_search_runs ADD COLUMN page_cursor TEXT;

-- The runs that read one strategy page by page share a chain id, so a later
-- pass replays every page read so far, not only the last one.
ALTER TABLE indexer_search_runs ADD COLUMN cursor_chain_id TEXT;
