-- Maintenance action execution (plan 137 Tracks D2/D3): per-rule effect
-- arming, bounded execution attempts on candidates, and the append-only
-- record of every action-handler attempt, holds included.

ALTER TABLE maintenance_rule_sets
    ADD COLUMN effect_arming TEXT NOT NULL DEFAULT 'none';

ALTER TABLE lifecycle_candidates
    ADD COLUMN action_attempts INTEGER NOT NULL DEFAULT 0;

CREATE TABLE lifecycle_action_runs (
    id TEXT PRIMARY KEY NOT NULL,
    candidate_id TEXT NOT NULL REFERENCES lifecycle_candidates(id) ON DELETE CASCADE,
    rule_set_id TEXT NOT NULL,
    revision_number INTEGER NOT NULL,
    title_id TEXT NOT NULL,
    action_kind TEXT NOT NULL,
    match_generation INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    status TEXT NOT NULL,
    hold_reason TEXT,
    error TEXT,
    detail TEXT NOT NULL DEFAULT '{}',
    started_at TEXT NOT NULL,
    finished_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (idempotency_key, attempt)
);

CREATE INDEX idx_lifecycle_action_runs_rule_set
    ON lifecycle_action_runs(rule_set_id, started_at DESC);

CREATE INDEX idx_lifecycle_action_runs_candidate
    ON lifecycle_action_runs(candidate_id);
