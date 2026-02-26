CREATE TABLE IF NOT EXISTS mediarr_schema_migrations (
    id INTEGER PRIMARY KEY,
    migration_key TEXT NOT NULL UNIQUE,
    migration_checksum TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    success INTEGER NOT NULL,
    error_message TEXT,
    runtime_version TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mediarr_schema_migrations_success
    ON mediarr_schema_migrations (success, migration_key);
