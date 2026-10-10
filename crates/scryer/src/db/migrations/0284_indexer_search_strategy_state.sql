-- Background search runs record where each strategy stands, so a strategy
-- the provider cannot finish (a result ceiling, a saturated partition) waits
-- out a backoff instead of being re-asked every pass. NULL marks runs written
-- before this column and operator or interactive runs, which never move the
-- background state.
ALTER TABLE indexer_search_runs ADD COLUMN strategy_state TEXT;

-- The convergence scope a background session searched for, so the
-- acquisition gate can tell a contained indexer from an uncovered one.
ALTER TABLE indexer_search_runs ADD COLUMN coverage_scope_key TEXT;

CREATE INDEX IF NOT EXISTS idx_indexer_search_runs_coverage_scope
    ON indexer_search_runs(coverage_scope_key, indexer_id, created_at DESC);
