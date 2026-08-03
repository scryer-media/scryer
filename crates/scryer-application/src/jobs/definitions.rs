use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tokio::time::{Duration, Sleep};

use crate::{LibraryScanSession, LibraryScanStatus};

const JOB_RUN_PUSH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobCategory {
    Library,
    Acquisition,
    Maintenance,
    Subtitles,
    System,
}

impl JobCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Acquisition => "acquisition",
            Self::Maintenance => "maintenance",
            Self::Subtitles => "subtitles",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobSection {
    Primary,
    Maintenance,
}

impl JobSection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Maintenance => "maintenance",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobScheduleKind {
    Manual,
    Interval,
    StartupAndInterval,
    DailyAtTime,
}

impl JobScheduleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Interval => "interval",
            Self::StartupAndInterval => "startup_interval",
            Self::DailyAtTime => "daily_at_time",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobTriggerSource {
    Manual,
    ScheduledStartup,
    ScheduledInterval,
    ScheduledDaily,
    SystemInternal,
}

impl JobTriggerSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::ScheduledStartup => "scheduled_startup",
            Self::ScheduledInterval => "scheduled_interval",
            Self::ScheduledDaily => "scheduled_daily",
            Self::SystemInternal => "system_internal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "manual" => Some(Self::Manual),
            "scheduled_startup" => Some(Self::ScheduledStartup),
            "scheduled_interval" => Some(Self::ScheduledInterval),
            "scheduled_daily" => Some(Self::ScheduledDaily),
            "system_internal" => Some(Self::SystemInternal),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobRunStatus {
    Queued,
    Discovering,
    Running,
    Completed,
    Warning,
    Failed,
}

impl JobRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Discovering => "discovering",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Warning => "warning",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "discovering" => Some(Self::Discovering),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "warning" => Some(Self::Warning),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Warning | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobKey {
    LibraryScanMovies,
    LibraryScanSeries,
    LibraryScanAnime,
    BackgroundLibraryRefreshMovies,
    BackgroundLibraryRefreshSeries,
    BackgroundLibraryRefreshAnime,
    ProwlarrSync,
    RssSync,
    SubtitleSearch,
    PluginRegistryRefresh,
    Housekeeping,
    HealthChecks,
    AutoBackup,
    PendingReleaseProcessing,
    StagedNzbPrune,
    DiscoverySync,
    TitleImageCacheRefresh,
    TitleDeletion,
    MediaFileDeletion,
    RecycleBinRestore,
    AcquisitionSearch,
}

