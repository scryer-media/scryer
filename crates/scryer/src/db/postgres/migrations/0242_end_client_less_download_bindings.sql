-- Retire legacy client-less download attribution.
--
-- Migration 0179 backfilled `download_client_bindings` (and left
-- `download_submissions.download_client_id` blank) for rows written before
-- per-client attribution existed. The runtime carried a second resolution rule
-- for those rows — "the single configured client of this type" — inside the
-- binding lookup, the guard and the reconciler. That rule is deleted, so the
-- rows are resolved once, here.
--
-- A row whose client type has exactly one configured client can only have come
-- from it, so it is attributed. Anything else (no client of that type, or two
-- or more) is unattributable, and an unattributable binding is ended: nothing
-- can ever read it, and a binding that cannot be read must not hold a scope.
UPDATE download_client_bindings
SET client_config_id = (
    SELECT dc.id
    FROM download_clients dc
    WHERE LOWER(TRIM(COALESCE(dc.client_type, '')))
          = LOWER(TRIM(COALESCE(download_client_bindings.client_type_snapshot, '')))
)
WHERE ended_at IS NULL
  AND TRIM(COALESCE(client_config_id, '')) = ''
  AND (
        SELECT COUNT(*)
        FROM download_clients dc_all
        WHERE LOWER(TRIM(COALESCE(dc_all.client_type, '')))
              = LOWER(TRIM(COALESCE(download_client_bindings.client_type_snapshot, '')))
      ) = 1
  -- Never collide with idx_download_client_bindings_active_locator_unique.
  AND NOT EXISTS (
        SELECT 1
        FROM download_client_bindings other
        WHERE other.download_id <> download_client_bindings.download_id
          AND other.ended_at IS NULL
          AND other.native_item_id = download_client_bindings.native_item_id
          AND LOWER(TRIM(COALESCE(other.client_type_snapshot, '')))
              = LOWER(TRIM(COALESCE(download_client_bindings.client_type_snapshot, '')))
          AND TRIM(COALESCE(other.client_config_id, '')) <> ''
      );

UPDATE download_client_bindings
SET ended_at = CURRENT_TIMESTAMP
WHERE ended_at IS NULL
  AND TRIM(COALESCE(client_config_id, '')) = '';

-- The same attribution for the durable submissions, so the guard can tell which
-- client a row runs on. A submission that stays blank is left alone: it names no
-- client, so nothing treats it as blocked and nothing waits on it.
UPDATE download_submissions
SET download_client_id = (
    SELECT dc.id
    FROM download_clients dc
    WHERE LOWER(TRIM(COALESCE(dc.client_type, '')))
          = LOWER(TRIM(COALESCE(download_submissions.download_client_type, '')))
)
WHERE TRIM(COALESCE(download_client_id, '')) = ''
  AND (
        SELECT COUNT(*)
        FROM download_clients dc_all
        WHERE LOWER(TRIM(COALESCE(dc_all.client_type, '')))
              = LOWER(TRIM(COALESCE(download_submissions.download_client_type, '')))
      ) = 1
  -- Never collide with UNIQUE(download_client_id, download_client_type, download_client_item_id).
  AND NOT EXISTS (
        SELECT 1
        FROM download_submissions other
        WHERE other.id <> download_submissions.id
          AND other.download_client_type = download_submissions.download_client_type
          AND COALESCE(other.download_client_item_id, '')
              = COALESCE(download_submissions.download_client_item_id, '')
          AND TRIM(COALESCE(other.download_client_id, '')) <> ''
      );
