CREATE TABLE indexer_errors (
    id TEXT PRIMARY KEY,
    indexer_id TEXT NOT NULL REFERENCES indexers(id) ON DELETE CASCADE,
    indexer_name TEXT NOT NULL,
    operation TEXT NOT NULL,
    http_status INTEGER NOT NULL,
    classification TEXT NOT NULL,
    provider_error_code INTEGER NULL,
    message TEXT NOT NULL,
    content_type TEXT NULL,
    payload_format_version INTEGER NOT NULL,
    response_zstd BLOB NOT NULL,
    occurred_at TEXT NOT NULL
);

CREATE INDEX idx_indexer_errors_indexer_occurred_at_id
    ON indexer_errors (indexer_id, occurred_at DESC, id DESC);

CREATE INDEX idx_indexer_errors_occurred_at
    ON indexer_errors (occurred_at);
