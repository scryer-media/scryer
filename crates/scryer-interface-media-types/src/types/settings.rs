use super::{
    ContentScopeValue, DelayProfilePreferredProtocolValue, FillerPolicyValue, ImportModeValue,
    Long, MediaFacetValue, RecapPolicyValue, RenameCollisionPolicyValue,
    RenameMissingMetadataPolicyValue, RootFolderPayload, ScoringPersonaValue,
};
use async_graphql::{Enum, ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

#[derive(SimpleObject, Clone)]
/// Subtitle language preference with hearing-impaired and forced flags.
pub struct SubtitleLanguagePreferencePayload {
    /// Language code.
    pub code: String,
    /// Whether hearing-impaired subtitles are preferred.
    pub hearing_impaired: bool,
    /// Whether forced subtitles are preferred.
    pub forced: bool,
}

#[derive(SimpleObject, Clone)]
/// Effective subtitle search, download, and synchronization settings.
pub struct SubtitleSettingsPayload {
    /// Whether subtitle processing is enabled.
    pub enabled: bool,
    /// Ordered subtitle language preferences.
    pub languages: Vec<SubtitleLanguagePreferencePayload>,
    /// Whether subtitles are downloaded automatically after import.
    pub auto_download_on_import: bool,
    /// Minimum subtitle score for series, on the provider score scale.
    pub minimum_score_series: i32,
    /// Minimum subtitle score for movies, on the provider score scale.
    pub minimum_score_movie: i32,
    /// Search interval in hours.
    pub search_interval_hours: i32,
    /// Whether AI-translated subtitles are eligible.
    pub include_ai_translated: bool,
    /// Whether machine-translated subtitles are eligible.
    pub include_machine_translated: bool,
    /// Whether subtitle synchronization is enabled.
    pub sync_enabled: bool,
    /// Synchronization threshold for series, on the provider score scale.
    pub sync_threshold_series: i32,
    /// Synchronization threshold for movies, on the provider score scale.
    pub sync_threshold_movie: i32,
    /// Maximum subtitle offset correction in seconds.
    pub sync_max_offset_seconds: i32,
}

#[derive(SimpleObject, Clone)]
/// Recycle-bin enablement setting.
pub struct RecycleBinSettingsPayload {
    /// Whether deleted media is moved to the recycle bin.
    pub enabled: bool,
}

#[derive(SimpleObject, Clone)]
/// Automatic official-plugin patch update setting.
pub struct PluginAutoUpdateSettingsPayload {
    /// Whether the scheduled plugin catalog refresh installs official patch updates automatically.
    pub enabled: bool,
}

#[derive(SimpleObject, Clone)]
/// Acquisition worker enablement and polling or convergence limits.
pub struct AcquisitionSettingsPayload {
    /// Whether automatic acquisition is enabled.
    pub enabled: bool,
    /// Upgrade cooldown in hours.
    pub upgrade_cooldown_hours: i32,
    /// Minimum score delta for same-tier upgrades.
    pub same_tier_min_delta: i32,
    /// Deprecated and inert. Quality tier is compared before score, so no
    /// score delta ever sees a cross-tier comparison; the stored value is
    /// returned unchanged and ignored by acquisition. Scheduled for removal in
    /// a later minor.
    #[graphql(
        deprecation = "Inert since 0.18.17: quality tier is compared before score, so no cross-tier delta is consulted. The value is stored and ignored; the field will be removed in a later minor."
    )]
    pub cross_tier_min_delta: i32,
    /// Score delta that bypasses normal forced-upgrade thresholds.
    pub forced_upgrade_delta_bypass: i32,
    /// Acquisition polling interval in seconds.
    pub poll_interval_seconds: i32,
    /// Maximum long-tail scopes processed per cycle.
    pub long_tail_backfill_max_scopes_per_cycle: i32,
    /// Number of days before long-tail scopes are reconverged.
    pub long_tail_reconverge_days: i32,
}

#[derive(SimpleObject, Clone)]
/// Trusted plugin HTTP certificate identified by SHA-256 fingerprint.
pub struct PluginHttpTrustedCertificatePayload {
    /// Lower-level certificate SHA-256 fingerprint.
    pub fingerprint_sha256: String,
    /// PEM-encoded certificate body.
    pub pem: String,
}

#[derive(SimpleObject, Clone)]
/// General service settings, including effective image-cache limits and trusted certificate data.
pub struct GeneralSettingsPayload {
    /// Whether import history is retained indefinitely.
    pub keep_history_forever: bool,
    /// History retention period in days when indefinite retention is false.
    pub history_retention_days: i32,
    /// Configured image-cache maximum in megabytes.
    pub image_cache_max_size_mb: i32,
    /// Effective image-cache maximum in bytes after environment overrides.
    pub effective_image_cache_max_size_bytes: Long,
    /// Effective image-cache maximum in megabytes after environment overrides.
    pub effective_image_cache_max_size_mb: f64,
    /// Whether an environment variable overrides the configured image-cache limit.
    pub image_cache_max_size_env_override_active: bool,
    /// PEM CA bundle used for plugin HTTP requests.
    pub plugin_http_ca_bundle_pem: String,
    /// Additional trusted plugin HTTP certificates.
    pub plugin_http_trusted_certificates: Vec<PluginHttpTrustedCertificatePayload>,
}

