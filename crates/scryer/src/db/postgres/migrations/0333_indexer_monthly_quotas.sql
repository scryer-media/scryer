ALTER TABLE indexers ADD COLUMN max_requests_per_month BIGINT;
ALTER TABLE indexers ADD COLUMN max_grabs_per_month BIGINT;

CREATE TABLE indexer_monthly_usage (
    indexer_id TEXT PRIMARY KEY NOT NULL,
    month_start TEXT NOT NULL,
    request_count BIGINT NOT NULL DEFAULT 0,
    grab_count BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
