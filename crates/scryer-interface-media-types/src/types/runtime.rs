use super::{JobRunPayload, Long, RuntimePathStyleValue};
use async_graphql::{Enum, ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

/// Audio stream metadata for a media file.
#[derive(SimpleObject, Clone)]
pub struct AudioStreamDetailPayload {
    /// Codec name, or null when unavailable.
    pub codec: Option<String>,
    /// Number of audio channels, or null when unavailable.
    pub channels: Option<i32>,
    /// Language code, or null when unavailable.
    pub language: Option<String>,
    /// Bitrate in kilobits per second, or null when unavailable.
    pub bitrate_kbps: Option<i32>,
}

/// Subtitle stream metadata for a media file.
#[derive(SimpleObject, Clone)]
pub struct SubtitleStreamDetailPayload {
    /// Codec name, or null when unavailable.
    pub codec: Option<String>,
    /// Language code, or null when unavailable.
    pub language: Option<String>,
    /// Stream name, or null when unavailable.
    pub name: Option<String>,
    /// Whether the subtitle is forced.
    pub forced: bool,
    /// Whether the subtitle is the default stream.
    pub default: bool,
}

/// Current service health and catalog counters.
#[derive(SimpleObject, Clone)]
pub struct SystemHealthPayload {
    /// Whether the service is ready to accept work.
    pub service_ready: bool,
    /// Configured database path.
    pub db_path: String,
    /// Datastore engine name.
    pub datastore_engine: String,
    /// Datastore migration key, or null when unavailable.
    pub datastore_migration_key: Option<String>,
    /// Runtime path syntax.
    pub runtime_path_style: RuntimePathStyleValue,
    /// Total title count across facets.
    pub total_titles: i32,
    /// Number of monitored titles.
    pub monitored_titles: i32,
    /// Total user count.
    pub total_users: i32,
    /// Movie title count.
    pub titles_movie: i32,
    /// Series title count.
    pub titles_series: i32,
    /// Anime title count.
    pub titles_anime: i32,
    /// Titles outside the named facets.
    pub titles_other: i32,
    /// Number of recent events included in the preview window.
    pub recent_events: i32,
    /// Recent event preview messages in source order.
    pub recent_event_preview: Vec<String>,
    /// Datastore migration version, or null when unavailable.
    pub db_migration_version: Option<String>,
    /// Query statistics for configured indexers.
    pub indexer_stats: Vec<IndexerQueryStatsPayload>,
}

/// Runtime path information used by clients interpreting filesystem values.
#[derive(SimpleObject, Clone, Debug)]
pub struct RuntimeInfoPayload {
    /// Path syntax used by the running service.
    pub runtime_path_style: RuntimePathStyleValue,
}

/// Compatibility result comparing the connected SMG version with requirements.
#[derive(SimpleObject, Clone)]
pub struct SmgVersionCompatibilityNoticePayload {
    /// Compatibility status string.
    pub status: String,
    /// Minimum supported SMG version.
    pub minimum_version: String,
    /// Connected SMG version.
    pub your_version: String,
    /// Human-readable compatibility message.
    pub message: String,
    /// Optional UTC deadline for upgrading.
    pub upgrade_deadline: Option<String>,
}

/// Available Scryer update information.
#[derive(SimpleObject, Clone)]
pub struct SmgScryerUpdateNoticePayload {
    /// Whether a newer version is available.
    pub available: bool,
    /// Currently running version.
    pub current_version: String,
    /// Latest available version.
    pub latest_version: String,
    /// Latest release tag.
    pub latest_tag: String,
    /// Release URL, or null when unavailable.
    pub release_url: Option<String>,
    /// UTC publication time, or null when unavailable.
    pub published_at: Option<DateTime<Utc>>,
    /// UTC time when the check completed.
    pub checked_at: DateTime<Utc>,
}

/// Installation layout identified for the in-app upgrade surface.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ApplicationInstallationKindValue {
    /// A writable standalone installation.
    Portable,
    /// A directly installed Windows MSI package.
    DirectMsi,
    /// A container-managed installation.
    Docker,
    /// A Homebrew-managed installation.
    Homebrew,
    /// A winget-managed Windows installation.
    Winget,
    /// A Windows service or session-zero installation.
    WindowsSupervised,
    /// In-app upgrades disabled by the operator.
    Disabled,
    /// An installation layout that is not supported for in-app upgrades.
    Unsupported,
}

/// Party responsible for managing application upgrades.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ApplicationUpgradeOwnerValue {
    /// The application can manage its own upgrade.
    InApp,
    /// The operator or an external package manager owns upgrades.
    Operator,
}

