use super::{LibraryScanSummaryPayload, MediaFacetValue};
use async_graphql::{Enum, ID, Json, SimpleObject};
use chrono::{DateTime, Utc};

/// Stable key identifying a scheduled or manually triggered job.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobKeyValue {
    /// Movie library scan.
    LibraryScanMovies,
    /// Series library scan.
    LibraryScanSeries,
    /// Anime library scan.
    LibraryScanAnime,
    /// Background movie library refresh.
    BackgroundLibraryRefreshMovies,
    /// Background series library refresh.
    BackgroundLibraryRefreshSeries,
    /// Background anime library refresh.
    BackgroundLibraryRefreshAnime,
    /// RSS synchronization.
    RssSync,
    /// Subtitle search.
    SubtitleSearch,
    /// Plugin registry refresh.
    PluginRegistryRefresh,
    /// Housekeeping work.
    Housekeeping,
    /// Health checks.
    HealthChecks,
    /// Automatic backup.
    AutoBackup,
    /// Prowlarr synchronization.
    ProwlarrSync,
    /// Pending-release processing.
    PendingReleaseProcessing,
    /// Staged NZB pruning.
    StagedNzbPrune,
    /// Background full-file content-hash backfill.
    FullHashBackfill,
    /// Discovery synchronization.
    DiscoverySync,
    /// Title-image cache refresh.
    TitleImageCacheRefresh,
    /// Title deletion.
    TitleDeletion,
    /// Title rename.
    TitleRename,
    /// Media-file deletion.
    MediaFileDeletion,
    /// Recycle-bin restore.
    RecycleBinRestore,
    /// Recycle-bin purge.
    RecycleBinPurge,
    /// Acquisition search.
    AcquisitionSearch,
    /// Application upgrade.
    ApplicationUpgrade,
    /// Location operation: a root move, transfer, or other placement change.
    LocationOperation,
}

/// Broad category assigned to a job.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobCategoryValue {
    /// Library work.
    Library,
    /// Acquisition work.
    Acquisition,
    /// Maintenance work.
    Maintenance,
    /// Subtitle work.
    Subtitles,
    /// System work.
    System,
}

/// Operational grouping assigned to a job.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobSectionValue {
    /// Primary operational jobs.
    Primary,
    /// Maintenance jobs.
    Maintenance,
}

/// Schedule rule used by a job definition.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobScheduleKindValue {
    /// Runs only when explicitly triggered.
    Manual,
    /// Repeats at a fixed interval.
    Interval,
    /// Runs at startup and then at a fixed interval.
    StartupAndInterval,
    /// Runs once per day at a configured local time.
    DailyAtTime,
}

/// Source that started a job run.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobTriggerSourceValue {
    /// Started by an explicit user or API action.
    Manual,
    /// Started by scheduled application startup.
    ScheduledStartup,
    /// Started by an interval schedule.
    ScheduledInterval,
    /// Started by a daily schedule.
    ScheduledDaily,
    /// Started internally by the system.
    SystemInternal,
}

/// Lifecycle state of a job run.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobRunStatusValue {
    /// Run accepted but not started.
    Queued,
    /// Run is discovering work.
    Discovering,
    /// Run is executing work.
    Running,
    /// Run completed successfully.
    Completed,
    /// Run completed with non-fatal issues.
    Warning,
    /// Run failed.
    Failed,
}

/// Mode used by a library scan.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LibraryScanModeValue {
    /// Reconcile the full library.
    Full,
    /// Add newly discovered content without a full reconciliation.
    Additive,
}

/// Lifecycle state of a library scan.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LibraryScanStatusValue {
    /// Scan is discovering files or titles.
    Discovering,
    /// Scan is processing discovered work.
    Running,
    /// Scan completed successfully.
    Completed,
    /// Scan was canceled.
    Canceled,
    /// Scan completed with non-fatal issues.
    Warning,
    /// Scan failed.
    Failed,
}

/// Counts for one phase of a library scan.
#[derive(SimpleObject, Clone)]
pub struct LibraryScanPhaseProgressPayload {
    /// Total units in this phase; zero is valid when no work was found.
    pub total: i32,
    /// Units completed so far.
    pub completed: i32,
    /// Units that failed in this phase.
    pub failed: i32,
}

