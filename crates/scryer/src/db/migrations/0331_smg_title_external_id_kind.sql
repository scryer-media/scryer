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
    SELECT json_group_array(
        CASE
            WHEN LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) = 'smg'
             AND TRIM(COALESCE(json_extract(entry.value, '$.kind'), '')) != ''
            THEN json_set(json(entry.value), '$.kind', 'title')
            ELSE json(entry.value)
        END
    )
    FROM json_each(titles.external_ids) AS entry
    WHERE NOT (
        LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) = 'smg'
        AND TRIM(COALESCE(json_extract(entry.value, '$.kind'), '')) NOT IN ('', 'title')
        AND EXISTS (
            SELECT 1
            FROM json_each(titles.external_ids) AS twin
            WHERE LOWER(TRIM(COALESCE(json_extract(twin.value, '$.source'), ''))) = 'smg'
              AND LOWER(TRIM(COALESCE(json_extract(twin.value, '$.kind'), ''))) = 'title'
              AND TRIM(COALESCE(json_extract(twin.value, '$.value'), ''))
                  = TRIM(COALESCE(json_extract(entry.value, '$.value'), ''))
        )
    )
)
WHERE json_valid(external_ids)
  AND json_type(external_ids) = 'array'
  AND EXISTS (
      SELECT 1
      FROM json_each(titles.external_ids) AS entry
      WHERE LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) = 'smg'
        AND LOWER(TRIM(COALESCE(json_extract(entry.value, '$.kind'), ''))) NOT IN ('', 'title')
  );

DELETE FROM title_external_ids
WHERE LOWER(TRIM(source)) = 'smg'
  AND external_kind NOT IN ('', 'title')
  AND EXISTS (
      SELECT 1
      FROM title_external_ids AS twin
      WHERE twin.title_id = title_external_ids.title_id
        AND LOWER(TRIM(twin.source)) = 'smg'
        AND twin.external_kind = 'title'
        AND TRIM(twin.external_id) = TRIM(title_external_ids.external_id)
  );

UPDATE title_external_ids
SET external_kind = 'title',
    external_key = LOWER(TRIM(source)) || ':title:' || external_id
WHERE LOWER(TRIM(source)) = 'smg'
  AND external_kind NOT IN ('', 'title')
  AND NOT EXISTS (
      SELECT 1
      FROM title_external_ids AS other
      WHERE other.library_id IS title_external_ids.library_id
        AND other.source = title_external_ids.source
        AND other.external_kind = 'title'
        AND other.external_id = title_external_ids.external_id
  );
