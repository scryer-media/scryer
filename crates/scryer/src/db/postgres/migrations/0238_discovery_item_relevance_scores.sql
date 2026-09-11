ALTER TABLE discovery_items ADD COLUMN recommendation_score DOUBLE PRECISION;
ALTER TABLE discovery_items ADD COLUMN base_rank DOUBLE PRECISION;
CREATE INDEX IF NOT EXISTS idx_discovery_items_generation_recommendation
    ON discovery_items(base_generation_id, recommendation_score DESC, rank_score DESC, sort_index ASC);
