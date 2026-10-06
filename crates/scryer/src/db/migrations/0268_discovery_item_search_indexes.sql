-- Interactive discovery search was blocked by two full-table scans on large
-- tables, each long enough to stall Scryer's HTTP server (single SQLite
-- connection) until the process was killed.
--
-- 1. discovery_item_rank_components has no index on run_id, so the per-run
--    cleanup / lookup "WHERE run_id = ?" scanned every row (455k rows, ~53s for
--    267 matching rows measured on a real library). Index run_id.
CREATE INDEX IF NOT EXISTS idx_discovery_item_rank_components_run
    ON discovery_item_rank_components(run_id);

-- 2. The discovery listing orders by recommendation_score while filtering only
--    tombstoned_at IS NULL (no base_generation_id filter), so the existing
--    generation-scoped idx_discovery_items_generation_recommendation cannot be
--    used and SQLite falls back to a full scan of discovery_items plus a
--    temp B-tree sort (115k rows, ~22s). A partial index over the active rows
--    serves the ordering directly (~0.03s).
CREATE INDEX IF NOT EXISTS idx_discovery_items_active_reco
    ON discovery_items(recommendation_score DESC)
    WHERE tombstoned_at IS NULL;
