CREATE TABLE maintenance_rule_exclusions (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- SQLite treats NULLs as distinct inside a UNIQUE constraint, so a plain
-- UNIQUE (rule_set_id, title_id) would let a title collect any number of
-- global exclusion rows. Two partial unique indexes express the invariant the
-- constraint cannot: exactly one global row per title, and exactly one row per
-- (rule, title).
CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_rule_title
    ON maintenance_rule_exclusions(rule_set_id, title_id)
    WHERE rule_set_id IS NOT NULL;

CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_global_title
    ON maintenance_rule_exclusions(title_id)
    WHERE rule_set_id IS NULL;

CREATE INDEX idx_maintenance_rule_exclusions_title
    ON maintenance_rule_exclusions(title_id);

CREATE TABLE maintenance_evaluation_runs (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT NOT NULL REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    revision_number INTEGER NOT NULL,
    matcher_content_hash TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    finished_at TEXT,
    status TEXT NOT NULL DEFAULT 'running',
    evaluated_count INTEGER NOT NULL DEFAULT 0,
    matched_count INTEGER NOT NULL DEFAULT 0,
    no_match_count INTEGER NOT NULL DEFAULT 0,
    unknown_count INTEGER NOT NULL DEFAULT 0,
    error_count INTEGER NOT NULL DEFAULT 0,
    canceled_candidates INTEGER NOT NULL DEFAULT 0,
    superseded_candidates INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER,
    error TEXT
);

CREATE INDEX idx_maintenance_evaluation_runs_rule_set
    ON maintenance_evaluation_runs(rule_set_id, started_at DESC);

CREATE TABLE lifecycle_candidates (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT NOT NULL REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    revision_number INTEGER NOT NULL,
    matcher_content_hash TEXT NOT NULL DEFAULT '',
    title_id TEXT NOT NULL,
    library_id TEXT NOT NULL DEFAULT '',
    facet TEXT NOT NULL DEFAULT '',
    subject_kind TEXT NOT NULL DEFAULT 'title',
    match_generation INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT 'observing',
    state_reason TEXT NOT NULL DEFAULT '',
    reason_codes TEXT NOT NULL DEFAULT '[]',
    action_kind TEXT NOT NULL DEFAULT 'do_nothing',
    grace_days INTEGER NOT NULL DEFAULT 0,
    first_matched_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    last_matched_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    due_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    last_evaluated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    held_since TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_lifecycle_candidates_rule_state
    ON lifecycle_candidates(rule_set_id, state);

CREATE INDEX idx_lifecycle_candidates_title
    ON lifecycle_candidates(title_id);

-- One rule set may hold at most one non-terminal candidate per title. The
-- terminal states are listed rather than negated through a column so the index
-- predicate stays immutable, which is what SQLite requires of a partial index.
CREATE UNIQUE INDEX idx_lifecycle_candidates_active_subject
    ON lifecycle_candidates(rule_set_id, title_id)
    WHERE state NOT IN ('succeeded', 'failed', 'canceled', 'excluded');
