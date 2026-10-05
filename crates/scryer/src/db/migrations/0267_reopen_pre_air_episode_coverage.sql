-- Episode coverage written on or before the episode's air day came from a
-- search that ran before release. It never expired, so the episode was never
-- searched again once it aired. Drop those receipts so the scope re-enters the
-- search walk; new pre-release searches are fingerprinted apart.
DELETE FROM scope_indexer_coverage
WHERE scope_key LIKE 'episode:%'
  AND EXISTS (
    SELECT 1
    FROM episodes
    WHERE episodes.id = substr(scope_indexer_coverage.scope_key, 9)
      AND episodes.air_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]*'
      AND substr(scope_indexer_coverage.searched_at, 1, 10) <= substr(episodes.air_date, 1, 10)
  );
