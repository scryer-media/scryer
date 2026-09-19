-- Index the artwork-refresh drive order. See the SQLite twin for which read
-- this serves and why the existing `titles` indexes do not answer it.
CREATE INDEX IF NOT EXISTS idx_titles_created_at
    ON titles(created_at);
