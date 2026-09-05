-- The community (AniDB/AniList/MAL) season layout SMG derives for an anime
-- series, stored verbatim per title. It is a cache of an upstream dataset, not
-- an entity: every hydration replaces the row wholesale, and losing it only
-- means the title falls back to plain TVDB numbering until the next hydration.
--
-- The seasons are a JSON document rather than a normalized child table because
-- nothing queries inside them: the acquisition and import lanes load the whole
-- bridge for one title and translate in memory.
CREATE TABLE title_anime_numbering_bridges (
    title_id TEXT PRIMARY KEY,
    generated_on TEXT NOT NULL,
    corroborating_order TEXT,
    seasons_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
