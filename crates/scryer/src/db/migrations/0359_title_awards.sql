-- A library title's awards from SMG, one row per award in SMG's order: the
-- award, the year it was given, its recipient, and whether the title or a
-- person won it. Metadata hydration replaces a title's rows as a set, an empty
-- set included, and a rematch clears them. Rows go with their title.
--
-- content_ratings_stored_at and awards_stored_at record when hydration last
-- stored each set. NULL means the set has not been stored: the title was never
-- hydrated, was rematched since, or was last hydrated before the set was
-- kept. Rules read a NULL set as unknown, never as "has none".
CREATE TABLE title_awards (
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    sort_index INTEGER NOT NULL,
    award_qid TEXT NOT NULL DEFAULT '',
    award_label TEXT NOT NULL DEFAULT '',
    award_year INTEGER,
    recipient_qid TEXT NOT NULL DEFAULT '',
    recipient_label TEXT NOT NULL DEFAULT '',
    claim_side TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (title_id, sort_index)
);

ALTER TABLE titles ADD COLUMN content_ratings_stored_at TEXT;
ALTER TABLE titles ADD COLUMN awards_stored_at TEXT;
