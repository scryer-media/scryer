-- Make the title external-id key carry the entity kind.
--
-- `(source, external_id)` is ambiguous: SMG hands out `tvdb:movie:7373` and
-- `tvdb:series:307111` as unrelated identities, and AniDB/MAL ids name anime
-- entries rather than the TVDB series. Projecting them onto `(library_id,
-- source, external_id)` made two different entities collide on one unique
-- index, so hydrating one title aborted and discarded every other metadata
-- field on it.
--
-- The kind is backfilled into `titles.external_ids` first so the stored JSON
-- and the projection agree, then copied onto the projection rows. Every step
-- only *splits* an existing key into a more specific one -- no two rows are
-- ever merged -- so an upgrade cannot manufacture a conflict, and a movie and
-- a series that share a TVDB number stop colliding.

ALTER TABLE title_external_ids ADD COLUMN external_kind TEXT NOT NULL DEFAULT '';
ALTER TABLE title_external_ids ADD COLUMN external_key TEXT;

-- Stamp the legacy kind onto each stored id that has none. Sources that name
-- an anime entry are always `anime`; SMG's own id names a title; the shared
-- movie/series sources take the kind from the title's facet. Anything else
-- stays kindless and keeps matching any kind.
UPDATE titles
SET external_ids = (
    SELECT json_group_array(
        CASE
            WHEN COALESCE(
                NULLIF(TRIM(LOWER(json_extract(entry.value, '$.kind'))), ''),
                CASE
                    WHEN LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) IN
                        ('anidb', 'mal', 'anilist', 'kitsu', 'simkl') THEN 'anime'
                    WHEN LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) = 'smg'
                        THEN 'title'
                    WHEN LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) IN
                        ('tvdb', 'tmdb', 'imdb') AND titles.facet = 'movie' THEN 'movie'
                    WHEN LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) IN
                        ('tvdb', 'tmdb', 'imdb') AND titles.facet IN ('series', 'anime')
                        THEN 'series'
                    ELSE ''
                END
            ) = ''
            THEN json_object(
                'source', json_extract(entry.value, '$.source'),
                'value', json_extract(entry.value, '$.value')
            )
            ELSE json_object(
                'source', json_extract(entry.value, '$.source'),
                'kind', COALESCE(
                    NULLIF(TRIM(LOWER(json_extract(entry.value, '$.kind'))), ''),
                    CASE
                        WHEN LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) IN
                            ('anidb', 'mal', 'anilist', 'kitsu', 'simkl') THEN 'anime'
                        WHEN LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) = 'smg'
                            THEN 'title'
                        WHEN LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) IN
                            ('tvdb', 'tmdb', 'imdb') AND titles.facet = 'movie' THEN 'movie'
                        WHEN LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), ''))) IN
                            ('tvdb', 'tmdb', 'imdb') AND titles.facet IN ('series', 'anime')
                            THEN 'series'
                        ELSE ''
                    END
                ),
                'value', json_extract(entry.value, '$.value')
            )
        END
    )
    FROM json_each(titles.external_ids) AS entry
)
WHERE json_valid(external_ids)
  AND json_array_length(external_ids) > 0;

-- Copy the stored kind onto the projection. The join is on the old key, which
-- is still unique per title, so each row takes the kind of the entry it was
-- projected from.
UPDATE title_external_ids
SET external_kind = COALESCE((
    SELECT TRIM(LOWER(json_extract(entry.value, '$.kind')))
    FROM titles, json_each(titles.external_ids) AS entry
    WHERE titles.id = title_external_ids.title_id
      AND LOWER(TRIM(COALESCE(json_extract(entry.value, '$.source'), '')))
          = LOWER(TRIM(title_external_ids.source))
      AND TRIM(COALESCE(json_extract(entry.value, '$.value'), ''))
          = TRIM(title_external_ids.external_id)
      AND TRIM(COALESCE(json_extract(entry.value, '$.kind'), '')) != ''
    LIMIT 1
), '');

UPDATE title_external_ids
SET external_key = CASE
    WHEN external_kind = '' THEN LOWER(source) || ':' || external_id
    ELSE LOWER(source) || ':' || external_kind || ':' || external_id
END;

DROP INDEX IF EXISTS idx_title_external_ids_library_lookup;

CREATE UNIQUE INDEX idx_title_external_ids_library_kind_lookup
    ON title_external_ids(library_id, source, external_kind, external_id);

-- Callers without a kind (user input, plugins, pre-kind rows) still match on
-- source and id alone, so that shape keeps an index of its own.
CREATE INDEX idx_title_external_ids_library_source_value
    ON title_external_ids(library_id, source, external_id);
