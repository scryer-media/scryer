-- Make the title external-id key carry the entity kind. See the SQLite
-- migration of the same name for why: `(source, external_id)` collapses
-- `tvdb:movie:7373` and `tvdb:series:307111` onto one unique key, so one
-- title's hydration aborted on another title's id.
--
-- Every step only splits an existing key into a more specific one, so an
-- upgrade can never manufacture a conflict.

ALTER TABLE title_external_ids ADD COLUMN external_kind text DEFAULT ''::text NOT NULL;
ALTER TABLE title_external_ids ADD COLUMN external_key text;

-- Stamp the legacy kind onto each stored id that has none, so the JSON and the
-- projection agree from here on.
UPDATE titles
SET external_ids = COALESCE((
    SELECT jsonb_agg(
        CASE
            WHEN legacy.kind = '' THEN jsonb_build_object(
                'source', legacy.entry -> 'source',
                'value', legacy.entry -> 'value'
            )
            ELSE jsonb_build_object(
                'source', legacy.entry -> 'source',
                'kind', to_jsonb(legacy.kind),
                'value', legacy.entry -> 'value'
            )
        END
        ORDER BY legacy.ordinality
    )
    FROM (
        SELECT
            entry.value AS entry,
            entry.ordinality AS ordinality,
            COALESCE(
                NULLIF(BTRIM(LOWER(entry.value ->> 'kind')), ''),
                CASE
                    WHEN LOWER(BTRIM(COALESCE(entry.value ->> 'source', ''))) IN
                        ('anidb', 'mal', 'anilist', 'kitsu', 'simkl') THEN 'anime'
                    WHEN LOWER(BTRIM(COALESCE(entry.value ->> 'source', ''))) = 'smg'
                        THEN 'title'
                    WHEN LOWER(BTRIM(COALESCE(entry.value ->> 'source', ''))) IN
                        ('tvdb', 'tmdb', 'imdb') AND titles.facet = 'movie' THEN 'movie'
                    WHEN LOWER(BTRIM(COALESCE(entry.value ->> 'source', ''))) IN
                        ('tvdb', 'tmdb', 'imdb') AND titles.facet IN ('series', 'anime')
                        THEN 'series'
                    ELSE ''
                END
            ) AS kind
        FROM jsonb_array_elements(titles.external_ids) WITH ORDINALITY AS entry(value, ordinality)
    ) AS legacy
), titles.external_ids)
WHERE jsonb_typeof(external_ids) = 'array'
  AND jsonb_array_length(external_ids) > 0;

-- Copy the stored kind onto the projection rows.
UPDATE title_external_ids
SET external_kind = COALESCE((
    SELECT BTRIM(LOWER(entry.value ->> 'kind'))
    FROM titles,
         jsonb_array_elements(titles.external_ids) AS entry(value)
    WHERE titles.id = title_external_ids.title_id
      AND LOWER(BTRIM(COALESCE(entry.value ->> 'source', '')))
          = LOWER(BTRIM(title_external_ids.source))
      AND BTRIM(COALESCE(entry.value ->> 'value', ''))
          = BTRIM(title_external_ids.external_id)
      AND BTRIM(COALESCE(entry.value ->> 'kind', '')) <> ''
    LIMIT 1
), '');

UPDATE title_external_ids
SET external_key = CASE
    WHEN external_kind = '' THEN LOWER(source) || ':' || external_id
    ELSE LOWER(source) || ':' || external_kind || ':' || external_id
END;

DROP INDEX IF EXISTS idx_title_external_ids_library_lookup;

CREATE UNIQUE INDEX idx_title_external_ids_library_kind_lookup
    ON title_external_ids USING btree (library_id, source, external_kind, external_id);

CREATE INDEX idx_title_external_ids_library_source_value
    ON title_external_ids USING btree (library_id, source, external_id);
