CREATE TABLE IF NOT EXISTS indexer_api_quotas (
    indexer_id TEXT PRIMARY KEY NOT NULL,
    api_current INTEGER,
    api_max INTEGER,
    grab_current INTEGER,
    grab_max INTEGER,
    queries_today INTEGER NOT NULL DEFAULT 0,
    last_query_at TEXT,
    last_reset_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