/// Progress snapshot for one library scan.
#[derive(SimpleObject, Clone)]
pub struct LibraryScanProgressPayload {
    /// ID of the scan session.
    pub session_id: ID,
    /// Media facet being scanned.
    pub facet: MediaFacetValue,
    /// ID of the library being scanned, or null for all libraries in the facet.
    pub library_id: Option<ID>,
    /// Scan mode.
    pub mode: LibraryScanModeValue,
    /// Current scan status.
    pub status: LibraryScanStatusValue,
    /// UTC time when the scan started.
    pub started_at: DateTime<Utc>,
    /// UTC time when the progress snapshot was updated.
    pub updated_at: DateTime<Utc>,
    /// Number of titles found so far.
    pub found_titles: i32,
    /// Whether the total title-match count is known.
    pub title_match_total_known: bool,
    /// Title matching progress counts.
    pub title_match_progress: LibraryScanPhaseProgressPayload,
    /// Whether the total hydration count is known.
    pub hydration_total_known: bool,
    /// Metadata hydration progress counts.
    pub hydration_progress: LibraryScanPhaseProgressPayload,
    /// Whether the total media-analysis count is known.
    pub media_analysis_total_known: bool,
    /// Media-analysis progress counts.
    pub media_analysis_progress: LibraryScanPhaseProgressPayload,
    /// Final scan summary, or null until a summary is available.
    pub summary: Option<LibraryScanSummaryPayload>,
}

/// Schedule details for a job definition.
#[derive(SimpleObject, Clone)]
pub struct JobScheduleInfoPayload {
    /// Schedule kind.
    pub kind: JobScheduleKindValue,
    /// Human-readable schedule description.
    pub description: String,
    /// Repeat interval in seconds, or null for non-interval schedules.
    pub interval_seconds: Option<i32>,
    /// Initial delay in seconds, or null when not applicable.
    pub initial_delay_seconds: Option<i32>,
    /// Next scheduled run as a UTC timestamp, or null when no run is scheduled.
    pub next_run_at: Option<DateTime<Utc>>,
}

/// Metadata describing a configured job.
#[derive(SimpleObject, Clone)]
pub struct JobDefinitionPayload {
    /// Stable job key.
    pub key: JobKeyValue,
    /// Display name of the job.
    pub display_name: String,
    /// Human-readable job description.
    pub description: String,
    /// Broad job category.
    pub category: JobCategoryValue,
    /// Job section.
    pub section: JobSectionValue,
    /// Whether an explicit trigger is allowed.
    pub manual_trigger_allowed: bool,
    /// Whether the run reports library-scan progress.
    pub uses_library_scan_progress: bool,
    /// Configured schedule details.
    pub schedule: JobScheduleInfoPayload,
}

/// Current status and progress of one job run.
#[derive(SimpleObject, Clone)]
pub struct JobRunPayload {
    /// ID of the job run.
    pub id: ID,
    /// Stable key of the job being run.
    pub job_key: JobKeyValue,
    /// Display name of the job.
    pub display_name: String,
    /// Broad job category.
    pub category: JobCategoryValue,
    /// Job section.
    pub section: JobSectionValue,
    /// Current run status.
    pub status: JobRunStatusValue,
    /// Source that started the run.
    pub trigger_source: JobTriggerSourceValue,
    /// UTC time when the run started.
    pub started_at: DateTime<Utc>,
    /// UTC completion time, or null while the run is active.
    pub completed_at: Option<DateTime<Utc>>,
    /// Structured result summary, or null before completion or when unavailable.
    pub summary_json: Option<Json<serde_json::Value>>,
    /// Human-readable result summary, or null when unavailable.
    pub summary_text: Option<String>,
    /// Failure detail, or null when the run has not failed.
    pub error_text: Option<String>,
    /// Structured progress data, or null when the job does not expose it.
    pub progress_json: Option<Json<serde_json::Value>>,
    /// Library-scan progress, or null for jobs without scan progress.
    pub library_scan_progress: Option<LibraryScanProgressPayload>,
}
