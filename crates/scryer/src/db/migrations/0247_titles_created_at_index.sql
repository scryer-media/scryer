-- Index the artwork-refresh drive order.
--
-- The background title-artwork loop picks its next candidates with
-- `ORDER BY t.created_at ASC LIMIT n` over `titles`, joined out to
-- `title_images` / `title_image_variants`. Nothing indexed `created_at`, so
-- every batch scanned `titles` whole and sorted the survivors in a temp
-- B-tree before taking its handful of rows. With the index SQLite walks
-- `created_at` order directly and stops at the limit.

CREATE INDEX IF NOT EXISTS idx_titles_created_at
    ON titles(created_at);
