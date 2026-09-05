-- PostgreSQL twin of migrations/0219_request_rule_sets.sql.
-- Request rules (spec 0003 section 6): rule sets, their append-only matcher
-- revisions, and the durable trace of every evaluation.
CREATE TABLE request_rule_sets (
    id text PRIMARY KEY NOT NULL,
    name text NOT NULL,
    description text NOT NULL DEFAULT '',
    enabled boolean NOT NULL DEFAULT false,
    evaluation_mode text NOT NULL DEFAULT 'disabled',
    library_ids text NOT NULL DEFAULT '[]',
    current_revision_number bigint NOT NULL DEFAULT 1,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW()
);

CREATE TABLE request_rule_revisions (
    id text PRIMARY KEY NOT NULL,
    rule_set_id text NOT NULL REFERENCES request_rule_sets(id) ON DELETE CASCADE,
    revision_number bigint NOT NULL,
    rego_source text NOT NULL,
    matcher_content_hash text NOT NULL,
    created_by text,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    UNIQUE (rule_set_id, revision_number)
);

CREATE INDEX idx_request_rule_revisions_rule_set
    ON request_rule_revisions(rule_set_id, revision_number DESC);

-- `request_id` carries no foreign key to `media_requests`: pre-flight evaluates
-- a draft that has not been submitted (spec 0003 FR-020), so a decision can
-- precede -- or never acquire -- a request row, and FR-016 forbids dropping the
-- trace.
CREATE TABLE request_rule_decisions (
    id text PRIMARY KEY NOT NULL,
    request_id text NOT NULL,
    evaluated_at timestamptz NOT NULL DEFAULT NOW(),
    mode text NOT NULL DEFAULT 'disabled',
    effective_outcome text NOT NULL DEFAULT 'manual_review',
    policy_outcome text NOT NULL DEFAULT 'manual_review',
    fallback_reason text,
    votes_json text NOT NULL DEFAULT '[]',
    tags_json text NOT NULL DEFAULT '[]',
    input_hash text NOT NULL DEFAULT '',
    input_schema_version bigint NOT NULL DEFAULT 1,
    created_at timestamptz NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_request_rule_decisions_request
    ON request_rule_decisions(request_id, evaluated_at DESC);
