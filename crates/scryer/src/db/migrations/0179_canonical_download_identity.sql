CREATE TABLE downloads (
    id TEXT PRIMARY KEY,
    origin TEXT NOT NULL CHECK (origin IN ('scryer_submission', 'foreign_observation')),
    created_at TEXT NOT NULL,
    first_observed_at TEXT,
    last_observed_at TEXT,
    terminal_at TEXT
);

CREATE TABLE download_client_bindings (
    download_id TEXT PRIMARY KEY,
    client_config_id TEXT,
    client_type_snapshot TEXT,
    client_name_snapshot TEXT,
    native_item_id TEXT,
    created_at TEXT NOT NULL,
    last_seen_at TEXT,
    ended_at TEXT,
    FOREIGN KEY (download_id) REFERENCES downloads(id)
);

CREATE INDEX idx_download_client_bindings_locator
    ON download_client_bindings(client_config_id, client_type_snapshot, native_item_id);

CREATE TEMP TABLE _0179_download_submission_episode_links AS
SELECT
    download_client_id,
    download_client_type,
    download_client_item_id,
    episode_id
FROM download_submission_episode_links;

DROP TABLE download_submission_episode_links;

CREATE TABLE download_submissions_0179 (
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
    UNIQUE(download_client_id, download_client_type, download_client_item_id)
);

INSERT INTO download_submissions_0179 (
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
ALTER TABLE download_submissions_0179 RENAME TO download_submissions;

CREATE INDEX idx_download_submissions_title_request_signature
    ON download_submissions(title_id, request_signature);
CREATE INDEX idx_download_submissions_download_id
    ON download_submissions(download_client_id, download_client_type, download_id);
CREATE INDEX idx_download_submissions_seed_info_hash
    ON download_submissions(seed_info_hash);

CREATE TABLE download_submission_episode_links (
    download_client_id TEXT NOT NULL DEFAULT '',
    download_client_type TEXT NOT NULL,
    download_client_item_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    PRIMARY KEY (
        download_client_id,
        download_client_type,
        download_client_item_id,
        episode_id
    ),
    FOREIGN KEY (download_client_id, download_client_type, download_client_item_id)
        REFERENCES download_submissions(download_client_id, download_client_type, download_client_item_id)
        ON DELETE CASCADE
);

INSERT INTO download_submission_episode_links (
    download_client_id,
    download_client_type,
    download_client_item_id,
    episode_id
)
SELECT
    download_client_id,
    download_client_type,
    download_client_item_id,
    episode_id
FROM _0179_download_submission_episode_links;

DROP TABLE _0179_download_submission_episode_links;

CREATE INDEX idx_download_submission_episode_links_episode
    ON download_submission_episode_links(episode_id);

ALTER TABLE download_identity_states ADD COLUMN canonical_download_id TEXT;
ALTER TABLE imports ADD COLUMN canonical_download_id TEXT;
ALTER TABLE download_import_artifacts ADD COLUMN canonical_download_id TEXT;
ALTER TABLE download_queue_commands ADD COLUMN canonical_download_id TEXT;
