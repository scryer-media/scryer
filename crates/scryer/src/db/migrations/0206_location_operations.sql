-- Location-operation persistence (plan D5/D7; FR-030..033, FR-043, FR-084, FR-089, FR-092).
--
-- One operation model behind all six operation types (folder reassignment, root
-- move, root change, root consolidation, cross-library transfer, adoption), so
-- Activity, restart resume, and the concurrency guard are written once:
--
--   location_operations                    the operation itself
--   location_operation_title_checkpoints   the resumable per-title unit of work
--   location_operation_verifications       the per-file proof that was actually taken
--   location_operation_owned_entities      the (title, root) ownership registry
--
-- Titles, roots, and media files are referenced without foreign keys on purpose:
-- these rows are the durable account of what an operation did, and they must
-- outlive a catalog entity that is later deleted. FR-084 already stops an entity
-- being deleted while an operation owns it. The `actor` reference follows the
-- `workflow_operations` precedent (ON DELETE SET NULL).

CREATE TABLE location_operations (
    id TEXT PRIMARY KEY NOT NULL,
    -- folder_reassignment | root_move | root_change | root_consolidation
    -- | cross_library_transfer | adoption
    operation_type TEXT NOT NULL,
    -- move | adopt | catalog_only
    execution_mode TEXT NOT NULL,
    -- queued | planning | awaiting_confirmation | running | paused | canceling
    -- | canceled | completed | completed_with_warnings | failed
    state TEXT NOT NULL DEFAULT 'queued',
    initiated_by_user_id TEXT,
    source_library_id TEXT,
    source_root_id TEXT,
    destination_library_id TEXT,
    destination_root_id TEXT,
    -- Fingerprint of the confirmed plan (FR-081/FR-089). Empty until a plan is built.
    plan_fingerprint TEXT NOT NULL DEFAULT '',
    plan_json TEXT,
    -- full | quick, resolved from the user preference when the plan is confirmed (FR-042/043)
    verification_depth TEXT NOT NULL DEFAULT 'full',
    -- Set when any file fell back to the quick floor, so the weaker guarantee is auditable.
    verification_fallback_count INTEGER NOT NULL DEFAULT 0,
    title_total INTEGER NOT NULL DEFAULT 0,
    title_completed_count INTEGER NOT NULL DEFAULT 0,
    title_blocked_count INTEGER NOT NULL DEFAULT 0,
    file_total INTEGER NOT NULL DEFAULT 0,
    file_completed_count INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    bytes_completed INTEGER NOT NULL DEFAULT 0,
    -- The Activity/job row this operation runs under, when it has one.
    job_run_id TEXT,
    workflow_operation_id TEXT,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    cancel_requested_at TEXT,
    failure_reason TEXT,
    confirmed_at TEXT,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (initiated_by_user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX idx_location_operations_state
    ON location_operations(state, updated_at DESC);
CREATE INDEX idx_location_operations_source_root
    ON location_operations(source_root_id);
CREATE INDEX idx_location_operations_destination_root
    ON location_operations(destination_root_id);

-- The safe-cancel / resume checkpoint: one title at a time, committed only after
-- that title's destination content verified (FR-031, FR-092).
CREATE TABLE location_operation_title_checkpoints (
    operation_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    sequence INTEGER NOT NULL DEFAULT 0,
    -- pending | in_progress | copied | verified | committed | skipped | no_op
    -- | blocked | failed | canceled
    state TEXT NOT NULL DEFAULT 'pending',
    -- move | merge | dedup | catalog_only | no_op | blocked (preview classification)
    classification TEXT,
    source_library_id TEXT,
    source_root_id TEXT,
    source_folder_path TEXT,
    destination_library_id TEXT,
    destination_root_id TEXT,
    destination_folder_path TEXT,
    -- Set when this title merges into an existing destination title (plan D8).
    merged_into_title_id TEXT,
    file_total INTEGER NOT NULL DEFAULT 0,
    file_completed_count INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    bytes_completed INTEGER NOT NULL DEFAULT 0,
    blocked_reason TEXT,
    failure_reason TEXT,
    checkpointed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (operation_id, title_id),
    FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE
);

CREATE INDEX idx_location_operation_title_checkpoints_order
    ON location_operation_title_checkpoints(operation_id, sequence ASC);
CREATE INDEX idx_location_operation_title_checkpoints_state
    ON location_operation_title_checkpoints(operation_id, state);
CREATE INDEX idx_location_operation_title_checkpoints_title
    ON location_operation_title_checkpoints(title_id);

-- Per-file verification outcome (FR-043, FR-044; Key Entities "Verification record").
-- `applied_depth` is what actually ran, which is not always `requested_depth`:
-- full mode falls back to the quick floor when a cache-bypassed read-back cannot
-- run, and that fallback is recorded rather than silently downgraded.
CREATE TABLE location_operation_verifications (
    id TEXT PRIMARY KEY NOT NULL,
    operation_id TEXT NOT NULL,
    title_id TEXT,
    media_file_id TEXT,
    source_path TEXT NOT NULL,
    destination_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    -- full | quick
    requested_depth TEXT NOT NULL,
    -- full | quick
    applied_depth TEXT NOT NULL,
    fell_back INTEGER NOT NULL DEFAULT 0,
    fallback_reason TEXT,
    -- passed | failed | skipped
    outcome TEXT NOT NULL,
    move_crc TEXT,
    move_crc_algorithm TEXT,
    full_blake3 TEXT,
    sampled_signature_scheme TEXT,
    sampled_signature_value TEXT,
    failure_reason TEXT,
    verified_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE
);

-- One record per destination file per operation: a resume must not repeat work
-- that is already verified (FR-092).
CREATE UNIQUE INDEX idx_location_operation_verifications_destination
    ON location_operation_verifications(operation_id, destination_path);
CREATE INDEX idx_location_operation_verifications_title
    ON location_operation_verifications(operation_id, title_id);
CREATE INDEX idx_location_operation_verifications_media_file
    ON location_operation_verifications(media_file_id);

-- The concurrency registry (FR-084, plan D7). A row exists while an operation
-- owns an entity; `released_at` closes it. The partial unique index is what
-- makes a second operation's claim fail rather than interleave.
CREATE TABLE location_operation_owned_entities (
    operation_id TEXT NOT NULL,
    -- title | root | library
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    -- exclusive | shared
    ownership_mode TEXT NOT NULL DEFAULT 'exclusive',
    acquired_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    released_at TEXT,
    PRIMARY KEY (operation_id, entity_type, entity_id),
    FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_location_operation_owned_entities_active
    ON location_operation_owned_entities(entity_type, entity_id)
    WHERE released_at IS NULL;
CREATE INDEX idx_location_operation_owned_entities_operation
    ON location_operation_owned_entities(operation_id, released_at);
