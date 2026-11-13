CREATE TABLE api_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    lookup_id TEXT NOT NULL UNIQUE,
    secret_hash TEXT NOT NULL,
    label TEXT NOT NULL,
    expires_at TEXT,
    revoked_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL,
    provisioning_source TEXT NOT NULL,
    CHECK (provisioning_source IN ('user', 'environment'))
);
CREATE TABLE application_compatibility_journal (
    migration_id TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    original_metadata TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'blocked', 'validated')),
    detail TEXT,
    PRIMARY KEY (migration_id, subject_id, source_digest)
);
CREATE TABLE application_migrations (
    migration_id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    execution_time_ms INTEGER NOT NULL
);
CREATE TABLE blocklist (
    id           TEXT PRIMARY KEY,
    title_id     TEXT NOT NULL,
    release_name TEXT,
    reason       TEXT,
    created_at   TEXT NOT NULL, normalized_release_name TEXT NOT NULL DEFAULT '', indexer_id TEXT NOT NULL DEFAULT '', info_hash TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE canonical_tag_vocabulary_cache (
    id TEXT PRIMARY KEY NOT NULL CHECK (id = 'canonical'),
    version TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    jitter_seconds INTEGER NOT NULL CHECK (jitter_seconds BETWEEN 0 AND 21600)
);
CREATE TABLE collection_external_ids(
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    collection_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    provenance TEXT NOT NULL,
    source_scope TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE
);
CREATE TABLE collections(
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    collection_type TEXT NOT NULL,
    collection_index TEXT NOT NULL,
    label TEXT,
    ordered_path TEXT,
    first_episode_number TEXT,
    last_episode_number TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT, monitored INTEGER NOT NULL DEFAULT 1, narrative_order TEXT, interstitial_tvdb_id TEXT, interstitial_name TEXT, interstitial_slug TEXT, interstitial_year INTEGER, interstitial_content_status TEXT, interstitial_overview TEXT, interstitial_poster_url TEXT, interstitial_language TEXT, interstitial_runtime_minutes INTEGER, interstitial_sort_title TEXT, interstitial_imdb_id TEXT, interstitial_genres_json TEXT, interstitial_studio TEXT, interstitial_digital_release_date TEXT, interstitial_association_confidence TEXT, interstitial_continuity_status TEXT, interstitial_movie_form TEXT, interstitial_confidence TEXT, interstitial_signal_summary TEXT, special_movies_json TEXT NOT NULL DEFAULT '[]', interstitial_placement TEXT, interstitial_movie_tmdb_id TEXT, interstitial_movie_mal_id TEXT, interstitial_season_episode TEXT, interstitial_movie_anidb_id TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE delay_profile_facets (
    profile_id TEXT NOT NULL REFERENCES delay_profiles(id) ON DELETE CASCADE,
    facet TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (profile_id, facet)
);
CREATE TABLE delay_profile_libraries (
    profile_id TEXT NOT NULL REFERENCES delay_profiles(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (profile_id, library_id)
);
CREATE TABLE delay_profile_tags (
    profile_id TEXT NOT NULL REFERENCES delay_profiles(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (profile_id, label)
);
CREATE TABLE delay_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    usenet_delay_minutes INTEGER NOT NULL DEFAULT 0,
    torrent_delay_minutes INTEGER NOT NULL DEFAULT 0,
    preferred_protocol TEXT NOT NULL DEFAULT 'usenet',
    min_age_minutes INTEGER NOT NULL DEFAULT 0,
    bypass_score_threshold INTEGER,
    enable_usenet INTEGER NOT NULL DEFAULT 1,
    enable_torrent INTEGER NOT NULL DEFAULT 1,
    bypass_if_highest_quality INTEGER NOT NULL DEFAULT 0,
    priority INTEGER NOT NULL DEFAULT 0,
    position INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE discovery_affinity_revision (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    revision TEXT NOT NULL
);
CREATE TABLE discovery_facets (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    facet_name TEXT NOT NULL,
    facet_value TEXT NOT NULL,
    smg_count INTEGER,
    local_count INTEGER,
    PRIMARY KEY (run_id, facet_name, facet_value)
);
CREATE TABLE discovery_item_library_provenance (
    item_id TEXT NOT NULL REFERENCES discovery_items(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    subject_key TEXT NOT NULL,
    title_id TEXT NOT NULL DEFAULT '',
    library_id TEXT NOT NULL DEFAULT '',
    UNIQUE (item_id, subject_key, title_id, library_id)
);
CREATE TABLE discovery_item_rank_components (
    item_id TEXT NOT NULL REFERENCES discovery_items(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    component_index INTEGER NOT NULL,
    component_name TEXT NOT NULL DEFAULT '',
    component_value TEXT NOT NULL DEFAULT '',
    UNIQUE (item_id, component_index)
);
CREATE TABLE discovery_item_subject_links (
    item_id TEXT NOT NULL REFERENCES discovery_items(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    link_type TEXT NOT NULL,
    subject_key TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (item_id, link_type, subject_key)
);
CREATE TABLE discovery_items (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    base_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    source_run_kind TEXT NOT NULL,
    section_id TEXT,
    sort_index INTEGER NOT NULL DEFAULT 0,
    best_source TEXT,
    source_count INTEGER,
    edge_count INTEGER,
    relation_count INTEGER,
    source_subject_count INTEGER,
    rank_score REAL,
    matched_subject_count INTEGER NOT NULL DEFAULT 0,
    owned_in_input INTEGER NOT NULL DEFAULT 0,
    tombstoned_by_run_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    tombstoned_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
, recommendation_score REAL, base_rank REAL);
CREATE TABLE discovery_library_graph_payloads (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES discovery_library_graph_runs(id) ON DELETE CASCADE,
    payload BLOB NOT NULL
);
CREATE TABLE discovery_library_graph_runs (
    id TEXT PRIMARY KEY NOT NULL,
    facet TEXT NOT NULL,
    axis TEXT NOT NULL,
    status TEXT NOT NULL,
    subject_count INTEGER NOT NULL DEFAULT 0,
    submitted_count INTEGER NOT NULL DEFAULT 0,
    generated_at TEXT,
    error_code TEXT,
    error_text TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
, retry_after_seconds INTEGER, omitted_subject_keys_json TEXT NOT NULL DEFAULT '[]');
CREATE TABLE discovery_neighbor_cache (
    target_key TEXT NOT NULL,
    language TEXT NOT NULL,
    payload_version INTEGER NOT NULL,
    payload_blob BLOB NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (target_key, language)
);
CREATE TABLE discovery_pending_context_changes (
    id TEXT PRIMARY KEY NOT NULL,
    scope_key TEXT NOT NULL DEFAULT 'default',
    subject_key TEXT,
    previous_subject_key TEXT,
    change_type TEXT NOT NULL,
    title_id TEXT REFERENCES titles(id) ON DELETE SET NULL,
    previous_title_id TEXT,
    library_facet TEXT,
    raw_subject_json TEXT,
    raw_previous_subject_json TEXT,
    first_seen_sequence INTEGER,
    last_seen_sequence INTEGER,
    first_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE discovery_presentation_selection (
    scope_key TEXT PRIMARY KEY NOT NULL,
    language TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1
);
CREATE TABLE discovery_section_items (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    section_id TEXT NOT NULL,
    item_id TEXT NOT NULL REFERENCES discovery_items(id) ON DELETE CASCADE,
    sort_index INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, section_id, item_id)
);
CREATE TABLE discovery_sections (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    section_id TEXT NOT NULL,
    section_type TEXT NOT NULL,
    surface TEXT NOT NULL,
    title TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE discovery_submitted_subjects (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    subject_key TEXT NOT NULL,
    title_id TEXT REFERENCES titles(id) ON DELETE SET NULL,
    library_id TEXT,
    library_facet TEXT,
    title_kind TEXT,
    display_title TEXT,
    external_ids_json TEXT NOT NULL DEFAULT '[]',
    raw_subject_json TEXT NOT NULL
);
CREATE TABLE discovery_sync_runs (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    trigger_source TEXT NOT NULL,
    region TEXT NOT NULL,
    language TEXT NOT NULL,
    subject_count INTEGER NOT NULL DEFAULT 0,
    subject_fingerprint TEXT,
    previous_subject_fingerprint TEXT,
    base_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    changed_subject_count INTEGER NOT NULL DEFAULT 0,
    affected_target_count INTEGER NOT NULL DEFAULT 0,
    smg_request_id TEXT,
    smg_status TEXT,
    discovery_index_watermark TEXT,
    page_count INTEGER,
    item_count INTEGER,
    facet_count INTEGER,
    error_text TEXT,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
, acknowledged_at TEXT, presentation_revision BIGINT NOT NULL DEFAULT 0);
CREATE TABLE discovery_sync_state (
    scope_key TEXT PRIMARY KEY NOT NULL,
    last_success_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    last_public_feed_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    last_subject_fingerprint TEXT,
    last_context_snapshot_completed_at TEXT,
    last_incremental_reload_completed_at TEXT,
    last_public_feed_completed_at TEXT,
    dirty_since TEXT,
    dirty_reason_mask INTEGER NOT NULL DEFAULT 0,
    bootstrap_started_at TEXT,
    bootstrap_quiet_until TEXT,
    next_context_snapshot_eligible_at TEXT,
    next_incremental_reload_eligible_at TEXT,
    next_public_feed_eligible_at TEXT,
    backoff_until TEXT,
    startup_jitter_seconds INTEGER NOT NULL DEFAULT 0,
    context_jitter_seconds INTEGER NOT NULL DEFAULT 0,
    incremental_reload_jitter_seconds INTEGER NOT NULL DEFAULT 0,
    public_feed_jitter_seconds INTEGER NOT NULL DEFAULT 0,
    last_seen_domain_event_sequence INTEGER,
    inflight_subject_fingerprint TEXT,
    inflight_domain_event_sequence INTEGER,
    inflight_context_snapshot_run_id TEXT,
    lease_owner_id TEXT,
    lease_expires_at TEXT,
    transient_failure_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE discovery_title_external_ids (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    external_kind TEXT NOT NULL DEFAULT '',
    external_id TEXT NOT NULL DEFAULT '',
    external_key TEXT NOT NULL DEFAULT '',
    external_identity TEXT NOT NULL DEFAULT '',
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, source, external_kind, external_identity)
);
CREATE TABLE discovery_title_metadata_external_ratings (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    value REAL,
    score REAL,
    normalized REAL NOT NULL,
    votes INTEGER,
    url TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (discovery_title_id, source)
);
CREATE TABLE discovery_title_metadata_rating_sources (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (discovery_title_id, source)
);
CREATE TABLE discovery_title_metadata_rating_summaries (
    discovery_title_id TEXT PRIMARY KEY NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    rating REAL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE discovery_title_metadata_tag_source_keys (
    discovery_title_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    source_tag_key TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (discovery_title_id, tag_key)
        REFERENCES discovery_title_metadata_tags(discovery_title_id, tag_key) ON DELETE CASCADE,
    UNIQUE (discovery_title_id, tag_key, source_tag_key)
);
CREATE TABLE discovery_title_metadata_tag_sources (
    discovery_title_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (discovery_title_id, tag_key)
        REFERENCES discovery_title_metadata_tags(discovery_title_id, tag_key) ON DELETE CASCADE,
    UNIQUE (discovery_title_id, tag_key, source)
);
CREATE TABLE discovery_title_metadata_tags (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    tag_key TEXT NOT NULL,
    category TEXT NOT NULL,
    name TEXT NOT NULL,
    confidence REAL,
    is_adult INTEGER NOT NULL DEFAULT 0,
    is_spoiler INTEGER NOT NULL DEFAULT 0,
    sort_index INTEGER NOT NULL DEFAULT 0, affinity_signals_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (discovery_title_id, tag_key)
);
CREATE TABLE discovery_title_source_tag_values (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    source_tag_sort_index INTEGER NOT NULL,
    source_tag_value TEXT NOT NULL,
    value_sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, source_tag_sort_index, source_tag_value)
);
CREATE TABLE discovery_title_source_tags (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    category TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL DEFAULT '',
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, sort_index, category, name)
);
CREATE TABLE discovery_title_terms (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    term_kind TEXT NOT NULL,
    term_category TEXT NOT NULL DEFAULT '',
    term_value TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, term_kind, term_category, term_value)
);
CREATE TABLE discovery_titles (
    id TEXT PRIMARY KEY NOT NULL,
    target_key TEXT NOT NULL,
    target_key_norm TEXT NOT NULL,
    language TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0,
    resolved_title_id TEXT REFERENCES titles(id) ON DELETE SET NULL,
    display_title TEXT NOT NULL,
    original_title TEXT,
    sort_title TEXT,
    year INTEGER,
    poster_path TEXT,
    poster_url TEXT,
    background_url TEXT,
    overview TEXT,
    content_type TEXT,
    tmdb_collection_id TEXT,
    tmdb_collection_name TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, is_adult INTEGER NOT NULL DEFAULT 0, content_ratings_json TEXT NOT NULL DEFAULT '[]', affinity_signals_json TEXT NOT NULL DEFAULT '[]',
    UNIQUE (target_key_norm, language)
);
CREATE TABLE domain_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    actor_user_id TEXT,
    title_id TEXT,
    facet TEXT,
    correlation_id TEXT,
    causation_id TEXT,
    schema_version INTEGER NOT NULL,
    stream_kind TEXT NOT NULL,
    stream_id TEXT,
    event_type TEXT NOT NULL,
    payload_json BLOB NOT NULL,
    actor_kind TEXT NOT NULL DEFAULT 'system',
    actor_display_name TEXT NOT NULL DEFAULT 'System',
    import_status TEXT,
    media_file_delete_reason TEXT,
    download_id TEXT
, import_skip_reason TEXT);
CREATE TABLE download_cleanup (
    download_id TEXT PRIMARY KEY REFERENCES downloads(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    client_type TEXT NOT NULL,
    item_id TEXT NOT NULL,
    title_id TEXT,
    facet TEXT,
    source_title TEXT,
    tracked_state TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'completed')),
    attempts BIGINT NOT NULL DEFAULT 0,
    history_offset BIGINT NOT NULL DEFAULT 0,
    payload_checkpoint TEXT,
    outcome TEXT,
    last_error TEXT,
    next_attempt_at TEXT NOT NULL,
    lease_until TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE download_cleanup_seed_state (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    changed_through TEXT NOT NULL,
    full_scan_at TEXT NOT NULL
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
CREATE TABLE download_client_status (
    client_config_id TEXT PRIMARY KEY NOT NULL,
    initial_failure_at TEXT,
    most_recent_failure_at TEXT,
    escalation_level INTEGER NOT NULL DEFAULT 0,
    disabled_until TEXT,
    FOREIGN KEY(client_config_id) REFERENCES download_clients(id) ON DELETE CASCADE
);
CREATE TABLE download_clients(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    client_type TEXT NOT NULL,
    base_url TEXT,
    config_json TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'idle',
    last_error TEXT,
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, client_priority INTEGER NOT NULL DEFAULT 0, proxy_config_id TEXT);
CREATE TABLE "download_identity_states" (
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
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id)
);
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
    canonical_download_id TEXT, workspace_relative_path TEXT,
    FOREIGN KEY (import_id) REFERENCES imports(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL,
    FOREIGN KEY (imported_media_file_id) REFERENCES media_files(id) ON DELETE SET NULL,
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id)
);
CREATE TABLE "download_queue_commands" (
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
CREATE TABLE download_submission_episode_links (
    download_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    PRIMARY KEY (download_id, episode_id),
    FOREIGN KEY (download_id) REFERENCES download_submissions(id) ON DELETE CASCADE
);
CREATE TABLE "download_submissions" (
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
    release_size_bytes INTEGER, info_hash TEXT, release_listing_json TEXT, password_candidates TEXT, password_retry_state TEXT, grabbed_file_names_json TEXT,
    FOREIGN KEY (id) REFERENCES downloads(id)
);
CREATE TABLE downloads (
    id TEXT PRIMARY KEY,
    origin TEXT NOT NULL CHECK (origin IN ('scryer_submission', 'foreign_observation')),
    created_at TEXT NOT NULL,
    first_observed_at TEXT,
    last_observed_at TEXT,
    terminal_at TEXT
);
CREATE TABLE emby_media_server_details (
    connection_id TEXT PRIMARY KEY,
    api_key TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL, server_id TEXT, connect_enabled INTEGER NOT NULL DEFAULT 0 CHECK (connect_enabled IN (0, 1)),
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);
CREATE TABLE episode_external_ids(
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    provenance TEXT NOT NULL,
    source_scope TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE
);
CREATE TABLE episodes(
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    collection_id TEXT,
    episode_type TEXT NOT NULL,
    episode_number TEXT,
    season_number TEXT,
    episode_label TEXT,
    title TEXT,
    air_date TEXT,
    duration_seconds INTEGER,
    has_multi_audio INTEGER DEFAULT 0,
    has_subtitle INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT, monitored INTEGER NOT NULL DEFAULT 1, overview TEXT, is_filler INTEGER NOT NULL DEFAULT 0, absolute_number TEXT, is_recap INTEGER NOT NULL DEFAULT 0, tvdb_id TEXT, image_url TEXT, contiguous_absolute_number INTEGER, tmdb_id TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL
);
CREATE TABLE event_subscriber_offsets(
    subscriber_name TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE external_import_monitor_snapshot_chunks (
            session_id TEXT NOT NULL,
            facet TEXT NOT NULL CHECK (facet IN ('movie', 'series', 'anime')),
            entry_kind TEXT NOT NULL CHECK (entry_kind IN ('movie', 'series')),
            chunk_index INTEGER NOT NULL,
            payload_ndjson TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (session_id, facet, entry_kind, chunk_index)
        );
CREATE TABLE external_import_setup_download_client_api_key_overrides (
    draft_key TEXT NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    dedup_key TEXT NOT NULL,
    api_key_encrypted TEXT NOT NULL,
    position INTEGER NOT NULL
        CHECK (position >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (draft_key, dedup_key),
    UNIQUE (draft_key, position)
);
CREATE TABLE external_import_setup_download_client_password_overrides (
    draft_key TEXT NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    dedup_key TEXT NOT NULL,
    password_encrypted TEXT NOT NULL,
    position INTEGER NOT NULL
        CHECK (position >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (draft_key, dedup_key),
    UNIQUE (draft_key, position)
);
CREATE TABLE external_import_setup_indexer_api_key_overrides (
    draft_key TEXT NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    dedup_key TEXT NOT NULL,
    api_key_encrypted TEXT NOT NULL,
    position INTEGER NOT NULL
        CHECK (position >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (draft_key, dedup_key),
    UNIQUE (draft_key, position)
);
CREATE TABLE external_import_setup_instance_api_keys (
    draft_key TEXT NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    instance_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('sonarr', 'radarr', 'prowlarr')),
    api_key_encrypted TEXT NOT NULL,
    position INTEGER NOT NULL
        CHECK (position >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (draft_key, instance_id),
    UNIQUE (draft_key, position)
);
CREATE TABLE external_import_setup_secret_drafts (
    draft_key TEXT PRIMARY KEY NOT NULL
        CHECK (draft_key = 'active'),
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE external_subtitle_probe_cache (
    media_file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_at TEXT,
    language TEXT,
    hearing_impaired INTEGER,
    detection_source_language TEXT NOT NULL,
    detection_source_hi TEXT NOT NULL,
    probe_version INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (media_file_id, file_path)
);
CREATE TABLE file_episode_map(
    file_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    is_filler INTEGER DEFAULT 0, role TEXT NOT NULL DEFAULT 'additional'
    CHECK (role IN ('primary', 'additional')),
    PRIMARY KEY (file_id, episode_id),
    FOREIGN KEY (file_id) REFERENCES media_files(id) ON DELETE CASCADE,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE
);
CREATE TABLE file_series_movie_link_map (
    file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    series_movie_link_id TEXT NOT NULL REFERENCES series_movie_links(id) ON DELETE CASCADE,
    PRIMARY KEY (file_id, series_movie_link_id)
);
CREATE TABLE history_events(
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    actor_user_id TEXT,
    title_id TEXT,
    message TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    source TEXT,
    created_at TEXT NOT NULL,
    metadata_json TEXT,
    FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL
);
CREATE TABLE image_proxy_cache_entries (
  token TEXT NOT NULL,
  variant TEXT NOT NULL,
  content_type TEXT NOT NULL,
  byte_size INTEGER NOT NULL,
  upstream_etag TEXT,
  upstream_last_modified TEXT,
  fetched_at TEXT NOT NULL,
  last_accessed_at TEXT NOT NULL,
  PRIMARY KEY (token, variant),
  FOREIGN KEY (token) REFERENCES image_proxy_sources(token) ON DELETE CASCADE
);
CREATE TABLE image_proxy_sources (
  token TEXT PRIMARY KEY,
  upstream_url TEXT,
  owner_type TEXT,
  owner_id TEXT,
  image_kind TEXT NOT NULL,
  fallback_class TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
);
CREATE TABLE import_space_incident_lock (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL
);
CREATE TABLE import_space_incidents (
    destination_key TEXT NOT NULL PRIMARY KEY,
    incident_id TEXT NOT NULL
);
CREATE TABLE import_space_members (
    member_key TEXT NOT NULL PRIMARY KEY,
    job_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    measurement_json TEXT NOT NULL,
    download_id TEXT NOT NULL
, observed_at TEXT NOT NULL DEFAULT '');
CREATE TABLE import_space_notification_attempts (
    event_id TEXT NOT NULL PRIMARY KEY,
    attempts INTEGER NOT NULL,
    first_attempt_at TEXT NOT NULL
);
CREATE TABLE import_space_notification_receipts (
    event_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    PRIMARY KEY (event_id, target_key)
);
CREATE TABLE "imports" (
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
CREATE TABLE indexer_api_client_grants (
    client_id TEXT NOT NULL REFERENCES indexer_api_clients(id) ON DELETE CASCADE,
    indexer_id TEXT NOT NULL REFERENCES indexers(id) ON DELETE CASCADE,
    categories_json TEXT,
    PRIMARY KEY (client_id, indexer_id)
);
CREATE TABLE indexer_api_clients (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    protocols_json TEXT NOT NULL,
    lookup_id TEXT UNIQUE,
    secret_hash TEXT,
    key_generation INTEGER NOT NULL CHECK (key_generation > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    last_used_at TEXT,
    last_outcome_code TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK ((lookup_id IS NULL) = (secret_hash IS NULL))
);
CREATE TABLE indexer_api_quotas (
    indexer_id TEXT PRIMARY KEY NOT NULL,
    api_current INTEGER,
    api_max INTEGER,
    grab_current INTEGER,
    grab_max INTEGER,
    queries_today INTEGER NOT NULL DEFAULT 0,
    last_query_at TEXT,
    last_reset_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE indexer_errors (
    id TEXT PRIMARY KEY,
    indexer_id TEXT NOT NULL REFERENCES indexers(id) ON DELETE CASCADE,
    indexer_name TEXT NOT NULL,
    operation TEXT NOT NULL,
    http_status INTEGER NOT NULL,
    classification TEXT NOT NULL,
    provider_error_code INTEGER NULL,
    message TEXT NOT NULL,
    content_type TEXT NULL,
    payload_format_version INTEGER NOT NULL,
    response_zstd BLOB NOT NULL,
    occurred_at TEXT NOT NULL
);
CREATE TABLE indexer_facet_learning (
    indexer_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    empty_title_ids TEXT NOT NULL DEFAULT '[]',
    suppressed_until TEXT,
    reprobe_claimed_at TEXT,
    revision INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (indexer_id, facet)
);
CREATE TABLE indexer_search_candidate_source_values (
    source_id TEXT NOT NULL REFERENCES indexer_search_candidate_sources(id) ON DELETE CASCADE,
    value_kind TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY(source_id, value_kind, ordinal)
);
CREATE TABLE indexer_search_candidate_sources (
    id TEXT PRIMARY KEY,
    candidate_id TEXT NOT NULL REFERENCES indexer_search_candidates(id) ON DELETE CASCADE,
    indexer_id TEXT NOT NULL,
    source_identity TEXT NOT NULL,
    provider_ref TEXT,
    source TEXT NOT NULL,
    encrypted_download_url TEXT,
    encrypted_link_url TEXT,
    published_at TEXT,
    thumbs_up INTEGER,
    thumbs_down INTEGER,
    grabs INTEGER,
    grab_current INTEGER,
    grab_max INTEGER,
    response_tvdb_id TEXT,
    response_tmdb_id TEXT,
    response_imdb_id TEXT,
    season INTEGER,
    episode INTEGER,
    absolute_episode INTEGER,
    release_group TEXT,
    provider_source TEXT,
    seeders INTEGER,
    peers INTEGER,
    download_volume_factor REAL,
    upload_volume_factor REAL,
    protected INTEGER,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    reusable_until TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    UNIQUE(candidate_id, indexer_id, source_identity)
);
CREATE TABLE indexer_search_candidates (
    id TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    normalized_title TEXT NOT NULL,
    size_bytes INTEGER,
    source_kind TEXT,
    info_hash TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    reusable_until TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
CREATE TABLE indexer_search_learning (
    indexer_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    strategy_key TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    empty_successes INTEGER NOT NULL DEFAULT 0,
    usable_successes INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,
    last_usable_at TEXT,
    suppressed INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (indexer_id, title_id, facet, strategy_key)
);
CREATE TABLE indexer_search_listing_readers (
    indexer_id TEXT NOT NULL,
    indexer_fingerprint TEXT NOT NULL,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    query_signature TEXT NOT NULL,
    strategy_family TEXT NOT NULL,
    first_page_fingerprint TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (indexer_id, indexer_fingerprint, title_id, facet, scope_key, query_signature)
);
CREATE TABLE indexer_search_listing_runs (
    run_id TEXT NOT NULL,
    indexer_id TEXT NOT NULL,
    indexer_fingerprint TEXT NOT NULL,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    first_page_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (run_id)
);
CREATE TABLE indexer_search_listings (
    indexer_id TEXT NOT NULL,
    indexer_fingerprint TEXT NOT NULL,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    first_page_fingerprint TEXT NOT NULL,
    owner_scope_key TEXT NOT NULL,
    owner_query_signature TEXT NOT NULL,
    owner_strategy_family TEXT NOT NULL,
    page_cursor TEXT,
    finished INTEGER NOT NULL DEFAULT 0,
    chain_started_at TEXT NOT NULL,
    advanced_at TEXT NOT NULL,
    PRIMARY KEY (indexer_id, indexer_fingerprint, title_id, facet, first_page_fingerprint)
);
CREATE TABLE indexer_search_run_candidate_sources (
    run_id TEXT NOT NULL REFERENCES indexer_search_runs(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES indexer_search_candidate_sources(id) ON DELETE CASCADE,
    search_session_id TEXT NOT NULL,
    PRIMARY KEY(run_id, source_id)
);
CREATE TABLE indexer_search_runs (
    id TEXT PRIMARY KEY,
    indexer_id TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    search_session_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    query_signature TEXT NOT NULL,
    branch TEXT NOT NULL,
    page INTEGER,
    -- Reserved for the per-strategy search corpus (plan 151): the provider
    -- offset this run requested and the next offset it advertised. Nothing
    -- reads or writes them yet.
    provider_offset INTEGER,
    next_provider_offset INTEGER,
    range_min_size INTEGER,
    range_max_size INTEGER,
    result_count INTEGER NOT NULL,
    completion_state TEXT NOT NULL,
    retry_at TEXT,
    error_summary TEXT,
    indexer_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL
, strategy_state TEXT, coverage_scope_key TEXT, page_cursor TEXT, cursor_chain_id TEXT);
CREATE TABLE indexer_system_backoffs (
    indexer_id TEXT PRIMARY KEY NOT NULL,
    disabled_until TEXT NOT NULL,
    escalation_level INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(indexer_id) REFERENCES indexers(id) ON DELETE CASCADE
);
CREATE TABLE indexers(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key_encrypted TEXT,
    rate_limit_seconds INTEGER,
    rate_limit_burst INTEGER,
    disabled_until TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    last_health_status TEXT,
    last_error_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, enable_interactive_search INTEGER NOT NULL DEFAULT 1, enable_auto_search INTEGER NOT NULL DEFAULT 1, config_json TEXT, managed_parent_config_id TEXT, managed_child_key TEXT, managed_metadata_json TEXT, caps_snapshot_json TEXT, proxy_config_id TEXT, last_error_message TEXT, download_client_id TEXT
    REFERENCES download_clients(id)
    ON DELETE SET NULL, seeding_profile_id TEXT, max_queries_per_minute INTEGER, last_error_code TEXT);
CREATE TABLE jellyfin_media_server_details (
    connection_id TEXT PRIMARY KEY,
    api_key TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);
CREATE TABLE libraries (
    id TEXT PRIMARY KEY,
    facet TEXT NOT NULL,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE library_probe_signatures(
    title_id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    probe_signature_scheme TEXT,
    probe_signature_value TEXT,
    last_probed_at TEXT,
    last_changed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE library_root_id_remaps (
    legacy_root_id TEXT PRIMARY KEY NOT NULL,
    root_id TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    remapped INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE library_roots (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL, legacy_path_derived_id TEXT,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);
CREATE TABLE library_scan_unmatched_items (
    id TEXT PRIMARY KEY,
    facet TEXT NOT NULL,
    scan_session_id TEXT NOT NULL,
    scan_root TEXT NOT NULL,
    item_path TEXT NOT NULL,
    display_name TEXT NOT NULL,
    query TEXT NOT NULL,
    year_hint INTEGER,
    reason_code TEXT NOT NULL,
    error_message TEXT,
    search_attempts_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, status TEXT NOT NULL DEFAULT 'pending', title_id TEXT, library_id TEXT, size_bytes INTEGER);
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
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')), subject_kind TEXT NOT NULL DEFAULT 'title', subject_id TEXT NOT NULL DEFAULT '',
    UNIQUE (idempotency_key, attempt)
);
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
, action_attempts INTEGER NOT NULL DEFAULT 0, subject_id TEXT NOT NULL DEFAULT '');
CREATE TABLE lifecycle_claims (
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    library_id TEXT NOT NULL DEFAULT '',
    producer TEXT NOT NULL,
    producer_ref TEXT,
    kind TEXT NOT NULL DEFAULT 'retain_until',
    state TEXT NOT NULL DEFAULT 'dormant',
    duration_days INTEGER,
    starts_at TEXT,
    expires_at TEXT,
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    released_reason TEXT
);
CREATE TABLE list_exclusion_external_ids (
    exclusion_id TEXT NOT NULL REFERENCES list_exclusions(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    value TEXT NOT NULL, external_kind TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (exclusion_id, source, value)
);
CREATE TABLE list_exclusions (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    display_title TEXT NOT NULL,
    year INTEGER,
    -- 'all_lists' | 'list'
    scope TEXT NOT NULL,
    subscription_id TEXT REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    created_by_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE list_memberships (
    subscription_id TEXT NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    item_key TEXT NOT NULL,
    rank INTEGER,
    season INTEGER,
    display_title TEXT,
    year INTEGER,
    external_ids_json TEXT NOT NULL DEFAULT '[]',
    smg_title_id INTEGER,
    title_id TEXT,
    request_id TEXT,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    state_reason TEXT,
    added_by_list INTEGER NOT NULL DEFAULT 0,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    left_at TEXT,
    left_handled INTEGER NOT NULL DEFAULT 0, series_movie_json TEXT,
    PRIMARY KEY (subscription_id, item_key)
);
CREATE TABLE list_subscription_routes (
    subscription_id TEXT NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    quality_profile_id TEXT,
    root_folder_id TEXT,
    monitor_type TEXT NOT NULL,
    min_availability TEXT,
    use_season_folders INTEGER,
    release_numbering TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (subscription_id, kind)
);
CREATE TABLE list_subscriptions (
    id TEXT PRIMARY KEY NOT NULL,
    -- 'public' | 'personal'
    scope TEXT NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_params_json TEXT NOT NULL DEFAULT '{}',
    -- 'smg_chart' | 'provider_fetch'
    source_origin TEXT NOT NULL,
    chart_key TEXT,
    chart_scope TEXT,
    name TEXT NOT NULL,
    provider_url TEXT,
    kinds_json TEXT NOT NULL DEFAULT '[]',
    enabled INTEGER NOT NULL DEFAULT 1,
    -- 'search' | 'add' | 'hold' | 'request' | 'discover'
    mode TEXT NOT NULL,
    filters_json TEXT NOT NULL DEFAULT '[]',
    max_per_sync INTEGER,
    -- 'keep' | 'log' | 'unmonitor' | 'tag'
    on_leave TEXT NOT NULL DEFAULT 'keep',
    interval_seconds INTEGER NOT NULL,
    credential_id TEXT REFERENCES user_list_accounts(id) ON DELETE SET NULL,
    -- 'ok' | 'new' | 'fail' | 'off'
    sync_state TEXT NOT NULL DEFAULT 'new',
    last_sync_at TEXT,
    next_sync_at TEXT,
    error_message TEXT,
    error_at TEXT,
    paused_until TEXT,
    fetch_fingerprint TEXT,
    count_total INTEGER NOT NULL DEFAULT 0,
    count_in_library INTEGER NOT NULL DEFAULT 0,
    count_added INTEGER NOT NULL DEFAULT 0,
    count_requested INTEGER NOT NULL DEFAULT 0,
    count_held INTEGER NOT NULL DEFAULT 0,
    count_filtered INTEGER NOT NULL DEFAULT 0,
    count_excluded INTEGER NOT NULL DEFAULT 0,
    count_unresolved INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE list_sync_runs (
    id TEXT PRIMARY KEY NOT NULL,
    subscription_id TEXT NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    job_run_id TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    -- 'succeeded' | 'failed' | 'skipped'
    outcome TEXT NOT NULL,
    counts_json TEXT NOT NULL DEFAULT '{}',
    error_message TEXT
);
CREATE TABLE location_file_resolutions (
    operation_id TEXT NOT NULL REFERENCES location_operations(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    resolution_json TEXT NOT NULL,
    PRIMARY KEY (operation_id, title_id, source_path)
);
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
CREATE TABLE location_operation_plans (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES location_operations(id) ON DELETE CASCADE,
    plan BLOB NOT NULL
);
CREATE TABLE location_operation_title_checkpoints (
    operation_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    sequence INTEGER NOT NULL DEFAULT 0,
    -- pending | moving | verifying | reconciling | cleaning_up | completed
    -- | completed_with_warnings | skipped | blocked | failed
    -- (`TitleCheckpointState` in scryer-application::location::model)
    state TEXT NOT NULL DEFAULT 'pending',
    -- Per-title preview classification, as previewed (FR-015/FR-080); value set
    -- owned by scryer-application::location::classify.
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
    -- The three explanations a checkpoint can carry, each in its own column so
    -- Activity never has to guess which one it is reading: why the title could
    -- not enter the operation, why it failed, and — for a title that finished
    -- with warnings (dedup, collision rename, preserve-instead-of-recycle) —
    -- what the user still has to see.
    blocked_reason TEXT,
    failure_reason TEXT,
    note TEXT,
    checkpointed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, reason_code TEXT,
    PRIMARY KEY (operation_id, title_id),
    FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE
);
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
    -- verified | mismatch | unavailable
    -- (`FileVerificationOutcome` in scryer-application::location::model)
    outcome TEXT NOT NULL,
    move_crc TEXT,
    move_crc_algorithm TEXT,
    full_blake3 TEXT,
    sampled_signature_scheme TEXT,
    sampled_signature_value TEXT,
    failure_reason TEXT,
    -- The note for a record that neither fell back nor failed: how a verified
    -- destination was proven, in the words Activity shows per file (FR-043).
    -- Distinct from `fallback_reason` and `failure_reason` so the three cases
    -- stay separable after the fact.
    detail TEXT,
    verified_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE
);
CREATE TABLE location_operations (
    id TEXT PRIMARY KEY NOT NULL,
    -- folder_reassignment | root_move | root_change | root_consolidation
    -- | cross_library_transfer | adoption
    operation_type TEXT NOT NULL,
    -- move_with_scryer | files_already_there | catalog_only
    -- (`LocationExecutionMode` in scryer-application::location::model)
    execution_mode TEXT NOT NULL,
    -- queued | preparing | moving | verifying | reconciling | cleaning_up
    -- | completed | completed_with_warnings | canceled | failed
    -- (`LocationOperationState` in scryer-application::location::model, FR-091)
    state TEXT NOT NULL DEFAULT 'queued',
    initiated_by_user_id TEXT,
    source_library_id TEXT,
    source_root_id TEXT,
    destination_library_id TEXT,
    destination_root_id TEXT,
    -- Fingerprint of the confirmed plan (FR-081/FR-089). Empty until a plan is built.
    plan_fingerprint TEXT NOT NULL DEFAULT '',
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
    -- Outcome counters Activity shows next to the volume counters above
    -- (FR-091, US8 scenario 1). Every one of them is derived from decisions the
    -- confirmed plan already made, so a resumed run recomputes the same values
    -- rather than incrementing a running total it cannot trust.
    --   merge_count      titles merged into an existing destination title (US7)
    --   dedup_count      files/assets recycled as proven duplicates (FR-073)
    --   rename_count     files/assets renamed to avoid a collision (FR-074/075)
    --   no_op_count      titles that needed no change
    --   unresolved_count items still needing a user decision (FR-016, FR-086)
    merge_count INTEGER NOT NULL DEFAULT 0,
    dedup_count INTEGER NOT NULL DEFAULT 0,
    rename_count INTEGER NOT NULL DEFAULT 0,
    no_op_count INTEGER NOT NULL DEFAULT 0,
    unresolved_count INTEGER NOT NULL DEFAULT 0,
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
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, reason_code TEXT, recovery_key TEXT, recovery_committed_at TEXT,
    FOREIGN KEY (initiated_by_user_id) REFERENCES users(id) ON DELETE SET NULL
);
CREATE TABLE location_recovery_staging (
    id TEXT PRIMARY KEY NOT NULL,
    operation_id TEXT NOT NULL REFERENCES location_operations(id),
    media_file_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    destination_path TEXT NOT NULL,
    staging_path TEXT NOT NULL UNIQUE
);
CREATE TABLE location_transfer_progress (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES location_operations(id) ON DELETE CASCADE,
    progress_basis_points BIGINT NOT NULL DEFAULT 0,
    titles_initialized BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE TABLE location_transfer_runtime (
    id BIGINT PRIMARY KEY NOT NULL CHECK (id = 1),
    generation BIGINT NOT NULL
);
CREATE TABLE location_transfer_titles (
    operation_id TEXT NOT NULL REFERENCES location_operations(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    hot_rank BIGINT NOT NULL,
    summary_json TEXT NOT NULL,
    PRIMARY KEY (operation_id, title_id)
);
CREATE TABLE login_verification_challenges (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    login_method TEXT NOT NULL,
    persist_session INTEGER NOT NULL,
    allow_passkey INTEGER NOT NULL,
    allow_totp INTEGER NOT NULL,
    auth_session_version TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (login_method IN ('local_password', 'jellyfin', 'emby')),
    CHECK (persist_session IN (0, 1)),
    CHECK (allow_passkey IN (0, 1)),
    CHECK (allow_totp IN (0, 1))
);
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
CREATE TABLE maintenance_rule_exclusions (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
, subject_kind TEXT NOT NULL DEFAULT 'title', subject_id TEXT NOT NULL DEFAULT '');
CREATE TABLE maintenance_rule_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT NOT NULL REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    revision_number INTEGER NOT NULL,
    rego_source TEXT NOT NULL,
    action_spec TEXT NOT NULL,
    grace_days INTEGER NOT NULL DEFAULT 0,
    matcher_content_hash TEXT NOT NULL,
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')), storage_root_id TEXT,
    UNIQUE (rule_set_id, revision_number)
);
CREATE TABLE maintenance_rule_set_libraries (
    rule_set_id TEXT NOT NULL REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE RESTRICT,
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (rule_set_id, library_id),
    UNIQUE (rule_set_id, position)
);
CREATE TABLE maintenance_rule_sets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 0,
    evaluation_mode TEXT NOT NULL DEFAULT 'disabled',
    subject_kind TEXT NOT NULL DEFAULT 'title',
    current_revision_number INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
, effect_arming TEXT NOT NULL DEFAULT 'none', destructive_rearm_required INTEGER NOT NULL DEFAULT 0, system_owned INTEGER NOT NULL DEFAULT 0);
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
CREATE TABLE manual_import_selection_candidates (
    id TEXT PRIMARY KEY,
    selection_id TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    quality TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (selection_id, canonical_path)
);
CREATE TABLE manual_import_selections (
    id TEXT PRIMARY KEY,
    actor_user_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    source_client_id TEXT NOT NULL DEFAULT '',
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    consumed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, release_evidence_json TEXT, trusted_source_root TEXT NOT NULL DEFAULT '', archive_workspace_root TEXT, canonical_download_id TEXT);
CREATE TABLE media_file_analyses (
    media_file_id TEXT PRIMARY KEY NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    analysis BLOB NOT NULL
);
CREATE TABLE media_files(
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    file_path TEXT NOT NULL UNIQUE,
    size_bytes INTEGER NOT NULL,
    quality_id TEXT,
    hash_sha256 TEXT,
    has_multiaudio INTEGER DEFAULT 0,
    scan_status TEXT NOT NULL DEFAULT 'pending',
    scan_error TEXT,
    created_at TEXT NOT NULL, video_codec TEXT, video_width INTEGER, video_height INTEGER, video_bitrate_kbps INTEGER, video_bit_depth INTEGER, video_hdr_format TEXT, audio_codec TEXT, audio_channels INTEGER, duration_seconds INTEGER, container_format TEXT, video_frame_rate TEXT, video_profile TEXT, audio_bitrate_kbps INTEGER, scene_name TEXT, release_group TEXT, source_type TEXT, resolution TEXT, video_codec_parsed TEXT, audio_codec_parsed TEXT, acquisition_score INTEGER, scoring_log TEXT, indexer_source TEXT, grabbed_release_title TEXT, grabbed_at TEXT, edition TEXT, original_file_path TEXT, release_hash TEXT, num_chapters INTEGER, source_signature_scheme TEXT, source_signature_value TEXT, audio_profile TEXT, audio_channels_parsed TEXT, role TEXT NOT NULL DEFAULT 'primary', announced_size_bytes INTEGER, full_blake3 TEXT, move_crc TEXT, move_crc_algorithm TEXT, hash_computed_at TEXT, analysis_revision INTEGER NOT NULL DEFAULT 0, analysis_attempt_revision INTEGER, analysis_attempted_at TEXT, analysis_attempt_source TEXT, analysis_attempt_status TEXT, analysis_attempt_json TEXT, release_listing_json TEXT, current_score_evaluation TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE media_request_external_ids (
    request_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (request_id, source, external_id),
    FOREIGN KEY (request_id) REFERENCES media_requests(id) ON DELETE CASCADE,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);
CREATE TABLE media_request_requesters (
    request_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    PRIMARY KEY (request_id, user_id),
    FOREIGN KEY (request_id) REFERENCES media_requests(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE "media_requests" (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    status TEXT NOT NULL,
    identity_fingerprint TEXT NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT,
    slug TEXT,
    poster_url TEXT,
    year INTEGER,
    overview TEXT,
    runtime_minutes INTEGER,
    language TEXT,
    content_status TEXT,
    requested_quality_profile_id TEXT,
    requested_quality_profile_name TEXT,
    requested_monitor_type TEXT,
    resolved_by_user_id TEXT,
    resolved_at TEXT,
    created_title_id TEXT,
    approved_quality_profile_id TEXT,
    approved_quality_profile_name TEXT,
    created_by_user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    rating_summary_json TEXT NOT NULL DEFAULT '{"rating":null,"rating_sources":[],"external_ratings":[]}', background_url TEXT, requested_lease_days INTEGER, approved_lease_days INTEGER, decision_id TEXT, decided_by_rule_set_ids TEXT NOT NULL DEFAULT '[]', policy_tags_json TEXT NOT NULL DEFAULT '[]', metadata_snapshot_json TEXT NOT NULL DEFAULT '{}', origin_kind TEXT NOT NULL DEFAULT 'manual', origin_subscription_id TEXT,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE,
    FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (resolved_by_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (created_title_id) REFERENCES titles(id) ON DELETE SET NULL,
    CHECK (facet IN ('movie', 'series', 'anime')),
    CHECK (status IN ('pending', 'approved', 'rejected', 'canceled'))
);
CREATE TABLE media_server_audience_plays (
    connection_id TEXT NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    external_account_id TEXT NOT NULL CHECK (external_account_id <> ''),
    provider_item_id TEXT NOT NULL CHECK (provider_item_id <> ''),
    kind TEXT NOT NULL CHECK (kind IN ('movie', 'episode')),
    scryer_title_id TEXT,
    scryer_episode_id TEXT,
    play_count INTEGER NOT NULL DEFAULT 0,
    last_played_at TEXT,
    PRIMARY KEY (connection_id, external_account_id, provider_item_id)
);
CREATE TABLE media_server_audience_reads (
    connection_id TEXT NOT NULL PRIMARY KEY
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    sweep_started_at TEXT NOT NULL,
    read_at TEXT NOT NULL,
    play_count INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE media_server_connections (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    login_enabled INTEGER NOT NULL DEFAULT 0,
    linking_enabled INTEGER NOT NULL DEFAULT 0,
    auto_add_enabled INTEGER NOT NULL DEFAULT 0,
    default_app_permissions INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL, external_url TEXT,
    CHECK (provider IN ('jellyfin', 'plex', 'emby'))
);
CREATE TABLE media_server_default_library_grants (
    connection_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    permissions INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (connection_id, library_id),
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);
CREATE TABLE media_server_path_mappings (
    id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    destination_path TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);
CREATE TABLE media_server_playback_items (
    connection_id TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    provider_item_id TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, entity_kind, entity_id),
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE,
    CHECK (entity_kind IN ('title', 'episode'))
);
CREATE TABLE media_server_signal_roster (
    connection_id TEXT NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    link_id TEXT NOT NULL CHECK (link_id <> ''),
    external_user_id TEXT NOT NULL,
    scryer_user_id TEXT NOT NULL,
    -- The link's verified_at as the sweep saw it, kept for diagnosis. A login
    -- refreshes the link's value without changing whose plays are read.
    link_verified_at TEXT,
    sweep_started_at TEXT NOT NULL,
    read_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, link_id)
);
CREATE TABLE media_server_signal_sync_state (
    connection_id TEXT PRIMARY KEY NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    -- Snapshot of the connection's enabled flag as of the last sweep, so a
    -- reader can tell "nothing was collected because the connection is off"
    -- from "nothing was collected because the sweep failed".
    enabled INTEGER NOT NULL DEFAULT 0,
    last_started_at TEXT,
    last_success_at TEXT,
    last_error TEXT,
    participant_count INTEGER NOT NULL DEFAULT 0,
    signal_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
, last_error_code TEXT);
CREATE TABLE media_server_user_media_signals (
    id TEXT PRIMARY KEY NOT NULL,
    connection_id TEXT NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    -- Denormalized from the linked account at sync time. Kept as a plain
    -- column rather than a join so a later account unlink leaves the
    -- observation attributable to the identity that produced it.
    scryer_user_id TEXT,
    provider_item_id TEXT NOT NULL,
    -- 'movie' or 'episode'. Show-level rollups are computed by readers, never
    -- stored: a stored rollup would be a second source of truth that the next
    -- sweep could silently contradict.
    kind TEXT NOT NULL,
    scryer_title_id TEXT,
    scryer_episode_id TEXT,
    played INTEGER NOT NULL DEFAULT 0,
    play_count INTEGER NOT NULL DEFAULT 0,
    last_played_at TEXT,
    observed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    sync_generation INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
, is_favorite INTEGER NOT NULL DEFAULT 0);
CREATE TABLE monitor_selections (
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    entry_kind TEXT NOT NULL,
    entry_key TEXT NOT NULL,
    label TEXT,
    external_ids_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (owner_kind, owner_id, entry_kind, entry_key),
    CHECK (owner_kind IN ('title', 'media_request')),
    CHECK (entry_kind IN ('season', 'series_movie'))
);
CREATE TABLE movie_entities (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT,
    slug TEXT,
    year INTEGER,
    overview TEXT,
    poster_url TEXT,
    background_url TEXT,
    language TEXT,
    runtime_minutes INTEGER,
    content_status TEXT,
    genres_json TEXT NOT NULL DEFAULT '[]',
    studio TEXT,
    digital_release_date TEXT,
    imdb_id TEXT,
    tvdb_id TEXT,
    tmdb_id TEXT,
    mal_id TEXT,
    anidb_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE notification_channels(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    config_json TEXT NOT NULL,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, media_server_connection_id TEXT);
CREATE TABLE "notification_subscriptions" (
    id TEXT PRIMARY KEY,
    channel_id TEXT,
    target_kind TEXT NOT NULL DEFAULT 'plugin_channel',
    target_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (target_kind IN ('plugin_channel', 'media_server_connection')),
    FOREIGN KEY (channel_id) REFERENCES notification_channels(id) ON DELETE CASCADE
);
CREATE TABLE oauth_authorization_codes (
    id TEXT PRIMARY KEY,
    code_hash TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    scope TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT
, authorization_source TEXT NOT NULL DEFAULT 'authenticated', auth_session_version TEXT NOT NULL DEFAULT '', jellyfin_connection_id TEXT NULL, jellyfin_external_url TEXT NULL, jellyfin_base_url TEXT NULL, jellyfin_api_key_hash TEXT NULL);
CREATE TABLE oauth_client_redirect_uris (
    client_id TEXT NOT NULL REFERENCES oauth_client_registrations(client_id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    PRIMARY KEY (client_id, redirect_uri)
);
CREATE TABLE oauth_client_registrations (
    client_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'custom'
        CHECK (kind IN ('custom', 'jellyfin_plugin')),
    CHECK (enabled IN (0, 1))
);
CREATE TABLE oauth_refresh_grants (
    id TEXT PRIMARY KEY,
    family_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    auth_session_version TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_used_at TEXT,
    revoked_at TEXT,
    revoked_reason TEXT
, authorization_source TEXT NOT NULL DEFAULT 'authenticated', redirect_uri TEXT NOT NULL DEFAULT '', jellyfin_connection_id TEXT NULL, jellyfin_external_url TEXT NULL, jellyfin_base_url TEXT NULL, jellyfin_api_key_hash TEXT NULL);
CREATE TABLE oauth_refresh_tokens (
    id TEXT PRIMARY KEY,
    grant_id TEXT NOT NULL REFERENCES oauth_refresh_grants(id) ON DELETE CASCADE,
    family_id TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    consumed_at TEXT,
    revoked_at TEXT
);
CREATE TABLE pending_releases (
    id TEXT PRIMARY KEY,
    wanted_item_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    release_title TEXT NOT NULL,
    release_url TEXT,
    release_size_bytes INTEGER,
    release_score INTEGER NOT NULL,
    scoring_log_json TEXT,
    indexer_source TEXT,
    release_guid TEXT,
    added_at TEXT NOT NULL,
    delay_until TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'waiting',
    grabbed_at TEXT
, source_kind TEXT, source_password TEXT, published_at TEXT, info_hash TEXT, indexer_id TEXT, minimum_seed_ratio REAL, minimum_seed_time_minutes INTEGER, season_pack_seed_ratio REAL, season_pack_seed_time_minutes INTEGER, seeders INTEGER, release_identity TEXT, last_observed_at TEXT NOT NULL DEFAULT '', coverage_identity TEXT, role TEXT NOT NULL DEFAULT 'primary', last_decision_code TEXT, release_age_unknown INTEGER NOT NULL DEFAULT 0, release_listing_json TEXT, http_release_json TEXT);
CREATE TABLE plex_media_server_details (
    connection_id TEXT PRIMARY KEY,
    machine_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL, api_key TEXT,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);
CREATE TABLE plugin_catalog_sources (
    source_key      TEXT PRIMARY KEY,
    source_kind     TEXT NOT NULL,
    source_url      TEXT NOT NULL,
    github_repo     TEXT,
    support_tier    TEXT NOT NULL DEFAULT 'official',
    catalog_json    TEXT,
    last_success_at TEXT,
    last_error      TEXT,
    updated_at      TEXT NOT NULL
);
CREATE TABLE plugin_catalog_status (
    status_key      TEXT PRIMARY KEY,
    status_json     TEXT NOT NULL,
    checked_at      TEXT NOT NULL
);
CREATE TABLE plugin_installations (
    id               TEXT PRIMARY KEY,
    plugin_id        TEXT NOT NULL UNIQUE,
    name             TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    version          TEXT NOT NULL,
    sdk_version      TEXT NOT NULL DEFAULT '',
    sdk_constraint   TEXT NOT NULL DEFAULT '',
    scryer_constraint TEXT,
    plugin_type      TEXT NOT NULL DEFAULT 'indexer',
    provider_type    TEXT NOT NULL,
    is_enabled       INTEGER NOT NULL DEFAULT 1,
    is_builtin       INTEGER NOT NULL DEFAULT 0,
    source_kind      TEXT NOT NULL DEFAULT 'downloaded',
    wasm_bytes       BLOB,
    wasm_encoding    TEXT NOT NULL DEFAULT 'identity',
    wasm_digest_algo TEXT,
    source_url       TEXT,
    support_tier     TEXT NOT NULL DEFAULT 'official',
    publisher        TEXT,
    docs_url         TEXT,
    source_repo      TEXT,
    manifest_url     TEXT,
    wasm_digest      TEXT,
    artifact_digest  TEXT,
    installed_at     TEXT NOT NULL,
    updated_at       TEXT NOT NULL
, descriptor_json TEXT);
CREATE TABLE post_processing_script_runs (
    id TEXT PRIMARY KEY,
    script_id TEXT NOT NULL,
    script_name TEXT NOT NULL,                    -- denormalized for history
    title_id TEXT,
    title_name TEXT,
    facet TEXT,
    file_path TEXT,
    status TEXT NOT NULL,                         -- 'success' | 'failed' | 'timeout' | 'running'
    exit_code INTEGER,
    stdout_tail BLOB,                             -- last 32 KiB, zstd frame
    stderr_tail BLOB,                             -- last 32 KiB, zstd frame
    duration_ms INTEGER,
    env_payload_json TEXT,                        -- the JSON payload passed to the script
    started_at TEXT NOT NULL,
    completed_at TEXT,
    FOREIGN KEY (script_id) REFERENCES post_processing_scripts(id) ON DELETE CASCADE
);
CREATE TABLE post_processing_scripts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT DEFAULT '',
    script_type TEXT NOT NULL DEFAULT 'inline',   -- 'inline' | 'file'
    script_content TEXT NOT NULL DEFAULT '',       -- shell command (inline) or file path
    applied_facets TEXT NOT NULL DEFAULT '[]',     -- JSON: ["movie","tv","anime"]
    execution_mode TEXT NOT NULL DEFAULT 'blocking', -- 'blocking' | 'fire_and_forget'
    timeout_secs INTEGER DEFAULT 300,
    priority INTEGER NOT NULL DEFAULT 0,          -- lower = runs first
    enabled INTEGER NOT NULL DEFAULT 1,
    debug INTEGER NOT NULL DEFAULT 0,             -- capture stdout/stderr when enabled
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, language TEXT NOT NULL DEFAULT 'shell', trigger TEXT NOT NULL DEFAULT 'post_import', schedule_json TEXT, run_on_startup INTEGER NOT NULL DEFAULT 0);
CREATE TABLE proxy_configs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    protocol TEXT,
    base_url TEXT NOT NULL,
    request_timeout_seconds INTEGER NOT NULL DEFAULT 60,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    username_encrypted TEXT,
    -- Transport proxy passwords. SSH authenticates with an Ed25519 private key.
    password_encrypted TEXT,
    remote_dns INTEGER NOT NULL DEFAULT 0,
    private_key_encrypted TEXT,
    private_key_passphrase_encrypted TEXT,
    host_key_fingerprint TEXT,
    host_key_pinned_at TEXT,
    last_health_status TEXT,
    last_error_message TEXT,
    last_error_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
, peer_public_key TEXT, preshared_key_encrypted TEXT, tunnel_public_key TEXT, tunnel_addresses TEXT, tunnel_dns_servers TEXT, tunnel_mtu INTEGER, tunnel_keepalive_seconds INTEGER);
CREATE TABLE quality_profile_audio_codec_allowlist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_audio_codec_blocklist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_quality_tiers(
    profile_id TEXT NOT NULL,
    quality_tier TEXT NOT NULL,
    sort_order INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, quality_tier),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_source_allowlist(
    profile_id TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, source),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_source_blocklist(
    profile_id TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, source),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_video_codec_allowlist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_video_codec_blocklist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profiles(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    archival_quality TEXT,
    allow_unknown_quality INTEGER NOT NULL DEFAULT 0,
    atmos_preferred INTEGER NOT NULL DEFAULT 0,
    dolby_vision_allowed INTEGER NOT NULL DEFAULT 0,
    detected_hdr_allowed INTEGER NOT NULL DEFAULT 1,
    prefer_remux INTEGER NOT NULL DEFAULT 0,
    allow_bd_disk INTEGER NOT NULL DEFAULT 0,
    allow_upgrades INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
, prefer_dual_audio INTEGER NOT NULL DEFAULT 0, required_audio_languages TEXT NOT NULL DEFAULT '[]', scoring_config TEXT NOT NULL DEFAULT '{}');
CREATE TABLE release_decisions (
    id TEXT PRIMARY KEY,
    wanted_item_id TEXT NOT NULL REFERENCES wanted_items(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL,
    release_title TEXT NOT NULL,
    release_url TEXT,
    release_size_bytes INTEGER,
    decision_code TEXT NOT NULL,
    candidate_score INTEGER NOT NULL,
    current_score INTEGER,
    score_delta INTEGER,
    explanation_json BLOB,
    created_at TEXT NOT NULL
);
CREATE TABLE release_download_attempts(
    id TEXT PRIMARY KEY,
    title_id TEXT,
    source_hint TEXT,
    source_title TEXT,
    outcome TEXT NOT NULL,
    error_message TEXT,
    attempted_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL, source_password TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL
);
CREATE TABLE request_rule_decisions (
    id TEXT PRIMARY KEY NOT NULL,
    request_id TEXT NOT NULL,
    evaluated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    mode TEXT NOT NULL DEFAULT 'disabled',
    effective_outcome TEXT NOT NULL DEFAULT 'manual_review',
    policy_outcome TEXT NOT NULL DEFAULT 'manual_review',
    fallback_reason TEXT,
    votes_json TEXT NOT NULL DEFAULT '[]',
    tags_json TEXT NOT NULL DEFAULT '[]',
    input_hash TEXT NOT NULL DEFAULT '',
    input_schema_version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE request_rule_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT NOT NULL REFERENCES request_rule_sets(id) ON DELETE CASCADE,
    revision_number INTEGER NOT NULL,
    rego_source TEXT NOT NULL,
    matcher_content_hash TEXT NOT NULL,
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (rule_set_id, revision_number)
);
CREATE TABLE request_rule_set_libraries (
    rule_set_id TEXT NOT NULL REFERENCES request_rule_sets(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE RESTRICT,
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (rule_set_id, library_id),
    UNIQUE (rule_set_id, position)
);
CREATE TABLE request_rule_sets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 0,
    evaluation_mode TEXT NOT NULL DEFAULT 'disabled',
    current_revision_number INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE rule_pack_installations (
    pack_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    digest TEXT NOT NULL,
    auto_update INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    last_updated TEXT NOT NULL,
    last_error TEXT
, customizable INTEGER NOT NULL DEFAULT 1);
CREATE TABLE rule_pack_members (
    pack_id TEXT NOT NULL REFERENCES rule_pack_installations(pack_id) ON DELETE CASCADE,
    template_id TEXT NOT NULL,
    rule_set_id TEXT NOT NULL REFERENCES rule_sets(id) ON DELETE CASCADE,
    removed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (pack_id, template_id),
    UNIQUE (rule_set_id)
);
CREATE TABLE rule_set_history (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT NOT NULL,
    action TEXT NOT NULL,
    rego_source TEXT,
    actor_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE rule_sets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    rego_source TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    priority INTEGER NOT NULL DEFAULT 0,
    applied_facets TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
, is_managed INTEGER NOT NULL DEFAULT 0, managed_key TEXT, managed_tag_filter TEXT, evaluation_phase TEXT NOT NULL DEFAULT 'additional', exclusive_group TEXT, disabled_reason TEXT, disabled_reason_code TEXT);
CREATE TABLE scope_indexer_coverage (
    scope_key TEXT NOT NULL,
    facet TEXT NOT NULL,
    indexer_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    searched_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (scope_key, facet, indexer_id)
);
CREATE TABLE seeding_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    ratio REAL,
    seed_time_minutes INTEGER,
    season_pack_mode TEXT NOT NULL DEFAULT 'inherit',
    season_pack_ratio REAL,
    season_pack_seed_time_minutes INTEGER,
    honor_tracker_minimums INTEGER NOT NULL DEFAULT 1,
    goal_met_action TEXT NOT NULL DEFAULT 'remove_entry',
    never_remove INTEGER NOT NULL DEFAULT 0,
    minimum_seeders INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
, post_import_tracking TEXT NOT NULL DEFAULT 'park');
CREATE TABLE series_movie_links (
    id TEXT PRIMARY KEY NOT NULL,
    series_title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT NOT NULL REFERENCES movie_entities(id) ON DELETE CASCADE,
    placement TEXT,
    narrative_order TEXT,
    after_season INTEGER,
    before_season INTEGER,
    linked_episode_id TEXT REFERENCES episodes(id) ON DELETE SET NULL,
    association_confidence TEXT,
    continuity_status TEXT,
    movie_form TEXT,
    confidence TEXT,
    signal_summary TEXT,
    source TEXT,
    monitored INTEGER NOT NULL DEFAULT 1,
    legacy_collection_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL, monitoring_override INTEGER, metadata_active INTEGER NOT NULL DEFAULT 1, tags TEXT NOT NULL DEFAULT '[]',
    UNIQUE(legacy_collection_id)
);
CREATE TABLE settings_definitions(
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    scope TEXT NOT NULL,
    key_name TEXT NOT NULL,
    data_type TEXT NOT NULL,
    default_value_json TEXT,
    is_sensitive INTEGER NOT NULL DEFAULT 0,
    validation_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(category, scope, key_name)
);
CREATE TABLE "settings_values"(
    id TEXT PRIMARY KEY,
    setting_definition_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    value_json TEXT NOT NULL,
    source TEXT NOT NULL,
    updated_by_user_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (setting_definition_id) REFERENCES settings_definitions(id) ON DELETE CASCADE
);
CREATE TABLE "subtitle_blocklist" (
    id TEXT PRIMARY KEY,
    media_file_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_file_id TEXT NOT NULL,
    language TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(media_file_id, provider, provider_file_id)
);
CREATE TABLE subtitle_downloads (
    id TEXT PRIMARY KEY,
    media_file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    episode_id TEXT,
    language TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_file_id TEXT,
    file_path TEXT NOT NULL,
    score INTEGER,
    hearing_impaired INTEGER NOT NULL DEFAULT 0,
    forced INTEGER NOT NULL DEFAULT 0,
    ai_translated INTEGER NOT NULL DEFAULT 0,
    machine_translated INTEGER NOT NULL DEFAULT 0,
    uploader TEXT,
    release_info TEXT,
    synced INTEGER NOT NULL DEFAULT 0,
    downloaded_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
, source_kind TEXT NOT NULL DEFAULT 'downloaded');
CREATE TABLE subtitle_provider_configs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    config_json TEXT NOT NULL,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    last_health_status TEXT,
    last_error TEXT,
    last_error_at TEXT,
    disabled_until TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, enabled_facets TEXT NOT NULL DEFAULT '[]');
CREATE TABLE subtitle_search_backoffs (
    media_file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    language TEXT NOT NULL,
    hearing_impaired INTEGER NOT NULL,
    forced INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    wanted_languages TEXT NOT NULL,
    enabled_providers TEXT NOT NULL,
    empty_searches INTEGER NOT NULL,
    searched_at TEXT NOT NULL,
    retry_after TEXT NOT NULL,
    PRIMARY KEY (media_file_id, language, hearing_impaired, forced)
);
CREATE TABLE title_anime_numbering_bridges (
    title_id TEXT PRIMARY KEY,
    generated_on TEXT NOT NULL,
    corroborating_order TEXT,
    seasons_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL, source TEXT NOT NULL DEFAULT 'anime_community',
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE title_credits (
    title_id TEXT REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL,
    person_id TEXT NOT NULL,
    person_name TEXT NOT NULL DEFAULT '',
    person_original_name TEXT NOT NULL DEFAULT '',
    person_image_url TEXT NOT NULL DEFAULT '',
    person_source TEXT NOT NULL DEFAULT '',
    person_external_id TEXT NOT NULL DEFAULT '',
    character_name TEXT NOT NULL DEFAULT '',
    language TEXT NOT NULL DEFAULT '',
    billing_order INTEGER NOT NULL DEFAULT 0,
    episode_count INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL))
);
CREATE TABLE title_external_ids(
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT, facet TEXT, library_id TEXT, external_kind TEXT NOT NULL DEFAULT '', external_key TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE title_image_blobs (
            digest TEXT PRIMARY KEY,
            format TEXT NOT NULL,
            width INTEGER NOT NULL,
            height INTEGER NOT NULL,
            bytes BLOB NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
CREATE TABLE "title_image_variants" (
            id TEXT PRIMARY KEY,
            title_image_id TEXT NOT NULL,
            variant_key TEXT NOT NULL,
            blob_digest TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE (title_image_id, variant_key),
            FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE,
            FOREIGN KEY (blob_digest) REFERENCES title_image_blobs(digest) ON DELETE RESTRICT
        );
CREATE TABLE title_images (
  id TEXT PRIMARY KEY,
  title_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  provider_image_id TEXT,
  kind TEXT NOT NULL CHECK (kind IN ('poster', 'fanart')),
  source_url TEXT NOT NULL,
  source_etag TEXT,
  source_last_modified TEXT,
  source_format TEXT NOT NULL,
  source_width INTEGER NOT NULL,
  source_height INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (title_id, kind),
  FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE title_metadata_external_ratings (
    title_id TEXT REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    value REAL,
    score REAL,
    normalized REAL NOT NULL,
    votes INTEGER,
    url TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL))
);
CREATE TABLE title_metadata_rating_sources (
    title_id TEXT REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL))
);
CREATE TABLE title_metadata_rating_summaries (
    title_id TEXT REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE,
    rating REAL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL))
);
CREATE TABLE title_metadata_tag_source_keys (
    title_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    source_tag_key TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (title_id, tag_key)
        REFERENCES title_metadata_tags(title_id, tag_key) ON DELETE CASCADE,
    UNIQUE (title_id, tag_key, source_tag_key)
);
CREATE TABLE title_metadata_tag_sources (
    title_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (title_id, tag_key)
        REFERENCES title_metadata_tags(title_id, tag_key) ON DELETE CASCADE,
    UNIQUE (title_id, tag_key, source)
);
CREATE TABLE title_metadata_tags (
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    tag_key TEXT NOT NULL,
    category TEXT NOT NULL,
    name TEXT NOT NULL,
    confidence REAL,
    is_adult INTEGER NOT NULL DEFAULT 0,
    is_spoiler INTEGER NOT NULL DEFAULT 0,
    sort_index INTEGER NOT NULL DEFAULT 0, affinity_signals_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (title_id, tag_key)
);
CREATE TABLE "title_more_like_this_items" (
    source_title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    discovery_title_id TEXT NOT NULL
        REFERENCES title_recommendation_cards(discovery_title_id) ON DELETE CASCADE,
    sort_index INTEGER NOT NULL DEFAULT 0,
    rank_score REAL,
    best_source TEXT,
    source_count INTEGER,
    edge_count INTEGER,
    relation_count INTEGER,
    source_subject_count INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (source_title_id, discovery_title_id)
);
CREATE TABLE title_recommendation_cards (
    discovery_title_id TEXT PRIMARY KEY NOT NULL,
    payload_version INTEGER NOT NULL DEFAULT 1,
    payload_blob BLOB,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE title_recycle_purges (
    title_id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL,
    recorded_at TEXT NOT NULL
);
CREATE TABLE title_search_collation_keys (
    term_id INTEGER NOT NULL REFERENCES title_search_terms(term_id) ON DELETE CASCADE,
    profile TEXT NOT NULL,
    collation_key BLOB NOT NULL,
    PRIMARY KEY (term_id, profile)
);
CREATE TABLE title_search_index_queue (
    seq INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    title_id TEXT NOT NULL
);
CREATE TABLE title_search_meta (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    collation_version TEXT NOT NULL DEFAULT '',
    projection_generation INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE title_search_terms (
    term_id INTEGER PRIMARY KEY,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    facet TEXT NOT NULL,
    term_kind TEXT NOT NULL,
    raw_term TEXT NOT NULL,
    normalized_term TEXT NOT NULL,
    weight INTEGER NOT NULL
, literal_term TEXT NOT NULL DEFAULT '', stripped_year_key TEXT NOT NULL DEFAULT '', match_term TEXT NOT NULL DEFAULT '', match_year INTEGER, script TEXT NOT NULL DEFAULT 'other', numbers_key TEXT NOT NULL DEFAULT '', char_length INTEGER NOT NULL DEFAULT 0, romanization_key TEXT, language_tag TEXT, title_year INTEGER);
CREATE TABLE title_tag_definitions (
    id TEXT PRIMARY KEY NOT NULL,
    label TEXT NOT NULL,
    description TEXT,
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE titles(
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    name_normalized TEXT NOT NULL DEFAULT '',
    facet TEXT NOT NULL,
    monitored INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active',
    tags TEXT NOT NULL DEFAULT '[]',
    external_ids TEXT NOT NULL DEFAULT '[]',
    created_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT,
    deleted_at TEXT
, year INTEGER, overview TEXT, poster_url TEXT, sort_title TEXT, slug TEXT, imdb_id TEXT, runtime_minutes INTEGER, genres TEXT NOT NULL DEFAULT '[]', content_status TEXT, language TEXT, first_aired TEXT, network TEXT, studio TEXT, country TEXT, aliases TEXT NOT NULL DEFAULT '[]', metadata_language TEXT, metadata_fetched_at TEXT, min_availability TEXT, digital_release_date TEXT, background_url TEXT, folder_path TEXT, tagged_aliases_json TEXT DEFAULT '[]', poster_local_path TEXT, background_local_path TEXT, metadata_hydration_next_attempt_at TEXT, metadata_hydration_attempt_count INTEGER NOT NULL DEFAULT 0, library_id TEXT, root_folder_id TEXT, catalog_sort_key TEXT NOT NULL DEFAULT '', popularity REAL, smg_identity_backfill_attempt_count INTEGER NOT NULL DEFAULT 0, movie_release_dates_json TEXT, catalog_name_tie_key TEXT NOT NULL DEFAULT '', folder_path_key TEXT, folder_conflict_reason TEXT, folder_conflict_path TEXT);
CREATE TABLE totp_credentials (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL UNIQUE,
    secret_base32 TEXT NOT NULL,
    algorithm TEXT NOT NULL,
    digits INTEGER NOT NULL,
    period_seconds INTEGER NOT NULL,
    last_accepted_step INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_used_at TEXT, attempt_window_started_at TEXT, attempt_count INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (algorithm IN ('SHA1', 'SHA256', 'SHA512')),
    CHECK (digits IN (6, 8)),
    CHECK (period_seconds > 0)
);
CREATE TABLE totp_enrollment_challenges (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    secret_base32 TEXT NOT NULL,
    algorithm TEXT NOT NULL,
    digits INTEGER NOT NULL,
    period_seconds INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL, auth_session_version TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (algorithm IN ('SHA1', 'SHA256', 'SHA512')),
    CHECK (digits IN (6, 8)),
    CHECK (period_seconds > 0)
);
CREATE TABLE totp_failed_attempts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    attempted_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE totp_recovery_codes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    used_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE upstream_destination_cooldowns (
    destination_key TEXT PRIMARY KEY,
    cooldown_until TEXT NOT NULL,
    retry_after_seconds INTEGER,
    source TEXT NOT NULL,
    status_code INTEGER,
    message TEXT,
    observed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE TABLE upstream_scheduler_rss_cadence (
    host_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    account_quota_key TEXT NOT NULL,
    rss_request_key TEXT NOT NULL DEFAULT '',
    last_successful_poll_at TEXT,
    last_attempt_at TEXT,
    target_interval_seconds INTEGER NOT NULL,
    latest_safe_poll_at TEXT,
    estimated_feed_depth INTEGER,
    freshness_risk REAL NOT NULL DEFAULT 0,
    destination_recent_activity_at TEXT,
    last_seen_release_identity TEXT,
    last_seen_release_published_at TEXT,
    last_feed_gap_start_at TEXT,
    last_feed_gap_end_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (host_key, destination_key, account_quota_key, rss_request_key)
);
CREATE TABLE upstream_scheduler_states (
    host_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    account_quota_key TEXT NOT NULL DEFAULT '',
    rss_request_key TEXT NOT NULL DEFAULT '',
    api_current INTEGER,
    api_max INTEGER,
    grab_current INTEGER,
    grab_max INTEGER,
    quota_observed_at TEXT,
    quota_probe_after TEXT,
    quota_reset_at TEXT,
    quota_source TEXT,
    last_decision TEXT,
    last_feedback_at TEXT,
    last_successful_at TEXT,
    last_attempt_at TEXT,
    admitted_count INTEGER NOT NULL DEFAULT 0,
    deferred_count INTEGER NOT NULL DEFAULT 0,
    skipped_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (host_key, destination_key, account_quota_key, rss_request_key)
);
CREATE TABLE user_app_permission_masks (
    user_id TEXT NOT NULL,
    permission_mask INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE "user_external_accounts" (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    external_user_id TEXT,
    username TEXT NOT NULL,
    display_name TEXT,
    avatar_url TEXT,
    status TEXT NOT NULL,
    verified_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_login_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id),
    CHECK (provider IN ('plex', 'jellyfin', 'emby')),
    CHECK (status IN ('pending_claim', 'active', 'disabled'))
);
CREATE TABLE user_library_permission_masks (
    user_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    permission_mask INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, library_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);
CREATE TABLE user_list_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    username TEXT NOT NULL,
    display_name TEXT,
    credential_encrypted TEXT NOT NULL,
    -- 'active' | 'expired' | 'revoked'
    status TEXT NOT NULL DEFAULT 'active',
    error_message TEXT,
    linked_at TEXT NOT NULL,
    last_used_at TEXT,
    last_refresh_at TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE user_list_policies (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- 'auto' | 'approval' | 'none'
    policy TEXT NOT NULL,
    updated_by_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE user_ui_catalog_views (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_class TEXT NOT NULL,
    facet TEXT NOT NULL,
    view_mode TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, device_class, facet)
);
CREATE TABLE user_ui_dashboard_layout_items (
    user_id TEXT NOT NULL,
    breakpoint TEXT NOT NULL,
    panel_id TEXT NOT NULL,
    x INTEGER NOT NULL,
    y INTEGER NOT NULL,
    w INTEGER NOT NULL,
    h INTEGER NOT NULL,
    PRIMARY KEY (user_id, breakpoint, panel_id),
    FOREIGN KEY (user_id, breakpoint)
        REFERENCES user_ui_dashboard_layouts(user_id, breakpoint) ON DELETE CASCADE
);
CREATE TABLE user_ui_dashboard_layouts (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    breakpoint TEXT NOT NULL,
    column_count INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, breakpoint)
);
CREATE TABLE user_ui_settings (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    theme TEXT NOT NULL DEFAULT 'dark',
    date_time_format TEXT NOT NULL DEFAULT 'locale',
    highlight_color TEXT,
    secondary_color TEXT,
    high_contrast_mode INTEGER NOT NULL DEFAULT 0,
    reduce_motion INTEGER NOT NULL DEFAULT 0,
    hide_sponsor_button INTEGER NOT NULL DEFAULT 0,
    density TEXT NOT NULL DEFAULT 'comfortable',
    sidebar_mode TEXT NOT NULL DEFAULT 'expanded',
    default_landing_view TEXT NOT NULL DEFAULT 'movies',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
, palette TEXT NOT NULL DEFAULT 'CLASSIC'
    CHECK (palette IN ('CLASSIC', 'SCRYER_DEFAULT', 'SLATE', 'OCEAN', 'FOREST', 'PLUM', 'EMBER', 'GRAPHITE', 'PAPYRUS')), discovery_grid_hide_owned INTEGER NOT NULL DEFAULT 1, discovery_grid_hide_requested INTEGER NOT NULL DEFAULT 0);
CREATE TABLE "user_ui_table_columns" (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_class TEXT NOT NULL DEFAULT 'desktop',
    facet TEXT NOT NULL,
    table_view_mode TEXT NOT NULL,
    column_id TEXT NOT NULL,
    column_order INTEGER NOT NULL,
    visible INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, device_class, facet, table_view_mode, column_id)
);
CREATE TABLE "users" (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    password_hash TEXT,
    passkey_public_key TEXT,
    locale TEXT,
    created_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    last_login_at TEXT
, account_kind TEXT NOT NULL DEFAULT 'local'
        CHECK (account_kind IN ('local', 'external_auto_provisioned')), auth_session_version TEXT, password_change_required INTEGER NOT NULL DEFAULT 0
    CHECK (password_change_required IN (0, 1)));
CREATE TABLE wanted_items (
    id              TEXT PRIMARY KEY,
    title_id        TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    episode_id      TEXT REFERENCES episodes(id) ON DELETE CASCADE,
    media_type      TEXT NOT NULL,
    last_search_at  TEXT,
    status          TEXT NOT NULL DEFAULT 'wanted',
    grabbed_release TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL, collection_id TEXT REFERENCES collections(id), series_movie_link_id TEXT REFERENCES series_movie_links(id),
    UNIQUE(title_id, episode_id)
);
CREATE TABLE webauthn_challenges (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    challenge_type TEXT NOT NULL,
    state_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL, purpose TEXT NOT NULL DEFAULT 'standalone_authentication', login_verification_challenge_id TEXT
    REFERENCES login_verification_challenges(id) ON DELETE CASCADE, auth_session_version TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (challenge_type IN ('registration', 'authentication'))
);
CREATE TABLE webauthn_credentials (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    credential_id TEXT NOT NULL,
    credential_json TEXT NOT NULL,
    friendly_name TEXT,
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE workflow_operations(
    id TEXT PRIMARY KEY,
    operation_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    actor_user_id TEXT,
    title_id TEXT,
    collection_id TEXT,
    episode_id TEXT,
    release_id TEXT,
    media_file_id TEXT,
    external_reference TEXT,
    progress_json TEXT,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    job_key TEXT,
    trigger_source TEXT,
    summary_json TEXT,
    summary_text TEXT,
    error_text TEXT, series_movie_link_id TEXT,
    FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL,
    FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE SET NULL
);
CREATE TABLE workflow_private_inputs (
    run_id TEXT NOT NULL PRIMARY KEY,
    actor_user_id TEXT NOT NULL,
    request_key TEXT NOT NULL,
    input_encrypted TEXT,
    UNIQUE (actor_user_id, request_key)
);
CREATE INDEX idx_collections_title
    ON collections (title_id, collection_type);
CREATE INDEX idx_episodes_title
    ON episodes (title_id, season_number);
CREATE INDEX idx_episodes_collection
    ON episodes (collection_id);
CREATE INDEX idx_file_episode_map_episode
    ON file_episode_map (episode_id);
CREATE UNIQUE INDEX idx_download_clients_name
    ON download_clients (name);
CREATE INDEX idx_history_events_title_time
    ON history_events (title_id, occurred_at DESC);
CREATE INDEX idx_history_events_type_time
    ON history_events (event_type, occurred_at DESC);
CREATE UNIQUE INDEX idx_notification_channels_name_type
    ON notification_channels (name, channel_type);
CREATE INDEX idx_titles_facet_monitored
    ON titles (facet, monitored);
CREATE INDEX idx_media_files_title_path
    ON media_files (title_id, file_path);
CREATE INDEX idx_quality_profiles_scope
    ON quality_profiles (scope, scope_id);
CREATE INDEX idx_quality_profile_quality_tiers_profile
    ON quality_profile_quality_tiers (profile_id, sort_order);
CREATE INDEX idx_quality_profile_source_allowlist_profile
    ON quality_profile_source_allowlist (profile_id);
CREATE INDEX idx_quality_profile_source_blocklist_profile
    ON quality_profile_source_blocklist (profile_id);
CREATE INDEX idx_quality_profile_video_codec_allowlist_profile
    ON quality_profile_video_codec_allowlist (profile_id);
CREATE INDEX idx_quality_profile_video_codec_blocklist_profile
    ON quality_profile_video_codec_blocklist (profile_id);
CREATE INDEX idx_quality_profile_audio_codec_allowlist_profile
    ON quality_profile_audio_codec_allowlist (profile_id);
CREATE INDEX idx_quality_profile_audio_codec_blocklist_profile
    ON quality_profile_audio_codec_blocklist (profile_id);
CREATE INDEX idx_download_clients_client_priority
    ON download_clients (client_priority);
CREATE UNIQUE INDEX idx_setting_values_scope_name
    ON settings_values(setting_definition_id, scope, COALESCE(scope_id, ''));
CREATE INDEX idx_settings_values_definition
    ON settings_values(setting_definition_id);
CREATE INDEX idx_release_download_attempts_outcome_attempted
    ON release_download_attempts (outcome, attempted_at DESC);
CREATE INDEX idx_release_download_attempts_source_hint
    ON release_download_attempts (source_hint);
CREATE INDEX idx_release_download_attempts_source_title
    ON release_download_attempts (source_title);
CREATE INDEX idx_pending_releases_status ON pending_releases(status);
CREATE INDEX idx_pending_releases_wanted ON pending_releases(wanted_item_id, status);
CREATE INDEX idx_blocklist_title_id
    ON blocklist (title_id);
CREATE INDEX idx_subtitle_downloads_media_file ON subtitle_downloads(media_file_id);
CREATE INDEX idx_subtitle_downloads_title ON subtitle_downloads(title_id);
CREATE INDEX idx_subtitle_downloads_language ON subtitle_downloads(language);
CREATE UNIQUE INDEX idx_rule_sets_managed_key ON rule_sets(managed_key) WHERE managed_key IS NOT NULL;
CREATE INDEX idx_library_probe_signatures_last_probed
    ON library_probe_signatures (last_probed_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_facet_updated
    ON library_scan_unmatched_items (facet, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_root_updated
    ON library_scan_unmatched_items (facet, scan_root, updated_at DESC);
CREATE INDEX idx_history_events_occurred_at
    ON history_events (occurred_at DESC);
CREATE INDEX idx_rule_set_history_created_at
    ON rule_set_history (created_at DESC);
CREATE INDEX idx_title_external_ids_title_id
    ON title_external_ids(title_id);
CREATE INDEX idx_titles_metadata_hydration_due
    ON titles(metadata_hydration_next_attempt_at, metadata_fetched_at);
CREATE INDEX idx_titles_facet_normalized_slug
ON titles (facet, LOWER(TRIM(slug)))
WHERE slug IS NOT NULL AND TRIM(slug) <> '';
CREATE INDEX idx_subtitle_provider_configs_provider_type
    ON subtitle_provider_configs(provider_type);
CREATE INDEX idx_subtitle_provider_configs_enabled
    ON subtitle_provider_configs(is_enabled);
CREATE INDEX idx_subtitle_provider_configs_disabled_until
    ON subtitle_provider_configs(disabled_until);
CREATE UNIQUE INDEX idx_collection_external_ids_unique
    ON collection_external_ids(collection_id, source, external_id, provenance, source_scope);
CREATE UNIQUE INDEX idx_episode_external_ids_unique
    ON episode_external_ids(episode_id, source, external_id, provenance, source_scope);
CREATE INDEX idx_collection_external_ids_title_provenance
    ON collection_external_ids(title_id, provenance);
CREATE INDEX idx_episode_external_ids_title_provenance
    ON episode_external_ids(title_id, provenance);
CREATE INDEX idx_library_scan_unmatched_items_facet_status_updated
    ON library_scan_unmatched_items (facet, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_root_status_updated
    ON library_scan_unmatched_items (facet, scan_root, status, updated_at DESC);
CREATE INDEX idx_title_search_terms_facet_normalized_term
    ON title_search_terms(facet, normalized_term);
CREATE INDEX idx_title_search_terms_normalized_term
    ON title_search_terms(normalized_term);
CREATE INDEX idx_library_scan_unmatched_items_facet_title_status_updated
    ON library_scan_unmatched_items (facet, title_id, status, updated_at DESC);
CREATE INDEX idx_external_subtitle_probe_cache_file_path
    ON external_subtitle_probe_cache(file_path);
CREATE INDEX idx_plugin_catalog_sources_kind
    ON plugin_catalog_sources(source_kind);
CREATE UNIQUE INDEX idx_libraries_facet_slug
    ON libraries(facet, slug);
CREATE UNIQUE INDEX idx_library_roots_normalized_path
    ON library_roots(normalized_path);
CREATE INDEX idx_library_roots_library
    ON library_roots(library_id, is_default DESC, path ASC);
CREATE INDEX idx_titles_library_name
    ON titles(library_id, LOWER(name), id);
CREATE INDEX idx_library_scan_unmatched_items_library_updated
    ON library_scan_unmatched_items(library_id, updated_at DESC);
CREATE UNIQUE INDEX idx_library_scan_unmatched_items_library_path
    ON library_scan_unmatched_items(library_id, item_path);
CREATE INDEX idx_indexers_managed_parent ON indexers(managed_parent_config_id);
CREATE UNIQUE INDEX idx_indexers_managed_child_identity
ON indexers(managed_parent_config_id, managed_child_key)
WHERE managed_parent_config_id IS NOT NULL AND managed_child_key IS NOT NULL;
CREATE INDEX idx_workflow_operations_job_key_started
    ON workflow_operations (job_key, started_at DESC);
CREATE INDEX idx_workflow_operations_status_started
    ON workflow_operations (status, started_at);
CREATE UNIQUE INDEX idx_webauthn_credentials_credential_id
    ON webauthn_credentials (credential_id);
CREATE INDEX idx_webauthn_credentials_user_id_created_at
    ON webauthn_credentials (user_id, created_at DESC);
CREATE INDEX idx_webauthn_challenges_expires_at
    ON webauthn_challenges (expires_at);
CREATE INDEX idx_webauthn_challenges_user_id
    ON webauthn_challenges (user_id);
CREATE INDEX idx_media_server_connections_provider
    ON media_server_connections (provider, enabled);
CREATE INDEX idx_media_server_path_mappings_connection
    ON media_server_path_mappings (connection_id, sort_order);
CREATE INDEX idx_media_request_external_ids_lookup
    ON media_request_external_ids (library_id, source, external_id);
CREATE INDEX idx_media_request_requesters_user
    ON media_request_requesters (user_id);
CREATE INDEX idx_totp_enrollment_challenges_expires_at
    ON totp_enrollment_challenges (expires_at);
CREATE INDEX idx_totp_enrollment_challenges_user_id
    ON totp_enrollment_challenges (user_id);
CREATE INDEX idx_totp_recovery_codes_user_id
    ON totp_recovery_codes (user_id, used_at);
CREATE INDEX idx_totp_failed_attempts_user_id_attempted_at
    ON totp_failed_attempts (user_id, attempted_at);
CREATE UNIQUE INDEX idx_notification_subscriptions_target_scope
    ON notification_subscriptions (
        target_kind,
        target_id,
        event_type,
        COALESCE(scope, ''),
        COALESCE(scope_id, '')
    );
CREATE INDEX idx_notification_subscriptions_channel
    ON notification_subscriptions (channel_id);
CREATE INDEX idx_notification_subscriptions_target
    ON notification_subscriptions (target_kind, target_id);
CREATE INDEX idx_movie_entities_tvdb_id
    ON movie_entities(tvdb_id)
    WHERE tvdb_id IS NOT NULL AND tvdb_id <> '';
CREATE INDEX idx_movie_entities_tmdb_id
    ON movie_entities(tmdb_id)
    WHERE tmdb_id IS NOT NULL AND tmdb_id <> '';
CREATE INDEX idx_movie_entities_imdb_id
    ON movie_entities(imdb_id)
    WHERE imdb_id IS NOT NULL AND imdb_id <> '';
CREATE INDEX idx_movie_entities_mal_id
    ON movie_entities(mal_id)
    WHERE mal_id IS NOT NULL AND mal_id <> '';
CREATE INDEX idx_movie_entities_anidb_id
    ON movie_entities(anidb_id)
    WHERE anidb_id IS NOT NULL AND anidb_id <> '';
CREATE UNIQUE INDEX idx_series_movie_links_legacy_collection
    ON series_movie_links(legacy_collection_id)
    WHERE legacy_collection_id IS NOT NULL;
CREATE INDEX idx_series_movie_links_title
    ON series_movie_links(series_title_id);
CREATE INDEX idx_series_movie_links_movie
    ON series_movie_links(movie_entity_id);
CREATE INDEX idx_file_series_movie_link_map_link
    ON file_series_movie_link_map(series_movie_link_id);
CREATE UNIQUE INDEX idx_wanted_items_series_movie_link
    ON wanted_items(series_movie_link_id)
    WHERE series_movie_link_id IS NOT NULL;
CREATE UNIQUE INDEX idx_wanted_items_movie_unique
    ON wanted_items(title_id)
    WHERE episode_id IS NULL
      AND collection_id IS NULL
      AND series_movie_link_id IS NULL;
CREATE INDEX idx_oauth_authorization_codes_user_id
    ON oauth_authorization_codes(user_id);
CREATE INDEX idx_oauth_authorization_codes_expires_at
    ON oauth_authorization_codes(expires_at);
CREATE INDEX idx_oauth_refresh_grants_user_id
    ON oauth_refresh_grants(user_id);
CREATE INDEX idx_oauth_refresh_grants_family_id
    ON oauth_refresh_grants(family_id);
CREATE INDEX idx_oauth_refresh_tokens_grant_id
    ON oauth_refresh_tokens(grant_id);
CREATE INDEX idx_oauth_refresh_tokens_family_id
    ON oauth_refresh_tokens(family_id);
CREATE INDEX idx_titles_root_folder_id
    ON titles(root_folder_id);
CREATE INDEX idx_oauth_refresh_grants_authorization_source
    ON oauth_refresh_grants(authorization_source);
CREATE INDEX idx_indexer_system_backoffs_disabled_until
    ON indexer_system_backoffs(disabled_until);
CREATE INDEX idx_title_image_variants_blob_digest
             ON title_image_variants(blob_digest);
CREATE INDEX idx_discovery_submitted_subjects_run_key
    ON discovery_submitted_subjects (run_id, subject_key, library_id, title_id);
CREATE INDEX idx_discovery_sync_runs_kind_status
    ON discovery_sync_runs(kind, status, updated_at);
CREATE INDEX idx_discovery_submitted_subjects_title
    ON discovery_submitted_subjects(title_id);
CREATE INDEX idx_discovery_pending_changes_scope_seen
    ON discovery_pending_context_changes(scope_key, last_seen_at);
CREATE INDEX idx_discovery_pending_changes_scope_sequence
    ON discovery_pending_context_changes(scope_key, last_seen_sequence);
CREATE INDEX idx_discovery_sections_run_surface
    ON discovery_sections(run_id, surface, sort_index);
CREATE INDEX idx_title_metadata_tags_category_name
    ON title_metadata_tags(category, name, title_id);
CREATE INDEX idx_discovery_title_metadata_tags_category_name
    ON discovery_title_metadata_tags(category, name, discovery_title_id);
CREATE INDEX idx_discovery_title_terms_kind_value
    ON discovery_title_terms(term_kind, term_value, discovery_title_id);
CREATE INDEX idx_discovery_title_terms_title
    ON discovery_title_terms(discovery_title_id, term_kind, sort_index);
CREATE INDEX idx_discovery_title_source_tag_values_title
    ON discovery_title_source_tag_values(discovery_title_id, source_tag_sort_index, value_sort_index);
CREATE INDEX idx_discovery_title_external_ids_title
    ON discovery_title_external_ids(discovery_title_id, sort_index);
CREATE INDEX idx_discovery_items_active_title
    ON discovery_items(base_generation_id, discovery_title_id, tombstoned_at);
CREATE INDEX idx_discovery_items_section
    ON discovery_items(section_id, sort_index, rank_score);
CREATE INDEX idx_discovery_items_run_section
    ON discovery_items(run_id, section_id, sort_index);
CREATE INDEX idx_discovery_section_items_run_section
    ON discovery_section_items(run_id, section_id, sort_index);
CREATE INDEX idx_discovery_item_subject_links_run_type_key
    ON discovery_item_subject_links(run_id, link_type, subject_key, item_id);
CREATE INDEX idx_discovery_item_subject_links_item
    ON discovery_item_subject_links(item_id, link_type, sort_index);
CREATE INDEX idx_discovery_item_library_provenance_library
    ON discovery_item_library_provenance(run_id, library_id, item_id);
CREATE INDEX idx_discovery_item_library_provenance_item
    ON discovery_item_library_provenance(item_id, subject_key, library_id, title_id);
CREATE INDEX idx_upstream_scheduler_states_destination
    ON upstream_scheduler_states (destination_key);
CREATE INDEX idx_upstream_destination_cooldowns_until
    ON upstream_destination_cooldowns (cooldown_until);
CREATE INDEX idx_upstream_scheduler_rss_latest_safe_poll
    ON upstream_scheduler_rss_cadence (latest_safe_poll_at);
CREATE INDEX idx_titles_popularity
    ON titles(popularity);
CREATE INDEX idx_workflow_operations_job_recent_started
    ON workflow_operations (started_at DESC)
    WHERE job_key IS NOT NULL;
CREATE INDEX idx_workflow_operations_active_job_started
    ON workflow_operations (started_at ASC)
    WHERE job_key IS NOT NULL
      AND status IN ('queued', 'running', 'discovering');
CREATE INDEX idx_discovery_title_metadata_rating_sources_order
    ON discovery_title_metadata_rating_sources(discovery_title_id, sort_index ASC, source ASC);
CREATE INDEX idx_discovery_title_metadata_external_ratings_order
    ON discovery_title_metadata_external_ratings(discovery_title_id, sort_index ASC, source ASC);
CREATE INDEX idx_discovery_title_metadata_external_ratings_source_norm
    ON discovery_title_metadata_external_ratings(source, normalized, discovery_title_id);
CREATE INDEX idx_scope_indexer_coverage_indexer
    ON scope_indexer_coverage(indexer_id);
CREATE INDEX idx_scope_indexer_coverage_searched_at
    ON scope_indexer_coverage(searched_at);
CREATE INDEX idx_discovery_items_generation_rank
    ON discovery_items(base_generation_id, tombstoned_at, owned_in_input);
CREATE INDEX idx_image_proxy_sources_last_seen_at
  ON image_proxy_sources(last_seen_at);
CREATE INDEX idx_image_proxy_cache_entries_last_accessed_at
  ON image_proxy_cache_entries(last_accessed_at);
CREATE INDEX idx_manual_import_selections_source
    ON manual_import_selections (source_client_id, source_system, source_ref);
CREATE INDEX idx_manual_import_selections_owner
    ON manual_import_selections (actor_user_id, title_id, source_client_id, source_system, source_ref);
CREATE INDEX idx_indexers_download_client_id
    ON indexers(download_client_id);
CREATE INDEX idx_pending_releases_indexer_id
    ON pending_releases(indexer_id);
CREATE UNIQUE INDEX idx_user_external_accounts_pending_username
    ON user_external_accounts (provider, connection_id, LOWER(username))
    WHERE status = 'pending_claim' AND external_user_id IS NULL;
CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity
    ON user_external_accounts (provider, connection_id, external_user_id);
CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection
    ON user_external_accounts (user_id, provider, connection_id);
CREATE INDEX idx_user_external_accounts_user_status
    ON user_external_accounts (user_id, status);
CREATE UNIQUE INDEX idx_file_episode_map_one_primary_per_episode
ON file_episode_map (episode_id)
WHERE role = 'primary';
CREATE UNIQUE INDEX idx_seeding_profiles_name
    ON seeding_profiles(LOWER(name));
CREATE INDEX idx_indexers_seeding_profile_id
    ON indexers(seeding_profile_id);
CREATE INDEX idx_login_verification_challenges_user_id
    ON login_verification_challenges (user_id);
CREATE INDEX idx_login_verification_challenges_expires_at
    ON login_verification_challenges (expires_at);
CREATE INDEX idx_webauthn_challenges_login_verification
    ON webauthn_challenges (login_verification_challenge_id);
CREATE INDEX idx_indexer_errors_indexer_occurred_at_id
    ON indexer_errors (indexer_id, occurred_at DESC, id DESC);
CREATE INDEX idx_indexer_errors_occurred_at
    ON indexer_errors (occurred_at);
CREATE INDEX idx_oauth_client_registrations_enabled
    ON oauth_client_registrations(enabled);
CREATE INDEX idx_api_keys_user_created
    ON api_keys(user_id, created_at DESC);
CREATE INDEX idx_download_client_bindings_locator
    ON download_client_bindings(client_config_id, client_type_snapshot, native_item_id);
CREATE INDEX idx_download_submissions_title_request_signature
    ON download_submissions(title_id, request_signature);
CREATE INDEX idx_download_submissions_download_id
    ON download_submissions(download_client_id, download_client_type, download_id);
CREATE INDEX idx_download_submissions_seed_info_hash
    ON download_submissions(seed_info_hash);
CREATE INDEX idx_download_submission_episode_links_episode
    ON download_submission_episode_links(episode_id);
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
CREATE INDEX idx_download_import_artifacts_episode
    ON download_import_artifacts (episode_id, result);
CREATE INDEX idx_download_import_artifacts_retention
    ON download_import_artifacts (created_at, import_id);
CREATE INDEX idx_download_import_artifacts_source
    ON download_import_artifacts (COALESCE(source_client_id, ''), source_system, source_ref, created_at);
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
CREATE INDEX idx_titles_movie_smg_identity_backfill_candidates
    ON titles(facet, smg_identity_backfill_attempt_count, id);
CREATE INDEX idx_media_server_playback_items_entity
    ON media_server_playback_items (entity_kind, entity_id);
CREATE INDEX idx_manual_import_selections_canonical_download
    ON manual_import_selections (canonical_download_id, actor_user_id, title_id, updated_at DESC);
CREATE INDEX idx_indexer_search_runs_scope_created
    ON indexer_search_runs(scope_key, created_at DESC);
CREATE INDEX idx_indexer_search_runs_indexer_created
    ON indexer_search_runs(indexer_id, created_at DESC);
CREATE INDEX idx_indexer_search_candidates_expiry
    ON indexer_search_candidates(expires_at);
CREATE INDEX idx_indexer_search_sources_indexer_reusable
    ON indexer_search_candidate_sources(indexer_id, reusable_until);
CREATE INDEX idx_indexer_search_run_sources_session
    ON indexer_search_run_candidate_sources(search_session_id);
CREATE UNIQUE INDEX idx_pending_releases_active_release_identity
    ON pending_releases(release_identity)
    WHERE status IN ('waiting', 'standby', 'processing', 'needs_review');
CREATE INDEX idx_pending_releases_active_coverage
    ON pending_releases(coverage_identity, status, published_at, added_at);
CREATE INDEX idx_pending_releases_active_unknown_age
    ON pending_releases(release_age_unknown, status, indexer_id, added_at)
    WHERE release_age_unknown = 1;
CREATE INDEX idx_download_identity_states_download_id
    ON download_identity_states(client_id, client_type, download_id);
CREATE UNIQUE INDEX idx_title_metadata_rating_summaries_title_owner
    ON title_metadata_rating_summaries(title_id)
    WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_rating_summaries_movie_owner
    ON title_metadata_rating_summaries(movie_entity_id)
    WHERE movie_entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_rating_sources_title_owner
    ON title_metadata_rating_sources(title_id, source)
    WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_rating_sources_movie_owner
    ON title_metadata_rating_sources(movie_entity_id, source)
    WHERE movie_entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_external_ratings_title_owner
    ON title_metadata_external_ratings(title_id, source)
    WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_external_ratings_movie_owner
    ON title_metadata_external_ratings(movie_entity_id, source)
    WHERE movie_entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_credits_title_owner
    ON title_credits(title_id, position)
    WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_credits_movie_owner
    ON title_credits(movie_entity_id, position)
    WHERE movie_entity_id IS NOT NULL;
CREATE INDEX idx_title_metadata_rating_sources_order
    ON title_metadata_rating_sources(title_id, sort_index ASC, source ASC);
CREATE INDEX idx_movie_entity_metadata_rating_sources_order
    ON title_metadata_rating_sources(movie_entity_id, sort_index ASC, source ASC);
CREATE INDEX idx_title_metadata_external_ratings_order
    ON title_metadata_external_ratings(title_id, sort_index ASC, source ASC);
CREATE INDEX idx_movie_entity_metadata_external_ratings_order
    ON title_metadata_external_ratings(movie_entity_id, sort_index ASC, source ASC);
CREATE INDEX idx_title_metadata_external_ratings_source_norm
    ON title_metadata_external_ratings(source, normalized, title_id);
CREATE INDEX idx_movie_entity_metadata_external_ratings_source_norm
    ON title_metadata_external_ratings(source, normalized, movie_entity_id);
CREATE INDEX idx_title_credits_title_kind
    ON title_credits(title_id, kind);
CREATE INDEX idx_title_credits_movie_kind
    ON title_credits(movie_entity_id, kind);
CREATE UNIQUE INDEX idx_blocklist_release_unique
    ON blocklist (title_id, indexer_id, normalized_release_name)
    WHERE info_hash IS NULL;
CREATE UNIQUE INDEX idx_blocklist_info_hash_unique
    ON blocklist (title_id, info_hash)
    WHERE info_hash IS NOT NULL;
CREATE INDEX idx_domain_events_event_type_sequence ON domain_events (event_type, sequence DESC);
CREATE INDEX idx_release_decisions_wanted ON release_decisions (wanted_item_id, created_at DESC);
CREATE INDEX idx_release_decisions_created_at ON release_decisions (created_at DESC);
CREATE INDEX idx_workflow_operations_actor_job_started
    ON workflow_operations (actor_user_id, job_key, started_at DESC)
    WHERE job_key IS NOT NULL
      AND actor_user_id IS NOT NULL;
CREATE INDEX idx_workflow_operations_actor_recent_started
    ON workflow_operations (actor_user_id, started_at DESC)
    WHERE job_key IS NOT NULL
      AND actor_user_id IS NOT NULL;
CREATE INDEX idx_domain_events_title_sequence
    ON domain_events (title_id, sequence DESC)
    WHERE title_id IS NOT NULL;
CREATE INDEX idx_title_more_like_this_items_source_order
    ON title_more_like_this_items(source_title_id, sort_index ASC, rank_score DESC);
CREATE INDEX idx_title_more_like_this_items_title
    ON title_more_like_this_items(discovery_title_id);
CREATE UNIQUE INDEX totp_enrollment_challenges_one_active_per_user
    ON totp_enrollment_challenges (user_id);
CREATE INDEX idx_indexer_search_run_sources_source
    ON indexer_search_run_candidate_sources(source_id);
CREATE INDEX idx_media_requests_library_facet_status
    ON media_requests (library_id, facet, status);
CREATE INDEX idx_media_requests_status_updated
    ON media_requests (status, updated_at);
CREATE INDEX idx_media_requests_created_title
    ON media_requests (created_title_id);
CREATE INDEX idx_pp_script_runs_script_id ON post_processing_script_runs(script_id, started_at DESC);
CREATE INDEX idx_pp_script_runs_title_id ON post_processing_script_runs(title_id, started_at DESC);
CREATE INDEX idx_download_cleanup_due ON download_cleanup(status, next_attempt_at, client_id);
CREATE INDEX idx_maintenance_rule_set_libraries_library
    ON maintenance_rule_set_libraries(library_id);
CREATE INDEX idx_maintenance_rule_revisions_rule_set
    ON maintenance_rule_revisions(rule_set_id, revision_number DESC);
CREATE INDEX idx_maintenance_rule_exclusions_title
    ON maintenance_rule_exclusions(title_id);
CREATE INDEX idx_maintenance_evaluation_runs_rule_set
    ON maintenance_evaluation_runs(rule_set_id, started_at DESC);
CREATE INDEX idx_lifecycle_candidates_rule_state
    ON lifecycle_candidates(rule_set_id, state);
CREATE INDEX idx_lifecycle_candidates_title
    ON lifecycle_candidates(title_id);
CREATE INDEX idx_lifecycle_action_runs_rule_set
    ON lifecycle_action_runs(rule_set_id, started_at DESC);
CREATE INDEX idx_lifecycle_action_runs_candidate
    ON lifecycle_action_runs(candidate_id);
CREATE UNIQUE INDEX idx_media_server_signals_participant_item
    ON media_server_user_media_signals(connection_id, external_user_id, provider_item_id);
CREATE INDEX idx_media_server_signals_title
    ON media_server_user_media_signals(scryer_title_id);
CREATE INDEX idx_media_server_signals_episode
    ON media_server_user_media_signals(scryer_episode_id);
CREATE INDEX idx_library_root_id_remaps_root
    ON library_root_id_remaps(root_id);
CREATE INDEX idx_media_files_full_blake3
    ON media_files(full_blake3)
    WHERE full_blake3 IS NOT NULL;
CREATE INDEX idx_media_files_full_hash_missing
    ON media_files(id)
    WHERE full_blake3 IS NULL;
CREATE INDEX idx_location_operations_state
    ON location_operations(state, updated_at DESC);
CREATE INDEX idx_location_operations_source_root
    ON location_operations(source_root_id);
CREATE INDEX idx_location_operations_destination_root
    ON location_operations(destination_root_id);
CREATE INDEX idx_location_operation_title_checkpoints_order
    ON location_operation_title_checkpoints(operation_id, sequence ASC);
CREATE INDEX idx_location_operation_title_checkpoints_state
    ON location_operation_title_checkpoints(operation_id, state);
CREATE INDEX idx_location_operation_title_checkpoints_title
    ON location_operation_title_checkpoints(title_id);
CREATE UNIQUE INDEX idx_location_operation_verifications_destination
    ON location_operation_verifications(operation_id, destination_path);
CREATE INDEX idx_location_operation_verifications_title
    ON location_operation_verifications(operation_id, title_id);
CREATE INDEX idx_location_operation_verifications_media_file
    ON location_operation_verifications(media_file_id);
CREATE UNIQUE INDEX idx_location_operation_owned_entities_active
    ON location_operation_owned_entities(entity_type, entity_id)
    WHERE released_at IS NULL;
CREATE INDEX idx_location_operation_owned_entities_operation
    ON location_operation_owned_entities(operation_id, released_at);
CREATE INDEX idx_proxy_configs_provider_type
    ON proxy_configs(provider_type);
CREATE INDEX idx_indexers_proxy_config_id
    ON indexers(proxy_config_id);
CREATE UNIQUE INDEX idx_title_tag_definitions_label
    ON title_tag_definitions(label);
CREATE INDEX idx_request_rule_set_libraries_library
    ON request_rule_set_libraries(library_id);
CREATE INDEX idx_request_rule_revisions_rule_set
    ON request_rule_revisions(rule_set_id, revision_number DESC);
CREATE INDEX idx_request_rule_decisions_request
    ON request_rule_decisions(request_id, evaluated_at DESC);
CREATE UNIQUE INDEX idx_lifecycle_claims_live_producer
    ON lifecycle_claims(producer, producer_ref)
    WHERE state IN ('dormant', 'active') AND producer_ref IS NOT NULL;
CREATE INDEX idx_lifecycle_claims_title_state
    ON lifecycle_claims(title_id, state);
CREATE INDEX idx_rule_pack_members_pack_id ON rule_pack_members(pack_id);
CREATE UNIQUE INDEX idx_lifecycle_candidates_active_subject
    ON lifecycle_candidates(rule_set_id, subject_kind, subject_id)
    WHERE state NOT IN ('succeeded', 'failed', 'canceled', 'excluded');
CREATE INDEX idx_lifecycle_candidates_subject_generation
    ON lifecycle_candidates(rule_set_id, subject_kind, subject_id, match_generation);
CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_rule_subject
    ON maintenance_rule_exclusions(rule_set_id, subject_kind, subject_id)
    WHERE rule_set_id IS NOT NULL;
CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_global_subject
    ON maintenance_rule_exclusions(subject_kind, subject_id)
    WHERE rule_set_id IS NULL;
CREATE INDEX idx_media_files_analysis_refresh ON media_files(analysis_revision, analysis_attempted_at, id);
CREATE INDEX idx_maintenance_action_step_attempts_step
    ON maintenance_action_step_attempts(candidate_id, match_generation, revision_number, step_id, attempt, started_at DESC);
CREATE INDEX idx_maintenance_action_job_receipts_step
    ON maintenance_action_job_receipts(candidate_id, match_generation, revision_number, step_id, dispatch_attempt DESC);
CREATE UNIQUE INDEX idx_maintenance_action_job_receipts_accepted_request
    ON maintenance_action_job_receipts(candidate_id, match_generation, revision_number, step_id, request_hash)
    WHERE state IN ('accepted', 'completed');
CREATE UNIQUE INDEX idx_maintenance_sequence_terminal_memberships_active
    ON maintenance_sequence_terminal_memberships(rule_set_id, revision_number, subject_kind, subject_id)
    WHERE released_at IS NULL;
CREATE INDEX idx_maintenance_sequence_terminal_memberships_active_page
    ON maintenance_sequence_terminal_memberships(rule_set_id, revision_number, id)
    WHERE released_at IS NULL;
CREATE INDEX idx_location_transfer_titles_page ON location_transfer_titles(operation_id, hot_rank, sequence, title_id);
CREATE INDEX idx_discovery_items_generation_recommendation
    ON discovery_items(base_generation_id, recommendation_score DESC, rank_score DESC, sort_index ASC);
CREATE INDEX idx_download_client_status_disabled_until
    ON download_client_status(disabled_until);
CREATE INDEX idx_download_identity_states_canonical_latest
    ON download_identity_states(canonical_download_id, updated_at DESC, id DESC);
CREATE INDEX idx_download_identity_states_client_item
    ON download_identity_states(client_type, download_client_item_id, client_id);
CREATE INDEX idx_download_submissions_client_item
    ON download_submissions(download_client_type, download_client_item_id, download_client_id);
CREATE UNIQUE INDEX idx_title_external_ids_library_kind_lookup
    ON title_external_ids(library_id, source, external_kind, external_id);
CREATE INDEX idx_title_external_ids_library_source_value
    ON title_external_ids(library_id, source, external_id);
CREATE INDEX idx_media_files_id_role_path
    ON media_files(id, role, file_path);
CREATE INDEX idx_domain_events_stream_sequence
    ON domain_events (stream_id, sequence)
    WHERE stream_id IS NOT NULL;
CREATE INDEX idx_titles_created_at
    ON titles(created_at);
CREATE INDEX idx_titles_library_folder_path
    ON titles(library_id, folder_path);
CREATE INDEX idx_title_search_collation_keys_profile_key
    ON title_search_collation_keys(profile, collation_key);
CREATE INDEX idx_title_search_terms_bucket
    ON title_search_terms(facet, script, numbers_key, char_length);
CREATE INDEX idx_title_search_terms_facet_literal
    ON title_search_terms(facet, literal_term);
CREATE INDEX idx_title_search_terms_facet_match_term
    ON title_search_terms(facet, match_term);
CREATE INDEX idx_title_search_terms_facet_romanization
    ON title_search_terms(facet, romanization_key);
CREATE INDEX idx_title_search_terms_stripped_year_key
    ON title_search_terms(term_kind, stripped_year_key);
CREATE INDEX idx_download_import_artifacts_import_episode
ON download_import_artifacts (import_id, episode_id, result, created_at DESC);
CREATE INDEX idx_title_search_index_queue_title_id
    ON title_search_index_queue(title_id);
CREATE INDEX idx_titles_library_folder_path_lookup
    ON titles(library_id, lower(replace(folder_path, '/', '\')));
CREATE UNIQUE INDEX idx_wanted_items_collection_id
    ON wanted_items(collection_id)
    WHERE collection_id IS NOT NULL AND episode_id IS NULL;
CREATE INDEX idx_import_space_members_job ON import_space_members(job_key);
CREATE INDEX idx_import_space_members_destination ON import_space_members(destination_key);
CREATE UNIQUE INDEX idx_user_list_accounts_identity
    ON user_list_accounts(user_id, provider, external_user_id);
CREATE INDEX idx_list_subscriptions_owner ON list_subscriptions(owner_user_id);
CREATE INDEX idx_list_subscriptions_due ON list_subscriptions(enabled, next_sync_at);
CREATE INDEX idx_list_memberships_title ON list_memberships(title_id);
CREATE INDEX idx_list_memberships_request ON list_memberships(request_id);
CREATE INDEX idx_list_exclusions_subscription ON list_exclusions(subscription_id);
CREATE INDEX idx_list_exclusion_external_ids_lookup
    ON list_exclusion_external_ids(source, value);
CREATE INDEX idx_list_sync_runs_subscription
    ON list_sync_runs(subscription_id, started_at);
CREATE INDEX idx_workflow_operations_title
    ON workflow_operations (title_id);
CREATE INDEX idx_workflow_operations_collection
    ON workflow_operations (collection_id);
CREATE INDEX idx_workflow_operations_episode
    ON workflow_operations (episode_id);
CREATE INDEX idx_workflow_operations_media_file
    ON workflow_operations (media_file_id);
CREATE INDEX idx_discovery_pending_changes_title
    ON discovery_pending_context_changes (title_id);
CREATE INDEX idx_discovery_titles_resolved_title
    ON discovery_titles (resolved_title_id);
CREATE INDEX idx_download_import_artifacts_title
    ON download_import_artifacts (title_id);
CREATE INDEX idx_download_import_artifacts_imported_media_file
    ON download_import_artifacts (imported_media_file_id);
CREATE INDEX idx_release_download_attempts_title
    ON release_download_attempts (title_id);
CREATE INDEX idx_series_movie_links_linked_episode
    ON series_movie_links (linked_episode_id);
CREATE INDEX idx_wanted_items_episode
    ON wanted_items (episode_id);
CREATE INDEX idx_wanted_items_collection
    ON wanted_items (collection_id);
CREATE INDEX idx_user_ui_table_columns_user_view
    ON user_ui_table_columns(user_id, device_class, facet, table_view_mode, column_order);
CREATE INDEX idx_discovery_item_rank_components_run
    ON discovery_item_rank_components(run_id);
CREATE INDEX idx_discovery_items_generation_relevance
    ON discovery_items(
        base_generation_id,
        owned_in_input,
        COALESCE(recommendation_score, -999999999.0) DESC,
        COALESCE(rank_score, -999999999.0) DESC,
        sort_index ASC
    )
    WHERE tombstoned_at IS NULL;
CREATE INDEX idx_title_search_terms_kind_literal
    ON title_search_terms(term_kind, literal_term);
CREATE INDEX idx_downloads_created_at_id
    ON downloads(created_at, id);
CREATE INDEX idx_download_identity_states_updated_at
    ON download_identity_states(updated_at);
CREATE INDEX idx_download_submissions_tracked_state_at
    ON download_submissions(tracked_state_at);
CREATE INDEX idx_workflow_operations_job_operation_started
    ON workflow_operations (job_key, operation_type, started_at DESC);
CREATE INDEX idx_indexer_search_runs_coverage_scope
    ON indexer_search_runs(coverage_scope_key, indexer_id, created_at DESC);
CREATE UNIQUE INDEX idx_title_search_terms_title_kind_language
ON title_search_terms (title_id, term_kind, literal_term, COALESCE(language_tag, ''));
CREATE INDEX idx_delay_profile_tags_label ON delay_profile_tags(label);
CREATE INDEX idx_delay_profile_libraries_library_id
    ON delay_profile_libraries(library_id);
CREATE UNIQUE INDEX idx_location_operations_recovery_key ON location_operations(recovery_key);
CREATE INDEX idx_title_external_ids_source_value_nocase
    ON title_external_ids(source, LOWER(external_id));
CREATE UNIQUE INDEX idx_lifecycle_claims_live_operator_keep
    ON lifecycle_claims(title_id)
    WHERE producer = 'operator_keep' AND state IN ('dormant', 'active');
CREATE INDEX idx_indexer_search_listings_owner
    ON indexer_search_listings(indexer_id, owner_scope_key, owner_query_signature);
CREATE INDEX idx_indexer_search_listing_readers_listing
    ON indexer_search_listing_readers(indexer_id, indexer_fingerprint, title_id, facet, first_page_fingerprint);
CREATE INDEX idx_indexer_search_listing_runs_listing
    ON indexer_search_listing_runs(indexer_id, indexer_fingerprint, title_id, facet, first_page_fingerprint, created_at);
CREATE INDEX indexer_api_client_grants_source ON indexer_api_client_grants(indexer_id);
CREATE INDEX idx_discovery_neighbor_cache_fetched_at
    ON discovery_neighbor_cache (fetched_at);
CREATE INDEX idx_discovery_library_graph_runs_facet_status_started
    ON discovery_library_graph_runs (facet, status, started_at);
CREATE INDEX idx_media_server_audience_plays_title
    ON media_server_audience_plays(scryer_title_id);
CREATE INDEX idx_titles_catalog_order_asc ON titles (
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX idx_titles_catalog_order_desc ON titles (
    catalog_sort_key COLLATE BINARY DESC,
    catalog_name_tie_key COLLATE BINARY DESC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year DESC,
    id COLLATE BINARY DESC
);
CREATE INDEX idx_title_external_ids_value_nocase
    ON title_external_ids(LOWER(TRIM(external_id)));
CREATE INDEX idx_title_external_ids_numeric_suffix
    ON title_external_ids(
        CAST(SUBSTR(TRIM(external_id), LENGTH(RTRIM(TRIM(external_id), '0123456789')) + 1) AS INTEGER)
    );
CREATE INDEX idx_titles_imdb_id_nocase
    ON titles(LOWER(TRIM(imdb_id)))
    WHERE imdb_id IS NOT NULL;
CREATE INDEX idx_titles_imdb_id_numeric_suffix
    ON titles(
        CAST(SUBSTR(TRIM(imdb_id), LENGTH(RTRIM(TRIM(imdb_id), '0123456789')) + 1) AS INTEGER)
    )
    WHERE imdb_id IS NOT NULL;
CREATE INDEX idx_episodes_air_date
    ON episodes(air_date);
CREATE UNIQUE INDEX titles_folder_path_key_unique
    ON titles(folder_path_key)
    WHERE folder_path_key IS NOT NULL;
CREATE INDEX idx_domain_events_event_type_occurred_at
    ON domain_events(event_type, occurred_at);
CREATE INDEX idx_imports_type_status_updated_at
    ON imports(import_type, status, updated_at);
CREATE INDEX idx_imports_canonical_download_id
    ON imports(canonical_download_id);
CREATE INDEX idx_imports_source_identity
    ON imports(COALESCE(source_client_id, ''), source_system, source_ref);
CREATE INDEX idx_wanted_items_status_updated_at
    ON wanted_items(status, updated_at);
CREATE INDEX idx_download_identity_states_reason_canonical
    ON download_identity_states(reason, canonical_download_id);
CREATE INDEX idx_release_decisions_title_created_at
    ON release_decisions(title_id, created_at);
CREATE INDEX idx_pending_releases_title_added
    ON pending_releases(title_id, added_at DESC);
CREATE INDEX idx_media_requests_requester_created
    ON media_requests(created_by_user_id, created_at, id);
CREATE INDEX idx_media_requests_fingerprint_created
    ON media_requests(identity_fingerprint, created_at, id);
CREATE INDEX idx_history_events_actor_user
    ON history_events(actor_user_id)
    WHERE actor_user_id IS NOT NULL;
CREATE INDEX idx_titles_library_catalog_order_asc ON titles (
    library_id,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX idx_titles_library_catalog_order_desc ON titles (
    library_id,
    catalog_sort_key COLLATE BINARY DESC,
    catalog_name_tie_key COLLATE BINARY DESC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year DESC,
    id COLLATE BINARY DESC
);
CREATE INDEX idx_media_files_title_attention
    ON media_files(title_id)
    WHERE role = 'primary' AND scan_status IN ('scan_failed', 'review_required');
CREATE INDEX idx_indexer_search_runs_session
    ON indexer_search_runs(search_session_id);
CREATE INDEX idx_request_rule_decisions_evaluated
    ON request_rule_decisions(evaluated_at DESC, id DESC);
CREATE INDEX idx_request_rule_decisions_outcome_evaluated
    ON request_rule_decisions(effective_outcome, evaluated_at DESC, id DESC);
CREATE INDEX idx_maintenance_evaluation_runs_started
    ON maintenance_evaluation_runs(started_at DESC, id);
CREATE INDEX idx_discovery_section_items_item
    ON discovery_section_items(item_id);
CREATE INDEX idx_discovery_items_discovery_title
    ON discovery_items(discovery_title_id);
CREATE INDEX idx_discovery_items_tombstoned_by_run
    ON discovery_items(tombstoned_by_run_id)
    WHERE tombstoned_by_run_id IS NOT NULL;
CREATE INDEX idx_manual_import_selections_title_id
    ON manual_import_selections(title_id);
CREATE INDEX idx_library_scan_unmatched_items_title_id
    ON library_scan_unmatched_items(title_id);
CREATE INDEX idx_indexer_search_learning_title_id
    ON indexer_search_learning(title_id);
CREATE INDEX idx_lifecycle_action_runs_status_finished
    ON lifecycle_action_runs(status, finished_at);
CREATE INDEX idx_lifecycle_action_runs_generation_started
    ON lifecycle_action_runs(candidate_id, match_generation, started_at, id);
CREATE INDEX idx_title_recycle_purges_recorded_at
    ON title_recycle_purges (recorded_at, title_id);
CREATE TRIGGER trg_titles_root_folder_id_required_insert
BEFORE INSERT ON titles
FOR EACH ROW
WHEN NEW.root_folder_id IS NULL OR trim(NEW.root_folder_id) = ''
BEGIN
    SELECT RAISE(ABORT, 'title root_folder_id is required');
END;
CREATE TRIGGER trg_titles_root_folder_id_required_update
BEFORE UPDATE OF root_folder_id ON titles
FOR EACH ROW
WHEN NEW.root_folder_id IS NULL OR trim(NEW.root_folder_id) = ''
BEGIN
    SELECT RAISE(ABORT, 'title root_folder_id is required');
END;
CREATE TRIGGER titles_delete_enqueue_fuzzy_index
AFTER DELETE ON titles
BEGIN
    INSERT INTO title_search_index_queue (title_id) VALUES (OLD.id);
END;
CREATE TRIGGER titles_insert_bump_affinity_revision
AFTER INSERT ON titles
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER titles_delete_bump_affinity_revision
AFTER DELETE ON titles
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER titles_update_bump_affinity_revision
AFTER UPDATE OF library_id, facet ON titles
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER title_metadata_tags_insert_bump_affinity_revision
AFTER INSERT ON title_metadata_tags
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER title_metadata_tags_delete_bump_affinity_revision
AFTER DELETE ON title_metadata_tags
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER title_metadata_tags_update_bump_affinity_revision
AFTER UPDATE ON title_metadata_tags
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER title_metadata_tag_sources_insert_bump_affinity_revision
AFTER INSERT ON title_metadata_tag_sources
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER title_metadata_tag_sources_delete_bump_affinity_revision
AFTER DELETE ON title_metadata_tag_sources
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER title_metadata_tag_sources_update_bump_affinity_revision
AFTER UPDATE ON title_metadata_tag_sources
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER title_metadata_tag_source_keys_insert_bump_affinity_revision
AFTER INSERT ON title_metadata_tag_source_keys
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER title_metadata_tag_source_keys_delete_bump_affinity_revision
AFTER DELETE ON title_metadata_tag_source_keys
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER title_metadata_tag_source_keys_update_bump_affinity_revision
AFTER UPDATE ON title_metadata_tag_source_keys
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER discovery_submitted_subjects_insert_bump_affinity_revision
AFTER INSERT ON discovery_submitted_subjects
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER discovery_submitted_subjects_delete_bump_affinity_revision
AFTER DELETE ON discovery_submitted_subjects
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
CREATE TRIGGER discovery_submitted_subjects_update_bump_affinity_revision
AFTER UPDATE ON discovery_submitted_subjects
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;
INSERT INTO "discovery_affinity_revision" ("id", "revision") VALUES (1, lower(hex(randomblob(16))));
INSERT INTO "import_space_incident_lock" ("id", "revision") VALUES (1, 0);
INSERT INTO "libraries" ("id", "facet", "name", "slug", "is_default", "created_at", "updated_at") VALUES ('anime_default_library', 'anime', 'Anime', 'anime', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "libraries" ("id", "facet", "name", "slug", "is_default", "created_at", "updated_at") VALUES ('movie_default_library', 'movie', 'Movies', 'movies', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "libraries" ("id", "facet", "name", "slug", "is_default", "created_at", "updated_at") VALUES ('series_default_library', 'series', 'Series', 'series', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "library_root_id_remaps" ("legacy_root_id", "root_id", "normalized_path", "remapped", "created_at") VALUES ('3e1fce99b6700cecffbea040ff14843bf59d0f63799a4be8f75956efa1393b0e', 'root_40ccaf1cd56bb07fe89c141c0ace99ef', '/data/series', 1, '1970-01-01T00:00:00Z');
INSERT INTO "library_root_id_remaps" ("legacy_root_id", "root_id", "normalized_path", "remapped", "created_at") VALUES ('8277630ec2b8f510eb797f12f9b66bec03e5c9a395982fb220034a05aa044637', 'root_a0ee8082805273c1aa79dbcaeb707b63', '/data/anime', 1, '1970-01-01T00:00:00Z');
INSERT INTO "library_root_id_remaps" ("legacy_root_id", "root_id", "normalized_path", "remapped", "created_at") VALUES ('8312cd866e8499fd5a7a90f518f5d121e7d8a90ffb285800f60eeb323506c456', 'root_fe62bfbd205b27685df9635ddaaf0860', '/data/movies', 1, '1970-01-01T00:00:00Z');
INSERT INTO "library_roots" ("id", "library_id", "path", "normalized_path", "is_default", "created_at", "updated_at", "legacy_path_derived_id") VALUES ('canonical_root_for_anime_default_library', 'anime_default_library', '/data/anime', '/data/anime', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z', '8277630ec2b8f510eb797f12f9b66bec03e5c9a395982fb220034a05aa044637');
INSERT INTO "library_roots" ("id", "library_id", "path", "normalized_path", "is_default", "created_at", "updated_at", "legacy_path_derived_id") VALUES ('canonical_root_for_movie_default_library', 'movie_default_library', '/data/movies', '/data/movies', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z', '8312cd866e8499fd5a7a90f518f5d121e7d8a90ffb285800f60eeb323506c456');
INSERT INTO "library_roots" ("id", "library_id", "path", "normalized_path", "is_default", "created_at", "updated_at", "legacy_path_derived_id") VALUES ('canonical_root_for_series_default_library', 'series_default_library', '/data/series', '/data/series', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z', '3e1fce99b6700cecffbea040ff14843bf59d0f63799a4be8f75956efa1393b0e');
INSERT INTO "location_transfer_runtime" ("id", "generation") VALUES (1, 0);
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('1080p', '1080P', 0, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('1080p', '720P', 1, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('4k', '1080P', 1, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('4k', '2160P', 0, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('4k', '720P', 2, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profiles" ("id", "name", "scope", "scope_id", "archival_quality", "allow_unknown_quality", "atmos_preferred", "dolby_vision_allowed", "detected_hdr_allowed", "prefer_remux", "allow_bd_disk", "allow_upgrades", "created_at", "prefer_dual_audio", "required_audio_languages", "scoring_config") VALUES ('1080p', '1080P', 'system', NULL, '1080P', 0, 1, 1, 1, 1, 0, 1, '1970-01-01T00:00:00Z', 0, '[]', '{}');
INSERT INTO "quality_profiles" ("id", "name", "scope", "scope_id", "archival_quality", "allow_unknown_quality", "atmos_preferred", "dolby_vision_allowed", "detected_hdr_allowed", "prefer_remux", "allow_bd_disk", "allow_upgrades", "created_at", "prefer_dual_audio", "required_audio_languages", "scoring_config") VALUES ('4k', '4K', 'system', NULL, '2160P', 0, 1, 1, 1, 1, 0, 1, '1970-01-01T00:00:00Z', 0, '[]', '{}');
INSERT INTO "settings_definitions" ("id", "category", "scope", "key_name", "data_type", "default_value_json", "is_sensitive", "validation_json", "created_at", "updated_at") VALUES ('media:system:movie.availability_delay_days', 'media', 'system', 'movie.availability_delay_days', 'number', '0', 0, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "settings_definitions" ("id", "category", "scope", "key_name", "data_type", "default_value_json", "is_sensitive", "validation_json", "created_at", "updated_at") VALUES ('media:system:movie.default_availability', 'media', 'system', 'movie.default_availability', 'string', '"released"', 0, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "settings_definitions" ("id", "category", "scope", "key_name", "data_type", "default_value_json", "is_sensitive", "validation_json", "created_at", "updated_at") VALUES ('media:system:movie.release_market', 'media', 'system', 'movie.release_market', 'string', '"US"', 0, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "settings_definitions" ("id", "category", "scope", "key_name", "data_type", "default_value_json", "is_sensitive", "validation_json", "created_at", "updated_at") VALUES ('title-tags-pending-deletions', 'system', 'system', 'title_tags.pending_deletions', 'json', '{}', 0, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "settings_definitions" ("id", "category", "scope", "key_name", "data_type", "default_value_json", "is_sensitive", "validation_json", "created_at", "updated_at") VALUES ('title-tags-pending-renames', 'system', 'system', 'title_tags.pending_renames', 'json', '{}', 0, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "title_search_meta" ("id", "collation_version", "projection_generation") VALUES (1, '', 0);
INSERT INTO "users" ("id", "username", "display_name", "status", "password_hash", "passkey_public_key", "locale", "created_at", "updated_at", "last_login_at", "account_kind", "auth_session_version", "password_change_required") VALUES ('00000000000000000000000000000001', 'admin', NULL, 'active', NULL, NULL, NULL, '', '1970-01-01T00:00:00Z', NULL, 'local', NULL, 0);
