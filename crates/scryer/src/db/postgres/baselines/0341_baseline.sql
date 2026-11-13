CREATE FUNCTION discovery_affinity_bump_revision() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, md5(random()::text || clock_timestamp()::text))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
    RETURN NULL;
END;
$$;
CREATE FUNCTION title_search_enqueue_deleted_title() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    INSERT INTO title_search_index_queue (title_id) VALUES (OLD.id);
    RETURN OLD;
END;
$$;
CREATE TABLE api_keys (
    id text NOT NULL,
    user_id text NOT NULL,
    lookup_id text NOT NULL,
    secret_hash text NOT NULL,
    label text NOT NULL,
    expires_at timestamp with time zone,
    revoked_at timestamp with time zone,
    last_used_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    provisioning_source text NOT NULL,
    CONSTRAINT api_keys_provisioning_source_check CHECK ((provisioning_source = ANY (ARRAY['user'::text, 'environment'::text])))
);
CREATE TABLE application_compatibility_journal (
    migration_id text NOT NULL,
    subject_id text NOT NULL,
    source_digest text NOT NULL,
    original_metadata text NOT NULL,
    status text NOT NULL,
    detail text,
    CONSTRAINT application_compatibility_journal_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'blocked'::text, 'validated'::text])))
);
CREATE TABLE application_migrations (
    migration_id text NOT NULL,
    description text NOT NULL,
    applied_at timestamp with time zone NOT NULL,
    execution_time_ms bigint NOT NULL
);
CREATE TABLE blocklist (
    id text NOT NULL,
    title_id text NOT NULL,
    release_name text,
    reason text,
    created_at timestamp with time zone NOT NULL,
    normalized_release_name text DEFAULT ''::text NOT NULL,
    indexer_id text DEFAULT ''::text NOT NULL,
    info_hash text
);
CREATE TABLE canonical_tag_vocabulary_cache (
    id text NOT NULL,
    version text NOT NULL,
    payload_json jsonb NOT NULL,
    checked_at timestamp with time zone NOT NULL,
    jitter_seconds bigint NOT NULL,
    CONSTRAINT canonical_tag_vocabulary_cache_id_check CHECK ((id = 'canonical'::text)),
    CONSTRAINT canonical_tag_vocabulary_cache_jitter_seconds_check CHECK (((jitter_seconds >= 0) AND (jitter_seconds <= 21600)))
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
CREATE TABLE delay_profile_facets (
    profile_id text NOT NULL,
    facet text NOT NULL,
    "position" integer DEFAULT 0 NOT NULL
);
CREATE TABLE delay_profile_libraries (
    profile_id text NOT NULL,
    library_id text NOT NULL,
    "position" integer DEFAULT 0 NOT NULL
);
CREATE TABLE delay_profile_tags (
    profile_id text NOT NULL,
    label text NOT NULL,
    "position" integer DEFAULT 0 NOT NULL
);
CREATE TABLE delay_profiles (
    id text NOT NULL,
    name text NOT NULL,
    usenet_delay_minutes bigint DEFAULT 0 NOT NULL,
    torrent_delay_minutes bigint DEFAULT 0 NOT NULL,
    preferred_protocol text DEFAULT 'usenet'::text NOT NULL,
    min_age_minutes bigint DEFAULT 0 NOT NULL,
    bypass_score_threshold integer,
    enable_usenet boolean DEFAULT true NOT NULL,
    enable_torrent boolean DEFAULT true NOT NULL,
    bypass_if_highest_quality boolean DEFAULT false NOT NULL,
    priority integer DEFAULT 0 NOT NULL,
    "position" bigint DEFAULT 0 NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_affinity_revision (
    id integer NOT NULL,
    revision text NOT NULL,
    CONSTRAINT discovery_affinity_revision_id_check CHECK ((id = 1))
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
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    recommendation_score double precision,
    base_rank double precision
);
CREATE TABLE discovery_library_graph_payloads (
    run_id text NOT NULL,
    payload bytea NOT NULL
);
CREATE TABLE discovery_library_graph_runs (
    id text NOT NULL,
    facet text NOT NULL,
    axis text NOT NULL,
    status text NOT NULL,
    subject_count bigint DEFAULT 0 NOT NULL,
    submitted_count bigint DEFAULT 0 NOT NULL,
    generated_at text,
    error_code text,
    error_text text,
    started_at timestamp with time zone NOT NULL,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    retry_after_seconds bigint,
    omitted_subject_keys_json text DEFAULT '[]'::text NOT NULL
);
CREATE TABLE discovery_neighbor_cache (
    target_key text NOT NULL,
    language text NOT NULL,
    payload_version integer NOT NULL,
    payload_blob bytea NOT NULL,
    fetched_at timestamp with time zone NOT NULL
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
CREATE TABLE discovery_presentation_selection (
    scope_key text NOT NULL,
    language text NOT NULL,
    revision bigint DEFAULT 1 NOT NULL
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
    error_text text,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    acknowledged_at timestamp with time zone,
    presentation_revision bigint DEFAULT 0 NOT NULL
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
    discovery_title_id text CONSTRAINT discovery_title_metadata_external_r_discovery_title_id_not_null NOT NULL,
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
    discovery_title_id text CONSTRAINT discovery_title_metadata_rating_sou_discovery_title_id_not_null NOT NULL,
    source text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_title_metadata_rating_summaries (
    discovery_title_id text CONSTRAINT discovery_title_metadata_rating_sum_discovery_title_id_not_null NOT NULL,
    rating double precision,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE discovery_title_metadata_tag_source_keys (
    discovery_title_id text CONSTRAINT discovery_title_metadata_tag_source_discovery_title_id_not_null NOT NULL,
    tag_key text NOT NULL,
    source_tag_key text CONSTRAINT discovery_title_metadata_tag_source_key_source_tag_key_not_null NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL
);
CREATE TABLE discovery_title_metadata_tag_sources (
    discovery_title_id text CONSTRAINT discovery_title_metadata_tag_sourc_discovery_title_id_not_null1 NOT NULL,
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
    sort_index integer DEFAULT 0 NOT NULL,
    affinity_signals_json text DEFAULT '[]'::text NOT NULL
);
CREATE TABLE discovery_title_source_tag_values (
    discovery_title_id text NOT NULL,
    source_tag_sort_index integer CONSTRAINT discovery_title_source_tag_value_source_tag_sort_index_not_null NOT NULL,
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
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    is_adult boolean DEFAULT false NOT NULL,
    content_ratings_json text DEFAULT '[]'::text NOT NULL,
    affinity_signals_json text DEFAULT '[]'::text NOT NULL
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
    payload_json bytea NOT NULL,
    actor_kind text DEFAULT 'system'::text NOT NULL,
    actor_display_name text DEFAULT 'System'::text NOT NULL,
    import_status text,
    media_file_delete_reason text,
    download_id text,
    import_skip_reason text
);
CREATE SEQUENCE domain_events_sequence_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;
ALTER SEQUENCE domain_events_sequence_seq OWNED BY domain_events.sequence;
CREATE TABLE download_cleanup (
    download_id text NOT NULL,
    client_id text NOT NULL,
    client_type text NOT NULL,
    item_id text NOT NULL,
    title_id text,
    facet text,
    source_title text,
    tracked_state text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    attempts bigint DEFAULT 0 NOT NULL,
    history_offset bigint DEFAULT 0 NOT NULL,
    payload_checkpoint text,
    outcome text,
    last_error text,
    next_attempt_at timestamp with time zone NOT NULL,
    lease_until timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT download_cleanup_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'completed'::text])))
);
CREATE TABLE download_cleanup_seed_state (
    id integer NOT NULL,
    changed_through timestamp with time zone NOT NULL,
    full_scan_at timestamp with time zone NOT NULL,
    CONSTRAINT download_cleanup_seed_state_id_check CHECK ((id = 1))
);
CREATE TABLE download_client_bindings (
    download_id text NOT NULL,
    client_config_id text,
    client_type_snapshot text,
    client_name_snapshot text,
    native_item_id text,
    created_at timestamp with time zone NOT NULL,
    last_seen_at timestamp with time zone,
    ended_at timestamp with time zone
);
CREATE TABLE download_client_status (
    client_config_id text NOT NULL,
    initial_failure_at timestamp with time zone,
    most_recent_failure_at timestamp with time zone,
    escalation_level integer DEFAULT 0 NOT NULL,
    disabled_until timestamp with time zone
);
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
    config_json text NOT NULL,
    proxy_config_id text
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
    canonical_download_id text NOT NULL
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
    source_client_id text,
    canonical_download_id text,
    workspace_relative_path text
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
    updated_at timestamp with time zone NOT NULL,
    canonical_download_id text
);
CREATE TABLE download_submission_episode_links (
    download_id text CONSTRAINT download_submission_episode_links_0180_download_id_not_null NOT NULL,
    episode_id text CONSTRAINT download_submission_episode_links_0180_episode_id_not_null NOT NULL
);
CREATE TABLE download_submissions (
    id text NOT NULL,
    title_id text DEFAULT ''::text NOT NULL,
    facet text DEFAULT ''::text NOT NULL,
    download_client_id text DEFAULT ''::text NOT NULL,
    download_client_type text NOT NULL,
    download_client_item_id text,
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
    actor_display_name text,
    source_provider_id text,
    source_provider_name text,
    seeding_profile_id text,
    seed_goal_ratio double precision,
    seed_goal_seconds bigint,
    seed_never_remove boolean,
    seed_goal_met_action text,
    seed_goal_source text,
    seed_info_hash text,
    seed_post_import_tracking text,
    release_size_bytes bigint,
    info_hash text,
    release_listing_json text,
    password_candidates text,
    password_retry_state text,
    grabbed_file_names_json text
);
CREATE TABLE downloads (
    id text NOT NULL,
    origin text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    first_observed_at timestamp with time zone,
    last_observed_at timestamp with time zone,
    terminal_at timestamp with time zone,
    CONSTRAINT downloads_origin_check CHECK ((origin = ANY (ARRAY['scryer_submission'::text, 'foreign_observation'::text])))
);
CREATE TABLE emby_media_server_details (
    connection_id text NOT NULL,
    api_key text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    server_id text,
    connect_enabled boolean DEFAULT false NOT NULL
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
    image_url text,
    contiguous_absolute_number bigint,
    tmdb_id text
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
    draft_key text CONSTRAINT external_import_setup_download_client_api_ke_draft_key_not_null NOT NULL,
    dedup_key text CONSTRAINT external_import_setup_download_client_api_ke_dedup_key_not_null NOT NULL,
    api_key_encrypted text CONSTRAINT external_import_setup_download_clien_api_key_encrypted_not_null NOT NULL,
    "position" integer CONSTRAINT external_import_setup_download_client_api_key_position_not_null NOT NULL,
    created_at timestamp with time zone CONSTRAINT external_import_setup_download_client_api_k_created_at_not_null NOT NULL,
    updated_at timestamp with time zone CONSTRAINT external_import_setup_download_client_api_k_updated_at_not_null NOT NULL,
    CONSTRAINT external_import_setup_download_client_api_key_ov_position_check CHECK (("position" >= 0))
);
CREATE TABLE external_import_setup_download_client_password_overrides (
    draft_key text CONSTRAINT external_import_setup_download_client_passwo_draft_key_not_null NOT NULL,
    dedup_key text CONSTRAINT external_import_setup_download_client_passwo_dedup_key_not_null NOT NULL,
    password_encrypted text CONSTRAINT external_import_setup_download_clie_password_encrypted_not_null NOT NULL,
    "position" integer CONSTRAINT external_import_setup_download_client_passwor_position_not_null NOT NULL,
    created_at timestamp with time zone CONSTRAINT external_import_setup_download_client_passw_created_at_not_null NOT NULL,
    updated_at timestamp with time zone CONSTRAINT external_import_setup_download_client_passw_updated_at_not_null NOT NULL,
    CONSTRAINT external_import_setup_download_client_password_o_position_check CHECK (("position" >= 0))
);
CREATE TABLE external_import_setup_indexer_api_key_overrides (
    draft_key text CONSTRAINT external_import_setup_indexer_api_key_overri_draft_key_not_null NOT NULL,
    dedup_key text CONSTRAINT external_import_setup_indexer_api_key_overri_dedup_key_not_null NOT NULL,
    api_key_encrypted text CONSTRAINT external_import_setup_indexer_api_ke_api_key_encrypted_not_null NOT NULL,
    "position" integer CONSTRAINT external_import_setup_indexer_api_key_overrid_position_not_null NOT NULL,
    created_at timestamp with time zone CONSTRAINT external_import_setup_indexer_api_key_overr_created_at_not_null NOT NULL,
    updated_at timestamp with time zone CONSTRAINT external_import_setup_indexer_api_key_overr_updated_at_not_null NOT NULL,
    CONSTRAINT external_import_setup_indexer_api_key_overrides_position_check CHECK (("position" >= 0))
);
CREATE TABLE external_import_setup_instance_api_keys (
    draft_key text NOT NULL,
    instance_id text NOT NULL,
    kind text NOT NULL,
    api_key_encrypted text CONSTRAINT external_import_setup_instance_api_k_api_key_encrypted_not_null NOT NULL,
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
    is_filler boolean DEFAULT false,
    role text DEFAULT 'additional'::text NOT NULL,
    CONSTRAINT file_episode_map_role_check CHECK ((role = ANY (ARRAY['primary'::text, 'additional'::text])))
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
CREATE TABLE image_proxy_cache_entries (
    token text NOT NULL,
    variant text NOT NULL,
    content_type text NOT NULL,
    byte_size bigint NOT NULL,
    upstream_etag text,
    upstream_last_modified text,
    fetched_at timestamp with time zone NOT NULL,
    last_accessed_at timestamp with time zone NOT NULL
);
CREATE TABLE image_proxy_sources (
    token text NOT NULL,
    upstream_url text,
    owner_type text,
    owner_id text,
    image_kind text NOT NULL,
    fallback_class text NOT NULL,
    last_seen_at timestamp with time zone NOT NULL
);
CREATE TABLE import_space_incident_lock (
    id integer NOT NULL,
    revision integer NOT NULL,
    CONSTRAINT import_space_incident_lock_id_check CHECK ((id = 1))
);
CREATE TABLE import_space_incidents (
    destination_key text NOT NULL,
    incident_id text NOT NULL
);
CREATE TABLE import_space_members (
    member_key text NOT NULL,
    job_key text NOT NULL,
    destination_key text NOT NULL,
    measurement_json text NOT NULL,
    download_id text NOT NULL,
    observed_at text DEFAULT ''::text NOT NULL
);
CREATE TABLE import_space_notification_attempts (
    event_id text NOT NULL,
    attempts integer NOT NULL,
    first_attempt_at text NOT NULL
);
CREATE TABLE import_space_notification_receipts (
    event_id text NOT NULL,
    target_key text NOT NULL
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
    import_transfer_updated_at timestamp with time zone,
    canonical_download_id text
);
CREATE TABLE indexer_api_client_grants (
    client_id text NOT NULL,
    indexer_id text NOT NULL,
    categories_json text
);
CREATE TABLE indexer_api_clients (
    id text NOT NULL,
    name text NOT NULL,
    enabled boolean NOT NULL,
    protocols_json text NOT NULL,
    lookup_id text,
    secret_hash text,
    key_generation bigint NOT NULL,
    revision bigint NOT NULL,
    last_used_at timestamp with time zone,
    last_outcome_code text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT indexer_api_clients_check CHECK (((lookup_id IS NULL) = (secret_hash IS NULL))),
    CONSTRAINT indexer_api_clients_key_generation_check CHECK ((key_generation > 0)),
    CONSTRAINT indexer_api_clients_revision_check CHECK ((revision > 0))
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
CREATE TABLE indexer_errors (
    id text NOT NULL,
    indexer_id text NOT NULL,
    indexer_name text NOT NULL,
    operation text NOT NULL,
    http_status integer NOT NULL,
    classification text NOT NULL,
    provider_error_code integer,
    message text NOT NULL,
    content_type text,
    payload_format_version integer NOT NULL,
    response_zstd bytea NOT NULL,
    occurred_at timestamp with time zone NOT NULL
);
CREATE TABLE indexer_facet_learning (
    indexer_id text NOT NULL,
    facet text NOT NULL,
    empty_title_ids text DEFAULT '[]'::text NOT NULL,
    suppressed_until timestamp with time zone,
    reprobe_claimed_at timestamp with time zone,
    revision bigint DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE indexer_search_candidate_source_values (
    source_id text NOT NULL,
    value_kind text NOT NULL,
    ordinal integer NOT NULL,
    value text NOT NULL
);
CREATE TABLE indexer_search_candidate_sources (
    id text NOT NULL,
    candidate_id text NOT NULL,
    indexer_id text NOT NULL,
    source_identity text NOT NULL,
    provider_ref text,
    source text NOT NULL,
    encrypted_download_url text,
    encrypted_link_url text,
    published_at text,
    thumbs_up integer,
    thumbs_down integer,
    grabs bigint,
    grab_current bigint,
    grab_max bigint,
    response_tvdb_id text,
    response_tmdb_id text,
    response_imdb_id text,
    season bigint,
    episode bigint,
    absolute_episode bigint,
    release_group text,
    provider_source text,
    seeders bigint,
    peers bigint,
    download_volume_factor double precision,
    upload_volume_factor double precision,
    protected boolean,
    first_seen_at timestamp with time zone NOT NULL,
    last_seen_at timestamp with time zone NOT NULL,
    reusable_until timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL
);
CREATE TABLE indexer_search_candidates (
    id text NOT NULL,
    fingerprint text NOT NULL,
    title text NOT NULL,
    normalized_title text NOT NULL,
    size_bytes bigint,
    source_kind text,
    info_hash text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    reusable_until timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL
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
CREATE TABLE indexer_search_listing_readers (
    indexer_id text NOT NULL,
    indexer_fingerprint text NOT NULL,
    title_id text NOT NULL,
    facet text NOT NULL,
    scope_key text NOT NULL,
    query_signature text NOT NULL,
    strategy_family text NOT NULL,
    first_page_fingerprint text NOT NULL,
    observed_at timestamp with time zone NOT NULL
);
CREATE TABLE indexer_search_listing_runs (
    run_id text NOT NULL,
    indexer_id text NOT NULL,
    indexer_fingerprint text NOT NULL,
    title_id text NOT NULL,
    facet text NOT NULL,
    first_page_fingerprint text NOT NULL,
    created_at timestamp with time zone NOT NULL
);
CREATE TABLE indexer_search_listings (
    indexer_id text NOT NULL,
    indexer_fingerprint text NOT NULL,
    title_id text NOT NULL,
    facet text NOT NULL,
    first_page_fingerprint text NOT NULL,
    owner_scope_key text NOT NULL,
    owner_query_signature text NOT NULL,
    owner_strategy_family text NOT NULL,
    page_cursor text,
    finished boolean DEFAULT false NOT NULL,
    chain_started_at timestamp with time zone NOT NULL,
    advanced_at timestamp with time zone NOT NULL
);
CREATE TABLE indexer_search_run_candidate_sources (
    run_id text NOT NULL,
    source_id text NOT NULL,
    search_session_id text NOT NULL
);
CREATE TABLE indexer_search_runs (
    id text NOT NULL,
    indexer_id text NOT NULL,
    provider_type text NOT NULL,
    search_session_id text NOT NULL,
    scope_key text NOT NULL,
    query_signature text NOT NULL,
    branch text NOT NULL,
    page integer,
    provider_offset integer,
    next_provider_offset integer,
    range_min_size bigint,
    range_max_size bigint,
    result_count integer NOT NULL,
    completion_state text NOT NULL,
    retry_at timestamp with time zone,
    error_summary text,
    indexer_fingerprint text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    strategy_state text,
    coverage_scope_key text,
    page_cursor text,
    cursor_chain_id text
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
    caps_snapshot_json text,
    proxy_config_id text,
    last_error_message text,
    download_client_id text,
    seeding_profile_id text,
    max_queries_per_minute bigint,
    last_error_code text
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
CREATE TABLE library_root_id_remaps (
    legacy_root_id text NOT NULL,
    root_id text NOT NULL,
    normalized_path text NOT NULL,
    remapped boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE library_roots (
    id text NOT NULL,
    library_id text NOT NULL,
    path text NOT NULL,
    normalized_path text NOT NULL,
    is_default boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    legacy_path_derived_id text
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
    search_attempts_json text DEFAULT '[]'::text,
    size_bytes bigint
);
CREATE TABLE lifecycle_action_runs (
    id text NOT NULL,
    candidate_id text NOT NULL,
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
    detail text DEFAULT '{}'::text NOT NULL,
    started_at timestamp with time zone NOT NULL,
    finished_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    subject_kind text DEFAULT 'title'::text NOT NULL,
    subject_id text DEFAULT ''::text NOT NULL
);
CREATE TABLE lifecycle_candidates (
    id text NOT NULL,
    rule_set_id text NOT NULL,
    revision_number bigint NOT NULL,
    matcher_content_hash text DEFAULT ''::text NOT NULL,
    title_id text NOT NULL,
    library_id text DEFAULT ''::text NOT NULL,
    facet text DEFAULT ''::text NOT NULL,
    subject_kind text DEFAULT 'title'::text NOT NULL,
    match_generation bigint DEFAULT 1 NOT NULL,
    state text DEFAULT 'observing'::text NOT NULL,
    state_reason text DEFAULT ''::text NOT NULL,
    reason_codes text DEFAULT '[]'::text NOT NULL,
    action_kind text DEFAULT 'do_nothing'::text NOT NULL,
    grace_days bigint DEFAULT 0 NOT NULL,
    first_matched_at timestamp with time zone DEFAULT now() NOT NULL,
    last_matched_at timestamp with time zone DEFAULT now() NOT NULL,
    due_at timestamp with time zone DEFAULT now() NOT NULL,
    last_evaluated_at timestamp with time zone DEFAULT now() NOT NULL,
    held_since timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    action_attempts bigint DEFAULT 0 NOT NULL,
    subject_id text DEFAULT ''::text NOT NULL
);
CREATE TABLE lifecycle_claims (
    id text NOT NULL,
    title_id text NOT NULL,
    library_id text DEFAULT ''::text NOT NULL,
    producer text NOT NULL,
    producer_ref text,
    kind text DEFAULT 'retain_until'::text NOT NULL,
    state text DEFAULT 'dormant'::text NOT NULL,
    duration_days bigint,
    starts_at timestamp with time zone,
    expires_at timestamp with time zone,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    released_reason text
);
CREATE TABLE list_exclusion_external_ids (
    exclusion_id text NOT NULL,
    source text NOT NULL,
    value text NOT NULL,
    external_kind text DEFAULT ''::text NOT NULL
);
CREATE TABLE list_exclusions (
    id text NOT NULL,
    kind text NOT NULL,
    display_title text NOT NULL,
    year integer,
    scope text NOT NULL,
    subscription_id text,
    created_by_user_id text,
    created_at timestamp with time zone NOT NULL
);
CREATE TABLE list_memberships (
    subscription_id text NOT NULL,
    item_key text NOT NULL,
    rank bigint,
    season integer,
    display_title text,
    year integer,
    external_ids_json text DEFAULT '[]'::text NOT NULL,
    smg_title_id bigint,
    title_id text,
    request_id text,
    kind text NOT NULL,
    state text NOT NULL,
    state_reason text,
    added_by_list boolean DEFAULT false NOT NULL,
    first_seen_at timestamp with time zone NOT NULL,
    last_seen_at timestamp with time zone NOT NULL,
    left_at timestamp with time zone,
    left_handled boolean DEFAULT false NOT NULL,
    series_movie_json jsonb
);
CREATE TABLE list_subscription_routes (
    subscription_id text NOT NULL,
    kind text NOT NULL,
    library_id text NOT NULL,
    quality_profile_id text,
    root_folder_id text,
    monitor_type text NOT NULL,
    min_availability text,
    use_season_folders boolean,
    release_numbering text,
    tags_json text DEFAULT '[]'::text NOT NULL
);
CREATE TABLE list_subscriptions (
    id text NOT NULL,
    scope text NOT NULL,
    owner_user_id text NOT NULL,
    provider text NOT NULL,
    source_type text NOT NULL,
    source_params_json text DEFAULT '{}'::text NOT NULL,
    source_origin text NOT NULL,
    chart_key text,
    chart_scope text,
    name text NOT NULL,
    provider_url text,
    kinds_json text DEFAULT '[]'::text NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    mode text NOT NULL,
    filters_json text DEFAULT '[]'::text NOT NULL,
    max_per_sync integer,
    on_leave text DEFAULT 'keep'::text NOT NULL,
    interval_seconds bigint NOT NULL,
    credential_id text,
    sync_state text DEFAULT 'new'::text NOT NULL,
    last_sync_at timestamp with time zone,
    next_sync_at timestamp with time zone,
    error_message text,
    error_at timestamp with time zone,
    paused_until timestamp with time zone,
    fetch_fingerprint text,
    count_total bigint DEFAULT 0 NOT NULL,
    count_in_library bigint DEFAULT 0 NOT NULL,
    count_added bigint DEFAULT 0 NOT NULL,
    count_requested bigint DEFAULT 0 NOT NULL,
    count_held bigint DEFAULT 0 NOT NULL,
    count_filtered bigint DEFAULT 0 NOT NULL,
    count_excluded bigint DEFAULT 0 NOT NULL,
    count_unresolved bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE list_sync_runs (
    id text NOT NULL,
    subscription_id text NOT NULL,
    job_run_id text,
    started_at timestamp with time zone NOT NULL,
    finished_at timestamp with time zone,
    outcome text NOT NULL,
    counts_json text DEFAULT '{}'::text NOT NULL,
    error_message text
);
CREATE TABLE location_file_resolutions (
    operation_id text NOT NULL,
    title_id text NOT NULL,
    source_path text NOT NULL,
    resolution_json text NOT NULL
);
CREATE TABLE location_operation_owned_entities (
    operation_id text NOT NULL,
    entity_type text NOT NULL,
    entity_id text NOT NULL,
    ownership_mode text DEFAULT 'exclusive'::text NOT NULL,
    acquired_at timestamp with time zone DEFAULT now() NOT NULL,
    released_at timestamp with time zone
);
CREATE TABLE location_operation_plans (
    operation_id text NOT NULL,
    plan bytea NOT NULL
);
CREATE TABLE location_operation_title_checkpoints (
    operation_id text NOT NULL,
    title_id text NOT NULL,
    sequence integer DEFAULT 0 NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    classification text,
    source_library_id text,
    source_root_id text,
    source_folder_path text,
    destination_library_id text,
    destination_root_id text,
    destination_folder_path text,
    merged_into_title_id text,
    file_total integer DEFAULT 0 NOT NULL,
    file_completed_count integer DEFAULT 0 CONSTRAINT location_operation_title_checkpoi_file_completed_count_not_null NOT NULL,
    bytes_total bigint DEFAULT 0 NOT NULL,
    bytes_completed bigint DEFAULT 0 NOT NULL,
    blocked_reason text,
    failure_reason text,
    note text,
    checkpointed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    reason_code text
);
CREATE TABLE location_operation_verifications (
    id text NOT NULL,
    operation_id text NOT NULL,
    title_id text,
    media_file_id text,
    source_path text NOT NULL,
    destination_path text NOT NULL,
    size_bytes bigint DEFAULT 0 NOT NULL,
    requested_depth text NOT NULL,
    applied_depth text NOT NULL,
    fell_back boolean DEFAULT false NOT NULL,
    fallback_reason text,
    outcome text NOT NULL,
    move_crc text,
    move_crc_algorithm text,
    full_blake3 text,
    sampled_signature_scheme text,
    sampled_signature_value text,
    failure_reason text,
    detail text,
    verified_at timestamp with time zone DEFAULT now() NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE location_operations (
    id text NOT NULL,
    operation_type text NOT NULL,
    execution_mode text NOT NULL,
    state text DEFAULT 'queued'::text NOT NULL,
    initiated_by_user_id text,
    source_library_id text,
    source_root_id text,
    destination_library_id text,
    destination_root_id text,
    plan_fingerprint text DEFAULT ''::text NOT NULL,
    verification_depth text DEFAULT 'full'::text NOT NULL,
    verification_fallback_count integer DEFAULT 0 NOT NULL,
    title_total integer DEFAULT 0 NOT NULL,
    title_completed_count integer DEFAULT 0 NOT NULL,
    title_blocked_count integer DEFAULT 0 NOT NULL,
    file_total integer DEFAULT 0 NOT NULL,
    file_completed_count integer DEFAULT 0 NOT NULL,
    bytes_total bigint DEFAULT 0 NOT NULL,
    bytes_completed bigint DEFAULT 0 NOT NULL,
    merge_count integer DEFAULT 0 NOT NULL,
    dedup_count integer DEFAULT 0 NOT NULL,
    rename_count integer DEFAULT 0 NOT NULL,
    no_op_count integer DEFAULT 0 NOT NULL,
    unresolved_count integer DEFAULT 0 NOT NULL,
    job_run_id text,
    workflow_operation_id text,
    cancel_requested boolean DEFAULT false NOT NULL,
    cancel_requested_at timestamp with time zone,
    failure_reason text,
    confirmed_at timestamp with time zone,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    reason_code text,
    recovery_key text,
    recovery_committed_at timestamp with time zone
);
CREATE TABLE location_recovery_staging (
    id text NOT NULL,
    operation_id text NOT NULL,
    media_file_id text NOT NULL,
    source_path text NOT NULL,
    destination_path text NOT NULL,
    staging_path text NOT NULL
);
CREATE TABLE location_transfer_progress (
    operation_id text NOT NULL,
    progress_basis_points bigint DEFAULT 0 NOT NULL,
    titles_initialized boolean DEFAULT false NOT NULL
);
CREATE TABLE location_transfer_runtime (
    id bigint NOT NULL,
    generation bigint NOT NULL,
    CONSTRAINT location_transfer_runtime_id_check CHECK ((id = 1))
);
CREATE TABLE location_transfer_titles (
    operation_id text NOT NULL,
    title_id text NOT NULL,
    sequence bigint NOT NULL,
    hot_rank bigint NOT NULL,
    summary_json text NOT NULL
);
CREATE TABLE login_verification_challenges (
    id text NOT NULL,
    user_id text NOT NULL,
    login_method text NOT NULL,
    persist_session boolean NOT NULL,
    allow_passkey boolean NOT NULL,
    allow_totp boolean NOT NULL,
    auth_session_version text,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    CONSTRAINT login_verification_challenges_login_method_check CHECK ((login_method = ANY (ARRAY['local_password'::text, 'jellyfin'::text, 'emby'::text])))
);
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
    reconciliation_evidence_json text DEFAULT '{}'::text CONSTRAINT maintenance_action_job_rece_reconciliation_evidence_js_not_null NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE maintenance_action_step_attempts (
    id text NOT NULL,
    candidate_id text NOT NULL,
    match_generation bigint NOT NULL,
    revision_number bigint NOT NULL,
    step_id text NOT NULL,
    attempt bigint NOT NULL,
    state text NOT NULL,
    intent_json text DEFAULT '{}'::text NOT NULL,
    before_state_json text DEFAULT '{}'::text NOT NULL,
    target_identity_json text DEFAULT '{}'::text NOT NULL,
    provenance_json text DEFAULT '{}'::text NOT NULL,
    hold_reason text,
    error text,
    started_at timestamp with time zone NOT NULL,
    finished_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE maintenance_action_steps (
    candidate_id text NOT NULL,
    match_generation bigint NOT NULL,
    revision_number bigint NOT NULL,
    step_id text NOT NULL,
    rule_set_id text NOT NULL,
    title_id text NOT NULL,
    subject_kind text NOT NULL,
    subject_id text NOT NULL,
    sequence_content_hash text NOT NULL,
    step_kind text NOT NULL,
    intent_json text DEFAULT '{}'::text NOT NULL,
    before_state_json text DEFAULT '{}'::text NOT NULL,
    target_identity_json text DEFAULT '{}'::text NOT NULL,
    provenance_json text DEFAULT '{}'::text NOT NULL,
    state text NOT NULL,
    attempt bigint DEFAULT 0 NOT NULL,
    lease_id text,
    lease_expires_at timestamp with time zone,
    hold_reason text,
    error text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    finished_at timestamp with time zone
);
CREATE TABLE maintenance_evaluation_runs (
    id text NOT NULL,
    rule_set_id text NOT NULL,
    revision_number bigint NOT NULL,
    matcher_content_hash text DEFAULT ''::text NOT NULL,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    finished_at timestamp with time zone,
    status text DEFAULT 'running'::text NOT NULL,
    evaluated_count bigint DEFAULT 0 NOT NULL,
    matched_count bigint DEFAULT 0 NOT NULL,
    no_match_count bigint DEFAULT 0 NOT NULL,
    unknown_count bigint DEFAULT 0 NOT NULL,
    error_count bigint DEFAULT 0 NOT NULL,
    canceled_candidates bigint DEFAULT 0 NOT NULL,
    superseded_candidates bigint DEFAULT 0 NOT NULL,
    duration_ms bigint,
    error text
);
CREATE TABLE maintenance_rule_exclusions (
    id text NOT NULL,
    rule_set_id text,
    title_id text NOT NULL,
    reason text DEFAULT ''::text NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    subject_kind text DEFAULT 'title'::text NOT NULL,
    subject_id text DEFAULT ''::text NOT NULL
);
CREATE TABLE maintenance_rule_revisions (
    id text NOT NULL,
    rule_set_id text NOT NULL,
    revision_number bigint NOT NULL,
    rego_source text NOT NULL,
    action_spec text NOT NULL,
    grace_days bigint DEFAULT 0 NOT NULL,
    matcher_content_hash text NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    storage_root_id text
);
CREATE TABLE maintenance_rule_set_libraries (
    rule_set_id text NOT NULL,
    library_id text NOT NULL,
    "position" bigint NOT NULL,
    CONSTRAINT maintenance_rule_set_libraries_position_check CHECK (("position" >= 0))
);
CREATE TABLE maintenance_rule_sets (
    id text NOT NULL,
    name text NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    enabled boolean DEFAULT false NOT NULL,
    evaluation_mode text DEFAULT 'disabled'::text NOT NULL,
    subject_kind text DEFAULT 'title'::text NOT NULL,
    current_revision_number bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    effect_arming text DEFAULT 'none'::text NOT NULL,
    destructive_rearm_required boolean DEFAULT false NOT NULL,
    system_owned boolean DEFAULT false NOT NULL
);
CREATE TABLE maintenance_sequence_terminal_memberships (
    id text NOT NULL,
    candidate_id text NOT NULL,
    rule_set_id text NOT NULL,
    revision_number bigint CONSTRAINT maintenance_sequence_terminal_membersh_revision_number_not_null NOT NULL,
    matcher_content_hash text CONSTRAINT maintenance_sequence_terminal_mem_matcher_content_hash_not_null NOT NULL,
    title_id text NOT NULL,
    subject_kind text NOT NULL,
    subject_id text NOT NULL,
    match_generation bigint CONSTRAINT maintenance_sequence_terminal_members_match_generation_not_null NOT NULL,
    sequence_content_hash text CONSTRAINT maintenance_sequence_terminal_me_sequence_content_hash_not_null NOT NULL,
    outcome text NOT NULL,
    expected_step_ids_json text DEFAULT '[]'::text CONSTRAINT maintenance_sequence_terminal_m_expected_step_ids_json_not_null NOT NULL,
    completed_step_ids_json text DEFAULT '[]'::text CONSTRAINT maintenance_sequence_terminal__completed_step_ids_json_not_null NOT NULL,
    terminal_step_id text,
    created_at timestamp with time zone NOT NULL,
    released_at timestamp with time zone
);
CREATE TABLE manual_import_selection_candidates (
    id text NOT NULL,
    selection_id text NOT NULL,
    canonical_path text NOT NULL,
    quality text,
    created_at timestamp with time zone NOT NULL
);
CREATE TABLE manual_import_selections (
    id text NOT NULL,
    actor_user_id text NOT NULL,
    title_id text NOT NULL,
    source_client_id text DEFAULT ''::text NOT NULL,
    source_system text NOT NULL,
    source_ref text NOT NULL,
    consumed_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    release_evidence_json text,
    trusted_source_root text DEFAULT ''::text NOT NULL,
    archive_workspace_root text,
    canonical_download_id text
);
CREATE TABLE media_file_analyses (
    media_file_id text NOT NULL,
    analysis bytea NOT NULL
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
    role text DEFAULT 'primary'::text NOT NULL,
    announced_size_bytes bigint,
    full_blake3 text,
    move_crc text,
    move_crc_algorithm text,
    hash_computed_at timestamp with time zone,
    analysis_revision integer DEFAULT 0 NOT NULL,
    analysis_attempt_revision integer,
    analysis_attempted_at text,
    analysis_attempt_source text,
    analysis_attempt_status text,
    analysis_attempt_json text,
    release_listing_json text,
    current_score_evaluation text
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
    rating_summary_json text DEFAULT '{"rating":null,"rating_sources":[],"external_ratings":[]}'::text NOT NULL,
    background_url text,
    requested_lease_days bigint,
    approved_lease_days bigint,
    decision_id text,
    decided_by_rule_set_ids text DEFAULT '[]'::text NOT NULL,
    policy_tags_json text DEFAULT '[]'::text NOT NULL,
    metadata_snapshot_json text DEFAULT '{}'::text NOT NULL,
    origin_kind text DEFAULT 'manual'::text NOT NULL,
    origin_subscription_id text,
    CONSTRAINT media_requests_facet_check CHECK ((facet = ANY (ARRAY['movie'::text, 'series'::text, 'anime'::text]))),
    CONSTRAINT media_requests_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'approved'::text, 'rejected'::text, 'canceled'::text])))
);
CREATE TABLE media_server_audience_plays (
    connection_id text NOT NULL,
    external_account_id text NOT NULL,
    provider_item_id text NOT NULL,
    kind text NOT NULL,
    scryer_title_id text,
    scryer_episode_id text,
    play_count bigint DEFAULT 0 NOT NULL,
    last_played_at timestamp with time zone,
    CONSTRAINT media_server_audience_plays_external_account_id_check CHECK ((external_account_id <> ''::text)),
    CONSTRAINT media_server_audience_plays_kind_check CHECK ((kind = ANY (ARRAY['movie'::text, 'episode'::text]))),
    CONSTRAINT media_server_audience_plays_provider_item_id_check CHECK ((provider_item_id <> ''::text))
);
CREATE TABLE media_server_audience_reads (
    connection_id text NOT NULL,
    sweep_started_at timestamp with time zone NOT NULL,
    read_at timestamp with time zone NOT NULL,
    play_count bigint DEFAULT 0 NOT NULL
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
    external_url text,
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
CREATE TABLE media_server_playback_items (
    connection_id text NOT NULL,
    entity_kind text NOT NULL,
    entity_id text NOT NULL,
    provider_item_id text NOT NULL,
    last_seen_at timestamp with time zone NOT NULL,
    CONSTRAINT media_server_playback_items_entity_kind_check CHECK ((entity_kind = ANY (ARRAY['title'::text, 'episode'::text])))
);
CREATE TABLE media_server_signal_roster (
    connection_id text NOT NULL,
    link_id text NOT NULL,
    external_user_id text NOT NULL,
    scryer_user_id text NOT NULL,
    link_verified_at timestamp with time zone,
    sweep_started_at timestamp with time zone NOT NULL,
    read_at timestamp with time zone NOT NULL,
    CONSTRAINT media_server_signal_roster_link_id_check CHECK ((link_id <> ''::text))
);
CREATE TABLE media_server_signal_sync_state (
    connection_id text NOT NULL,
    provider text NOT NULL,
    enabled boolean DEFAULT false NOT NULL,
    last_started_at timestamp with time zone,
    last_success_at timestamp with time zone,
    last_error text,
    participant_count bigint DEFAULT 0 NOT NULL,
    signal_count bigint DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    last_error_code text
);
CREATE TABLE media_server_user_media_signals (
    id text NOT NULL,
    connection_id text NOT NULL,
    provider text NOT NULL,
    external_user_id text NOT NULL,
    scryer_user_id text,
    provider_item_id text NOT NULL,
    kind text NOT NULL,
    scryer_title_id text,
    scryer_episode_id text,
    played boolean DEFAULT false NOT NULL,
    play_count bigint DEFAULT 0 NOT NULL,
    last_played_at timestamp with time zone,
    observed_at timestamp with time zone DEFAULT now() NOT NULL,
    sync_generation bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    is_favorite boolean DEFAULT false NOT NULL
);
CREATE TABLE monitor_selections (
    owner_kind text NOT NULL,
    owner_id text NOT NULL,
    entry_kind text NOT NULL,
    entry_key text NOT NULL,
    label text,
    external_ids_json text DEFAULT '[]'::text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT monitor_selections_entry_kind_check CHECK ((entry_kind = ANY (ARRAY['season'::text, 'series_movie'::text]))),
    CONSTRAINT monitor_selections_owner_kind_check CHECK ((owner_kind = ANY (ARRAY['title'::text, 'media_request'::text])))
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
    authorization_source text DEFAULT 'authenticated'::text NOT NULL,
    auth_session_version text DEFAULT ''::text NOT NULL,
    jellyfin_connection_id text,
    jellyfin_external_url text,
    jellyfin_base_url text,
    jellyfin_api_key_hash text
);
CREATE TABLE oauth_client_redirect_uris (
    client_id text NOT NULL,
    redirect_uri text NOT NULL
);
CREATE TABLE oauth_client_registrations (
    client_id text NOT NULL,
    display_name text NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    kind text DEFAULT 'custom'::text NOT NULL,
    CONSTRAINT oauth_client_registrations_kind_check CHECK ((kind = ANY (ARRAY['custom'::text, 'jellyfin_plugin'::text])))
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
    authorization_source text DEFAULT 'authenticated'::text NOT NULL,
    redirect_uri text DEFAULT ''::text NOT NULL,
    jellyfin_connection_id text,
    jellyfin_external_url text,
    jellyfin_base_url text,
    jellyfin_api_key_hash text
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
    info_hash text,
    indexer_id text,
    minimum_seed_ratio double precision,
    minimum_seed_time_minutes bigint,
    season_pack_seed_ratio double precision,
    season_pack_seed_time_minutes bigint,
    seeders bigint,
    release_identity text,
    last_observed_at timestamp with time zone NOT NULL,
    coverage_identity text,
    role text DEFAULT 'primary'::text NOT NULL,
    last_decision_code text,
    release_age_unknown boolean DEFAULT false NOT NULL,
    release_listing_json text,
    http_release_json text
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
    stdout_tail bytea,
    stderr_tail bytea,
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
    debug boolean DEFAULT false NOT NULL,
    language text DEFAULT 'shell'::text NOT NULL,
    trigger text DEFAULT 'post_import'::text NOT NULL,
    schedule_json text,
    run_on_startup boolean DEFAULT false NOT NULL
);
CREATE TABLE proxy_configs (
    id text CONSTRAINT indexer_proxy_configs_id_not_null NOT NULL,
    name text CONSTRAINT indexer_proxy_configs_name_not_null NOT NULL,
    provider_type text CONSTRAINT indexer_proxy_configs_provider_type_not_null NOT NULL,
    protocol text,
    base_url text CONSTRAINT indexer_proxy_configs_base_url_not_null NOT NULL,
    request_timeout_seconds integer DEFAULT 60 CONSTRAINT indexer_proxy_configs_request_timeout_seconds_not_null NOT NULL,
    is_enabled boolean DEFAULT true CONSTRAINT indexer_proxy_configs_is_enabled_not_null NOT NULL,
    last_health_status text,
    last_error_message text,
    last_error_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() CONSTRAINT indexer_proxy_configs_created_at_not_null NOT NULL,
    updated_at timestamp with time zone DEFAULT now() CONSTRAINT indexer_proxy_configs_updated_at_not_null NOT NULL,
    username_encrypted text,
    password_encrypted text,
    remote_dns boolean DEFAULT false NOT NULL,
    private_key_encrypted text,
    private_key_passphrase_encrypted text,
    host_key_fingerprint text,
    host_key_pinned_at timestamp with time zone,
    peer_public_key text,
    preshared_key_encrypted text,
    tunnel_public_key text,
    tunnel_addresses text,
    tunnel_dns_servers text,
    tunnel_mtu integer,
    tunnel_keepalive_seconds integer
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
    explanation_json bytea,
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
CREATE TABLE request_rule_decisions (
    id text NOT NULL,
    request_id text NOT NULL,
    evaluated_at timestamp with time zone DEFAULT now() NOT NULL,
    mode text DEFAULT 'disabled'::text NOT NULL,
    effective_outcome text DEFAULT 'manual_review'::text NOT NULL,
    policy_outcome text DEFAULT 'manual_review'::text NOT NULL,
    fallback_reason text,
    votes_json text DEFAULT '[]'::text NOT NULL,
    tags_json text DEFAULT '[]'::text NOT NULL,
    input_hash text DEFAULT ''::text NOT NULL,
    input_schema_version bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE request_rule_revisions (
    id text NOT NULL,
    rule_set_id text NOT NULL,
    revision_number bigint NOT NULL,
    rego_source text NOT NULL,
    matcher_content_hash text NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE request_rule_set_libraries (
    rule_set_id text NOT NULL,
    library_id text NOT NULL,
    "position" bigint NOT NULL,
    CONSTRAINT request_rule_set_libraries_position_check CHECK (("position" >= 0))
);
CREATE TABLE request_rule_sets (
    id text NOT NULL,
    name text NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    enabled boolean DEFAULT false NOT NULL,
    evaluation_mode text DEFAULT 'disabled'::text NOT NULL,
    current_revision_number bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE rule_pack_installations (
    pack_id text NOT NULL,
    name text NOT NULL,
    version text NOT NULL,
    digest text NOT NULL,
    auto_update boolean NOT NULL,
    revision bigint NOT NULL,
    last_updated timestamp with time zone NOT NULL,
    last_error text,
    customizable boolean DEFAULT true NOT NULL
);
CREATE TABLE rule_pack_members (
    pack_id text NOT NULL,
    template_id text NOT NULL,
    rule_set_id text NOT NULL,
    removed boolean DEFAULT false NOT NULL
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
    applied_facets text DEFAULT '[]'::text NOT NULL,
    managed_tag_filter text,
    evaluation_phase text DEFAULT 'additional'::text NOT NULL,
    exclusive_group text,
    disabled_reason text,
    disabled_reason_code text
);
CREATE TABLE scope_indexer_coverage (
    scope_key text NOT NULL,
    facet text NOT NULL,
    indexer_id text NOT NULL,
    fingerprint text NOT NULL,
    searched_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE seeding_profiles (
    id text NOT NULL,
    name text NOT NULL,
    ratio double precision,
    seed_time_minutes bigint,
    season_pack_mode text DEFAULT 'inherit'::text NOT NULL,
    season_pack_ratio double precision,
    season_pack_seed_time_minutes bigint,
    honor_tracker_minimums boolean DEFAULT true NOT NULL,
    goal_met_action text DEFAULT 'remove_entry'::text NOT NULL,
    never_remove boolean DEFAULT false NOT NULL,
    minimum_seeders integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    post_import_tracking text DEFAULT 'park'::text NOT NULL
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
    updated_at timestamp with time zone NOT NULL,
    monitoring_override boolean,
    metadata_active boolean DEFAULT true NOT NULL,
    tags jsonb DEFAULT '[]'::jsonb NOT NULL
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
CREATE TABLE subtitle_search_backoffs (
    media_file_id text NOT NULL,
    language text NOT NULL,
    hearing_impaired boolean NOT NULL,
    forced boolean NOT NULL,
    file_path text NOT NULL,
    size_bytes bigint NOT NULL,
    wanted_languages text NOT NULL,
    enabled_providers text NOT NULL,
    empty_searches integer NOT NULL,
    searched_at timestamp with time zone NOT NULL,
    retry_after timestamp with time zone NOT NULL
);
CREATE TABLE title_anime_numbering_bridges (
    title_id text NOT NULL,
    generated_on text NOT NULL,
    corroborating_order text,
    seasons_json text DEFAULT '[]'::text NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    source text DEFAULT 'anime_community'::text NOT NULL
);
CREATE TABLE title_credits (
    title_id text,
    "position" integer NOT NULL,
    kind text NOT NULL,
    person_id text NOT NULL,
    person_name text DEFAULT ''::text NOT NULL,
    person_original_name text DEFAULT ''::text NOT NULL,
    person_image_url text DEFAULT ''::text NOT NULL,
    person_source text DEFAULT ''::text NOT NULL,
    person_external_id text DEFAULT ''::text NOT NULL,
    character_name text DEFAULT ''::text NOT NULL,
    language text DEFAULT ''::text NOT NULL,
    billing_order integer DEFAULT 0 NOT NULL,
    episode_count integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    movie_entity_id text,
    CONSTRAINT title_credits_owner_check CHECK (((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL)))
);
CREATE TABLE title_external_ids (
    id text NOT NULL,
    title_id text NOT NULL,
    facet text,
    source text NOT NULL,
    external_id text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone,
    library_id text,
    external_kind text DEFAULT ''::text NOT NULL,
    external_key text
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
    title_id text,
    source text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL,
    value double precision,
    score double precision,
    normalized double precision NOT NULL,
    votes integer,
    url text DEFAULT ''::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    movie_entity_id text,
    CONSTRAINT title_metadata_external_ratings_owner_check CHECK (((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL)))
);
CREATE TABLE title_metadata_rating_sources (
    title_id text,
    source text NOT NULL,
    sort_index integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    movie_entity_id text,
    CONSTRAINT title_metadata_rating_sources_owner_check CHECK (((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL)))
);
CREATE TABLE title_metadata_rating_summaries (
    title_id text,
    rating double precision,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    movie_entity_id text,
    CONSTRAINT title_metadata_rating_summaries_owner_check CHECK (((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL)))
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
    sort_index integer DEFAULT 0 NOT NULL,
    affinity_signals_json text DEFAULT '[]'::text NOT NULL
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
CREATE TABLE title_recommendation_cards (
    discovery_title_id text NOT NULL,
    payload_version integer DEFAULT 1 NOT NULL,
    payload_blob bytea,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE title_recycle_purges (
    title_id text NOT NULL,
    library_id text NOT NULL,
    recorded_at timestamp with time zone NOT NULL
);
CREATE TABLE title_search_collation_keys (
    term_id bigint NOT NULL,
    profile text NOT NULL,
    collation_key bytea NOT NULL
);
CREATE TABLE title_search_index_queue (
    seq bigint NOT NULL,
    title_id text NOT NULL
);
CREATE SEQUENCE title_search_index_queue_seq_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;
ALTER SEQUENCE title_search_index_queue_seq_seq OWNED BY title_search_index_queue.seq;
CREATE TABLE title_search_meta (
    id bigint NOT NULL,
    collation_version text DEFAULT ''::text NOT NULL,
    projection_generation bigint DEFAULT 0 NOT NULL,
    CONSTRAINT title_search_meta_single_row CHECK ((id = 1))
);
CREATE TABLE title_search_terms (
    term_id bigint NOT NULL,
    title_id text NOT NULL,
    facet text NOT NULL,
    term_kind text NOT NULL,
    raw_term text NOT NULL,
    normalized_term text NOT NULL,
    weight bigint NOT NULL,
    literal_term text DEFAULT ''::text NOT NULL,
    stripped_year_key text DEFAULT ''::text NOT NULL,
    match_term text DEFAULT ''::text NOT NULL,
    match_year bigint,
    script text DEFAULT 'other'::text NOT NULL,
    numbers_key text DEFAULT ''::text NOT NULL,
    char_length bigint DEFAULT 0 NOT NULL,
    romanization_key text,
    language_tag text,
    title_year bigint
);
CREATE SEQUENCE title_search_terms_term_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;
ALTER SEQUENCE title_search_terms_term_id_seq OWNED BY title_search_terms.term_id;
CREATE TABLE title_tag_definitions (
    id text NOT NULL,
    label text NOT NULL,
    description text,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
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
    popularity double precision,
    smg_identity_backfill_attempt_count bigint DEFAULT 0 NOT NULL,
    movie_release_dates_json text,
    catalog_name_tie_key text DEFAULT ''::text NOT NULL,
    folder_path_key text COLLATE pg_catalog."C",
    folder_conflict_reason text,
    folder_conflict_path text
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
    attempt_window_started_at timestamp with time zone,
    attempt_count integer DEFAULT 0 NOT NULL,
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
    auth_session_version text,
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
    CONSTRAINT user_external_accounts_provider_check CHECK ((provider = ANY (ARRAY['plex'::text, 'jellyfin'::text, 'emby'::text]))),
    CONSTRAINT user_external_accounts_status_check CHECK ((status = ANY (ARRAY['pending_claim'::text, 'active'::text, 'disabled'::text])))
);
CREATE TABLE user_library_permission_masks (
    user_id text NOT NULL,
    library_id text NOT NULL,
    permission_mask bigint NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE user_list_accounts (
    id text NOT NULL,
    user_id text NOT NULL,
    provider text NOT NULL,
    external_user_id text NOT NULL,
    username text NOT NULL,
    display_name text,
    credential_encrypted text NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    error_message text,
    linked_at timestamp with time zone NOT NULL,
    last_used_at timestamp with time zone,
    last_refresh_at timestamp with time zone,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE user_list_policies (
    user_id text NOT NULL,
    policy text NOT NULL,
    updated_by_user_id text,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE user_ui_catalog_views (
    user_id text NOT NULL,
    device_class text NOT NULL,
    facet text NOT NULL,
    view_mode text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE user_ui_dashboard_layout_items (
    user_id text NOT NULL,
    breakpoint text NOT NULL,
    panel_id text NOT NULL,
    x integer NOT NULL,
    y integer NOT NULL,
    w integer NOT NULL,
    h integer NOT NULL
);
CREATE TABLE user_ui_dashboard_layouts (
    user_id text NOT NULL,
    breakpoint text NOT NULL,
    column_count integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
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
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    palette text DEFAULT 'CLASSIC'::text NOT NULL,
    discovery_grid_hide_owned boolean DEFAULT true NOT NULL,
    discovery_grid_hide_requested boolean DEFAULT false NOT NULL,
    CONSTRAINT user_ui_settings_palette_check CHECK ((palette = ANY (ARRAY['CLASSIC'::text, 'SCRYER_DEFAULT'::text, 'SLATE'::text, 'OCEAN'::text, 'FOREST'::text, 'PLUM'::text, 'EMBER'::text, 'GRAPHITE'::text, 'PAPYRUS'::text])))
);
CREATE TABLE user_ui_table_columns (
    user_id text CONSTRAINT user_ui_table_columns_user_id_not_null1 NOT NULL,
    device_class text DEFAULT 'desktop'::text NOT NULL,
    facet text CONSTRAINT user_ui_table_columns_facet_not_null1 NOT NULL,
    table_view_mode text CONSTRAINT user_ui_table_columns_table_view_mode_not_null1 NOT NULL,
    column_id text CONSTRAINT user_ui_table_columns_column_id_not_null1 NOT NULL,
    column_order integer CONSTRAINT user_ui_table_columns_column_order_not_null1 NOT NULL,
    visible boolean DEFAULT true CONSTRAINT user_ui_table_columns_visible_not_null1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() CONSTRAINT user_ui_table_columns_created_at_not_null1 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() CONSTRAINT user_ui_table_columns_updated_at_not_null1 NOT NULL
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
    password_change_required boolean DEFAULT false NOT NULL,
    CONSTRAINT users_account_kind_check CHECK ((account_kind = ANY (ARRAY['local'::text, 'external_auto_provisioned'::text])))
);
CREATE TABLE wanted_items (
    id text NOT NULL,
    title_id text NOT NULL,
    episode_id text,
    collection_id text,
    media_type text NOT NULL,
    last_search_at timestamp with time zone,
    status text DEFAULT 'wanted'::text NOT NULL,
    grabbed_release text,
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
    purpose text DEFAULT 'standalone_authentication'::text NOT NULL,
    login_verification_challenge_id text,
    auth_session_version text,
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
CREATE TABLE workflow_private_inputs (
    run_id text NOT NULL,
    actor_user_id text NOT NULL,
    request_key text NOT NULL,
    input_encrypted text
);
ALTER TABLE ONLY domain_events ALTER COLUMN sequence SET DEFAULT nextval('domain_events_sequence_seq'::regclass);
ALTER TABLE ONLY title_search_index_queue ALTER COLUMN seq SET DEFAULT nextval('title_search_index_queue_seq_seq'::regclass);
ALTER TABLE ONLY title_search_terms ALTER COLUMN term_id SET DEFAULT nextval('title_search_terms_term_id_seq'::regclass);
ALTER TABLE ONLY api_keys
    ADD CONSTRAINT api_keys_lookup_id_key UNIQUE (lookup_id);
ALTER TABLE ONLY api_keys
    ADD CONSTRAINT api_keys_pkey PRIMARY KEY (id);
ALTER TABLE ONLY application_compatibility_journal
    ADD CONSTRAINT application_compatibility_journal_pkey PRIMARY KEY (migration_id, subject_id, source_digest);
ALTER TABLE ONLY application_migrations
    ADD CONSTRAINT application_migrations_pkey PRIMARY KEY (migration_id);
ALTER TABLE ONLY blocklist
    ADD CONSTRAINT blocklist_pkey PRIMARY KEY (id);
ALTER TABLE ONLY canonical_tag_vocabulary_cache
    ADD CONSTRAINT canonical_tag_vocabulary_cache_pkey PRIMARY KEY (id);
ALTER TABLE ONLY collection_external_ids
    ADD CONSTRAINT collection_external_ids_pkey PRIMARY KEY (id);
ALTER TABLE ONLY collections
    ADD CONSTRAINT collections_pkey PRIMARY KEY (id);
ALTER TABLE ONLY delay_profile_facets
    ADD CONSTRAINT delay_profile_facets_pkey PRIMARY KEY (profile_id, facet);
ALTER TABLE ONLY delay_profile_libraries
    ADD CONSTRAINT delay_profile_libraries_pkey PRIMARY KEY (profile_id, library_id);
ALTER TABLE ONLY delay_profile_tags
    ADD CONSTRAINT delay_profile_tags_pkey PRIMARY KEY (profile_id, label);
ALTER TABLE ONLY delay_profiles
    ADD CONSTRAINT delay_profiles_pkey PRIMARY KEY (id);
ALTER TABLE ONLY discovery_affinity_revision
    ADD CONSTRAINT discovery_affinity_revision_pkey PRIMARY KEY (id);
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
ALTER TABLE ONLY discovery_library_graph_payloads
    ADD CONSTRAINT discovery_library_graph_payloads_pkey PRIMARY KEY (run_id);
ALTER TABLE ONLY discovery_library_graph_runs
    ADD CONSTRAINT discovery_library_graph_runs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY discovery_neighbor_cache
    ADD CONSTRAINT discovery_neighbor_cache_pkey PRIMARY KEY (target_key, language);
ALTER TABLE ONLY discovery_pending_context_changes
    ADD CONSTRAINT discovery_pending_context_changes_pkey PRIMARY KEY (id);
ALTER TABLE ONLY discovery_presentation_selection
    ADD CONSTRAINT discovery_presentation_selection_pkey PRIMARY KEY (scope_key);
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
ALTER TABLE ONLY download_cleanup
    ADD CONSTRAINT download_cleanup_pkey PRIMARY KEY (download_id);
ALTER TABLE ONLY download_cleanup_seed_state
    ADD CONSTRAINT download_cleanup_seed_state_pkey PRIMARY KEY (id);
ALTER TABLE ONLY download_client_bindings
    ADD CONSTRAINT download_client_bindings_pkey PRIMARY KEY (download_id);
ALTER TABLE ONLY download_client_status
    ADD CONSTRAINT download_client_status_pkey PRIMARY KEY (client_config_id);
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
    ADD CONSTRAINT download_submission_episode_links_0180_pkey PRIMARY KEY (download_id, episode_id);
ALTER TABLE ONLY download_submissions
    ADD CONSTRAINT download_submissions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY downloads
    ADD CONSTRAINT downloads_pkey PRIMARY KEY (id);
ALTER TABLE ONLY emby_media_server_details
    ADD CONSTRAINT emby_media_server_details_pkey PRIMARY KEY (connection_id);
ALTER TABLE ONLY episode_external_ids
    ADD CONSTRAINT episode_external_ids_pkey PRIMARY KEY (id);
ALTER TABLE ONLY episodes
    ADD CONSTRAINT episodes_pkey PRIMARY KEY (id);
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
ALTER TABLE ONLY image_proxy_cache_entries
    ADD CONSTRAINT image_proxy_cache_entries_pkey PRIMARY KEY (token, variant);
ALTER TABLE ONLY image_proxy_sources
    ADD CONSTRAINT image_proxy_sources_pkey PRIMARY KEY (token);
ALTER TABLE ONLY import_space_incident_lock
    ADD CONSTRAINT import_space_incident_lock_pkey PRIMARY KEY (id);
ALTER TABLE ONLY import_space_incidents
    ADD CONSTRAINT import_space_incidents_pkey PRIMARY KEY (destination_key);
ALTER TABLE ONLY import_space_members
    ADD CONSTRAINT import_space_members_pkey PRIMARY KEY (member_key);
ALTER TABLE ONLY import_space_notification_attempts
    ADD CONSTRAINT import_space_notification_attempts_pkey PRIMARY KEY (event_id);
ALTER TABLE ONLY import_space_notification_receipts
    ADD CONSTRAINT import_space_notification_receipts_pkey PRIMARY KEY (event_id, target_key);
ALTER TABLE ONLY imports
    ADD CONSTRAINT imports_pkey PRIMARY KEY (id);
ALTER TABLE ONLY indexer_api_client_grants
    ADD CONSTRAINT indexer_api_client_grants_pkey PRIMARY KEY (client_id, indexer_id);
ALTER TABLE ONLY indexer_api_clients
    ADD CONSTRAINT indexer_api_clients_lookup_id_key UNIQUE (lookup_id);
ALTER TABLE ONLY indexer_api_clients
    ADD CONSTRAINT indexer_api_clients_pkey PRIMARY KEY (id);
ALTER TABLE ONLY indexer_api_quotas
    ADD CONSTRAINT indexer_api_quotas_pkey PRIMARY KEY (indexer_id);
ALTER TABLE ONLY indexer_errors
    ADD CONSTRAINT indexer_errors_pkey PRIMARY KEY (id);
ALTER TABLE ONLY indexer_facet_learning
    ADD CONSTRAINT indexer_facet_learning_pkey PRIMARY KEY (indexer_id, facet);
ALTER TABLE ONLY proxy_configs
    ADD CONSTRAINT indexer_proxy_configs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY indexer_search_candidate_sources
    ADD CONSTRAINT indexer_search_candidate_sour_candidate_id_indexer_id_sourc_key UNIQUE (candidate_id, indexer_id, source_identity);
ALTER TABLE ONLY indexer_search_candidate_source_values
    ADD CONSTRAINT indexer_search_candidate_source_values_pkey PRIMARY KEY (source_id, value_kind, ordinal);
ALTER TABLE ONLY indexer_search_candidate_sources
    ADD CONSTRAINT indexer_search_candidate_sources_pkey PRIMARY KEY (id);
ALTER TABLE ONLY indexer_search_candidates
    ADD CONSTRAINT indexer_search_candidates_fingerprint_key UNIQUE (fingerprint);
ALTER TABLE ONLY indexer_search_candidates
    ADD CONSTRAINT indexer_search_candidates_pkey PRIMARY KEY (id);
ALTER TABLE ONLY indexer_search_learning
    ADD CONSTRAINT indexer_search_learning_pkey PRIMARY KEY (indexer_id, title_id, facet, strategy_key);
ALTER TABLE ONLY indexer_search_listing_readers
    ADD CONSTRAINT indexer_search_listing_readers_pkey PRIMARY KEY (indexer_id, indexer_fingerprint, title_id, facet, scope_key, query_signature);
ALTER TABLE ONLY indexer_search_listing_runs
    ADD CONSTRAINT indexer_search_listing_runs_pkey PRIMARY KEY (run_id);
ALTER TABLE ONLY indexer_search_listings
    ADD CONSTRAINT indexer_search_listings_pkey PRIMARY KEY (indexer_id, indexer_fingerprint, title_id, facet, first_page_fingerprint);
ALTER TABLE ONLY indexer_search_run_candidate_sources
    ADD CONSTRAINT indexer_search_run_candidate_sources_pkey PRIMARY KEY (run_id, source_id);
ALTER TABLE ONLY indexer_search_runs
    ADD CONSTRAINT indexer_search_runs_pkey PRIMARY KEY (id);
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
ALTER TABLE ONLY library_root_id_remaps
    ADD CONSTRAINT library_root_id_remaps_pkey PRIMARY KEY (legacy_root_id);
ALTER TABLE ONLY library_roots
    ADD CONSTRAINT library_roots_pkey PRIMARY KEY (id);
ALTER TABLE ONLY library_scan_unmatched_items
    ADD CONSTRAINT library_scan_unmatched_items_pkey PRIMARY KEY (id);
ALTER TABLE ONLY lifecycle_action_runs
    ADD CONSTRAINT lifecycle_action_runs_idempotency_key_attempt_key UNIQUE (idempotency_key, attempt);
ALTER TABLE ONLY lifecycle_action_runs
    ADD CONSTRAINT lifecycle_action_runs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY lifecycle_candidates
    ADD CONSTRAINT lifecycle_candidates_pkey PRIMARY KEY (id);
ALTER TABLE ONLY lifecycle_claims
    ADD CONSTRAINT lifecycle_claims_pkey PRIMARY KEY (id);
ALTER TABLE ONLY list_exclusion_external_ids
    ADD CONSTRAINT list_exclusion_external_ids_pkey PRIMARY KEY (exclusion_id, source, value);
ALTER TABLE ONLY list_exclusions
    ADD CONSTRAINT list_exclusions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY list_memberships
    ADD CONSTRAINT list_memberships_pkey PRIMARY KEY (subscription_id, item_key);
ALTER TABLE ONLY list_subscription_routes
    ADD CONSTRAINT list_subscription_routes_pkey PRIMARY KEY (subscription_id, kind);
ALTER TABLE ONLY list_subscriptions
    ADD CONSTRAINT list_subscriptions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY list_sync_runs
    ADD CONSTRAINT list_sync_runs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY location_file_resolutions
    ADD CONSTRAINT location_file_resolutions_pkey PRIMARY KEY (operation_id, title_id, source_path);
ALTER TABLE ONLY location_operation_owned_entities
    ADD CONSTRAINT location_operation_owned_entities_pkey PRIMARY KEY (operation_id, entity_type, entity_id);
ALTER TABLE ONLY location_operation_plans
    ADD CONSTRAINT location_operation_plans_pkey PRIMARY KEY (operation_id);
ALTER TABLE ONLY location_operation_title_checkpoints
    ADD CONSTRAINT location_operation_title_checkpoints_pkey PRIMARY KEY (operation_id, title_id);
ALTER TABLE ONLY location_operation_verifications
    ADD CONSTRAINT location_operation_verifications_pkey PRIMARY KEY (id);
ALTER TABLE ONLY location_operations
    ADD CONSTRAINT location_operations_pkey PRIMARY KEY (id);
ALTER TABLE ONLY location_recovery_staging
    ADD CONSTRAINT location_recovery_staging_pkey PRIMARY KEY (id);
ALTER TABLE ONLY location_recovery_staging
    ADD CONSTRAINT location_recovery_staging_staging_path_key UNIQUE (staging_path);
ALTER TABLE ONLY location_transfer_progress
    ADD CONSTRAINT location_transfer_progress_pkey PRIMARY KEY (operation_id);
ALTER TABLE ONLY location_transfer_runtime
    ADD CONSTRAINT location_transfer_runtime_pkey PRIMARY KEY (id);
ALTER TABLE ONLY location_transfer_titles
    ADD CONSTRAINT location_transfer_titles_pkey PRIMARY KEY (operation_id, title_id);
ALTER TABLE ONLY login_verification_challenges
    ADD CONSTRAINT login_verification_challenges_pkey PRIMARY KEY (id);
ALTER TABLE ONLY maintenance_action_job_receipts
    ADD CONSTRAINT maintenance_action_job_receipts_pkey PRIMARY KEY (candidate_id, match_generation, revision_number, step_id, dispatch_attempt);
ALTER TABLE ONLY maintenance_action_step_attempts
    ADD CONSTRAINT maintenance_action_step_attempts_pkey PRIMARY KEY (id);
ALTER TABLE ONLY maintenance_action_steps
    ADD CONSTRAINT maintenance_action_steps_pkey PRIMARY KEY (candidate_id, match_generation, revision_number, step_id);
ALTER TABLE ONLY maintenance_evaluation_runs
    ADD CONSTRAINT maintenance_evaluation_runs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY maintenance_rule_exclusions
    ADD CONSTRAINT maintenance_rule_exclusions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY maintenance_rule_revisions
    ADD CONSTRAINT maintenance_rule_revisions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY maintenance_rule_revisions
    ADD CONSTRAINT maintenance_rule_revisions_rule_set_id_revision_number_key UNIQUE (rule_set_id, revision_number);
ALTER TABLE ONLY maintenance_rule_set_libraries
    ADD CONSTRAINT maintenance_rule_set_libraries_pkey PRIMARY KEY (rule_set_id, library_id);
ALTER TABLE ONLY maintenance_rule_set_libraries
    ADD CONSTRAINT maintenance_rule_set_libraries_rule_set_id_position_key UNIQUE (rule_set_id, "position");
ALTER TABLE ONLY maintenance_rule_sets
    ADD CONSTRAINT maintenance_rule_sets_pkey PRIMARY KEY (id);
ALTER TABLE ONLY maintenance_sequence_terminal_memberships
    ADD CONSTRAINT maintenance_sequence_terminal_memberships_candidate_id_key UNIQUE (candidate_id);
ALTER TABLE ONLY maintenance_sequence_terminal_memberships
    ADD CONSTRAINT maintenance_sequence_terminal_memberships_pkey PRIMARY KEY (id);
ALTER TABLE ONLY manual_import_selection_candidates
    ADD CONSTRAINT manual_import_selection_candida_selection_id_canonical_path_key UNIQUE (selection_id, canonical_path);
ALTER TABLE ONLY manual_import_selection_candidates
    ADD CONSTRAINT manual_import_selection_candidates_pkey PRIMARY KEY (id);
ALTER TABLE ONLY manual_import_selections
    ADD CONSTRAINT manual_import_selections_pkey PRIMARY KEY (id);
ALTER TABLE ONLY media_file_analyses
    ADD CONSTRAINT media_file_analyses_pkey PRIMARY KEY (media_file_id);
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
ALTER TABLE ONLY media_server_audience_plays
    ADD CONSTRAINT media_server_audience_plays_pkey PRIMARY KEY (connection_id, external_account_id, provider_item_id);
ALTER TABLE ONLY media_server_audience_reads
    ADD CONSTRAINT media_server_audience_reads_pkey PRIMARY KEY (connection_id);
ALTER TABLE ONLY media_server_connections
    ADD CONSTRAINT media_server_connections_pkey PRIMARY KEY (id);
ALTER TABLE ONLY media_server_default_library_grants
    ADD CONSTRAINT media_server_default_library_grants_pkey PRIMARY KEY (connection_id, library_id);
ALTER TABLE ONLY media_server_path_mappings
    ADD CONSTRAINT media_server_path_mappings_pkey PRIMARY KEY (id);
ALTER TABLE ONLY media_server_playback_items
    ADD CONSTRAINT media_server_playback_items_pkey PRIMARY KEY (connection_id, entity_kind, entity_id);
ALTER TABLE ONLY media_server_signal_roster
    ADD CONSTRAINT media_server_signal_roster_pkey PRIMARY KEY (connection_id, link_id);
ALTER TABLE ONLY media_server_signal_sync_state
    ADD CONSTRAINT media_server_signal_sync_state_pkey PRIMARY KEY (connection_id);
ALTER TABLE ONLY media_server_user_media_signals
    ADD CONSTRAINT media_server_user_media_signals_pkey PRIMARY KEY (id);
ALTER TABLE ONLY monitor_selections
    ADD CONSTRAINT monitor_selections_pkey PRIMARY KEY (owner_kind, owner_id, entry_kind, entry_key);
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
ALTER TABLE ONLY oauth_client_redirect_uris
    ADD CONSTRAINT oauth_client_redirect_uris_pkey PRIMARY KEY (client_id, redirect_uri);
ALTER TABLE ONLY oauth_client_registrations
    ADD CONSTRAINT oauth_client_registrations_pkey PRIMARY KEY (client_id);
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
ALTER TABLE ONLY request_rule_decisions
    ADD CONSTRAINT request_rule_decisions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY request_rule_revisions
    ADD CONSTRAINT request_rule_revisions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY request_rule_revisions
    ADD CONSTRAINT request_rule_revisions_rule_set_id_revision_number_key UNIQUE (rule_set_id, revision_number);
ALTER TABLE ONLY request_rule_set_libraries
    ADD CONSTRAINT request_rule_set_libraries_pkey PRIMARY KEY (rule_set_id, library_id);
ALTER TABLE ONLY request_rule_set_libraries
    ADD CONSTRAINT request_rule_set_libraries_rule_set_id_position_key UNIQUE (rule_set_id, "position");
ALTER TABLE ONLY request_rule_sets
    ADD CONSTRAINT request_rule_sets_pkey PRIMARY KEY (id);
ALTER TABLE ONLY rule_pack_installations
    ADD CONSTRAINT rule_pack_installations_pkey PRIMARY KEY (pack_id);
ALTER TABLE ONLY rule_pack_members
    ADD CONSTRAINT rule_pack_members_pkey PRIMARY KEY (pack_id, template_id);
ALTER TABLE ONLY rule_pack_members
    ADD CONSTRAINT rule_pack_members_rule_set_id_key UNIQUE (rule_set_id);
ALTER TABLE ONLY rule_set_history
    ADD CONSTRAINT rule_set_history_pkey PRIMARY KEY (id);
ALTER TABLE ONLY rule_sets
    ADD CONSTRAINT rule_sets_pkey PRIMARY KEY (id);
ALTER TABLE ONLY scope_indexer_coverage
    ADD CONSTRAINT scope_indexer_coverage_pkey PRIMARY KEY (scope_key, facet, indexer_id);
ALTER TABLE ONLY seeding_profiles
    ADD CONSTRAINT seeding_profiles_pkey PRIMARY KEY (id);
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
ALTER TABLE ONLY subtitle_search_backoffs
    ADD CONSTRAINT subtitle_search_backoffs_pkey PRIMARY KEY (media_file_id, language, hearing_impaired, forced);
ALTER TABLE ONLY title_anime_numbering_bridges
    ADD CONSTRAINT title_anime_numbering_bridges_pkey PRIMARY KEY (title_id);
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
ALTER TABLE ONLY title_metadata_tag_source_keys
    ADD CONSTRAINT title_metadata_tag_source_key_title_id_tag_key_source_tag_k_key UNIQUE (title_id, tag_key, source_tag_key);
ALTER TABLE ONLY title_metadata_tag_sources
    ADD CONSTRAINT title_metadata_tag_sources_title_id_tag_key_source_key UNIQUE (title_id, tag_key, source);
ALTER TABLE ONLY title_metadata_tags
    ADD CONSTRAINT title_metadata_tags_pkey PRIMARY KEY (title_id, tag_key);
ALTER TABLE ONLY title_more_like_this_items
    ADD CONSTRAINT title_more_like_this_items_source_title_id_discovery_title__key UNIQUE (source_title_id, discovery_title_id);
ALTER TABLE ONLY title_recommendation_cards
    ADD CONSTRAINT title_recommendation_cards_pkey PRIMARY KEY (discovery_title_id);
ALTER TABLE ONLY title_recycle_purges
    ADD CONSTRAINT title_recycle_purges_pkey PRIMARY KEY (title_id);
ALTER TABLE ONLY title_search_collation_keys
    ADD CONSTRAINT title_search_collation_keys_pkey PRIMARY KEY (term_id, profile);
ALTER TABLE ONLY title_search_index_queue
    ADD CONSTRAINT title_search_index_queue_pkey PRIMARY KEY (seq);
ALTER TABLE ONLY title_search_meta
    ADD CONSTRAINT title_search_meta_pkey PRIMARY KEY (id);
ALTER TABLE ONLY title_search_terms
    ADD CONSTRAINT title_search_terms_pkey PRIMARY KEY (term_id);
ALTER TABLE ONLY title_tag_definitions
    ADD CONSTRAINT title_tag_definitions_pkey PRIMARY KEY (id);
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
ALTER TABLE ONLY user_list_accounts
    ADD CONSTRAINT user_list_accounts_pkey PRIMARY KEY (id);
ALTER TABLE ONLY user_list_policies
    ADD CONSTRAINT user_list_policies_pkey PRIMARY KEY (user_id);
ALTER TABLE ONLY user_ui_catalog_views
    ADD CONSTRAINT user_ui_catalog_views_pkey PRIMARY KEY (user_id, device_class, facet);
ALTER TABLE ONLY user_ui_dashboard_layout_items
    ADD CONSTRAINT user_ui_dashboard_layout_items_pkey PRIMARY KEY (user_id, breakpoint, panel_id);
ALTER TABLE ONLY user_ui_dashboard_layouts
    ADD CONSTRAINT user_ui_dashboard_layouts_pkey PRIMARY KEY (user_id, breakpoint);
ALTER TABLE ONLY user_ui_settings
    ADD CONSTRAINT user_ui_settings_pkey PRIMARY KEY (user_id);
ALTER TABLE ONLY user_ui_table_columns
    ADD CONSTRAINT user_ui_table_columns_pkey PRIMARY KEY (user_id, device_class, facet, table_view_mode, column_id);
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
ALTER TABLE ONLY workflow_private_inputs
    ADD CONSTRAINT workflow_private_inputs_actor_user_id_request_key_key UNIQUE (actor_user_id, request_key);
ALTER TABLE ONLY workflow_private_inputs
    ADD CONSTRAINT workflow_private_inputs_pkey PRIMARY KEY (run_id);
CREATE INDEX idx_api_keys_user_created ON api_keys USING btree (user_id, created_at DESC);
CREATE UNIQUE INDEX idx_blocklist_info_hash_unique ON blocklist USING btree (title_id, info_hash) WHERE (info_hash IS NOT NULL);
CREATE UNIQUE INDEX idx_blocklist_release_unique ON blocklist USING btree (title_id, indexer_id, normalized_release_name) WHERE (info_hash IS NULL);
CREATE INDEX idx_blocklist_title_id ON blocklist USING btree (title_id);
CREATE INDEX idx_collection_external_ids_title_provenance ON collection_external_ids USING btree (title_id, provenance);
CREATE UNIQUE INDEX idx_collection_external_ids_unique ON collection_external_ids USING btree (collection_id, source, external_id, provenance, source_scope);
CREATE INDEX idx_collections_title ON collections USING btree (title_id, collection_type);
CREATE INDEX idx_delay_profile_libraries_library_id ON delay_profile_libraries USING btree (library_id);
CREATE INDEX idx_delay_profile_tags_label ON delay_profile_tags USING btree (label);
CREATE INDEX idx_discovery_item_library_provenance_item ON discovery_item_library_provenance USING btree (item_id, subject_key, library_id, title_id);
CREATE INDEX idx_discovery_item_library_provenance_library ON discovery_item_library_provenance USING btree (run_id, library_id, item_id);
CREATE INDEX idx_discovery_item_rank_components_run ON discovery_item_rank_components USING btree (run_id);
CREATE INDEX idx_discovery_item_subject_links_item ON discovery_item_subject_links USING btree (item_id, link_type, sort_index);
CREATE INDEX idx_discovery_item_subject_links_run_type_key ON discovery_item_subject_links USING btree (run_id, link_type, subject_key, item_id);
CREATE INDEX idx_discovery_items_active_title ON discovery_items USING btree (base_generation_id, discovery_title_id, tombstoned_at);
CREATE INDEX idx_discovery_items_discovery_title ON discovery_items USING btree (discovery_title_id);
CREATE INDEX idx_discovery_items_generation_rank ON discovery_items USING btree (base_generation_id, tombstoned_at, owned_in_input);
CREATE INDEX idx_discovery_items_generation_recommendation ON discovery_items USING btree (base_generation_id, recommendation_score DESC, rank_score DESC, sort_index);
CREATE INDEX idx_discovery_items_generation_relevance ON discovery_items USING btree (base_generation_id, owned_in_input, COALESCE(recommendation_score, ('-999999999.0'::numeric)::double precision) DESC, COALESCE(rank_score, ('-999999999.0'::numeric)::double precision) DESC, sort_index) WHERE (tombstoned_at IS NULL);
CREATE INDEX idx_discovery_items_run_section ON discovery_items USING btree (run_id, section_id, sort_index);
CREATE INDEX idx_discovery_items_section ON discovery_items USING btree (section_id, sort_index, rank_score);
CREATE INDEX idx_discovery_items_tombstoned_by_run ON discovery_items USING btree (tombstoned_by_run_id) WHERE (tombstoned_by_run_id IS NOT NULL);
CREATE INDEX idx_discovery_library_graph_runs_facet_status_started ON discovery_library_graph_runs USING btree (facet, status, started_at);
CREATE INDEX idx_discovery_neighbor_cache_fetched_at ON discovery_neighbor_cache USING btree (fetched_at);
CREATE INDEX idx_discovery_pending_changes_scope_seen ON discovery_pending_context_changes USING btree (scope_key, last_seen_at);
CREATE INDEX idx_discovery_pending_changes_scope_sequence ON discovery_pending_context_changes USING btree (scope_key, last_seen_sequence);
CREATE INDEX idx_discovery_pending_changes_title ON discovery_pending_context_changes USING btree (title_id);
CREATE INDEX idx_discovery_section_items_item ON discovery_section_items USING btree (item_id);
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
CREATE INDEX idx_discovery_title_terms_kind_value ON discovery_title_terms USING btree (term_kind, term_value, discovery_title_id);
CREATE INDEX idx_discovery_title_terms_title ON discovery_title_terms USING btree (discovery_title_id, term_kind, sort_index);
CREATE INDEX idx_discovery_titles_resolved_title ON discovery_titles USING btree (resolved_title_id);
CREATE INDEX idx_domain_events_event_type_occurred_at ON domain_events USING btree (event_type, occurred_at);
CREATE INDEX idx_domain_events_event_type_sequence ON domain_events USING btree (event_type, sequence DESC);
CREATE INDEX idx_domain_events_stream_sequence ON domain_events USING btree (stream_id, sequence) WHERE (stream_id IS NOT NULL);
CREATE INDEX idx_domain_events_title_sequence ON domain_events USING btree (title_id, sequence DESC) WHERE (title_id IS NOT NULL);
CREATE INDEX idx_download_cleanup_due ON download_cleanup USING btree (status, next_attempt_at, client_id);
CREATE UNIQUE INDEX idx_download_client_bindings_active_locator_unique ON download_client_bindings USING btree (client_config_id, client_type_snapshot, native_item_id) WHERE ((native_item_id IS NOT NULL) AND (ended_at IS NULL));
CREATE INDEX idx_download_client_bindings_locator ON download_client_bindings USING btree (client_config_id, client_type_snapshot, native_item_id);
CREATE INDEX idx_download_client_status_disabled_until ON download_client_status USING btree (disabled_until);
CREATE INDEX idx_download_clients_client_priority ON download_clients USING btree (client_priority);
CREATE UNIQUE INDEX idx_download_clients_name ON download_clients USING btree (name);
CREATE INDEX idx_download_identity_states_canonical_latest ON download_identity_states USING btree (canonical_download_id, updated_at DESC, id DESC);
CREATE INDEX idx_download_identity_states_client_item ON download_identity_states USING btree (client_type, download_client_item_id, client_id);
CREATE INDEX idx_download_identity_states_download_id ON download_identity_states USING btree (client_id, client_type, download_id);
CREATE INDEX idx_download_identity_states_reason_canonical ON download_identity_states USING btree (reason, canonical_download_id);
CREATE INDEX idx_download_identity_states_updated_at ON download_identity_states USING btree (updated_at);
CREATE INDEX idx_download_import_artifacts_episode ON download_import_artifacts USING btree (episode_id, result);
CREATE INDEX idx_download_import_artifacts_import_episode ON download_import_artifacts USING btree (import_id, episode_id, result, created_at DESC);
CREATE INDEX idx_download_import_artifacts_imported_media_file ON download_import_artifacts USING btree (imported_media_file_id);
CREATE INDEX idx_download_import_artifacts_retention ON download_import_artifacts USING btree (created_at, import_id);
CREATE INDEX idx_download_import_artifacts_source ON download_import_artifacts USING btree (COALESCE(source_client_id, ''::text), source_system, source_ref, created_at);
CREATE INDEX idx_download_import_artifacts_title ON download_import_artifacts USING btree (title_id);
CREATE UNIQUE INDEX idx_download_queue_commands_active_unique ON download_queue_commands USING btree (action, COALESCE(client_id, ''::text), client_type, download_client_item_id, is_history) WHERE (status = ANY (ARRAY['queued'::text, 'running'::text]));
CREATE INDEX idx_download_queue_commands_source ON download_queue_commands USING btree (COALESCE(client_id, ''::text), client_type, download_client_item_id, is_history, created_at DESC);
CREATE INDEX idx_download_queue_commands_status ON download_queue_commands USING btree (action, status, updated_at);
CREATE INDEX idx_download_submission_episode_links_episode ON download_submission_episode_links USING btree (episode_id);
CREATE INDEX idx_download_submissions_client_item ON download_submissions USING btree (download_client_type, download_client_item_id, download_client_id);
CREATE INDEX idx_download_submissions_download_id ON download_submissions USING btree (download_client_id, download_client_type, download_id);
CREATE INDEX idx_download_submissions_seed_info_hash ON download_submissions USING btree (seed_info_hash);
CREATE INDEX idx_download_submissions_title_request_signature ON download_submissions USING btree (title_id, request_signature);
CREATE INDEX idx_download_submissions_tracked_state_at ON download_submissions USING btree (tracked_state_at);
CREATE INDEX idx_downloads_created_at_id ON downloads USING btree (created_at, id);
CREATE INDEX idx_episode_external_ids_title_provenance ON episode_external_ids USING btree (title_id, provenance);
CREATE UNIQUE INDEX idx_episode_external_ids_unique ON episode_external_ids USING btree (episode_id, source, external_id, provenance, source_scope);
CREATE INDEX idx_episodes_air_date ON episodes USING btree (air_date);
CREATE INDEX idx_episodes_collection ON episodes USING btree (collection_id);
CREATE INDEX idx_episodes_title ON episodes USING btree (title_id, season_number);
CREATE INDEX idx_external_subtitle_probe_cache_file_path ON external_subtitle_probe_cache USING btree (file_path);
CREATE INDEX idx_file_episode_map_episode ON file_episode_map USING btree (episode_id);
CREATE UNIQUE INDEX idx_file_episode_map_one_primary_per_episode ON file_episode_map USING btree (episode_id) WHERE (role = 'primary'::text);
CREATE INDEX idx_file_series_movie_link_map_link ON file_series_movie_link_map USING btree (series_movie_link_id);
CREATE INDEX idx_history_events_actor_user ON history_events USING btree (actor_user_id) WHERE (actor_user_id IS NOT NULL);
CREATE INDEX idx_history_events_occurred_at ON history_events USING btree (occurred_at DESC);
CREATE INDEX idx_history_events_title_time ON history_events USING btree (title_id, occurred_at DESC);
CREATE INDEX idx_history_events_type_time ON history_events USING btree (event_type, occurred_at DESC);
CREATE INDEX idx_image_proxy_cache_entries_last_accessed_at ON image_proxy_cache_entries USING btree (last_accessed_at);
CREATE INDEX idx_image_proxy_sources_last_seen_at ON image_proxy_sources USING btree (last_seen_at);
CREATE INDEX idx_import_space_members_destination ON import_space_members USING btree (destination_key);
CREATE INDEX idx_import_space_members_job ON import_space_members USING btree (job_key);
CREATE UNIQUE INDEX idx_imports_active_download_id ON imports USING btree (COALESCE(source_client_id, ''::text), source_system, download_id) WHERE ((download_id IS NOT NULL) AND (status = ANY (ARRAY['pending'::text, 'running'::text, 'processing'::text])));
CREATE INDEX idx_imports_canonical_download_id ON imports USING btree (canonical_download_id);
CREATE INDEX idx_imports_download_id ON imports USING btree (source_client_id, source_system, download_id);
CREATE INDEX idx_imports_source_identity ON imports USING btree (COALESCE(source_client_id, ''::text), source_system, source_ref);
CREATE UNIQUE INDEX idx_imports_source_ref ON imports USING btree (COALESCE(source_client_id, ''::text), source_system, source_ref, import_type) WHERE (download_id IS NULL);
CREATE INDEX idx_imports_status_updated_at ON imports USING btree (status, updated_at);
CREATE INDEX idx_imports_type_status_updated_at ON imports USING btree (import_type, status, updated_at);
CREATE INDEX idx_indexer_errors_indexer_occurred_at_id ON indexer_errors USING btree (indexer_id, occurred_at DESC, id DESC);
CREATE INDEX idx_indexer_errors_occurred_at ON indexer_errors USING btree (occurred_at);
CREATE INDEX idx_indexer_search_candidates_expiry ON indexer_search_candidates USING btree (expires_at);
CREATE INDEX idx_indexer_search_learning_title_id ON indexer_search_learning USING btree (title_id);
CREATE INDEX idx_indexer_search_listing_readers_listing ON indexer_search_listing_readers USING btree (indexer_id, indexer_fingerprint, title_id, facet, first_page_fingerprint);
CREATE INDEX idx_indexer_search_listing_runs_listing ON indexer_search_listing_runs USING btree (indexer_id, indexer_fingerprint, title_id, facet, first_page_fingerprint, created_at);
CREATE INDEX idx_indexer_search_listings_owner ON indexer_search_listings USING btree (indexer_id, owner_scope_key, owner_query_signature);
CREATE INDEX idx_indexer_search_run_sources_session ON indexer_search_run_candidate_sources USING btree (search_session_id);
CREATE INDEX idx_indexer_search_run_sources_source ON indexer_search_run_candidate_sources USING btree (source_id);
CREATE INDEX idx_indexer_search_runs_coverage_scope ON indexer_search_runs USING btree (coverage_scope_key, indexer_id, created_at DESC);
CREATE INDEX idx_indexer_search_runs_indexer_created ON indexer_search_runs USING btree (indexer_id, created_at DESC);
CREATE INDEX idx_indexer_search_runs_scope_created ON indexer_search_runs USING btree (scope_key, created_at DESC);
CREATE INDEX idx_indexer_search_runs_session ON indexer_search_runs USING btree (search_session_id);
CREATE INDEX idx_indexer_search_sources_indexer_reusable ON indexer_search_candidate_sources USING btree (indexer_id, reusable_until);
CREATE INDEX idx_indexer_system_backoffs_disabled_until ON indexer_system_backoffs USING btree (disabled_until);
CREATE INDEX idx_indexers_download_client_id ON indexers USING btree (download_client_id);
CREATE UNIQUE INDEX idx_indexers_managed_child_identity ON indexers USING btree (managed_parent_config_id, managed_child_key) WHERE ((managed_parent_config_id IS NOT NULL) AND (managed_child_key IS NOT NULL));
CREATE INDEX idx_indexers_managed_parent ON indexers USING btree (managed_parent_config_id);
CREATE INDEX idx_indexers_proxy_config_id ON indexers USING btree (proxy_config_id);
CREATE INDEX idx_indexers_seeding_profile_id ON indexers USING btree (seeding_profile_id);
CREATE UNIQUE INDEX idx_libraries_facet_slug ON libraries USING btree (facet, slug);
CREATE INDEX idx_library_probe_signatures_last_probed ON library_probe_signatures USING btree (last_probed_at DESC);
CREATE INDEX idx_library_root_id_remaps_root ON library_root_id_remaps USING btree (root_id);
CREATE INDEX idx_library_roots_library ON library_roots USING btree (library_id, is_default DESC, path);
CREATE UNIQUE INDEX idx_library_roots_normalized_path ON library_roots USING btree (normalized_path);
CREATE INDEX idx_library_scan_unmatched_items_facet_status_updated ON library_scan_unmatched_items USING btree (facet, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_facet_title_status_updated ON library_scan_unmatched_items USING btree (facet, title_id, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_facet_updated ON library_scan_unmatched_items USING btree (facet, updated_at DESC);
CREATE UNIQUE INDEX idx_library_scan_unmatched_items_library_path ON library_scan_unmatched_items USING btree (library_id, item_path);
CREATE INDEX idx_library_scan_unmatched_items_library_updated ON library_scan_unmatched_items USING btree (library_id, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_root_status_updated ON library_scan_unmatched_items USING btree (facet, scan_root, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_root_updated ON library_scan_unmatched_items USING btree (facet, scan_root, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_title_id ON library_scan_unmatched_items USING btree (title_id);
CREATE INDEX idx_lifecycle_action_runs_candidate ON lifecycle_action_runs USING btree (candidate_id);
CREATE INDEX idx_lifecycle_action_runs_generation_started ON lifecycle_action_runs USING btree (candidate_id, match_generation, started_at, id);
CREATE INDEX idx_lifecycle_action_runs_rule_set ON lifecycle_action_runs USING btree (rule_set_id, started_at DESC);
CREATE INDEX idx_lifecycle_action_runs_status_finished ON lifecycle_action_runs USING btree (status, finished_at);
CREATE UNIQUE INDEX idx_lifecycle_candidates_active_subject ON lifecycle_candidates USING btree (rule_set_id, subject_kind, subject_id) WHERE (state <> ALL (ARRAY['succeeded'::text, 'failed'::text, 'canceled'::text, 'excluded'::text]));
CREATE INDEX idx_lifecycle_candidates_rule_state ON lifecycle_candidates USING btree (rule_set_id, state);
CREATE INDEX idx_lifecycle_candidates_subject_generation ON lifecycle_candidates USING btree (rule_set_id, subject_kind, subject_id, match_generation);
CREATE INDEX idx_lifecycle_candidates_title ON lifecycle_candidates USING btree (title_id);
CREATE UNIQUE INDEX idx_lifecycle_claims_live_operator_keep ON lifecycle_claims USING btree (title_id) WHERE ((producer = 'operator_keep'::text) AND (state = ANY (ARRAY['dormant'::text, 'active'::text])));
CREATE UNIQUE INDEX idx_lifecycle_claims_live_producer ON lifecycle_claims USING btree (producer, producer_ref) WHERE ((state = ANY (ARRAY['dormant'::text, 'active'::text])) AND (producer_ref IS NOT NULL));
CREATE INDEX idx_lifecycle_claims_title_state ON lifecycle_claims USING btree (title_id, state);
CREATE INDEX idx_list_exclusion_external_ids_lookup ON list_exclusion_external_ids USING btree (source, value);
CREATE INDEX idx_list_exclusions_subscription ON list_exclusions USING btree (subscription_id);
CREATE INDEX idx_list_memberships_request ON list_memberships USING btree (request_id);
CREATE INDEX idx_list_memberships_title ON list_memberships USING btree (title_id);
CREATE INDEX idx_list_subscriptions_due ON list_subscriptions USING btree (enabled, next_sync_at);
CREATE INDEX idx_list_subscriptions_owner ON list_subscriptions USING btree (owner_user_id);
CREATE INDEX idx_list_sync_runs_subscription ON list_sync_runs USING btree (subscription_id, started_at);
CREATE UNIQUE INDEX idx_location_operation_owned_entities_active ON location_operation_owned_entities USING btree (entity_type, entity_id) WHERE (released_at IS NULL);
CREATE INDEX idx_location_operation_owned_entities_operation ON location_operation_owned_entities USING btree (operation_id, released_at);
CREATE INDEX idx_location_operation_title_checkpoints_order ON location_operation_title_checkpoints USING btree (operation_id, sequence);
CREATE INDEX idx_location_operation_title_checkpoints_state ON location_operation_title_checkpoints USING btree (operation_id, state);
CREATE INDEX idx_location_operation_title_checkpoints_title ON location_operation_title_checkpoints USING btree (title_id);
CREATE UNIQUE INDEX idx_location_operation_verifications_destination ON location_operation_verifications USING btree (operation_id, destination_path);
CREATE INDEX idx_location_operation_verifications_media_file ON location_operation_verifications USING btree (media_file_id);
CREATE INDEX idx_location_operation_verifications_title ON location_operation_verifications USING btree (operation_id, title_id);
CREATE INDEX idx_location_operations_destination_root ON location_operations USING btree (destination_root_id);
CREATE UNIQUE INDEX idx_location_operations_recovery_key ON location_operations USING btree (recovery_key);
CREATE INDEX idx_location_operations_source_root ON location_operations USING btree (source_root_id);
CREATE INDEX idx_location_operations_state ON location_operations USING btree (state, updated_at DESC);
CREATE INDEX idx_location_transfer_titles_page ON location_transfer_titles USING btree (operation_id, hot_rank, sequence, title_id);
CREATE INDEX idx_login_verification_challenges_expires_at ON login_verification_challenges USING btree (expires_at);
CREATE INDEX idx_login_verification_challenges_user_id ON login_verification_challenges USING btree (user_id);
CREATE UNIQUE INDEX idx_maintenance_action_job_receipts_accepted_request ON maintenance_action_job_receipts USING btree (candidate_id, match_generation, revision_number, step_id, request_hash) WHERE (state = ANY (ARRAY['accepted'::text, 'completed'::text]));
CREATE INDEX idx_maintenance_action_job_receipts_step ON maintenance_action_job_receipts USING btree (candidate_id, match_generation, revision_number, step_id, dispatch_attempt DESC);
CREATE INDEX idx_maintenance_action_step_attempts_step ON maintenance_action_step_attempts USING btree (candidate_id, match_generation, revision_number, step_id, attempt, started_at DESC);
CREATE INDEX idx_maintenance_evaluation_runs_rule_set ON maintenance_evaluation_runs USING btree (rule_set_id, started_at DESC);
CREATE INDEX idx_maintenance_evaluation_runs_started ON maintenance_evaluation_runs USING btree (started_at DESC, id);
CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_global_subject ON maintenance_rule_exclusions USING btree (subject_kind, subject_id) WHERE (rule_set_id IS NULL);
CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_rule_subject ON maintenance_rule_exclusions USING btree (rule_set_id, subject_kind, subject_id) WHERE (rule_set_id IS NOT NULL);
CREATE INDEX idx_maintenance_rule_exclusions_title ON maintenance_rule_exclusions USING btree (title_id);
CREATE INDEX idx_maintenance_rule_revisions_rule_set ON maintenance_rule_revisions USING btree (rule_set_id, revision_number DESC);
CREATE INDEX idx_maintenance_rule_set_libraries_library ON maintenance_rule_set_libraries USING btree (library_id);
CREATE UNIQUE INDEX idx_maintenance_sequence_terminal_memberships_active ON maintenance_sequence_terminal_memberships USING btree (rule_set_id, revision_number, subject_kind, subject_id) WHERE (released_at IS NULL);
CREATE INDEX idx_maintenance_sequence_terminal_memberships_active_page ON maintenance_sequence_terminal_memberships USING btree (rule_set_id, revision_number, id) WHERE (released_at IS NULL);
CREATE INDEX idx_manual_import_selections_canonical_download ON manual_import_selections USING btree (canonical_download_id, actor_user_id, title_id, updated_at DESC);
CREATE INDEX idx_manual_import_selections_owner ON manual_import_selections USING btree (actor_user_id, title_id, source_client_id, source_system, source_ref);
CREATE INDEX idx_manual_import_selections_source ON manual_import_selections USING btree (source_client_id, source_system, source_ref);
CREATE INDEX idx_manual_import_selections_title_id ON manual_import_selections USING btree (title_id);
CREATE INDEX idx_media_files_analysis_refresh ON media_files USING btree (analysis_revision, analysis_attempted_at, id);
CREATE INDEX idx_media_files_full_blake3 ON media_files USING btree (full_blake3) WHERE (full_blake3 IS NOT NULL);
CREATE INDEX idx_media_files_full_hash_missing ON media_files USING btree (id) WHERE (full_blake3 IS NULL);
CREATE INDEX idx_media_files_id_role_path ON media_files USING btree (id, role, file_path);
CREATE INDEX idx_media_files_title_attention ON media_files USING btree (title_id) WHERE ((role = 'primary'::text) AND (scan_status = ANY (ARRAY['scan_failed'::text, 'review_required'::text])));
CREATE INDEX idx_media_files_title_path ON media_files USING btree (title_id, file_path);
CREATE INDEX idx_media_request_external_ids_lookup ON media_request_external_ids USING btree (library_id, source, external_id);
CREATE INDEX idx_media_request_requesters_user ON media_request_requesters USING btree (user_id);
CREATE INDEX idx_media_requests_created_title ON media_requests USING btree (created_title_id);
CREATE INDEX idx_media_requests_fingerprint_created ON media_requests USING btree (identity_fingerprint, created_at, id);
CREATE INDEX idx_media_requests_library_facet_status ON media_requests USING btree (library_id, facet, status);
CREATE INDEX idx_media_requests_requester_created ON media_requests USING btree (created_by_user_id, created_at, id);
CREATE INDEX idx_media_requests_status_updated ON media_requests USING btree (status, updated_at);
CREATE INDEX idx_media_server_audience_plays_title ON media_server_audience_plays USING btree (scryer_title_id);
CREATE INDEX idx_media_server_connections_provider ON media_server_connections USING btree (provider, enabled);
CREATE INDEX idx_media_server_path_mappings_connection ON media_server_path_mappings USING btree (connection_id, sort_order);
CREATE INDEX idx_media_server_playback_items_entity ON media_server_playback_items USING btree (entity_kind, entity_id);
CREATE INDEX idx_media_server_signals_episode ON media_server_user_media_signals USING btree (scryer_episode_id);
CREATE UNIQUE INDEX idx_media_server_signals_participant_item ON media_server_user_media_signals USING btree (connection_id, external_user_id, provider_item_id);
CREATE INDEX idx_media_server_signals_title ON media_server_user_media_signals USING btree (scryer_title_id);
CREATE INDEX idx_movie_entities_anidb_id ON movie_entities USING btree (anidb_id) WHERE ((anidb_id IS NOT NULL) AND (anidb_id <> ''::text));
CREATE INDEX idx_movie_entities_imdb_id ON movie_entities USING btree (imdb_id) WHERE ((imdb_id IS NOT NULL) AND (imdb_id <> ''::text));
CREATE INDEX idx_movie_entities_mal_id ON movie_entities USING btree (mal_id) WHERE ((mal_id IS NOT NULL) AND (mal_id <> ''::text));
CREATE INDEX idx_movie_entities_tmdb_id ON movie_entities USING btree (tmdb_id) WHERE ((tmdb_id IS NOT NULL) AND (tmdb_id <> ''::text));
CREATE INDEX idx_movie_entities_tvdb_id ON movie_entities USING btree (tvdb_id) WHERE ((tvdb_id IS NOT NULL) AND (tvdb_id <> ''::text));
CREATE INDEX idx_movie_entity_metadata_external_ratings_order ON title_metadata_external_ratings USING btree (movie_entity_id, sort_index, source);
CREATE INDEX idx_movie_entity_metadata_external_ratings_source_norm ON title_metadata_external_ratings USING btree (source, normalized, movie_entity_id);
CREATE INDEX idx_movie_entity_metadata_rating_sources_order ON title_metadata_rating_sources USING btree (movie_entity_id, sort_index, source);
CREATE UNIQUE INDEX idx_notification_channels_name_type ON notification_channels USING btree (name, channel_type);
CREATE INDEX idx_notification_subscriptions_channel ON notification_subscriptions USING btree (channel_id) WHERE (channel_id IS NOT NULL);
CREATE INDEX idx_notification_subscriptions_target ON notification_subscriptions USING btree (target_kind, target_id);
CREATE UNIQUE INDEX idx_notification_subscriptions_target_scope ON notification_subscriptions USING btree (target_kind, target_id, event_type, COALESCE(scope, ''::text), COALESCE(scope_id, ''::text));
CREATE INDEX idx_oauth_authorization_codes_expires_at ON oauth_authorization_codes USING btree (expires_at);
CREATE INDEX idx_oauth_authorization_codes_user_id ON oauth_authorization_codes USING btree (user_id);
CREATE INDEX idx_oauth_client_registrations_enabled ON oauth_client_registrations USING btree (enabled);
CREATE INDEX idx_oauth_refresh_grants_authorization_source ON oauth_refresh_grants USING btree (authorization_source);
CREATE INDEX idx_oauth_refresh_grants_family_id ON oauth_refresh_grants USING btree (family_id);
CREATE INDEX idx_oauth_refresh_grants_user_id ON oauth_refresh_grants USING btree (user_id);
CREATE INDEX idx_oauth_refresh_tokens_family_id ON oauth_refresh_tokens USING btree (family_id);
CREATE INDEX idx_oauth_refresh_tokens_grant_id ON oauth_refresh_tokens USING btree (grant_id);
CREATE INDEX idx_pending_releases_active_coverage ON pending_releases USING btree (coverage_identity, status, published_at, added_at);
CREATE UNIQUE INDEX idx_pending_releases_active_release_identity ON pending_releases USING btree (release_identity) WHERE (status = ANY (ARRAY['waiting'::text, 'standby'::text, 'processing'::text, 'needs_review'::text]));
CREATE INDEX idx_pending_releases_active_unknown_age ON pending_releases USING btree (release_age_unknown, status, indexer_id, added_at) WHERE release_age_unknown;
CREATE INDEX idx_pending_releases_indexer_id ON pending_releases USING btree (indexer_id);
CREATE INDEX idx_pending_releases_status ON pending_releases USING btree (status);
CREATE INDEX idx_pending_releases_title_added ON pending_releases USING btree (title_id, added_at DESC);
CREATE INDEX idx_pending_releases_wanted ON pending_releases USING btree (wanted_item_id, status);
CREATE INDEX idx_plugin_catalog_sources_kind ON plugin_catalog_sources USING btree (source_kind);
CREATE INDEX idx_pp_script_runs_script_id ON post_processing_script_runs USING btree (script_id, started_at DESC);
CREATE INDEX idx_pp_script_runs_title_id ON post_processing_script_runs USING btree (title_id, started_at DESC);
CREATE INDEX idx_proxy_configs_provider_type ON proxy_configs USING btree (provider_type);
CREATE INDEX idx_quality_profile_audio_codec_allowlist_profile ON quality_profile_audio_codec_allowlist USING btree (profile_id);
CREATE INDEX idx_quality_profile_audio_codec_blocklist_profile ON quality_profile_audio_codec_blocklist USING btree (profile_id);
CREATE INDEX idx_quality_profile_quality_tiers_profile ON quality_profile_quality_tiers USING btree (profile_id, sort_order);
CREATE INDEX idx_quality_profile_source_allowlist_profile ON quality_profile_source_allowlist USING btree (profile_id);
CREATE INDEX idx_quality_profile_source_blocklist_profile ON quality_profile_source_blocklist USING btree (profile_id);
CREATE INDEX idx_quality_profile_video_codec_allowlist_profile ON quality_profile_video_codec_allowlist USING btree (profile_id);
CREATE INDEX idx_quality_profile_video_codec_blocklist_profile ON quality_profile_video_codec_blocklist USING btree (profile_id);
CREATE INDEX idx_quality_profiles_scope ON quality_profiles USING btree (scope, scope_id);
CREATE INDEX idx_release_decisions_created_at ON release_decisions USING btree (created_at DESC);
CREATE INDEX idx_release_decisions_title_created_at ON release_decisions USING btree (title_id, created_at);
CREATE INDEX idx_release_decisions_wanted ON release_decisions USING btree (wanted_item_id, created_at DESC);
CREATE INDEX idx_release_download_attempts_outcome_attempted ON release_download_attempts USING btree (outcome, attempted_at DESC);
CREATE INDEX idx_release_download_attempts_source_hint ON release_download_attempts USING btree (source_hint);
CREATE INDEX idx_release_download_attempts_source_title ON release_download_attempts USING btree (source_title);
CREATE INDEX idx_release_download_attempts_title ON release_download_attempts USING btree (title_id);
CREATE INDEX idx_request_rule_decisions_evaluated ON request_rule_decisions USING btree (evaluated_at DESC, id DESC);
CREATE INDEX idx_request_rule_decisions_outcome_evaluated ON request_rule_decisions USING btree (effective_outcome, evaluated_at DESC, id DESC);
CREATE INDEX idx_request_rule_decisions_request ON request_rule_decisions USING btree (request_id, evaluated_at DESC);
CREATE INDEX idx_request_rule_revisions_rule_set ON request_rule_revisions USING btree (rule_set_id, revision_number DESC);
CREATE INDEX idx_request_rule_set_libraries_library ON request_rule_set_libraries USING btree (library_id);
CREATE INDEX idx_rule_pack_members_pack_id ON rule_pack_members USING btree (pack_id);
CREATE INDEX idx_rule_set_history_created_at ON rule_set_history USING btree (created_at DESC);
CREATE UNIQUE INDEX idx_rule_sets_managed_key ON rule_sets USING btree (managed_key) WHERE (managed_key IS NOT NULL);
CREATE INDEX idx_scope_indexer_coverage_indexer ON scope_indexer_coverage USING btree (indexer_id);
CREATE INDEX idx_scope_indexer_coverage_searched_at ON scope_indexer_coverage USING btree (searched_at);
CREATE UNIQUE INDEX idx_seeding_profiles_name ON seeding_profiles USING btree (lower(name));
CREATE UNIQUE INDEX idx_series_movie_links_legacy_collection ON series_movie_links USING btree (legacy_collection_id) WHERE (legacy_collection_id IS NOT NULL);
CREATE INDEX idx_series_movie_links_linked_episode ON series_movie_links USING btree (linked_episode_id);
CREATE INDEX idx_series_movie_links_movie ON series_movie_links USING btree (movie_entity_id);
CREATE INDEX idx_series_movie_links_title ON series_movie_links USING btree (series_title_id);
CREATE UNIQUE INDEX idx_setting_values_scope_name ON settings_values USING btree (setting_definition_id, scope, COALESCE(scope_id, ''::text));
CREATE INDEX idx_settings_values_definition ON settings_values USING btree (setting_definition_id);
CREATE INDEX idx_subtitle_downloads_language ON subtitle_downloads USING btree (language);
CREATE INDEX idx_subtitle_downloads_media_file ON subtitle_downloads USING btree (media_file_id);
CREATE INDEX idx_subtitle_downloads_title ON subtitle_downloads USING btree (title_id);
CREATE INDEX idx_subtitle_provider_configs_disabled_until ON subtitle_provider_configs USING btree (disabled_until);
CREATE INDEX idx_subtitle_provider_configs_enabled ON subtitle_provider_configs USING btree (is_enabled);
CREATE INDEX idx_subtitle_provider_configs_provider_type ON subtitle_provider_configs USING btree (provider_type);
CREATE INDEX idx_title_credits_movie_kind ON title_credits USING btree (movie_entity_id, kind);
CREATE UNIQUE INDEX idx_title_credits_movie_owner ON title_credits USING btree (movie_entity_id, "position") WHERE (movie_entity_id IS NOT NULL);
CREATE INDEX idx_title_credits_title_kind ON title_credits USING btree (title_id, kind);
CREATE UNIQUE INDEX idx_title_credits_title_owner ON title_credits USING btree (title_id, "position") WHERE (title_id IS NOT NULL);
CREATE UNIQUE INDEX idx_title_external_ids_library_kind_lookup ON title_external_ids USING btree (library_id, source, external_kind, external_id);
CREATE INDEX idx_title_external_ids_library_source_value ON title_external_ids USING btree (library_id, source, external_id);
CREATE INDEX idx_title_external_ids_numeric_suffix ON title_external_ids USING btree ((("substring"(external_id, '([0-9]+)\s*$'::text))::numeric));
CREATE INDEX idx_title_external_ids_source_value_nocase ON title_external_ids USING btree (source, lower(external_id));
CREATE INDEX idx_title_external_ids_title_id ON title_external_ids USING btree (title_id);
CREATE INDEX idx_title_external_ids_value_nocase ON title_external_ids USING btree (lower(TRIM(BOTH FROM external_id)));
CREATE INDEX idx_title_image_variants_blob_digest ON title_image_variants USING btree (blob_digest);
CREATE UNIQUE INDEX idx_title_metadata_external_ratings_movie_owner ON title_metadata_external_ratings USING btree (movie_entity_id, source) WHERE (movie_entity_id IS NOT NULL);
CREATE INDEX idx_title_metadata_external_ratings_order ON title_metadata_external_ratings USING btree (title_id, sort_index, source);
CREATE INDEX idx_title_metadata_external_ratings_source_norm ON title_metadata_external_ratings USING btree (source, normalized, title_id);
CREATE UNIQUE INDEX idx_title_metadata_external_ratings_title_owner ON title_metadata_external_ratings USING btree (title_id, source) WHERE (title_id IS NOT NULL);
CREATE UNIQUE INDEX idx_title_metadata_rating_sources_movie_owner ON title_metadata_rating_sources USING btree (movie_entity_id, source) WHERE (movie_entity_id IS NOT NULL);
CREATE INDEX idx_title_metadata_rating_sources_order ON title_metadata_rating_sources USING btree (title_id, sort_index, source);
CREATE UNIQUE INDEX idx_title_metadata_rating_sources_title_owner ON title_metadata_rating_sources USING btree (title_id, source) WHERE (title_id IS NOT NULL);
CREATE UNIQUE INDEX idx_title_metadata_rating_summaries_movie_owner ON title_metadata_rating_summaries USING btree (movie_entity_id) WHERE (movie_entity_id IS NOT NULL);
CREATE UNIQUE INDEX idx_title_metadata_rating_summaries_title_owner ON title_metadata_rating_summaries USING btree (title_id) WHERE (title_id IS NOT NULL);
CREATE INDEX idx_title_metadata_tags_category_name ON title_metadata_tags USING btree (category, name, title_id);
CREATE INDEX idx_title_more_like_this_items_source_order ON title_more_like_this_items USING btree (source_title_id, sort_index, rank_score DESC);
CREATE INDEX idx_title_more_like_this_items_title ON title_more_like_this_items USING btree (discovery_title_id);
CREATE INDEX idx_title_recycle_purges_recorded_at ON title_recycle_purges USING btree (recorded_at, title_id);
CREATE INDEX idx_title_search_collation_keys_profile_key ON title_search_collation_keys USING btree (profile, collation_key);
CREATE INDEX idx_title_search_index_queue_title_id ON title_search_index_queue USING btree (title_id);
CREATE INDEX idx_title_search_terms_bucket ON title_search_terms USING btree (facet, script, numbers_key, char_length);
CREATE INDEX idx_title_search_terms_facet_literal ON title_search_terms USING btree (facet, literal_term);
CREATE INDEX idx_title_search_terms_facet_match_term ON title_search_terms USING btree (facet, match_term);
CREATE INDEX idx_title_search_terms_facet_normalized_term ON title_search_terms USING btree (facet, normalized_term);
CREATE INDEX idx_title_search_terms_facet_romanization ON title_search_terms USING btree (facet, romanization_key);
CREATE INDEX idx_title_search_terms_kind_literal ON title_search_terms USING btree (term_kind, literal_term);
CREATE INDEX idx_title_search_terms_normalized_term ON title_search_terms USING btree (normalized_term);
CREATE INDEX idx_title_search_terms_stripped_year_key ON title_search_terms USING btree (term_kind, stripped_year_key);
CREATE UNIQUE INDEX idx_title_search_terms_title_kind_language ON title_search_terms USING btree (title_id, term_kind, literal_term, COALESCE(language_tag, ''::text));
CREATE UNIQUE INDEX idx_title_tag_definitions_label ON title_tag_definitions USING btree (label);
CREATE INDEX idx_titles_catalog_order_asc ON titles USING btree (catalog_sort_key COLLATE "C", catalog_name_tie_key COLLATE "C", (
CASE
    WHEN (year IS NULL) THEN 1
    ELSE 0
END), year, id COLLATE "C");
CREATE INDEX idx_titles_catalog_order_desc ON titles USING btree (catalog_sort_key COLLATE "C" DESC, catalog_name_tie_key COLLATE "C" DESC, (
CASE
    WHEN (year IS NULL) THEN 1
    ELSE 0
END), year DESC, id COLLATE "C" DESC);
CREATE INDEX idx_titles_created_at ON titles USING btree (created_at);
CREATE INDEX idx_titles_facet_monitored ON titles USING btree (facet, monitored);
CREATE INDEX idx_titles_facet_normalized_slug ON titles USING btree (facet, lower(TRIM(BOTH FROM slug))) WHERE ((slug IS NOT NULL) AND (TRIM(BOTH FROM slug) <> ''::text));
CREATE INDEX idx_titles_imdb_id_nocase ON titles USING btree (lower(TRIM(BOTH FROM imdb_id))) WHERE (imdb_id IS NOT NULL);
CREATE INDEX idx_titles_imdb_id_numeric_suffix ON titles USING btree ((("substring"(imdb_id, '([0-9]+)\s*$'::text))::numeric)) WHERE (imdb_id IS NOT NULL);
CREATE INDEX idx_titles_library_catalog_order_asc ON titles USING btree (library_id, catalog_sort_key COLLATE "C", catalog_name_tie_key COLLATE "C", (
CASE
    WHEN (year IS NULL) THEN 1
    ELSE 0
END), year, id COLLATE "C");
CREATE INDEX idx_titles_library_catalog_order_desc ON titles USING btree (library_id, catalog_sort_key COLLATE "C" DESC, catalog_name_tie_key COLLATE "C" DESC, (
CASE
    WHEN (year IS NULL) THEN 1
    ELSE 0
END), year DESC, id COLLATE "C" DESC);
CREATE INDEX idx_titles_library_folder_path ON titles USING btree (library_id, folder_path);
CREATE INDEX idx_titles_library_folder_path_lookup ON titles USING btree (library_id, lower(replace(folder_path, '/'::text, '\'::text)));
CREATE INDEX idx_titles_library_name ON titles USING btree (library_id, lower(name), id);
CREATE INDEX idx_titles_metadata_hydration_due ON titles USING btree (metadata_hydration_next_attempt_at, metadata_fetched_at);
CREATE INDEX idx_titles_movie_smg_identity_backfill_candidates ON titles USING btree (facet, smg_identity_backfill_attempt_count, id);
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
CREATE UNIQUE INDEX idx_user_list_accounts_identity ON user_list_accounts USING btree (user_id, provider, external_user_id);
CREATE INDEX idx_user_ui_table_columns_user_view ON user_ui_table_columns USING btree (user_id, device_class, facet, table_view_mode, column_order);
CREATE INDEX idx_wanted_items_collection ON wanted_items USING btree (collection_id);
CREATE UNIQUE INDEX idx_wanted_items_collection_id ON wanted_items USING btree (collection_id) WHERE ((collection_id IS NOT NULL) AND (episode_id IS NULL));
CREATE INDEX idx_wanted_items_episode ON wanted_items USING btree (episode_id);
CREATE UNIQUE INDEX idx_wanted_items_movie_unique ON wanted_items USING btree (title_id) WHERE ((episode_id IS NULL) AND (collection_id IS NULL) AND (series_movie_link_id IS NULL));
CREATE UNIQUE INDEX idx_wanted_items_series_movie_link ON wanted_items USING btree (series_movie_link_id) WHERE (series_movie_link_id IS NOT NULL);
CREATE INDEX idx_wanted_items_status_updated_at ON wanted_items USING btree (status, updated_at);
CREATE INDEX idx_webauthn_challenges_expires_at ON webauthn_challenges USING btree (expires_at);
CREATE INDEX idx_webauthn_challenges_login_verification ON webauthn_challenges USING btree (login_verification_challenge_id);
CREATE INDEX idx_webauthn_challenges_user_id ON webauthn_challenges USING btree (user_id);
CREATE INDEX idx_webauthn_credentials_user_id_created_at ON webauthn_credentials USING btree (user_id, created_at DESC);
CREATE INDEX idx_workflow_operations_active_job_started ON workflow_operations USING btree (started_at) WHERE ((job_key IS NOT NULL) AND (status = ANY (ARRAY['queued'::text, 'running'::text, 'discovering'::text])));
CREATE INDEX idx_workflow_operations_actor_job_started ON workflow_operations USING btree (actor_user_id, job_key, started_at DESC) WHERE ((job_key IS NOT NULL) AND (actor_user_id IS NOT NULL));
CREATE INDEX idx_workflow_operations_actor_recent_started ON workflow_operations USING btree (actor_user_id, started_at DESC) WHERE ((job_key IS NOT NULL) AND (actor_user_id IS NOT NULL));
CREATE INDEX idx_workflow_operations_collection ON workflow_operations USING btree (collection_id);
CREATE INDEX idx_workflow_operations_episode ON workflow_operations USING btree (episode_id);
CREATE INDEX idx_workflow_operations_job_key_started ON workflow_operations USING btree (job_key, started_at DESC);
CREATE INDEX idx_workflow_operations_job_operation_started ON workflow_operations USING btree (job_key, operation_type, started_at DESC);
CREATE INDEX idx_workflow_operations_job_recent_started ON workflow_operations USING btree (started_at DESC) WHERE (job_key IS NOT NULL);
CREATE INDEX idx_workflow_operations_media_file ON workflow_operations USING btree (media_file_id);
CREATE INDEX idx_workflow_operations_status_started ON workflow_operations USING btree (status, started_at);
CREATE INDEX idx_workflow_operations_title ON workflow_operations USING btree (title_id);
CREATE INDEX indexer_api_client_grants_source ON indexer_api_client_grants USING btree (indexer_id);
CREATE UNIQUE INDEX titles_folder_path_key_unique ON titles USING btree (folder_path_key) WHERE (folder_path_key IS NOT NULL);
CREATE UNIQUE INDEX totp_enrollment_challenges_one_active_per_user ON totp_enrollment_challenges USING btree (user_id);
CREATE TRIGGER discovery_submitted_subjects_delete_bump_affinity_revision AFTER DELETE ON discovery_submitted_subjects FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER discovery_submitted_subjects_insert_bump_affinity_revision AFTER INSERT ON discovery_submitted_subjects FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER discovery_submitted_subjects_update_bump_affinity_revision AFTER UPDATE ON discovery_submitted_subjects FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER title_metadata_tag_source_keys_delete_bump_affinity_revision AFTER DELETE ON title_metadata_tag_source_keys FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER title_metadata_tag_source_keys_insert_bump_affinity_revision AFTER INSERT ON title_metadata_tag_source_keys FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER title_metadata_tag_source_keys_update_bump_affinity_revision AFTER UPDATE ON title_metadata_tag_source_keys FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER title_metadata_tag_sources_delete_bump_affinity_revision AFTER DELETE ON title_metadata_tag_sources FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER title_metadata_tag_sources_insert_bump_affinity_revision AFTER INSERT ON title_metadata_tag_sources FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER title_metadata_tag_sources_update_bump_affinity_revision AFTER UPDATE ON title_metadata_tag_sources FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER title_metadata_tags_delete_bump_affinity_revision AFTER DELETE ON title_metadata_tags FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER title_metadata_tags_insert_bump_affinity_revision AFTER INSERT ON title_metadata_tags FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER title_metadata_tags_update_bump_affinity_revision AFTER UPDATE ON title_metadata_tags FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER titles_delete_bump_affinity_revision AFTER DELETE ON titles FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER titles_delete_enqueue_fuzzy_index AFTER DELETE ON titles FOR EACH ROW EXECUTE FUNCTION title_search_enqueue_deleted_title();
CREATE TRIGGER titles_insert_bump_affinity_revision AFTER INSERT ON titles FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
CREATE TRIGGER titles_update_bump_affinity_revision AFTER UPDATE OF library_id, facet ON titles FOR EACH STATEMENT EXECUTE FUNCTION discovery_affinity_bump_revision();
ALTER TABLE ONLY api_keys
    ADD CONSTRAINT api_keys_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY blocklist
    ADD CONSTRAINT blocklist_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY collection_external_ids
    ADD CONSTRAINT collection_external_ids_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE;
ALTER TABLE ONLY collection_external_ids
    ADD CONSTRAINT collection_external_ids_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY collections
    ADD CONSTRAINT collections_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY delay_profile_facets
    ADD CONSTRAINT delay_profile_facets_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES delay_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY delay_profile_libraries
    ADD CONSTRAINT delay_profile_libraries_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE;
ALTER TABLE ONLY delay_profile_libraries
    ADD CONSTRAINT delay_profile_libraries_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES delay_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY delay_profile_tags
    ADD CONSTRAINT delay_profile_tags_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES delay_profiles(id) ON DELETE CASCADE;
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
ALTER TABLE ONLY discovery_library_graph_payloads
    ADD CONSTRAINT discovery_library_graph_payloads_run_id_fkey FOREIGN KEY (run_id) REFERENCES discovery_library_graph_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY discovery_pending_context_changes
    ADD CONSTRAINT discovery_pending_context_changes_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
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
ALTER TABLE ONLY download_cleanup
    ADD CONSTRAINT download_cleanup_download_id_fkey FOREIGN KEY (download_id) REFERENCES downloads(id) ON DELETE CASCADE;
ALTER TABLE ONLY download_client_bindings
    ADD CONSTRAINT download_client_bindings_download_id_fkey FOREIGN KEY (download_id) REFERENCES downloads(id);
ALTER TABLE ONLY download_client_status
    ADD CONSTRAINT download_client_status_client_config_id_fkey FOREIGN KEY (client_config_id) REFERENCES download_clients(id) ON DELETE CASCADE;
ALTER TABLE ONLY download_identity_states
    ADD CONSTRAINT download_identity_states_canonical_download_id_fkey FOREIGN KEY (canonical_download_id) REFERENCES downloads(id);
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_canonical_download_id_fkey FOREIGN KEY (canonical_download_id) REFERENCES downloads(id);
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_import_id_fkey FOREIGN KEY (import_id) REFERENCES imports(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_imported_media_file_id_fkey FOREIGN KEY (imported_media_file_id) REFERENCES media_files(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_queue_commands
    ADD CONSTRAINT download_queue_commands_canonical_download_id_fkey FOREIGN KEY (canonical_download_id) REFERENCES downloads(id);
ALTER TABLE ONLY download_submission_episode_links
    ADD CONSTRAINT download_submission_episode_links_0180_download_id_fkey FOREIGN KEY (download_id) REFERENCES download_submissions(id) ON DELETE CASCADE;
ALTER TABLE ONLY download_submissions
    ADD CONSTRAINT download_submissions_id_fkey FOREIGN KEY (id) REFERENCES downloads(id);
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
ALTER TABLE ONLY image_proxy_cache_entries
    ADD CONSTRAINT image_proxy_cache_entries_token_fkey FOREIGN KEY (token) REFERENCES image_proxy_sources(token) ON DELETE CASCADE;
ALTER TABLE ONLY imports
    ADD CONSTRAINT imports_canonical_download_id_fkey FOREIGN KEY (canonical_download_id) REFERENCES downloads(id);
ALTER TABLE ONLY indexer_api_client_grants
    ADD CONSTRAINT indexer_api_client_grants_client_id_fkey FOREIGN KEY (client_id) REFERENCES indexer_api_clients(id) ON DELETE CASCADE;
ALTER TABLE ONLY indexer_api_client_grants
    ADD CONSTRAINT indexer_api_client_grants_indexer_id_fkey FOREIGN KEY (indexer_id) REFERENCES indexers(id) ON DELETE CASCADE;
ALTER TABLE ONLY indexer_errors
    ADD CONSTRAINT indexer_errors_indexer_id_fkey FOREIGN KEY (indexer_id) REFERENCES indexers(id) ON DELETE CASCADE;
ALTER TABLE ONLY indexer_search_candidate_source_values
    ADD CONSTRAINT indexer_search_candidate_source_values_source_id_fkey FOREIGN KEY (source_id) REFERENCES indexer_search_candidate_sources(id) ON DELETE CASCADE;
ALTER TABLE ONLY indexer_search_candidate_sources
    ADD CONSTRAINT indexer_search_candidate_sources_candidate_id_fkey FOREIGN KEY (candidate_id) REFERENCES indexer_search_candidates(id) ON DELETE CASCADE;
ALTER TABLE ONLY indexer_search_run_candidate_sources
    ADD CONSTRAINT indexer_search_run_candidate_sources_run_id_fkey FOREIGN KEY (run_id) REFERENCES indexer_search_runs(id) ON DELETE CASCADE;
ALTER TABLE ONLY indexer_search_run_candidate_sources
    ADD CONSTRAINT indexer_search_run_candidate_sources_source_id_fkey FOREIGN KEY (source_id) REFERENCES indexer_search_candidate_sources(id) ON DELETE CASCADE;
ALTER TABLE ONLY indexer_system_backoffs
    ADD CONSTRAINT indexer_system_backoffs_indexer_id_fkey FOREIGN KEY (indexer_id) REFERENCES indexers(id) ON DELETE CASCADE;
ALTER TABLE ONLY indexers
    ADD CONSTRAINT indexers_download_client_id_fkey FOREIGN KEY (download_client_id) REFERENCES download_clients(id) ON DELETE SET NULL;
ALTER TABLE ONLY indexers
    ADD CONSTRAINT indexers_indexer_proxy_config_id_fkey FOREIGN KEY (proxy_config_id) REFERENCES proxy_configs(id) ON DELETE SET NULL;
ALTER TABLE ONLY jellyfin_media_server_details
    ADD CONSTRAINT jellyfin_media_server_details_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY library_probe_signatures
    ADD CONSTRAINT library_probe_signatures_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY library_roots
    ADD CONSTRAINT library_roots_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE;
ALTER TABLE ONLY lifecycle_action_runs
    ADD CONSTRAINT lifecycle_action_runs_candidate_id_fkey FOREIGN KEY (candidate_id) REFERENCES lifecycle_candidates(id) ON DELETE CASCADE;
ALTER TABLE ONLY lifecycle_candidates
    ADD CONSTRAINT lifecycle_candidates_rule_set_id_fkey FOREIGN KEY (rule_set_id) REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE;
ALTER TABLE ONLY list_exclusion_external_ids
    ADD CONSTRAINT list_exclusion_external_ids_exclusion_id_fkey FOREIGN KEY (exclusion_id) REFERENCES list_exclusions(id) ON DELETE CASCADE;
ALTER TABLE ONLY list_exclusions
    ADD CONSTRAINT list_exclusions_created_by_user_id_fkey FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY list_exclusions
    ADD CONSTRAINT list_exclusions_subscription_id_fkey FOREIGN KEY (subscription_id) REFERENCES list_subscriptions(id) ON DELETE CASCADE;
ALTER TABLE ONLY list_memberships
    ADD CONSTRAINT list_memberships_subscription_id_fkey FOREIGN KEY (subscription_id) REFERENCES list_subscriptions(id) ON DELETE CASCADE;
ALTER TABLE ONLY list_subscription_routes
    ADD CONSTRAINT list_subscription_routes_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE;
ALTER TABLE ONLY list_subscription_routes
    ADD CONSTRAINT list_subscription_routes_subscription_id_fkey FOREIGN KEY (subscription_id) REFERENCES list_subscriptions(id) ON DELETE CASCADE;
ALTER TABLE ONLY list_subscriptions
    ADD CONSTRAINT list_subscriptions_credential_id_fkey FOREIGN KEY (credential_id) REFERENCES user_list_accounts(id) ON DELETE SET NULL;
ALTER TABLE ONLY list_subscriptions
    ADD CONSTRAINT list_subscriptions_owner_user_id_fkey FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY list_sync_runs
    ADD CONSTRAINT list_sync_runs_subscription_id_fkey FOREIGN KEY (subscription_id) REFERENCES list_subscriptions(id) ON DELETE CASCADE;
ALTER TABLE ONLY location_file_resolutions
    ADD CONSTRAINT location_file_resolutions_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE;
ALTER TABLE ONLY location_operation_owned_entities
    ADD CONSTRAINT location_operation_owned_entities_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE;
ALTER TABLE ONLY location_operation_plans
    ADD CONSTRAINT location_operation_plans_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE;
ALTER TABLE ONLY location_operation_title_checkpoints
    ADD CONSTRAINT location_operation_title_checkpoints_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE;
ALTER TABLE ONLY location_operation_verifications
    ADD CONSTRAINT location_operation_verifications_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE;
ALTER TABLE ONLY location_operations
    ADD CONSTRAINT location_operations_initiated_by_user_id_fkey FOREIGN KEY (initiated_by_user_id) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY location_recovery_staging
    ADD CONSTRAINT location_recovery_staging_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES location_operations(id);
ALTER TABLE ONLY location_transfer_progress
    ADD CONSTRAINT location_transfer_progress_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE;
ALTER TABLE ONLY location_transfer_titles
    ADD CONSTRAINT location_transfer_titles_operation_id_fkey FOREIGN KEY (operation_id) REFERENCES location_operations(id) ON DELETE CASCADE;
ALTER TABLE ONLY login_verification_challenges
    ADD CONSTRAINT login_verification_challenges_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY maintenance_action_job_receipts
    ADD CONSTRAINT maintenance_action_job_receip_candidate_id_match_generatio_fkey FOREIGN KEY (candidate_id, match_generation, revision_number, step_id) REFERENCES maintenance_action_steps(candidate_id, match_generation, revision_number, step_id) ON DELETE CASCADE;
ALTER TABLE ONLY maintenance_action_step_attempts
    ADD CONSTRAINT maintenance_action_step_attem_candidate_id_match_generatio_fkey FOREIGN KEY (candidate_id, match_generation, revision_number, step_id) REFERENCES maintenance_action_steps(candidate_id, match_generation, revision_number, step_id) ON DELETE CASCADE;
ALTER TABLE ONLY maintenance_action_steps
    ADD CONSTRAINT maintenance_action_steps_candidate_id_fkey FOREIGN KEY (candidate_id) REFERENCES lifecycle_candidates(id) ON DELETE CASCADE;
ALTER TABLE ONLY maintenance_evaluation_runs
    ADD CONSTRAINT maintenance_evaluation_runs_rule_set_id_fkey FOREIGN KEY (rule_set_id) REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE;
ALTER TABLE ONLY maintenance_rule_exclusions
    ADD CONSTRAINT maintenance_rule_exclusions_rule_set_id_fkey FOREIGN KEY (rule_set_id) REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE;
ALTER TABLE ONLY maintenance_rule_revisions
    ADD CONSTRAINT maintenance_rule_revisions_rule_set_id_fkey FOREIGN KEY (rule_set_id) REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE;
ALTER TABLE ONLY maintenance_rule_set_libraries
    ADD CONSTRAINT maintenance_rule_set_libraries_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE RESTRICT;
ALTER TABLE ONLY maintenance_rule_set_libraries
    ADD CONSTRAINT maintenance_rule_set_libraries_rule_set_id_fkey FOREIGN KEY (rule_set_id) REFERENCES maintenance_rule_sets(id) ON DELETE CASCADE;
ALTER TABLE ONLY maintenance_sequence_terminal_memberships
    ADD CONSTRAINT maintenance_sequence_terminal_memberships_candidate_id_fkey FOREIGN KEY (candidate_id) REFERENCES lifecycle_candidates(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_file_analyses
    ADD CONSTRAINT media_file_analyses_media_file_id_fkey FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE CASCADE;
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
ALTER TABLE ONLY media_server_audience_plays
    ADD CONSTRAINT media_server_audience_plays_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_audience_reads
    ADD CONSTRAINT media_server_audience_reads_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_default_library_grants
    ADD CONSTRAINT media_server_default_library_grants_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_default_library_grants
    ADD CONSTRAINT media_server_default_library_grants_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_path_mappings
    ADD CONSTRAINT media_server_path_mappings_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_playback_items
    ADD CONSTRAINT media_server_playback_items_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_signal_roster
    ADD CONSTRAINT media_server_signal_roster_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_signal_sync_state
    ADD CONSTRAINT media_server_signal_sync_state_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_server_user_media_signals
    ADD CONSTRAINT media_server_user_media_signals_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE;
ALTER TABLE ONLY notification_subscriptions
    ADD CONSTRAINT notification_subscriptions_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES notification_channels(id) ON DELETE CASCADE;
ALTER TABLE ONLY oauth_authorization_codes
    ADD CONSTRAINT oauth_authorization_codes_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY oauth_client_redirect_uris
    ADD CONSTRAINT oauth_client_redirect_uris_client_id_fkey FOREIGN KEY (client_id) REFERENCES oauth_client_registrations(client_id) ON DELETE CASCADE;
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
ALTER TABLE ONLY request_rule_revisions
    ADD CONSTRAINT request_rule_revisions_rule_set_id_fkey FOREIGN KEY (rule_set_id) REFERENCES request_rule_sets(id) ON DELETE CASCADE;
ALTER TABLE ONLY request_rule_set_libraries
    ADD CONSTRAINT request_rule_set_libraries_library_id_fkey FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE RESTRICT;
ALTER TABLE ONLY request_rule_set_libraries
    ADD CONSTRAINT request_rule_set_libraries_rule_set_id_fkey FOREIGN KEY (rule_set_id) REFERENCES request_rule_sets(id) ON DELETE CASCADE;
ALTER TABLE ONLY rule_pack_members
    ADD CONSTRAINT rule_pack_members_pack_id_fkey FOREIGN KEY (pack_id) REFERENCES rule_pack_installations(pack_id) ON DELETE CASCADE;
ALTER TABLE ONLY rule_pack_members
    ADD CONSTRAINT rule_pack_members_rule_set_id_fkey FOREIGN KEY (rule_set_id) REFERENCES rule_sets(id) ON DELETE CASCADE;
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
ALTER TABLE ONLY subtitle_search_backoffs
    ADD CONSTRAINT subtitle_search_backoffs_media_file_id_fkey FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_anime_numbering_bridges
    ADD CONSTRAINT title_anime_numbering_bridges_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_credits
    ADD CONSTRAINT title_credits_movie_entity_id_fkey FOREIGN KEY (movie_entity_id) REFERENCES movie_entities(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_credits
    ADD CONSTRAINT title_credits_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_external_ids
    ADD CONSTRAINT title_external_ids_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_image_variants
    ADD CONSTRAINT title_image_variants_blob_digest_fkey FOREIGN KEY (blob_digest) REFERENCES title_image_blobs(digest) ON DELETE RESTRICT;
ALTER TABLE ONLY title_image_variants
    ADD CONSTRAINT title_image_variants_title_image_id_fkey FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_images
    ADD CONSTRAINT title_images_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_external_ratings
    ADD CONSTRAINT title_metadata_external_ratings_movie_entity_id_fkey FOREIGN KEY (movie_entity_id) REFERENCES movie_entities(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_external_ratings
    ADD CONSTRAINT title_metadata_external_ratings_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_rating_sources
    ADD CONSTRAINT title_metadata_rating_sources_movie_entity_id_fkey FOREIGN KEY (movie_entity_id) REFERENCES movie_entities(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_rating_sources
    ADD CONSTRAINT title_metadata_rating_sources_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_rating_summaries
    ADD CONSTRAINT title_metadata_rating_summaries_movie_entity_id_fkey FOREIGN KEY (movie_entity_id) REFERENCES movie_entities(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_rating_summaries
    ADD CONSTRAINT title_metadata_rating_summaries_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_tag_source_keys
    ADD CONSTRAINT title_metadata_tag_source_keys_title_id_tag_key_fkey FOREIGN KEY (title_id, tag_key) REFERENCES title_metadata_tags(title_id, tag_key) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_tag_sources
    ADD CONSTRAINT title_metadata_tag_sources_title_id_tag_key_fkey FOREIGN KEY (title_id, tag_key) REFERENCES title_metadata_tags(title_id, tag_key) ON DELETE CASCADE;
ALTER TABLE ONLY title_metadata_tags
    ADD CONSTRAINT title_metadata_tags_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_more_like_this_items
    ADD CONSTRAINT title_more_like_this_items_recommendation_card_fkey FOREIGN KEY (discovery_title_id) REFERENCES title_recommendation_cards(discovery_title_id) ON DELETE CASCADE;
ALTER TABLE ONLY title_more_like_this_items
    ADD CONSTRAINT title_more_like_this_items_source_title_id_fkey FOREIGN KEY (source_title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_search_collation_keys
    ADD CONSTRAINT title_search_collation_keys_term_id_fkey FOREIGN KEY (term_id) REFERENCES title_search_terms(term_id) ON DELETE CASCADE;
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
ALTER TABLE ONLY user_list_accounts
    ADD CONSTRAINT user_list_accounts_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_list_policies
    ADD CONSTRAINT user_list_policies_updated_by_user_id_fkey FOREIGN KEY (updated_by_user_id) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY user_list_policies
    ADD CONSTRAINT user_list_policies_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_ui_catalog_views
    ADD CONSTRAINT user_ui_catalog_views_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_ui_dashboard_layout_items
    ADD CONSTRAINT user_ui_dashboard_layout_items_user_id_breakpoint_fkey FOREIGN KEY (user_id, breakpoint) REFERENCES user_ui_dashboard_layouts(user_id, breakpoint) ON DELETE CASCADE;
ALTER TABLE ONLY user_ui_dashboard_layouts
    ADD CONSTRAINT user_ui_dashboard_layouts_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_ui_settings
    ADD CONSTRAINT user_ui_settings_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY user_ui_table_columns
    ADD CONSTRAINT user_ui_table_columns_user_id_fkey1 FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id);
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_series_movie_link_id_fkey FOREIGN KEY (series_movie_link_id) REFERENCES series_movie_links(id) ON DELETE SET NULL;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY webauthn_challenges
    ADD CONSTRAINT webauthn_challenges_login_verification_challenge_id_fkey FOREIGN KEY (login_verification_challenge_id) REFERENCES login_verification_challenges(id) ON DELETE CASCADE;
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
INSERT INTO discovery_affinity_revision (id, revision) VALUES (1, md5(random()::text || clock_timestamp()::text));
INSERT INTO import_space_incident_lock (id, revision) VALUES (1, 0);
INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at) VALUES ('anime_default_library', 'anime', 'Anime', 'anime', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at) VALUES ('movie_default_library', 'movie', 'Movies', 'movies', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at) VALUES ('series_default_library', 'series', 'Series', 'series', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO library_root_id_remaps (legacy_root_id, root_id, normalized_path, remapped, created_at) VALUES ('3e1fce99b6700cecffbea040ff14843bf59d0f63799a4be8f75956efa1393b0e', 'canonical_root_for_series_default_library', '/data/series', false, '1970-01-01T00:00:00Z');
INSERT INTO library_root_id_remaps (legacy_root_id, root_id, normalized_path, remapped, created_at) VALUES ('8277630ec2b8f510eb797f12f9b66bec03e5c9a395982fb220034a05aa044637', 'canonical_root_for_anime_default_library', '/data/anime', false, '1970-01-01T00:00:00Z');
INSERT INTO library_root_id_remaps (legacy_root_id, root_id, normalized_path, remapped, created_at) VALUES ('8312cd866e8499fd5a7a90f518f5d121e7d8a90ffb285800f60eeb323506c456', 'canonical_root_for_movie_default_library', '/data/movies', false, '1970-01-01T00:00:00Z');
INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at, legacy_path_derived_id) VALUES ('canonical_root_for_anime_default_library', 'anime_default_library', '/data/anime', '/data/anime', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z', '8277630ec2b8f510eb797f12f9b66bec03e5c9a395982fb220034a05aa044637');
INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at, legacy_path_derived_id) VALUES ('canonical_root_for_movie_default_library', 'movie_default_library', '/data/movies', '/data/movies', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z', '8312cd866e8499fd5a7a90f518f5d121e7d8a90ffb285800f60eeb323506c456');
INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at, legacy_path_derived_id) VALUES ('canonical_root_for_series_default_library', 'series_default_library', '/data/series', '/data/series', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z', '3e1fce99b6700cecffbea040ff14843bf59d0f63799a4be8f75956efa1393b0e');
INSERT INTO location_transfer_runtime (id, generation) VALUES (1, 0);
INSERT INTO quality_profiles (id, name, scope, scope_id, archival_quality, allow_unknown_quality, atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux, allow_bd_disk, allow_upgrades, created_at, prefer_dual_audio, required_audio_languages, scoring_config) VALUES ('1080p', '1080P', 'system', NULL, '1080P', false, true, true, true, true, false, true, '1970-01-01T00:00:00Z', false, '[]', '{}');
INSERT INTO quality_profiles (id, name, scope, scope_id, archival_quality, allow_unknown_quality, atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux, allow_bd_disk, allow_upgrades, created_at, prefer_dual_audio, required_audio_languages, scoring_config) VALUES ('4k', '4K', 'system', NULL, '2160P', false, true, true, true, true, false, true, '1970-01-01T00:00:00Z', false, '[]', '{}');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('1080p', '1080P', 0, '1970-01-01T00:00:00Z');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('1080p', '720P', 1, '1970-01-01T00:00:00Z');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('4k', '1080P', 1, '1970-01-01T00:00:00Z');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('4k', '2160P', 0, '1970-01-01T00:00:00Z');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('4k', '720P', 2, '1970-01-01T00:00:00Z');
INSERT INTO settings_definitions (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at) VALUES ('media:system:movie.availability_delay_days', 'media', 'system', 'movie.availability_delay_days', 'number', '0', false, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO settings_definitions (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at) VALUES ('media:system:movie.default_availability', 'media', 'system', 'movie.default_availability', 'string', '"released"', false, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO settings_definitions (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at) VALUES ('media:system:movie.release_market', 'media', 'system', 'movie.release_market', 'string', '"US"', false, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO settings_definitions (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at) VALUES ('title-tags-pending-deletions', 'system', 'system', 'title_tags.pending_deletions', 'json', '{}', false, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO settings_definitions (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at) VALUES ('title-tags-pending-renames', 'system', 'system', 'title_tags.pending_renames', 'json', '{}', false, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO title_search_meta (id, collation_version, projection_generation) VALUES (1, '', 0);
INSERT INTO users (id, username, password_hash, display_name, status, passkey_public_key, locale, created_at, updated_at, last_login_at, account_kind, auth_session_version, password_change_required) VALUES ('00000000000000000000000000000001', 'admin', NULL, NULL, 'active', NULL, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z', NULL, 'local', NULL, false);
