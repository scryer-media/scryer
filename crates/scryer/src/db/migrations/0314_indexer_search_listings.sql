-- Some indexers ignore the season, episode and absolute-number parameters and
-- answer every scope of a title with the same title-level listing. A listing
-- is identified by its first page. Each row is one listing of one title on one
-- indexer, the background strategy that owns the page chain reading it, and
-- how far that chain has read.
CREATE TABLE IF NOT EXISTS indexer_search_listings (
    indexer_id TEXT NOT NULL,
    indexer_fingerprint TEXT NOT NULL,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    first_page_fingerprint TEXT NOT NULL,
    owner_scope_key TEXT NOT NULL,
    owner_query_signature TEXT NOT NULL,
    owner_strategy_family TEXT NOT NULL,
    page_cursor TEXT,
    finished INTEGER NOT NULL DEFAULT 0,
    chain_started_at TEXT NOT NULL,
    advanced_at TEXT NOT NULL,
    PRIMARY KEY (indexer_id, indexer_fingerprint, title_id, facet, first_page_fingerprint)
);

CREATE INDEX IF NOT EXISTS idx_indexer_search_listings_owner
    ON indexer_search_listings(indexer_id, owner_scope_key, owner_query_signature);

-- A background strategy found reading another strategy's listing: the scope
-- is answered from that listing instead of paging it again.
CREATE TABLE IF NOT EXISTS indexer_search_listing_readers (
    indexer_id TEXT NOT NULL,
    indexer_fingerprint TEXT NOT NULL,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    query_signature TEXT NOT NULL,
    strategy_family TEXT NOT NULL,
    first_page_fingerprint TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (indexer_id, indexer_fingerprint, title_id, facet, scope_key, query_signature)
);

CREATE INDEX IF NOT EXISTS idx_indexer_search_listing_readers_listing
    ON indexer_search_listing_readers(indexer_id, indexer_fingerprint, title_id, facet, first_page_fingerprint);

-- The search runs whose pages make up a listing's chain. Their candidates keep
-- every result the indexer returned, for whichever scope replays them.
CREATE TABLE IF NOT EXISTS indexer_search_listing_runs (
    run_id TEXT NOT NULL,
    indexer_id TEXT NOT NULL,
    indexer_fingerprint TEXT NOT NULL,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    first_page_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (run_id)
);

CREATE INDEX IF NOT EXISTS idx_indexer_search_listing_runs_listing
    ON indexer_search_listing_runs(indexer_id, indexer_fingerprint, title_id, facet, first_page_fingerprint, created_at);
