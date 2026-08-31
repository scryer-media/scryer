-- Movies that only carried a TMDB or IMDb id were previously parked because
-- title hydration requires TVDB. Requeue them for the title-id hydration path.
UPDATE titles
SET metadata_hydration_next_attempt_at = NOW(),
    metadata_hydration_attempt_count = 0
WHERE facet = 'movie'
  AND metadata_fetched_at IS NULL
  AND EXISTS (
      SELECT 1
      FROM jsonb_array_elements(COALESCE(titles.external_ids, '[]'::jsonb)) AS external_id
      WHERE LOWER(BTRIM(COALESCE(external_id ->> 'source', ''))) IN ('tmdb', 'imdb')
        AND BTRIM(COALESCE(external_id ->> 'value', '')) <> ''
  )
  AND NOT EXISTS (
      SELECT 1
      FROM jsonb_array_elements(COALESCE(titles.external_ids, '[]'::jsonb)) AS external_id
      WHERE LOWER(BTRIM(COALESCE(external_id ->> 'source', ''))) = 'tvdb'
        AND BTRIM(COALESCE(external_id ->> 'value', '')) <> ''
  );
