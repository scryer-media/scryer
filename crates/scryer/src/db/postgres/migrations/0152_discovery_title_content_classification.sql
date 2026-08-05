ALTER TABLE discovery_titles
    ADD COLUMN is_adult BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE discovery_titles
    ADD COLUMN content_ratings_json TEXT NOT NULL DEFAULT '[]';
