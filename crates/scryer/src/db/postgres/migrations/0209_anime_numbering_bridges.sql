-- PostgreSQL twin of migrations/0209_anime_numbering_bridges.sql.
-- The community (AniDB/AniList/MAL) season layout SMG derives for an anime
-- series, stored verbatim per title. It is a cache of an upstream dataset, not
-- an entity: every hydration replaces the row wholesale, and losing it only
-- means the title falls back to plain TVDB numbering until the next hydration.
CREATE TABLE title_anime_numbering_bridges (
    title_id text PRIMARY KEY REFERENCES titles(id) ON DELETE CASCADE,
    generated_on text NOT NULL,
    corroborating_order text,
    seasons_json text NOT NULL DEFAULT '[]',
    updated_at timestamp with time zone NOT NULL
);
