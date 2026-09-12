CREATE TABLE application_compatibility_journal (
    migration_id TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    original_metadata TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'blocked', 'validated')),
    detail TEXT,
    PRIMARY KEY (migration_id, subject_id, source_digest)
);
