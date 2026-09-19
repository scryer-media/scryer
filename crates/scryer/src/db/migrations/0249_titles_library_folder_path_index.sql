-- Index the folder-ownership lookup.
--
-- Claiming a folder for a title asks whether another title in the same library
-- already owns it. That read used to list the whole library and sift it in
-- memory; it is now a narrowed
-- `WHERE library_id = ? AND id <> ? AND folder_path IN (...)`. Nothing indexed
-- `folder_path`, so without this the narrowed read still scans every title row
-- of the library once per folder a scan touches.

CREATE INDEX IF NOT EXISTS idx_titles_library_folder_path
    ON titles(library_id, folder_path);
