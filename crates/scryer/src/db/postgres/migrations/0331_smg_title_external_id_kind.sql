-- Keep one SMG id per title.
--
-- SMG's own id always names one of its titles. Hydration stored it as
-- `smg:title:N`, but metadata search results kind it by facet, so a title
-- added from a search also kept `smg:movie:N` or `smg:series:N`: the same id
-- twice, in `titles.external_ids` and in the projection. Scryer now reads every
-- SMG id as kind `title`; this folds the stored copies onto that kind.
--
-- A facet-kinded copy with a `title` twin on the same title is dropped. One
-- without a twin is relabelled, unless another title in the library already
-- holds that `smg:title` id, in which case it is left as it is rather than
-- break the unique key. Kindless SMG ids are left alone.

UPDATE titles
SET external_ids = (
    SELECT COALESCE(
        jsonb_agg(
            CASE
                WHEN LOWER(BTRIM(COALESCE(entry.value ->> 'source', ''))) = 'smg'
                 AND BTRIM(COALESCE(entry.value ->> 'kind', '')) <> ''
                THEN jsonb_set(entry.value, '{kind}', '"title"'::jsonb)
                ELSE entry.value
            END
            ORDER BY entry.ordinality
        ),
        '[]'::jsonb
    )
    FROM jsonb_array_elements(titles.external_ids) WITH ORDINALITY AS entry(value, ordinality)
    WHERE NOT (
        LOWER(BTRIM(COALESCE(entry.value ->> 'source', ''))) = 'smg'
        AND LOWER(BTRIM(COALESCE(entry.value ->> 'kind', ''))) NOT IN ('', 'title')
        AND EXISTS (
            SELECT 1
            FROM jsonb_array_elements(titles.external_ids) AS twin(value)
            WHERE LOWER(BTRIM(COALESCE(twin.value ->> 'source', ''))) = 'smg'
              AND LOWER(BTRIM(COALESCE(twin.value ->> 'kind', ''))) = 'title'
              AND BTRIM(COALESCE(twin.value ->> 'value', ''))
                  = BTRIM(COALESCE(entry.value ->> 'value', ''))
        )
    )
)
WHERE jsonb_typeof(external_ids) = 'array'
  AND EXISTS (
      SELECT 1
      FROM jsonb_array_elements(titles.external_ids) AS entry(value)
      WHERE LOWER(BTRIM(COALESCE(entry.value ->> 'source', ''))) = 'smg'
        AND LOWER(BTRIM(COALESCE(entry.value ->> 'kind', ''))) NOT IN ('', 'title')
  );

DELETE FROM title_external_ids
WHERE LOWER(BTRIM(source)) = 'smg'
  AND external_kind NOT IN ('', 'title')
  AND EXISTS (
      SELECT 1
      FROM title_external_ids AS twin
      WHERE twin.title_id = title_external_ids.title_id
        AND LOWER(BTRIM(twin.source)) = 'smg'
        AND twin.external_kind = 'title'
        AND BTRIM(twin.external_id) = BTRIM(title_external_ids.external_id)
  );

UPDATE title_external_ids
SET external_kind = 'title',
    external_key = LOWER(BTRIM(source)) || ':title:' || external_id
WHERE LOWER(BTRIM(source)) = 'smg'
  AND external_kind NOT IN ('', 'title')
  AND NOT EXISTS (
      SELECT 1
      FROM title_external_ids AS other
      WHERE other.library_id IS NOT DISTINCT FROM title_external_ids.library_id
        AND other.source = title_external_ids.source
        AND other.external_kind = 'title'
        AND other.external_id = title_external_ids.external_id
  );
