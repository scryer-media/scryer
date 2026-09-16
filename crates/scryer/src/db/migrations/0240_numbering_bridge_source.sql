-- Which dataset a stored numbering bridge came from.
--
-- Until now every row was the anime community (AniBridge) layout SMG derives.
-- Scryer now also builds bridges for ordinary series from the alternate and DVD
-- episode orders TVDB publishes, and a reading through one of those is worth
-- less than a community reading: it has to be corroborated before it may move
-- an import. Existing rows keep the anime meaning through the default.
ALTER TABLE title_anime_numbering_bridges
    ADD COLUMN source TEXT NOT NULL DEFAULT 'anime_community';
