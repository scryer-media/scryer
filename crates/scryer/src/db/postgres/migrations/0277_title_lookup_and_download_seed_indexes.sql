-- Release and import lookup reads title terms by exact spelling without a
-- facet. Every other literal_term index leads with facet, so the spelling side
-- of the lookup scanned the whole terms table.
CREATE INDEX IF NOT EXISTS idx_title_search_terms_kind_literal
    ON title_search_terms(term_kind, literal_term);

-- Cleanup seeding pages through downloads in creation order.
CREATE INDEX IF NOT EXISTS idx_downloads_created_at_id
    ON downloads(created_at, id);
