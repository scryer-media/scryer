-- Import history lists the newest imports first across every status and
-- type; no existing index leads with the creation time.
CREATE INDEX IF NOT EXISTS idx_imports_created_at
    ON imports (created_at DESC);
