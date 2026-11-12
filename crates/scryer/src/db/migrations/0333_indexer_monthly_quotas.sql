ALTER TABLE indexers ADD COLUMN max_requests_per_month INTEGER;
ALTER TABLE indexers ADD COLUMN max_grabs_per_month INTEGER;

CREATE TABLE indexer_monthly_usage (
    indexer_id TEXT PRIMARY KEY NOT NULL,
    month_start TEXT NOT NULL,
    request_count INTEGER NOT NULL DEFAULT 0,
    grab_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
