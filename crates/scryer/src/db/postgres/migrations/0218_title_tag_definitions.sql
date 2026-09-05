-- Admin-defined title tags: the registry half. PostgreSQL twin of 0218.
--
-- See the SQLite file for the rationale. Only the column types differ; the
-- `adopt_existing_title_tag_definitions` Rust hook in the same migration
-- performs the identical adoption pass on both engines.
CREATE TABLE title_tag_definitions (
    id text PRIMARY KEY NOT NULL,
    label text NOT NULL,
    description text,
    created_by text,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_title_tag_definitions_label
    ON title_tag_definitions(label);

-- Series-movie tag membership; see the SQLite file for why links carry their
-- own bag instead of borrowing the title's.
ALTER TABLE series_movie_links ADD COLUMN tags jsonb NOT NULL DEFAULT '[]'::jsonb;
