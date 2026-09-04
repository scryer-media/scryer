CREATE TABLE maintenance_rule_sets (
    id text PRIMARY KEY NOT NULL,
    name text NOT NULL,
    description text NOT NULL DEFAULT '',
    enabled boolean NOT NULL DEFAULT false,
    evaluation_mode text NOT NULL DEFAULT 'disabled',
    subject_kind text NOT NULL DEFAULT 'title',
    library_ids text NOT NULL DEFAULT '[]',
    current_revision_number bigint NOT NULL DEFAULT 1,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW()
);

CREATE TABLE maintenance_rule_revisions (
    id text PRIMARY KEY NOT NULL,
    rule_set_id text NOT NULL REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    revision_number bigint NOT NULL,
    rego_source text NOT NULL,
    action_spec text NOT NULL,
    grace_days bigint NOT NULL DEFAULT 0,
    matcher_content_hash text NOT NULL,
    created_by text,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    UNIQUE (rule_set_id, revision_number)
);

CREATE INDEX idx_maintenance_rule_revisions_rule_set
    ON maintenance_rule_revisions(rule_set_id, revision_number DESC);
