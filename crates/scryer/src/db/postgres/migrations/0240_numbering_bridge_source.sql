-- Which dataset a stored numbering bridge came from. See the SQLite twin.
ALTER TABLE title_anime_numbering_bridges
    ADD COLUMN source text NOT NULL DEFAULT 'anime_community';
