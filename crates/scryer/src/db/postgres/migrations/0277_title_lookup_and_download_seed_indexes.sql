-- Release and import lookup reads title terms by exact spelling without a
-- facet. Every other literal_term index leads with facet, so the spelling side
-- of the lookup scanned the whole terms table.
CREATE INDEX IF NOT EXISTS idx_title_search_terms_kind_literal
    ON title_search_terms(term_kind, literal_term);

-- Cleanup seeding pages through downloads in creation order.
CREATE INDEX IF NOT EXISTS idx_downloads_created_at_id
    ON downloads(created_at, id);

-- Between full passes, cleanup seeding reads only the downloads whose
-- identity state or submission state changed since its last pass.
CREATE INDEX IF NOT EXISTS idx_download_identity_states_updated_at
    ON download_identity_states(updated_at);
CREATE INDEX IF NOT EXISTS idx_download_submissions_tracked_state_at
    ON download_submissions(tracked_state_at);

-- One row: changes before changed_through have been examined, and the last
-- complete full pass started at full_scan_at. No row means the next pass is a
-- full one.
CREATE TABLE IF NOT EXISTS download_cleanup_seed_state (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    changed_through TIMESTAMPTZ NOT NULL,
    full_scan_at TIMESTAMPTZ NOT NULL
);
