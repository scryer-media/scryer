-- Durable v2 maintenance action sequences: current per-step intent,
-- immutable attempt evidence, durable job receipts, and continuous-membership
-- terminal latches. Legacy action runs and candidates remain untouched.

CREATE TABLE maintenance_action_steps (
    candidate_id TEXT NOT NULL REFERENCES lifecycle_candidates(id) ON DELETE CASCADE,
    match_generation INTEGER NOT NULL,
    revision_number INTEGER NOT NULL,
    step_id TEXT NOT NULL,
    rule_set_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    sequence_content_hash TEXT NOT NULL,
    step_kind TEXT NOT NULL,
    intent_json TEXT NOT NULL DEFAULT '{}',
    before_state_json TEXT NOT NULL DEFAULT '{}',
    target_identity_json TEXT NOT NULL DEFAULT '{}',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    state TEXT NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 0,
    lease_id TEXT,
    lease_expires_at TEXT,
    hold_reason TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finished_at TEXT,
    PRIMARY KEY (candidate_id, match_generation, revision_number, step_id)
);

CREATE INDEX idx_maintenance_action_steps_candidate
    ON maintenance_action_steps(candidate_id, match_generation, revision_number, step_id);

CREATE TABLE maintenance_action_step_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    candidate_id TEXT NOT NULL,
    match_generation INTEGER NOT NULL,
    revision_number INTEGER NOT NULL,
    step_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    state TEXT NOT NULL,
    intent_json TEXT NOT NULL DEFAULT '{}',
    before_state_json TEXT NOT NULL DEFAULT '{}',
    target_identity_json TEXT NOT NULL DEFAULT '{}',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    hold_reason TEXT,
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (candidate_id, match_generation, revision_number, step_id)
        REFERENCES maintenance_action_steps(candidate_id, match_generation, revision_number, step_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_maintenance_action_step_attempts_step
    ON maintenance_action_step_attempts(candidate_id, match_generation, revision_number, step_id, attempt, started_at DESC);

CREATE TABLE maintenance_action_job_receipts (
    candidate_id TEXT NOT NULL,
    match_generation INTEGER NOT NULL,
    revision_number INTEGER NOT NULL,
    step_id TEXT NOT NULL,
    dispatch_attempt INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    logical_request_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    job_run_id TEXT,
    state TEXT NOT NULL,
    reconciliation_evidence_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
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
    id TEXT PRIMARY KEY NOT NULL,
    candidate_id TEXT NOT NULL UNIQUE REFERENCES lifecycle_candidates(id) ON DELETE CASCADE,
    rule_set_id TEXT NOT NULL,
    revision_number INTEGER NOT NULL,
    matcher_content_hash TEXT NOT NULL,
    title_id TEXT NOT NULL,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    match_generation INTEGER NOT NULL,
    sequence_content_hash TEXT NOT NULL,
    outcome TEXT NOT NULL,
    expected_step_ids_json TEXT NOT NULL DEFAULT '[]',
    completed_step_ids_json TEXT NOT NULL DEFAULT '[]',
    terminal_step_id TEXT,
    created_at TEXT NOT NULL,
    released_at TEXT
);

CREATE UNIQUE INDEX idx_maintenance_sequence_terminal_memberships_active
    ON maintenance_sequence_terminal_memberships(rule_set_id, revision_number, subject_kind, subject_id)
    WHERE released_at IS NULL;

CREATE INDEX idx_maintenance_sequence_terminal_memberships_active_page
    ON maintenance_sequence_terminal_memberships(rule_set_id, revision_number, id)
    WHERE released_at IS NULL;
