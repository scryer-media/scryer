use super::*;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SubmissionScope {
    Episode { episode_id: String },
    EpisodeSet { episode_ids: Vec<String> },
    SeriesMovie { series_movie_link_id: String },
    Collection { collection_id: String },
    Title,
    Orphan,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DownloadSubmissionPurpose {
    #[default]
    Standard,
    AdditionalFile,
    /// A release an operator chose directly (interactive Queue or a redeemed
    /// download token). It still runs import guards, but a guard failure is
    /// held for manual import instead of burning the release.
    OperatorQueued,
    /// A manually-queued release chosen by the operator to replace the existing
    /// primary file. On import it bypasses the required-audio gate (like a manual
    /// file-pick) and forces the upgrade/replace path regardless of score, so a
    /// known-correct lower-scored release can replace a mis-scored one.
    ManualReplacement,
}

impl DownloadSubmissionPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::AdditionalFile => "additional_file",
            Self::OperatorQueued => "operator_queued",
            Self::ManualReplacement => "manual_replacement",
        }
    }

    pub fn from_label(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "additional_file" => Self::AdditionalFile,
            "operator_queued" => Self::OperatorQueued,
            "manual_replacement" => Self::ManualReplacement,
            _ => Self::Standard,
        }
    }

    pub fn is_additional_file(self) -> bool {
        self == Self::AdditionalFile
    }

    /// A manual operator-chosen replacement for the primary file.
    pub fn is_manual_replacement(self) -> bool {
        self == Self::ManualReplacement
    }

    /// A release selected by an operator rather than a convergence lane.
    pub fn is_operator_queued(self) -> bool {
        self == Self::OperatorQueued
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaFileRole {
    #[default]
    Primary,
    Additional,
}

impl MediaFileRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Additional => "additional",
        }
    }

    pub fn from_label(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "additional" => Self::Additional,
            _ => Self::Primary,
        }
    }

    pub fn is_primary(self) -> bool {
        self == Self::Primary
    }

    pub fn is_additional(self) -> bool {
        self == Self::Additional
    }
}

impl SubmissionScope {
    pub fn from_persisted(
        title_id: &str,
        episode_id: Option<String>,
        collection_id: Option<String>,
        series_movie_link_id: Option<String>,
        episode_set_ids: Option<Vec<String>>,
    ) -> Self {
        if let Some(mut episode_ids) = episode_set_ids {
            episode_ids.retain(|episode_id| !episode_id.trim().is_empty());
            episode_ids.sort();
            episode_ids.dedup();
            if !episode_ids.is_empty() {
                return Self::EpisodeSet { episode_ids };
            }
        }

        if let Some(episode_id) = episode_id {
            return Self::Episode { episode_id };
        }

        if let Some(series_movie_link_id) = series_movie_link_id {
            return Self::SeriesMovie {
                series_movie_link_id,
            };
        }

        if let Some(collection_id) = collection_id {
            return Self::Collection { collection_id };
        }

        if title_id.trim().is_empty() {
            Self::Orphan
        } else {
            Self::Title
        }
    }

    pub fn episode_id(&self) -> Option<&str> {
        match self {
            Self::Episode { episode_id } => Some(episode_id.as_str()),
            _ => None,
        }
    }

    pub fn collection_id(&self) -> Option<&str> {
        match self {
            Self::Collection { collection_id } => Some(collection_id.as_str()),
            _ => None,
        }
    }

    pub fn series_movie_link_id(&self) -> Option<&str> {
        match self {
            Self::SeriesMovie {
                series_movie_link_id,
            } => Some(series_movie_link_id.as_str()),
            _ => None,
        }
    }

    pub fn persisted_episode_id(&self) -> Option<&str> {
        self.episode_id()
    }

    pub fn persisted_collection_id(&self) -> Option<&str> {
        self.collection_id()
    }

    pub fn persisted_series_movie_link_id(&self) -> Option<&str> {
        self.series_movie_link_id()
    }