/// Current update availability and installation eligibility.
#[derive(SimpleObject, Clone)]
pub struct ApplicationUpgradeStatusPayload {
    /// Running application version.
    pub current_version: String,
    /// Available application version, or null when no update notice exists.
    pub update_version: Option<String>,
    /// Available release tag, or null when no update notice exists.
    pub update_tag: Option<String>,
    /// Whether the current update notice offers a newer version.
    pub update_available: bool,
    /// Installation layout identified at startup.
    pub installation_kind: ApplicationInstallationKindValue,
    /// Party responsible for managing application upgrades.
    pub management_owner: ApplicationUpgradeOwnerValue,
    /// Whether this installation is eligible for an in-app upgrade.
    pub eligible: bool,
    /// Stable snake_case code explaining eligibility.
    pub eligibility_reason: String,
    /// In-memory application-upgrade job still running in this process, if any.
    pub active_run: Option<JobRunPayload>,
    /// Most recent persisted application-upgrade job, if any.
    pub latest_run: Option<JobRunPayload>,
}

/// The exact update notice accepted by `startApplicationUpgrade`.
#[derive(InputObject, Clone)]
pub struct StartApplicationUpgradeInput {
    /// Release tag displayed by the current update notice.
    pub expected_tag: String,
    /// Release version displayed by the current update notice.
    pub expected_version: String,
}

/// Durable job run accepted for an application upgrade.
#[derive(SimpleObject, Clone)]
pub struct ApplicationUpgradeStartPayload {
    /// The registered application-upgrade job.
    pub job_run: JobRunPayload,
}

/// Query and quota counters for one indexer.
#[derive(SimpleObject, Clone)]
pub struct IndexerQueryStatsPayload {
    /// ID of the indexer.
    pub indexer_id: ID,
    /// Configured indexer name.
    pub indexer_name: String,
    /// Queries during the trailing 24 hours.
    pub queries_last_24h: i32,
    /// Successful queries during the trailing 24 hours.
    pub successful_last_24h: i32,
    /// Failed queries during the trailing 24 hours.
    pub failed_last_24h: i32,
    /// Releases grabbed through this indexer during the trailing 24 hours, counted by Scryer rather than reported by the provider.
    pub grabs_last_24h: i32,
    /// UTC time of the most recent query, or null when never queried.
    pub last_query_at: Option<DateTime<Utc>>,
    /// Current API request count, or null when not reported.
    pub api_current: Option<i32>,
    /// API request limit, or null when not reported.
    pub api_max: Option<i32>,
    /// Current grab count, or null when not reported.
    pub grab_current: Option<i32>,
    /// Grab limit, or null when not reported.
    pub grab_max: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Row count for one table in a backup.
pub struct BackupRowCountPayload {
    /// Table name.
    pub table: String,
    /// Number of rows copied from the table.
    pub row_count: Long,
}

#[derive(SimpleObject, Clone)]
/// Metadata and lifecycle result for one backup file.
pub struct BackupInfoPayload {
    /// Backup filename.
    pub filename: String,
    /// Backup size in bytes.
    pub size_bytes: Long,
    /// Backup creation time in UTC.
    pub created_at: DateTime<Utc>,
    /// Backup format version.
    pub format_version: String,
    /// Database engine that produced the backup.
    pub source_engine: String,
    /// Source migration key, when recorded.
    pub source_migration_key: Option<String>,
    /// Whether the backup is encrypted.
    pub encrypted: bool,
    /// Counts of rows included by table.
    pub row_counts: Vec<BackupRowCountPayload>,
    /// Operation that created the backup.
    pub trigger: String,
    /// Backup lifecycle status.
    pub status: String,
    /// Failure detail, or null when the operation succeeded.
    pub error_message: Option<String>,
}

#[derive(InputObject)]
/// Password used to encrypt a new backup.
pub struct CreateBackupInput {
    /// Encryption password; it is not returned in backup metadata.
    pub password: String,
}

#[derive(InputObject)]
/// Backup filename to prepare for download.
pub struct PrepareBackupDownloadInput {
    /// Existing backup filename.
    pub filename: String,
}

#[derive(InputObject)]
/// Backup filename to remove.
pub struct DeleteBackupInput {
    /// Existing backup filename.
    pub filename: String,
}

#[derive(SimpleObject, Clone)]
/// Result of attempting to delete a backup file.
pub struct DeleteBackupPayload {
    /// Filename targeted by the deletion.
    pub filename: String,
    /// False when no backup file with that name existed.
    pub deleted: bool,
}

#[derive(SimpleObject, Clone)]
/// Result of completing initial application setup.
pub struct CompleteSetupPayload {
    /// Whether setup is now complete.
    pub completed: bool,
}

#[derive(SimpleObject, Clone)]
/// Accepted request to clear cached title images.
pub struct ClearTitleImageCachePayload {
    /// When the cache-clear request was accepted.
    pub requested_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Current setup prerequisites and completion state.
pub struct SetupStatusPayload {
    /// Whether initial setup has been completed.
    pub setup_complete: bool,
    /// Whether at least one download client is configured.
    pub has_download_clients: bool,
    /// Whether at least one indexer is configured.
    pub has_indexers: bool,
}

#[derive(SimpleObject, Clone)]
/// Directory entry returned while browsing a configured path.
pub struct DirectoryEntryPayload {
    /// Entry name.
    pub name: String,
    /// Entry path.
    pub path: String,
    /// Whether the entry is a directory rather than a file.
    pub is_directory: bool,
}
