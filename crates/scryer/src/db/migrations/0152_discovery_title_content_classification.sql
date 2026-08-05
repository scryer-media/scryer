ALTER TABLE discovery_titles
    ADD COLUMN is_adult INTEGER NOT NULL DEFAULT 0;

ALTER TABLE discovery_titles
    ADD COLUMN content_ratings_json TEXT NOT NULL DEFAULT '[]';
