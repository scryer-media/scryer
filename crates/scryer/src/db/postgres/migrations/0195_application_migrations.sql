CREATE TABLE application_migrations (
    migration_id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL,
    execution_time_ms BIGINT NOT NULL
);
