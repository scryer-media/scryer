-- Admin-defined title tags: the registry half.
--
-- Membership stays where it already is, as a label inside the `titles.tags`
-- JSON bag, so every existing reader (delay-profile matching, maintenance
-- facts, release-rule context, the managed locale packs) keeps working with no
-- join and no rewrite. What changes is that the bag is now gated: an unprefixed
-- label may only be written if a row here defines it.
--
-- The `scryer:` namespace inside the bag is untouched by this table. Those
-- entries are structured per-title settings (quality profile, monitor type, the
-- anime metadata trio) and are never user tags, so they are never registered.
--
-- `label` is stored already normalized (trimmed, lowercased, internal
-- whitespace collapsed) and is unique, because it *is* the join key against the
-- bag. `created_by` is nullable so the companion
-- `adopt_existing_title_tag_definitions` hook can adopt labels that predate the
-- registry without inventing an author for them.
CREATE TABLE title_tag_definitions (
    id TEXT PRIMARY KEY NOT NULL,
    label TEXT NOT NULL,
    description TEXT,
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE UNIQUE INDEX idx_title_tag_definitions_label
    ON title_tag_definitions(label);

-- Series movies carry their own tags. A series movie is a `series_movie_links`
-- row, not a title, so its membership cannot live in `titles.tags`: the link
-- points at a shared `movie_entities` record that several series may reference,
-- and the same movie placed in two series is two independently taggable things.
--
-- Same shape and same rules as the title bag: a JSON array of normalized
-- labels, gated by the registry above, with reserved `scryer:` entries
-- preserved should anything ever write one here.
ALTER TABLE series_movie_links ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
