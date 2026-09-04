CREATE TABLE maintenance_rule_exclusions (
    id text PRIMARY KEY NOT NULL,
    rule_set_id text REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    title_id text NOT NULL,
    reason text NOT NULL DEFAULT '',
    created_by text,
    created_at timestamptz NOT NULL DEFAULT NOW()
);

-- Postgres, like SQLite, treats NULLs as distinct inside a UNIQUE constraint,
-- so a plain UNIQUE (rule_set_id, title_id) would let a title collect any
-- number of global exclusion rows. Two partial unique indexes express the
-- invariant the constraint cannot: exactly one global row per title, and
-- exactly one row per (rule, title).
CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_rule_title
    ON maintenance_rule_exclusions(rule_set_id, title_id)
    WHERE rule_set_id IS NOT NULL;

CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_global_title
    ON maintenance_rule_exclusions(title_id)
    WHERE rule_set_id IS NULL;

CREATE INDEX idx_maintenance_rule_exclusions_title
    ON maintenance_rule_exclusions(title_id);

CREATE TABLE maintenance_evaluation_runs (
    id text PRIMARY KEY NOT NULL,
    rule_set_id text NOT NULL REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    revision_number bigint NOT NULL,
    matcher_content_hash text NOT NULL DEFAULT '',
    started_at timestamptz NOT NULL DEFAULT NOW(),
    finished_at timestamptz,
    status text NOT NULL DEFAULT 'running',
    evaluated_count bigint NOT NULL DEFAULT 0,
    matched_count bigint NOT NULL DEFAULT 0,
    no_match_count bigint NOT NULL DEFAULT 0,
    unknown_count bigint NOT NULL DEFAULT 0,
    error_count bigint NOT NULL DEFAULT 0,
    canceled_candidates bigint NOT NULL DEFAULT 0,
    superseded_candidates bigint NOT NULL DEFAULT 0,
    duration_ms bigint,
    error text
);

CREATE INDEX idx_maintenance_evaluation_runs_rule_set
    ON maintenance_evaluation_runs(rule_set_id, started_at DESC);

CREATE TABLE lifecycle_candidates (
    id text PRIMARY KEY NOT NULL,
    rule_set_id text NOT NULL REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    revision_number bigint NOT NULL,
    matcher_content_hash text NOT NULL DEFAULT '',
    title_id text NOT NULL,
    library_id text NOT NULL DEFAULT '',
    facet text NOT NULL DEFAULT '',
    subject_kind text NOT NULL DEFAULT 'title',
    match_generation bigint NOT NULL DEFAULT 1,
    state text NOT NULL DEFAULT 'observing',
    state_reason text NOT NULL DEFAULT '',
    reason_codes text NOT NULL DEFAULT '[]',
    action_kind text NOT NULL DEFAULT 'do_nothing',
    grace_days bigint NOT NULL DEFAULT 0,
    first_matched_at timestamptz NOT NULL DEFAULT NOW(),
    last_matched_at timestamptz NOT NULL DEFAULT NOW(),
    due_at timestamptz NOT NULL DEFAULT NOW(),
    last_evaluated_at timestamptz NOT NULL DEFAULT NOW(),
    held_since timestamptz,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_lifecycle_candidates_rule_state
    ON lifecycle_candidates(rule_set_id, state);

CREATE INDEX idx_lifecycle_candidates_title
    ON lifecycle_candidates(title_id);

-- One rule set may hold at most one non-terminal candidate per title. The
-- terminal states are listed rather than negated through a column so the index
-- predicate stays immutable, which is what a partial index requires.
CREATE UNIQUE INDEX idx_lifecycle_candidates_active_subject
    ON lifecycle_candidates(rule_set_id, title_id)
    WHERE state NOT IN ('succeeded', 'failed', 'canceled', 'excluded');
