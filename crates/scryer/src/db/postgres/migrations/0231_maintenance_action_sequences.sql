-- Durable v2 maintenance action sequences: current per-step intent,
-- immutable attempt evidence, durable job receipts, and continuous-membership
-- terminal latches. Legacy action runs and candidates remain untouched.

CREATE TABLE maintenance_action_steps (
    candidate_id text NOT NULL REFERENCES lifecycle_candidates(id) ON DELETE CASCADE,
    match_generation bigint NOT NULL,
    revision_number bigint NOT NULL,
    step_id text NOT NULL,
    rule_set_id text NOT NULL,
    title_id text NOT NULL,
    subject_kind text NOT NULL,
    subject_id text NOT NULL,
    sequence_content_hash text NOT NULL,
    step_kind text NOT NULL,
    intent_json text NOT NULL DEFAULT '{}',
    before_state_json text NOT NULL DEFAULT '{}',
    target_identity_json text NOT NULL DEFAULT '{}',
    provenance_json text NOT NULL DEFAULT '{}',
    state text NOT NULL,
    attempt bigint NOT NULL DEFAULT 0,
    lease_id text,
    lease_expires_at timestamptz,
    hold_reason text,
    error text,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    finished_at timestamptz,
    PRIMARY KEY (candidate_id, match_generation, revision_number, step_id)
);

CREATE INDEX idx_maintenance_action_steps_candidate
    ON maintenance_action_steps(candidate_id, match_generation, revision_number, step_id);

CREATE TABLE maintenance_action_step_attempts (
    id text PRIMARY KEY NOT NULL,
    candidate_id text NOT NULL,
    match_generation bigint NOT NULL,
    revision_number bigint NOT NULL,
    step_id text NOT NULL,
    attempt bigint NOT NULL,
    state text NOT NULL,
    intent_json text NOT NULL DEFAULT '{}',
    before_state_json text NOT NULL DEFAULT '{}',
    target_identity_json text NOT NULL DEFAULT '{}',
    provenance_json text NOT NULL DEFAULT '{}',
    hold_reason text,
    error text,
    started_at timestamptz NOT NULL,
    finished_at timestamptz,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    FOREIGN KEY (candidate_id, match_generation, revision_number, step_id)
        REFERENCES maintenance_action_steps(candidate_id, match_generation, revision_number, step_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_maintenance_action_step_attempts_step
    ON maintenance_action_step_attempts(candidate_id, match_generation, revision_number, step_id, attempt, started_at DESC);

CREATE TABLE maintenance_action_job_receipts (
    candidate_id text NOT NULL,
    match_generation bigint NOT NULL,
    revision_number bigint NOT NULL,
    step_id text NOT NULL,
    dispatch_attempt bigint NOT NULL,
    schema_version integer NOT NULL,
    logical_request_key text NOT NULL,
    request_hash text NOT NULL,
    job_run_id text,
    state text NOT NULL,
    reconciliation_evidence_json text NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (candidate_id, match_generation, revision_number, step_id, dispatch_attempt),
    FOREIGN KEY (candidate_id, match_generation, revision_number, step_id)
        REFERENCES maintenance_action_steps(candidate_id, match_generation, revision_number, step_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_maintenance_action_job_receipts_step
    ON maintenance_action_job_receipts(candidate_id, match_generation, revision_number, step_id, dispatch_attempt DESC);

CREATE UNIQUE INDEX idx_maintenance_action_job_receipts_accepted_request
    ON maintenance_action_job_receipts(candidate_id, match_generation, revision_number, step_id, request_hash)
    WHERE state IN ('accepted', 'completed');

CREATE TABLE maintenance_sequence_terminal_memberships (
    id text PRIMARY KEY NOT NULL,
    candidate_id text NOT NULL UNIQUE REFERENCES lifecycle_candidates(id) ON DELETE CASCADE,
    rule_set_id text NOT NULL,
    revision_number bigint NOT NULL,
    matcher_content_hash text NOT NULL,
    title_id text NOT NULL,
    subject_kind text NOT NULL,
    subject_id text NOT NULL,
    match_generation bigint NOT NULL,
    sequence_content_hash text NOT NULL,
    outcome text NOT NULL,
    expected_step_ids_json text NOT NULL DEFAULT '[]',
    completed_step_ids_json text NOT NULL DEFAULT '[]',
    terminal_step_id text,
    created_at timestamptz NOT NULL,
    released_at timestamptz
);

CREATE UNIQUE INDEX idx_maintenance_sequence_terminal_memberships_active
    ON maintenance_sequence_terminal_memberships(rule_set_id, revision_number, subject_kind, subject_id)
    WHERE released_at IS NULL;

CREATE INDEX idx_maintenance_sequence_terminal_memberships_active_page
    ON maintenance_sequence_terminal_memberships(rule_set_id, revision_number, id)
    WHERE released_at IS NULL;