#[derive(SimpleObject, Clone)]
/// Automatic backup schedule and encryption-key readiness.
pub struct AutoBackupSettingsPayload {
    /// Whether automatic backups are enabled.
    pub enabled: bool,
    /// Daily local-time schedule in the service's configured time format.
    pub daily_time_local: String,
    /// Whether the automatic-backup encryption key is present.
    pub auto_backup_key_present: bool,
    /// Whether automatic backup is disabled because the key is missing.
    pub auto_backup_disabled_missing_key_notice: bool,
    /// Next scheduled run in UTC, or null when no run is scheduled.
    pub next_run_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Configured, default, and effective backup paths.
pub struct BackupSettingsPayload {
    /// Custom backup path, or null when the default path is used.
    pub custom_backup_path: Option<String>,
    /// Service default backup path.
    pub default_backup_path: String,
    /// Effective path selected after applying the custom override.
    pub effective_backup_path: String,
}

#[derive(SimpleObject, Clone)]
/// Security settings and effective environment overrides.
pub struct SecuritySettingsPayload {
    /// Whether form-based login is enabled by configuration.
    pub form_login_enabled: bool,
    /// Minimum accepted password length.
    pub password_min_length: i32,
    /// Whether local IPs may skip login.
    pub skip_login_for_local_ips: bool,
    /// Whether API keys are restricted to users with system-settings permission.
    pub api_keys_restrict_to_system_settings_users: bool,
    /// Whether configuration changes require MFA step-up.
    pub mfa_require_config_step_up: bool,
    /// Whether password login is required alongside MFA.
    pub mfa_require_password_login: bool,
    /// Whether Jellyfin login requires an enrolled authentication factor.
    pub mfa_require_jellyfin_login: bool,
    /// Whether Emby login requires an enrolled authentication factor.
    pub mfa_require_emby_login: bool,
    /// Deprecated alias for `mfaRequireJellyfinLogin`.
    #[graphql(deprecation = "Use mfaRequireJellyfinLogin.")]
    pub totp_require_jellyfin_login: bool,
    /// Deprecated alias for `mfaRequireEmbyLogin`.
    #[graphql(deprecation = "Use mfaRequireEmbyLogin.")]
    pub totp_require_emby_login: bool,
    /// Effective form-login state after environment overrides.
    pub effective_form_login_enabled: bool,
    /// Whether an environment override is active.
    pub env_override_active: bool,
    /// Description of the active override, or null when none is active.
    pub env_override_description: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Origin of an OAuth client registration.
pub enum OAuthClientSourceValue {
    /// Registration managed by Scryer and not editable through settings.
    Managed,
    /// Registration created and maintained by an administrator.
    Custom,
}

#[derive(SimpleObject, Clone)]
/// An OAuth application allowed to receive Scryer authorization codes.
pub struct OAuthClientRegistrationPayload {
    /// Immutable OAuth client identifier.
    pub client_id: String,
    /// Name shown to users on the authorization screen.
    pub display_name: String,
    /// Exact callback URL allowlist. Managed native clients may use an empty list.
    pub redirect_uris: Vec<String>,
    /// Whether the application can authorize or refresh tokens.
    pub enabled: bool,
    /// Whether the application is managed by Scryer or an administrator.
    pub source: OAuthClientSourceValue,
}

#[derive(SimpleObject, Clone)]
/// Public authorization-screen identity for a validated OAuth request.
pub struct OAuthAuthorizationClientPayload {
    /// Requested OAuth client identifier.
    pub client_id: String,
    /// Client name safe to display after callback validation.
    pub display_name: String,
    /// Server-validated, canonical requested scope set for this authorization decision.
    pub scope: String,
}

#[derive(SimpleObject, Clone)]
/// Result of deleting a custom OAuth application.
pub struct DeleteOAuthClientRegistrationPayload {
    /// Deleted OAuth client identifier.
    pub client_id: String,
    /// Whether a persisted custom registration was removed.
    pub deleted: bool,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Theme preference for the caller's settings.
pub enum UiThemeValue {
    /// Light theme.
    Light,
    /// Dark theme.
    Dark,
    /// Pride theme.
    Pride,
    /// Follow the system theme.
    System,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Date and time display format preference.
pub enum UiDateTimeFormatValue {
    /// Use locale-specific formatting.
    Locale,
    #[graphql(name = "ISO24H")]
    /// Use ISO date and 24-hour time formatting.
    Iso24h,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Density preference for settings and data presentation.
pub enum UiDensityValue {
    /// Compact density.
    Compact,
    /// Comfortable density.
    Comfortable,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Sidebar expansion preference.
pub enum UiSidebarModeValue {
    /// Keep the sidebar collapsed.
    Collapsed,
    /// Keep the sidebar expanded.
    Expanded,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Default landing content area.
pub enum UiDefaultLandingViewValue {
    /// Movies view.
    Movies,
    /// Series view.
    Series,
    /// Anime view.
    Anime,
    /// Activity view.
    Activity,
    /// Calendar view.
    Calendar,
    /// Wanted view.
    Wanted,
    /// History view.
    History,
    /// Settings view.
    Settings,
    /// System view.
    System,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Media facet used by table-column settings.
pub enum UiSettingsFacetValue {
    /// Movies facet.
    Movies,
    /// Series facet.
    Series,
    /// Anime facet.
    Anime,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Table presentation mode.
pub enum UiTableViewModeValue {
    /// Compact table mode.
    Compact,
    /// Poster table mode.
    PosterTable,
}

#[derive(SimpleObject, Clone)]
/// One persisted table-column preference for a facet and table mode.
pub struct UiTableColumnSettingPayload {
    /// Media facet to which the column setting applies.
    pub facet: UiSettingsFacetValue,
    /// Table mode to which the column setting applies.
    pub table_view_mode: UiTableViewModeValue,
    /// Stable column identifier.
    pub column_id: String,
    /// Zero-based display order.
    pub column_order: i32,
    /// Whether the column is visible.
    pub visible: bool,
}

#[derive(SimpleObject, Clone)]
/// Persisted caller presentation preferences and table-column settings.
pub struct UiSettingsPayload {
    /// Selected theme.
    pub theme: UiThemeValue,
    /// Selected date and time format.
    pub date_time_format: UiDateTimeFormatValue,
    /// Primary highlight color, or null when using the default.
    pub highlight_color: Option<String>,
    /// Secondary color, or null when using the default.
    pub secondary_color: Option<String>,
    /// Whether high-contrast presentation is enabled.
    pub high_contrast_mode: bool,
    /// Whether motion-reduction behavior is enabled.
    pub reduce_motion: bool,
    /// Whether the sponsor button is hidden.
    pub hide_sponsor_button: bool,
    /// Selected density.
    pub density: UiDensityValue,
    /// Selected sidebar mode.
    pub sidebar_mode: UiSidebarModeValue,
    /// Content area opened by default.
    pub default_landing_view: UiDefaultLandingViewValue,
    /// Persisted table-column settings.
    pub table_columns: Vec<UiTableColumnSettingPayload>,
}

#[derive(InputObject, Clone)]
/// Input form for one table-column preference.
pub struct UiTableColumnSettingInput {
    /// Media facet to which the setting applies.
    pub facet: UiSettingsFacetValue,
    /// Table mode to which the setting applies.
    pub table_view_mode: UiTableViewModeValue,
    /// Stable column identifier.
    pub column_id: String,
    /// Zero-based display order.
    pub column_order: i32,
    /// Whether the column should be visible.
    pub visible: bool,
}

#[derive(InputObject, Clone)]
/// Complete caller presentation-preferences update.
pub struct SetMyUiSettingsInput {
    /// Selected theme.
    pub theme: UiThemeValue,
    /// Optional date and time format; null preserves the existing value.
    pub date_time_format: Option<UiDateTimeFormatValue>,
    /// Optional primary highlight color; null preserves the existing value.
    pub highlight_color: Option<String>,
    /// Optional secondary color; null preserves the existing value.
    pub secondary_color: Option<String>,
    /// Whether high-contrast presentation should be enabled.
    pub high_contrast_mode: bool,
    /// Whether motion-reduction behavior should be enabled.
    pub reduce_motion: bool,
    /// Whether the sponsor button should be hidden.
    pub hide_sponsor_button: bool,
    /// Selected density.
    pub density: UiDensityValue,
    /// Selected sidebar mode.
    pub sidebar_mode: UiSidebarModeValue,
    /// Content area to open by default.
    pub default_landing_view: UiDefaultLandingViewValue,
    /// Complete table-column setting list; empty clears all saved column settings.
    pub table_columns: Vec<UiTableColumnSettingInput>,
}

#[derive(SimpleObject, Clone)]
/// Effective authentication runtime flags after configuration and environment overrides.
pub struct AuthRuntimeStatePayload {
    /// Effective form-login state.
    pub effective_form_login_enabled: bool,
    /// Whether local IPs may skip login.
    pub skip_login_for_local_ips: bool,
    /// Whether passkey authentication is enabled.
    pub passkey_enabled: bool,
    /// Whether this request's network provenance defaults sessions to persistent storage.
    pub default_persist_session: bool,
    /// Whether an environment override is active.
    pub env_override_active: bool,
    /// Whether password login is required alongside MFA.
    pub mfa_require_password_login: bool,
    /// Whether configuration changes require MFA step-up.
    pub mfa_require_config_step_up: bool,
    /// Whether Jellyfin login requires an enrolled authentication factor.
    pub mfa_require_jellyfin_login: bool,
    /// Whether Emby login requires an enrolled authentication factor.
    pub mfa_require_emby_login: bool,
    /// Deprecated alias for `mfaRequireJellyfinLogin`.
    #[graphql(deprecation = "Use mfaRequireJellyfinLogin.")]
    pub totp_require_jellyfin_login: bool,
    /// Deprecated alias for `mfaRequireEmbyLogin`.
    #[graphql(deprecation = "Use mfaRequireEmbyLogin.")]
    pub totp_require_emby_login: bool,
}

#[derive(SimpleObject, Clone)]
/// Delay profile controlling protocol timing and acquisition bypass rules.
pub struct DelayProfilePayload {
    /// Delay-profile ID.
    pub id: ID,
    /// Delay-profile name.
    pub name: String,
    /// Usenet delay in minutes.
    pub usenet_delay_minutes: i32,
    /// Torrent delay in minutes.
    pub torrent_delay_minutes: i32,
    /// Whether Usenet releases are eligible for this profile.
    pub enable_usenet: bool,
    /// Whether torrent releases are eligible for this profile.
    pub enable_torrent: bool,
    /// Preferred protocol after delay eligibility.
    pub preferred_protocol: DelayProfilePreferredProtocolValue,
    /// Minimum release age in minutes.
    pub min_age_minutes: i32,
    /// Score threshold that bypasses delay, or null when disabled.
    pub bypass_score_threshold: Option<i32>,
    /// Whether the highest-quality release bypasses its eligible delay.
    pub bypass_if_highest_quality: bool,
    /// Media facets to which the profile applies.
    pub applies_to_facets: Vec<MediaFacetValue>,
    /// Tags used to select this profile.
    pub tags: Vec<String>,
    /// Lower values run with higher priority according to service ordering.
    pub priority: i32,
    /// Whether the profile is enabled.
    pub enabled: bool,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after requesting delay-profile deletion.
pub struct DelayProfileDeletionPayload {
    /// Deleted delay-profile ID.
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
/// Optional boolean scoring overrides for quality evaluation.
pub struct ScoringOverridesPayload {
    /// Whether non-4K x265 releases are allowed.
    pub allow_x265_non4k: Option<bool>,
    /// Whether Dolby Vision without a fallback is blocked.
    pub block_dv_without_fallback: Option<bool>,
    /// Whether compact encodes are preferred.
    pub prefer_compact_encodes: Option<bool>,
    /// Whether lossless audio is preferred.
    pub prefer_lossless_audio: Option<bool>,
    /// Whether upscaled releases are blocked.
    pub block_upscaled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Quality acceptance criteria and scoring controls.
pub struct QualityProfileCriteriaPayload {
    /// Ordered quality tiers accepted by the profile.
    pub quality_tiers: Vec<String>,
    /// Archival quality label, or null when not configured.
    pub archival_quality: Option<String>,
    /// Whether unknown quality values are accepted.
    pub allow_unknown_quality: bool,
    /// Allowed source labels.
    pub source_allowlist: Vec<String>,
    /// Blocked source labels.
    pub source_blocklist: Vec<String>,
    /// Allowed video codec labels.
    pub video_codec_allowlist: Vec<String>,
    /// Blocked video codec labels.
    pub video_codec_blocklist: Vec<String>,
    /// Allowed audio codec labels.
    pub audio_codec_allowlist: Vec<String>,
    /// Blocked audio codec labels.
    pub audio_codec_blocklist: Vec<String>,
    /// Whether Dolby Vision is accepted.
    pub dolby_vision_allowed: bool,
    /// Whether detected HDR is accepted.
    pub detected_hdr_allowed: bool,
    /// Whether remux releases are preferred.
    pub prefer_remux: bool,
    /// Whether Blu-ray disc releases are accepted.
    pub allow_bd_disk: bool,
    /// Whether quality upgrades are allowed.
    pub allow_upgrades: bool,
    /// Optional boolean scoring overrides.
    pub scoring_overrides: ScoringOverridesPayload,
    /// Cutoff quality tier, or null when no cutoff is configured.
    pub cutoff_tier: Option<String>,
    /// Minimum score required to grab, or null when no threshold is configured
    /// (Sonarr's `MinFormatScore`).
    pub min_score_to_grab: Option<i32>,
    /// Score past which a file is good enough and same-tier upgrades stop, or
    /// null when upgrades never stop (Sonarr's `CutoffFormatScore`).
    pub cutoff_score: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Named quality profile and its acceptance criteria.
pub struct QualityProfilePayload {
    /// Quality-profile ID.
    pub id: ID,
    /// Quality-profile name.
    pub name: String,
    /// Acceptance criteria and scoring controls.
    pub criteria: QualityProfileCriteriaPayload,
}

#[derive(SimpleObject, Clone)]
/// Effective quality-profile selection for one content scope.
pub struct QualityProfileSelectionPayload {
    /// Content scope to which the selection applies.
    pub scope: ContentScopeValue,
    /// Scope-specific profile override ID, or null when inherited.
    pub override_profile_id: Option<ID>,
    /// Quality-profile ID selected after applying inheritance and overrides.
    pub effective_profile_id: ID,
    /// Whether the effective profile comes from the global setting.
    pub inherits_global: bool,
}

#[derive(SimpleObject, Clone)]
/// Effective scoring-persona selection for one content scope.
pub struct FacetScoringPersonaSelectionPayload {
    /// Content scope to which the selection applies.
    pub scope: ContentScopeValue,
    /// Scope-specific persona override, or null when inherited.
    pub override_persona: Option<ScoringPersonaValue>,
    /// Effective scoring persona.
    pub effective_persona: ScoringPersonaValue,
    /// Whether the effective persona comes from the global setting.
    pub inherits_global: bool,
}

#[derive(SimpleObject, Clone)]
/// Quality profiles, global defaults, and per-content-scope selections.
pub struct QualityProfileSettingsPayload {
    /// Available quality profiles.
    pub profiles: Vec<QualityProfilePayload>,
    /// Global quality-profile ID.
    pub global_profile_id: ID,
    /// Scoring persona inherited by content scopes without an override.
    pub global_scoring_persona: ScoringPersonaValue,
    /// Effective profile selection by content scope.
    pub category_selections: Vec<QualityProfileSelectionPayload>,
    /// Effective scoring-persona selection by content scope.
    pub category_persona_selections: Vec<FacetScoringPersonaSelectionPayload>,
}

#[derive(SimpleObject, Clone)]
/// Download-client routing behavior for one client.
pub struct DownloadClientRoutingEntryPayload {
    /// Download-client ID.
    pub client_id: ID,
    /// Whether this route is enabled.
    pub enabled: bool,
    /// Optional category assigned to recent downloads.
    pub category: Option<String>,
    /// Priority for recent queue items, or null when unspecified.
    pub recent_queue_priority: Option<String>,
    /// Priority for older queue items, or null when unspecified.
    pub older_queue_priority: Option<String>,
    /// Whether completed items are removed from the client.
    pub remove_completed: bool,
    /// Whether failed items are removed from the client.
    pub remove_failed: bool,
    /// Optional seeding profile ID applied to torrents routed to this client.
    pub seeding_profile_id: Option<ID>,
}

#[derive(SimpleObject, Clone)]
/// Indexer routing behavior for one indexer.
pub struct IndexerRoutingEntryPayload {
    /// Indexer configuration ID.
    pub indexer_id: ID,
    /// Whether this route is enabled.
    pub enabled: bool,
    /// Categories accepted by this route.
    pub categories: Vec<String>,
    /// Relative routing priority.
    pub priority: i32,
}

#[derive(SimpleObject, Clone)]
/// Effective media path, naming, import, permission, and monitoring settings for one content scope.
pub struct MediaSettingsPayload {
    /// Content scope to which these settings apply.
    pub scope: ContentScopeValue,
    /// Primary library path.
    pub library_path: String,
    /// Configured root folders.
    pub root_folders: Vec<RootFolderPayload>,
    /// Effective configured requirements; `original` remains unchanged.
    pub required_audio_languages: Vec<String>,
    /// Whether episodic titles use season folders.
    pub use_season_folders: bool,
    /// Folder naming template.
    pub folder_template: String,
    /// Season-folder template, or null for facets without seasons.
    pub season_folder_template: Option<String>,
    /// Specials-folder template, or null when not configured.
    pub specials_folder_template: Option<String>,
    /// Whether automatic renaming is enabled.
    pub rename_enabled: bool,
    /// Rename filename template.
    pub rename_template: String,
    /// Collision policy for renames.
    pub rename_collision_policy: RenameCollisionPolicyValue,
    /// Missing-metadata policy for renames.
    pub rename_missing_metadata_policy: RenameMissingMetadataPolicyValue,
    /// Effective filler policy, or null when unset.
    pub filler_policy: Option<FillerPolicyValue>,
    /// Effective recap policy, or null when unset.
    pub recap_policy: Option<RecapPolicyValue>,
    /// Whether specials are monitored, or null when unset.
    pub monitor_specials: Option<bool>,
    /// Whether inter-season movies are monitored, or null when unset.
    pub inter_season_movies: Option<bool>,
    /// Whether filler movies are monitored, or null when unset.
    pub monitor_filler_movies: Option<bool>,
    /// Whether NFO files are written on import.
    pub nfo_write_on_import: bool,
    /// Whether Plex match files are written on import, or null when unset.
    pub plexmatch_write_on_import: Option<bool>,
    /// Import mode applied to this scope.
    pub import_mode: ImportModeValue,
    /// Whether Linux permissions are updated after import.
    pub set_permissions_linux: bool,
    /// File chmod mode, or null when unset.
    pub file_chmod: Option<String>,
    /// Folder chmod mode, or null when unset.
    pub folder_chmod: Option<String>,
    /// Chown group, or null when unset.
    pub chown_group: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Filesystem paths used by the three supported media facets.
pub struct LibraryPathsPayload {
    /// Movie library path.
    pub movie_path: String,
    /// Series library path.
    pub series_path: String,
    /// Anime library path.
    pub anime_path: String,
}

#[derive(SimpleObject, Clone)]
/// TLS certificate and private-key paths used by the service.
pub struct ServiceSettingsPayload {
    /// Filesystem path to the TLS certificate.
    pub tls_cert_path: String,
    /// Filesystem path to the TLS private key.
    pub tls_key_path: String,
}

#[derive(InputObject, Clone)]
/// Subtitle language preference with optional hearing-impaired and forced flags.
pub struct SubtitleLanguagePreferenceInput {
    /// BCP 47 or provider language code.
    pub code: String,
    /// Whether hearing-impaired subtitles are preferred.
    pub hearing_impaired: Option<bool>,
    /// Whether forced subtitles are preferred.
    pub forced: Option<bool>,
}

#[derive(InputObject, Clone)]
/// Delay and routing policy for a download source.
pub struct DelayProfileInput {
    /// Existing delay-profile identity to update.
    pub id: ID,
    /// Display name of the delay profile.
    pub name: String,
    /// Usenet delay in minutes.
    pub usenet_delay_minutes: i32,
    /// Torrent delay in minutes.
    pub torrent_delay_minutes: i32,
    /// Whether Usenet releases are eligible. Defaults to enabled for existing clients.
    pub enable_usenet: Option<bool>,
    /// Whether torrent releases are eligible. Defaults to enabled for existing clients.
    pub enable_torrent: Option<bool>,
    /// Preferred protocol when both delayed sources qualify.
    pub preferred_protocol: DelayProfilePreferredProtocolValue,
    /// Minimum release age in minutes.
    pub min_age_minutes: i32,
    /// Optional score threshold that bypasses the delay.
    pub bypass_score_threshold: Option<i32>,
    /// Whether the highest-quality release bypasses its eligible delay. Defaults to disabled.
    pub bypass_if_highest_quality: Option<bool>,
    /// Facets to which the profile applies.
    pub applies_to_facets: Vec<MediaFacetValue>,
    /// Tags restricting the profile's scope.
    pub tags: Vec<String>,
    /// Relative priority among delay profiles.
    pub priority: i32,
    /// Whether this delay profile is active.
    pub enabled: bool,
}

#[derive(InputObject, Clone)]
/// Library root path and default marker.
pub struct RootFolderInput {
    /// Absolute filesystem path for the library root.
    pub path: String,
    /// Whether this root is the library default.
    pub is_default: bool,
}

#[derive(InputObject, Clone)]
/// Optional media-settings values applied to a content scope.
pub struct UpdateMediaSettingsInput {
    /// Content scope receiving the settings.
    pub scope: ContentScopeValue,
    /// Optional library path override.
    pub library_path: Option<String>,
    /// Optional replacement list of library roots; null leaves existing roots unchanged.
    pub root_folders: Option<Vec<RootFolderInput>>,
    /// Optional required audio-language codes; use `original` to resolve per title.
    pub required_audio_languages: Option<Vec<String>>,
    /// Whether episodic titles use season folders.
    pub use_season_folders: Option<bool>,
    /// Optional title folder template.
    pub folder_template: Option<String>,
    /// Optional season folder template.
    pub season_folder_template: Option<String>,
    /// Optional specials folder template.
    pub specials_folder_template: Option<String>,
    /// Whether automatic renaming is enabled.
    pub rename_enabled: Option<bool>,
    /// Optional file and folder rename template.
    pub rename_template: Option<String>,
    /// Collision policy for generated paths.
    pub rename_collision_policy: Option<RenameCollisionPolicyValue>,
    /// Behavior when metadata needed for renaming is missing.
    pub rename_missing_metadata_policy: Option<RenameMissingMetadataPolicyValue>,
    /// Filler monitoring policy.
    pub filler_policy: Option<FillerPolicyValue>,
    /// Recap monitoring policy.
    pub recap_policy: Option<RecapPolicyValue>,
    /// Whether specials are monitored.
    pub monitor_specials: Option<bool>,
    /// Whether inter-season movies are monitored.
    pub inter_season_movies: Option<bool>,
    /// Whether filler movies are monitored.
    pub monitor_filler_movies: Option<bool>,
    /// Whether NFO metadata is written during import.
    pub nfo_write_on_import: Option<bool>,
    /// Whether Plex match metadata is written during import.
    pub plexmatch_write_on_import: Option<bool>,
    /// Import mode used for files in this scope.
    pub import_mode: Option<ImportModeValue>,
    /// Whether Linux ownership and mode changes are applied.
    pub set_permissions_linux: Option<bool>,
    /// File chmod mode in numeric or accepted symbolic notation.
    pub file_chmod: Option<String>,
    /// Folder chmod mode in numeric or accepted symbolic notation.
    pub folder_chmod: Option<String>,
    /// Optional Unix group name for imported paths.
    pub chown_group: Option<String>,
}

#[derive(InputObject, Clone)]
/// Root paths for movie, series, and optional anime libraries.
pub struct UpdateLibraryPathsInput {
    /// Movie library path.
    pub movie_path: String,
    /// Series library path.
    pub series_path: String,
    /// Anime library path, when configured.
    pub anime_path: Option<String>,
}

#[derive(InputObject, Clone)]
/// TLS certificate and private-key filesystem paths.
pub struct UpdateServiceSettingsInput {
    /// Absolute TLS certificate path.
    pub tls_cert_path: String,
    /// Absolute TLS private-key path.
    pub tls_key_path: String,
}

#[derive(InputObject, Clone)]
/// General retention, cache, and plugin trust settings.
pub struct UpdateGeneralSettingsInput {
    /// Whether history is retained without expiry.
    pub keep_history_forever: Option<bool>,
    /// History retention period in days when not retained forever.
    pub history_retention_days: Option<i32>,
    /// Maximum image-cache size in megabytes.
    pub image_cache_max_size_mb: Option<i32>,
    /// PEM bundle path or contents used to trust plugin HTTP certificates.
    pub plugin_http_ca_bundle_pem: Option<String>,
}

#[derive(InputObject, Clone)]
/// Automatic-backup schedule and key-management settings.
pub struct UpdateAutoBackupSettingsInput {
    /// Whether automatic backups are enabled.
    pub enabled: bool,
    /// Daily backup time in local time format.
    pub daily_time_local: String,
    /// New automatic-backup encryption key, when rotating or setting one.
    pub set_auto_backup_key: Option<String>,
    /// Whether the stored automatic-backup key should be cleared.
    pub clear_auto_backup_key: bool,
}

#[derive(InputObject, Clone)]
/// Optional custom backup destination.
pub struct UpdateBackupSettingsInput {
    /// Filesystem path used for custom backups.
    pub custom_backup_path: Option<String>,
}

#[derive(InputObject, Clone)]
/// Authentication and local-access security settings.
pub struct UpdateSecuritySettingsInput {
    /// Whether form login is enabled.
    pub form_login_enabled: bool,
    /// Minimum accepted password length.
    pub password_min_length: i32,
    /// Whether local IPs may skip login.
    pub skip_login_for_local_ips: bool,
    /// Whether API keys are restricted to users with system-settings permission.
    pub api_keys_restrict_to_system_settings_users: Option<bool>,
    /// Whether sensitive configuration changes require MFA step-up.
    pub mfa_require_config_step_up: bool,
    /// Whether password login requires MFA.
    pub mfa_require_password_login: bool,
    /// Whether Jellyfin login requires an enrolled authentication factor.
    pub mfa_require_jellyfin_login: Option<bool>,
    /// Whether Emby login requires an enrolled authentication factor. Omission preserves the saved setting.
    pub mfa_require_emby_login: Option<bool>,
    /// Deprecated alias for `mfaRequireJellyfinLogin`.
    #[graphql(deprecation = "Use mfaRequireJellyfinLogin.")]
    pub totp_require_jellyfin_login: Option<bool>,
    /// Deprecated alias for `mfaRequireEmbyLogin`. Omission preserves the saved setting.
    #[graphql(deprecation = "Use mfaRequireEmbyLogin.")]
    pub totp_require_emby_login: Option<bool>,
}

#[derive(InputObject, Clone)]
/// Custom public OAuth application details. Authorization-code plus S256 PKCE is required.
pub struct CreateOAuthClientRegistrationInput {
    /// Name displayed to users during authorization.
    pub display_name: String,
    /// Exact HTTPS callback URLs permitted for this application.
    pub redirect_uris: Vec<String>,
}

#[derive(InputObject, Clone)]
/// Replacement details for a custom OAuth application.
pub struct UpdateOAuthClientRegistrationInput {
    /// Name displayed to users during authorization.
    pub display_name: String,
    /// Exact HTTPS callback URLs permitted for this application.
    pub redirect_uris: Vec<String>,
    /// Whether the application is active. Disabling revokes its existing grants.
    pub enabled: bool,
}

#[derive(InputObject, Clone)]
/// Subtitle acquisition, translation, and synchronization settings.
pub struct UpdateSubtitleSettingsInput {
    /// Whether subtitle processing is enabled.
    pub enabled: bool,
    /// Ordered language preferences.
    pub languages: Vec<SubtitleLanguagePreferenceInput>,
    /// Whether subtitles are downloaded automatically after import.
    pub auto_download_on_import: bool,
    /// Minimum provider score for series subtitles.
    pub minimum_score_series: i32,
    /// Minimum provider score for movie subtitles.
    pub minimum_score_movie: i32,
    /// Search interval in hours.
    pub search_interval_hours: i32,
    /// Whether AI-translated subtitles are accepted.
    pub include_ai_translated: bool,
    /// Whether machine-translated subtitles are accepted.
    pub include_machine_translated: bool,
    /// Whether subtitle synchronization is enabled.
    pub sync_enabled: bool,
    /// Synchronization threshold for series subtitles.
    pub sync_threshold_series: i32,
    /// Synchronization threshold for movie subtitles.
    pub sync_threshold_movie: i32,
    /// Maximum synchronization offset in seconds.
    pub sync_max_offset_seconds: i32,
}

#[derive(InputObject, Clone)]
/// Recycle-bin enablement setting.
pub struct UpdateRecycleBinSettingsInput {
    /// Whether deleted media is retained in the recycle bin.
    pub enabled: bool,
}

#[derive(InputObject, Clone)]
/// Automatic official-plugin patch update setting.
pub struct UpdatePluginAutoUpdateSettingsInput {
    /// Whether the scheduled plugin catalog refresh installs official patch updates automatically.
    pub enabled: bool,
}

#[derive(InputObject, Clone)]
/// Acquisition scheduler timing and scoring thresholds.
pub struct UpdateAcquisitionSettingsInput {
    /// Whether acquisition scheduling is enabled.
    pub enabled: bool,
    /// Upgrade cooldown in hours.
    pub upgrade_cooldown_hours: i32,
    /// Minimum score improvement for a same-tier upgrade.
    pub same_tier_min_delta: i32,
    /// Deprecated and inert: accepted and stored for compatibility, ignored by
    /// acquisition (quality tier is compared before score, so no cross-tier
    /// delta is ever consulted). Will be removed in a later minor.
    pub cross_tier_min_delta: i32,
    /// Score delta that bypasses the forced-upgrade guard.
    pub forced_upgrade_delta_bypass: i32,
    /// Scheduler poll interval in seconds.
    pub poll_interval_seconds: i32,
    /// Maximum long-tail scopes processed per cycle.
    pub long_tail_backfill_max_scopes_per_cycle: i32,
    /// Days between long-tail reconvergence passes.
    pub long_tail_reconverge_days: i32,
}

#[derive(InputObject, Clone)]
/// Per-title scoring behavior overrides.
pub struct ScoringOverridesInput {
    /// Whether non-4K x265 releases are allowed.
    pub allow_x265_non4k: Option<bool>,
    /// Whether Dolby Vision without a fallback is blocked.
    pub block_dv_without_fallback: Option<bool>,
    /// Whether compact encodes are preferred.
    pub prefer_compact_encodes: Option<bool>,
    /// Whether lossless audio is preferred.
    pub prefer_lossless_audio: Option<bool>,
    /// Whether upscaled releases are blocked.
    pub block_upscaled: Option<bool>,
}

#[derive(InputObject, Clone)]
/// Quality constraints and scoring rules for release selection.
pub struct QualityProfileCriteriaInput {
    /// Ordered quality-tier identifiers.
    pub quality_tiers: Vec<String>,
    /// Optional archival quality tier.
    pub archival_quality: Option<String>,
    /// Whether releases with unknown quality are allowed.
    pub allow_unknown_quality: bool,
    /// Allowed release-source identifiers.
    pub source_allowlist: Vec<String>,
    /// Blocked release-source identifiers.
    pub source_blocklist: Vec<String>,
    /// Allowed video-codec identifiers.
    pub video_codec_allowlist: Vec<String>,
    /// Blocked video-codec identifiers.
    pub video_codec_blocklist: Vec<String>,
    /// Allowed audio-codec identifiers.
    pub audio_codec_allowlist: Vec<String>,
    /// Blocked audio-codec identifiers.
    pub audio_codec_blocklist: Vec<String>,
    /// Whether Dolby Vision is allowed.
    pub dolby_vision_allowed: bool,
    /// Whether detected HDR is allowed.
    pub detected_hdr_allowed: bool,
    /// Whether remux releases are preferred.
    pub prefer_remux: bool,
    /// Whether Blu-ray disk releases are allowed.
    pub allow_bd_disk: bool,
    /// Whether upgrades above the current file are allowed.
    pub allow_upgrades: bool,
    /// Additional scoring preferences.
    pub scoring_overrides: ScoringOverridesInput,
    /// Optional cutoff quality tier.
    pub cutoff_tier: Option<String>,
    /// Optional minimum score required to grab a release.
    pub min_score_to_grab: Option<i32>,
    /// Optional score past which same-tier upgrades stop. Omitting it keeps the
    /// stored value rather than clearing it, so a UI save cannot wipe a value
    /// set through the API before the editor exposes the field.
    pub cutoff_score: Option<i32>,
}

#[derive(InputObject, Clone)]
/// Named quality profile definition.
pub struct QualityProfileInput {
    /// Quality profile identity.
    pub id: ID,
    /// Display name of the profile.
    pub name: String,
    /// Constraints used to accept and score releases.
    pub criteria: QualityProfileCriteriaInput,
}

#[derive(InputObject, Clone)]
/// Quality profile selection for a content scope.
pub struct QualityProfileSelectionInput {
    /// Content scope receiving the selection.
    pub scope: ContentScopeValue,
    /// Quality profile identity; null uses inherited or global behavior.
    pub profile_id: Option<ID>,
    /// Whether the scope inherits the global profile.
    pub inherit_global: bool,
}

#[derive(InputObject, Clone)]
/// Scoring-persona selection for a content scope.
pub struct FacetScoringPersonaSelectionInput {
    /// Content scope receiving the selection.
    pub scope: ContentScopeValue,
    /// Optional scoring persona override.
    pub persona: Option<ScoringPersonaValue>,
    /// Whether the scope inherits the global persona.
    pub inherit_global: bool,
}

#[derive(InputObject, Clone)]
/// Complete quality-profile and scoring-persona settings replacement.
pub struct SaveQualityProfileSettingsInput {
    /// Quality profiles to store.
    pub profiles: Vec<QualityProfileInput>,
    /// Global quality profile identity, when configured.
    pub global_profile_id: Option<ID>,
    /// Global scoring persona, when configured.
    pub global_scoring_persona: Option<ScoringPersonaValue>,
    /// Per-category quality-profile selections.
    pub category_selections: Vec<QualityProfileSelectionInput>,
    /// Per-category scoring-persona selections.
    pub category_persona_selections: Vec<FacetScoringPersonaSelectionInput>,
    /// Whether existing stored profiles and selections are replaced.
    pub replace_existing: bool,
}

#[derive(InputObject, Clone)]
/// Routing settings for one download client.
pub struct DownloadClientRoutingEntryInput {
    /// Download-client identity.
    pub client_id: ID,
    /// Whether this client participates in routing.
    pub enabled: bool,
    /// Optional category sent to the client.
    pub category: Option<String>,
    /// Priority label for recent queue items.
    pub recent_queue_priority: Option<String>,
    /// Priority label for older queue items.
    pub older_queue_priority: Option<String>,
    /// Whether completed items are removed from the client.
    pub remove_completed: bool,
    /// Whether failed items are removed from the client.
    pub remove_failed: bool,
    /// Optional seeding profile ID applied to torrents routed to this client.
    pub seeding_profile_id: Option<ID>,
}

#[derive(InputObject, Clone)]
/// Download-client routing entries for a content scope.
pub struct UpdateDownloadClientRoutingInput {
    /// Content scope receiving the routing rules.
    pub scope: ContentScopeValue,
    /// Routing entries keyed by download-client identity.
    pub entries: Vec<DownloadClientRoutingEntryInput>,
}

#[derive(InputObject, Clone)]
/// Routing settings for one indexer.
pub struct IndexerRoutingEntryInput {
    /// Indexer identity.
    pub indexer_id: ID,
    /// Whether this indexer participates in routing.
    pub enabled: bool,
    /// Indexer categories to request.
    pub categories: Vec<String>,
    /// Relative indexer priority.
    pub priority: i32,
}

#[derive(InputObject, Clone)]
/// Indexer routing entries for a content scope.
pub struct UpdateIndexerRoutingInput {
    /// Content scope receiving the routing rules.
    pub scope: ContentScopeValue,
    /// Routing entries keyed by indexer identity.
    pub entries: Vec<IndexerRoutingEntryInput>,
}
