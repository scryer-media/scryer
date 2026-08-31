CREATE TEMP TABLE _0180_download_submission_episode_links AS
SELECT
    download_client_id,
    download_client_type,
    download_client_item_id,
    episode_id
FROM download_submission_episode_links;

CREATE TEMP TABLE _0180_download_import_artifacts AS
SELECT
    id,
    source_system,
    source_ref,
    import_id,
    relative_path,
    normalized_file_name,
    media_kind,
    title_id,
    episode_id,
    season_number,
    episode_number,
    result,
    reason_code,
    imported_media_file_id,
    created_at,
    source_client_id,
    canonical_download_id
FROM download_import_artifacts;

DROP TABLE download_submission_episode_links;
DROP TABLE download_import_artifacts;

CREATE TABLE download_submissions_0180 (
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    download_client_id TEXT NOT NULL DEFAULT '',
    download_client_type TEXT NOT NULL,
    download_client_item_id TEXT,
    source_title TEXT,
    submitted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    collection_id TEXT,
    tracked_state TEXT,
    tracked_state_at TEXT,
    source_hint TEXT,
    source_kind TEXT,
    request_signature TEXT,
    episode_id TEXT,
    download_id TEXT,
    purpose TEXT NOT NULL DEFAULT 'standard',
    series_movie_link_id TEXT,
    actor_kind TEXT,
    actor_user_id TEXT,
    actor_display_name TEXT,
    source_provider_id TEXT,
    source_provider_name TEXT,
    seeding_profile_id TEXT,
    seed_goal_ratio REAL,
    seed_goal_seconds INTEGER,
    seed_never_remove INTEGER,
    seed_goal_met_action TEXT,
    seed_goal_source TEXT,
    seed_info_hash TEXT,
    seed_post_import_tracking TEXT,
    release_size_bytes INTEGER,
    FOREIGN KEY (id) REFERENCES downloads(id)
);

INSERT INTO download_submissions_0180 (
    id,
    title_id,
    facet,
    download_client_id,
    download_client_type,
    download_client_item_id,
    source_title,
    submitted_at,
    collection_id,
    tracked_state,
    tracked_state_at,
    source_hint,
    source_kind,
    request_signature,
    episode_id,
    download_id,
    purpose,
    series_movie_link_id,
    actor_kind,
    actor_user_id,
    actor_display_name,
    source_provider_id,
    source_provider_name,
    seeding_profile_id,
    seed_goal_ratio,
    seed_goal_seconds,
    seed_never_remove,
    seed_goal_met_action,
    seed_goal_source,
    seed_info_hash,
    seed_post_import_tracking,
    release_size_bytes
)
SELECT
    id,
    title_id,
    facet,
    download_client_id,
    download_client_type,
    download_client_item_id,
    source_title,
    submitted_at,
    collection_id,
    tracked_state,
    tracked_state_at,
    source_hint,
    source_kind,
    request_signature,
    episode_id,
    download_id,
    purpose,
    series_movie_link_id,
    actor_kind,
    actor_user_id,
    actor_display_name,
    source_provider_id,
    source_provider_name,
    seeding_profile_id,
    seed_goal_ratio,
    seed_goal_seconds,
    seed_never_remove,
    seed_goal_met_action,
    seed_goal_source,
    seed_info_hash,
    seed_post_import_tracking,
    release_size_bytes
FROM download_submissions;

DROP TABLE download_submissions;
ALTER TABLE download_submissions_0180 RENAME TO download_submissions;

CREATE INDEX idx_download_submissions_title_request_signature
    ON download_submissions(title_id, request_signature);
CREATE INDEX idx_download_submissions_download_id
    ON download_submissions(download_client_id, download_client_type, download_id);
CREATE INDEX idx_download_submissions_seed_info_hash
    ON download_submissions(seed_info_hash);

CREATE TABLE download_submission_episode_links (
    download_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    PRIMARY KEY (download_id, episode_id),
    FOREIGN KEY (download_id) REFERENCES download_submissions(id) ON DELETE CASCADE
);

INSERT INTO download_submission_episode_links (download_id, episode_id)
SELECT submissions.id, links.episode_id
FROM _0180_download_submission_episode_links links
JOIN download_submissions submissions
    ON submissions.download_client_id = links.download_client_id
   AND submissions.download_client_type = links.download_client_type
   AND submissions.download_client_item_id = links.download_client_item_id;

DROP TABLE _0180_download_submission_episode_links;

CREATE INDEX idx_download_submission_episode_links_episode
    ON download_submission_episode_links(episode_id);

CREATE TABLE download_identity_states_0179 (
    id TEXT PRIMARY KEY,
    identity_key TEXT NOT NULL UNIQUE,
    canonical_download_id TEXT NOT NULL,
    download_id TEXT,
    client_id TEXT,
    client_type TEXT,
    download_client_item_id TEXT,
    tracked_state TEXT NOT NULL,
    reason TEXT,
    detail TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (download_id IS NOT NULL)
);

INSERT INTO download_identity_states_0179 (
    id,
    identity_key,
    canonical_download_id,
    download_id,
    client_id,
    client_type,
    download_client_item_id,
    tracked_state,
    reason,
    detail,
    created_at,
    updated_at
)
SELECT
    id,
    identity_key,
    canonical_download_id,
    download_id,
    client_id,
    client_type,
    download_client_item_id,
    tracked_state,
    reason,
    detail,
    created_at,
    updated_at
FROM download_identity_states;

DROP TABLE download_identity_states;
ALTER TABLE download_identity_states_0179 RENAME TO download_identity_states;

CREATE INDEX idx_download_identity_states_download_id
    ON download_identity_states(client_id, client_type, download_id);

