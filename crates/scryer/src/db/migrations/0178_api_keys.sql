CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    lookup_id TEXT NOT NULL UNIQUE,
    secret_hash TEXT NOT NULL,
    label TEXT NOT NULL,
    expires_at TEXT,
    revoked_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL,
    provisioning_source TEXT NOT NULL,
    CHECK (provisioning_source IN ('user', 'environment'))
);

CREATE INDEX IF NOT EXISTS idx_api_keys_user_created
    ON api_keys(user_id, created_at DESC);
