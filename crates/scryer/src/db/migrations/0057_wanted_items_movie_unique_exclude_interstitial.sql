-- The idx_wanted_items_movie_unique index enforces one wanted item per movie
-- title, but interstitial anime movies share the same title_id (the parent
-- series) and have episode_id = NULL. Narrow the index to exclude rows that
-- have a collection_id (interstitial movies), which are already uniquely
-- constrained by idx_wanted_items_collection_id.

DROP INDEX IF EXISTS idx_wanted_items_movie_unique;

CREATE UNIQUE INDEX idx_wanted_items_movie_unique
    ON wanted_items(title_id)
    WHERE episode_id IS NULL AND collection_id IS NULL;
