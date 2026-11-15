-- A library title's content ratings from SMG, one row per country in SMG's
-- order: the certifications that country's bodies gave it and the minimum age
-- they imply. Metadata hydration replaces a title's rows as a set, an empty
-- set included; other writes leave them alone. Rows go with their title.
--
-- They are the same ratings a request's metadata snapshot captures, in the
-- same shape, so the certification a title shows is the one request rules
-- judge it by.
CREATE TABLE title_content_ratings (
    title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    sort_index integer NOT NULL,
    country text NOT NULL,
    certifications_json text NOT NULL DEFAULT '[]',
    age_rating integer,
    age_rating_source text,
    PRIMARY KEY (title_id, sort_index)
);
