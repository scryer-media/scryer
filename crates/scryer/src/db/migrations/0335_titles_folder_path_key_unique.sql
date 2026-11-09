-- No two titles may record the same folder.
--
-- The repair before this one cleared every colliding folder and filled in the
-- key for every title that keeps one, so the index builds on any database.
CREATE UNIQUE INDEX IF NOT EXISTS titles_folder_path_key_unique
    ON titles(folder_path_key)
    WHERE folder_path_key IS NOT NULL;
