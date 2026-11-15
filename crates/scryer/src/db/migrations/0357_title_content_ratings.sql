-- A library title's content ratings from SMG, one row per country in SMG's
-- order: the certifications that country's bodies gave it and the minimum age
-- they imply. Metadata hydration replaces a title's rows as a set, an empty
-- set included, and a rematch clears them. Rows go with their title.
--
-- They are the same ratings a request's metadata snapshot captures, in the
-- same shape, and the certification a title shows is picked the way request
-- rules pick it.
CREATE TABLE title_content_ratings (
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    sort_index INTEGER NOT NULL,
    country TEXT NOT NULL,
    certifications_json TEXT NOT NULL DEFAULT '[]',
    age_rating INTEGER,
    age_rating_source TEXT,
    PRIMARY KEY (title_id, sort_index)
);
