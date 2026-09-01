CREATE TABLE blocklist (
    id text NOT NULL,
    title_id text NOT NULL,
    source_title text,
    source_hint text,
    quality text,
    download_id text,
    reason text,
    data_json jsonb,
    created_at timestamp with time zone NOT NULL
);
CREATE TABLE collection_external_ids (
    id text NOT NULL,
    collection_id text NOT NULL,
    source text NOT NULL,
    external_id text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone,
    provenance text DEFAULT 'metadata'::text NOT NULL,
    source_scope text DEFAULT ''::text NOT NULL,
    title_id text NOT NULL
);
CREATE TABLE collections (
    id text NOT NULL,
    title_id text NOT NULL,
    collection_type text NOT NULL,
    collection_index text NOT NULL,
    label text,
    ordered_path text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone,
    narrative_order text,
    first_episode_number text,
    last_episode_number text,
    interstitial_season_episode text,
    monitored boolean DEFAULT true NOT NULL,
    interstitial_tvdb_id text,
    interstitial_name text,
    interstitial_slug text,
    interstitial_year integer,
    interstitial_content_status text,
    interstitial_overview text,
    interstitial_poster_url text,
    interstitial_language text,
    interstitial_runtime_minutes bigint,
    interstitial_sort_title text,
    interstitial_imdb_id text,
    interstitial_genres_json text,
    interstitial_studio text,
    interstitial_digital_release_date text,
    interstitial_association_confidence text,
    interstitial_continuity_status text,
    interstitial_movie_form text,
    interstitial_confidence text,
    interstitial_signal_summary text,
    interstitial_placement text,
    interstitial_movie_tmdb_id text,
    interstitial_movie_mal_id text,
    interstitial_movie_anidb_id text,
    special_movies_json text DEFAULT '[]'::text NOT NULL
);
CREATE TABLE discovery_facets (
    run_id text NOT NULL,
    facet_name text NOT NULL,
    facet_value text NOT NULL,
    smg_count bigint,
    local_count bigint
);
CREATE TABLE discovery_item_library_provenance (
    item_id text NOT NULL,
    run_id text NOT NULL,
    subject_key text NOT NULL,
    title_id text DEFAULT ''::text NOT NULL,
    library_id text DEFAULT ''::text NOT NULL
);
CREATE TABLE discovery_item_rank_components (
    item_id text NOT NULL,
    run_id text NOT NULL,
    component_index integer NOT NULL,
    component_name text DEFAULT ''::text NOT NULL,
    component_value text DEFAULT ''::text NOT NULL
);
CREATE TABLE discovery_item_subject_links (
    item_id text NOT NULL,
    run_id text NOT NULL,
    link_type text NOT NULL,
    subject_key text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_items (
    id text NOT NULL,
    run_id text NOT NULL,
    base_generation_id text,
    discovery_title_id text NOT NULL,
    source_run_kind text NOT NULL,
    section_id text,
    sort_index integer DEFAULT 0 NOT NULL,
    best_source text,
    source_count integer,
    edge_count integer,
    relation_count integer,
    source_subject_count integer,
    rank_score double precision,
    matched_subject_count integer DEFAULT 0 NOT NULL,
    owned_in_input boolean DEFAULT false NOT NULL,
    tombstoned_by_run_id text,
    tombstoned_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_pending_context_changes (
    id text NOT NULL,
    scope_key text DEFAULT 'default'::text NOT NULL,
    subject_key text,
    previous_subject_key text,
    change_type text NOT NULL,
    title_id text,
    previous_title_id text,
    library_facet text,
    raw_subject_json jsonb,
    raw_previous_subject_json jsonb,
    first_seen_sequence bigint,
    last_seen_sequence bigint,
    first_seen_at timestamp with time zone DEFAULT now() NOT NULL,
    last_seen_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_raw_pages (
    run_id text NOT NULL,
    payload_kind text NOT NULL,
    page_number integer DEFAULT 0 NOT NULL,
    compression text DEFAULT 'none'::text NOT NULL,
    raw_payload text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_section_items (
    run_id text NOT NULL,
    section_id text NOT NULL,
    item_id text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_sections (
    id text NOT NULL,
    run_id text NOT NULL,
    section_id text NOT NULL,
    section_type text NOT NULL,
    surface text NOT NULL,
    title text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_submitted_subjects (
    run_id text NOT NULL,
    subject_key text NOT NULL,
    title_id text,
    library_id text,
    library_facet text,
    title_kind text,
    display_title text,
    external_ids_json jsonb DEFAULT '[]'::jsonb NOT NULL,
    raw_subject_json jsonb NOT NULL
);
CREATE TABLE discovery_sync_runs (
    id text NOT NULL,
    kind text NOT NULL,
    status text NOT NULL,
    trigger_source text NOT NULL,
    region text NOT NULL,
    language text NOT NULL,
    subject_count bigint DEFAULT 0 NOT NULL,
    subject_fingerprint text,
    previous_subject_fingerprint text,
    base_generation_id text,
    changed_subject_count bigint DEFAULT 0 NOT NULL,
    affected_target_count bigint DEFAULT 0 NOT NULL,
    smg_request_id text,
    smg_status text,
    discovery_index_watermark text,
    page_count integer,
    item_count bigint,
    facet_count bigint,
    raw_submit_json jsonb,
    raw_changes_json jsonb,
    raw_final_status_json jsonb,
    raw_ack_json jsonb,
    error_text text,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_sync_state (
    scope_key text NOT NULL,
    last_success_generation_id text,
    last_public_feed_generation_id text,
    last_subject_fingerprint text,
    last_context_snapshot_completed_at timestamp with time zone,
    last_incremental_reload_completed_at timestamp with time zone,
    last_public_feed_completed_at timestamp with time zone,
    dirty_since timestamp with time zone,
    dirty_reason_mask bigint DEFAULT 0 NOT NULL,
    bootstrap_started_at timestamp with time zone,
    bootstrap_quiet_until timestamp with time zone,
    next_context_snapshot_eligible_at timestamp with time zone,
    next_incremental_reload_eligible_at timestamp with time zone,
    next_public_feed_eligible_at timestamp with time zone,
    backoff_until timestamp with time zone,
    startup_jitter_seconds bigint DEFAULT 0 NOT NULL,
    context_jitter_seconds bigint DEFAULT 0 NOT NULL,
    incremental_reload_jitter_seconds bigint DEFAULT 0 NOT NULL,
    public_feed_jitter_seconds bigint DEFAULT 0 NOT NULL,
    last_seen_domain_event_sequence bigint,
    inflight_subject_fingerprint text,
    inflight_domain_event_sequence bigint,
    inflight_context_snapshot_run_id text,
    lease_owner_id text,
    lease_expires_at timestamp with time zone,
    transient_failure_count bigint DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_title_external_ids (
    discovery_title_id text NOT NULL,
    source text NOT NULL,
    external_kind text DEFAULT ''::text NOT NULL,
    external_id text DEFAULT ''::text NOT NULL,
    external_key text DEFAULT ''::text NOT NULL,
    external_identity text DEFAULT ''::text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_title_metadata_external_ratings (
    discovery_title_id text NOT NULL,
    source text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL,
    value double precision,
    score double precision,
    normalized double precision NOT NULL,
    votes integer,
    url text DEFAULT ''::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_title_metadata_rating_sources (
    discovery_title_id text NOT NULL,
    source text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_title_metadata_rating_summaries (
    discovery_title_id text NOT NULL,
    rating double precision,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_title_metadata_tag_source_keys (
    discovery_title_id text NOT NULL,
    tag_key text NOT NULL,
    source_tag_key text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_title_metadata_tag_sources (
    discovery_title_id text NOT NULL,
    tag_key text NOT NULL,
    source text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_title_metadata_tags (
    discovery_title_id text NOT NULL,
    tag_key text NOT NULL,
    category text NOT NULL,
    name text NOT NULL,
    confidence double precision,
    is_adult boolean DEFAULT false NOT NULL,
    is_spoiler boolean DEFAULT false NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_title_source_tag_values (
    discovery_title_id text NOT NULL,
    source_tag_sort_index integer NOT NULL,
    source_tag_value text NOT NULL,
    value_sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_title_source_tags (
    discovery_title_id text NOT NULL,
    category text DEFAULT ''::text NOT NULL,
    name text DEFAULT ''::text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_title_terms (
    discovery_title_id text NOT NULL,
    term_kind text NOT NULL,
    term_category text DEFAULT ''::text NOT NULL,
    term_value text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_titles (
    id text NOT NULL,
    target_key text NOT NULL,
    target_key_norm text NOT NULL,
    language text NOT NULL,
    target_kind text NOT NULL,
    resolved boolean DEFAULT false NOT NULL,
    resolved_title_id text,
    display_title text NOT NULL,
    original_title text,
    sort_title text,
    year integer,
    poster_path text,
    poster_url text,
    background_url text,
    overview text,
    content_type text,
    tmdb_collection_id text,
    tmdb_collection_name text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE domain_events (
    sequence bigint NOT NULL,
    event_id text NOT NULL,
    occurred_at timestamp with time zone NOT NULL,
    actor_user_id text,
    title_id text,
    facet text,
    correlation_id text,
    causation_id text,
    schema_version bigint NOT NULL,
    stream_kind text NOT NULL,
    stream_id text,
    event_type text NOT NULL,
    payload_json jsonb NOT NULL,
    actor_kind text DEFAULT 'system'::text NOT NULL,
    actor_display_name text DEFAULT 'System'::text NOT NULL
);
CREATE SEQUENCE domain_events_sequence_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;
ALTER SEQUENCE domain_events_sequence_seq OWNED BY domain_events.sequence;
CREATE TABLE download_clients (
    id text NOT NULL,
    name text NOT NULL,
    client_type text NOT NULL,
    base_url text,
    is_enabled boolean DEFAULT true NOT NULL,
    status text DEFAULT 'idle'::text NOT NULL,
    last_error text,
    last_seen_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    client_priority bigint DEFAULT 0 NOT NULL,
    config_json text NOT NULL
);
CREATE TABLE download_identity_states (
    id text NOT NULL,
    identity_key text NOT NULL,
    download_id text,
    client_id text,
    client_type text,
    download_client_item_id text,
    tracked_state text NOT NULL,
    reason text,
    detail text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT download_identity_states_download_id_check CHECK ((download_id IS NOT NULL))
);
CREATE TABLE download_import_artifacts (
    id text NOT NULL,
    source_system text,
    source_ref text,
    import_id text,
    relative_path text,
    normalized_file_name text,
    media_kind text,
    title_id text,
    episode_id text,
    season_number bigint,
    episode_number bigint,
    result text,
    reason_code text,
    imported_media_file_id text,
    created_at timestamp with time zone DEFAULT now(),
    source_client_id text
);
CREATE TABLE download_queue_commands (
    id text NOT NULL,
    action text NOT NULL,
    client_id text,
    client_type text NOT NULL,
    download_client_item_id text NOT NULL,
    is_history boolean DEFAULT false NOT NULL,
    status text NOT NULL,
    error_text text,
    requested_by_user_id text,
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE download_submission_episode_links (
    episode_id text NOT NULL,
    download_client_id text DEFAULT ''::text NOT NULL,
    download_client_type text NOT NULL,
    download_client_item_id text NOT NULL
);
CREATE TABLE download_submissions (
    id text NOT NULL,
    title_id text DEFAULT ''::text NOT NULL,
    facet text DEFAULT ''::text NOT NULL,
    download_client_id text DEFAULT ''::text NOT NULL,
    download_client_type text NOT NULL,
    download_client_item_id text NOT NULL,
    source_title text,
    submitted_at timestamp with time zone DEFAULT now() NOT NULL,
    collection_id text,
    tracked_state text,
    tracked_state_at timestamp with time zone,
    source_hint text,
    source_kind text,
    request_signature text,
    episode_id text,
    download_id text,
    purpose text DEFAULT 'standard'::text NOT NULL,
    series_movie_link_id text,
    actor_kind text,
    actor_user_id text,
    actor_display_name text
);
CREATE TABLE emby_media_server_details (
    connection_id text NOT NULL,
    api_key text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE episode_external_ids (
    id text NOT NULL,
    episode_id text NOT NULL,
    source text NOT NULL,
    external_id text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone,
    provenance text DEFAULT 'metadata'::text NOT NULL,
    source_scope text DEFAULT ''::text NOT NULL,
    title_id text NOT NULL
);
CREATE TABLE episodes (
    id text NOT NULL,
    title_id text NOT NULL,
    collection_id text,
    episode_type text NOT NULL,
    episode_number text,
    season_number text,
    episode_label text,
    title text,
    air_date text,
    duration_seconds bigint,
    monitored boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone,
    has_multi_audio boolean DEFAULT false,
    has_subtitle boolean DEFAULT false,
    is_filler boolean DEFAULT false,
    is_recap boolean DEFAULT false,
    absolute_number text,
    overview text,
    tvdb_id text,
    image_url text
);
CREATE TABLE event_outboxes (
    id text NOT NULL,
    payload_json jsonb NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    dispatched_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    history_event_id text NOT NULL,
    channel_key text NOT NULL,
    attempt_count bigint DEFAULT 0 NOT NULL,
    last_error text
);
CREATE TABLE event_subscriber_offsets (
    subscriber_name text NOT NULL,
    sequence bigint NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE external_import_monitor_snapshot_chunks (
    facet text NOT NULL,
    entry_kind text NOT NULL,
    chunk_index integer NOT NULL,
    payload_ndjson text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    session_id text NOT NULL,
    CONSTRAINT external_import_monitor_snapshot_chunks_entry_kind_check1 CHECK ((entry_kind = ANY (ARRAY['movie'::text, 'series'::text]))),
    CONSTRAINT external_import_monitor_snapshot_chunks_facet_check1 CHECK ((facet = ANY (ARRAY['movie'::text, 'series'::text, 'anime'::text])))
);
CREATE TABLE external_import_setup_download_client_api_key_overrides (
    draft_key text NOT NULL,
    dedup_key text NOT NULL,
    api_key_encrypted text NOT NULL,
    "position" integer NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT external_import_setup_download_client_api_key_ov_position_check CHECK (("position" >= 0))
);
CREATE TABLE external_import_setup_download_client_password_overrides (
    draft_key text NOT NULL,
    dedup_key text NOT NULL,
    password_encrypted text NOT NULL,
    "position" integer NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT external_import_setup_download_client_password_o_position_check CHECK (("position" >= 0))
);
CREATE TABLE external_import_setup_indexer_api_key_overrides (
    draft_key text NOT NULL,
    dedup_key text NOT NULL,
    api_key_encrypted text NOT NULL,
    "position" integer NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT external_import_setup_indexer_api_key_overrides_position_check CHECK (("position" >= 0))
);
CREATE TABLE external_import_setup_instance_api_keys (
    draft_key text NOT NULL,
    instance_id text NOT NULL,
    kind text NOT NULL,
    api_key_encrypted text NOT NULL,
    "position" integer NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT external_import_setup_instance_api_keys_kind_check CHECK ((kind = ANY (ARRAY['sonarr'::text, 'radarr'::text, 'prowlarr'::text]))),
    CONSTRAINT external_import_setup_instance_api_keys_position_check CHECK (("position" >= 0))
);
CREATE TABLE external_import_setup_secret_drafts (
    draft_key text NOT NULL,
    owner_user_id text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT external_import_setup_secret_drafts_draft_key_check CHECK ((draft_key = 'active'::text))
);
CREATE TABLE external_subtitle_probe_cache (
    media_file_id text NOT NULL,
    file_path text NOT NULL,
    size_bytes bigint,
    modified_at timestamp with time zone,
    language text,
    hearing_impaired boolean,
    detection_source_language text,
    detection_source_hi text,
    probe_version bigint,
    updated_at timestamp with time zone DEFAULT now()
);
CREATE TABLE file_episode_map (
    file_id text NOT NULL,
    episode_id text NOT NULL,
    is_filler boolean DEFAULT false
);
CREATE TABLE file_series_movie_link_map (
    file_id text NOT NULL,
    series_movie_link_id text NOT NULL
);
CREATE TABLE history_events (
    id text NOT NULL,
    title_id text,
    event_type text,
    occurred_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    actor_user_id text,
    message text NOT NULL,
    metadata_json jsonb,
    source text
);
CREATE TABLE imports (
    id text NOT NULL,
    source_system text NOT NULL,
    source_ref text NOT NULL,
    import_type text NOT NULL,
    status text DEFAULT 'queued'::text NOT NULL,
    payload_json jsonb NOT NULL,
    result_json jsonb,
    rename_plan_json jsonb,
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    source_client_id text,
    download_id text,
    import_transfer_phase text,
    import_transfer_bytes bigint,
    import_transfer_total_bytes bigint,
    import_transfer_started_at timestamp with time zone,
    import_transfer_updated_at timestamp with time zone
);
CREATE TABLE indexer_api_quotas (
    indexer_id text NOT NULL,
    api_current bigint,
    api_max bigint,
    grab_current bigint,
    grab_max bigint,
    queries_today bigint DEFAULT 0 NOT NULL,
    last_query_at timestamp with time zone,
    last_reset_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE indexer_search_learning (
    indexer_id text NOT NULL,
    title_id text NOT NULL,
    facet text NOT NULL,
    strategy_key text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    empty_successes integer DEFAULT 0 NOT NULL,
    usable_successes integer DEFAULT 0 NOT NULL,
    last_attempt_at timestamp with time zone,
    last_usable_at timestamp with time zone,
    suppressed boolean DEFAULT false NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE indexer_system_backoffs (
    indexer_id text NOT NULL,
    disabled_until timestamp with time zone NOT NULL,
    escalation_level integer DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE indexers (
    id text NOT NULL,
    name text NOT NULL,
    provider_type text NOT NULL,
    base_url text NOT NULL,
    api_key_encrypted text,
    rate_limit_seconds bigint,
    rate_limit_burst bigint,
    disabled_until timestamp with time zone,
    is_enabled boolean DEFAULT true NOT NULL,
    enable_interactive_search boolean DEFAULT true NOT NULL,
    enable_auto_search boolean DEFAULT true NOT NULL,
    managed_parent_config_id text,
    managed_child_key text,
    managed_metadata_json text,
    last_health_status text,
    last_error_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    config_json text,
    caps_snapshot_json text
);
CREATE TABLE jellyfin_media_server_details (
    connection_id text NOT NULL,
    api_key text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE libraries (
    id text NOT NULL,
    facet text NOT NULL,
    name text NOT NULL,
    slug text NOT NULL,
    is_default boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE library_probe_signatures (
    title_id text NOT NULL,
    path text,
    probe_signature_scheme text,
    probe_signature_value text,
    last_probed_at timestamp with time zone,
    last_changed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);
CREATE TABLE library_roots (
    id text NOT NULL,
    library_id text NOT NULL,
    path text NOT NULL,
    normalized_path text NOT NULL,
    is_default boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE library_scan_unmatched_items (
    id text NOT NULL,
    facet text NOT NULL,
    title_id text,
    scan_root text,
    item_path text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    library_id text,
    scan_session_id text,
    display_name text,
    query text,
    year_hint bigint,
    reason_code text,
    error_message text,
    search_attempts_json text DEFAULT '[]'::text
);
CREATE TABLE media_files (
    id text NOT NULL,
    title_id text NOT NULL,
    file_path text NOT NULL,
    size_bytes bigint NOT NULL,
    quality_id text,
    hash_sha256 text,
    has_multiaudio boolean DEFAULT false,
    scan_status text DEFAULT 'pending'::text NOT NULL,
    scan_error text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    video_codec text,
    video_width bigint,
    video_height bigint,
    video_bitrate_kbps bigint,
    video_bit_depth bigint,
    video_hdr_format text,
    audio_codec text,
    audio_channels bigint,
    duration_seconds bigint,
    container_format text,
    analysis_json text,
    video_frame_rate text,
    video_profile text,
    audio_bitrate_kbps bigint,
    scene_name text,
    release_group text,
    source_type text,
    resolution text,
    video_codec_parsed text,
    audio_codec_parsed text,
    acquisition_score bigint,
    scoring_log text,
    indexer_source text,
    grabbed_release_title text,
    grabbed_at timestamp with time zone,
    edition text,
    original_file_path text,
    release_hash text,
    num_chapters bigint,
    source_signature_scheme text,
    source_signature_value text,
    audio_profile text,
    audio_channels_parsed text,
    role text DEFAULT 'primary'::text NOT NULL
);
CREATE TABLE media_request_external_ids (
    request_id text NOT NULL,
    library_id text NOT NULL,
    source text NOT NULL,
    external_id text NOT NULL,
    created_at timestamp with time zone NOT NULL
);
CREATE TABLE media_request_requesters (
    request_id text NOT NULL,
    user_id text NOT NULL,
    requested_at timestamp with time zone NOT NULL
);
CREATE TABLE media_requests (
    id text NOT NULL,
    library_id text NOT NULL,
    facet text NOT NULL,
    status text NOT NULL,
    identity_fingerprint text NOT NULL,
    title text NOT NULL,
    sort_title text,
    slug text,
    poster_url text,
    year integer,
    overview text,
    runtime_minutes integer,
    language text,
    content_status text,
    created_by_user_id text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    requested_quality_profile_id text,
    requested_quality_profile_name text,
    resolved_by_user_id text,
    resolved_at timestamp with time zone,
    created_title_id text,
    approved_quality_profile_id text,
    approved_quality_profile_name text,
    requested_monitor_type text,
    CONSTRAINT media_requests_facet_check CHECK ((facet = ANY (ARRAY['movie'::text, 'series'::text, 'anime'::text]))),
    CONSTRAINT media_requests_requested_monitor_type_check CHECK (((requested_monitor_type IS NULL) OR (requested_monitor_type = ANY (ARRAY['monitored'::text, 'unmonitored'::text, 'futureepisodes'::text, 'missingandfutureepisodes'::text, 'allepisodes'::text, 'none'::text])))),
    CONSTRAINT media_requests_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'approved'::text, 'rejected'::text, 'canceled'::text])))
);
CREATE TABLE media_server_connections (
    id text NOT NULL,
    provider text NOT NULL,
    display_name text NOT NULL,
    base_url text NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    login_enabled boolean DEFAULT false NOT NULL,
    linking_enabled boolean DEFAULT false NOT NULL,
    auto_add_enabled boolean DEFAULT false NOT NULL,
    default_app_permissions bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT media_server_connections_provider_check CHECK ((provider = ANY (ARRAY['jellyfin'::text, 'plex'::text, 'emby'::text])))
);
CREATE TABLE media_server_default_library_grants (
    connection_id text NOT NULL,
    library_id text NOT NULL,
    permissions bigint DEFAULT 0 NOT NULL
);
CREATE TABLE media_server_path_mappings (
    id text NOT NULL,
    connection_id text NOT NULL,
    source_path text NOT NULL,
    destination_path text NOT NULL,
    sort_order bigint DEFAULT 0 NOT NULL
);
CREATE TABLE movie_entities (
    id text NOT NULL,
    title text NOT NULL,
    sort_title text,
    slug text,
    year integer,
    overview text,
    poster_url text,
    background_url text,
    language text,
    runtime_minutes integer,
    content_status text,
    genres_json text DEFAULT '[]'::text NOT NULL,
    studio text,
    digital_release_date text,
    imdb_id text,
    tvdb_id text,
    tmdb_id text,
    mal_id text,
    anidb_id text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE notification_channels (
    id text NOT NULL,
    name text NOT NULL,
    channel_type text NOT NULL,
    config_json text NOT NULL,
    is_enabled boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    media_server_connection_id text
);
CREATE TABLE notification_subscriptions (
    id text NOT NULL,
    channel_id text,
    event_type text NOT NULL,
    scope text NOT NULL,
    scope_id text,
    is_enabled boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    target_kind text DEFAULT 'plugin_channel'::text NOT NULL,
    target_id text NOT NULL,
    CONSTRAINT notification_subscriptions_target_kind_check CHECK ((target_kind = ANY (ARRAY['plugin_channel'::text, 'media_server_connection'::text])))
);
CREATE TABLE oauth_authorization_codes (
    id text NOT NULL,
    code_hash text NOT NULL,
    client_id text NOT NULL,
    user_id text NOT NULL,
    redirect_uri text NOT NULL,
    scope text NOT NULL,
    code_challenge text NOT NULL,
    code_challenge_method text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    authorization_source text DEFAULT 'authenticated'::text NOT NULL
);
CREATE TABLE oauth_refresh_grants (
    id text NOT NULL,
    family_id text NOT NULL,
    user_id text NOT NULL,
    client_id text NOT NULL,
    scope text NOT NULL,
    auth_session_version text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    last_used_at timestamp with time zone,
    revoked_at timestamp with time zone,
    revoked_reason text,
    authorization_source text DEFAULT 'authenticated'::text NOT NULL
);
CREATE TABLE oauth_refresh_tokens (
    id text NOT NULL,
    grant_id text NOT NULL,
    family_id text NOT NULL,
    token_hash text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    revoked_at timestamp with time zone
);
CREATE TABLE pending_releases (
    id text NOT NULL,
    wanted_item_id text,
    title_id text,
    release_title text,
    release_url text,
    source_kind text,
    release_size_bytes bigint,
    release_score bigint,
    scoring_log_json jsonb,
    indexer_source text,
    release_guid text,
    added_at timestamp with time zone,
    delay_until timestamp with time zone,
    status text DEFAULT 'waiting'::text,
    grabbed_at timestamp with time zone,
    source_password text,
    published_at timestamp with time zone,
    info_hash text
);
CREATE TABLE plex_media_server_details (
    connection_id text NOT NULL,
    machine_id text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    api_key text
);
CREATE TABLE plugin_catalog_sources (
    source_key text NOT NULL,
    source_kind text NOT NULL,
    source_url text NOT NULL,
    github_repo text,
    support_tier text NOT NULL,
    catalog_json text,
    last_success_at timestamp with time zone,
    last_error text,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE plugin_catalog_status (
    status_key text NOT NULL,
    status_json text NOT NULL,
    checked_at timestamp with time zone NOT NULL
);
CREATE TABLE plugin_installations (
    id text NOT NULL,
    plugin_id text NOT NULL,
    name text NOT NULL,
    description text NOT NULL,
    version text NOT NULL,
    sdk_version text NOT NULL,
    sdk_constraint text NOT NULL,
    scryer_constraint text,
    plugin_type text NOT NULL,
    provider_type text NOT NULL,
    source_kind text NOT NULL,
    is_enabled boolean DEFAULT true NOT NULL,
    is_builtin boolean DEFAULT false NOT NULL,
    wasm_bytes bytea,
    wasm_encoding text DEFAULT 'identity'::text NOT NULL,
    wasm_digest_algo text,
    source_url text,
    support_tier text DEFAULT 'official'::text NOT NULL,
    publisher text,
    docs_url text,
    source_repo text,
    manifest_url text,
    wasm_digest text,
    artifact_digest text,
    descriptor_json text,
    installed_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE post_processing_script_runs (
    id text NOT NULL,
    script_id text NOT NULL,
    status text NOT NULL,
    started_at timestamp with time zone NOT NULL,
    script_name text DEFAULT ''::text NOT NULL,
    title_id text,
    title_name text,
    facet text,
    file_path text,
    exit_code integer,
    stdout_tail text,
    stderr_tail text,
    duration_ms bigint,
    env_payload_json text,
    completed_at timestamp with time zone
);
CREATE TABLE post_processing_scripts (
    id text NOT NULL,
    name text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    priority integer DEFAULT 0 NOT NULL,
    description text DEFAULT ''::text,
    script_type text DEFAULT 'inline'::text NOT NULL,
    script_content text DEFAULT ''::text NOT NULL,
    applied_facets text DEFAULT '[]'::text NOT NULL,
    execution_mode text DEFAULT 'blocking'::text NOT NULL,
    timeout_secs bigint DEFAULT 300,
    enabled boolean DEFAULT true NOT NULL,
    debug boolean DEFAULT false NOT NULL
);
CREATE TABLE quality_profile_audio_codec_allowlist (
    profile_id text NOT NULL,
    codec text NOT NULL,
    created_at timestamp with time zone DEFAULT now()
);
CREATE TABLE quality_profile_audio_codec_blocklist (
    profile_id text NOT NULL,
    codec text NOT NULL,
    created_at timestamp with time zone DEFAULT now()
);
CREATE TABLE quality_profile_quality_tiers (
    profile_id text NOT NULL,
    quality_tier text NOT NULL,
    sort_order bigint NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE quality_profile_source_allowlist (
    profile_id text NOT NULL,
    source text NOT NULL,
    created_at timestamp with time zone DEFAULT now()
);
CREATE TABLE quality_profile_source_blocklist (
    profile_id text NOT NULL,
    source text NOT NULL,
    created_at timestamp with time zone DEFAULT now()
);
CREATE TABLE quality_profile_video_codec_allowlist (
    profile_id text NOT NULL,
    codec text NOT NULL,
    created_at timestamp with time zone DEFAULT now()
);
CREATE TABLE quality_profile_video_codec_blocklist (
    profile_id text NOT NULL,
    codec text NOT NULL,
    created_at timestamp with time zone DEFAULT now()
);
CREATE TABLE quality_profiles (
    id text NOT NULL,
    name text NOT NULL,
    scope text NOT NULL,
    scope_id text,
    archival_quality text,
    allow_unknown_quality boolean DEFAULT false NOT NULL,
    atmos_preferred boolean DEFAULT false NOT NULL,
    dolby_vision_allowed boolean DEFAULT false NOT NULL,
    detected_hdr_allowed boolean DEFAULT true NOT NULL,
    prefer_remux boolean DEFAULT false NOT NULL,
    allow_bd_disk boolean DEFAULT false NOT NULL,
    allow_upgrades boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    prefer_dual_audio boolean DEFAULT false NOT NULL,
    required_audio_languages jsonb DEFAULT '[]'::jsonb NOT NULL,
    scoring_config jsonb DEFAULT '{}'::jsonb NOT NULL
);
CREATE TABLE release_decisions (
    id text NOT NULL,
    wanted_item_id text,
    title_id text,
    release_title text,
    release_url text,
    release_size_bytes bigint,
    decision_code text,
    candidate_score bigint,
    current_score bigint,
    score_delta bigint,
    explanation_json jsonb,
    created_at timestamp with time zone DEFAULT now()
);
CREATE TABLE release_download_attempts (
    id text NOT NULL,
    title_id text,
    source_hint text,
    source_title text,
    outcome text NOT NULL,
    error_message text,
    source_password text,
    attempted_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE rule_set_history (
    id text NOT NULL,
    rule_set_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    action text NOT NULL,
    rego_source text,
    actor_id text
);
CREATE TABLE rule_sets (
    id text NOT NULL,
    name text NOT NULL,
    managed_key text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    priority integer DEFAULT 0 NOT NULL,
    is_managed boolean DEFAULT false NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    rego_source text DEFAULT ''::text NOT NULL,
    applied_facets text DEFAULT '[]'::text NOT NULL
);
CREATE TABLE series_movie_links (
    id text NOT NULL,
    series_title_id text NOT NULL,
    movie_entity_id text NOT NULL,
    placement text,
    narrative_order text,
    after_season integer,
    before_season integer,
    linked_episode_id text,
    association_confidence text,
    continuity_status text,
    movie_form text,
    confidence text,
    signal_summary text,
    source text,
    monitored boolean DEFAULT true NOT NULL,
    legacy_collection_id text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE settings_definitions (
    id text NOT NULL,
    category text NOT NULL,
    scope text NOT NULL,
    key_name text NOT NULL,
    data_type text NOT NULL,
    default_value_json jsonb NOT NULL,
    is_sensitive boolean DEFAULT false NOT NULL,
    validation_json jsonb,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE settings_values (
    id text NOT NULL,
    setting_definition_id text NOT NULL,
    scope text NOT NULL,
    scope_id text,
    value_json jsonb NOT NULL,
    source text NOT NULL,
    updated_by_user_id text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE subtitle_blocklist (
    id text NOT NULL,
    media_file_id text NOT NULL,
    provider text NOT NULL,
    provider_file_id text NOT NULL,
    language text NOT NULL,
    reason text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE subtitle_downloads (
    id text NOT NULL,
    media_file_id text,
    title_id text,
    episode_id text,
    language text,
    provider text,
    provider_file_id text,
    file_path text,
    score bigint,
    hearing_impaired boolean DEFAULT false,
    forced boolean DEFAULT false,
    ai_translated boolean DEFAULT false,
    machine_translated boolean DEFAULT false,
    uploader text,
    release_info text,
    synced boolean DEFAULT false,
    downloaded_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now(),
    source_kind text DEFAULT 'downloaded'::text
);
CREATE TABLE subtitle_provider_configs (
    id text NOT NULL,
    name text NOT NULL,
    provider_type text NOT NULL,
    enabled_facets jsonb DEFAULT '[]'::jsonb NOT NULL,
    is_enabled boolean DEFAULT true NOT NULL,
    last_error text,
    last_health_status text,
    last_error_at timestamp with time zone,
    disabled_until timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    config_json text NOT NULL
);
CREATE TABLE title_external_ids (
    id text NOT NULL,
    title_id text NOT NULL,
    facet text,
    source text NOT NULL,
    external_id text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone,
    library_id text
);
CREATE TABLE title_image_blobs (
    digest text NOT NULL,
    format text NOT NULL,
    width bigint NOT NULL,
    height bigint NOT NULL,
    bytes bytea NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE title_image_variants (
    id text NOT NULL,
    title_image_id text NOT NULL,
    variant_key text NOT NULL,
    blob_digest text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE title_images (
    id text NOT NULL,
    title_id text NOT NULL,
    provider text NOT NULL,
    provider_image_id text,
    kind text NOT NULL,
    source_url text NOT NULL,
    source_etag text,
    source_last_modified text,
    source_format text NOT NULL,
    source_width bigint,
    source_height bigint,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE title_metadata_external_ratings (
    title_id text NOT NULL,
    source text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL,
    value double precision,
    score double precision,
    normalized double precision NOT NULL,
    votes integer,
    url text DEFAULT ''::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE title_metadata_rating_sources (
    title_id text NOT NULL,
    source text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE title_metadata_rating_summaries (
    title_id text NOT NULL,
    rating double precision,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE title_metadata_tag_source_keys (
    title_id text NOT NULL,
    tag_key text NOT NULL,
    source_tag_key text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE title_metadata_tag_sources (
    title_id text NOT NULL,
    tag_key text NOT NULL,
    source text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE title_metadata_tags (
    title_id text NOT NULL,
    tag_key text NOT NULL,
    category text NOT NULL,
    name text NOT NULL,
    confidence double precision,
    is_adult boolean DEFAULT false NOT NULL,
    is_spoiler boolean DEFAULT false NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE title_more_like_this_items (
    source_title_id text NOT NULL,
    discovery_title_id text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL,
    rank_score double precision,
    best_source text,
    source_count integer,
    edge_count integer,
    relation_count integer,
    source_subject_count integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE title_search_terms (
    term_id bigint NOT NULL,
    title_id text NOT NULL,
    facet text NOT NULL,
    term_kind text NOT NULL,
    raw_term text NOT NULL,
    normalized_term text NOT NULL,
    weight bigint NOT NULL
);
CREATE SEQUENCE title_search_terms_term_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;
ALTER SEQUENCE title_search_terms_term_id_seq OWNED BY title_search_terms.term_id;
CREATE TABLE titles (
    id text NOT NULL,
    library_id text DEFAULT ''::text NOT NULL,
    name text NOT NULL,
    monitored boolean DEFAULT true NOT NULL,
    facet text NOT NULL,
    tags jsonb DEFAULT '[]'::jsonb NOT NULL,
    external_ids jsonb DEFAULT '[]'::jsonb NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    year integer,
    overview text,
    poster_url text,
    poster_local_path text,
    background_url text,
    background_local_path text,
    sort_title text,
    slug text,
    imdb_id text,
    runtime_minutes integer,
    genres jsonb DEFAULT '[]'::jsonb NOT NULL,
    content_status text,
    language text,
    first_aired text,
    network text,
    studio text,
    country text,
    aliases jsonb DEFAULT '[]'::jsonb NOT NULL,
    metadata_language text,
    metadata_fetched_at timestamp with time zone,
    min_availability text,
    digital_release_date text,
    folder_path text,
    tagged_aliases_json jsonb DEFAULT '[]'::jsonb NOT NULL,
    metadata_hydration_next_attempt_at timestamp with time zone,
    metadata_hydration_attempt_count bigint DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now(),
    name_normalized text DEFAULT ''::text NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    deleted_at timestamp with time zone,
    root_folder_id text NOT NULL,
    catalog_sort_key text DEFAULT ''::text NOT NULL,
    popularity double precision
);
CREATE TABLE totp_credentials (
    id text NOT NULL,
    user_id text NOT NULL,
    secret_base32 text NOT NULL,
    algorithm text NOT NULL,
    digits integer NOT NULL,
    period_seconds integer NOT NULL,
    last_accepted_step bigint,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    last_used_at timestamp with time zone,
    CONSTRAINT totp_credentials_algorithm_check CHECK ((algorithm = ANY (ARRAY['SHA1'::text, 'SHA256'::text, 'SHA512'::text]))),
    CONSTRAINT totp_credentials_digits_check CHECK ((digits = ANY (ARRAY[6, 8]))),
    CONSTRAINT totp_credentials_period_seconds_check CHECK ((period_seconds > 0))
);
CREATE TABLE totp_enrollment_challenges (
    id text NOT NULL,
    user_id text NOT NULL,
    secret_base32 text NOT NULL,
    algorithm text NOT NULL,
    digits integer NOT NULL,
    period_seconds integer NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    CONSTRAINT totp_enrollment_challenges_algorithm_check CHECK ((algorithm = ANY (ARRAY['SHA1'::text, 'SHA256'::text, 'SHA512'::text]))),
    CONSTRAINT totp_enrollment_challenges_digits_check CHECK ((digits = ANY (ARRAY[6, 8]))),
    CONSTRAINT totp_enrollment_challenges_period_seconds_check CHECK ((period_seconds > 0))
);
CREATE TABLE totp_failed_attempts (
    id text NOT NULL,
    user_id text NOT NULL,
    attempted_at timestamp with time zone NOT NULL
);
CREATE TABLE totp_recovery_codes (
    id text NOT NULL,
    user_id text NOT NULL,
    code_hash text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone
);
CREATE TABLE upstream_destination_cooldowns (
    destination_key text NOT NULL,
    cooldown_until timestamp with time zone NOT NULL,
    retry_after_seconds bigint,
    source text NOT NULL,
    status_code bigint,
    message text,
    observed_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE upstream_scheduler_rss_cadence (
    host_key text NOT NULL,
    destination_key text NOT NULL,
    account_quota_key text NOT NULL,
    rss_request_key text DEFAULT ''::text NOT NULL,
    last_successful_poll_at timestamp with time zone,
    last_attempt_at timestamp with time zone,
    target_interval_seconds bigint NOT NULL,
    latest_safe_poll_at timestamp with time zone,
    estimated_feed_depth bigint,
    freshness_risk double precision DEFAULT 0 NOT NULL,
    destination_recent_activity_at timestamp with time zone,
    last_seen_release_identity text,
    last_seen_release_published_at timestamp with time zone,
    last_feed_gap_start_at timestamp with time zone,
    last_feed_gap_end_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE upstream_scheduler_states (
    host_key text NOT NULL,
    destination_key text NOT NULL,
    account_quota_key text DEFAULT ''::text NOT NULL,
    rss_request_key text DEFAULT ''::text NOT NULL,
    api_current bigint,
    api_max bigint,
    grab_current bigint,
    grab_max bigint,
    quota_observed_at timestamp with time zone,
    quota_probe_after timestamp with time zone,
    quota_reset_at timestamp with time zone,
    quota_source text,
    last_decision text,
    last_feedback_at timestamp with time zone,
    last_successful_at timestamp with time zone,
    last_attempt_at timestamp with time zone,
    admitted_count bigint DEFAULT 0 NOT NULL,
    deferred_count bigint DEFAULT 0 NOT NULL,
    skipped_count bigint DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE user_app_permission_masks (
    user_id text NOT NULL,
    permission_mask bigint NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE user_external_accounts (
    id text NOT NULL,
    user_id text NOT NULL,
    provider text NOT NULL,
    connection_id text NOT NULL,
    external_user_id text,
    username text NOT NULL,
    display_name text,
    avatar_url text,
    status text NOT NULL,
    verified_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    last_login_at timestamp with time zone,
    CONSTRAINT user_external_accounts_provider_check CHECK ((provider = ANY (ARRAY['plex'::text, 'jellyfin'::text]))),
    CONSTRAINT user_external_accounts_status_check CHECK ((status = ANY (ARRAY['pending_claim'::text, 'active'::text, 'disabled'::text])))
);
CREATE TABLE user_library_permission_masks (
    user_id text NOT NULL,
    library_id text NOT NULL,
    permission_mask bigint NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE user_ui_settings (
    user_id text NOT NULL,
    theme text DEFAULT 'dark'::text NOT NULL,
    date_time_format text DEFAULT 'locale'::text NOT NULL,
    highlight_color text,
    secondary_color text,
    high_contrast_mode boolean DEFAULT false NOT NULL,
    reduce_motion boolean DEFAULT false NOT NULL,
    hide_sponsor_button boolean DEFAULT false NOT NULL,
    density text DEFAULT 'comfortable'::text NOT NULL,
    sidebar_mode text DEFAULT 'expanded'::text NOT NULL,
    default_landing_view text DEFAULT 'movies'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE user_ui_table_columns (
    user_id text NOT NULL,
    facet text NOT NULL,
    table_view_mode text NOT NULL,
    column_id text NOT NULL,
    column_order integer NOT NULL,
    visible boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE users (
    id text NOT NULL,
    username text NOT NULL,
    password_hash text,
    display_name text,
    status text DEFAULT 'active'::text NOT NULL,
    passkey_public_key text,
    locale text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    last_login_at timestamp with time zone,
    account_kind text DEFAULT 'local'::text NOT NULL,
    auth_session_version text,
    CONSTRAINT users_account_kind_check CHECK ((account_kind = ANY (ARRAY['local'::text, 'external_auto_provisioned'::text])))
);
CREATE TABLE wanted_items (
    id text NOT NULL,
    title_id text NOT NULL,
    episode_id text,
    collection_id text,
    media_type text NOT NULL,
    search_phase text DEFAULT 'primary'::text NOT NULL,
    next_search_at timestamp with time zone,
    last_search_at timestamp with time zone,
    search_count bigint DEFAULT 0 NOT NULL,
    baseline_date timestamp with time zone,
    status text DEFAULT 'wanted'::text NOT NULL,
    grabbed_release text,
    current_score bigint,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    series_movie_link_id text
);
CREATE TABLE webauthn_challenges (
    id text NOT NULL,
    user_id text,
    challenge_type text NOT NULL,
    state_json text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    CONSTRAINT webauthn_challenges_challenge_type_check CHECK ((challenge_type = ANY (ARRAY['registration'::text, 'authentication'::text])))
);
CREATE TABLE webauthn_credentials (
    id text NOT NULL,
    user_id text NOT NULL,
    credential_id text NOT NULL,
    credential_json text NOT NULL,
    friendly_name text,
    created_at timestamp with time zone NOT NULL,
    last_used_at timestamp with time zone
);
CREATE TABLE workflow_operations (
    id text NOT NULL,
    operation_type text NOT NULL,
    status text DEFAULT 'queued'::text NOT NULL,
    job_key text,
    trigger_source text,
    actor_user_id text,
    progress_json jsonb,
    summary_json jsonb,
    summary_text text,
    error_text text,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    title_id text,
    collection_id text,
    episode_id text,
    release_id text,
    media_file_id text,
    external_reference text,
    series_movie_link_id text
);
ALTER TABLE ONLY domain_events ALTER COLUMN sequence SET DEFAULT nextval('domain_events_sequence_seq'::regclass);
ALTER TABLE ONLY title_search_terms ALTER COLUMN term_id SET DEFAULT nextval('title_search_terms_term_id_seq'::regclass);
ALTER TABLE ONLY blocklist
    ADD CONSTRAINT blocklist_pkey PRIMARY KEY (id);
ALTER TABLE ONLY collection_external_ids
    ADD CONSTRAINT collection_external_ids_pkey PRIMARY KEY (id);
ALTER TABLE ONLY collections
    ADD CONSTRAINT collections_pkey PRIMARY KEY (id);
ALTER TABLE ONLY discovery_facets
    ADD CONSTRAINT discovery_facets_pkey PRIMARY KEY (run_id, facet_name, facet_value);
ALTER TABLE ONLY discovery_item_library_provenance
    ADD CONSTRAINT discovery_item_library_proven_item_id_subject_key_title_id__key UNIQUE (item_id, subject_key, title_id, library_id);
ALTER TABLE ONLY discovery_item_rank_components
    ADD CONSTRAINT discovery_item_rank_components_item_id_component_index_key UNIQUE (item_id, component_index);
ALTER TABLE ONLY discovery_item_subject_links
    ADD CONSTRAINT discovery_item_subject_links_item_id_link_type_subject_key_key UNIQUE (item_id, link_type, subject_key);
ALTER TABLE ONLY discovery_items
    ADD CONSTRAINT discovery_items_pkey PRIMARY KEY (id);
ALTER TABLE ONLY discovery_pending_context_changes
    ADD CONSTRAINT discovery_pending_context_changes_pkey PRIMARY KEY (id);
ALTER TABLE ONLY discovery_raw_pages
    ADD CONSTRAINT discovery_raw_pages_pkey PRIMARY KEY (run_id, payload_kind, page_number);
ALTER TABLE ONLY discovery_section_items
    ADD CONSTRAINT discovery_section_items_pkey PRIMARY KEY (run_id, section_id, item_id);
ALTER TABLE ONLY discovery_sections
    ADD CONSTRAINT discovery_sections_pkey PRIMARY KEY (id);
ALTER TABLE ONLY discovery_sync_runs
    ADD CONSTRAINT discovery_sync_runs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY discovery_sync_state
    ADD CONSTRAINT discovery_sync_state_pkey PRIMARY KEY (scope_key);
ALTER TABLE ONLY discovery_title_external_ids
    ADD CONSTRAINT discovery_title_external_ids_discovery_title_id_source_exte_key UNIQUE (discovery_title_id, source, external_kind, external_identity);
ALTER TABLE ONLY discovery_title_metadata_external_ratings
    ADD CONSTRAINT discovery_title_metadata_external_ratings_pkey PRIMARY KEY (discovery_title_id, source);
ALTER TABLE ONLY discovery_title_metadata_rating_sources
    ADD CONSTRAINT discovery_title_metadata_rating_sources_pkey PRIMARY KEY (discovery_title_id, source);
ALTER TABLE ONLY discovery_title_metadata_rating_summaries
    ADD CONSTRAINT discovery_title_metadata_rating_summaries_pkey PRIMARY KEY (discovery_title_id);
ALTER TABLE ONLY discovery_title_metadata_tag_source_keys
    ADD CONSTRAINT discovery_title_metadata_tag__discovery_title_id_tag_key_s_key1 UNIQUE (discovery_title_id, tag_key, source_tag_key);
ALTER TABLE ONLY discovery_title_metadata_tag_sources
    ADD CONSTRAINT discovery_title_metadata_tag__discovery_title_id_tag_key_so_key UNIQUE (discovery_title_id, tag_key, source);
ALTER TABLE ONLY discovery_title_metadata_tags
    ADD CONSTRAINT discovery_title_metadata_tags_pkey PRIMARY KEY (discovery_title_id, tag_key);
ALTER TABLE ONLY discovery_title_source_tag_values
    ADD CONSTRAINT discovery_title_source_tag_va_discovery_title_id_source_tag_key UNIQUE (discovery_title_id, source_tag_sort_index, source_tag_value);
ALTER TABLE ONLY discovery_title_source_tags
    ADD CONSTRAINT discovery_title_source_tags_discovery_title_id_sort_index_c_key UNIQUE (discovery_title_id, sort_index, category, name);
ALTER TABLE ONLY discovery_title_terms
    ADD CONSTRAINT discovery_title_terms_discovery_title_id_term_kind_term_cat_key UNIQUE (discovery_title_id, term_kind, term_category, term_value);
ALTER TABLE ONLY discovery_titles
    ADD CONSTRAINT discovery_titles_pkey PRIMARY KEY (id);
ALTER TABLE ONLY discovery_titles
    ADD CONSTRAINT discovery_titles_target_key_norm_language_key UNIQUE (target_key_norm, language);
ALTER TABLE ONLY domain_events
    ADD CONSTRAINT domain_events_event_id_key UNIQUE (event_id);
ALTER TABLE ONLY domain_events
    ADD CONSTRAINT domain_events_pkey PRIMARY KEY (sequence);
ALTER TABLE ONLY download_clients
    ADD CONSTRAINT download_clients_pkey PRIMARY KEY (id);
ALTER TABLE ONLY download_identity_states
    ADD CONSTRAINT download_identity_states_identity_key_key UNIQUE (identity_key);
ALTER TABLE ONLY download_identity_states
    ADD CONSTRAINT download_identity_states_pkey PRIMARY KEY (id);
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_pkey PRIMARY KEY (id);
ALTER TABLE ONLY download_queue_commands
    ADD CONSTRAINT download_queue_commands_pkey PRIMARY KEY (id);
ALTER TABLE ONLY download_submission_episode_links
    ADD CONSTRAINT download_submission_episode_links_pkey PRIMARY KEY (download_client_id, download_client_type, download_client_item_id, episode_id);
ALTER TABLE ONLY download_submissions
    ADD CONSTRAINT download_submissions_download_client_id_download_client_typ_key UNIQUE (download_client_id, download_client_type, download_client_item_id);
ALTER TABLE ONLY download_submissions
    ADD CONSTRAINT download_submissions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY emby_media_server_details
    ADD CONSTRAINT emby_media_server_details_pkey PRIMARY KEY (connection_id);
ALTER TABLE ONLY episode_external_ids
    ADD CONSTRAINT episode_external_ids_pkey PRIMARY KEY (id);
ALTER TABLE ONLY episodes
    ADD CONSTRAINT episodes_pkey PRIMARY KEY (id);
ALTER TABLE ONLY event_outboxes
    ADD CONSTRAINT event_outboxes_pkey PRIMARY KEY (id);
ALTER TABLE ONLY event_subscriber_offsets
    ADD CONSTRAINT event_subscriber_offsets_pkey PRIMARY KEY (subscriber_name);
ALTER TABLE ONLY external_import_monitor_snapshot_chunks
    ADD CONSTRAINT external_import_monitor_snapshot_chunks_pkey PRIMARY KEY (session_id, facet, entry_kind, chunk_index);
ALTER TABLE ONLY external_import_setup_download_client_api_key_overrides
    ADD CONSTRAINT external_import_setup_download_client_ap_draft_key_position_key UNIQUE (draft_key, "position");
ALTER TABLE ONLY external_import_setup_download_client_api_key_overrides
    ADD CONSTRAINT external_import_setup_download_client_api_key_overrides_pkey PRIMARY KEY (draft_key, dedup_key);
ALTER TABLE ONLY external_import_setup_download_client_password_overrides
    ADD CONSTRAINT external_import_setup_download_client_pa_draft_key_position_key UNIQUE (draft_key, "position");
ALTER TABLE ONLY external_import_setup_download_client_password_overrides
    ADD CONSTRAINT external_import_setup_download_client_password_overrides_pkey PRIMARY KEY (draft_key, dedup_key);
ALTER TABLE ONLY external_import_setup_indexer_api_key_overrides
    ADD CONSTRAINT external_import_setup_indexer_api_key_ov_draft_key_position_key UNIQUE (draft_key, "position");
ALTER TABLE ONLY external_import_setup_indexer_api_key_overrides
    ADD CONSTRAINT external_import_setup_indexer_api_key_overrides_pkey PRIMARY KEY (draft_key, dedup_key);
ALTER TABLE ONLY external_import_setup_instance_api_keys
    ADD CONSTRAINT external_import_setup_instance_api_keys_draft_key_position_key UNIQUE (draft_key, "position");
ALTER TABLE ONLY external_import_setup_instance_api_keys
    ADD CONSTRAINT external_import_setup_instance_api_keys_pkey PRIMARY KEY (draft_key, instance_id);
ALTER TABLE ONLY external_import_setup_secret_drafts
    ADD CONSTRAINT external_import_setup_secret_drafts_pkey PRIMARY KEY (draft_key);
ALTER TABLE ONLY external_subtitle_probe_cache
    ADD CONSTRAINT external_subtitle_probe_cache_pkey PRIMARY KEY (media_file_id, file_path);
ALTER TABLE ONLY file_episode_map
    ADD CONSTRAINT file_episode_map_pkey PRIMARY KEY (file_id, episode_id);
ALTER TABLE ONLY file_series_movie_link_map
    ADD CONSTRAINT file_series_movie_link_map_pkey PRIMARY KEY (file_id, series_movie_link_id);
ALTER TABLE ONLY history_events
    ADD CONSTRAINT history_events_pkey PRIMARY KEY (id);
ALTER TABLE ONLY imports
    ADD CONSTRAINT imports_pkey PRIMARY KEY (id);
ALTER TABLE ONLY indexer_api_quotas
    ADD CONSTRAINT indexer_api_quotas_pkey PRIMARY KEY (indexer_id);
ALTER TABLE ONLY indexer_search_learning
    ADD CONSTRAINT indexer_search_learning_pkey PRIMARY KEY (indexer_id, title_id, facet, strategy_key);
ALTER TABLE ONLY indexer_system_backoffs
    ADD CONSTRAINT indexer_system_backoffs_pkey PRIMARY KEY (indexer_id);
ALTER TABLE ONLY indexers
    ADD CONSTRAINT indexers_pkey PRIMARY KEY (id);
ALTER TABLE ONLY jellyfin_media_server_details
    ADD CONSTRAINT jellyfin_media_server_details_pkey PRIMARY KEY (connection_id);
ALTER TABLE ONLY libraries
    ADD CONSTRAINT libraries_pkey PRIMARY KEY (id);
ALTER TABLE ONLY library_probe_signatures
    ADD CONSTRAINT library_probe_signatures_pkey PRIMARY KEY (title_id);
ALTER TABLE ONLY library_roots
    ADD CONSTRAINT library_roots_pkey PRIMARY KEY (id);
ALTER TABLE ONLY library_scan_unmatched_items
    ADD CONSTRAINT library_scan_unmatched_items_pkey PRIMARY KEY (id);
ALTER TABLE ONLY media_files
    ADD CONSTRAINT media_files_file_path_key UNIQUE (file_path);
ALTER TABLE ONLY media_files
    ADD CONSTRAINT media_files_pkey PRIMARY KEY (id);
ALTER TABLE ONLY media_request_external_ids
    ADD CONSTRAINT media_request_external_ids_pkey PRIMARY KEY (request_id, source, external_id);
ALTER TABLE ONLY media_request_requesters
    ADD CONSTRAINT media_request_requesters_pkey PRIMARY KEY (request_id, user_id);
ALTER TABLE ONLY media_requests
    ADD CONSTRAINT media_requests_pkey PRIMARY KEY (id);
ALTER TABLE ONLY media_server_connections
    ADD CONSTRAINT media_server_connections_pkey PRIMARY KEY (id);
ALTER TABLE ONLY media_server_default_library_grants
    ADD CONSTRAINT media_server_default_library_grants_pkey PRIMARY KEY (connection_id, library_id);
ALTER TABLE ONLY media_server_path_mappings
    ADD CONSTRAINT media_server_path_mappings_pkey PRIMARY KEY (id);
ALTER TABLE ONLY movie_entities
    ADD CONSTRAINT movie_entities_pkey PRIMARY KEY (id);
ALTER TABLE ONLY notification_channels
    ADD CONSTRAINT notification_channels_pkey PRIMARY KEY (id);
ALTER TABLE ONLY notification_subscriptions
    ADD CONSTRAINT notification_subscriptions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY oauth_authorization_codes
    ADD CONSTRAINT oauth_authorization_codes_code_hash_key UNIQUE (code_hash);
ALTER TABLE ONLY oauth_authorization_codes
    ADD CONSTRAINT oauth_authorization_codes_pkey PRIMARY KEY (id);
ALTER TABLE ONLY oauth_refresh_grants
    ADD CONSTRAINT oauth_refresh_grants_pkey PRIMARY KEY (id);
ALTER TABLE ONLY oauth_refresh_tokens
    ADD CONSTRAINT oauth_refresh_tokens_pkey PRIMARY KEY (id);
ALTER TABLE ONLY oauth_refresh_tokens
    ADD CONSTRAINT oauth_refresh_tokens_token_hash_key UNIQUE (token_hash);
ALTER TABLE ONLY pending_releases
    ADD CONSTRAINT pending_releases_pkey PRIMARY KEY (id);
ALTER TABLE ONLY plex_media_server_details
    ADD CONSTRAINT plex_media_server_details_pkey PRIMARY KEY (connection_id);
ALTER TABLE ONLY plugin_catalog_sources
    ADD CONSTRAINT plugin_catalog_sources_pkey PRIMARY KEY (source_key);
ALTER TABLE ONLY plugin_catalog_status
    ADD CONSTRAINT plugin_catalog_status_pkey PRIMARY KEY (status_key);
ALTER TABLE ONLY plugin_installations
    ADD CONSTRAINT plugin_installations_pkey PRIMARY KEY (id);
ALTER TABLE ONLY plugin_installations
    ADD CONSTRAINT plugin_installations_plugin_id_key UNIQUE (plugin_id);
ALTER TABLE ONLY post_processing_script_runs
    ADD CONSTRAINT post_processing_script_runs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY post_processing_scripts
    ADD CONSTRAINT post_processing_scripts_pkey PRIMARY KEY (id);
ALTER TABLE ONLY quality_profile_audio_codec_allowlist
    ADD CONSTRAINT quality_profile_audio_codec_allowlist_pkey PRIMARY KEY (profile_id, codec);
ALTER TABLE ONLY quality_profile_audio_codec_blocklist
    ADD CONSTRAINT quality_profile_audio_codec_blocklist_pkey PRIMARY KEY (profile_id, codec);
ALTER TABLE ONLY quality_profile_quality_tiers
    ADD CONSTRAINT quality_profile_quality_tiers_pkey PRIMARY KEY (profile_id, quality_tier);
ALTER TABLE ONLY quality_profile_source_allowlist
    ADD CONSTRAINT quality_profile_source_allowlist_pkey PRIMARY KEY (profile_id, source);
ALTER TABLE ONLY quality_profile_source_blocklist
    ADD CONSTRAINT quality_profile_source_blocklist_pkey PRIMARY KEY (profile_id, source);
ALTER TABLE ONLY quality_profile_video_codec_allowlist
    ADD CONSTRAINT quality_profile_video_codec_allowlist_pkey PRIMARY KEY (profile_id, codec);
ALTER TABLE ONLY quality_profile_video_codec_blocklist
    ADD CONSTRAINT quality_profile_video_codec_blocklist_pkey PRIMARY KEY (profile_id, codec);
ALTER TABLE ONLY quality_profiles
    ADD CONSTRAINT quality_profiles_pkey PRIMARY KEY (id);
ALTER TABLE ONLY release_decisions
    ADD CONSTRAINT release_decisions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY release_download_attempts
    ADD CONSTRAINT release_download_attempts_pkey PRIMARY KEY (id);
ALTER TABLE ONLY rule_set_history
    ADD CONSTRAINT rule_set_history_pkey PRIMARY KEY (id);
ALTER TABLE ONLY rule_sets
    ADD CONSTRAINT rule_sets_pkey PRIMARY KEY (id);
ALTER TABLE ONLY series_movie_links
    ADD CONSTRAINT series_movie_links_legacy_collection_id_key UNIQUE (legacy_collection_id);
ALTER TABLE ONLY series_movie_links
    ADD CONSTRAINT series_movie_links_pkey PRIMARY KEY (id);
ALTER TABLE ONLY settings_definitions
    ADD CONSTRAINT settings_definitions_category_scope_key_name_key UNIQUE (category, scope, key_name);
ALTER TABLE ONLY settings_definitions
    ADD CONSTRAINT settings_definitions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY settings_values
    ADD CONSTRAINT settings_values_pkey PRIMARY KEY (id);
ALTER TABLE ONLY subtitle_blocklist
    ADD CONSTRAINT subtitle_blocklist_media_file_provider_provider_file_id_key UNIQUE (media_file_id, provider, provider_file_id);
ALTER TABLE ONLY subtitle_blocklist
    ADD CONSTRAINT subtitle_blocklist_pkey PRIMARY KEY (id);
ALTER TABLE ONLY subtitle_downloads
    ADD CONSTRAINT subtitle_downloads_pkey PRIMARY KEY (id);
ALTER TABLE ONLY subtitle_provider_configs
    ADD CONSTRAINT subtitle_provider_configs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY title_external_ids
    ADD CONSTRAINT title_external_ids_pkey PRIMARY KEY (id);
ALTER TABLE ONLY title_image_blobs
    ADD CONSTRAINT title_image_blobs_pkey PRIMARY KEY (digest);
ALTER TABLE ONLY title_image_variants
    ADD CONSTRAINT title_image_variants_pkey PRIMARY KEY (id);
ALTER TABLE ONLY title_image_variants
    ADD CONSTRAINT title_image_variants_title_image_id_variant_key_key UNIQUE (title_image_id, variant_key);
ALTER TABLE ONLY title_images
    ADD CONSTRAINT title_images_pkey PRIMARY KEY (id);
ALTER TABLE ONLY title_images
    ADD CONSTRAINT title_images_title_id_kind_key UNIQUE (title_id, kind);
ALTER TABLE ONLY title_metadata_external_ratings
    ADD CONSTRAINT title_metadata_external_ratings_pkey PRIMARY KEY (title_id, source);
ALTER TABLE ONLY title_metadata_rating_sources
    ADD CONSTRAINT title_metadata_rating_sources_pkey PRIMARY KEY (title_id, source);
ALTER TABLE ONLY title_metadata_rating_summaries
    ADD CONSTRAINT title_metadata_rating_summaries_pkey PRIMARY KEY (title_id);
ALTER TABLE ONLY title_metadata_tag_source_keys
    ADD CONSTRAINT title_metadata_tag_source_key_title_id_tag_key_source_tag_k_key UNIQUE (title_id, tag_key, source_tag_key);
ALTER TABLE ONLY title_metadata_tag_sources
    ADD CONSTRAINT title_metadata_tag_sources_title_id_tag_key_source_key UNIQUE (title_id, tag_key, source);
ALTER TABLE ONLY title_metadata_tags
    ADD CONSTRAINT title_metadata_tags_pkey PRIMARY KEY (title_id, tag_key);
ALTER TABLE ONLY title_more_like_this_items
    ADD CONSTRAINT title_more_like_this_items_source_title_id_discovery_title__key UNIQUE (source_title_id, discovery_title_id);
ALTER TABLE ONLY title_search_terms
    ADD CONSTRAINT title_search_terms_pkey PRIMARY KEY (term_id);
ALTER TABLE ONLY titles
    ADD CONSTRAINT titles_pkey PRIMARY KEY (id);
ALTER TABLE ONLY totp_credentials
    ADD CONSTRAINT totp_credentials_pkey PRIMARY KEY (id);
ALTER TABLE ONLY totp_credentials
    ADD CONSTRAINT totp_credentials_user_id_key UNIQUE (user_id);
ALTER TABLE ONLY totp_enrollment_challenges
    ADD CONSTRAINT totp_enrollment_challenges_pkey PRIMARY KEY (id);
ALTER TABLE ONLY totp_failed_attempts
    ADD CONSTRAINT totp_failed_attempts_pkey PRIMARY KEY (id);
ALTER TABLE ONLY totp_recovery_codes
    ADD CONSTRAINT totp_recovery_codes_pkey PRIMARY KEY (id);
ALTER TABLE ONLY upstream_destination_cooldowns
    ADD CONSTRAINT upstream_destination_cooldowns_pkey PRIMARY KEY (destination_key);
ALTER TABLE ONLY upstream_scheduler_rss_cadence
    ADD CONSTRAINT upstream_scheduler_rss_cadence_pkey PRIMARY KEY (host_key, destination_key, account_quota_key, rss_request_key);
ALTER TABLE ONLY upstream_scheduler_states
    ADD CONSTRAINT upstream_scheduler_states_pkey PRIMARY KEY (host_key, destination_key, account_quota_key, rss_request_key);
ALTER TABLE ONLY user_app_permission_masks
    ADD CONSTRAINT user_app_permission_masks_pkey PRIMARY KEY (user_id);
ALTER TABLE ONLY user_external_accounts
    ADD CONSTRAINT user_external_accounts_pkey PRIMARY KEY (id);
ALTER TABLE ONLY user_library_permission_masks
    ADD CONSTRAINT user_library_permission_masks_pkey PRIMARY KEY (user_id, library_id);
ALTER TABLE ONLY user_ui_settings
    ADD CONSTRAINT user_ui_settings_pkey PRIMARY KEY (user_id);
ALTER TABLE ONLY user_ui_table_columns
    ADD CONSTRAINT user_ui_table_columns_pkey PRIMARY KEY (user_id, facet, table_view_mode, column_id);
ALTER TABLE ONLY users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);
ALTER TABLE ONLY users
    ADD CONSTRAINT users_username_key UNIQUE (username);
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_pkey PRIMARY KEY (id);
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_title_episode_key UNIQUE (title_id, episode_id);
ALTER TABLE ONLY webauthn_challenges
    ADD CONSTRAINT webauthn_challenges_pkey PRIMARY KEY (id);
ALTER TABLE ONLY webauthn_credentials
    ADD CONSTRAINT webauthn_credentials_credential_id_key UNIQUE (credential_id);
ALTER TABLE ONLY webauthn_credentials
    ADD CONSTRAINT webauthn_credentials_pkey PRIMARY KEY (id);
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_pkey PRIMARY KEY (id);
CREATE INDEX idx_blocklist_source_title ON blocklist USING btree (source_title) WHERE (source_title IS NOT NULL);
CREATE INDEX idx_blocklist_title_id ON blocklist USING btree (title_id);
CREATE INDEX idx_collection_external_ids_title_provenance ON collection_external_ids USING btree (title_id, provenance);
CREATE UNIQUE INDEX idx_collection_external_ids_unique ON collection_external_ids USING btree (collection_id, source, external_id, provenance, source_scope);
CREATE INDEX idx_collections_title ON collections USING btree (title_id, collection_type);
CREATE INDEX idx_discovery_item_library_provenance_item ON discovery_item_library_provenance USING btree (item_id, subject_key, library_id, title_id);
CREATE INDEX idx_discovery_item_library_provenance_library ON discovery_item_library_provenance USING btree (run_id, library_id, item_id);
CREATE INDEX idx_discovery_item_rank_components_item ON discovery_item_rank_components USING btree (item_id, component_index);
CREATE INDEX idx_discovery_item_subject_links_item ON discovery_item_subject_links USING btree (item_id, link_type, sort_index);
CREATE INDEX idx_discovery_item_subject_links_run_type_key ON discovery_item_subject_links USING btree (run_id, link_type, subject_key, item_id);
CREATE INDEX idx_discovery_items_active_title ON discovery_items USING btree (base_generation_id, discovery_title_id, tombstoned_at);
CREATE INDEX idx_discovery_items_run ON discovery_items USING btree (run_id);
CREATE INDEX idx_discovery_items_run_section ON discovery_items USING btree (run_id, section_id, sort_index);
CREATE INDEX idx_discovery_items_section ON discovery_items USING btree (section_id, sort_index, rank_score);
CREATE INDEX idx_discovery_pending_changes_scope_seen ON discovery_pending_context_changes USING btree (scope_key, last_seen_at);
CREATE INDEX idx_discovery_pending_changes_scope_sequence ON discovery_pending_context_changes USING btree (scope_key, last_seen_sequence);
CREATE INDEX idx_discovery_raw_pages_run ON discovery_raw_pages USING btree (run_id, payload_kind, page_number);
CREATE INDEX idx_discovery_section_items_run_section ON discovery_section_items USING btree (run_id, section_id, sort_index);
CREATE INDEX idx_discovery_sections_run_surface ON discovery_sections USING btree (run_id, surface, sort_index);
CREATE INDEX idx_discovery_submitted_subjects_run_key ON discovery_submitted_subjects USING btree (run_id, subject_key, library_id, title_id);
CREATE INDEX idx_discovery_submitted_subjects_title ON discovery_submitted_subjects USING btree (title_id);
CREATE INDEX idx_discovery_sync_runs_kind_status ON discovery_sync_runs USING btree (kind, status, updated_at);
CREATE INDEX idx_discovery_title_external_ids_title ON discovery_title_external_ids USING btree (discovery_title_id, sort_index);
CREATE INDEX idx_discovery_title_metadata_external_ratings_order ON discovery_title_metadata_external_ratings USING btree (discovery_title_id, sort_index, source);
CREATE INDEX idx_discovery_title_metadata_external_ratings_source_norm ON discovery_title_metadata_external_ratings USING btree (source, normalized, discovery_title_id);
CREATE INDEX idx_discovery_title_metadata_rating_sources_order ON discovery_title_metadata_rating_sources USING btree (discovery_title_id, sort_index, source);
CREATE INDEX idx_discovery_title_metadata_tags_category_name ON discovery_title_metadata_tags USING btree (category, name, discovery_title_id);
CREATE INDEX idx_discovery_title_source_tag_values_title ON discovery_title_source_tag_values USING btree (discovery_title_id, source_tag_sort_index, value_sort_index);
CREATE INDEX idx_discovery_title_source_tags_title ON discovery_title_source_tags USING btree (discovery_title_id, sort_index);
CREATE INDEX idx_discovery_title_terms_kind_value ON discovery_title_terms USING btree (term_kind, term_value, discovery_title_id);
CREATE INDEX idx_discovery_title_terms_title ON discovery_title_terms USING btree (discovery_title_id, term_kind, sort_index);
CREATE INDEX idx_discovery_titles_key_language ON discovery_titles USING btree (target_key_norm, language);
CREATE INDEX idx_domain_events_event_type_sequence ON domain_events USING btree (event_type, sequence DESC);
CREATE INDEX idx_domain_events_facet_sequence ON domain_events USING btree (facet, sequence DESC);
CREATE INDEX idx_domain_events_occurred_at ON domain_events USING btree (occurred_at DESC);
CREATE INDEX idx_domain_events_stream_sequence ON domain_events USING btree (stream_kind, stream_id, sequence DESC);
CREATE INDEX idx_domain_events_title_sequence ON domain_events USING btree (title_id, sequence DESC);
CREATE INDEX idx_download_clients_client_priority ON download_clients USING btree (client_priority);
CREATE UNIQUE INDEX idx_download_clients_name ON download_clients USING btree (name);
CREATE INDEX idx_download_identity_states_download_id ON download_identity_states USING btree (client_id, client_type, download_id);
CREATE INDEX idx_download_import_artifacts_episode ON download_import_artifacts USING btree (episode_id, result);
CREATE INDEX idx_download_import_artifacts_retention ON download_import_artifacts USING btree (created_at, import_id);
CREATE INDEX idx_download_import_artifacts_source ON download_import_artifacts USING btree (COALESCE(source_client_id, ''::text), source_system, source_ref, created_at);
CREATE UNIQUE INDEX idx_download_queue_commands_active_unique ON download_queue_commands USING btree (action, COALESCE(client_id, ''::text), client_type, download_client_item_id, is_history) WHERE (status = ANY (ARRAY['queued'::text, 'running'::text]));
CREATE INDEX idx_download_queue_commands_source ON download_queue_commands USING btree (COALESCE(client_id, ''::text), client_type, download_client_item_id, is_history, created_at DESC);
CREATE INDEX idx_download_queue_commands_status ON download_queue_commands USING btree (action, status, updated_at);
CREATE INDEX idx_download_submission_episode_links_episode ON download_submission_episode_links USING btree (episode_id);
CREATE INDEX idx_download_submissions_download_id ON download_submissions USING btree (download_client_id, download_client_type, download_id);
CREATE INDEX idx_download_submissions_title_request_signature ON download_submissions USING btree (title_id, request_signature);
CREATE INDEX idx_episode_external_ids_title_provenance ON episode_external_ids USING btree (title_id, provenance);
CREATE UNIQUE INDEX idx_episode_external_ids_unique ON episode_external_ids USING btree (episode_id, source, external_id, provenance, source_scope);
CREATE INDEX idx_episodes_collection ON episodes USING btree (collection_id);
CREATE INDEX idx_episodes_title ON episodes USING btree (title_id, season_number);
CREATE INDEX idx_event_outboxes_channel ON event_outboxes USING btree (channel_key);
CREATE INDEX idx_event_outboxes_status ON event_outboxes USING btree (status, updated_at);
CREATE INDEX idx_external_subtitle_probe_cache_file_path ON external_subtitle_probe_cache USING btree (file_path);
CREATE INDEX idx_external_subtitle_probe_cache_media_file ON external_subtitle_probe_cache USING btree (media_file_id);
CREATE INDEX idx_file_episode_map_episode ON file_episode_map USING btree (episode_id);
CREATE INDEX idx_file_series_movie_link_map_link ON file_series_movie_link_map USING btree (series_movie_link_id);
CREATE INDEX idx_history_events_occurred_at ON history_events USING btree (occurred_at DESC);
CREATE INDEX idx_history_events_title_time ON history_events USING btree (title_id, occurred_at DESC);
CREATE INDEX idx_history_events_type_time ON history_events USING btree (event_type, occurred_at DESC);
CREATE INDEX idx_history_title_time ON history_events USING btree (title_id, occurred_at DESC);
CREATE INDEX idx_history_type_time ON history_events USING btree (event_type, occurred_at DESC);
CREATE UNIQUE INDEX idx_imports_active_download_id ON imports USING btree (COALESCE(source_client_id, ''::text), source_system, download_id) WHERE ((download_id IS NOT NULL) AND (status = ANY (ARRAY['pending'::text, 'running'::text, 'processing'::text])));
CREATE INDEX idx_imports_download_id ON imports USING btree (source_client_id, source_system, download_id);
CREATE UNIQUE INDEX idx_imports_source_ref ON imports USING btree (COALESCE(source_client_id, ''::text), source_system, source_ref, import_type) WHERE (download_id IS NULL);
CREATE INDEX idx_imports_status_updated_at ON imports USING btree (status, updated_at);
CREATE INDEX idx_indexer_search_learning_title ON indexer_search_learning USING btree (indexer_id, title_id, facet);
CREATE INDEX idx_indexer_system_backoffs_disabled_until ON indexer_system_backoffs USING btree (disabled_until);
CREATE UNIQUE INDEX idx_indexers_managed_child_identity ON indexers USING btree (managed_parent_config_id, managed_child_key) WHERE ((managed_parent_config_id IS NOT NULL) AND (managed_child_key IS NOT NULL));
CREATE INDEX idx_indexers_managed_parent ON indexers USING btree (managed_parent_config_id);
CREATE UNIQUE INDEX idx_libraries_facet_slug ON libraries USING btree (facet, slug);
CREATE INDEX idx_library_probe_signatures_last_probed ON library_probe_signatures USING btree (last_probed_at DESC);
CREATE INDEX idx_library_roots_library ON library_roots USING btree (library_id, is_default DESC, path);
CREATE UNIQUE INDEX idx_library_roots_normalized_path ON library_roots USING btree (normalized_path);
CREATE INDEX idx_library_scan_unmatched_items_facet_status_updated ON library_scan_unmatched_items USING btree (facet, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_facet_title_status_updated ON library_scan_unmatched_items USING btree (facet, title_id, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_facet_updated ON library_scan_unmatched_items USING btree (facet, updated_at DESC);
CREATE UNIQUE INDEX idx_library_scan_unmatched_items_library_path ON library_scan_unmatched_items USING btree (library_id, item_path);
CREATE INDEX idx_library_scan_unmatched_items_library_updated ON library_scan_unmatched_items USING btree (library_id, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_root_status_updated ON library_scan_unmatched_items USING btree (facet, scan_root, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_root_updated ON library_scan_unmatched_items USING btree (facet, scan_root, updated_at DESC);
CREATE INDEX idx_media_files_title ON media_files USING btree (title_id);
CREATE INDEX idx_media_files_title_path ON media_files USING btree (title_id, file_path);
CREATE INDEX idx_media_request_external_ids_lookup ON media_request_external_ids USING btree (library_id, source, external_id);
CREATE INDEX idx_media_request_requesters_user ON media_request_requesters USING btree (user_id);
CREATE INDEX idx_media_requests_created_title ON media_requests USING btree (created_title_id);
CREATE INDEX idx_media_requests_library_facet_status ON media_requests USING btree (library_id, facet, status);
CREATE INDEX idx_media_requests_status_updated ON media_requests USING btree (status, updated_at);
CREATE INDEX idx_media_server_connections_provider ON media_server_connections USING btree (provider, enabled);
CREATE INDEX idx_media_server_path_mappings_connection ON media_server_path_mappings USING btree (connection_id, sort_order);
CREATE INDEX idx_movie_entities_anidb_id ON movie_entities USING btree (anidb_id) WHERE ((anidb_id IS NOT NULL) AND (anidb_id <> ''::text));
CREATE INDEX idx_movie_entities_imdb_id ON movie_entities USING btree (imdb_id) WHERE ((imdb_id IS NOT NULL) AND (imdb_id <> ''::text));
CREATE INDEX idx_movie_entities_mal_id ON movie_entities USING btree (mal_id) WHERE ((mal_id IS NOT NULL) AND (mal_id <> ''::text));
CREATE INDEX idx_movie_entities_tmdb_id ON movie_entities USING btree (tmdb_id) WHERE ((tmdb_id IS NOT NULL) AND (tmdb_id <> ''::text));
CREATE INDEX idx_movie_entities_tvdb_id ON movie_entities USING btree (tvdb_id) WHERE ((tvdb_id IS NOT NULL) AND (tvdb_id <> ''::text));
CREATE UNIQUE INDEX idx_notification_channels_name_type ON notification_channels USING btree (name, channel_type);
CREATE INDEX idx_notification_subscriptions_channel ON notification_subscriptions USING btree (channel_id) WHERE (channel_id IS NOT NULL);
CREATE INDEX idx_notification_subscriptions_target ON notification_subscriptions USING btree (target_kind, target_id);
CREATE UNIQUE INDEX idx_notification_subscriptions_target_scope ON notification_subscriptions USING btree (target_kind, target_id, event_type, COALESCE(scope, ''::text), COALESCE(scope_id, ''::text));
CREATE INDEX idx_oauth_authorization_codes_expires_at ON oauth_authorization_codes USING btree (expires_at);
CREATE INDEX idx_oauth_authorization_codes_user_id ON oauth_authorization_codes USING btree (user_id);
CREATE INDEX idx_oauth_refresh_grants_authorization_source ON oauth_refresh_grants USING btree (authorization_source);
CREATE INDEX idx_oauth_refresh_grants_family_id ON oauth_refresh_grants USING btree (family_id);
CREATE INDEX idx_oauth_refresh_grants_user_id ON oauth_refresh_grants USING btree (user_id);
CREATE INDEX idx_oauth_refresh_tokens_family_id ON oauth_refresh_tokens USING btree (family_id);
CREATE INDEX idx_oauth_refresh_tokens_grant_id ON oauth_refresh_tokens USING btree (grant_id);
CREATE INDEX idx_operations_status_time ON workflow_operations USING btree (status, started_at DESC);
CREATE INDEX idx_pending_releases_status ON pending_releases USING btree (status);
CREATE INDEX idx_pending_releases_wanted ON pending_releases USING btree (wanted_item_id, status);
CREATE INDEX idx_plugin_catalog_sources_kind ON plugin_catalog_sources USING btree (source_kind);
CREATE INDEX idx_pp_script_runs_script_id ON post_processing_script_runs USING btree (script_id, started_at DESC);
CREATE INDEX idx_pp_script_runs_title_id ON post_processing_script_runs USING btree (title_id, started_at DESC);
CREATE INDEX idx_quality_profile_audio_codec_allowlist_profile ON quality_profile_audio_codec_allowlist USING btree (profile_id);
CREATE INDEX idx_quality_profile_audio_codec_blocklist_profile ON quality_profile_audio_codec_blocklist USING btree (profile_id);
CREATE INDEX idx_quality_profile_quality_tiers_profile ON quality_profile_quality_tiers USING btree (profile_id, sort_order);
CREATE INDEX idx_quality_profile_source_allowlist_profile ON quality_profile_source_allowlist USING btree (profile_id);
CREATE INDEX idx_quality_profile_source_blocklist_profile ON quality_profile_source_blocklist USING btree (profile_id);
CREATE INDEX idx_quality_profile_video_codec_allowlist_profile ON quality_profile_video_codec_allowlist USING btree (profile_id);
CREATE INDEX idx_quality_profile_video_codec_blocklist_profile ON quality_profile_video_codec_blocklist USING btree (profile_id);
CREATE INDEX idx_quality_profiles_scope ON quality_profiles USING btree (scope, scope_id);
CREATE INDEX idx_release_decisions_created_at ON release_decisions USING btree (created_at DESC);
CREATE INDEX idx_release_decisions_wanted ON release_decisions USING btree (wanted_item_id, created_at DESC);
CREATE INDEX idx_release_download_attempts_outcome_attempted ON release_download_attempts USING btree (outcome, attempted_at DESC);
CREATE INDEX idx_release_download_attempts_source_hint ON release_download_attempts USING btree (source_hint);
CREATE INDEX idx_release_download_attempts_source_title ON release_download_attempts USING btree (source_title);
CREATE INDEX idx_rule_set_history_created_at ON rule_set_history USING btree (created_at DESC);
CREATE UNIQUE INDEX idx_rule_sets_managed_key ON rule_sets USING btree (managed_key) WHERE (managed_key IS NOT NULL);
CREATE UNIQUE INDEX idx_series_movie_links_legacy_collection ON series_movie_links USING btree (legacy_collection_id) WHERE (legacy_collection_id IS NOT NULL);
CREATE INDEX idx_series_movie_links_movie ON series_movie_links USING btree (movie_entity_id);
CREATE INDEX idx_series_movie_links_title ON series_movie_links USING btree (series_title_id);
CREATE UNIQUE INDEX idx_setting_values_scope_name ON settings_values USING btree (setting_definition_id, scope, COALESCE(scope_id, ''::text));
CREATE INDEX idx_settings_values_definition ON settings_values USING btree (setting_definition_id);
CREATE INDEX idx_subtitle_blocklist_media_file ON subtitle_blocklist USING btree (media_file_id);
CREATE INDEX idx_subtitle_downloads_language ON subtitle_downloads USING btree (language);
CREATE INDEX idx_subtitle_downloads_media_file ON subtitle_downloads USING btree (media_file_id);
CREATE INDEX idx_subtitle_downloads_title ON subtitle_downloads USING btree (title_id);
CREATE INDEX idx_subtitle_provider_configs_disabled_until ON subtitle_provider_configs USING btree (disabled_until);
CREATE INDEX idx_subtitle_provider_configs_enabled ON subtitle_provider_configs USING btree (is_enabled);
CREATE INDEX idx_subtitle_provider_configs_provider_type ON subtitle_provider_configs USING btree (provider_type);
CREATE UNIQUE INDEX idx_title_external_ids_library_lookup ON title_external_ids USING btree (library_id, source, external_id);
CREATE INDEX idx_title_external_ids_title_id ON title_external_ids USING btree (title_id);
CREATE INDEX idx_title_image_variants_blob_digest ON title_image_variants USING btree (blob_digest);
CREATE INDEX idx_title_image_variants_image_variant ON title_image_variants USING btree (title_image_id, variant_key);
CREATE INDEX idx_title_images_title_kind ON title_images USING btree (title_id, kind);
CREATE INDEX idx_title_metadata_external_ratings_order ON title_metadata_external_ratings USING btree (title_id, sort_index, source);
CREATE INDEX idx_title_metadata_external_ratings_source_norm ON title_metadata_external_ratings USING btree (source, normalized, title_id);
CREATE INDEX idx_title_metadata_rating_sources_order ON title_metadata_rating_sources USING btree (title_id, sort_index, source);
CREATE INDEX idx_title_metadata_tags_category_name ON title_metadata_tags USING btree (category, name, title_id);
CREATE INDEX idx_title_more_like_this_items_source_order ON title_more_like_this_items USING btree (source_title_id, sort_index, rank_score DESC);
CREATE INDEX idx_title_more_like_this_items_title ON title_more_like_this_items USING btree (discovery_title_id);
CREATE INDEX idx_title_search_terms_facet_normalized_term ON title_search_terms USING btree (facet, normalized_term);
CREATE INDEX idx_title_search_terms_normalized_term ON title_search_terms USING btree (normalized_term);
CREATE INDEX idx_title_search_terms_title_id ON title_search_terms USING btree (title_id);
CREATE UNIQUE INDEX idx_title_search_terms_title_kind_normalized ON title_search_terms USING btree (title_id, term_kind, normalized_term);
CREATE INDEX idx_titles_catalog_sort_key ON titles USING btree (catalog_sort_key, name, year, id);
CREATE INDEX idx_titles_facet_monitored ON titles USING btree (facet, monitored);
CREATE INDEX idx_titles_facet_normalized_slug ON titles USING btree (facet, lower(TRIM(BOTH FROM slug))) WHERE ((slug IS NOT NULL) AND (TRIM(BOTH FROM slug) <> ''::text));
CREATE INDEX idx_titles_library_name ON titles USING btree (library_id, lower(name), id);
CREATE INDEX idx_titles_metadata_hydration_due ON titles USING btree (metadata_hydration_next_attempt_at, metadata_fetched_at);
CREATE INDEX idx_titles_popularity ON titles USING btree (popularity);
CREATE INDEX idx_titles_root_folder_id ON titles USING btree (root_folder_id);
CREATE INDEX idx_totp_enrollment_challenges_expires_at ON totp_enrollment_challenges USING btree (expires_at);
CREATE INDEX idx_totp_enrollment_challenges_user_id ON totp_enrollment_challenges USING btree (user_id);
CREATE INDEX idx_totp_failed_attempts_user_id_attempted_at ON totp_failed_attempts USING btree (user_id, attempted_at);
CREATE INDEX idx_totp_recovery_codes_user_id ON totp_recovery_codes USING btree (user_id, used_at);
CREATE INDEX idx_upstream_destination_cooldowns_until ON upstream_destination_cooldowns USING btree (cooldown_until);
CREATE INDEX idx_upstream_scheduler_rss_latest_safe_poll ON upstream_scheduler_rss_cadence USING btree (latest_safe_poll_at);
CREATE INDEX idx_upstream_scheduler_states_destination ON upstream_scheduler_states USING btree (destination_key);
CREATE UNIQUE INDEX idx_user_external_accounts_pending_username ON user_external_accounts USING btree (provider, connection_id, lower(username)) WHERE ((status = 'pending_claim'::text) AND (external_user_id IS NULL));
CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity ON user_external_accounts USING btree (provider, connection_id, external_user_id);
CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection ON user_external_accounts USING btree (user_id, provider, connection_id);
CREATE INDEX idx_user_external_accounts_user_status ON user_external_accounts USING btree (user_id, status);
CREATE INDEX idx_user_ui_table_columns_user_view ON user_ui_table_columns USING btree (user_id, facet, table_view_mode, column_order);
CREATE UNIQUE INDEX idx_wanted_items_collection_id ON wanted_items USING btree (collection_id) WHERE (collection_id IS NOT NULL);
CREATE UNIQUE INDEX idx_wanted_items_movie_unique ON wanted_items USING btree (title_id) WHERE ((episode_id IS NULL) AND (collection_id IS NULL) AND (series_movie_link_id IS NULL));
CREATE INDEX idx_wanted_items_next_search ON wanted_items USING btree (status, next_search_at);
CREATE UNIQUE INDEX idx_wanted_items_series_movie_link ON wanted_items USING btree (series_movie_link_id) WHERE (series_movie_link_id IS NOT NULL);
CREATE INDEX idx_wanted_items_title ON wanted_items USING btree (title_id);
CREATE INDEX idx_webauthn_challenges_expires_at ON webauthn_challenges USING btree (expires_at);
CREATE INDEX idx_webauthn_challenges_user_id ON webauthn_challenges USING btree (user_id);
CREATE INDEX idx_webauthn_credentials_user_id_created_at ON webauthn_credentials USING btree (user_id, created_at DESC);
CREATE INDEX idx_workflow_operations_active_job_started ON workflow_operations USING btree (started_at) WHERE ((job_key IS NOT NULL) AND (status = ANY (ARRAY['queued'::text, 'running'::text, 'discovering'::text])));
CREATE INDEX idx_workflow_operations_actor_job_started ON workflow_operations USING btree (actor_user_id, job_key, started_at DESC) WHERE (job_key IS NOT NULL);
CREATE INDEX idx_workflow_operations_actor_recent_started ON workflow_operations USING btree (actor_user_id, started_at DESC) WHERE (job_key IS NOT NULL);
CREATE INDEX idx_workflow_operations_job_key_started ON workflow_operations USING btree (job_key, started_at DESC);
CREATE INDEX idx_workflow_operations_job_key_status ON workflow_operations USING btree (job_key, status, started_at DESC);
CREATE INDEX idx_workflow_operations_job_recent_started ON workflow_operations USING btree (started_at DESC) WHERE (job_key IS NOT NULL);
CREATE INDEX idx_workflow_operations_status_started ON workflow_operations USING btree (status, started_at);
ALTER TABLE ONLY blocklist
    ADD CONSTRAINT blocklist_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY collection_external_ids
    ADD CONSTRAINT collection_external_ids_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE;
ALTER TABLE ONLY collection_external_ids
    ADD CONSTRAINT collection_external_ids_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY collections
    ADD CONSTRAINT collections_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_facets
    ADD CONSTRAINT discovery_facets_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_sync_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_item_library_provenance
    ADD CONSTRAINT discovery_item_library_provenance_item_id_fkey FOREIGN KEY (item_id) REFERENCES discovery_items(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_item_library_provenance
    ADD CONSTRAINT discovery_item_library_provenance_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_sync_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_item_rank_components
    ADD CONSTRAINT discovery_item_rank_components_item_id_fkey FOREIGN KEY (item_id) REFERENCES discovery_items(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_item_rank_components
    ADD CONSTRAINT discovery_item_rank_components_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_sync_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_item_subject_links
    ADD CONSTRAINT discovery_item_subject_links_item_id_fkey FOREIGN KEY (item_id) REFERENCES discovery_items(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_item_subject_links
    ADD CONSTRAINT discovery_item_subject_links_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_sync_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_items
    ADD CONSTRAINT discovery_items_base_generation_id_fkey FOREIGN KEY (base_generation_id) REFERENCES discovery_sync_runs(id) ON DELETE SET NULL;
ALTER TABLE ONLY discovery_items
    ADD CONSTRAINT discovery_items_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_items
    ADD CONSTRAINT discovery_items_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_sync_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_items
    ADD CONSTRAINT discovery_items_tombstoned_by_run_id_fkey FOREIGN KEY (tombstoned_by_run_id) REFERENCES discovery_sync_runs(id) ON DELETE SET NULL;
ALTER TABLE ONLY discovery_pending_context_changes
    ADD CONSTRAINT discovery_pending_context_changes_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY discovery_raw_pages
    ADD CONSTRAINT discovery_raw_pages_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_sync_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_section_items
    ADD CONSTRAINT discovery_section_items_item_id_fkey FOREIGN KEY (item_id) REFERENCES discovery_items(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_section_items
    ADD CONSTRAINT discovery_section_items_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_sync_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_sections
    ADD CONSTRAINT discovery_sections_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_sync_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_submitted_subjects
    ADD CONSTRAINT discovery_submitted_subjects_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_sync_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_submitted_subjects
    ADD CONSTRAINT discovery_submitted_subjects_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY discovery_sync_runs
    ADD CONSTRAINT discovery_sync_runs_base_generation_id_fkey FOREIGN KEY (base_generation_id) REFERENCES discovery_sync_runs(id) ON DELETE SET NULL;
ALTER TABLE ONLY discovery_sync_state
    ADD CONSTRAINT discovery_sync_state_last_public_feed_generation_id_fkey FOREIGN KEY (last_public_feed_generation_id) REFERENCES discovery_sync_runs(id) ON DELETE SET NULL;
ALTER TABLE ONLY discovery_sync_state
    ADD CONSTRAINT discovery_sync_state_last_success_generation_id_fkey FOREIGN KEY (last_success_generation_id) REFERENCES discovery_sync_runs(id) ON DELETE SET NULL;
ALTER TABLE ONLY discovery_title_external_ids
    ADD CONSTRAINT discovery_title_external_ids_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_title_metadata_external_ratings
    ADD CONSTRAINT discovery_title_metadata_external_ratin_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_title_metadata_rating_sources
    ADD CONSTRAINT discovery_title_metadata_rating_sources_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_title_metadata_rating_summaries
    ADD CONSTRAINT discovery_title_metadata_rating_summari_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_title_metadata_tag_source_keys
    ADD CONSTRAINT discovery_title_metadata_tag_s_discovery_title_id_tag_key_fkey1 FOREIGN KEY (discovery_title_id, tag_key) REFERENCES discovery_title_metadata_tags(discovery_title_id, tag_key) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_title_metadata_tag_sources
    ADD CONSTRAINT discovery_title_metadata_tag_so_discovery_title_id_tag_key_fkey FOREIGN KEY (discovery_title_id, tag_key) REFERENCES discovery_title_metadata_tags(discovery_title_id, tag_key) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_title_metadata_tags
    ADD CONSTRAINT discovery_title_metadata_tags_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_title_source_tag_values
    ADD CONSTRAINT discovery_title_source_tag_values_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_title_source_tags
    ADD CONSTRAINT discovery_title_source_tags_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_title_terms
    ADD CONSTRAINT discovery_title_terms_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_titles
    ADD CONSTRAINT discovery_titles_resolved_title_id_fkey FOREIGN KEY (resolved_title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_import_id_fkey FOREIGN KEY (import_id) REFERENCES imports(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_imported_media_file_id_fkey FOREIGN KEY (imported_media_file_id) REFERENCES media_files(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_submission_episode_links
    ADD CONSTRAINT download_submission_episode_links_submission_fkey FOREIGN KEY (download_client_id, download_client_type, download_client_item_id) REFERENCES download_submissions(download_client_id, download_client_type, download_client_item_id) ON DELETE CASCADE;
ALTER TABLE ONLY emby_media_server_details
    ADD CONSTRAINT emby_media_server_details_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY episode_external_ids
    ADD CONSTRAINT episode_external_ids_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE;
ALTER TABLE ONLY episode_external_ids
    ADD CONSTRAINT episode_external_ids_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY episodes
    ADD CONSTRAINT episodes_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL;
ALTER TABLE ONLY episodes
    ADD CONSTRAINT episodes_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY event_outboxes
    ADD CONSTRAINT event_outboxes_history_event_id_fkey FOREIGN KEY (history_event_id) REFERENCES history_events(id) ON DELETE CASCADE;
ALTER TABLE ONLY external_import_setup_download_client_api_key_overrides
    ADD CONSTRAINT external_import_setup_download_client_api_key_ov_draft_key_fkey FOREIGN KEY (draft_key) REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE;
ALTER TABLE ONLY external_import_setup_download_client_password_overrides
    ADD CONSTRAINT external_import_setup_download_client_password_o_draft_key_fkey FOREIGN KEY (draft_key) REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE;
ALTER TABLE ONLY external_import_setup_indexer_api_key_overrides
    ADD CONSTRAINT external_import_setup_indexer_api_key_overrides_draft_key_fkey FOREIGN KEY (draft_key) REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE;
ALTER TABLE ONLY external_import_setup_instance_api_keys
    ADD CONSTRAINT external_import_setup_instance_api_keys_draft_key_fkey FOREIGN KEY (draft_key) REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE;
ALTER TABLE ONLY external_import_setup_secret_drafts
    ADD CONSTRAINT external_import_setup_secret_drafts_owner_user_id_fkey FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY external_subtitle_probe_cache
    ADD CONSTRAINT external_subtitle_probe_cache_media_file_id_fkey FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE CASCADE;
ALTER TABLE ONLY file_episode_map
    ADD CONSTRAINT file_episode_map_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE;
ALTER TABLE ONLY file_episode_map
    ADD CONSTRAINT file_episode_map_file_id_fkey FOREIGN KEY (file_id) REFERENCES media_files(id) ON DELETE CASCADE;
ALTER TABLE ONLY file_series_movie_link_map
    ADD CONSTRAINT file_series_movie_link_map_file_id_fkey FOREIGN KEY (file_id) REFERENCES media_files(id) ON DELETE CASCADE;
ALTER TABLE ONLY file_series_movie_link_map
    ADD CONSTRAINT file_series_movie_link_map_series_movie_link_id_fkey FOREIGN KEY (series_movie_link_id) REFERENCES series_movie_links(id) ON DELETE CASCADE;
ALTER TABLE ONLY history_events
    ADD CONSTRAINT history_events_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY history_events
    ADD CONSTRAINT history_events_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY indexer_system_backoffs
    ADD CONSTRAINT indexer_system_backoffs_indexer_id_fkey FOREIGN KEY (indexer_id) REFERENCES indexers(id) ON DELETE CASCADE;
ALTER TABLE ONLY jellyfin_media_server_details
    ADD CONSTRAINT jellyfin_media_server_details_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY library_probe_signatures
    ADD CONSTRAINT library_probe_signatures_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY library_roots
    ADD CONSTRAINT library_roots_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_files
    ADD CONSTRAINT media_files_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_request_external_ids
    ADD CONSTRAINT media_request_external_ids_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_request_external_ids
    ADD CONSTRAINT media_request_external_ids_request_id_fkey FOREIGN KEY (request_id) REFERENCES media_requests(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_request_requesters
    ADD CONSTRAINT media_request_requesters_request_id_fkey FOREIGN KEY (request_id) REFERENCES media_requests(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_request_requesters
    ADD CONSTRAINT media_request_requesters_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_requests
    ADD CONSTRAINT media_requests_created_by_user_id_fkey FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_requests
    ADD CONSTRAINT media_requests_created_title_id_fkey FOREIGN KEY (created_title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY media_requests
    ADD CONSTRAINT media_requests_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_requests
    ADD CONSTRAINT media_requests_resolved_by_user_id_fkey FOREIGN KEY (resolved_by_user_id) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY media_server_default_library_grants
    ADD CONSTRAINT media_server_default_library_grants_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_default_library_grants
    ADD CONSTRAINT media_server_default_library_grants_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_path_mappings
    ADD CONSTRAINT media_server_path_mappings_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY notification_subscriptions
    ADD CONSTRAINT notification_subscriptions_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES notification_channels(id) ON DELETE CASCADE;
ALTER TABLE ONLY oauth_authorization_codes
    ADD CONSTRAINT oauth_authorization_codes_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY oauth_refresh_grants
    ADD CONSTRAINT oauth_refresh_grants_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY oauth_refresh_tokens
    ADD CONSTRAINT oauth_refresh_tokens_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES oauth_refresh_grants(id) ON DELETE CASCADE;
ALTER TABLE ONLY plex_media_server_details
    ADD CONSTRAINT plex_media_server_details_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY post_processing_script_runs
    ADD CONSTRAINT post_processing_script_runs_script_id_fkey FOREIGN KEY (script_id) REFERENCES post_processing_scripts(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_audio_codec_allowlist
    ADD CONSTRAINT quality_profile_audio_codec_allowlist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_audio_codec_blocklist
    ADD CONSTRAINT quality_profile_audio_codec_blocklist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_quality_tiers
    ADD CONSTRAINT quality_profile_quality_tiers_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_source_allowlist
    ADD CONSTRAINT quality_profile_source_allowlist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_source_blocklist
    ADD CONSTRAINT quality_profile_source_blocklist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_video_codec_allowlist
    ADD CONSTRAINT quality_profile_video_codec_allowlist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_video_codec_blocklist
    ADD CONSTRAINT quality_profile_video_codec_blocklist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY release_decisions
    ADD CONSTRAINT release_decisions_wanted_item_id_fkey FOREIGN KEY (wanted_item_id) REFERENCES wanted_items(id) ON DELETE CASCADE;
ALTER TABLE ONLY release_download_attempts
    ADD CONSTRAINT release_download_attempts_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY series_movie_links
    ADD CONSTRAINT series_movie_links_linked_episode_id_fkey FOREIGN KEY (linked_episode_id) REFERENCES episodes(id) ON DELETE SET NULL;
ALTER TABLE ONLY series_movie_links
    ADD CONSTRAINT series_movie_links_movie_entity_id_fkey FOREIGN KEY (movie_entity_id) REFERENCES movie_entities(id) ON DELETE CASCADE;
ALTER TABLE ONLY series_movie_links
    ADD CONSTRAINT series_movie_links_series_title_id_fkey FOREIGN KEY (series_title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY settings_values
    ADD CONSTRAINT settings_values_setting_definition_id_fkey FOREIGN KEY (setting_definition_id) REFERENCES settings_definitions(id) ON DELETE CASCADE;
ALTER TABLE ONLY subtitle_downloads
    ADD CONSTRAINT subtitle_downloads_media_file_id_fkey FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE CASCADE;
ALTER TABLE ONLY subtitle_downloads
    ADD CONSTRAINT subtitle_downloads_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_external_ids
    ADD CONSTRAINT title_external_ids_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_image_variants
    ADD CONSTRAINT title_image_variants_blob_digest_fkey FOREIGN KEY (blob_digest) REFERENCES title_image_blobs(digest) ON DELETE RESTRICT;
ALTER TABLE ONLY title_image_variants
    ADD CONSTRAINT title_image_variants_title_image_id_fkey FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_images
    ADD CONSTRAINT title_images_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_external_ratings
    ADD CONSTRAINT title_metadata_external_ratings_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_rating_sources
    ADD CONSTRAINT title_metadata_rating_sources_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_rating_summaries
    ADD CONSTRAINT title_metadata_rating_summaries_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_tag_source_keys
    ADD CONSTRAINT title_metadata_tag_source_keys_title_id_tag_key_fkey FOREIGN KEY (title_id, tag_key) REFERENCES title_metadata_tags(title_id, tag_key) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_tag_sources
    ADD CONSTRAINT title_metadata_tag_sources_title_id_tag_key_fkey FOREIGN KEY (title_id, tag_key) REFERENCES title_metadata_tags(title_id, tag_key) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_tags
    ADD CONSTRAINT title_metadata_tags_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_more_like_this_items
    ADD CONSTRAINT title_more_like_this_items_discovery_title_id_fkey FOREIGN KEY (discovery_title_id) REFERENCES discovery_titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_more_like_this_items
    ADD CONSTRAINT title_more_like_this_items_source_title_id_fkey FOREIGN KEY (source_title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_search_terms
    ADD CONSTRAINT title_search_terms_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY totp_credentials
    ADD CONSTRAINT totp_credentials_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY totp_enrollment_challenges
    ADD CONSTRAINT totp_enrollment_challenges_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY totp_failed_attempts
    ADD CONSTRAINT totp_failed_attempts_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY totp_recovery_codes
    ADD CONSTRAINT totp_recovery_codes_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_app_permission_masks
    ADD CONSTRAINT user_app_permission_masks_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_external_accounts
    ADD CONSTRAINT user_external_accounts_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id);
ALTER TABLE ONLY user_external_accounts
    ADD CONSTRAINT user_external_accounts_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_library_permission_masks
    ADD CONSTRAINT user_library_permission_masks_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_library_permission_masks
    ADD CONSTRAINT user_library_permission_masks_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_ui_settings
    ADD CONSTRAINT user_ui_settings_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_ui_table_columns
    ADD CONSTRAINT user_ui_table_columns_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id);
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_series_movie_link_id_fkey FOREIGN KEY (series_movie_link_id) REFERENCES series_movie_links(id) ON DELETE SET NULL;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY webauthn_challenges
    ADD CONSTRAINT webauthn_challenges_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY webauthn_credentials
    ADD CONSTRAINT webauthn_credentials_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_media_file_id_fkey FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_series_movie_link_id_fkey FOREIGN KEY (series_movie_link_id) REFERENCES series_movie_links(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