CREATE TABLE imports_0179 (
    id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    import_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    payload_json TEXT NOT NULL,
    result_json TEXT,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    rename_plan_json TEXT,
    source_client_id TEXT,
    download_id TEXT,
    import_transfer_phase TEXT,
    import_transfer_bytes INTEGER,
    import_transfer_total_bytes INTEGER,
    import_transfer_started_at TEXT,
    import_transfer_updated_at TEXT,
    canonical_download_id TEXT,
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id)
);

INSERT INTO imports_0179 (
    id,
    source_system,
    source_ref,
    import_type,
    status,
    payload_json,
    result_json,
    started_at,
    finished_at,
    created_at,
    updated_at,
    rename_plan_json,
    source_client_id,
    download_id,
    import_transfer_phase,
    import_transfer_bytes,
    import_transfer_total_bytes,
    import_transfer_started_at,
    import_transfer_updated_at,
    canonical_download_id
)
SELECT
    id,
    source_system,
    source_ref,
    import_type,
    status,
    payload_json,
    result_json,
    started_at,
    finished_at,
    created_at,
    updated_at,
    rename_plan_json,
    source_client_id,
    download_id,
    import_transfer_phase,
    import_transfer_bytes,
    import_transfer_total_bytes,
    import_transfer_started_at,
    import_transfer_updated_at,
    canonical_download_id
FROM imports;

DROP TABLE imports;
ALTER TABLE imports_0179 RENAME TO imports;

CREATE UNIQUE INDEX idx_imports_active_download_id
    ON imports (COALESCE(source_client_id, ''), source_system, download_id)
    WHERE download_id IS NOT NULL
      AND status IN ('pending', 'running', 'processing');
CREATE INDEX idx_imports_download_id
    ON imports(source_client_id, source_system, download_id);
CREATE UNIQUE INDEX idx_imports_source_ref
    ON imports (COALESCE(source_client_id, ''), source_system, source_ref, import_type)
    WHERE download_id IS NULL;
CREATE INDEX idx_imports_status_updated_at
    ON imports (status, updated_at);

CREATE TABLE download_import_artifacts (
    id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    import_id TEXT,
    relative_path TEXT,
    normalized_file_name TEXT NOT NULL,
    media_kind TEXT NOT NULL,
    title_id TEXT,
    episode_id TEXT,
    season_number INTEGER,
    episode_number INTEGER,
    result TEXT NOT NULL,
    reason_code TEXT,
    imported_media_file_id TEXT,
    created_at TEXT NOT NULL,
    source_client_id TEXT,
    canonical_download_id TEXT,
    FOREIGN KEY (import_id) REFERENCES imports(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL,
    FOREIGN KEY (imported_media_file_id) REFERENCES media_files(id) ON DELETE SET NULL,
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id)
);

INSERT INTO download_import_artifacts (
    id,
    source_system,
    source_ref,
    import_id,
    relative_path,
    normalized_file_name,
    media_kind,
    title_id,
    episode_id,
    season_number,
    episode_number,
    result,
    reason_code,
    imported_media_file_id,
    created_at,
    source_client_id,
    canonical_download_id
)
SELECT
    id,
    source_system,
    source_ref,
    import_id,
    relative_path,
    normalized_file_name,
    media_kind,
    title_id,
    episode_id,
    season_number,
    episode_number,
    result,
    reason_code,
    imported_media_file_id,
    created_at,
    source_client_id,
    canonical_download_id
FROM _0180_download_import_artifacts;

DROP TABLE _0180_download_import_artifacts;

CREATE INDEX idx_download_import_artifacts_episode
    ON download_import_artifacts (episode_id, result);
CREATE INDEX idx_download_import_artifacts_retention
    ON download_import_artifacts (created_at, import_id);
CREATE INDEX idx_download_import_artifacts_source
    ON download_import_artifacts (COALESCE(source_client_id, ''), source_system, source_ref, created_at);

CREATE TABLE download_queue_commands_0179 (
    id TEXT PRIMARY KEY,
    action TEXT NOT NULL,
    client_type TEXT NOT NULL,
    download_client_item_id TEXT NOT NULL,
    is_history INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    error_text TEXT,
    requested_by_user_id TEXT,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    client_id TEXT,
    canonical_download_id TEXT,
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id)
);

INSERT INTO download_queue_commands_0179 (
    id,
    action,
    client_type,
    download_client_item_id,
    is_history,
    status,
    error_text,
    requested_by_user_id,
    started_at,
    finished_at,
    created_at,
    updated_at,
    client_id,
    canonical_download_id
)
SELECT
    id,
    action,
    client_type,
    download_client_item_id,
    is_history,
    status,
    error_text,
    requested_by_user_id,
    started_at,
    finished_at,
    created_at,
    updated_at,
    client_id,
    canonical_download_id
FROM download_queue_commands;

DROP TABLE download_queue_commands;
ALTER TABLE download_queue_commands_0179 RENAME TO download_queue_commands;

CREATE UNIQUE INDEX idx_download_queue_commands_active_unique
    ON download_queue_commands(action, COALESCE(client_id, ''), client_type, download_client_item_id, is_history)
    WHERE status IN ('queued', 'running');
CREATE INDEX idx_download_queue_commands_source
    ON download_queue_commands(COALESCE(client_id, ''), client_type, download_client_item_id, is_history, created_at DESC);
CREATE INDEX idx_download_queue_commands_status
    ON download_queue_commands(action, status, updated_at);

CREATE UNIQUE INDEX idx_download_client_bindings_active_locator_unique
    ON download_client_bindings(client_config_id, client_type_snapshot, native_item_id)
    WHERE native_item_id IS NOT NULL
      AND ended_at IS NULL;
