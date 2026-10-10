CREATE TABLE canonical_tag_vocabulary_cache (
    id TEXT PRIMARY KEY NOT NULL CHECK (id = 'canonical'),
    version TEXT NOT NULL,
    payload_json JSONB NOT NULL,
    checked_at TIMESTAMPTZ NOT NULL,
    jitter_seconds BIGINT NOT NULL CHECK (jitter_seconds BETWEEN 0 AND 21600)
);
