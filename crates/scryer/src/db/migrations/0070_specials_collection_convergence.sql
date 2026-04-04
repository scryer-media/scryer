DROP TABLE IF EXISTS _legacy_specials_dupes;

CREATE TEMP TABLE _legacy_specials_dupes AS
SELECT legacy.id AS legacy_id, canonical.id AS canonical_id
FROM collections AS legacy
INNER JOIN titles AS title
    ON title.id = legacy.title_id
INNER JOIN collections AS canonical
    ON canonical.title_id = legacy.title_id
   AND canonical.collection_type = 'specials'
   AND canonical.collection_index = '0'
WHERE legacy.collection_type = 'season'
  AND legacy.collection_index = '0'
  AND title.facet IN ('series', 'anime');

DELETE FROM wanted_items
WHERE collection_id IN (SELECT legacy_id FROM _legacy_specials_dupes)
  AND EXISTS (
      SELECT 1
      FROM _legacy_specials_dupes AS merge_map
      INNER JOIN wanted_items AS canonical_item
          ON canonical_item.collection_id = merge_map.canonical_id
      WHERE merge_map.legacy_id = wanted_items.collection_id
  );

UPDATE episodes
SET collection_id = (
    SELECT canonical_id
    FROM _legacy_specials_dupes
    WHERE legacy_id = episodes.collection_id
)
WHERE collection_id IN (SELECT legacy_id FROM _legacy_specials_dupes);

UPDATE releases
SET collection_id = (
    SELECT canonical_id
    FROM _legacy_specials_dupes
    WHERE legacy_id = releases.collection_id
)
WHERE collection_id IN (SELECT legacy_id FROM _legacy_specials_dupes);

UPDATE wanted_items
SET collection_id = (
    SELECT canonical_id
    FROM _legacy_specials_dupes
    WHERE legacy_id = wanted_items.collection_id
)
WHERE collection_id IN (SELECT legacy_id FROM _legacy_specials_dupes);

UPDATE workflow_operations
SET collection_id = (
    SELECT canonical_id
    FROM _legacy_specials_dupes
    WHERE legacy_id = workflow_operations.collection_id
)
WHERE collection_id IN (SELECT legacy_id FROM _legacy_specials_dupes);

UPDATE title_history
SET collection_id = (
    SELECT canonical_id
    FROM _legacy_specials_dupes
    WHERE legacy_id = title_history.collection_id
)
WHERE collection_id IN (SELECT legacy_id FROM _legacy_specials_dupes);

UPDATE download_submissions
SET collection_id = (
    SELECT canonical_id
    FROM _legacy_specials_dupes
    WHERE legacy_id = download_submissions.collection_id
)
WHERE collection_id IN (SELECT legacy_id FROM _legacy_specials_dupes);

UPDATE collections AS canonical
SET label = COALESCE(
        NULLIF(canonical.label, ''),
        (
            SELECT NULLIF(legacy.label, '')
            FROM collections AS legacy
            INNER JOIN _legacy_specials_dupes AS merge_map
                ON merge_map.legacy_id = legacy.id
            WHERE merge_map.canonical_id = canonical.id
            LIMIT 1
        )
    ),
    ordered_path = COALESCE(
        canonical.ordered_path,
        (
            SELECT legacy.ordered_path
            FROM collections AS legacy
            INNER JOIN _legacy_specials_dupes AS merge_map
                ON merge_map.legacy_id = legacy.id
            WHERE merge_map.canonical_id = canonical.id
            LIMIT 1
        )
    ),
    narrative_order = COALESCE(
        canonical.narrative_order,
        (
            SELECT legacy.narrative_order
            FROM collections AS legacy
            INNER JOIN _legacy_specials_dupes AS merge_map
                ON merge_map.legacy_id = legacy.id
            WHERE merge_map.canonical_id = canonical.id
            LIMIT 1
        )
    ),
    first_episode_number = COALESCE(
        canonical.first_episode_number,
        (
            SELECT legacy.first_episode_number
            FROM collections AS legacy
            INNER JOIN _legacy_specials_dupes AS merge_map
                ON merge_map.legacy_id = legacy.id
            WHERE merge_map.canonical_id = canonical.id
            LIMIT 1
        )
    ),
    last_episode_number = COALESCE(
        canonical.last_episode_number,
        (
            SELECT legacy.last_episode_number
            FROM collections AS legacy
            INNER JOIN _legacy_specials_dupes AS merge_map
                ON merge_map.legacy_id = legacy.id
            WHERE merge_map.canonical_id = canonical.id
            LIMIT 1
        )
    ),
    monitored = CASE
        WHEN canonical.monitored = 1 THEN 1
        WHEN EXISTS (
            SELECT 1
            FROM collections AS legacy
            INNER JOIN _legacy_specials_dupes AS merge_map
                ON merge_map.legacy_id = legacy.id
            WHERE merge_map.canonical_id = canonical.id
              AND legacy.monitored = 1
        ) THEN 1
        ELSE canonical.monitored
    END,
    special_movies_json = CASE
        WHEN COALESCE(NULLIF(canonical.special_movies_json, ''), '[]') <> '[]' THEN canonical.special_movies_json
        ELSE COALESCE(
            (
                SELECT NULLIF(legacy.special_movies_json, '[]')
                FROM collections AS legacy
                INNER JOIN _legacy_specials_dupes AS merge_map
                    ON merge_map.legacy_id = legacy.id
                WHERE merge_map.canonical_id = canonical.id
                LIMIT 1
            ),
            canonical.special_movies_json
        )
    END
WHERE canonical.id IN (SELECT canonical_id FROM _legacy_specials_dupes);

DELETE FROM collections
WHERE id IN (SELECT legacy_id FROM _legacy_specials_dupes);

UPDATE collections
SET collection_type = 'specials'
WHERE collection_type = 'season'
  AND collection_index = '0'
  AND title_id IN (
      SELECT id
      FROM titles
      WHERE facet IN ('series', 'anime')
  );

DROP TABLE IF EXISTS _legacy_specials_dupes;
