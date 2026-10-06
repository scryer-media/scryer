-- Run replacement clears rank components by run_id, not item_id.
CREATE INDEX IF NOT EXISTS idx_discovery_item_rank_components_run
    ON discovery_item_rank_components(run_id);

-- Match personalized home filtering and its null-safe relevance ordering.
-- target_key belongs to discovery_titles; only that final tie-breaker still
-- requires sorting, rather than sorting every candidate in the generation.
CREATE INDEX IF NOT EXISTS idx_discovery_items_generation_relevance
    ON discovery_items(
        base_generation_id,
        owned_in_input,
        COALESCE(recommendation_score, -999999999.0) DESC,
        COALESCE(rank_score, -999999999.0) DESC,
        sort_index ASC
    )
    WHERE tombstoned_at IS NULL;
