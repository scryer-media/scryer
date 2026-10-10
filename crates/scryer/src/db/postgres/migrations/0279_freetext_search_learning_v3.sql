-- Title-text search learning moved from one shared `v2:freetext` row per
-- (indexer, title, facet) to one `v3:freetext:<query digest>` row per query.
-- A shared row's counters cannot be split by the queries that produced them,
-- so it maps deterministically onto one reserved v3 row for the same
-- (indexer, title, facet). Readers only consult text rows for a usable
-- success, so a shared row without one carries nothing forward and is
-- dropped, as is every row whose title or indexer no longer exists.
DELETE FROM indexer_search_learning
 WHERE NOT EXISTS (SELECT 1 FROM titles t WHERE t.id = indexer_search_learning.title_id)
    OR NOT EXISTS (SELECT 1 FROM indexers i WHERE i.id = indexer_search_learning.indexer_id);

INSERT INTO indexer_search_learning (
    indexer_id, title_id, facet, strategy_key, attempts, empty_successes,
    usable_successes, last_attempt_at, last_usable_at, suppressed, updated_at
)
SELECT indexer_id, title_id, facet, 'v3:freetext:v2-aggregate', attempts, empty_successes,
       usable_successes, last_attempt_at, last_usable_at, FALSE, updated_at
  FROM indexer_search_learning
 WHERE strategy_key = 'v2:freetext' AND usable_successes > 0
ON CONFLICT (indexer_id, title_id, facet, strategy_key) DO NOTHING;

DELETE FROM indexer_search_learning WHERE strategy_key = 'v2:freetext';

-- Apply the per-title cap the store enforces on write (32 text rows per
-- indexer, title and facet; usable rows first, then the most recent).
DELETE FROM indexer_search_learning
 WHERE (indexer_id, title_id, facet, strategy_key) IN (
    SELECT indexer_id, title_id, facet, strategy_key
      FROM (
        SELECT indexer_id, title_id, facet, strategy_key,
               ROW_NUMBER() OVER (
                   PARTITION BY indexer_id, title_id, facet
                   ORDER BY CASE WHEN usable_successes > 0 THEN 0 ELSE 1 END,
                            COALESCE(last_attempt_at, updated_at) DESC,
                            strategy_key
               ) AS keep_rank
          FROM indexer_search_learning
         WHERE strategy_key LIKE 'v3:freetext:%'
      ) ranked
     WHERE keep_rank > 32
 );
