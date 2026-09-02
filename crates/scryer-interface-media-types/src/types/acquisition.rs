use super::{
    ConvergenceStateValue, DownloadQueueStateValue, DownloadSourceKindValue, Long, MediaFacetValue,
    PendingReleaseStatusValue, PluginConfigFieldOptionPayload, PluginConfigFieldRoleValue,
    PluginConfigFieldTypeValue, PluginConfigValueSourceValue, QueueDownloadPurposeValue,
    QueueDownloadScopeInput, WantedKindValue, WantedStatusValue,
};
use async_graphql::{Enum, ID, InputObject, MaybeUndefined, SimpleObject};
use chrono::{DateTime, Utc};

#[derive(SimpleObject, Clone)]
/// One blocked release for a title.
pub struct TitleReleaseBlocklistEntryPayload {
    /// Blocklist entry ID.
    pub id: ID,
    /// The blocked release's name, as the indexer presented it.
    pub release_name: String,
    /// Failure or blocklist reason, or null when unavailable.
    pub error_message: Option<String>,
    /// UTC time when the release was attempted.
    pub attempted_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Indexer search result with parsed release metadata, scoring, and queue eligibility.
pub struct IndexerSearchResultPayload {
    /// Indexer source name.
    pub source: String,
    /// Release title shown by the indexer.
    pub title: String,
    /// Informational release link, or null when unavailable.
    pub link: Option<String>,
    /// Direct download URL, or null when unavailable.
    pub download_url: Option<String>,
    /// Source kind such as Usenet or torrent, or null when unknown.
    pub source_kind: Option<DownloadSourceKindValue>,
    /// Release size in bytes, or null when unknown.
    pub size_bytes: Option<Long>,
    /// Publication time in UTC, or null when unavailable.
    pub published_at: Option<DateTime<Utc>>,
    /// Positive vote count, or null when not reported.
    pub thumbs_up: Option<i32>,
    /// Negative vote count, or null when not reported.
    pub thumbs_down: Option<i32>,
    /// Parsed release fields, or null when parsing did not produce a result.
    pub parsed_release: Option<ParsedReleasePayload>,
    /// Quality-profile decision, or null when no profile was evaluated.
    pub quality_profile_decision: Option<QualityProfileDecisionPayload>,
    // Torrent-specific fields
    /// Torrent seeder count, or null for non-torrent results.
    pub seeders: Option<i32>,
    /// Torrent peer count, or null for non-torrent results.
    pub peers: Option<i32>,
    /// Torrent info hash, or null for non-torrent results.
    pub info_hash: Option<String>,
    /// Whether the torrent is marked freeleech, or null for non-torrent results.
    pub freeleech: Option<bool>,
    /// Torrent download volume factor, or null for non-torrent results.
    pub download_volume_factor: Option<f64>,
    /// Opaque token accepted when queuing this candidate, or null when unavailable.
    pub candidate_token: Option<String>,
    /// Acquisition scope targeted by this result, or null when no scope was inferred.
    pub queue_scope: Option<QueueDownloadScopePayload>,
    /// Whether automatic acquisition may select this result.
    pub auto_eligible: Option<bool>,
    /// Automatic acquisition decision code, or null when not evaluated.
    pub auto_decision_code: Option<String>,
    /// Human-readable automatic acquisition decision summary, or null when not evaluated.
    pub auto_decision_summary: Option<String>,
}

/// Acquisition scope targeted by a queued download.
#[derive(async_graphql::Union, Clone)]
pub enum QueueDownloadScopePayload {
    /// A single episode target.
    Episode(EpisodeScopePayload),
    /// A set of episode targets.
    EpisodeSet(EpisodeSetScopePayload),
    /// A series-movie link target.
    SeriesMovie(SeriesMovieScopePayload),
    /// A collection target.
    Collection(CollectionScopePayload),
    /// An entire-title target.
    Title(TitleScopePayload),
    /// A queued download with no known acquisition scope.
    Orphan(OrphanScopePayload),
}

impl QueueDownloadScopePayload {
    pub fn episode(episode_id: ID) -> Self {
        Self::Episode(EpisodeScopePayload { episode_id })
    }
}

#[derive(SimpleObject, Clone)]
/// Union member identifying one episode by ID.
pub struct EpisodeScopePayload {
    /// Target episode ID.
    pub episode_id: ID,
}

#[derive(SimpleObject, Clone)]
/// Union member identifying multiple episodes by ID.
pub struct EpisodeSetScopePayload {
    /// Target episode IDs; empty means no episode scope was supplied.
    pub episode_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Union member identifying a series-movie relationship by ID.
pub struct SeriesMovieScopePayload {
    /// Target series-movie link ID.
    pub series_movie_link_id: ID,
}

#[derive(SimpleObject, Clone)]
/// Union member identifying a collection by ID.
pub struct CollectionScopePayload {
    /// Target collection ID.
    pub collection_id: ID,
}

#[derive(SimpleObject, Clone)]
/// Union member indicating the entire title is the acquisition scope.
pub struct TitleScopePayload {
    /// Always true when this marker is emitted.
    pub whole_title: bool,
}

#[derive(SimpleObject, Clone)]
/// Union member indicating that no known acquisition scope is attached.
pub struct OrphanScopePayload {
    /// Always true when this marker is emitted.
    pub orphaned: bool,
}

#[derive(SimpleObject, Clone)]
/// Parsed episode numbering extracted from a release title.
pub struct ParsedEpisodePayload {
    /// Season number, or null when no season was parsed.
    pub season: Option<i32>,
    /// Parsed episode numbers; empty means none were detected.
    pub episode_numbers: Vec<i32>,
}

#[derive(SimpleObject, Clone)]
/// Parsed release title metadata and parser confidence.
pub struct ParsedReleasePayload {
    /// Original release title before normalization.
    pub raw_title: String,
    /// Normalized title used for matching.
    pub normalized_title: String,
    /// Release group, or null when not detected.
    pub release_group: Option<String>,
    /// Quality label, or null when not detected.
    pub quality: Option<String>,
    /// Source label, or null when not detected.
    pub source: Option<String>,
    /// Video codec, or null when not detected.
    pub video_codec: Option<String>,
    /// Video encoding, or null when not detected.
    pub video_encoding: Option<String>,
    /// Audio description, or null when not detected.
    pub audio: Option<String>,
    /// Whether dual audio was detected.
    pub is_dual_audio: bool,
    /// Whether Atmos audio was detected.
    pub is_atmos: bool,
    /// Whether Dolby Vision was detected.
    pub is_dolby_vision: bool,
    /// Whether HDR was detected.
    pub detected_hdr: bool,
    /// Whether the release is marked as a proper upload.
    pub is_proper_upload: bool,
    /// Whether the release is a remux.
    pub is_remux: bool,
    /// Whether the release is a Blu-ray disc image.
    pub is_bd_disk: bool,
    /// Whether an AI-enhanced marker was detected.
    pub is_ai_enhanced: bool,
    /// Parser confidence on the service's 0 through 1 scale.
    pub parse_confidence: f32,
    /// Parser hints retained for diagnostics.
    pub parse_hints: Vec<String>,
    /// Parsed episode numbering, or null for non-episodic releases.
    pub episode: Option<ParsedEpisodePayload>,
}

#[derive(SimpleObject, Clone)]
/// One scoring rule contribution used in a quality decision.
pub struct ScoringEntryPayload {
    /// Stable scoring rule code.
    pub code: String,
    /// Signed score delta contributed by the rule.
    pub delta: i32,
    /// Source of the scoring rule.
    pub source: String,
    /// Rule-set name, or null when not associated with a named set.
    pub rule_set_name: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Quality-profile acceptance and score breakdown for a release.
pub struct QualityProfileDecisionPayload {
    /// Whether the release passed the profile decision.
    pub allowed: bool,
    /// Blocking rule codes that prevented acceptance.
    pub block_codes: Vec<String>,
    /// Aggregate release score.
    pub release_score: i32,
    /// Aggregate preference score.
    pub preference_score: i32,
    /// Ordered scoring contributions used to compute the decision.
    pub scoring_log: Vec<ScoringEntryPayload>,
}

#[derive(SimpleObject, Clone)]
/// Provider configuration field metadata and its masked stored value.
pub struct ProviderConfigValuePayload {
    /// Stable provider configuration key.
    pub key: String,
    /// Human-readable field label, or null when the provider supplies none.
    pub label: Option<String>,
    /// Configuration field type, or null when unspecified.
    pub field_type: Option<PluginConfigFieldTypeValue>,
    /// Whether a value is required by the provider.
    pub required: bool,
    /// Provider default value, or null when no default exists.
    pub default_value: Option<String>,
    /// Source of the effective value, or null when no source is recorded.
    pub value_source: Option<PluginConfigValueSourceValue>,
    /// Semantic role of the field, or null when unspecified.
    pub role: Option<PluginConfigFieldRoleValue>,
    /// Host binding required by the field, or null when not applicable.
    pub host_binding: Option<String>,
    /// Allowed field options; empty means the field has no enumerated options.
    pub options: Vec<PluginConfigFieldOptionPayload>,
    /// Provider help text, or null when unavailable.
    pub help_text: Option<String>,
    /// The stored value as a typed union; null when the field is unset.
    pub value: Option<ProviderConfigFieldValue>,
}

/// Typed provider configuration value; secret variants expose presence but not plaintext.
#[derive(async_graphql::Union, Clone)]
pub enum ProviderConfigFieldValue {
    /// Stored string value.
    String(StringConfigValuePayload),
    /// Stored boolean value.
    Bool(BoolConfigValuePayload),
    /// Stored signed integer value.
    Int(IntConfigValuePayload),
    /// Stored floating-point value.
    Float(FloatConfigValuePayload),
    /// Secret presence marker without the secret contents.
    Secret(SecretConfigValuePayload),
}

#[derive(SimpleObject, Clone)]
/// Stored string provider configuration value.
pub struct StringConfigValuePayload {
    /// String value.
    pub value: String,
}

#[derive(SimpleObject, Clone)]
/// Stored boolean provider configuration value.
pub struct BoolConfigValuePayload {
    /// Boolean value.
    pub value: bool,
}

#[derive(SimpleObject, Clone)]
/// Stored integer provider configuration value.
pub struct IntConfigValuePayload {
    /// Signed integer value.
    pub value: i64,
}

#[derive(SimpleObject, Clone)]
/// Stored floating-point provider configuration value.
pub struct FloatConfigValuePayload {
    /// Floating-point value.
    pub value: f64,
}

#[derive(SimpleObject, Clone)]
/// Secret provider configuration value represented without plaintext.
pub struct SecretConfigValuePayload {
    /// True when a secret is stored; false means absent or cleared.
    pub stored: bool,
}

#[derive(InputObject, Clone)]
/// One provider configuration update, with typed value slots and explicit secret clearing.
pub struct ProviderConfigValueInput {
    /// Provider configuration key being written.
    pub key: String,
    /// String value slot; null means this slot is not selected.
    pub string_value: Option<String>,
    /// Boolean value slot; null means this slot is not selected.
    pub bool_value: Option<bool>,
    /// Signed integer value slot; null means this slot is not selected.
    pub int_value: Option<i64>,
    /// Floating-point value slot; null means this slot is not selected.
    pub float_value: Option<f64>,
    /// New secret value; it is accepted for writing but never returned in payloads.
    pub secret_value: Option<String>,
    /// When true, clear the stored secret instead of returning or preserving it.
    pub clear_secret: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Indexer configuration, health, capability, routing, and masked secret metadata.
pub struct IndexerConfigPayload {
    /// Indexer configuration ID.
    pub id: ID,
    /// User-facing indexer name.
    pub name: String,
    /// Stable provider implementation key for this indexer.
    pub provider_type: String,
    /// Indexer base URL.
    pub base_url: String,
    /// Optional proxy configuration ID used by this indexer.
    pub proxy_config_id: Option<ID>,
    /// Optional download-client ID associated with this indexer.
    pub download_client_id: Option<ID>,
    /// Optional seeding profile ID applied to torrents grabbed from this indexer.
    pub seeding_profile_id: Option<ID>,
    /// Whether Prowlarr supplied seed criteria for this managed child. When it
    /// did and no seeding profile is assigned, those criteria apply; assigning
    /// a profile overrides them.
    pub has_prowlarr_seed_criteria: bool,
    /// Minimum seeders Prowlarr imported for this managed child, or null when it
    /// supplied none. Zero means Prowlarr turned the seeder check off. Read-only:
    /// it is edited in Prowlarr, and an assigned seeding profile overrides it.
    pub prowlarr_minimum_seeders: Option<i32>,
    /// Whether an API key is configured without exposing it.
    pub has_api_key: bool,
    /// Whether this configuration is managed by a parent configuration.
    pub is_managed: bool,
    /// Parent managed configuration ID, or null when independent.
    pub managed_parent_config_id: Option<ID>,
    /// Whether managed child synchronization is supported.
    pub supports_managed_children_sync: bool,
    /// Names of stored secret fields, never their values.
    pub stored_secret_keys: Vec<String>,
    /// Minimum interval between requests in seconds, or null when unlimited.
    pub rate_limit_seconds: Option<i64>,
    /// Maximum burst size for rate limiting, or null when unspecified.
    pub rate_limit_burst: Option<i64>,
    /// UTC time until which the indexer is disabled, or null when not disabled.
    pub disabled_until: Option<DateTime<Utc>>,
    /// Whether the indexer is enabled.
    pub is_enabled: bool,
    /// Whether interactive searches are enabled.
    pub enable_interactive_search: bool,
    /// Whether automatic searches are enabled.
    pub enable_auto_search: bool,
    /// Most recent health status, or null before the first check.
    pub last_health_status: Option<String>,
    /// Most recent health error, or null when none is recorded.
    pub last_error_message: Option<String>,
    /// UTC time of the most recent health error, or null when none is recorded.
    pub last_error_at: Option<DateTime<Utc>>,
    /// UTC time of the most recent query, or null before the first query.
    pub last_query_at: Option<DateTime<Utc>>,
    /// Provider configuration fields with secret values masked.
    pub config: Vec<ProviderConfigValuePayload>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Available indexers, download clients, and provider compatibility mappings.
pub struct IndexerDownloadClientMappingCatalogPayload {
    /// Download clients available for mapping.
    pub clients: Vec<IndexerDownloadClientMappingClientPayload>,
    /// Indexers and their current mapping state.
    pub indexers: Vec<IndexerDownloadClientMappingIndexerPayload>,
    /// Provider-level compatibility information used when no concrete indexer exists.
    pub provider_compatibility: Vec<IndexerDownloadClientProviderCompatibilityPayload>,
}

#[derive(SimpleObject, Clone)]
/// Download client option used by the indexer mapping catalog.
pub struct IndexerDownloadClientMappingClientPayload {
    /// Download client ID.
    pub id: ID,
    /// Download client name.
    pub name: String,
    /// Download client provider type.
    pub client_type: String,
    /// Whether the client is enabled.
    pub is_enabled: bool,
    /// Current health status string.
    pub health_status: String,
}

#[derive(SimpleObject, Clone)]
/// Indexer mapping state and compatible download-client IDs.
pub struct IndexerDownloadClientMappingIndexerPayload {
    /// Indexer configuration ID.
    pub id: ID,
    /// Indexer name.
    pub name: String,
    /// Currently mapped download-client ID, or null when unmapped.
    pub download_client_id: Option<ID>,
    /// Protocol families supported by the indexer.
    pub protocol_families: Vec<String>,
    /// Whether this indexer supports explicit client mapping.
    pub supports_mapping: bool,
    /// Download-client IDs compatible with the indexer.
    pub compatible_client_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Provider-level protocol compatibility and compatible download clients.
pub struct IndexerDownloadClientProviderCompatibilityPayload {
    /// Stable indexer provider implementation key.
    pub provider_type: String,
    /// Protocol families supported by the provider.
    pub protocol_families: Vec<String>,
    /// Whether the provider supports explicit mapping.
    pub supports_mapping: bool,
    /// Compatible download-client IDs.
    pub compatible_client_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Proxy configuration and latest health state.
pub struct ProxyConfigPayload {
    /// Proxy configuration ID.
    pub id: ID,
    /// Proxy configuration name.
    pub name: String,
    /// Proxy provider type.
    pub provider_type: String,
    /// Challenge-solver protocol, or null for transport proxies, which speak
    /// no protocol of their own.
    pub protocol: Option<String>,
    /// Proxy base URL.
    pub base_url: String,
    /// Request timeout in seconds.
    pub request_timeout_seconds: i32,
    /// Whether a username or password is stored for this proxy, without
    /// exposing either value. Always false for challenge solvers, which take
    /// no credentials.
    pub has_credentials: bool,
    /// Whether destination hostnames are resolved at the proxy (`socks5h`).
    /// Always false outside SOCKS5.
    pub remote_dns: bool,
    /// Whether a private key is stored for this tunnel, without exposing it.
    /// Always false outside the tunnel providers.
    pub has_private_key: bool,
    /// WireGuard peer public key, from the `[Peer]` section, or null outside
    /// WireGuard. A public key is public, so this value is shown rather than
    /// masked.
    pub peer_public_key: Option<String>,
    /// Whether a WireGuard preshared key is stored, without exposing it.
    /// Always false outside WireGuard.
    pub has_preshared_key: bool,
    /// This tunnel's own public key, derived from its private key, or null
    /// when no private key is stored. It is the line the operator must paste
    /// into the server's `[Peer]` section, so it is shown rather than masked.
    pub tunnel_public_key: Option<String>,
    /// WireGuard interface addresses, from the `[Interface] Address` line.
    /// Empty outside WireGuard.
    pub tunnel_addresses: Vec<String>,
    /// WireGuard resolvers reached through the tunnel, from the
    /// `[Interface] DNS` line. Empty when none are configured.
    pub tunnel_dns_servers: Vec<String>,
    /// WireGuard tunnel MTU, or null to use the engine's default.
    pub tunnel_mtu: Option<i32>,
    /// WireGuard persistent keepalive in seconds, or null to use the engine's
    /// default. Zero means keepalive is switched off.
    pub tunnel_keepalive_seconds: Option<i32>,
    /// Host key pinned on the first successful tunnel connect, formatted as
    /// OpenSSH prints it, or null before the first connect. A host key is
    /// public, so this value is shown rather than masked.
    pub host_key_fingerprint: Option<String>,
    /// UTC time the host key above was pinned, or null when none is pinned.
    pub host_key_pinned_at: Option<DateTime<Utc>>,
    /// Whether the proxy is enabled.
    pub is_enabled: bool,
    /// Most recent health status, or null before the first check.
    pub last_health_status: Option<String>,
    /// Most recent health error, or null when none is recorded.
    pub last_error_message: Option<String>,
    /// UTC time of the most recent health error, or null when none is recorded.
    pub last_error_at: Option<DateTime<Utc>>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Result of testing a proxy connection.
pub struct ProxyTestResultPayload {
    /// Whether the connection test succeeded.
    pub ok: bool,
    /// Machine-readable test status.
    pub status: String,
    /// Optional diagnostic message.
    pub message: Option<String>,
    /// Test duration in milliseconds, or null when unavailable.
    pub duration_ms: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// IDs created, updated, and deleted while synchronizing managed indexer configurations.
pub struct IndexerConfigSyncPayload {
    /// Parent configuration ID used for synchronization.
    pub parent_config_id: ID,
    /// IDs created by synchronization.
    pub created_ids: Vec<ID>,
    /// IDs updated by synchronization.
    pub updated_ids: Vec<ID>,
    /// IDs deleted by synchronization.
    pub deleted_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Download-client configuration with provider metadata and masked secret fields.
pub struct DownloadClientConfigPayload {
    /// Download-client configuration ID.
    pub id: ID,
    /// Download-client name.
    pub name: String,
    /// Download-client provider type.
    pub client_type: String,
    /// Base URL, or null for clients without a URL.
    pub base_url: Option<String>,
    /// Provider configuration fields with secrets masked.
    pub config: Vec<ProviderConfigValuePayload>,
    /// Names of stored secret fields, never their values.
    pub stored_secret_keys: Vec<String>,
    /// Whether the client is enabled.
    pub is_enabled: bool,
    /// Current client status.
    pub status: String,
    /// Most recent error, or null when none is recorded.
    pub last_error: Option<String>,
    /// UTC time the client was last observed, or null before the first observation.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Proxy carrying this client's traffic, or null when none is assigned.
    /// Any proxy kind may be assigned. A challenge solver has no effect on a
    /// native client, whose requests are not made by a plugin guest.
    pub proxy_config_id: Option<ID>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

/// How a seeding profile treats season packs relative to its own goals.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum SeasonPackSeedModeValue {
    /// Season packs use the profile's own ratio and seed-time goals.
    Inherit,
    /// Season packs use the profile's season-pack goals instead.
    Override,
}

/// What happens to a torrent once its seeding goal is met.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum SeedGoalMetActionValue {
    /// Remove the entry from the download client.
    RemoveEntry,
    /// Stop seeding but keep the entry in the download client.
    StopSeeding,
    /// Leave the torrent alone.
    Keep,
}

/// Whether Scryer keeps managing a torrent after it has been imported.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PostImportTrackingValue {
    /// Keep managing the torrent: hold the entry while it seeds, then apply the goal-met action.
    Park,
    /// Stop managing the torrent after import; the entry stays untouched and leaves the queue.
    HandOff,
}

#[derive(SimpleObject, Clone)]
/// Named torrent seeding policy assignable to indexers, routing entries, or the global default.
pub struct SeedingProfilePayload {
    /// Seeding profile ID.
    pub id: ID,
    /// Seeding profile name.
    pub name: String,
    /// Share ratio goal, or null to defer to the download client's own limits.
    pub ratio: Option<f64>,
    /// Seed time goal in minutes, or null to defer to the download client's own limits.
    pub seed_time_minutes: Option<i64>,
    /// Whether season packs inherit or override the profile's goals.
    pub season_pack_mode: SeasonPackSeedModeValue,
    /// Season-pack share ratio goal, or null when unset.
    pub season_pack_ratio: Option<f64>,
    /// Season-pack seed time goal in minutes, or null when unset.
    pub season_pack_seed_time_minutes: Option<i64>,
    /// Whether resolved goals are raised to tracker-declared minimums.
    pub honor_tracker_minimums: bool,
    /// Action taken once the seeding goal is met.
    pub goal_met_action: SeedGoalMetActionValue,
    /// Whether torrents grabbed under this profile are never auto-removed.
    pub never_remove: bool,
    /// Fewest seeders a candidate may report and still be grabbed. Null inherits the system floor; 0 disables the check.
    pub minimum_seeders: Option<i32>,
    /// Whether Scryer keeps managing torrents grabbed under this profile after import.
    pub post_import_tracking: PostImportTrackingValue,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Subtitle-provider configuration and health state without exposing secret values.
pub struct SubtitleProviderConfigPayload {
    /// Subtitle-provider configuration ID.
    pub id: ID,
    /// Provider configuration name.
    pub name: String,
    /// Subtitle-provider type identifier.
    pub provider_type: String,
    /// Whether provider configuration exists.
    pub has_config: bool,
    /// Names of stored secret fields, never their values.
    pub stored_secret_keys: Vec<String>,
    /// Media facets enabled for this provider.
    pub enabled_facets: Vec<MediaFacetValue>,
    /// Whether the provider is enabled.
    pub is_enabled: bool,
    /// Most recent health status, or null before the first check.
    pub last_health_status: Option<String>,
    /// Most recent error, or null when none is recorded.
    pub last_error: Option<String>,
    /// UTC time of the most recent error, or null when none is recorded.
    pub last_error_at: Option<DateTime<Utc>>,
    /// UTC time until which the provider is disabled, or null when not disabled.
    pub disabled_until: Option<DateTime<Utc>>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(InputObject)]
/// Filters for an interactive release search.
pub struct SearchReleasesInput {
    /// Title identity whose releases are searched.
    pub title_id: ID,
    /// Optional series/movie link identity for an episodic movie target.
    pub series_movie_link_id: Option<ID>,
    /// Optional season label or number to search.
    pub season: Option<String>,
    /// Optional episode label or number to search.
    pub episode: Option<String>,
    /// Optional result limit; the resolver applies its own default and cap.
    pub limit: Option<i32>,
}

#[derive(InputObject)]
/// Signed-candidate submission choices for an existing title.
pub struct QueueDownloadInput {
    /// Title identity receiving the queued release.
    pub title_id: ID,
    /// Signed token identifying and authorizing the candidate release.
    pub candidate_token: String,
    /// Indexer-announced release size, checked against the signed candidate when present.
    pub size_bytes: Option<Long>,
    /// Acquisition scope targeted by the submission.
    pub scope: QueueDownloadScopeInput,
    /// Whether an in-progress submission may be replaced; omission uses the resolver default.
    pub replace_in_progress: Option<bool>,
    /// Submission purpose; omission uses the normal download purpose.
    pub purpose: Option<QueueDownloadPurposeValue>,
}

#[derive(InputObject)]
/// Scope and replacement choices for selecting the best release.
pub struct QueueBestReleaseInput {
    /// Title identity whose best release is selected.
    pub title_id: ID,
    /// Acquisition scope targeted by the selection.
    pub scope: QueueDownloadScopeInput,
    /// Whether an in-progress submission may be replaced; omission uses the resolver default.
    pub replace_in_progress: Option<bool>,
}

#[derive(InputObject)]
/// Scope filters for a background acquisition search.
pub struct TriggerAcquisitionSearchInput {
    /// Wanted category to search, defaulting to missing items.
    pub wanted_kind: Option<WantedKindValue>,
    /// Optional facet restriction.
    pub facet: Option<MediaFacetValue>,
    /// Optional library identities to include.
    pub library_ids: Option<Vec<ID>>,
    /// Optional title identity to include.
    pub title_id: Option<ID>,
    /// Optional season number for the selected title.
    pub season_number: Option<i32>,
    /// Optional wanted-scope identity for searching exactly one scope.
    pub wanted_item_id: Option<ID>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Whether a download submission was accepted or conflicted with existing work.
pub enum QueueDownloadResultStatusValue {
    /// The release was accepted and queued.
    Queued,
    /// The release was not queued because the target scope conflicted.
    Conflict,
}

#[derive(SimpleObject, Clone)]
/// Existing download work that prevented a new submission.
pub struct QueueDownloadConflictPayload {
    /// Title identity involved in the conflict.
    pub title_id: ID,
    /// Current title name.
    pub title_name: String,
    /// Download-client identity, null when the conflict is not tied to a configured client.
    pub download_client_id: Option<ID>,
    /// Download-client provider type.
    pub download_client_type: String,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
    /// Release title already associated with the conflict, when known.
    pub source_title: Option<String>,
    /// Release source category, when known.
    pub source_kind: Option<DownloadSourceKindValue>,
    /// Acquisition scope occupied by the conflicting work.
    pub scope: QueueDownloadScopePayload,
    /// Current queue state, when available.
    pub state: Option<DownloadQueueStateValue>,
    /// Whether the conflicting work may be replaced.
    pub replaceable: bool,
}

#[derive(SimpleObject, Clone)]
/// Result of a release queue request.
pub struct QueueDownloadPayload {
    /// Whether the request was queued or conflicted.
    pub status: QueueDownloadResultStatusValue,
    /// Background job identity when the request was queued.
    pub job_id: Option<ID>,
    /// Title identity receiving the request.
    pub title_id: ID,
    /// Current title name.
    pub title_name: String,
    /// Queued release title, null when the request conflicted.
    pub source_title: Option<String>,
    /// Queued release source category, null when unavailable or conflicted.
    pub source_kind: Option<DownloadSourceKindValue>,
    /// Conflict details when the request was not queued.
    pub conflict: Option<QueueDownloadConflictPayload>,
}

#[derive(SimpleObject, Clone)]
/// Counts reported for a wanted search attempt.
pub struct WantedSearchPayload {
    /// Number of scopes queued for search.
    pub queued_count: i32,
    /// Number of scopes skipped because work was already in progress.
    pub skipped_in_progress_count: i32,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Action represented by a download queue mutation response.
pub enum DownloadQueueActionKindValue {
    /// A manual import was queued.
    QueuedManualImport,
    /// A tracked download was marked ignored.
    IgnoredTrackedDownload,
    /// A tracked download was marked failed.
    MarkedTrackedDownloadFailed,
    /// A title was assigned to a tracked download.
    AssignedTrackedDownloadTitle,
    /// A tracked download was paused.
    Paused,
    /// A tracked download was resumed.
    Resumed,
    /// A queued download deletion command was issued.
    DeleteQueued,
    /// A tracked download was deleted.
    Deleted,
}

#[derive(InputObject)]
/// Configuration for a new indexer provider.
pub struct CreateIndexerConfigInput {
    /// Display name of the indexer.
    pub name: String,
    /// Provider implementation identifier.
    pub provider_type: String,
    /// Optional proxy configuration identity.
    pub proxy_config_id: Option<ID>,
    /// Optional download-client identity used for routed grabs.
    pub download_client_id: Option<ID>,
    /// Rate-limit interval in seconds.
    pub rate_limit_seconds: Option<i64>,
    /// Maximum requests allowed in one rate-limit burst.
    pub rate_limit_burst: Option<i64>,
    /// Whether the indexer is enabled.
    pub is_enabled: Option<bool>,
    /// Whether interactive searches use this indexer.
    pub enable_interactive_search: Option<bool>,
    /// Whether automatic searches use this indexer.
    pub enable_auto_search: Option<bool>,
    /// Provider configuration values, including secret fields.
    pub config: Option<Vec<ProviderConfigValueInput>>,
}

#[derive(InputObject)]
/// Patch for an existing indexer provider.
pub struct UpdateIndexerConfigInput {
    /// Indexer configuration identity to patch.
    pub id: ID,
    /// Replacement display name; omission preserves the current value.
    pub name: Option<String>,
    /// Replacement provider implementation; omission preserves the current value.
    pub provider_type: Option<String>,
    /// Proxy identity: omission preserves it, null clears it, and a value replaces it.
    pub proxy_config_id: MaybeUndefined<ID>,
    /// Download-client identity: omission preserves it, null clears it, and a value replaces it.
    pub download_client_id: MaybeUndefined<ID>,
    /// Replacement rate-limit interval in seconds; omission preserves it.
    pub rate_limit_seconds: Option<i64>,
    /// Replacement rate-limit burst; omission preserves it.
    pub rate_limit_burst: Option<i64>,
    /// Replacement enabled state; omission preserves it.
    pub is_enabled: Option<bool>,
    /// Replacement interactive-search state; omission preserves it.
    pub enable_interactive_search: Option<bool>,
    /// Replacement automatic-search state; omission preserves it.
    pub enable_auto_search: Option<bool>,
    /// Replacement provider configuration; omitted secret fields retain stored secrets.
    pub config: Option<Vec<ProviderConfigValueInput>>,
}

#[derive(InputObject)]
/// Download-client mapping for one indexer.
pub struct SetIndexerDownloadClientMappingInput {
    /// Indexer identity to update.
    pub indexer_id: ID,
    /// Download-client identity to assign, or null to clear the mapping.
    pub download_client_id: Option<ID>,
}

#[derive(InputObject)]
/// Configuration for a proxy provider.
pub struct CreateProxyConfigInput {
    /// Display name of the proxy.
    pub name: String,
    /// Proxy provider implementation identifier.
    pub provider_type: String,
    /// Challenge-solver protocol. Omit to take the single protocol Scryer
    /// speaks; transport proxies reject any value because they speak none.
    pub protocol: Option<String>,
    /// Proxy base URL.
    pub base_url: String,
    /// Request timeout in seconds.
    pub request_timeout_seconds: Option<i32>,
    /// Transport-proxy username. Write-only: stored encrypted and never read
    /// back. Challenge solvers reject it.
    pub username: Option<String>,
    /// Transport-proxy password. Write-only: stored encrypted and never read
    /// back. Requires a username; challenge solvers reject it.
    pub password: Option<String>,
    /// SOCKS5 only: resolve destination hostnames at the proxy. A `socks5h://`
    /// base URL implies true.
    pub remote_dns: Option<bool>,
    /// Private key for a tunnel provider: PEM for an SSH tunnel, base64 for
    /// WireGuard. Write-only: stored encrypted and never read back. An SSH
    /// tunnel needs either this or a password; WireGuard requires it.
    pub private_key: Option<String>,
    /// Passphrase protecting the private key above. Write-only: stored
    /// encrypted and never read back. Requires a private key, and only SSH
    /// tunnels accept one.
    pub private_key_passphrase: Option<String>,
    /// WireGuard peer public key, from the `[Peer]` section. Required for
    /// WireGuard and rejected by every other provider. Not a secret: it is
    /// read back in full.
    pub peer_public_key: Option<String>,
    /// Optional WireGuard preshared key. Write-only: stored encrypted and
    /// never read back. Only WireGuard accepts it.
    pub preshared_key: Option<String>,
    /// WireGuard interface addresses, from the `[Interface] Address` line.
    /// At least one is required for WireGuard; a single comma-separated entry
    /// is also accepted. Only WireGuard accepts them.
    pub tunnel_addresses: Option<Vec<String>>,
    /// WireGuard resolvers, from the `[Interface] DNS` line. Optional; without
    /// them a destination must be addressed by IP. Only WireGuard accepts
    /// them.
    pub tunnel_dns_servers: Option<Vec<String>>,
    /// WireGuard tunnel MTU. Omit to use the engine's default. Only WireGuard
    /// accepts it.
    pub tunnel_mtu: Option<i32>,
    /// WireGuard persistent keepalive in seconds. Omit to use the engine's
    /// default; zero switches keepalive off. Only WireGuard accepts it.
    pub tunnel_keepalive_seconds: Option<i32>,
    /// Whether the proxy is enabled.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Patch for a proxy configuration.
pub struct UpdateProxyConfigInput {
    /// Proxy configuration identity to patch.
    pub id: ID,
    /// Replacement display name; omission preserves it.
    pub name: Option<String>,
    /// Replacement base URL; omission preserves it.
    pub base_url: Option<String>,
    /// Replacement request timeout in seconds; omission preserves it.
    pub request_timeout_seconds: Option<i32>,
    /// Replacement transport-proxy username. Write-only: omission preserves
    /// the stored value, null clears it, and it is never read back.
    pub username: MaybeUndefined<String>,
    /// Replacement transport-proxy password. Write-only: omission preserves
    /// the stored value, null clears it, and it is never read back.
    pub password: MaybeUndefined<String>,
    /// Replacement SOCKS5 remote-DNS state; omission preserves it.
    pub remote_dns: Option<bool>,
    /// Replacement tunnel private key: PEM for an SSH tunnel, base64 for
    /// WireGuard. Write-only: omission preserves the stored value, null clears
    /// it, and it is never read back. Writing it re-derives `tunnelPublicKey`.
    pub private_key: MaybeUndefined<String>,
    /// Replacement private key passphrase. Write-only: omission preserves the
    /// stored value, null clears it, and it is never read back.
    pub private_key_passphrase: MaybeUndefined<String>,
    /// Replacement WireGuard peer public key; omission preserves it. It has no
    /// cleared state: a WireGuard tunnel cannot exist without one.
    pub peer_public_key: Option<String>,
    /// Replacement WireGuard preshared key. Write-only: omission preserves the
    /// stored value, null clears it, and it is never read back.
    pub preshared_key: MaybeUndefined<String>,
    /// Replacement WireGuard interface addresses; omission preserves them, and
    /// an empty list clears them.
    pub tunnel_addresses: Option<Vec<String>>,
    /// Replacement WireGuard resolvers; omission preserves them, and an empty
    /// list clears them.
    pub tunnel_dns_servers: Option<Vec<String>>,
    /// Replacement WireGuard MTU; omission preserves it and null restores the
    /// engine's default.
    pub tunnel_mtu: MaybeUndefined<i32>,
    /// Replacement WireGuard keepalive in seconds; omission preserves it, null
    /// restores the engine's default, and zero switches keepalive off.
    pub tunnel_keepalive_seconds: MaybeUndefined<i32>,
    /// Replacement enabled state; omission preserves it.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Seeding profile assignment for one indexer.
pub struct SetIndexerSeedingProfileInput {
    /// Indexer identity to update.
    pub indexer_id: ID,
    /// Seeding profile identity to assign, or null to clear the assignment.
    pub seeding_profile_id: Option<ID>,
}

#[derive(InputObject)]
/// Global default seeding profile assignment.
pub struct SetDefaultSeedingProfileInput {
    /// Seeding profile identity to use as the default, or null to clear it.
    pub seeding_profile_id: Option<ID>,
}

#[derive(InputObject)]
/// Replacement system-wide minimum-seeder floor.
pub struct SetMinimumSeedersFloorInput {
    /// Fewest seeders a torrent candidate may report when no seeding profile resolves; 0 disables the check.
    pub minimum_seeders_floor: i32,
}

#[derive(InputObject)]
/// Goals and removal behavior for a new seeding profile.
pub struct CreateSeedingProfileInput {
    /// Seeding profile name; must be unique.
    pub name: String,
    /// Share ratio goal, or null to defer to the download client's own limits.
    pub ratio: Option<f64>,
    /// Seed time goal in minutes, or null to defer to the download client's own limits.
    pub seed_time_minutes: Option<i64>,
    /// Whether season packs inherit or override the profile's goals; defaults to inherit.
    pub season_pack_mode: Option<SeasonPackSeedModeValue>,
    /// Season-pack share ratio goal; only applied in override mode.
    pub season_pack_ratio: Option<f64>,
    /// Season-pack seed time goal in minutes; only applied in override mode.
    pub season_pack_seed_time_minutes: Option<i64>,
    /// Whether resolved goals are raised to tracker-declared minimums; defaults to true.
    pub honor_tracker_minimums: Option<bool>,
    /// Action taken once the goal is met; defaults to removing the entry.
    pub goal_met_action: Option<SeedGoalMetActionValue>,
    /// Whether torrents grabbed under this profile are never auto-removed; defaults to false.
    pub never_remove: Option<bool>,
    /// Fewest seeders a candidate may report and still be grabbed. Null inherits the system floor; 0 disables the check.
    pub minimum_seeders: Option<i32>,
    /// Whether Scryer keeps managing torrents after import; defaults to parking them.
    pub post_import_tracking: Option<PostImportTrackingValue>,
}

#[derive(InputObject)]
/// Patch for a stored seeding profile; omitted fields are preserved and explicit nulls clear goals.
pub struct UpdateSeedingProfileInput {
    /// Seeding profile identity to patch.
    pub id: ID,
    /// Replacement name; omission preserves it.
    pub name: Option<String>,
    /// Replacement share ratio goal; null clears it and omission preserves it.
    pub ratio: MaybeUndefined<f64>,
    /// Replacement seed time goal in minutes; null clears it and omission preserves it.
    pub seed_time_minutes: MaybeUndefined<i64>,
    /// Replacement season-pack mode; omission preserves it.
    pub season_pack_mode: Option<SeasonPackSeedModeValue>,
    /// Replacement season-pack ratio goal; null clears it and omission preserves it.
    pub season_pack_ratio: MaybeUndefined<f64>,
    /// Replacement season-pack seed time goal in minutes; null clears it and omission preserves it.
    pub season_pack_seed_time_minutes: MaybeUndefined<i64>,
    /// Replacement tracker-minimum handling; omission preserves it.
    pub honor_tracker_minimums: Option<bool>,
    /// Replacement goal-met action; omission preserves it.
    pub goal_met_action: Option<SeedGoalMetActionValue>,
    /// Replacement never-remove flag; omission preserves it.
    pub never_remove: Option<bool>,
    /// Replacement minimum seeders; null restores the system floor and omission preserves it.
    pub minimum_seeders: MaybeUndefined<i32>,
    /// Replacement post-import tracking mode; omission preserves it.
    pub post_import_tracking: Option<PostImportTrackingValue>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a seeding profile.
pub struct DeleteSeedingProfilePayload {
    /// Deleted seeding profile identity.
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
/// Global default seeding profile after an assignment change.
pub struct DefaultSeedingProfilePayload {
    /// Seeding profile identity used as the default, or null when unset.
    pub seeding_profile_id: Option<ID>,
    /// Fewest seeders a torrent candidate may report when no seeding profile resolves.
    pub minimum_seeders_floor: i32,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a proxy.
pub struct DeleteProxyConfigPayload {
    /// Deleted proxy configuration identity.
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting an indexer configuration.
pub struct DeleteIndexerConfigPayload {
    /// Deleted indexer configuration identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Configuration for a new download client.
pub struct CreateDownloadClientConfigInput {
    /// Display name of the client.
    pub name: String,
    /// Download-client provider implementation identifier.
    pub client_type: String,
    /// Provider configuration values, including secret fields.
    pub config: Vec<ProviderConfigValueInput>,
    /// Proxy to carry this client's traffic. Any proxy kind is accepted, and
    /// the proxy must exist and be enabled.
    pub proxy_config_id: Option<ID>,
    /// Whether the client is enabled.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Patch for an existing download client.
pub struct UpdateDownloadClientConfigInput {
    /// Download-client configuration identity to patch.
    pub id: ID,
    /// Replacement display name; omission preserves it.
    pub name: Option<String>,
    /// Replacement provider implementation; omission preserves it.
    pub client_type: Option<String>,
    /// Replacement provider configuration; omitted secret fields retain stored secrets.
    pub config: Option<Vec<ProviderConfigValueInput>>,
    /// Replacement proxy assignment; omission preserves it and null clears it.
    pub proxy_config_id: MaybeUndefined<ID>,
    /// Replacement enabled state; omission preserves it.
    pub is_enabled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Result of deleting a download-client configuration.
pub struct DeleteDownloadClientConfigPayload {
    /// Deleted download-client identity.
    pub id: async_graphql::ID,
    /// Number of indexer mappings cleared as a consequence.
    pub cleared_indexer_mapping_count: i32,
}

#[derive(InputObject)]
/// New ordering for download-client configurations.
pub struct ReorderDownloadClientConfigsInput {
    /// Download-client identities in desired order.
    pub ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Persisted download-client configuration order.
pub struct ReorderDownloadClientConfigsPayload {
    /// Download-client identities in stored order.
    pub ids: Vec<ID>,
}

#[derive(InputObject)]
/// Connection details used to test a download client.
pub struct TestDownloadClientConnectionInput {
    /// Existing client identity, when testing a stored configuration.
    pub id: Option<ID>,
    /// Provider implementation identifier for an unsaved configuration.
    pub client_type: String,
    /// Provider configuration values used for the test.
    pub config: Vec<ProviderConfigValueInput>,
    /// Proxy to dial through for this test, so it exercises the same egress
    /// live traffic will use. Omit to test the client directly.
    pub proxy_config_id: Option<ID>,
}

#[derive(InputObject)]
/// Configuration for a new subtitle provider.
pub struct CreateSubtitleProviderConfigInput {
    /// Display name of the provider.
    pub name: String,
    /// Subtitle provider implementation identifier.
    pub provider_type: String,
    /// Provider configuration values, including secret fields.
    pub config: Vec<ProviderConfigValueInput>,
    /// Facets for which the provider is enabled.
    pub enabled_facets: Option<Vec<MediaFacetValue>>,
    /// Whether the provider is enabled.
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
/// Patch for an existing subtitle provider.
pub struct UpdateSubtitleProviderConfigInput {
    /// Subtitle-provider configuration identity to patch.
    pub id: ID,
    /// Replacement display name; omission preserves it.
    pub name: Option<String>,
    /// Replacement provider implementation; omission preserves it.
    pub provider_type: Option<String>,
    /// Replacement provider configuration; omitted secret fields retain stored secrets.
    pub config: Option<Vec<ProviderConfigValueInput>>,
    /// Replacement enabled facets; omission preserves them.
    pub enabled_facets: Option<Vec<MediaFacetValue>>,
    /// Replacement enabled state; omission preserves it.
    pub is_enabled: Option<bool>,
    /// Disable-until timestamp: omission preserves it, null clears it, and a value replaces it.
    pub disabled_until: MaybeUndefined<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a subtitle provider.
pub struct DeleteSubtitleProviderConfigPayload {
    /// Deleted subtitle-provider identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Connection details used to test a subtitle provider.
pub struct TestSubtitleProviderConnectionInput {
    /// Existing provider identity, when testing a stored configuration.
    pub id: Option<ID>,
    /// Provider implementation identifier for an unsaved configuration.
    pub provider_type: String,
    /// Provider configuration values used for the test.
    pub config: Vec<ProviderConfigValueInput>,
}

#[derive(InputObject)]
/// Connection details used to test an indexer provider.
pub struct TestIndexerConnectionInput {
    /// Provider implementation identifier.
    pub provider_type: String,
    /// Optional provider configuration values used for the test.
    pub config: Option<Vec<ProviderConfigValueInput>>,
    /// Existing indexer identity, when testing a stored configuration.
    pub indexer_id: Option<ID>,
    /// Proxy identity: omission preserves the stored association, null clears it, and a value replaces it.
    pub proxy_config_id: MaybeUndefined<ID>,
}

#[derive(InputObject)]
/// Identity of a download item to pause.
pub struct PauseDownloadInput {
    /// Download-client identity; null identifies the default or unscoped client behavior.
    pub client_id: Option<ID>,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
}

#[derive(InputObject)]
/// Identity of a download item to resume.
pub struct ResumeDownloadInput {
    /// Download-client identity; null identifies the default or unscoped client behavior.
    pub client_id: Option<ID>,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
}

#[derive(InputObject)]
/// Identity and history behavior for deleting a tracked download.
pub struct DeleteDownloadInput {
    /// Download-client identity, when the item is scoped to a configured client.
    pub client_id: Option<ID>,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
    /// Whether deletion should use the provider's history path.
    pub is_history: bool,
}

// --- Manual Import ---

#[derive(SimpleObject, Clone)]
/// Media facts obtained while qualifying a manual-import candidate.
pub struct ManualImportVideoFactsPayload {
    /// Detected container format, or null when unavailable.
    pub container_format: Option<String>,
    /// Detected video codec, or null when unavailable.
    pub video_codec: Option<String>,
    /// Detected audio codec, or null when unavailable.
    pub audio_codec: Option<String>,
    /// Detected video width in pixels, or null when unavailable.
    pub video_width: Option<i32>,
    /// Detected video height in pixels, or null when unavailable.
    pub video_height: Option<i32>,
    /// Detected runtime in seconds, or null when unavailable.
    pub duration_seconds: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Candidate file details used to preview a manual import selection.
pub struct ManualImportFilePreviewPayload {
    /// Candidate ID within the persisted manual-import selection; use it only with that selection.
    pub candidate_id: ID,
    /// Candidate file name.
    pub file_name: String,
    /// Candidate file size in bytes.
    pub size_bytes: Long,
    /// Media facts when the native content probe can identify the file.
    pub video_facts: Option<ManualImportVideoFactsPayload>,
    /// Parsed quality label, or null when unavailable.
    pub quality: Option<String>,
    /// Parsed season number, or null when unavailable.
    pub parsed_season: Option<i32>,
    /// Parsed episode numbers; empty means none were detected.
    pub parsed_episodes: Vec<i32>,
    /// Suggested episode ID, or null when no single suggestion is available.
    pub suggested_episode_id: Option<ID>,
    /// Label for the suggested episode, or null when no suggestion exists.
    pub suggested_episode_label: Option<String>,
    /// Suggested series-movie link, or null when this is not a grabbed series movie.
    pub suggested_series_movie_link_id: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Series-movie target candidate for a manual import.
pub struct ManualImportSeriesMovieTargetPayload {
    /// Series-movie link identity targeted by the candidate.
    pub series_movie_link_id: String,
    /// Movie title associated with the target.
    pub movie_title: String,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Runtime in minutes, or null when unavailable.
    pub runtime_minutes: Option<i32>,
}

#[derive(InputObject)]
/// Maps a persisted manual-import candidate to an episode, series-movie link, or title-level movie target.
pub struct ManualImportCandidateMappingInput {
    /// Candidate ID from the persisted manual-import selection.
    pub candidate_id: ID,
    /// Episode target ID for an episodic import; null for a series-movie or movie import.
    pub episode_id: Option<ID>,
    /// Series-movie link ID for a series-movie import; null for an episode or movie import.
    pub series_movie_link_id: Option<ID>,
}

// --- Wanted Items / Acquisition ---

#[derive(SimpleObject, Clone)]
/// Count of wanted items grouped by decision code.
pub struct DecisionCodeCountPayload {
    /// Decision code.
    pub code: String,
    /// Number of wanted items with this code.
    pub count: i64,
}

#[derive(SimpleObject, Clone)]
/// Count of wanted items grouped by lifecycle status.
pub struct WantedStatusCountPayload {
    /// Wanted-item status.
    pub status: WantedStatusValue,
    /// Number of items with this status.
    pub count: i64,
}

#[derive(SimpleObject, Clone)]
/// Count of pending releases grouped by lifecycle status.
pub struct PendingReleaseStatusCountPayload {
    /// Pending-release status.
    pub status: PendingReleaseStatusValue,
    /// Number of releases with this status.
    pub count: i64,
}

#[derive(SimpleObject, Clone)]
/// One cutoff-unmet target with its current and target quality tiers and convergence state.
pub struct CutoffUnmetItemPayload {
    /// Target title ID.
    pub title_id: ID,
    /// Title display name.
    pub title_name: String,
    /// Title slug, or null when unavailable.
    pub title_slug: Option<String>,
    /// Media facet containing the title.
    pub title_facet: MediaFacetValue,
    /// Library ID containing the title.
    pub library_id: ID,
    /// Library name, or null when unavailable.
    pub library_name: Option<String>,
    /// Library slug, or null when unavailable.
    pub library_slug: Option<String>,
    /// Episode ID for episodic targets, or null for title-level targets.
    pub episode_id: Option<ID>,
    /// Season number as parsed text, or null when unavailable.
    pub season_number: Option<String>,
    /// Episode number as parsed text, or null when unavailable.
    pub episode_number: Option<String>,
    /// Current quality tier.
    pub current_tier: String,
    /// Required target quality tier.
    pub target_tier: String,
    /// Convergence state for this upgrade scope.
    pub convergence_state: ConvergenceStateValue,
    /// Number of indexers covering the target.
    pub indexers_covered: i32,
    /// Number of indexers selected by routing.
    pub indexers_routed: i32,
}

#[derive(SimpleObject, Clone)]
/// One page of cutoff-unmet targets plus the full matching count.
pub struct CutoffUnmetTitlesPagePayload {
    /// Items in the requested page.
    pub items: Vec<CutoffUnmetItemPayload>,
    /// Total matching targets before pagination.
    pub total_count: i64,
    /// Whether more matching targets exist after this page.
    pub has_more: bool,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after pausing a wanted state row or convergence scope.
pub struct PauseWantedItemPayload {
    /// State-row ID or convergence scope key that was paused.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after resuming a wanted state row or convergence scope.
pub struct ResumeWantedItemPayload {
    /// State-row ID or convergence scope key that was resumed.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Result of queuing title-mismatch recovery searches.
pub struct TriggerTitleMismatchRecoverySearchPayload {
    /// Title ID searched.
    pub title_id: ID,
    /// Number of recovery searches accepted for background processing.
    pub queued_count: i32,
}

/// Lifecycle state of a background acquisition search.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum AcquisitionSearchJobStateValue {
    /// Search is still running.
    Running,
    /// Search completed successfully.
    Completed,
    /// Search was canceled before completion.
    Cancelled,
    /// Search failed.
    Failed,
}

#[derive(SimpleObject, Clone)]
/// Progress snapshot for a background acquisition search.
pub struct AcquisitionSearchJobPayload {
    /// Acquisition-search job ID.
    pub id: ID,
    /// Current job lifecycle state.
    pub state: AcquisitionSearchJobStateValue,
    /// Total search targets.
    pub total: i32,
    /// Number of targets processed.
    pub processed: i32,
    /// Number of releases grabbed.
    pub grabbed_count: i32,
    /// Number of target searches that failed.
    pub failed_count: i32,
    /// Title currently being processed, or null when idle or complete.
    pub current_title: Option<String>,
    /// UTC job start time.
    pub started_at: DateTime<Utc>,
    /// UTC completion time, or null while running.
    pub finished_at: Option<DateTime<Utc>>,
}

/// Lifecycle state of a background interactive release search.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum InteractiveReleaseSearchStateValue {
    /// Search is still running.
    Running,
    /// Search completed and its snapshot is final.
    Completed,
    /// Search was canceled before completion.
    Cancelled,
}

/// Per-indexer progress within an interactive release search.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum InteractiveReleaseSearchIndexerStatusValue {
    /// Indexer has not started.
    Pending,
    /// Indexer is being queried.
    Searching,
    /// Indexer returned successfully.
    Completed,
    /// Indexer query failed.
    Failed,
    /// Indexer was excluded from this search.
    Skipped,
}

#[derive(SimpleObject, Clone)]
/// Per-indexer progress and result count for an interactive release search.
pub struct InteractiveReleaseSearchIndexerPayload {
    /// Indexer configuration ID.
    pub indexer_id: ID,
    /// Indexer name.
    pub name: String,
    /// Current indexer lifecycle state.
    pub status: InteractiveReleaseSearchIndexerStatusValue,
    /// The indexer's own result count (before cross-indexer dedup).
    pub result_count: i32,
    /// Failure reason, or null when the indexer did not fail.
    pub failure_reason: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Pollable snapshot of an interactive release-search job with partial results.
pub struct InteractiveReleaseSearchPayload {
    /// Interactive release-search job ID.
    pub id: ID,
    /// Current search lifecycle state.
    pub state: InteractiveReleaseSearchStateValue,
    /// Scored, cross-indexer-deduped snapshot of the merged results so far.
    pub results: Vec<IndexerSearchResultPayload>,
    /// Per-indexer progress states.
    pub indexers: Vec<InteractiveReleaseSearchIndexerPayload>,
    /// UTC search start time.
    pub started_at: DateTime<Utc>,
    /// UTC completion time, or null while running.
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Result of requesting cancellation for an interactive release search.
pub struct CancelInteractiveReleaseSearchPayload {
    /// Interactive search job ID.
    pub id: ID,
    /// True when cancellation was accepted; false when the search was already terminal.
    pub accepted: bool,
}

#[derive(SimpleObject, Clone)]
/// Counts produced by one RSS synchronization pass.
pub struct RssSyncReportPayload {
    /// Number of releases fetched from feeds.
    pub releases_fetched: i32,
    /// Number of fetched releases matched to known titles.
    pub releases_matched: i32,
    /// Number of matched releases accepted for grabbing.
    pub releases_grabbed: i32,
    /// Number of matched releases held instead of grabbed.
    pub releases_held: i32,
}

#[derive(SimpleObject, Clone)]
/// Result of requesting an immediate grab for a pending release.
pub struct ForceGrabPendingReleasePayload {
    /// Pending release ID.
    pub id: async_graphql::ID,
    /// Whether the release was accepted for grabbing.
    pub grabbed: bool,
}

#[derive(SimpleObject, Clone)]
/// Identifier of a dismissed pending release.
pub struct DismissPendingReleasePayload {
    /// Pending release ID.
    pub id: async_graphql::ID,
}
