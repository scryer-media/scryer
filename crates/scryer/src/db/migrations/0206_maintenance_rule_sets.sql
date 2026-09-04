CREATE TABLE maintenance_rule_sets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 0,
    evaluation_mode TEXT NOT NULL DEFAULT 'disabled',
    subject_kind TEXT NOT NULL DEFAULT 'title',
    library_ids TEXT NOT NULL DEFAULT '[]',
    current_revision_number INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE maintenance_rule_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT NOT NULL REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    revision_number INTEGER NOT NULL,
    rego_source TEXT NOT NULL,
    action_spec TEXT NOT NULL,
    grace_days INTEGER NOT NULL DEFAULT 0,
    matcher_content_hash TEXT NOT NULL,
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (rule_set_id, revision_number)
);

CREATE INDEX idx_maintenance_rule_revisions_rule_set
    ON maintenance_rule_revisions(rule_set_id, revision_number DESC);