impl JobKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LibraryScanMovies => "library_scan_movies",
            Self::LibraryScanSeries => "library_scan_series",
            Self::LibraryScanAnime => "library_scan_anime",
            Self::BackgroundLibraryRefreshMovies => "background_library_refresh_movies",
            Self::BackgroundLibraryRefreshSeries => "background_library_refresh_series",
            Self::BackgroundLibraryRefreshAnime => "background_library_refresh_anime",
            Self::ProwlarrSync => "prowlarr_sync",
            Self::RssSync => "rss_sync",
            Self::SubtitleSearch => "subtitle_search",
            Self::PluginRegistryRefresh => "plugin_registry_refresh",
            Self::Housekeeping => "housekeeping",
            Self::HealthChecks => "health_checks",
            Self::AutoBackup => "auto_backup",
            Self::PendingReleaseProcessing => "pending_release_processing",
            Self::StagedNzbPrune => "staged_nzb_prune",
            Self::DiscoverySync => "discovery_sync",
            Self::TitleImageCacheRefresh => "title_image_cache_refresh",
            Self::TitleDeletion => "title_deletion",
            Self::MediaFileDeletion => "media_file_deletion",
            Self::RecycleBinRestore => "recycle_bin_restore",
            Self::AcquisitionSearch => "acquisition_search",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "library_scan_movies" => Some(Self::LibraryScanMovies),
            "library_scan_series" => Some(Self::LibraryScanSeries),
            "library_scan_anime" => Some(Self::LibraryScanAnime),
            "background_library_refresh_movies" => Some(Self::BackgroundLibraryRefreshMovies),
            "background_library_refresh_series" => Some(Self::BackgroundLibraryRefreshSeries),
            "background_library_refresh_anime" => Some(Self::BackgroundLibraryRefreshAnime),
            "prowlarr_sync" => Some(Self::ProwlarrSync),
            "rss_sync" => Some(Self::RssSync),
            "subtitle_search" => Some(Self::SubtitleSearch),
            "plugin_registry_refresh" => Some(Self::PluginRegistryRefresh),
            "housekeeping" => Some(Self::Housekeeping),
            "health_checks" => Some(Self::HealthChecks),
            "auto_backup" => Some(Self::AutoBackup),
            "pending_release_processing" => Some(Self::PendingReleaseProcessing),
            "staged_nzb_prune" => Some(Self::StagedNzbPrune),
            "discovery_sync" => Some(Self::DiscoverySync),
            "title_image_cache_refresh" => Some(Self::TitleImageCacheRefresh),
            "title_deletion" => Some(Self::TitleDeletion),
            "media_file_deletion" => Some(Self::MediaFileDeletion),
            "recycle_bin_restore" => Some(Self::RecycleBinRestore),
            "acquisition_search" => Some(Self::AcquisitionSearch),
            _ => None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::LibraryScanMovies => "Library Scan: Movies",
            Self::LibraryScanSeries => "Library Scan: Series",
            Self::LibraryScanAnime => "Library Scan: Anime",
            Self::BackgroundLibraryRefreshMovies => "Background Library Refresh: Movies",
            Self::BackgroundLibraryRefreshSeries => "Background Library Refresh: Series",
            Self::BackgroundLibraryRefreshAnime => "Background Library Refresh: Anime",
            Self::ProwlarrSync => "Prowlarr Sync",
            Self::RssSync => "RSS Sync",
            Self::SubtitleSearch => "Subtitle Search",
            Self::PluginRegistryRefresh => "Plugin Catalog Refresh",
            Self::Housekeeping => "Housekeeping",
            Self::HealthChecks => "Health Checks",
            Self::AutoBackup => "Automatic Backup",
            Self::PendingReleaseProcessing => "Pending Release Processing",
            Self::StagedNzbPrune => "Staged NZB Prune",
            Self::DiscoverySync => "Discovery Sync",
            Self::TitleImageCacheRefresh => "Title Image Cache Refresh",
            Self::TitleDeletion => "Title Deletion",
            Self::MediaFileDeletion => "Media File Deletion",
            Self::RecycleBinRestore => "Recycle Bin Restore",
            Self::AcquisitionSearch => "Acquisition Search",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::LibraryScanMovies => "Scan the movies library and import missing movie files.",
            Self::LibraryScanSeries => "Scan the series library and reconcile episodic files.",
            Self::LibraryScanAnime => "Scan the anime library and reconcile episodic files.",
            Self::BackgroundLibraryRefreshMovies => {
                "Lightweight movie library refresh for newly discovered folders or files."
            }
            Self::BackgroundLibraryRefreshSeries => {
                "Lightweight series refresh that discovers new folders and additive file changes."
            }
            Self::BackgroundLibraryRefreshAnime => {
                "Lightweight anime refresh that discovers new folders and additive file changes."
            }
            Self::ProwlarrSync => {
                "Sync enabled managed Prowlarr parents and reconcile child indexers."
            }
            Self::RssSync => "Fetch RSS feeds from enabled indexers and evaluate new releases.",
            Self::SubtitleSearch => "Search for missing subtitles for monitored media.",
            Self::PluginRegistryRefresh => "Refresh the installed plugin catalog metadata.",
            Self::Housekeeping => "Clean stale records and purge expired artifacts.",
            Self::HealthChecks => "Run configured system health checks.",
            Self::AutoBackup => {
                "Create a daily backup snapshot and keep the newest successful automatic backups."
            }
            Self::PendingReleaseProcessing => {
                "Process delayed pending releases whose hold period has expired."
            }
            Self::StagedNzbPrune => "Prune expired staged NZB artifacts.",
            Self::DiscoverySync => {
                "Evaluate local discovery freshness and refresh SMG discovery snapshots."
            }
            Self::TitleImageCacheRefresh => {
                "Refresh remote artwork URLs from SMG and rebuild locally processed title images."
            }
            Self::TitleDeletion => "Delete selected titles from the catalog.",
            Self::MediaFileDeletion => "Delete a media file from disk and the catalog.",
            Self::RecycleBinRestore => "Restore a recycled file to its library.",
            Self::AcquisitionSearch => {
                "Interactive acquisition search over the selected wanted/upgrade scopes."
            }
        }
    }

    pub fn category(self) -> JobCategory {
        match self {
            Self::LibraryScanMovies
            | Self::LibraryScanSeries
            | Self::LibraryScanAnime
            | Self::BackgroundLibraryRefreshMovies
            | Self::BackgroundLibraryRefreshSeries
            | Self::BackgroundLibraryRefreshAnime => JobCategory::Library,
            Self::ProwlarrSync | Self::RssSync | Self::AcquisitionSearch => {
                JobCategory::Acquisition
            }
            Self::SubtitleSearch => JobCategory::Subtitles,
            Self::PluginRegistryRefresh
            | Self::HealthChecks
            | Self::AutoBackup
            | Self::DiscoverySync
            | Self::TitleImageCacheRefresh
            | Self::TitleDeletion
            | Self::MediaFileDeletion
            | Self::RecycleBinRestore => JobCategory::System,
            Self::Housekeeping | Self::PendingReleaseProcessing | Self::StagedNzbPrune => {
                JobCategory::Maintenance
            }
        }
    }

    pub fn section(self) -> JobSection {
        match self {
            Self::PendingReleaseProcessing | Self::StagedNzbPrune => JobSection::Maintenance,
            _ => JobSection::Primary,
        }
    }

    pub fn schedule_kind(self) -> JobScheduleKind {
        match self {
            Self::BackgroundLibraryRefreshMovies
            | Self::BackgroundLibraryRefreshSeries
            | Self::BackgroundLibraryRefreshAnime => JobScheduleKind::StartupAndInterval,
            Self::ProwlarrSync
            | Self::RssSync
            | Self::SubtitleSearch
            | Self::PluginRegistryRefresh
            | Self::Housekeeping
            | Self::HealthChecks
            | Self::PendingReleaseProcessing
            | Self::StagedNzbPrune => JobScheduleKind::Interval,
            Self::DiscoverySync => JobScheduleKind::StartupAndInterval,
            Self::AutoBackup => JobScheduleKind::DailyAtTime,
            Self::LibraryScanMovies
            | Self::LibraryScanSeries
            | Self::LibraryScanAnime
            | Self::TitleImageCacheRefresh
            | Self::TitleDeletion
            | Self::MediaFileDeletion
            | Self::RecycleBinRestore
            | Self::AcquisitionSearch => JobScheduleKind::Manual,
        }
    }

    pub fn schedule_description(self) -> &'static str {
        match self {
            Self::BackgroundLibraryRefreshMovies
            | Self::BackgroundLibraryRefreshSeries
            | Self::BackgroundLibraryRefreshAnime => "Every 2 hours",
            Self::ProwlarrSync => "Every 5 minutes",
            Self::RssSync => "Scheduler-paced per indexer (15-minute target)",
            Self::SubtitleSearch => "Based on subtitle settings interval",
            Self::PluginRegistryRefresh => "Every 24 hours",
            Self::Housekeeping => "Every 24 hours",
            Self::HealthChecks => "Every 6 hours",
            Self::AutoBackup => "Daily at configured local time",
            Self::PendingReleaseProcessing => "Every minute",
            Self::StagedNzbPrune => "Every hour",
            Self::DiscoverySync => "Dynamic discovery evaluator with daily backstop",
            Self::LibraryScanMovies
            | Self::LibraryScanSeries
            | Self::LibraryScanAnime
            | Self::TitleImageCacheRefresh
            | Self::TitleDeletion
            | Self::MediaFileDeletion
            | Self::RecycleBinRestore
            | Self::AcquisitionSearch => "Manual only",
        }
    }

    pub fn interval_seconds(self) -> Option<i64> {
        match self {
            Self::BackgroundLibraryRefreshMovies
            | Self::BackgroundLibraryRefreshSeries
            | Self::BackgroundLibraryRefreshAnime => Some(2 * 3600),
            Self::ProwlarrSync => Some(5 * 60),
            Self::RssSync => Some(15 * 60),
            Self::PluginRegistryRefresh => Some(24 * 3600),
            Self::Housekeeping => Some(24 * 3600),
            Self::HealthChecks => Some(6 * 3600),
            Self::PendingReleaseProcessing => Some(60),
            Self::StagedNzbPrune => Some(3600),
            Self::DiscoverySync => Some(24 * 3600),
            _ => None,
        }
    }

    pub fn initial_delay_seconds(self) -> Option<i64> {
        match self {
            Self::BackgroundLibraryRefreshMovies
            | Self::BackgroundLibraryRefreshSeries
            | Self::BackgroundLibraryRefreshAnime => None,
            Self::SubtitleSearch => Some(120),
            Self::HealthChecks => Some(30),
            Self::DiscoverySync => Some(30 * 60),
            _ => None,
        }
    }

    pub fn manual_trigger_allowed(self) -> bool {
        !matches!(
            self,
            Self::AutoBackup
                | Self::TitleDeletion
                | Self::MediaFileDeletion
                | Self::RecycleBinRestore
                | Self::AcquisitionSearch
        )
    }

    pub fn uses_library_scan_progress(self) -> bool {
        matches!(
            self,
            Self::LibraryScanMovies
                | Self::LibraryScanSeries
                | Self::LibraryScanAnime
                | Self::BackgroundLibraryRefreshMovies
                | Self::BackgroundLibraryRefreshSeries
                | Self::BackgroundLibraryRefreshAnime
        )
    }
}

