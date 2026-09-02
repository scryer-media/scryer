-- Location-operation persistence (plan D5/D7; FR-030..033, FR-043, FR-084, FR-089, FR-092).
-- PostgreSQL half of 0206; see the SQLite file for the table-by-table rationale.
-- Column names and semantics are identical; only the types differ (timestamptz
-- for instants, boolean for the flags SQLite stores as INTEGER). That parity is
-- load-bearing: the store issues one statement per operation for both engines.
-- Amended before release alongside the SQLite half with the FR-091 outcome
-- counters (merge/dedup/rename/no_op/unresolved), the checkpoint `note`, and the
-- verification `detail`; see that file for what each column means.

CREATE TABLE location_operations (
    id text PRIMARY KEY NOT NULL,
    operation_type text NOT NULL,
    execution_mode text NOT NULL,
    state text NOT NULL DEFAULT 'queued',
    initiated_by_user_id text,
    source_library_id text,
    source_root_id text,
    destination_library_id text,
    destination_root_id text,
    plan_fingerprint text NOT NULL DEFAULT '',
    plan_json text,
    verification_depth text NOT NULL DEFAULT 'full',
    verification_fallback_count integer NOT NULL DEFAULT 0,
    title_total integer NOT NULL DEFAULT 0,
    title_completed_count integer NOT NULL DEFAULT 0,
    title_blocked_count integer NOT NULL DEFAULT 0,
    file_total integer NOT NULL DEFAULT 0,
    file_completed_count integer NOT NULL DEFAULT 0,
    bytes_total bigint NOT NULL DEFAULT 0,
    bytes_completed bigint NOT NULL DEFAULT 0,
    merge_count integer NOT NULL DEFAULT 0,
    dedup_count integer NOT NULL DEFAULT 0,
    rename_count integer NOT NULL DEFAULT 0,
    no_op_count integer NOT NULL DEFAULT 0,
    unresolved_count integer NOT NULL DEFAULT 0,
    job_run_id text,
    workflow_operation_id text,
    cancel_requested boolean NOT NULL DEFAULT false,
    cancel_requested_at timestamptz,
    failure_reason text,
    confirmed_at timestamptz,
    started_at timestamptz,
    completed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    CONSTRAINT location_operations_initiated_by_user_id_fkey
        FOREIGN KEY (initiated_by_user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX idx_location_operations_state
    ON location_operations(state, updated_at DESC);
CREATE INDEX idx_location_operations_source_root
    ON location_operations(source_root_id);
CREATE INDEX idx_location_operations_destination_root
    ON location_operations(destination_root_id);

CREATE TABLE location_operation_title_checkpoints (
    operation_id text NOT NULL,
    title_id text NOT NULL,
    sequence integer NOT NULL DEFAULT 0,
    state text NOT NULL DEFAULT 'pending',
    classification text,
    source_library_id text,
    source_root_id text,
    source_folder_path text,
    destination_library_id text,
    destination_root_id text,
    destination_folder_path text,
    merged_into_title_id text,
    file_total integer NOT NULL DEFAULT 0,
    file_completed_count integer NOT NULL DEFAULT 0,
    bytes_total bigint NOT NULL DEFAULT 0,
    bytes_completed bigint NOT NULL DEFAULT 0,
    blocked_reason text,
    failure_reason text,
    note text,
    checkpointed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    PRIMARY KEY (operation_id, title_id),
    CONSTRAINT location_operation_title_checkpoints_operation_id_fkey
        FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE
);

CREATE INDEX idx_location_operation_title_checkpoints_order
    ON location_operation_title_checkpoints(operation_id, sequence ASC);
CREATE INDEX idx_location_operation_title_checkpoints_state
    ON location_operation_title_checkpoints(operation_id, state);
CREATE INDEX idx_location_operation_title_checkpoints_title
    ON location_operation_title_checkpoints(title_id);

CREATE TABLE location_operation_verifications (
    id text PRIMARY KEY NOT NULL,
    operation_id text NOT NULL,
    title_id text,
    media_file_id text,
    source_path text NOT NULL,
    destination_path text NOT NULL,
    size_bytes bigint NOT NULL DEFAULT 0,
    requested_depth text NOT NULL,
    applied_depth text NOT NULL,
    fell_back boolean NOT NULL DEFAULT false,
    fallback_reason text,
    outcome text NOT NULL,
    move_crc text,
    move_crc_algorithm text,
    full_blake3 text,
    sampled_signature_scheme text,
    sampled_signature_value text,
    failure_reason text,
    detail text,
    verified_at timestamptz NOT NULL DEFAULT NOW(),
    created_at timestamptz NOT NULL DEFAULT NOW(),
    CONSTRAINT location_operation_verifications_operation_id_fkey
        FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_location_operation_verifications_destination
    ON location_operation_verifications(operation_id, destination_path);
CREATE INDEX idx_location_operation_verifications_title
    ON location_operation_verifications(operation_id, title_id);
CREATE INDEX idx_location_operation_verifications_media_file
    ON location_operation_verifications(media_file_id);

CREATE TABLE location_operation_owned_entities (
    operation_id text NOT NULL,
    entity_type text NOT NULL,
    entity_id text NOT NULL,
    ownership_mode text NOT NULL DEFAULT 'exclusive',
    acquired_at timestamptz NOT NULL DEFAULT NOW(),
    released_at timestamptz,
    PRIMARY KEY (operation_id, entity_type, entity_id),
    CONSTRAINT location_operation_owned_entities_operation_id_fkey
        FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_location_operation_owned_entities_active
    ON location_operation_owned_entities(entity_type, entity_id)
    WHERE released_at IS NULL;
CREATE INDEX idx_location_operation_owned_entities_operation
    ON location_operation_owned_entities(operation_id, released_at);
