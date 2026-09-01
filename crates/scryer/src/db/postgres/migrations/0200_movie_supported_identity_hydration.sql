-- Give currently idle movie titles one hydration attempt now that non-TVDB
-- movie identities are supported. Leave active retries and nonzero attempt
-- counters untouched; terminal rows whose cleared state reset the counter may
-- receive this one-time retry.
UPDATE titles
SET metadata_hydration_next_attempt_at = NOW(),
    metadata_hydration_attempt_count = 0
WHERE facet = 'movie'
  AND metadata_fetched_at IS NULL
  AND metadata_hydration_next_attempt_at IS NULL
  AND metadata_hydration_attempt_count = 0
  AND EXISTS (
      SELECT 1
      FROM jsonb_array_elements(COALESCE(titles.external_ids, '[]'::jsonb)) AS external_id
      WHERE LOWER(BTRIM(COALESCE(external_id ->> 'source', '')))
              IN ('smg', 'tvdb', 'tmdb', 'imdb')
        AND BTRIM(COALESCE(external_id ->> 'value', '')) <> ''
  );
