use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Id(pub String);

impl Id {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Generate an ID safe for use as a Rego package segment.
    /// Format: `r` + 32 hex chars (UUID without hyphens).
    pub fn new_rego_safe() -> Self {
        Self(format!("r{}", Uuid::new_v4().to_string().replace('-', "")))
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum MediaFacet {
    #[default]
    Movie,
    Tv,
    Anime,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootFolderEntry {
    pub path: String,
    #[serde(rename = "isDefault")]
    pub is_default: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalId {
    pub source: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Title {
    pub id: String,
    pub name: String,
    pub facet: MediaFacet,
    pub monitored: bool,
    pub tags: Vec<String>,
    pub external_ids: Vec<ExternalId>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    // rich metadata (hydrated from metadata gateway)
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub banner_url: Option<String>,
    pub background_url: Option<String>,
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
    pub metadata_fetched_at: Option<DateTime<Utc>>,
    pub min_availability: Option<String>,
    pub digital_release_date: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InterstitialMovieMetadata {
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
    #[serde(default)]
    pub association_confidence: Option<String>,
    #[serde(default)]
    pub continuity_status: Option<String>,
    #[serde(default)]
    pub movie_form: Option<String>,
    #[serde(default)]
    pub confidence: Option<String>,
    #[serde(default)]
    pub signal_summary: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Collection {
    pub id: String,
    pub title_id: String,
    pub collection_type: String,
    pub collection_index: String,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
    pub narrative_order: Option<String>,
    pub first_episode_number: Option<String>,
    pub last_episode_number: Option<String>,
    pub interstitial_movie: Option<InterstitialMovieMetadata>,
    #[serde(default)]
    pub specials_movies: Vec<InterstitialMovieMetadata>,
    pub monitored: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Episode {
    pub id: String,
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
    pub is_filler: bool,
    pub is_recap: bool,
    pub absolute_number: Option<String>,
    pub overview: Option<String>,
    pub monitored: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CalendarEpisode {
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerConfig {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key_encrypted: Option<String>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub disabled_until: Option<DateTime<Utc>>,
    pub is_enabled: bool,
    pub enable_interactive_search: bool,
    pub enable_auto_search: bool,
    pub last_health_status: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub config_json: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewIndexerConfig {
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key_encrypted: Option<String>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub is_enabled: bool,
    pub enable_interactive_search: bool,
    pub enable_auto_search: bool,
    pub config_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadClientConfig {
    pub id: String,
    pub name: String,
    pub client_type: String,
    pub base_url: Option<String>,
    pub config_json: String,
    pub client_priority: i64,
    pub is_enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewDownloadClientConfig {
    pub name: String,
    pub client_type: String,
    pub base_url: Option<String>,
    pub config_json: String,
    pub client_priority: i64,
    pub is_enabled: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadQueueState {
    Queued,
    Downloading,
    Verifying,
    Repairing,
    Extracting,
    Paused,
    Completed,
    ImportPending,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadQueueItem {
    pub id: String,
    pub title_id: Option<String>,
    pub title_name: String,
    pub facet: Option<String>,
    pub client_id: String,
    pub client_name: String,
    pub client_type: String,
    pub state: DownloadQueueState,
    pub progress_percent: u8,
    pub size_bytes: Option<i64>,
    pub remaining_seconds: Option<i64>,
    pub queued_at: Option<String>,
    pub last_updated_at: Option<String>,
    pub attention_required: bool,
    pub attention_reason: Option<String>,
    pub download_client_item_id: String,
    pub import_status: Option<String>,
    pub import_error_message: Option<String>,
    pub imported_at: Option<String>,
    pub is_scryer_origin: bool,
}

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "wmv", "mov", "m4v", "ts", "m2ts", "webm", "flv", "ogv",
];

pub fn is_video_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| VIDEO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub const ARCHIVE_EXTENSIONS: &[&str] = &["rar", "7z", "zip"];

/// Check if a path is a RAR volume file (.rar, .r00, .r01, etc.)
pub fn is_rar_volume(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lower = ext.to_ascii_lowercase();
    lower == "rar"
        || (lower.starts_with('r')
            && lower.len() >= 2
            && lower[1..].chars().all(|c| c.is_ascii_digit()))
}

pub fn is_archive_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ARCHIVE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletedDownload {
    pub client_type: String,
    pub client_id: String,
    pub download_client_item_id: String,
    pub name: String,
    pub dest_dir: String,
    pub category: Option<String>,
    pub size_bytes: Option<i64>,
    pub completed_at: Option<DateTime<Utc>>,
    pub parameters: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Queued,
    Processing,
    Completed,
    Failed,
    Skipped,
}

impl ImportStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportDecision {
    Imported,
    Rejected,
    Skipped,
    Conflict,
    Unmatched,
    Failed,
}

impl ImportDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::Rejected => "rejected",
            Self::Skipped => "skipped",
            Self::Conflict => "conflict",
            Self::Unmatched => "unmatched",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportSkipReason {
    AlreadyImported,
    DuplicateFile,
    PostDownloadRuleBlocked,
    PolicyMismatch,
    UnresolvedIdentity,
    NoVideoFiles,
    DiskFull,
    PermissionDenied,
}

impl ImportSkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AlreadyImported => "already_imported",
            Self::DuplicateFile => "duplicate_file",
            Self::PostDownloadRuleBlocked => "post_download_rule_blocked",
            Self::PolicyMismatch => "policy_mismatch",
            Self::UnresolvedIdentity => "unresolved_identity",
            Self::NoVideoFiles => "no_video_files",
            Self::DiskFull => "disk_full",
            Self::PermissionDenied => "permission_denied",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportStrategy {
    HardLink,
    Copy,
}

impl ImportStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HardLink => "hardlink",
            Self::Copy => "copy",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub import_id: String,
    pub decision: ImportDecision,
    pub skip_reason: Option<ImportSkipReason>,
    pub title_id: Option<String>,
    pub source_path: String,
    pub dest_path: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub link_type: Option<ImportStrategy>,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportRecord {
    pub id: String,
    pub source_system: String,
    pub source_ref: String,
    pub import_type: String,
    pub status: String,
    pub payload_json: String,
    pub result_json: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct ImportFileResult {
    pub strategy: ImportStrategy,
    pub source_path: std::path::PathBuf,
    pub dest_path: std::path::PathBuf,
    pub size_bytes: u64,
}

// ── Title history ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TitleHistoryEventType {
    Grabbed,
    DownloadCompleted,
    Imported,
    ImportFailed,
    ImportSkipped,
    FileDeleted,
    FileRenamed,
    DownloadIgnored,
}

impl TitleHistoryEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Grabbed => "grabbed",
            Self::DownloadCompleted => "download_completed",
            Self::Imported => "imported",
            Self::ImportFailed => "import_failed",
            Self::ImportSkipped => "import_skipped",
            Self::FileDeleted => "file_deleted",
            Self::FileRenamed => "file_renamed",
            Self::DownloadIgnored => "download_ignored",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "grabbed" => Some(Self::Grabbed),
            "download_completed" => Some(Self::DownloadCompleted),
            "imported" => Some(Self::Imported),
            "import_failed" => Some(Self::ImportFailed),
            "import_skipped" => Some(Self::ImportSkipped),
            "file_deleted" => Some(Self::FileDeleted),
            "file_renamed" => Some(Self::FileRenamed),
            "download_ignored" => Some(Self::DownloadIgnored),
            _ => None,
        }
    }

    pub const ALL: &[Self] = &[
        Self::Grabbed,
        Self::DownloadCompleted,
        Self::Imported,
        Self::ImportFailed,
        Self::ImportSkipped,
        Self::FileDeleted,
        Self::FileRenamed,
        Self::DownloadIgnored,
    ];
}

impl std::fmt::Display for TitleHistoryEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TitleHistoryRecord {
    pub id: String,
    pub title_id: String,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub event_type: String,
    pub source_title: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub data_json: Option<String>,
    pub occurred_at: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct BlocklistEntry {
    pub id: String,
    pub title_id: String,
    pub source_title: Option<String>,
    pub source_hint: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub reason: Option<String>,
    pub data_json: Option<String>,
    pub created_at: String,
}

// ── Titles ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewTitle {
    pub name: String,
    pub facet: MediaFacet,
    pub monitored: bool,
    pub tags: Vec<String>,
    pub external_ids: Vec<ExternalId>,
    #[serde(default)]
    pub min_availability: Option<String>,
    #[serde(default)]
    pub poster_url: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub sort_title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub runtime_minutes: Option<i32>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub content_status: Option<String>,
}

impl NewTitle {
    pub fn with_defaults(name: impl Into<String>, facet: MediaFacet) -> Self {
        Self {
            name: name.into(),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            poster_url: None,
            year: None,
            overview: None,
            sort_title: None,
            slug: None,
            runtime_minutes: None,
            language: None,
            content_status: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEvent {
    pub id: String,
    pub event_type: EventType,
    pub actor_user_id: Option<String>,
    pub title_id: Option<String>,
    pub message: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    TitleAdded,
    TitleUpdated,
    PolicyEvaluated,
    ActionTriggered,
    ActionCompleted,
    FileUpgraded,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyInput {
    pub title_id: String,
    pub facet: MediaFacet,
    pub has_existing_file: bool,
    pub candidate_quality: Option<String>,
    pub requested_mode: String,
    pub release_title: Option<String>,
    pub quality_profile_id: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub is_anime: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyOutput {
    pub decision: bool,
    pub score: f32,
    pub reason_codes: Vec<String>,
    pub explanation: String,
    pub scoring_log: Vec<PolicyScoringEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyScoringEntry {
    pub code: String,
    pub delta: i32,
    pub source: String,
}

/// A plugin installation record.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginInstallation {
    pub id: String,
    /// Unique plugin identifier from the registry (e.g. "nzbgeek", "newznab").
    pub plugin_id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub is_enabled: bool,
    pub is_builtin: bool,
    pub wasm_sha256: Option<String>,
    pub source_url: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A user-authored rule set definition.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleSet {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rego_source: String,
    pub enabled: bool,
    pub priority: i32,
    /// Facets this rule applies to. Empty = all facets.
    pub applied_facets: Vec<MediaFacet>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Entitlement {
    ViewCatalog,
    MonitorTitle,
    ManageTitle,
    TriggerActions,
    ManageConfig,
    ViewHistory,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: Option<String>,
    pub entitlements: Vec<Entitlement>,
}

impl User {
    pub fn new_admin(username: impl Into<String>) -> Self {
        Self {
            id: Id::new().0,
            username: username.into(),
            password_hash: None,
            entitlements: Self::all_entitlements(),
        }
    }

    pub fn all_entitlements() -> Vec<Entitlement> {
        vec![
            Entitlement::ViewCatalog,
            Entitlement::MonitorTitle,
            Entitlement::ManageTitle,
            Entitlement::TriggerActions,
            Entitlement::ManageConfig,
            Entitlement::ViewHistory,
        ]
    }

    pub fn with_password_hash(
        username: impl Into<String>,
        password_hash: impl Into<String>,
    ) -> Self {
        Self {
            id: Id::new().0,
            username: username.into(),
            password_hash: Some(password_hash.into()),
            entitlements: Self::all_entitlements(),
        }
    }

    pub fn has_entitlement(&self, required: &Entitlement) -> bool {
        self.entitlements.contains(required)
    }

    pub fn has_all_entitlements(&self) -> bool {
        let all = Self::all_entitlements();
        all.iter()
            .all(|entitlement| self.entitlements.contains(entitlement))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewUser {
    pub username: String,
    pub password: String,
    pub entitlements: Vec<Entitlement>,
}

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("resource not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("repository error: {0}")]
    Repository(String),
}

pub type DomainResult<T> = Result<T, DomainError>;

/// Indexer capabilities declared by a plugin. Used by the dispatcher to skip
/// indexers that don't support a given search type.
fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerProviderCapabilities {
    #[serde(default = "default_true")]
    pub rss: bool,
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub imdb_search: bool,
    #[serde(default)]
    pub tvdb_search: bool,
    #[serde(default)]
    pub anidb_search: bool,
}

/// Describes a single configuration field a plugin expects.
/// Used by the plugin system to advertise what config keys are needed,
/// and by the frontend to render dynamic form fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigFieldDef {
    /// Config key name (e.g. "custom_endpoint"). Used as the JSON key in
    /// `config_json` and the Extism config key.
    pub key: String,
    /// Human-readable label for the form field.
    pub label: String,
    /// Field type: "string", "password", "bool", "select", "number".
    pub field_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// For "select" fields: the available options.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ConfigFieldOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

/// A single option for "select"-type config fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigFieldOption {
    pub value: String,
    pub label: String,
}

// ── Notification types ──────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationChannelConfig {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub config_json: String,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewNotificationChannelConfig {
    pub name: String,
    pub channel_type: String,
    pub config_json: String,
    pub is_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationSubscription {
    pub id: String,
    pub channel_id: String,
    pub event_type: String,
    pub scope: String,
    pub scope_id: Option<String>,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewNotificationSubscription {
    pub channel_id: String,
    pub event_type: String,
    pub scope: String,
    pub scope_id: Option<String>,
    pub is_enabled: bool,
}

/// All notification event types supported by the system.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEventType {
    Grab,
    Download,
    Upgrade,
    ImportComplete,
    Rename,
    TitleAdded,
    TitleDeleted,
    FileDeleted,
    FileDeletedForUpgrade,
    HealthIssue,
    HealthRestored,
    ApplicationUpdate,
    ManualInteractionRequired,
    Test,
}

impl NotificationEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Grab => "grab",
            Self::Download => "download",
            Self::Upgrade => "upgrade",
            Self::ImportComplete => "import_complete",
            Self::Rename => "rename",
            Self::TitleAdded => "title_added",
            Self::TitleDeleted => "title_deleted",
            Self::FileDeleted => "file_deleted",
            Self::FileDeletedForUpgrade => "file_deleted_for_upgrade",
            Self::HealthIssue => "health_issue",
            Self::HealthRestored => "health_restored",
            Self::ApplicationUpdate => "application_update",
            Self::ManualInteractionRequired => "manual_interaction_required",
            Self::Test => "test",
        }
    }

    pub fn all() -> &'static [NotificationEventType] {
        &[
            Self::Grab,
            Self::Download,
            Self::Upgrade,
            Self::ImportComplete,
            Self::Rename,
            Self::TitleAdded,
            Self::TitleDeleted,
            Self::FileDeleted,
            Self::FileDeletedForUpgrade,
            Self::HealthIssue,
            Self::HealthRestored,
            Self::ApplicationUpdate,
            Self::ManualInteractionRequired,
            Self::Test,
        ]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "grab" => Some(Self::Grab),
            "download" => Some(Self::Download),
            "upgrade" => Some(Self::Upgrade),
            "import_complete" => Some(Self::ImportComplete),
            "rename" => Some(Self::Rename),
            "title_added" => Some(Self::TitleAdded),
            "title_deleted" => Some(Self::TitleDeleted),
            "file_deleted" => Some(Self::FileDeleted),
            "file_deleted_for_upgrade" => Some(Self::FileDeletedForUpgrade),
            "health_issue" => Some(Self::HealthIssue),
            "health_restored" => Some(Self::HealthRestored),
            "application_update" => Some(Self::ApplicationUpdate),
            "manual_interaction_required" => Some(Self::ManualInteractionRequired),
            "test" => Some(Self::Test),
            _ => None,
        }
    }
}

impl std::str::FromStr for NotificationEventType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(())
    }
}

// ── Post-Processing Scripts ──────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostProcessingScript {
    pub id: String,
    pub name: String,
    pub description: String,
    pub script_type: String,         // "inline" | "file"
    pub script_content: String,      // shell command or file path
    pub applied_facets: Vec<String>, // ["movie", "tv", "anime"]
    pub execution_mode: String,      // "blocking" | "fire_and_forget"
    pub timeout_secs: i64,
    pub priority: i32,
    pub enabled: bool,
    pub debug: bool, // capture stdout/stderr when true
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostProcessingScriptRun {
    pub id: String,
    pub script_id: String,
    pub script_name: String,
    pub title_id: Option<String>,
    pub title_name: Option<String>,
    pub facet: Option<String>,
    pub file_path: Option<String>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub duration_ms: Option<i64>,
    pub env_payload_json: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

pub fn parse_query(value: &str) -> String {
    value.trim().to_lowercase()
}

pub fn match_fuzzy(candidate: &str, query: &str) -> bool {
    let target = parse_query(candidate);
    let q = parse_query(query);
    if q.is_empty() {
        return true;
    }
    target.contains(&q)
}

pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut output = HashSet::new();
    for tag in tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() {
            // Preserve case for structured scryer: tags (they may contain paths)
            if trimmed.starts_with("scryer:") {
                output.insert(trimmed.to_string());
            } else {
                output.insert(trimmed.to_lowercase());
            }
        }
    }
    let mut ordered: Vec<String> = output.into_iter().collect();
    ordered.sort_unstable();
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_round_trip() {
        let id = Id::new();
        assert!(!id.0.is_empty());
    }

    #[test]
    fn tags_normalize() {
        assert_eq!(
            normalize_tags(&["Anime".into(), "anime".into(), " tv ".into()]),
            vec!["anime".to_string(), "tv".to_string()]
        );
    }

    #[test]
    fn fuzzy_search_matches_partial() {
        assert!(match_fuzzy("Cowboy Bebop", "bebo"));
        assert!(!match_fuzzy("Cowboy Bebop", "dune"));
    }

    #[test]
    fn admin_has_all_entitlements() {
        let admin = User::new_admin("root");
        assert!(admin.has_entitlement(&Entitlement::ManageConfig));
        assert!(admin.has_entitlement(&Entitlement::ViewHistory));
    }
}

#[cfg(test)]
#[path = "domain_tests.rs"]
mod domain_tests;