pub const ALL_JOB_KEYS: [JobKey; 16] = [
    JobKey::LibraryScanMovies,
    JobKey::LibraryScanSeries,
    JobKey::LibraryScanAnime,
    JobKey::BackgroundLibraryRefreshMovies,
    JobKey::BackgroundLibraryRefreshSeries,
    JobKey::BackgroundLibraryRefreshAnime,
    JobKey::RssSync,
    JobKey::SubtitleSearch,
    JobKey::PluginRegistryRefresh,
    JobKey::Housekeeping,
    JobKey::HealthChecks,
    JobKey::AutoBackup,
    JobKey::PendingReleaseProcessing,
    JobKey::StagedNzbPrune,
    JobKey::DiscoverySync,
    JobKey::TitleImageCacheRefresh,
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobScheduleInfo {
    pub kind: JobScheduleKind,
    pub description: String,
    pub interval_seconds: Option<i64>,
    pub initial_delay_seconds: Option<i64>,
    pub next_run_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobDefinition {
    pub key: JobKey,
    pub display_name: String,
    pub description: String,
    pub category: JobCategory,
    pub section: JobSection,
    pub manual_trigger_allowed: bool,
    pub uses_library_scan_progress: bool,
    pub schedule: JobScheduleInfo,
}

impl JobDefinition {
    pub fn from_key(key: JobKey, next_run_at: Option<DateTime<Utc>>) -> Self {
        Self {
            key,
            display_name: key.display_name().to_string(),
            description: key.description().to_string(),
            category: key.category(),
            section: key.section(),
            manual_trigger_allowed: key.manual_trigger_allowed(),
            uses_library_scan_progress: key.uses_library_scan_progress(),
            schedule: JobScheduleInfo {
                kind: key.schedule_kind(),
                description: key.schedule_description().to_string(),
                interval_seconds: key.interval_seconds(),
                initial_delay_seconds: key.initial_delay_seconds(),
                next_run_at,
            },
        }
    }
}

pub fn all_job_definitions(next_runs: &HashMap<JobKey, DateTime<Utc>>) -> Vec<JobDefinition> {
    ALL_JOB_KEYS
        .iter()
        .copied()
        .map(|key| JobDefinition::from_key(key, next_runs.get(&key).copied()))
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRunRecord {
    pub id: String,
    pub job_key: JobKey,
    pub operation_type: String,
    pub status: JobRunStatus,
    pub trigger_source: JobTriggerSource,
    pub actor_user_id: Option<String>,
    pub progress_json: Option<String>,
    pub summary_json: Option<String>,
    pub summary_text: Option<String>,
    pub error_text: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct JobRun {
    pub id: String,
    /// Internal operation metadata used to recover an interactive run's library scope.
    /// This is deliberately not exposed through the GraphQL payload.
    pub operation_type: String,
    /// Internal ownership metadata used to scope interactive job notifications.
    /// This is deliberately not exposed through the GraphQL payload.
    pub actor_user_id: Option<String>,
    pub job_key: JobKey,
    pub display_name: String,
    pub category: JobCategory,
    pub section: JobSection,
    pub status: JobRunStatus,
    pub trigger_source: JobTriggerSource,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub summary_json: Option<String>,
    pub summary_text: Option<String>,
    pub error_text: Option<String>,
    pub progress_json: Option<String>,
    pub library_scan_progress: Option<LibraryScanSession>,
}

impl JobRun {
    pub fn from_record(
        record: &JobRunRecord,
        library_scan_progress: Option<LibraryScanSession>,
    ) -> Self {
        let status = if let Some(session) = library_scan_progress.as_ref() {
            match session.status {
                LibraryScanStatus::Discovering => JobRunStatus::Discovering,
                LibraryScanStatus::Running => JobRunStatus::Running,
                LibraryScanStatus::Completed => JobRunStatus::Completed,
                LibraryScanStatus::Canceled => JobRunStatus::Warning,
                LibraryScanStatus::Warning => JobRunStatus::Warning,
                LibraryScanStatus::Failed => JobRunStatus::Failed,
            }
        } else {
            record.status
        };

        Self {
            id: record.id.clone(),
            operation_type: record.operation_type.clone(),
            actor_user_id: record.actor_user_id.clone(),
            job_key: record.job_key,
            display_name: record.job_key.display_name().to_string(),
            category: record.job_key.category(),
            section: record.job_key.section(),
            status,
            trigger_source: record.trigger_source,
            started_at: record.started_at,
            completed_at: record.completed_at,
            summary_json: record.summary_json.clone(),
            summary_text: record.summary_text.clone(),
            error_text: record.error_text.clone(),
            progress_json: record.progress_json.clone(),
            library_scan_progress,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryProbeSignature {
    pub title_id: String,
    pub path: String,
    pub probe_signature_scheme: Option<String>,
    pub probe_signature_value: Option<String>,
    pub last_probed_at: Option<DateTime<Utc>>,
    pub last_changed_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct JobRunTrackerState {
    active_runs: HashMap<String, JobRun>,
    next_run_at: HashMap<JobKey, DateTime<Utc>>,
}

#[derive(Clone)]
pub struct JobRunTracker {
    state: Arc<Mutex<JobRunTrackerState>>,
    broadcast: broadcast::Sender<JobRun>,
}

impl Default for JobRunTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRunTracker {
    pub fn new() -> Self {
        let (broadcast, _) = broadcast::channel(256);
        Self {
            state: Arc::new(Mutex::new(JobRunTrackerState::default())),
            broadcast,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<JobRun> {
        let mut source = self.broadcast.subscribe();
        let (tx, rx) = broadcast::channel(256);

        tokio::spawn(async move {
            let mut ready = VecDeque::new();
            let mut pending_library_scan_runs: HashMap<String, JobRun> = HashMap::new();
            let mut flush_timer: Option<Pin<Box<Sleep>>> = None;
            let mut closed = false;

            loop {
                if let Some(run) = ready.pop_front() {
                    if tx.send(run).is_err() {
                        break;
                    }
                    continue;
                }

                if closed {
                    break;
                }

                if let Some(timer) = flush_timer.as_mut() {
                    tokio::select! {
                        recv_result = source.recv() => {
                            match recv_result {
                                Ok(run) => {
                                    if should_coalesce_job_run_event(&run) {
                                        pending_library_scan_runs.insert(run.id.clone(), run);
                                    } else {
                                        pending_library_scan_runs.remove(&run.id);
                                        ready.push_back(run);
                                    }
                                }
                                Err(broadcast::error::RecvError::Lagged(n)) => {
                                    tracing::debug!("job_run_events: receiver lagged, skipped {n} messages");
                                }
                                Err(broadcast::error::RecvError::Closed) => {
                                    flush_pending_job_run_events(&mut pending_library_scan_runs, &mut ready);
                                    flush_timer = None;
                                    closed = true;
                                }
                            }
                        }
                        _ = timer.as_mut() => {
                            flush_pending_job_run_events(&mut pending_library_scan_runs, &mut ready);
                            flush_timer = None;
                        }
                    }
                    continue;
                }

                match source.recv().await {
                    Ok(run) => {
                        if should_coalesce_job_run_event(&run) {
                            pending_library_scan_runs.insert(run.id.clone(), run);
                            flush_timer = Some(Box::pin(tokio::time::sleep(JOB_RUN_PUSH_INTERVAL)));
                        } else if tx.send(run).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("job_run_events: receiver lagged, skipped {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        rx
    }

    pub async fn list_active(&self) -> Vec<JobRun> {
        let state = self.state.lock().await;
        let mut runs = state.active_runs.values().cloned().collect::<Vec<_>>();
        runs.sort_by_key(|run| run.started_at);
        runs
    }

    pub async fn has_active_job(&self, job_key: JobKey) -> bool {
        let state = self.state.lock().await;
        state.active_runs.values().any(|run| run.job_key == job_key)
    }

    pub async fn active_run_for_job(&self, job_key: JobKey) -> Option<JobRun> {
        let state = self.state.lock().await;
        state
            .active_runs
            .values()
            .find(|run| run.job_key == job_key)
            .cloned()
    }

    pub async fn all_next_runs(&self) -> HashMap<JobKey, DateTime<Utc>> {
        let state = self.state.lock().await;
        state.next_run_at.clone()
    }

    pub async fn upsert_active_run(&self, run: JobRun) {
        {
            let mut state = self.state.lock().await;
            if run.status.is_terminal() {
                state.active_runs.remove(&run.id);
            } else {
                state.active_runs.insert(run.id.clone(), run.clone());
            }
        }
        let _ = self.broadcast.send(run);
    }

    pub async fn merge_library_scan_progress(&self, session: LibraryScanSession) {
        let maybe_run = {
            let mut state = self.state.lock().await;
            let Some(run) = state.active_runs.get_mut(&session.session_id) else {
                return;
            };
            run.library_scan_progress = Some(session.clone());
            run.status = match session.status {
                LibraryScanStatus::Discovering => JobRunStatus::Discovering,
                LibraryScanStatus::Running => JobRunStatus::Running,
                LibraryScanStatus::Completed => JobRunStatus::Completed,
                LibraryScanStatus::Canceled => JobRunStatus::Warning,
                LibraryScanStatus::Warning => JobRunStatus::Warning,
                LibraryScanStatus::Failed => JobRunStatus::Failed,
            };
            if run.status.is_terminal() {
                run.completed_at = Some(session.updated_at);
            }
            run.clone()
        };
        let _ = self.broadcast.send(maybe_run);
    }

    pub async fn set_next_run_at(&self, job_key: JobKey, next_run_at: DateTime<Utc>) {
        let mut state = self.state.lock().await;
        state.next_run_at.insert(job_key, next_run_at);
    }

    pub async fn clear_next_run_at(&self, job_key: JobKey) {
        let mut state = self.state.lock().await;
        state.next_run_at.remove(&job_key);
    }

    pub async fn next_run_at(&self, job_key: JobKey) -> Option<DateTime<Utc>> {
        let state = self.state.lock().await;
        state.next_run_at.get(&job_key).copied()
    }
}

fn should_coalesce_job_run_event(run: &JobRun) -> bool {
    run.library_scan_progress.is_some()
        && matches!(
            run.status,
            JobRunStatus::Discovering | JobRunStatus::Running
        )
}

fn flush_pending_job_run_events(
    pending_library_scan_runs: &mut HashMap<String, JobRun>,
    ready: &mut VecDeque<JobRun>,
) {
    if pending_library_scan_runs.is_empty() {
        return;
    }

    let mut pending = pending_library_scan_runs
        .drain()
        .map(|(_, run)| run)
        .collect::<Vec<_>>();
    pending.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    ready.extend(pending);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LibraryScanMode, LibraryScanPhaseProgress, MediaFacet};

    #[test]
    fn background_library_refresh_definitions_advertise_two_hour_schedule() {
        let definition = JobDefinition::from_key(JobKey::BackgroundLibraryRefreshMovies, None);

        assert_eq!(definition.schedule.description, "Every 2 hours");
        assert_eq!(definition.schedule.initial_delay_seconds, None);
    }

    #[test]
    fn title_image_cache_refresh_is_registered_as_manual_system_job() {
        let definition = JobDefinition::from_key(JobKey::TitleImageCacheRefresh, None);

        assert_eq!(
            JobKey::TitleImageCacheRefresh.as_str(),
            "title_image_cache_refresh"
        );
        assert_eq!(
            JobKey::parse("title_image_cache_refresh"),
            Some(JobKey::TitleImageCacheRefresh)
        );
        assert!(ALL_JOB_KEYS.contains(&JobKey::TitleImageCacheRefresh));
        assert_eq!(definition.display_name, "Title Image Cache Refresh");
        assert_eq!(definition.category, JobCategory::System);
        assert_eq!(definition.schedule.kind, JobScheduleKind::Manual);
        assert_eq!(definition.schedule.description, "Manual only");
        assert!(definition.manual_trigger_allowed);
    }

    #[tokio::test]
    async fn terminal_library_scan_merge_keeps_run_active_until_final_upsert() {
        let tracker = JobRunTracker::new();
        let now = Utc::now();
        let run = JobRun {
            id: "run-1".to_string(),
            operation_type: "background_library_refresh_series".to_string(),
            actor_user_id: None,
            job_key: JobKey::BackgroundLibraryRefreshSeries,
            display_name: JobKey::BackgroundLibraryRefreshSeries
                .display_name()
                .to_string(),
            category: JobCategory::Library,
            section: JobSection::Primary,
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::ScheduledInterval,
            started_at: now,
            completed_at: None,
            summary_json: None,
            summary_text: None,
            error_text: None,
            progress_json: None,
            library_scan_progress: None,
        };
        tracker.upsert_active_run(run.clone()).await;

        tracker
            .merge_library_scan_progress(LibraryScanSession {
                session_id: run.id.clone(),
                facet: MediaFacet::Series,
                library_id: None,
                mode: LibraryScanMode::Additive,
                status: LibraryScanStatus::Completed,
                started_at: now,
                updated_at: now,
                found_titles: 1,
                title_match_total_known: true,
                metadata_total_known: true,
                file_total_known: true,
                title_match_progress: LibraryScanPhaseProgress {
                    total: 1,
                    completed: 1,
                    failed: 0,
                },
                metadata_progress: LibraryScanPhaseProgress {
                    total: 0,
                    completed: 0,
                    failed: 0,
                },
                file_progress: LibraryScanPhaseProgress {
                    total: 0,
                    completed: 0,
                    failed: 0,
                },
                summary: None,
                warning_message: None,
            })
            .await;

        let active_after_merge = tracker.list_active().await;
        assert_eq!(active_after_merge.len(), 1);
        assert_eq!(active_after_merge[0].status, JobRunStatus::Completed);

        tracker
            .upsert_active_run(active_after_merge[0].clone())
            .await;
        assert!(tracker.list_active().await.is_empty());
    }
}
