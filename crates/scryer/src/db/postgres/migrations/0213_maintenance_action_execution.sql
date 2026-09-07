-- Maintenance action execution (plan 137 Tracks D2/D3): per-rule effect
-- arming, bounded execution attempts on candidates, and the append-only
-- record of every action-handler attempt, holds included.

ALTER TABLE maintenance_rule_sets
    ADD COLUMN effect_arming text NOT NULL DEFAULT 'none';

ALTER TABLE lifecycle_candidates
    ADD COLUMN action_attempts bigint NOT NULL DEFAULT 0;

CREATE TABLE lifecycle_action_runs (
    id text PRIMARY KEY NOT NULL,
    candidate_id text NOT NULL REFERENCES lifecycle_candidates(id) ON DELETE CASCADE,
    rule_set_id text NOT NULL,
    revision_number bigint NOT NULL,
    title_id text NOT NULL,
    action_kind text NOT NULL,
    match_generation bigint NOT NULL,
    idempotency_key text NOT NULL,
    attempt bigint NOT NULL,
    status text NOT NULL,
    hold_reason text,
    error text,
    detail text NOT NULL DEFAULT '{}',
    started_at timestamptz NOT NULL,
    finished_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    UNIQUE (idempotency_key, attempt)
);

CREATE INDEX idx_lifecycle_action_runs_rule_set
    ON lifecycle_action_runs(rule_set_id, started_at DESC);

CREATE INDEX idx_lifecycle_action_runs_candidate
    ON lifecycle_action_runs(candidate_id);