    pub fn episode_ids(&self) -> Option<&[String]> {
        match self {
            Self::EpisodeSet { episode_ids } => Some(episode_ids.as_slice()),
            Self::Episode { episode_id } => Some(std::slice::from_ref(episode_id)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DownloadSubmission {
    /// Canonical identity allocated immediately before this client mutation.
    pub download_id: scryer_domain::download_identity::DownloadId,
    pub title_id: String,
    pub facet: String,
    pub download_client_id: Option<String>,
    pub download_client_type: String,
    pub download_client_item_id: String,
    pub source_hint: Option<String>,
    pub source_provider_id: Option<String>,
    pub source_provider_name: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: Option<String>,
    /// BitTorrent v1 infohash the indexer announced at grab time.
    ///
    /// The blocklist keys a failed torrent on its infohash rather than its
    /// name, because content identity is the same wherever the torrent came
    /// from. The failure path resolves that key off the submission, so the hint
    /// has to be persisted here — `seed_info_hash` only exists when a seeding
    /// profile applied. `None` for usenet and for rows written before the
    /// column, which blocklist by release name instead.
    pub info_hash: Option<String>,
    /// The size the indexer announced for this release, when it announced one.
    ///
    /// D18 scores an in-flight submission as a pseudo-incumbent, and size is a
    /// term in that score. Without it the queued release carries no size term
    /// while the candidate beside it does, so any candidate in a larger size
    /// band reads as an upgrade over an identical release already downloading.
    /// `None` on rows written before the column existed; a size-less queued
    /// release is then compared size-less, which is the honest reading.
    pub release_size_bytes: Option<i64>,
    pub request_signature: Option<String>,
    pub purpose: DownloadSubmissionPurpose,
    pub scope: SubmissionScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadSubmissionActorSnapshot {
    pub kind: scryer_domain::DomainEventActorKind,
    pub user_id: Option<String>,
    pub display_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadSubmissionIdentity {
    pub download_id: Option<String>,
}

/// Seeding goals as resolved at grab time and frozen onto the download
/// submission row. Persisting the resolution (rather than re-deriving it from
/// history at poll time, the way Sonarr does) means a torrent keeps the goals
/// it was grabbed under even if the profile is later edited or unassigned.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PersistedSeedGoals {
    pub seeding_profile_id: Option<String>,
    pub seed_goal_ratio: Option<f64>,
    pub seed_goal_seconds: Option<i64>,
    pub never_remove: bool,
    pub goal_met_action: Option<scryer_domain::SeedGoalMetAction>,
    /// Whether Scryer keeps managing this torrent after import. Defaults to
    /// `Park`, so a grab with no profile — and any row written before the
    /// column existed — keeps the fail-closed behaviour.
    pub post_import_tracking: scryer_domain::PostImportTracking,
    pub resolution_source: SeedGoalResolutionSource,
    /// Info hash the client accepted, lowercased. Lets the Tier-B evaluator
    /// look a goal up straight off an observed torrent.
    pub info_hash: Option<String>,
}

impl PersistedSeedGoals {
    pub fn has_goals(&self) -> bool {
        self.seed_goal_ratio.is_some() || self.seed_goal_seconds.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalDownloadIdentityDisposition {
    Requested,
    AdoptedExisting {
        download_id: scryer_domain::download_identity::DownloadId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClientJobLocator {
    pub client_id: Option<String>,
    pub client_type: String,
    pub item_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedManualImport {
    pub import_id: String,
    pub source_identity: ClientJobLocator,
}

impl ClientJobLocator {
    pub fn new(
        client_id: Option<&str>,
        client_type: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> Self {
        let client_id = client_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Self {
            client_id,
            client_type: client_type.as_ref().trim().to_ascii_lowercase(),
            item_id: item_id.as_ref().trim().to_string(),
        }
    }

    pub fn for_import_artifact(
        client_id: Option<&str>,
        client_type: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> Self {
        let mut identity = Self::new(client_id, client_type, item_id);
        identity.client_id = identity.client_id.map(|value| value.to_ascii_lowercase());
        identity
    }

    pub fn from_submission(submission: &DownloadSubmission) -> Self {
        Self::new(
            submission.download_client_id.as_deref(),
            &submission.download_client_type,
            &submission.download_client_item_id,
        )
    }

    pub fn has_client_id(&self) -> bool {
        self.client_id.is_some()
    }

    pub fn client_id_or_empty(&self) -> &str {
        self.client_id.as_deref().unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadOrigin {
    ScryerSubmission,
    ForeignObservation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadRecord {
    pub id: scryer_domain::download_identity::DownloadId,
    pub origin: DownloadOrigin,
    pub created_at: DateTime<Utc>,
    pub first_observed_at: Option<DateTime<Utc>>,
    pub last_observed_at: Option<DateTime<Utc>>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadClientBindingRecord {
    pub download_id: scryer_domain::download_identity::DownloadId,
    pub client_config_id: Option<String>,
    pub client_type_snapshot: Option<String>,
    pub client_name_snapshot: Option<String>,
    pub native_item_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// One finished download, read from the durable registry/submission rows rather
/// than from a client's live list.
///
/// Download history was projected only from the live tracked-download snapshot,
/// so a client that evicts finished jobs from its own list (rTorrent does)
/// erased history entries for downloads Scryer had already imported. These rows
/// are the persisted side of that projection: the canonical download, its
/// terminal tracked state, and the submission columns a history row is built
/// from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalDownloadHistoryRow {
    pub download_id: scryer_domain::download_identity::DownloadId,
    pub origin: DownloadOrigin,
    /// The persisted [`scryer_domain::TrackedDownloadState`] string. Only
    /// terminal states are returned.
    pub tracked_state: String,
    pub tracked_reason: Option<String>,
    pub tracked_detail: Option<String>,
    pub title_id: Option<String>,
    pub episode_id: Option<String>,
    pub facet: Option<String>,
    pub source_title: Option<String>,
    pub client_id: Option<String>,
    pub client_type: Option<String>,
    pub client_name: Option<String>,
    pub download_client_item_id: Option<String>,
    pub source_provider_name: Option<String>,
    pub size_bytes: Option<i64>,
    pub submitted_at: Option<DateTime<Utc>>,
    /// When the row last changed state; the history sort value.
    pub last_state_at: Option<DateTime<Utc>>,
}

/// One observed client job, keyed by the configured-client/native-item locator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedClientJob {
    pub locator: ClientJobLocator,
    pub wire_token: Option<String>,
    pub observed_name: Option<String>,
    pub observed_at: DateTime<Utc>,
}

/// Canonical identity selected for an observed client job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservationResolution {
    Resolved {
        download_id: scryer_domain::download_identity::DownloadId,
        newly_foreign: bool,
        attached: bool,
    },
    Conflict {
        token_id: scryer_domain::download_identity::DownloadId,
        binding_download_id: scryer_domain::download_identity::DownloadId,
    },
    BindingAlreadyEnded,
}

#[derive(Clone, Debug)]
pub struct SuccessfulGrabCommit {
    pub wanted_item_id: String,
    pub covered_wanted_item_ids: Vec<String>,
    pub grabbed_release: String,
    pub last_search_at: Option<String>,
    pub grabbed_pending_release_id: Option<String>,
    pub grabbed_at: Option<String>,
}

/// Per-file import outcome history for completion verification across passes.
#[derive(Clone, Debug)]
pub struct ImportArtifact {
    pub id: String,
    pub source_client_id: Option<String>,
    pub source_system: String,
    pub source_ref: String,
    pub import_id: Option<String>,
    pub relative_path: Option<String>,
    pub normalized_file_name: String,
    pub media_kind: String,
    pub title_id: Option<String>,
    pub episode_id: Option<String>,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub result: String,
    pub reason_code: Option<String>,
    pub imported_media_file_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ImportArtifact {
    pub fn source_identity(&self) -> ClientJobLocator {
        ClientJobLocator::for_import_artifact(
            self.source_client_id.as_deref(),
            &self.source_system,
            &self.source_ref,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedNzbRef {
    pub id: String,
    pub compressed_path: PathBuf,
    pub raw_size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct PendingStagedNzb {
    pub id: String,
    pub compressed_path: PathBuf,
    pub partial_path: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct IndexerConfigUpdate {
    pub id: String,
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub derived_base_url: Option<String>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub is_enabled: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_auto_search: Option<bool>,
    pub proxy_config_id: Option<Option<String>>,
    pub download_client_id: Option<Option<String>>,
    pub seeding_profile_id: Option<Option<String>>,
    pub managed_parent_config_id: Option<Option<String>>,
    pub managed_child_key: Option<Option<String>>,
    pub managed_metadata_json: Option<Option<String>>,
    pub caps_snapshot_json: Option<Option<String>>,
    pub config_json: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NewProxyConfig {
    pub name: String,
    pub provider_type: scryer_domain::ProxyProviderType,
    /// Only meaningful for challenge-solver providers; supplying one for a
    /// transport provider is rejected rather than silently dropped. `None` on
    /// a solver provider takes the single protocol Scryer speaks.
    pub protocol: Option<scryer_domain::ChallengeSolverProtocol>,
    pub base_url: String,
    pub request_timeout_seconds: Option<u32>,
    pub is_enabled: bool,
    /// Plaintext proxy credentials. Persisted encrypted at rest.
    pub username: Option<String>,
    pub password: Option<String>,
    /// SOCKS5 only: resolve destination hostnames at the proxy (`socks5h`).
    pub remote_dns: Option<bool>,
    /// Tunnel providers only: PEM-encoded private key, pasted by the operator
    /// and persisted encrypted at rest.
    pub private_key: Option<String>,
    /// Passphrase for `private_key`, when the key has one.
    pub private_key_passphrase: Option<String>,
    /// WireGuard only: the peer's base64 public key. Required for that
    /// provider and rejected for every other one. Not a secret — it is stored
    /// and read back in the clear.
    pub peer_public_key: Option<String>,
    /// WireGuard only: the optional symmetric preshared key, write-only like
    /// the private key.
    pub preshared_key: Option<String>,
    /// WireGuard only: the `[Interface] Address` entries. At least one is
    /// required for that provider.
    pub tunnel_addresses: Option<Vec<String>>,
    /// WireGuard only: the `[Interface] DNS` entries. May be empty.
    pub tunnel_dns_servers: Option<Vec<String>>,
    /// WireGuard only: tunnel MTU. `None` takes the engine's default.
    pub tunnel_mtu: Option<u16>,
    /// WireGuard only: `PersistentKeepalive`. `None` takes the engine's
    /// default; `Some(0)` switches it off.
    pub tunnel_keepalive_seconds: Option<u16>,
}

#[derive(Clone, Debug, Default)]
pub struct ProxyConfigUpdate {
    pub id: String,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub request_timeout_seconds: Option<u32>,
    pub is_enabled: Option<bool>,
    /// Write-only credentials, following the codebase's nested-`Option`
    /// patch convention: outer `None` leaves the stored secret untouched,
    /// `Some(None)` clears it, `Some(Some(value))` replaces it.
    pub username: Option<Option<String>>,
    pub password: Option<Option<String>>,
    pub remote_dns: Option<bool>,
    /// Tunnel key material, same write-only tri-state as the credentials.
    pub private_key: Option<Option<String>>,
    pub private_key_passphrase: Option<Option<String>>,
    /// WireGuard peer public key. A plain `Option` rather than the tri-state,
    /// because this one is not a secret and not optional: a WireGuard tunnel
    /// without it cannot exist, so there is no "clear it" to express. Omission
    /// keeps the stored value.
    pub peer_public_key: Option<String>,
    /// WireGuard preshared key, write-only tri-state: it really is optional,
    /// so clearing it is a thing an operator can mean.
    pub preshared_key: Option<Option<String>>,
    /// WireGuard interface addresses. Omission keeps the stored list; an empty
    /// vector clears it (and is then refused by validation for a WireGuard
    /// row, which needs at least one).
    pub tunnel_addresses: Option<Vec<String>>,
    /// WireGuard resolvers. Omission keeps the stored list; an empty vector
    /// clears it, which is legal — a tunnel may address destinations by IP.
    pub tunnel_dns_servers: Option<Vec<String>>,
    /// WireGuard MTU and keepalive, tri-state: omission keeps the stored
    /// value, `Some(None)` restores the engine's default.
    pub tunnel_mtu: Option<Option<u16>>,
    pub tunnel_keepalive_seconds: Option<Option<u16>>,
}

impl ProxyConfigUpdate {
    pub fn has_changes(&self) -> bool {
        self.name.is_some()
            || self.base_url.is_some()
            || self.request_timeout_seconds.is_some()
            || self.is_enabled.is_some()
            || self.username.is_some()
            || self.password.is_some()
            || self.remote_dns.is_some()
            || self.private_key.is_some()
            || self.private_key_passphrase.is_some()
            || self.peer_public_key.is_some()
            || self.preshared_key.is_some()
            || self.tunnel_addresses.is_some()
            || self.tunnel_dns_servers.is_some()
            || self.tunnel_mtu.is_some()
            || self.tunnel_keepalive_seconds.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyTestResult {
    pub ok: bool,
    pub status: scryer_domain::ProxyHealthStatus,
    pub message: Option<String>,
    pub duration_ms: Option<u64>,
}

impl IndexerConfigUpdate {
    pub fn has_changes(&self) -> bool {
        self.name.is_some()
            || self.provider_type.is_some()
            || self.derived_base_url.is_some()
            || self.rate_limit_seconds.is_some()
            || self.rate_limit_burst.is_some()
            || self.is_enabled.is_some()
            || self.enable_interactive_search.is_some()
            || self.enable_auto_search.is_some()
            || self.proxy_config_id.is_some()
            || self.download_client_id.is_some()
            || self.seeding_profile_id.is_some()
            || self.managed_parent_config_id.is_some()
            || self.managed_child_key.is_some()
            || self.managed_metadata_json.is_some()
            || self.caps_snapshot_json.is_some()
            || self.config_json.is_some()
    }
}

#[derive(Clone, Debug)]
pub struct NewSeedingProfile {
    pub name: String,
    pub ratio: Option<f64>,
    pub seed_time_minutes: Option<i64>,
    pub season_pack_mode: scryer_domain::SeasonPackSeedMode,
    pub season_pack_ratio: Option<f64>,
    pub season_pack_seed_time_minutes: Option<i64>,
    pub honor_tracker_minimums: bool,
    pub goal_met_action: scryer_domain::SeedGoalMetAction,
    pub never_remove: bool,
    pub minimum_seeders: Option<i32>,
    pub post_import_tracking: scryer_domain::PostImportTracking,
}

/// Patch for a stored seeding profile. Nullable goals use the
/// `Option<Option<_>>` convention: `None` preserves, `Some(None)` clears.
#[derive(Clone, Debug, Default)]
pub struct SeedingProfileUpdate {
    pub id: String,
    pub name: Option<String>,
    pub ratio: Option<Option<f64>>,
    pub seed_time_minutes: Option<Option<i64>>,
    pub season_pack_mode: Option<scryer_domain::SeasonPackSeedMode>,
    pub season_pack_ratio: Option<Option<f64>>,
    pub season_pack_seed_time_minutes: Option<Option<i64>>,
    pub honor_tracker_minimums: Option<bool>,
    pub goal_met_action: Option<scryer_domain::SeedGoalMetAction>,
    pub never_remove: Option<bool>,
    pub minimum_seeders: Option<Option<i32>>,
    pub post_import_tracking: Option<scryer_domain::PostImportTracking>,
}

impl SeedingProfileUpdate {
    pub fn has_changes(&self) -> bool {
        self.name.is_some()
            || self.ratio.is_some()
            || self.seed_time_minutes.is_some()
            || self.season_pack_mode.is_some()
            || self.season_pack_ratio.is_some()
            || self.season_pack_seed_time_minutes.is_some()
            || self.honor_tracker_minimums.is_some()
            || self.goal_met_action.is_some()
            || self.never_remove.is_some()
            || self.minimum_seeders.is_some()
            || self.post_import_tracking.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DownloadClientConfigUpdate {
    pub id: String,
    pub name: Option<String>,
    pub client_type: Option<String>,
    pub config_json: Option<String>,
    pub is_enabled: Option<bool>,
    /// Proxy assignment, using the same nested-`Option` patch convention as
    /// `IndexerConfigUpdate::proxy_config_id`: outer `None` leaves the stored
    /// assignment alone, `Some(None)` clears it, `Some(Some(id))` sets it.
    pub proxy_config_id: Option<Option<String>>,
}

impl DownloadClientConfigUpdate {
    pub fn has_changes(&self) -> bool {
        self.name.is_some()
            || self.client_type.is_some()
            || self.config_json.is_some()
            || self.is_enabled.is_some()
            || self.proxy_config_id.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct SubtitleProviderConfigUpdate {
    pub id: String,
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub config_json: Option<String>,
    pub enabled_facets: Option<Vec<String>>,
    pub is_enabled: Option<bool>,
    pub last_health_status: Option<String>,
    pub last_error: Option<Option<String>>,
    pub last_error_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    pub disabled_until: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

impl SubtitleProviderConfigUpdate {
    pub fn has_changes(&self) -> bool {
        self.name.is_some()
            || self.provider_type.is_some()
            || self.config_json.is_some()
            || self.enabled_facets.is_some()
            || self.is_enabled.is_some()
            || self.last_health_status.is_some()
            || self.last_error.is_some()
            || self.last_error_at.is_some()
            || self.disabled_until.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubtitleProviderValidationResult {
    pub status: String,
    pub message: Option<String>,
    pub retry_after_seconds: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerValidationResult {
    pub status: String,
    pub message: Option<String>,
    pub retry_after_seconds: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedIndexerRoutingScope {
    pub scope_id: String,
    pub categories: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedIndexerChildPlan {
    pub child_key: String,
    pub name: String,
    pub provider_type: String,
    pub config_json: String,
    pub is_enabled: bool,
    pub enable_interactive_search: bool,
    pub enable_auto_search: bool,
    pub managed_metadata_json: Option<String>,
    pub caps_snapshot_json: Option<String>,
    pub routing_scopes: Vec<ManagedIndexerRoutingScope>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerSyncPlan {
    pub children: Vec<ManagedIndexerChildPlan>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerConfigSyncResult {
    pub parent_config_id: String,
    pub created_ids: Vec<String>,
    pub updated_ids: Vec<String>,
    pub deleted_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleGenerationInput {
    pub media_kind: String,
    pub facet: Option<String>,
    pub input_path: PathBuf,
    pub mime_type: String,
    pub duration_seconds: i64,
    pub size_bytes: i64,
    pub checksum: String,
    pub languages: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueuedReleaseSelection {
    pub indexer_id: Option<String>,
    pub source_hint: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: Option<String>,
    pub source_password: Option<String>,
    /// BitTorrent v1 info hash supplied by the indexer, when available.
    pub info_hash_hint: Option<String>,
    /// Indexer-announced release size, preserved for queued pseudo-incumbent scoring.
    pub size_bytes: Option<i64>,
    /// Indexer-reported seeder count at the moment the candidate was offered.
    ///
    /// Carried so redemption can re-judge admission without trusting the
    /// caller: the client never supplies this, it only round-trips inside the
    /// signed token.
    pub seeders: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionConflictPolicy {
    Abort,
    Skip,
    ReplaceEarly,
}

impl SubmissionConflictPolicy {
    pub fn from_replace_flag(replace_in_progress: bool) -> Self {
        if replace_in_progress {
            Self::ReplaceEarly
        } else {
            Self::Abort
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionScopeConflict {
    pub title_id: String,
    pub title_name: String,
    pub download_client_id: Option<String>,
    pub download_client_type: String,
    pub download_client_item_id: String,
    pub source_title: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub scope: SubmissionScope,
    pub state: Option<DownloadQueueState>,
    pub replaceable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedDownloadResult {
    pub job_id: String,
    pub queued_release: QueuedReleaseSelection,
    pub reused_existing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueDownloadOutcome {
    Queued(QueuedDownloadResult),
    Conflict(SubmissionScopeConflict),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WantedSearchOutcome {
    pub queued_count: usize,
    pub skipped_in_progress_count: usize,
    pub conflict: Option<SubmissionScopeConflict>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollectionUpdate {
    pub collection_type: Option<CollectionType>,
    pub collection_index: Option<String>,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
    pub clear_ordered_path: bool,
    pub first_episode_number: Option<String>,
    pub last_episode_number: Option<String>,
    pub monitored: Option<bool>,
}

impl CollectionUpdate {
    pub fn has_changes(&self) -> bool {
        self.collection_type.is_some()
            || self.collection_index.is_some()
            || self.label.is_some()
            || self.ordered_path.is_some()
            || self.clear_ordered_path
            || self.first_episode_number.is_some()
            || self.last_episode_number.is_some()
            || self.monitored.is_some()
    }

    pub fn has_non_monitor_changes(&self) -> bool {
        self.collection_type.is_some()
            || self.collection_index.is_some()
            || self.label.is_some()
            || self.ordered_path.is_some()
            || self.clear_ordered_path
            || self.first_episode_number.is_some()
            || self.last_episode_number.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EpisodeUpdate {
    pub episode_type: Option<scryer_domain::EpisodeType>,
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
    pub overview: Option<String>,
    pub tvdb_id: Option<String>,
    pub image_url: Option<String>,
    pub clear_image_url: bool,
}

impl EpisodeUpdate {
    pub fn has_changes(&self) -> bool {
        self.episode_type.is_some()
            || self.episode_number.is_some()
            || self.season_number.is_some()
            || self.episode_label.is_some()
            || self.title.is_some()
            || self.air_date.is_some()
            || self.duration_seconds.is_some()
            || self.has_multi_audio.is_some()
            || self.has_subtitle.is_some()
            || self.monitored.is_some()
            || self.collection_id.is_some()
            || self.overview.is_some()
            || self.tvdb_id.is_some()
            || self.image_url.is_some()
            || self.clear_image_url
    }

    pub fn has_non_monitor_changes(&self) -> bool {
        self.episode_type.is_some()
            || self.episode_number.is_some()
            || self.season_number.is_some()
            || self.episode_label.is_some()
            || self.title.is_some()
            || self.air_date.is_some()
            || self.duration_seconds.is_some()
            || self.has_multi_audio.is_some()
            || self.has_subtitle.is_some()
            || self.collection_id.is_some()
            || self.overview.is_some()
            || self.tvdb_id.is_some()
            || self.image_url.is_some()
            || self.clear_image_url
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum NotificationScopeIdUpdate {
    #[default]
    NoChange,
    Clear,
    Set(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteExecutionConfirmation {
    pub preview_fingerprint: String,
    pub typed_confirmation: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcquisitionScopeStatesQuery {
    pub statuses: Vec<String>,
    pub media_types: Vec<String>,
    pub title_id: Option<String>,
    pub library_ids: Vec<String>,
    pub title_search: Option<String>,
    pub latest_decision_codes: Vec<String>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseDecisionsQuery {
    pub wanted_item_id: Option<String>,
    pub title_id: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

/// Ordering for a paged pending-releases read. Preserves the historic per-call
/// order: the root pending-releases view sorted by `delay_until ASC` while the
/// per-wanted-item view sorted by `release_score DESC`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PendingReleasePageSort {
    #[default]
    DelayUntilAsc,
    ReleaseScoreDesc,
}

/// Storage-level filter for a single page of `waiting` pending releases plus the
/// matching total count. `library_ids` scopes rows to titles in those libraries
/// (empty means no library filter — the caller has already authorized the
/// scope, e.g. a single wanted item). `statuses` narrows within the `waiting`
/// base set to preserve the historic in-memory status filter semantics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingReleasesPageQuery {
    pub library_ids: Vec<String>,
    pub title_id: Option<String>,
    pub wanted_item_id: Option<String>,
    pub statuses: Vec<String>,
    pub limit: i64,
    pub offset: i64,
    pub sort: PendingReleasePageSort,
}

/// Parsed media properties from media analysis — application-layer DTO.
/// A single audio stream, mirroring `scryer_mediainfo::AudioStreamDetail`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioStreamDetail {
    pub codec: Option<String>,
    pub profile: Option<String>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub bitrate_kbps: Option<i32>,
}

/// A single subtitle stream, mirroring `scryer_mediainfo::SubtitleStreamDetail`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubtitleStreamDetail {
    pub codec: Option<String>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub forced: bool,
    pub default: bool,
}

/// Mirrors `scryer_mediainfo::MediaAnalysis` without depending on that crate.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MediaFileAnalysis {
    pub video_codec: Option<crate::release_parser::VideoCodec>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    pub video_hdr_format: Option<String>,
    /// Dolby Vision profile and BL-compatibility id. Serialized with the rest of
    /// the analysis, so rows written before these existed simply read as `None`
    /// rather than needing a migration.
    #[serde(default)]
    pub dovi_profile: Option<u8>,
    #[serde(default)]
    pub dovi_bl_compat_id: Option<u8>,
    pub video_frame_rate: Option<String>,
    pub video_profile: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_profile: Option<String>,
    pub audio_channels: Option<i32>,
    pub audio_bitrate_kbps: Option<i32>,
    pub audio_languages: Vec<String>,
    pub audio_streams: Vec<AudioStreamDetail>,
    pub subtitle_languages: Vec<String>,
    pub subtitle_codecs: Vec<String>,
    pub subtitle_streams: Vec<SubtitleStreamDetail>,
    pub has_multiaudio: bool,
    pub duration_seconds: Option<i32>,
    pub num_chapters: Option<i32>,
    pub container_format: Option<String>,
}

#[derive(Clone, Debug)]
pub enum MediaAnalysisOutcome {
    Valid(Box<MediaFileAnalysis>),
    Invalid(String),
}

/// Input for inserting a media file record with rich metadata.
#[derive(Clone, Debug, Default)]
pub struct InsertMediaFileInput {
    pub title_id: String,
    pub file_path: String,
    pub size_bytes: i64,
    /// The announced size the import scored this file on, when it did
    /// (`canonical_scoring::persisted_announced_size_bytes`); `None` otherwise.
    pub announced_size_bytes: Option<i64>,
    pub role: MediaFileRole,
    pub source_signature_scheme: Option<String>,
    pub source_signature_value: Option<String>,
    pub quality_label: Option<String>,
    pub scene_name: Option<String>,
    pub release_group: Option<String>,
    pub source_type: Option<String>,
    pub resolution: Option<String>,
    pub video_codec_parsed: Option<crate::release_parser::VideoCodec>,
    pub audio_codec_parsed: Option<String>,
    pub audio_channels_parsed: Option<String>,
    pub acquisition_score: Option<i32>,
    pub scoring_log: Option<String>,
    pub indexer_source: Option<String>,
    pub grabbed_release_title: Option<String>,
    pub grabbed_at: Option<String>,
    pub edition: Option<String>,
    pub original_file_path: Option<String>,
    pub release_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaFileCatalogDisposition {
    Created,
    Reused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedMediaFile {
    pub media_file_id: String,
    pub disposition: MediaFileCatalogDisposition,
}

#[derive(Clone, Debug, Default)]
pub struct TitleHistoryFilter {
    pub event_types: Option<Vec<TitleHistoryEventType>>,
    pub title_ids: Option<Vec<String>>,
    pub library_ids: Option<Vec<String>>,
    pub title_search: Option<String>,
    pub download_id: Option<String>,
    pub episode_id: Option<String>,
    pub group_by_event: bool,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug)]
pub struct TitleHistoryPage {
    pub records: Vec<TitleHistoryRecord>,
    pub total_count: i64,
}

/// Title-history event counts for one trailing time window, aggregated in SQL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActivityWindowCounts {
    pub grabbed: i64,
    pub upgraded: i64,
    pub imported: i64,
    pub import_failed: i64,
    pub download_failed: i64,
}

/// A trailing activity window paired with the window immediately before it, so
/// callers can render a count and its period-over-period delta.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DashboardActivityStats {
    pub current: ActivityWindowCounts,
    pub previous: ActivityWindowCounts,
}

/// One library root folder together with the filesystem usage of the volume
/// backing it. Usage is `None` when the filesystem could not be inspected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageRootUsage {
    pub path: String,
    pub library_id: String,
    pub library_name: String,
    pub facet: MediaFacet,
    pub used_bytes: Option<i64>,
    pub total_bytes: Option<i64>,
}

/// A release to block for a title.
///
/// `release_name` is required: acquisition's title guard means a grabbed
/// release always had a name that resolved to its title, so a caller that
/// cannot name the release has nothing to block.
#[derive(Clone, Debug)]
pub struct NewBlocklistEntry {
    pub title_id: String,
    pub release_name: String,
    /// The indexer the release failed on. Empty blocks it on every indexer --
    /// correct when the caller has no stable indexer identity to record.
    pub indexer_id: String,
    /// The torrent's infohash when known; keys the block indexer-independently.
    pub info_hash: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMode {
    Interactive,
    Auto,
}

/// Per-indexer routing entry resolved from the `indexer.routing:<scope>` setting.
#[derive(Clone, Debug)]
pub struct IndexerRoutingEntry {
    pub enabled: bool,
    pub categories: Vec<String>,
    pub priority: i64,
}

/// Per-indexer routing plan for a given facet scope.
/// When `Some`, indexers not in the map use default behavior; indexers
/// with `enabled: false` are skipped entirely for this scope.
#[derive(Clone, Debug)]
pub struct IndexerRoutingPlan {
    pub entries: std::collections::HashMap<String, IndexerRoutingEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexerSearchEligibility {
    Eligible,
    ExcludedBySearchRestriction,
    DisabledForScope,
}

pub fn indexer_search_eligibility(
    routing: Option<&IndexerRoutingPlan>,
    search_restriction: Option<&std::collections::HashSet<String>>,
    indexer_id: &str,
) -> IndexerSearchEligibility {
    if search_restriction.is_some_and(|allowed| !allowed.contains(indexer_id)) {
        return IndexerSearchEligibility::ExcludedBySearchRestriction;
    }
    if routing
        .and_then(|plan| plan.entries.get(indexer_id))
        .is_some_and(|entry| !entry.enabled)
    {
        return IndexerSearchEligibility::DisabledForScope;
    }
    IndexerSearchEligibility::Eligible
}

#[derive(Clone, Debug)]
pub struct DownloadClientAddRequest {
    pub title: Title,
    /// The facet this grab was searched and validated as, when it differs
    /// from the owning title's facet — a series-movie grab is movie-faceted
    /// while `title` is the owning series. Category gates must compare
    /// against this; `None` means the owner facet is the search facet.
    pub search_facet: Option<MediaFacet>,
    pub purpose: DownloadSubmissionPurpose,
    /// Canonical identity allocated before submitting this concrete mutation.
    pub download_id: Option<scryer_domain::download_identity::DownloadId>,
    pub source_hint: Option<String>,
    pub staged_nzb: Option<StagedNzbRef>,
    pub resolved_download_artifact: Option<ResolvedDownloadArtifact>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: Option<String>,
    pub source_password: Option<String>,
    pub category: Option<String>,
    pub queue_priority: Option<String>,
    pub download_directory: Option<String>,
    pub release_title: Option<String>,
    pub indexer_name: Option<String>,
    pub indexer_id: Option<String>,
    pub info_hash_hint: Option<String>,
    pub seed_goal_ratio: Option<f64>,
    pub seed_goal_seconds: Option<i64>,
    /// Tracker-declared minimums carried by the release (`minimum_seed_ratio` /
    /// `minimum_seed_time_minutes` and the season-pack twins in the indexer
    /// `extra` map). The grab-time resolver clamps profile goals up to these
    /// when the profile honors tracker minimums; `None` means the release
    /// carried none, or the construction site has no release object.
    pub tracker_min_seed_ratio: Option<f64>,
    pub tracker_min_seed_time_minutes: Option<i64>,
    pub season_pack_seed_ratio: Option<f64>,
    pub season_pack_seed_time_minutes: Option<i64>,
    pub is_recent: Option<bool>,
    pub season_pack: Option<bool>,
    /// Download client the operator picked for this one grab.
    ///
    /// The router normally derives the client from the indexer mapping and the
    /// title's routing scope; an unlinked interactive grab (D8/D16) has no
    /// title to route by, so the operator names the client instead and it wins
    /// over both. `None` everywhere else, which leaves routing untouched.
    pub pinned_download_client_id: Option<String>,
}

impl DownloadClientAddRequest {
    pub fn from_legacy(
        title: &Title,
        source_hint: Option<String>,
        source_kind: Option<DownloadSourceKind>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> Self {
        Self {
            title: title.clone(),
            search_facet: None,
            purpose: DownloadSubmissionPurpose::Standard,
            download_id: None,
            source_hint,
            staged_nzb: None,
            resolved_download_artifact: None,
            source_kind,
            source_title,
            source_password,
            category,
            queue_priority: None,
            download_directory: None,
            release_title: None,
            indexer_name: None,
            indexer_id: None,
            info_hash_hint: None,
            seed_goal_ratio: None,
            seed_goal_seconds: None,
            tracker_min_seed_ratio: None,
            tracker_min_seed_time_minutes: None,
            season_pack_seed_ratio: None,
            season_pack_seed_time_minutes: None,
            is_recent: None,
            season_pack: None,
            pinned_download_client_id: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ResolvedDownloadArtifact {
    Nzb {
        bytes: Vec<u8>,
        file_name: Option<String>,
        content_type: Option<String>,
    },
    Magnet {
        uri: String,
        info_hash_hint: Option<String>,
    },
    TorrentFile {
        bytes: Vec<u8>,
        file_name: Option<String>,
        content_type: Option<String>,
        info_hash_hint: Option<String>,
    },
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DownloadClientStatus {
    pub version: Option<String>,
    pub is_localhost: Option<bool>,
    pub remote_output_roots: Vec<String>,
    pub removes_completed_downloads: Option<bool>,
    pub sorting_mode: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DownloadClientMarkImportedRequest {
    pub client_item_id: String,
    pub info_hash: Option<String>,
    pub title_id: Option<String>,
    pub title_name: Option<String>,
    pub category: Option<String>,
    pub imported_path: Option<String>,
    pub download_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerDownloadClientMappingCatalog {
    pub clients: Vec<IndexerDownloadClientMappingClient>,
    pub indexers: Vec<IndexerDownloadClientMappingIndexer>,
    pub provider_compatibility: Vec<IndexerDownloadClientProviderCompatibility>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerDownloadClientMappingClient {
    pub id: String,
    pub name: String,
    pub client_type: String,
    pub is_enabled: bool,
    pub health_status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerDownloadClientMappingIndexer {
    pub id: String,
    pub name: String,
    pub download_client_id: Option<String>,
    pub protocol_families: Vec<String>,
    pub supports_mapping: bool,
    pub compatible_client_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerDownloadClientProviderCompatibility {
    pub provider_type: String,
    pub protocol_families: Vec<String>,
    pub supports_mapping: bool,
    pub compatible_client_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        fs,
        path::Path,
    };

    use super::{
        ClientJobLocator, IndexerRoutingEntry, IndexerRoutingPlan, IndexerSearchEligibility,
        indexer_search_eligibility,
    };

    fn routing_entry(enabled: bool) -> IndexerRoutingEntry {
        IndexerRoutingEntry {
            enabled,
            categories: Vec::new(),
            priority: 0,
        }
    }

    #[test]
    fn search_restriction_is_distinct_from_scope_routing() {
        let plan = IndexerRoutingPlan {
            entries: HashMap::from([
                ("world".to_string(), routing_entry(true)),
                ("c411".to_string(), routing_entry(true)),
                ("parent".to_string(), routing_entry(false)),
            ]),
        };
        let c411_only = HashSet::from(["c411".to_string()]);

        assert_eq!(
            indexer_search_eligibility(Some(&plan), Some(&c411_only), "c411"),
            IndexerSearchEligibility::Eligible
        );
        assert_eq!(
            indexer_search_eligibility(Some(&plan), Some(&c411_only), "world"),
            IndexerSearchEligibility::ExcludedBySearchRestriction
        );
        assert_eq!(
            indexer_search_eligibility(Some(&plan), Some(&c411_only), "parent"),
            IndexerSearchEligibility::ExcludedBySearchRestriction
        );
        assert!(plan.entries["world"].enabled);
        assert!(plan.entries["c411"].enabled);
        assert!(!plan.entries["parent"].enabled);
    }

    #[test]
    fn selected_scope_disabled_and_default_routed_indexers_are_distinct() {
        let disabled_plan = IndexerRoutingPlan {
            entries: HashMap::from([("parent".to_string(), routing_entry(false))]),
        };
        let parent_only = HashSet::from(["parent".to_string()]);
        assert_eq!(
            indexer_search_eligibility(Some(&disabled_plan), Some(&parent_only), "parent"),
            IndexerSearchEligibility::DisabledForScope
        );
        let default_plan = IndexerRoutingPlan {
            entries: HashMap::new(),
        };
        let new_indexer_only = HashSet::from(["new-indexer".to_string()]);
        assert_eq!(
            indexer_search_eligibility(Some(&default_plan), Some(&new_indexer_only), "new-indexer"),
            IndexerSearchEligibility::Eligible
        );
    }

    #[test]
    fn client_job_locator_new_normalizes_locator_values() {
        let client_config_id = Some("  config-1  ");
        let client_type = "  SABnzbd  ";
        let native_item_id = "  nzo-123  ";

        let locator = ClientJobLocator::new(client_config_id, client_type, native_item_id);
        assert_eq!(locator.client_id.as_deref(), Some("config-1"));
        assert_eq!(locator.client_type, "sabnzbd");
        assert_eq!(locator.item_id, "nzo-123");

        assert_eq!(
            ClientJobLocator::new(Some("  \t "), " NZBGet ", " 10010 ").client_id,
            None
        );
    }

    #[test]
    fn canonical_download_identity_source_guard() {
        // Keep all forbidden patterns here; add future retired identity machinery to this list.
        let forbidden = [
            (
                "global identity-state key selection",
                "download_identity_state_",
                "is_global",
            ),
            ("retired source identity type", "Download", "SourceIdentity"),
            (
                "tuple submission conflict target",
                "ON CONFLICT(",
                "download_client_id",
            ),
        ];
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("application crate lives below the workspace root");
        let mut files = fs::read_dir(workspace_root.join("crates"))
            .expect("crates directory should be readable")
            .map(|entry| {
                entry
                    .expect("crate directory entry should be readable")
                    .path()
                    .join("src")
            })
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        while let Some(path) = files.pop() {
            for entry in fs::read_dir(&path).expect("source tree should be readable") {
                let entry = entry.expect("source tree entry should be readable");
                let path = entry.path();
                if path.is_dir() {
                    files.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs")
                    || path.file_name().is_some_and(|name| name == "migrations.rs")
                    || path
                        .components()
                        .any(|component| component.as_os_str() == "migrations")
                {
                    continue;
                }
                let source = fs::read_to_string(&path).expect("Rust source should be readable");
                for (description, first, second) in forbidden {
                    let pattern = format!("{first}{second}");
                    assert!(
                        !source.contains(&pattern),
                        "{description} reappeared in {}",
                        path.display()
                    );
                }
            }
        }
    }
}
