-- Movies that only carried a TMDB or IMDb id were previously parked because
-- title hydration requires TVDB. Requeue them for the title-id hydration path.
UPDATE titles
SET metadata_hydration_next_attempt_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    metadata_hydration_attempt_count = 0
WHERE facet = 'movie'
  AND metadata_fetched_at IS NULL
  AND EXISTS (
      SELECT 1
      FROM json_each(titles.external_ids) AS external_id
      WHERE LOWER(TRIM(COALESCE(json_extract(external_id.value, '$.source'), ''))) IN ('tmdb', 'imdb')
        AND TRIM(COALESCE(json_extract(external_id.value, '$.value'), '')) != ''
  )
  AND NOT EXISTS (
      SELECT 1
      FROM json_each(titles.external_ids) AS external_id
      WHERE LOWER(TRIM(COALESCE(json_extract(external_id.value, '$.source'), ''))) = 'tvdb'
        AND TRIM(COALESCE(json_extract(external_id.value, '$.value'), '')) != ''
  );
