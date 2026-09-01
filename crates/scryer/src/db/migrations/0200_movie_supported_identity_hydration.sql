-- Give currently idle movie titles one hydration attempt now that non-TVDB
-- movie identities are supported. Leave active retries and nonzero attempt
-- counters untouched; terminal rows whose cleared state reset the counter may
-- receive this one-time retry.
UPDATE titles
SET metadata_hydration_next_attempt_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    metadata_hydration_attempt_count = 0
WHERE facet = 'movie'
  AND metadata_fetched_at IS NULL
  AND metadata_hydration_next_attempt_at IS NULL
  AND metadata_hydration_attempt_count = 0
  AND EXISTS (
      SELECT 1
      FROM json_each(titles.external_ids) AS external_id
      WHERE LOWER(TRIM(COALESCE(json_extract(external_id.value, '$.source'), '')))
              IN ('smg', 'tvdb', 'tmdb', 'imdb')
        AND TRIM(COALESCE(json_extract(external_id.value, '$.value'), '')) != ''
  );
