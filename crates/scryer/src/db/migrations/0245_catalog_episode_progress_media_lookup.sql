-- Cover the media-file lookup the catalog's episode-progress join makes.
--
-- Sorting the title catalog by Episodes joins every episode to its primary
-- media file so the page can count owned episodes. That join resolves
-- `media_files` by primary key and then reads only `role` and `file_path`,
-- but `media_files` rows are wide -- roughly 4 KB each in a populated library,
-- dominated by `analysis_json` and `scoring_log` -- so every owned episode
-- pulled a full 4 KB row off disk to read about eighty bytes of it. At 100k
-- episodes that is the whole cost of the sort.
--
-- Leading with `id` keeps the same primary-key seek the planner already chose
-- and simply carries the two columns it needs, turning the row fetch into a
-- covering index scan. The existing primary-key index stays: this one is a
-- superset used only where those two columns are read alongside the key.
CREATE INDEX IF NOT EXISTS idx_media_files_id_role_path
    ON media_files(id, role, file_path);
