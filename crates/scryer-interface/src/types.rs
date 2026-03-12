use async_graphql::{InputObject, SimpleObject};

#[derive(InputObject)]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

#[derive(SimpleObject, Clone)]
pub struct LoginPayload {
    pub token: String,
    pub user: UserPayload,
    pub expires_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalIdPayload {
    pub source: String,
    pub value: String,
}

#[derive(SimpleObject, Clone)]
pub struct TitlePayload {
    pub id: String,
    pub name: String,
    pub facet: String,
    pub monitored: bool,
    pub tags: Vec<String>,
    pub external_ids: Vec<ExternalIdPayload>,
    pub created_by: Option<String>,
    pub created_at: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub imdb_id: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub genres: Vec<String>,
    pub content_status: Option<String>,
    pub language: Option<String>,
    pub first_aired: Option<String>,
    pub network: Option<String>,
    pub studio: Option<String>,
    pub country: Option<String>,
    pub aliases: Vec<String>,
    pub metadata_language: Option<String>,
    pub metadata_fetched_at: Option<String>,
    pub min_availability: Option<String>,
    pub digital_release_date: Option<String>,
    /// Primary collection label (quality tier), populated in list queries.
    pub quality_tier: Option<String>,
    /// Aggregated media-file size in bytes for the title, populated in list queries.
    pub size_bytes: Option<i64>,
}

#[derive(SimpleObject, Clone)]
pub struct InterstitialMovieMetadataPayload {
    pub tvdb_id: String,
    pub name: String,
    pub slug: String,
    pub year: Option<i32>,
    pub content_status: String,
    pub overview: String,
    pub poster_url: String,
    pub language: String,
    pub runtime_minutes: i32,
    pub sort_title: String,
    pub imdb_id: String,
    pub genres: Vec<String>,
    pub studio: String,
    pub digital_release_date: Option<String>,
    pub association_confidence: Option<String>,
    pub continuity_status: Option<String>,
    pub movie_form: Option<String>,
    pub confidence: Option<String>,
    pub signal_summary: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct CollectionPayload {
    pub id: String,
    pub title_id: String,
    pub collection_type: String,
    pub collection_index: String,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
    pub narrative_order: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub first_episode_number: Option<String>,
    pub last_episode_number: Option<String>,
    pub interstitial_movie: Option<InterstitialMovieMetadataPayload>,
    pub specials_movies: Vec<InterstitialMovieMetadataPayload>,
    pub monitored: bool,
    pub created_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct SetCollectionMonitoredPayload {
    pub id: String,
    pub monitored: bool,
    pub episodes: Vec<EpisodePayload>,
}

#[derive(SimpleObject, Clone)]
pub struct EpisodePayload {
    pub id: String,
    pub title_id: String,
    pub collection_id: Option<String>,
    pub episode_type: String,
    pub episode_number: Option<String>,
    pub season_number: Option<String>,
    pub episode_label: Option<String>,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub duration_seconds: Option<i64>,
    pub has_multi_audio: bool,
    pub has_subtitle: bool,
    pub is_filler: bool,
    pub is_recap: bool,
    pub absolute_number: Option<String>,
    pub monitored: bool,
    pub created_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct AudioStreamDetailPayload {
    pub codec: Option<String>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub bitrate_kbps: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct SubtitleStreamDetailPayload {
    pub codec: Option<String>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub forced: bool,
    pub default: bool,
}

#[derive(SimpleObject, Clone)]
pub struct TitleMediaFilePayload {
    pub id: String,
    pub title_id: String,
    pub episode_id: Option<String>,
    pub file_path: String,
    pub size_bytes: String,
    pub quality_label: Option<String>,
    pub scan_status: String,
    pub created_at: String,
    // Media analysis (populated after media scan; null until scan_status = "scanned")
    pub video_codec: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    pub video_hdr_format: Option<String>,
    pub video_frame_rate: Option<String>,
    pub video_profile: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<i32>,
    pub audio_bitrate_kbps: Option<i32>,
    pub audio_languages: Vec<String>,
    pub audio_streams: Vec<AudioStreamDetailPayload>,
    pub subtitle_languages: Vec<String>,
    pub subtitle_codecs: Vec<String>,
    pub subtitle_streams: Vec<SubtitleStreamDetailPayload>,
    pub has_multiaudio: bool,
    pub duration_seconds: Option<i32>,
    pub num_chapters: Option<i32>,
    pub container_format: Option<String>,
    // Rich metadata (populated at import from parsed release name)
    pub scene_name: Option<String>,
    pub release_group: Option<String>,
    pub source_type: Option<String>,
    pub resolution: Option<String>,
    pub video_codec_parsed: Option<String>,
    pub audio_codec_parsed: Option<String>,
    pub acquisition_score: Option<i32>,
    pub scoring_log: Option<String>,
    pub indexer_source: Option<String>,
    pub grabbed_release_title: Option<String>,
    pub grabbed_at: Option<String>,
    pub edition: Option<String>,
    pub original_file_path: Option<String>,
    pub release_hash: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct DiskSpacePayload {
    pub path: String,
    pub label: String,
    pub total_bytes: String,
    pub free_bytes: String,
    pub used_bytes: String,
}

#[derive(SimpleObject, Clone)]
pub struct SystemHealthPayload {
    pub service_ready: bool,
    pub db_path: String,
    pub total_titles: i32,
    pub monitored_titles: i32,
    pub total_users: i32,
    pub titles_movie: i32,
    pub titles_tv: i32,
    pub titles_anime: i32,
    pub titles_other: i32,
    pub recent_events: i32,
    pub recent_event_preview: Vec<String>,
    pub db_migration_version: Option<String>,
    pub db_pending_migrations: i32,
    pub smg_cert_expires_at: Option<String>,
    pub smg_cert_days_remaining: Option<i32>,
    pub indexer_stats: Vec<IndexerQueryStatsPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerQueryStatsPayload {
    pub indexer_id: String,
    pub indexer_name: String,
    pub queries_last_24h: i32,
    pub successful_last_24h: i32,
    pub failed_last_24h: i32,
    pub last_query_at: Option<String>,
    pub api_current: Option<i32>,
    pub api_max: Option<i32>,
    pub grab_current: Option<i32>,
    pub grab_max: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct UserPayload {
    pub id: String,
    pub username: String,
    pub entitlements: Vec<String>,
}

#[derive(SimpleObject, Clone)]
pub struct PolicyOutputPayload {
    pub decision: bool,
    pub score: f32,
    pub reason_codes: Vec<String>,
    pub explanation: String,
    pub scoring_log: Vec<ScoringEntryPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct EventPayload {
    pub id: String,
    pub event_type: String,
    pub actor_user_id: Option<String>,
    pub title_id: Option<String>,
    pub message: String,
    pub occurred_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct ActivityEventPayload {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub channels: Vec<String>,
    pub actor_user_id: Option<String>,
    pub title_id: Option<String>,
    pub message: String,
    pub occurred_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct TitleReleaseBlocklistEntryPayload {
    pub source_hint: Option<String>,
    pub source_title: Option<String>,
    pub error_message: Option<String>,
    pub attempted_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerSearchResultPayload {
    pub source: String,
    pub title: String,
    pub link: Option<String>,
    pub download_url: Option<String>,
    pub source_kind: Option<String>,
    pub size_bytes: Option<i64>,
    pub published_at: Option<String>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub parsed_release: Option<ParsedReleasePayload>,
    pub quality_profile_decision: Option<QualityProfileDecisionPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct ParsedEpisodePayload {
    pub season: Option<i32>,
    pub episode_numbers: Vec<i32>,
    pub absolute_episode: Option<i32>,
    pub raw: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ParsedReleasePayload {
    pub raw_title: String,
    pub normalized_title: String,
    pub release_group: Option<String>,
    pub languages_audio: Vec<String>,
    pub languages_subtitles: Vec<String>,
    pub year: Option<i32>,
    pub quality: Option<String>,
    pub source: Option<String>,
    pub video_codec: Option<String>,
    pub video_encoding: Option<String>,
    pub audio: Option<String>,
    pub audio_channels: Option<String>,
    pub is_dual_audio: bool,
    pub is_atmos: bool,
    pub is_dolby_vision: bool,
    pub detected_hdr: bool,
    pub fps: Option<f32>,
    pub is_proper_upload: bool,
    pub is_remux: bool,
    pub is_bd_disk: bool,
    pub is_ai_enhanced: bool,
    pub parser_version: String,
    pub parse_confidence: f32,
    pub missing_fields: Vec<String>,
    pub parse_hints: Vec<String>,
    pub episode: Option<ParsedEpisodePayload>,
}

#[derive(SimpleObject, Clone)]
pub struct ScoringEntryPayload {
    pub code: String,
    pub delta: i32,
    pub source: String,
}

#[derive(SimpleObject, Clone)]
pub struct QualityProfileDecisionPayload {
    pub allowed: bool,
    pub block_codes: Vec<String>,
    pub release_score: i32,
    pub preference_score: i32,
    pub scoring_log: Vec<ScoringEntryPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerConfigPayload {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub has_api_key: bool,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub disabled_until: Option<String>,
    pub is_enabled: bool,
    pub enable_interactive_search: bool,
    pub enable_auto_search: bool,
    pub last_health_status: Option<String>,
    pub last_error_at: Option<String>,
    pub last_query_at: Option<String>,
    pub config_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct DownloadClientConfigPayload {
    pub id: String,
    pub name: String,
    pub client_type: String,
    pub base_url: Option<String>,
    pub config_json: String,
    pub is_enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct DownloadQueueItemPayload {
    pub id: String,
    pub title_id: Option<String>,
    pub title_name: String,
    pub facet: Option<String>,
    pub is_scryer_origin: bool,
    pub client_id: String,
    pub client_name: String,
    pub client_type: String,
    pub state: String,
    pub progress_percent: i32,
    pub size_bytes: Option<String>,
    pub remaining_seconds: Option<i32>,
    pub queued_at: Option<String>,
    pub last_updated_at: Option<String>,
    pub attention_required: bool,
    pub attention_reason: Option<String>,
    pub download_client_item_id: String,
    pub import_status: Option<String>,
    pub import_error_message: Option<String>,
    pub imported_at: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ImportResultPayload {
    pub import_id: String,
    pub decision: String,
    pub skip_reason: Option<String>,
    pub title_id: Option<String>,
    pub source_path: String,
    pub dest_path: Option<String>,
    pub file_size_bytes: Option<String>,
    pub link_type: Option<String>,
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ImportRecordPayload {
    pub id: String,
    pub source_system: String,
    pub source_ref: String,
    pub source_title: Option<String>,
    pub import_type: String,
    pub status: String,
    pub error_message: Option<String>,
    pub decision: Option<String>,
    pub skip_reason: Option<String>,
    pub title_id: Option<String>,
    pub source_path: Option<String>,
    pub dest_path: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
}

#[derive(InputObject)]
pub struct TriggerImportInput {
    pub download_client_item_id: String,
    pub title_id: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct AddTitleResult {
    pub title: TitlePayload,
    pub download_job_id: String,
}

#[derive(SimpleObject, Clone)]
pub struct LibraryScanSummaryPayload {
    pub scanned: i32,
    pub matched: i32,
    pub imported: i32,
    pub skipped: i32,
    pub unmatched: i32,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRenamePlanItemPayload {
    pub collection_id: Option<String>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub normalized_filename: Option<String>,
    pub collision: bool,
    pub reason_code: String,
    pub write_action: String,
    pub source_size_bytes: Option<String>,
    pub source_mtime_unix_ms: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRenamePlanPayload {
    pub facet: String,
    pub title_id: Option<String>,
    pub template: String,
    pub collision_policy: String,
    pub missing_metadata_policy: String,
    pub fingerprint: String,
    pub total: i32,
    pub renamable: i32,
    pub noop: i32,
    pub conflicts: i32,
    pub errors: i32,
    pub items: Vec<MediaRenamePlanItemPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRenameApplyItemPayload {
    pub collection_id: Option<String>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub final_path: Option<String>,
    pub write_action: String,
    pub status: String,
    pub reason_code: String,
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRenameApplyPayload {
    pub plan_fingerprint: String,
    pub total: i32,
    pub applied: i32,
    pub skipped: i32,
    pub failed: i32,
    pub items: Vec<MediaRenameApplyItemPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct AdminSettingsItemPayload {
    pub category: String,
    pub scope: String,
    pub key_name: String,
    pub data_type: String,
    pub default_value_json: String,
    pub effective_value_json: Option<String>,
    pub value_json: Option<String>,
    pub source: Option<String>,
    pub has_override: bool,
    pub is_sensitive: bool,
    pub validation_json: Option<String>,
    pub scope_id: Option<String>,
    pub updated_by_user_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct AdminSettingsPayload {
    pub scope: String,
    pub scope_id: Option<String>,
    pub items: Vec<AdminSettingsItemPayload>,
    pub quality_profiles: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct TvdbScanOperationPayload {
    pub id: String,
    pub operation_type: String,
    pub status: String,
    pub actor_user_id: Option<String>,
    pub limit: i64,
    pub source: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(InputObject, Clone)]
pub struct ExternalIdInput {
    pub source: String,
    pub value: String,
}

#[derive(InputObject, Clone)]
pub struct AddTitleInput {
    pub name: String,
    pub facet: String,
    pub monitored: bool,
    pub tags: Vec<String>,
    pub external_ids: Option<Vec<ExternalIdInput>>,
    pub source_hint: Option<String>,
    pub source_kind: Option<String>,
    pub source_title: Option<String>,
    pub min_availability: Option<String>,
    // Metadata fields the frontend can supply from the search result so the
    // title is created with rich data immediately, without relying on a
    // separate hydration round-trip to the metadata gateway.
    pub poster_url: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub language: Option<String>,
    pub content_status: Option<String>,
}

#[derive(InputObject)]
pub struct PolicyInputPayload {
    pub title_id: String,
    pub facet: String,
    pub has_existing_file: bool,
    pub candidate_quality: Option<String>,
    pub requested_mode: String,
}

#[derive(InputObject)]
pub struct QueueDownloadInput {
    pub title_id: String,
    pub source_hint: Option<String>,
    pub source_kind: Option<String>,
    pub source_title: Option<String>,
}

#[derive(InputObject)]
pub struct QueueManualImportInput {
    pub title_id: Option<String>,
    pub client_type: Option<String>,
    pub download_client_item_id: String,
}

#[derive(InputObject, Clone)]
pub struct MediaRenamePreviewInput {
    pub facet: String,
    pub title_id: Option<String>,
    pub dry_run: Option<bool>,
}

#[derive(InputObject, Clone)]
pub struct MediaRenameApplyInput {
    pub facet: String,
    pub title_id: String,
    pub fingerprint: String,
    pub idempotency_key: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct MediaRenameBulkApplyInput {
    pub facet: String,
    pub fingerprint: String,
    pub idempotency_key: Option<String>,
}

#[derive(InputObject)]
pub struct AdminSettingsUpdateItemInput {
    pub key_name: String,
    pub value: String,
}

#[derive(InputObject)]
pub struct AdminSettingsUpdateInput {
    pub scope: String,
    pub scope_id: Option<String>,
    pub items: Vec<AdminSettingsUpdateItemInput>,
}

#[derive(InputObject)]
pub struct DeleteQualityProfileInput {
    pub profile_id: String,
}

#[derive(InputObject)]
pub struct QueueTvdbMoviesScanInput {
    pub limit: i64,
    pub source: String,
}

#[derive(InputObject)]
pub struct CreateIndexerConfigInput {
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub is_enabled: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_auto_search: Option<bool>,
    pub config_json: Option<String>,
}

#[derive(InputObject)]
pub struct UpdateIndexerConfigInput {
    pub id: String,
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub is_enabled: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_auto_search: Option<bool>,
    pub config_json: Option<String>,
}

#[derive(InputObject)]
pub struct DeleteIndexerConfigInput {
    pub id: String,
}

#[derive(InputObject)]
pub struct CreateDownloadClientConfigInput {
    pub name: String,
    pub client_type: String,
    pub base_url: Option<String>,
    pub config_json: String,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateDownloadClientConfigInput {
    pub id: String,
    pub name: Option<String>,
    pub client_type: Option<String>,
    pub base_url: Option<String>,
    pub config_json: Option<String>,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct DeleteDownloadClientConfigInput {
    pub id: String,
}

#[derive(InputObject)]
pub struct ReorderDownloadClientConfigsInput {
    pub ids: Vec<String>,
}

#[derive(InputObject)]
pub struct TestDownloadClientConnectionInput {
    pub client_type: String,
    pub base_url: String,
    pub config_json: String,
}

#[derive(InputObject)]
pub struct TestIndexerConnectionInput {
    pub provider_type: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub config_json: Option<String>,
}

#[derive(InputObject)]
pub struct DeleteTitleInput {
    pub title_id: String,
    pub delete_files_on_disk: Option<bool>,
}

#[derive(InputObject)]
pub struct CreateUserInput {
    pub username: String,
    pub password: String,
    pub entitlements: Vec<String>,
}

#[derive(InputObject)]
pub struct SetUserPasswordInput {
    pub user_id: String,
    pub password: String,
    pub current_password: Option<String>,
}

#[derive(InputObject)]
pub struct SetTitleMonitoredInput {
    pub title_id: String,
    pub monitored: bool,
}

#[derive(InputObject)]
pub struct UpdateTitleInput {
    pub title_id: String,
    pub name: Option<String>,
    pub facet: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(InputObject)]
pub struct CreateCollectionInput {
    pub title_id: String,
    pub collection_type: String,
    pub collection_index: String,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
    pub first_episode_number: Option<String>,
    pub last_episode_number: Option<String>,
}

#[derive(InputObject)]
pub struct CreateEpisodeInput {
    pub title_id: String,
    pub collection_id: Option<String>,
    pub episode_type: String,
    pub episode_number: Option<String>,
    pub season_number: Option<String>,
    pub episode_label: Option<String>,
    pub title: Option<String>,
    pub air_date: Option<String>,
    pub duration_seconds: Option<i64>,
    pub has_multi_audio: bool,
    pub has_subtitle: bool,
}

#[derive(InputObject)]
pub struct UpdateCollectionInput {
    pub collection_id: String,
    pub collection_type: Option<String>,
    pub collection_index: Option<String>,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
    pub first_episode_number: Option<String>,
    pub last_episode_number: Option<String>,
    pub monitored: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateEpisodeInput {
    pub episode_id: String,
    pub episode_type: Option<String>,
    pub episode_number: Option<String>,
    pub season_number: Option<String>,
    pub episode_label: Option<String>,
    pub title: Option<String>,
    pub air_date: Option<String>,
    pub duration_seconds: Option<i64>,
    pub has_multi_audio: Option<bool>,
    pub has_subtitle: Option<bool>,
    pub monitored: Option<bool>,
    pub collection_id: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct SetCollectionMonitoredInput {
    pub collection_id: String,
    pub monitored: bool,
}

#[derive(InputObject, Clone)]
pub struct SetEpisodeMonitoredInput {
    pub episode_id: String,
    pub monitored: bool,
}

#[derive(InputObject)]
pub struct SetUserEntitlementsInput {
    pub user_id: String,
    pub entitlements: Vec<String>,
}

#[derive(InputObject)]
pub struct DeleteUserInput {
    pub user_id: String,
}

#[derive(InputObject)]
pub struct DeleteCollectionInput {
    pub collection_id: String,
}

#[derive(InputObject)]
pub struct DeleteEpisodeInput {
    pub episode_id: String,
}

#[derive(InputObject)]
pub struct PauseDownloadInput {
    pub download_client_item_id: String,
}

#[derive(InputObject)]
pub struct ResumeDownloadInput {
    pub download_client_item_id: String,
}

#[derive(InputObject)]
pub struct DeleteDownloadInput {
    pub download_client_item_id: String,
    pub is_history: bool,
}

// --- Manual Import ---

#[derive(SimpleObject, Clone)]
pub struct ManualImportFilePreviewPayload {
    pub file_path: String,
    pub file_name: String,
    pub size_bytes: String,
    pub quality: Option<String>,
    pub parsed_season: Option<i32>,
    pub parsed_episodes: Vec<i32>,
    pub suggested_episode_id: Option<String>,
    pub suggested_episode_label: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ManualImportPreviewPayload {
    pub files: Vec<ManualImportFilePreviewPayload>,
    pub available_episodes: Vec<EpisodePayload>,
}

#[derive(InputObject)]
pub struct ManualImportFileMappingInput {
    pub file_path: String,
    pub episode_id: String,
    pub quality: Option<String>,
}

#[derive(InputObject)]
pub struct ExecuteManualImportInput {
    pub title_id: String,
    pub files: Vec<ManualImportFileMappingInput>,
}

#[derive(SimpleObject, Clone)]
pub struct ManualImportFileResultPayload {
    pub file_path: String,
    pub episode_id: String,
    pub success: bool,
    pub dest_path: Option<String>,
    pub error_message: Option<String>,
}

// --- Wanted Items / Acquisition ---

#[derive(SimpleObject, Clone)]
pub struct WantedItemPayload {
    pub id: String,
    pub title_id: String,
    pub title_name: Option<String>,
    pub episode_id: Option<String>,
    pub media_type: String,
    pub search_phase: String,
    pub next_search_at: Option<String>,
    pub last_search_at: Option<String>,
    pub search_count: i64,
    pub baseline_date: Option<String>,
    pub status: String,
    pub grabbed_release: Option<String>,
    pub current_score: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct WantedItemsListPayload {
    pub items: Vec<WantedItemPayload>,
    pub total: i64,
}

#[derive(SimpleObject, Clone)]
pub struct ReleaseDecisionPayload {
    pub id: String,
    pub wanted_item_id: String,
    pub title_id: String,
    pub release_title: String,
    pub release_url: Option<String>,
    pub release_size_bytes: Option<i64>,
    pub decision_code: String,
    pub candidate_score: i32,
    pub current_score: Option<i32>,
    pub score_delta: Option<i32>,
    pub explanation_json: Option<String>,
    pub created_at: String,
}

#[derive(InputObject)]
pub struct WantedItemIdInput {
    pub wanted_item_id: String,
}

#[derive(InputObject)]
pub struct TitleIdInput {
    pub title_id: String,
}

// ── Rule Sets ──────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct RuleSetPayload {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rego_source: String,
    pub enabled: bool,
    pub priority: i32,
    pub applied_facets: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct RuleValidationResultPayload {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(InputObject)]
pub struct CreateRuleSetInput {
    pub name: String,
    pub description: Option<String>,
    pub rego_source: String,
    pub applied_facets: Option<Vec<String>>,
    pub priority: Option<i32>,
}

#[derive(InputObject)]
pub struct UpdateRuleSetInput {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub rego_source: Option<String>,
    pub applied_facets: Option<Vec<String>>,
    pub priority: Option<i32>,
}

#[derive(InputObject)]
pub struct ToggleRuleSetInput {
    pub id: String,
    pub enabled: bool,
}

#[derive(InputObject)]
pub struct ValidateRuleSetInput {
    pub rego_source: String,
    pub rule_set_id: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ServiceLogsPayload {
    pub generated_at: String,
    pub lines: Vec<String>,
    pub count: i32,
}

// ── Metadata Gateway (proxied from SMG) ────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct MetadataSearchItemPayload {
    pub tvdb_id: String,
    pub name: String,
    pub imdb_id: Option<String>,
    pub slug: Option<String>,
    #[graphql(name = "type")]
    pub type_hint: Option<String>,
    pub year: Option<i32>,
    pub status: Option<String>,
    pub overview: Option<String>,
    pub popularity: Option<f64>,
    pub poster_url: Option<String>,
    pub language: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub sort_title: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataSearchMultiPayload {
    pub movies: Vec<MetadataSearchItemPayload>,
    pub series: Vec<MetadataSearchItemPayload>,
    pub anime: Vec<MetadataSearchItemPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataMoviePayload {
    pub tvdb_id: String,
    pub name: String,
    pub slug: String,
    pub year: Option<i32>,
    pub status: String,
    pub overview: String,
    pub poster_url: String,
    pub language: String,
    pub runtime_minutes: i32,
    pub sort_title: String,
    pub imdb_id: String,
    pub genres: Vec<String>,
    pub studio: String,
    pub tmdb_release_date: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataSeriesPayload {
    pub tvdb_id: String,
    pub name: String,
    pub sort_name: String,
    pub slug: String,
    pub year: Option<i32>,
    pub status: String,
    pub first_aired: String,
    pub overview: String,
    pub network: String,
    pub runtime_minutes: i32,
    pub poster_url: String,
    pub country: String,
    pub genres: Vec<String>,
    pub aliases: Vec<String>,
    pub seasons: Vec<MetadataSeasonPayload>,
    pub episodes: Vec<MetadataEpisodePayload>,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataSeasonPayload {
    pub tvdb_id: String,
    pub number: i32,
    pub label: String,
    pub episode_type: String,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataEpisodePayload {
    pub tvdb_id: String,
    pub episode_number: i32,
    pub season_number: i32,
    pub name: String,
    pub aired: String,
    pub runtime_minutes: i32,
    pub is_filler: bool,
}

#[derive(SimpleObject, Clone)]
pub struct CalendarEpisodePayload {
    pub id: String,
    pub title_id: String,
    pub title_name: String,
    pub title_facet: String,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub episode_title: Option<String>,
    pub air_date: Option<String>,
    pub monitored: bool,
}

// ── Plugins ────────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct RegistryPluginPayload {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub author: String,
    pub official: bool,
    pub builtin: bool,
    pub source_url: Option<String>,
    pub is_installed: bool,
    pub is_enabled: bool,
    pub installed_version: Option<String>,
    pub update_available: bool,
    pub default_base_url: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct PluginInstallationPayload {
    pub id: String,
    pub plugin_id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub is_enabled: bool,
    pub is_builtin: bool,
    pub source_url: Option<String>,
    pub installed_at: String,
    pub updated_at: String,
}

#[derive(InputObject)]
pub struct InstallPluginInput {
    pub plugin_id: String,
}

#[derive(InputObject)]
pub struct UninstallPluginInput {
    pub plugin_id: String,
}

#[derive(InputObject)]
pub struct TogglePluginInput {
    pub plugin_id: String,
    pub enabled: bool,
}

#[derive(InputObject)]
pub struct UpgradePluginInput {
    pub plugin_id: String,
}

// ── Provider Type Config Schema ─────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct PluginConfigFieldOptionPayload {
    pub value: String,
    pub label: String,
}

#[derive(SimpleObject, Clone)]
pub struct PluginConfigFieldPayload {
    pub key: String,
    pub label: String,
    pub field_type: String,
    pub required: bool,
    pub default_value: Option<String>,
    pub options: Vec<PluginConfigFieldOptionPayload>,
    pub help_text: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ProviderTypePayload {
    pub provider_type: String,
    pub name: String,
    pub config_fields: Vec<PluginConfigFieldPayload>,
    pub default_base_url: Option<String>,
}

// ── Notification types ─────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct NotificationChannelPayload {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub config_json: String,
    pub is_enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct NotificationSubscriptionPayload {
    pub id: String,
    pub channel_id: String,
    pub event_type: String,
    pub scope: String,
    pub scope_id: Option<String>,
    pub is_enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(InputObject)]
pub struct CreateNotificationChannelInput {
    pub name: String,
    pub channel_type: String,
    pub config_json: String,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateNotificationChannelInput {
    pub id: String,
    pub name: Option<String>,
    pub config_json: Option<String>,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct CreateNotificationSubscriptionInput {
    pub channel_id: String,
    pub event_type: String,
    pub scope: String,
    pub scope_id: Option<String>,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateNotificationSubscriptionInput {
    pub id: String,
    pub event_type: Option<String>,
    pub scope: Option<String>,
    pub scope_id: Option<String>,
    pub is_enabled: Option<bool>,
}

/// Notification provider type payload (reuses the same shape as indexer provider types)
#[derive(SimpleObject, Clone)]
pub struct NotificationProviderTypePayload {
    pub provider_type: String,
    pub name: String,
    pub config_fields: Vec<PluginConfigFieldPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct BackupInfoPayload {
    pub filename: String,
    pub size_bytes: String,
    pub created_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct HealthCheckPayload {
    pub source: String,
    pub status: String,
    pub message: String,
}

#[derive(SimpleObject, Clone)]
pub struct RssSyncReportPayload {
    pub releases_fetched: i32,
    pub releases_matched: i32,
    pub releases_grabbed: i32,
    pub releases_held: i32,
}

#[derive(SimpleObject, Clone)]
pub struct PendingReleasePayload {
    pub id: String,
    pub wanted_item_id: String,
    pub title_id: String,
    pub release_title: String,
    pub release_url: Option<String>,
    pub release_size_bytes: Option<String>,
    pub release_score: i32,
    pub scoring_log_json: Option<String>,
    pub indexer_source: Option<String>,
    pub added_at: String,
    pub delay_until: String,
    pub status: String,
}

#[derive(InputObject)]
pub struct PendingReleaseActionInput {
    pub id: String,
}

#[derive(SimpleObject, Clone)]
pub struct HousekeepingReportPayload {
    pub orphaned_media_files: i32,
    pub stale_release_decisions: i32,
    pub stale_release_attempts: i32,
    pub expired_event_outboxes: i32,
    pub stale_history_events: i32,
    pub recycled_purged: i32,
    pub ran_at: String,
}

#[derive(SimpleObject, Clone)]
pub struct SetupStatusPayload {
    pub setup_complete: bool,
    pub has_download_clients: bool,
    pub has_indexers: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DirectoryEntryPayload {
    pub name: String,
    pub path: String,
}

// ── External Import (Sonarr/Radarr) ────────────────────────────────────────

#[derive(InputObject)]
pub struct ExternalImportConnectionInput {
    pub base_url: String,
    pub api_key: String,
}

#[derive(InputObject)]
pub struct PreviewExternalImportInput {
    pub sonarr: Option<ExternalImportConnectionInput>,
    pub radarr: Option<ExternalImportConnectionInput>,
}

#[derive(InputObject)]
pub struct ExecuteExternalImportInput {
    pub sonarr: Option<ExternalImportConnectionInput>,
    pub radarr: Option<ExternalImportConnectionInput>,
    pub selected_movies_path: Option<String>,
    pub selected_series_path: Option<String>,
    pub selected_anime_path: Option<String>,
    pub selected_download_client_dedup_keys: Vec<String>,
    pub selected_indexer_dedup_keys: Vec<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportPreviewPayload {
    pub sonarr_connected: bool,
    pub radarr_connected: bool,
    pub sonarr_version: Option<String>,
    pub radarr_version: Option<String>,
    pub root_folders: Vec<ExternalImportRootFolderPayload>,
    pub download_clients: Vec<ExternalImportDownloadClientPayload>,
    pub indexers: Vec<ExternalImportIndexerPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportRootFolderPayload {
    pub source: String,
    pub path: String,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportDownloadClientPayload {
    pub sources: Vec<String>,
    pub name: String,
    pub implementation: String,
    pub scryer_client_type: Option<String>,
    pub host: Option<String>,
    pub port: Option<String>,
    pub use_ssl: bool,
    pub url_base: Option<String>,
    pub username: Option<String>,
    pub api_key: Option<String>,
    pub dedup_key: String,
    pub supported: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportIndexerPayload {
    pub sources: Vec<String>,
    pub name: String,
    pub implementation: String,
    pub scryer_provider_type: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub dedup_key: String,
    pub supported: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportResultPayload {
    pub media_paths_saved: bool,
    pub download_clients_created: i32,
    pub indexers_created: i32,
    pub plugins_installed: Vec<String>,
    pub errors: Vec<String>,
}
