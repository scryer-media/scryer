-- Optional per-indexer limits on what this Scryer instance sends in a UTC
-- calendar month. NULL means no limit.
ALTER TABLE indexers ADD COLUMN max_requests_per_month INTEGER;
ALTER TABLE indexers ADD COLUMN max_grabs_per_month INTEGER;

-- Usage counted against those limits: at most one row per indexer, rewritten
-- in place when a new UTC month starts (`month_start` is `YYYY-MM-01`). A row
-- exists only for an indexer that has had a limit configured, and is removed
-- in the same transaction that deletes its indexer.
CREATE TABLE indexer_monthly_usage (
    indexer_id TEXT PRIMARY KEY NOT NULL,
    month_start TEXT NOT NULL,
    request_count INTEGER NOT NULL DEFAULT 0,
    grab_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
