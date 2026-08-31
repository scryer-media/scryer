//! TrackedDownloads — scryer-side download lifecycle state machine.
//!
//! Maintains an in-memory cache of active downloads, each enriched with title
//! resolution metadata and driven through a workflow state machine independent
//! of the download client's reported status.

use chrono::{DateTime, Utc};
use scryer_domain::{
    CompletedDownload, DownloadQueueItem, DownloadQueueState, Title, TitleMatchType,
    TrackedDownloadState, TrackedDownloadStatus, download_identity::DownloadId,
};
use std::collections::{HashMap, HashSet};
use tokio::sync::{mpsc, oneshot};

use crate::{
    AppResult, AppUseCase, ClientJobLocator, DownloadSubmission, DownloadSubmissionActorSnapshot,
    DownloadSubmissionIdentity, SubmissionScope,
};

const DEFAULT_TRACKED_DOWNLOAD_CACHE_TTL_HOURS: i64 = 24;
const DEFAULT_TRACKED_DOWNLOAD_CACHE_MAX_ENTRIES: usize = 5_000;

// ── TrackedDownload ──────────────────────────────────────────────────────────

/// A download being tracked through scryer's import workflow.
#[derive(Clone, Debug)]
pub struct TrackedDownload {
    /// Canonical identity selected by the download registry for this client job.
    ///
    /// On the narrow registry-outage fallback path this is an opaque sentinel;
    /// that path remains keyed by the legacy identifier.
    pub download_id: DownloadId,
    /// Composite key scoped to the configured client when available.
    pub id: String,
    pub client_id: String,
    pub client_type: String,
    /// Latest snapshot from the download client.
    pub client_item: DownloadQueueItem,
    /// Exact completed-download source retained for manual import after the
    /// client stops exposing the item in a live history snapshot.
    pub completed_source: Option<CompletedDownload>,
    /// Scryer's workflow state (independent of client status).
    pub state: TrackedDownloadState,
    /// Health/warning overlay.
    pub status: TrackedDownloadStatus,
    /// Human-readable status messages.
    pub status_messages: Vec<String>,
    /// Resolved scryer title.
    pub title_id: Option<String>,
    pub facet: Option<String>,
    /// Release name from grab history (fallback parsing source).
    pub source_title: Option<String>,
    pub indexer: Option<String>,
    pub added_at: Option<DateTime<Utc>>,
    /// Whether the user has been notified about manual intervention.
    pub notified_manual_interaction: bool,
    /// How the title was resolved.
    pub match_type: TitleMatchType,
    /// Whether this download is still visible in the client.
    pub is_trackable: bool,
    /// Whether import() has been called at least once. Prevents check() from
    /// re-evaluating a post-import ImportBlocked back to ImportPending.
    pub import_attempted: bool,
    /// Completed item is waiting for the client to expose matching history.
    pub waiting_for_completed_history: bool,
    /// When a completed download path first became unavailable.
    pub path_missing_since: Option<DateTime<Utc>>,
    /// Runtime-only retry state for completed imports that temporarily contain no videos.
    pub no_video_import_retry: Option<NoVideoImportRetryState>,
    /// Runtime-only backoff for an approved import whose *execution* failed
    /// (transfer/IO/permissions/root missing). Sonarr treats every such
    /// failure as `Skipped` and re-attempts on its refresh cadence; this state
    /// gives Scryer's faster poller a capped backoff instead of a hot loop.
    pub import_execution_retry: Option<ImportExecutionRetryState>,
    /// Runtime-only reason this completed download is held back and hidden.
    /// Never persisted as a tracked-download outcome.
    pub import_hold: Option<ImportHold>,
    /// Manual failure actions can record the failure without reacquiring.
    pub skip_reacquire_on_failure: bool,
    /// Runtime-only marker that this failed row must use the import-style
    /// seeding-aware cleanup path. Burned import rejections and warnings that
    /// outlived a completed payload both set it.
    pub burned_by_import_gate: bool,
    /// When this download first went missing from a pruning snapshot.
    ///
    /// A snapshot can be PARTIAL without saying so: the router degrades
    /// per-client — a feedback read that times out puts that client on an
    /// exponential backoff (15s doubling to a 120s cap) during which its reads
    /// are silently skipped — yet the poller still treated every snapshot as
    /// authoritative and pruned whatever was absent. One transient timeout at
    /// the wrong moment therefore erased live tracked downloads, and any that
    /// completed during the blackout were never imported. Absence only counts
    /// once it has persisted beyond the grace window, which must outlast the
    /// router's maximum backoff.
    pub snapshot_missing_since: Option<DateTime<Utc>>,
}

/// Content-only reason a completed download is temporarily held from import.
/// This is never an ownership or provenance classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportHold {
    NoImportableVideo,
    ExternalManager,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoVideoImportRetryState {
    pub signature: NoVideoImportSourceSignature,
    pub attempts: u8,
    pub next_retry_at: DateTime<Utc>,
}

/// Backoff for an approved import whose execution failed (Sonarr's `Skipped`).
/// Attempts are never capped: like Sonarr, the retry continues until the
/// import succeeds or the operator removes/ignores/marks the download failed;
/// only the delay between attempts grows (see `import_execution_retry_delay`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportExecutionRetryState {
    pub attempts: u32,
    pub next_retry_at: DateTime<Utc>,
}

/// Delay before the next automatic re-attempt of an import whose execution
/// failed: 30 s, then 2 min, then 5 min, then 15 min for every later attempt.
pub fn import_execution_retry_delay(attempts: u32) -> chrono::Duration {
    match attempts {
        0 | 1 => chrono::Duration::seconds(30),
        2 => chrono::Duration::minutes(2),
        3 => chrono::Duration::minutes(5),
        _ => chrono::Duration::minutes(15),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoVideoImportSourceSignature {
    pub source_path: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub latest_mtime: Option<DateTime<Utc>>,
}

/// `Eq` is not derivable: the client item carries an observed seed ratio
/// (`f64`).
#[derive(Clone, Debug, PartialEq)]
pub struct TrackedDownloadQueueMetadata {
    pub client_item: DownloadQueueItem,
    pub client_id: String,
    pub client_type: String,
    pub title_id: Option<String>,
    pub facet: Option<String>,
    pub source_title: Option<String>,
    pub state: TrackedDownloadState,
    pub status: TrackedDownloadStatus,
    pub status_messages: Vec<String>,
    pub match_type: TitleMatchType,
    pub import_hold: Option<ImportHold>,
}

impl From<&TrackedDownload> for TrackedDownloadQueueMetadata {
    fn from(value: &TrackedDownload) -> Self {
        Self {
            client_item: value.client_item.clone(),
            client_id: value.client_id.clone(),
            client_type: value.client_type.clone(),
            title_id: value.title_id.clone(),
            facet: value.facet.clone(),
            source_title: value.source_title.clone(),
            state: value.state,
            status: value.status,
            status_messages: value.status_messages.clone(),
            match_type: value.match_type,
            import_hold: value.import_hold,
        }
    }
}

impl TrackedDownload {
    pub(crate) fn canonical_download_id(&self) -> Option<&DownloadId> {
        Some(&self.download_id)
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.status = TrackedDownloadStatus::Warning;
        self.status_messages.push(message.into());
    }

    pub fn clear_warnings(&mut self) {
        self.status = TrackedDownloadStatus::Ok;
        self.status_messages.clear();
    }

    pub fn fail(&mut self) {
        self.status = TrackedDownloadStatus::Error;
        self.state = TrackedDownloadState::FailedPending;
    }

    pub(crate) fn merge_background_work_state_from(&mut self, finished: TrackedDownload) {
        self.state = finished.state;
        self.status = finished.status;
        self.status_messages = finished.status_messages;
        self.title_id = finished.title_id;
        self.facet = finished.facet;
        self.source_title = finished.source_title;
        self.indexer = finished.indexer;
        self.added_at = finished.added_at;
        self.notified_manual_interaction = finished.notified_manual_interaction;
        self.match_type = finished.match_type;
        self.import_attempted = finished.import_attempted;
        self.waiting_for_completed_history = finished.waiting_for_completed_history;
        self.path_missing_since = finished.path_missing_since;
        self.no_video_import_retry = finished.no_video_import_retry;
        self.import_execution_retry = finished.import_execution_retry;
        self.import_hold = finished.import_hold;
        self.completed_source = finished.completed_source;
        self.burned_by_import_gate = finished.burned_by_import_gate;
    }

    pub(crate) fn reset_for_import_retry(&mut self) {
        self.state = TrackedDownloadState::ImportPending;
        self.status = TrackedDownloadStatus::Ok;
        self.status_messages.clear();
        self.import_attempted = false;
        self.waiting_for_completed_history = false;
        self.path_missing_since = None;
        self.no_video_import_retry = None;
        self.import_execution_retry = None;
        self.import_hold = None;
        self.skip_reacquire_on_failure = false;
        self.burned_by_import_gate = false;
    }

    /// Whether an automatic import re-attempt is still inside its backoff
    /// window (either the no-video probe or an execution-failure retry).
    pub(crate) fn import_retry_deferred(&self, now: DateTime<Utc>) -> bool {
        self.no_video_import_retry
            .as_ref()
            .is_some_and(|retry| retry.next_retry_at > now)
            || self
                .import_execution_retry
                .as_ref()
                .is_some_and(|retry| retry.next_retry_at > now)
    }

    /// Schedules the next automatic re-attempt after an approved import failed
    /// to execute; returns the attempt number just recorded.
    pub(crate) fn schedule_import_execution_retry(
        &mut self,
        now: DateTime<Utc>,
        message: impl FnOnce(u32, DateTime<Utc>) -> String,
    ) -> u32 {
        let attempts = self
            .import_execution_retry
            .as_ref()
            .map(|retry| retry.attempts.saturating_add(1))
            .unwrap_or(1);
        let next_retry_at = now + import_execution_retry_delay(attempts);
        self.import_execution_retry = Some(ImportExecutionRetryState {
            attempts,
            next_retry_at,
        });
        self.state = TrackedDownloadState::ImportPending;
        self.waiting_for_completed_history = false;
        self.status = TrackedDownloadStatus::Warning;
        self.status_messages = vec![message(attempts, next_retry_at)];
        attempts
    }

    pub(crate) fn clear_import_execution_retry(&mut self) {
        self.import_execution_retry = None;
    }

    pub(crate) fn schedule_no_video_import_retry(
        &mut self,
        signature: NoVideoImportSourceSignature,
        attempts: u8,
        next_retry_at: DateTime<Utc>,
        message: impl Into<String>,
    ) {
        self.no_video_import_retry = Some(NoVideoImportRetryState {
            signature,
            attempts,
            next_retry_at,
        });
        self.state = TrackedDownloadState::ImportPending;
        self.waiting_for_completed_history = false;
        self.status = TrackedDownloadStatus::Warning;
        self.status_messages = vec![message.into()];
    }

    pub(crate) fn block_no_video_import_after_retries(&mut self, message: impl Into<String>) {
        self.no_video_import_retry = None;
        self.state = TrackedDownloadState::ImportBlocked;
        self.waiting_for_completed_history = false;
        self.status = TrackedDownloadStatus::Warning;
        self.status_messages = vec![message.into()];
    }

    pub(crate) fn clear_no_video_import_retry(&mut self) {
        self.no_video_import_retry = None;
    }
}

// ── TrackedDownloadService ───────────────────────────────────────────────────

pub(crate) const IMPORT_GATE_REJECTED_TRACKED_STATE_REASON: &str = "import_gate_rejected";
pub(crate) const WARNING_TIMEOUT_TRACKED_STATE_REASON: &str = "warning_timeout";
pub(crate) const WARNING_TIMEOUT_STATUS_MESSAGE_PREFIX: &str =
    "download client warning persisted for 24h:";

/// Durable reason an import-blocked download requires operator attention.
///
/// These values are persisted in `download_identity_states.reason`; parsing is
/// deliberately restricted to that storage boundary rather than user-facing
/// status text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportBlockedReason {
    PreImport,
    AfterImport,
    ManualMappingRequired,
    MissingCompletedHistoryIdentity,
    AssignedTitleMissing,
    CompletedTitleIdentityMismatch,
    UnverifiedAlreadyImported,
}

impl ImportBlockedReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PreImport => "import_blocked_pre_import",
            Self::AfterImport => "import_blocked_after_import",
            Self::ManualMappingRequired => "manual_import_mapping_required",
            Self::MissingCompletedHistoryIdentity => "missing_completed_history_identity",
            Self::AssignedTitleMissing => "assigned_title_missing",
            Self::CompletedTitleIdentityMismatch => "completed_title_identity_mismatch",
            Self::UnverifiedAlreadyImported => "unverified_already_imported",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "import_blocked_pre_import" => Some(Self::PreImport),
            "import_blocked_after_import" => Some(Self::AfterImport),
            "manual_import_mapping_required" => Some(Self::ManualMappingRequired),
            "missing_completed_history_identity" => Some(Self::MissingCompletedHistoryIdentity),
            "assigned_title_missing" => Some(Self::AssignedTitleMissing),
            "completed_title_identity_mismatch" => Some(Self::CompletedTitleIdentityMismatch),
            "unverified_already_imported" => Some(Self::UnverifiedAlreadyImported),
            _ => None,
        }
    }

    pub(crate) const fn reopens_for_verification(self) -> bool {
        matches!(self, Self::UnverifiedAlreadyImported)
    }
}

/// In-memory cache of tracked downloads with title resolution and state management.
///
/// Warning timers are intentionally runtime-only: a restart gives a still-warned
/// download a fresh 24-hour window.
#[derive(Default)]
pub struct TrackedDownloadService {
    /// Normal tracked-download path, keyed only by canonical identity.
    cache: HashMap<DownloadId, TrackedDownload>,
    last_seen_at: HashMap<DownloadId, DateTime<Utc>>,
    warning_since: HashMap<DownloadId, DateTime<Utc>>,
}

impl TrackedDownloadService {
    /// Sonarr has no equivalent timeout. A client warning that survives for a
    /// day (for example stalled data, missing files, or a disk error) is a
    /// failed download so the acquisition scope can move on.
    pub(crate) const WARNING_FAILURE_TIMEOUT: chrono::Duration = chrono::Duration::hours(24);

    pub fn new() -> Self {
        Self::default()
    }

    /// Create or update a tracked download from a client item snapshot.
    ///
    /// On first see: resolves title, checks for terminal state in DB.
    /// On update: refreshes client_item but preserves scryer state if past Downloading.
    pub async fn track(&mut self, app: &AppUseCase, client_item: DownloadQueueItem) {
        let observed_job = crate::download_identity::observed_queue_item_job(&client_item);
        let resolved_download_id =
            crate::download_identity::resolve_observed_client_job(app, observed_job.clone()).await;
        let id = tracked_download_id_for_item(&client_item);
        let download_id = match resolved_download_id {
            crate::download_identity::ObservedClientJobResolution::Resolved(download_id) => {
                download_id
            }
            crate::download_identity::ObservedClientJobResolution::Conflict => {
                tracing::warn!(
                    client_id = client_item.client_id.as_str(),
                    client_type = client_item.client_type.as_str(),
                    download_client_item_id = client_item.download_client_item_id.as_str(),
                    "conflicting canonical download identity; skipping item for this cycle"
                );
                return;
            }
            crate::download_identity::ObservedClientJobResolution::Unavailable => match app
                .services
                .workflow
                .download_registry
                .find_active_binding_by_locator(&observed_job.locator)
                .await
            {
                Ok(Some(binding)) => binding.download_id,
                Ok(None) => {
                    tracing::warn!(
                        client_id = client_item.client_id.as_str(),
                        client_type = client_item.client_type.as_str(),
                        download_client_item_id = client_item.download_client_item_id.as_str(),
                        "failed to recover canonical download identity; skipping item for this cycle"
                    );
                    return;
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        client_id = client_item.client_id.as_str(),
                        client_type = client_item.client_type.as_str(),
                        download_client_item_id = client_item.download_client_item_id.as_str(),
                        "failed to recover canonical download identity; skipping item for this cycle"
                    );
                    return;
                }
            },
        };

        let now = Utc::now();
        self.last_seen_at.insert(download_id, now);
        if client_item.state != DownloadQueueState::Warning {
            self.warning_since.remove(&download_id);
        }
        let existing = self.cache.get_mut(&download_id);

        if let Some(existing) = existing {
            let matcher_dirty = app
                .runtime
                .catalog
                .monitored_title_matcher
                .read()
                .await
                .dirty;
            let should_reresolve = should_reresolve_title(existing, &client_item, matcher_dirty);
            // Update the client snapshot but preserve scryer state if not Downloading.
            if existing.state == TrackedDownloadState::Downloading {
                existing.status = TrackedDownloadStatus::Ok;
                existing.status_messages.clear();
            }
            let mut client_item = client_item;
            // A refresh that carries no seeding observation must not erase the
            // one we have. Several sources for the same download report
            // different shapes (a history row framed as a completed download
            // has no seeding fields at all), and a torrent whose observation
            // blinked out would be read as "unknown" and held forever. A
            // *present* observation always wins, so this retains, it does not
            // staleness-lock.
            if client_item.seeding.is_none() {
                client_item.seeding = existing.client_item.seeding.clone();
            }
            existing.client_item = client_item;
            existing.is_trackable = true;
            if should_reresolve {
                Self::resolve_title(app, existing).await;
            }
            return;
        }

        // First time seeing this download — build, resolve, and insert.
        let td = Self::build_new_tracked_download(app, download_id, id.clone(), client_item).await;
        self.cache.insert(download_id, td);
        self.prune_cache();
    }

    /// Build a new TrackedDownload, resolving title and reconstructing state.
    async fn build_new_tracked_download(
        app: &AppUseCase,
        download_id: DownloadId,
        id: String,
        client_item: DownloadQueueItem,
    ) -> TrackedDownload {
        let mut td = TrackedDownload {
            download_id,
            id,
            client_id: client_item.client_id.clone(),
            client_type: client_item.client_type.clone(),
            title_id: client_item.title_id.clone(),
            facet: client_item.facet.clone(),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Unmatched,
            is_trackable: true,
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            client_item,
            completed_source: None,
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_execution_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
            burned_by_import_gate: false,
            snapshot_missing_since: None,
        };

        Self::resolve_title(app, &mut td).await;
        Self::reconstruct_state(app, &mut td).await;
        td
    }

    pub fn find(&self, id: &str) -> Option<&TrackedDownload> {
        self.cache.values().find(|tracked| tracked.id == id)
    }

    #[cfg(test)]
    pub(crate) fn insert_for_tests(&mut self, tracked: TrackedDownload) -> DownloadId {
        let download_id = tracked.download_id;
        self.last_seen_at.insert(download_id, Utc::now());
        self.cache.insert(download_id, tracked);
        download_id
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut TrackedDownload> {
        let download_id = self
            .cache
            .iter()
            .find_map(|(download_id, tracked)| (tracked.id == id).then_some(*download_id));
        download_id.and_then(|download_id| self.cache.get_mut(&download_id))
    }

    pub fn resolve_cached_id(&self, requested_id: &str) -> Option<String> {
        self.cache.values().find_map(|tracked| {
            (tracked.id == requested_id
                || tracked_download_matches_source_id(tracked, requested_id))
            .then(|| tracked.id.clone())
        })
    }

    pub fn get_all(&self) -> Vec<&TrackedDownload> {
        self.cache.values().collect()
    }

    pub fn completed_source_for_identity(
        &self,
        identity: &ClientJobLocator,
    ) -> Option<CompletedDownload> {
        self.cache
            .values()
            .find(|tracked| {
                ClientJobLocator::new(
                    Some(tracked.client_id.as_str()),
                    tracked.client_type.as_str(),
                    tracked.client_item.download_client_item_id.as_str(),
                ) == *identity
            })
            .and_then(|tracked| tracked.completed_source.clone())
    }

    pub fn cached_id_for_source_identity(&self, identity: &ClientJobLocator) -> Option<String> {
        self.cache.values().find_map(|tracked| {
            (ClientJobLocator::new(
                Some(tracked.client_id.as_str()),
                tracked.client_type.as_str(),
                tracked.client_item.download_client_item_id.as_str(),
            ) == *identity)
                .then(|| tracked.id.clone())
        })
    }

    pub fn cached_id_for_source_identity_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &ClientJobLocator,
    ) -> Option<String> {
        let _identity = identity;
        canonical_download_id.and_then(|canonical_download_id| {
            self.cache.values().find_map(|tracked| {
                (tracked.canonical_download_id() == Some(canonical_download_id))
                    .then(|| tracked.id.clone())
            })
        })
    }

    pub fn cached_id_for_canonical_download_id(
        &self,
        canonical_download_id: &DownloadId,
    ) -> Option<String> {
        self.cache.values().find_map(|tracked| {
            (tracked.canonical_download_id() == Some(canonical_download_id))
                .then(|| tracked.id.clone())
        })
    }

    pub fn get_trackable(&self) -> Vec<&TrackedDownload> {
        self.cache
            .values()
            .filter(|td| td.is_trackable && !td.state.is_terminal())
            .collect()
    }

    pub fn get_trackable_ids(&self) -> Vec<String> {
        self.cache
            .values()
            .filter(|td| td.is_trackable && !td.state.is_terminal())
            .map(|td| td.id.clone())
            .collect()
    }

    /// Convert an actionable client warning into the existing failed-download
    /// path after it has remained continuously warned for a full day.
    ///
    /// `now` is passed in so the timeout stays deterministic in tests.
    ///
    /// `timeout_applies` is decided by the caller (`warning_timeout_applies`):
    /// a torrent grabbed under a seeding profile is never timed out — its
    /// indexer's rules own it and the warning stays visible for the operator.
    /// When it is `false` the clock is dropped and nothing changes.
    pub(crate) fn fail_persistent_warning(
        &mut self,
        id: &str,
        now: DateTime<Utc>,
        timeout_applies: bool,
    ) -> bool {
        let canonical_download_id = self
            .cache
            .iter()
            .find_map(|(download_id, tracked)| (tracked.id == id).then_some(*download_id));
        let Some(canonical_download_id) = canonical_download_id else {
            return false;
        };
        let tracked = self
            .cache
            .get(&canonical_download_id)
            .expect("tracked download was present when warning timeout started");

        let warning_is_actionable = tracked.client_item.is_scryer_origin
            && matches!(
                tracked.state,
                TrackedDownloadState::Downloading
                    | TrackedDownloadState::ImportPending
                    | TrackedDownloadState::ImportBlocked
            )
            && tracked.client_item.state == DownloadQueueState::Warning;
        if !warning_is_actionable || !timeout_applies {
            self.warning_since.remove(&canonical_download_id);
            return false;
        }

        let since = *self
            .warning_since
            .entry(canonical_download_id)
            .or_insert(now);
        if now - since < Self::WARNING_FAILURE_TIMEOUT {
            return false;
        }

        let payload_completed = matches!(
            tracked.state,
            TrackedDownloadState::ImportPending | TrackedDownloadState::ImportBlocked
        );
        let attention_reason = tracked
            .client_item
            .attention_reason
            .as_deref()
            .filter(|reason| !reason.trim().is_empty())
            .unwrap_or("no reason supplied")
            .to_owned();
        self.warning_since.remove(&canonical_download_id);

        let tracked = self
            .cache
            .get_mut(&canonical_download_id)
            .expect("tracked download was present when warning timeout was evaluated");
        tracked.state = TrackedDownloadState::FailedPending;
        tracked.status = TrackedDownloadStatus::Error;
        if !tracked
            .status_messages
            .iter()
            .any(|message| message.starts_with(WARNING_TIMEOUT_STATUS_MESSAGE_PREFIX))
        {
            tracked.status_messages.push(format!(
                "{WARNING_TIMEOUT_STATUS_MESSAGE_PREFIX} {attention_reason}"
            ));
        }
        // A completed torrent may still owe seeding. Reuse the burned-import
        // cleanup rail so only this path reaches its terminal removal gate.
        tracked.burned_by_import_gate = payload_completed;
        true
    }

    /// States whose work continues after a queue/history item legitimately
    /// rolls out of the client's rolling activity window.
    pub(crate) fn should_preserve_tracking(state: TrackedDownloadState) -> bool {
        matches!(
            state,
            TrackedDownloadState::ImportPending
                | TrackedDownloadState::Importing
                | TrackedDownloadState::ImportBlocked
                | TrackedDownloadState::FailedPending
        )
    }

    /// Mark downloads no longer visible in any client as untrackable.
    pub fn update_trackable(&mut self, seen_ids: &HashSet<String>) -> Vec<ClientJobLocator> {
        let mut unavailable_sources = Vec::new();
        for td in self.cache.values_mut() {
            if Self::should_preserve_tracking(td.state) {
                td.snapshot_missing_since = None;
                continue;
            }
            if td.is_trackable && !seen_ids.contains(&td.id) {
                td.is_trackable = false;
                unavailable_sources.push(ClientJobLocator::new(
                    Some(&td.client_id),
                    &td.client_type,
                    &td.client_item.download_client_item_id,
                ));
            }
        }
        self.clear_untracked_warning_clocks();
        self.prune_cache();
        unavailable_sources
    }

    /// Mark downloads no longer visible in non-excluded clients as untrackable.
    /// How long a download may stay missing from pruning snapshots before it is
    /// actually pruned.
    ///
    /// Must OUTLAST the router's maximum feedback backoff (120s): while a
    /// client is backing off, its reads are skipped and its items are absent
    /// from every snapshot, so any smaller grace re-creates the erase-on-blip
    /// bug this exists to fix. The cost of the debounce is that a download
    /// removed in the client's own UI lingers in Activity for up to this long.
    pub(crate) const SNAPSHOT_ABSENCE_PRUNE_GRACE_SECS: i64 = 150;

    pub fn update_trackable_excluding_client_types(
        &mut self,
        seen_ids: &HashSet<String>,
        excluded_client_types: &[&str],
    ) -> Vec<ClientJobLocator> {
        self.update_trackable_excluding_client_types_for_authoritative_clients(
            seen_ids,
            excluded_client_types,
            None,
        )
    }

    /// Mark jobs absent only when their client completed both sides of the
    /// snapshot read. A non-authoritative client is treated as seen so a
    /// previous absence debounce cannot mature during an outage.
    pub fn update_trackable_excluding_client_types_for_authoritative_clients(
        &mut self,
        seen_ids: &HashSet<String>,
        excluded_client_types: &[&str],
        authoritative_client_ids: Option<&HashSet<String>>,
    ) -> Vec<ClientJobLocator> {
        let now = Utc::now();
        let mut unavailable_sources = Vec::new();
        for td in self.cache.values_mut() {
            if tracked_client_type_is_excluded(&td.client_type, excluded_client_types) {
                continue;
            }
            if authoritative_client_ids
                .is_some_and(|client_ids| !client_ids.contains(&td.client_id))
            {
                td.snapshot_missing_since = None;
                continue;
            }
            if seen_ids.contains(&td.id) {
                td.snapshot_missing_since = None;
                continue;
            }
            if Self::should_preserve_tracking(td.state) {
                td.snapshot_missing_since = None;
                continue;
            }
            if td.is_trackable && snapshot_absence_exceeds_grace(td, now) {
                td.is_trackable = false;
                unavailable_sources.push(ClientJobLocator::new(
                    Some(&td.client_id),
                    &td.client_type,
                    &td.client_item.download_client_item_id,
                ));
            }
        }
        self.clear_untracked_warning_clocks();
        self.prune_cache();
        unavailable_sources
    }

    /// Mark downloads absent from an authoritative client-scoped snapshot as untrackable.
    pub fn update_trackable_for_scope(
        &mut self,
        seen_ids: &HashSet<String>,
        scope: &TrackedDownloadSnapshotScope,
    ) -> Vec<ClientJobLocator> {
        self.update_trackable_for_scope_for_authoritative_clients(seen_ids, scope, None)
    }

    /// The scope is authoritative only when its client completed both queue
    /// and activity reads. Non-authoritative scoped jobs are treated as seen.
    pub fn update_trackable_for_scope_for_authoritative_clients(
        &mut self,
        seen_ids: &HashSet<String>,
        scope: &TrackedDownloadSnapshotScope,
        authoritative_client_ids: Option<&HashSet<String>>,
    ) -> Vec<ClientJobLocator> {
        let TrackedDownloadSnapshotScope::AuthoritativeForClient {
            client_id,
            client_type,
        } = scope
        else {
            return Vec::new();
        };

        let now = Utc::now();
        let mut unavailable_sources = Vec::new();
        for td in self.cache.values_mut() {
            if !tracked_matches_snapshot_scope(td, client_id.as_deref(), client_type) {
                continue;
            }
            if authoritative_client_ids
                .is_some_and(|client_ids| !client_ids.contains(&td.client_id))
            {
                td.snapshot_missing_since = None;
                continue;
            }
            if seen_ids.contains(&td.id) {
                td.snapshot_missing_since = None;
                continue;
            }
            if Self::should_preserve_tracking(td.state) {
                td.snapshot_missing_since = None;
                continue;
            }
            if td.is_trackable && snapshot_absence_exceeds_grace(td, now) {
                td.is_trackable = false;
                unavailable_sources.push(ClientJobLocator::new(
                    Some(&td.client_id),
                    &td.client_type,
                    &td.client_item.download_client_item_id,
                ));
            }
        }
        self.clear_untracked_warning_clocks();
        self.prune_cache();
        unavailable_sources
    }

    /// Remove a download from the cache (after terminal state).
    pub fn stop_tracking(&mut self, id: &str) {
        if let Some(download_id) = self
            .cache
            .iter()
            .find_map(|(download_id, tracked)| (tracked.id == id).then_some(*download_id))
        {
            self.cache.remove(&download_id);
            self.last_seen_at.remove(&download_id);
            self.warning_since.remove(&download_id);
        }
    }

    fn clear_untracked_warning_clocks(&mut self) {
        self.warning_since.retain(|id, _| {
            self.cache
                .get(id)
                .is_some_and(|tracked| tracked.is_trackable)
        });
    }

    fn prune_cache(&mut self) {
        let ttl = tracked_download_cache_ttl();
        let stale_cutoff = Utc::now() - ttl;
        let max_entries = tracked_download_cache_max_entries();
        self.prune_cache_with_limits(stale_cutoff, max_entries);
    }

    fn prune_cache_with_limits(&mut self, stale_cutoff: DateTime<Utc>, max_entries: usize) {
        let last_seen_at = &self.last_seen_at;
        self.cache.retain(|id, tracked| {
            tracked.is_trackable
                || last_seen_at
                    .get(id)
                    .is_none_or(|last_seen| *last_seen >= stale_cutoff)
        });
        if self.cache.len() > max_entries {
            let mut eviction_candidates = self
                .cache
                .iter()
                .filter(|(_, tracked)| tracked_download_can_be_evicted_for_cache_pressure(tracked))
                .map(|(id, _)| {
                    (
                        self.last_seen_at.get(id).copied().unwrap_or(stale_cutoff),
                        *id,
                    )
                })
                .collect::<Vec<_>>();
            eviction_candidates.sort_by_key(|(last_seen, _)| *last_seen);
            let overage = self.cache.len().saturating_sub(max_entries);
            for (_, id) in eviction_candidates.into_iter().take(overage) {
                self.cache.remove(&id);
            }
        }

        self.last_seen_at
            .retain(|id, _| self.cache.contains_key(id));
        self.warning_since
            .retain(|id, _| self.cache.contains_key(id));
    }

    /// Persist a terminal state to download_submissions.
    pub async fn persist_terminal_state(
        &self,
        app: &AppUseCase,
        id: &str,
        state: TrackedDownloadState,
    ) -> bool {
        if !state.is_terminal() {
            return true;
        }
        let Some(td) = self.find(id) else {
            return false;
        };
        let (reason, detail) = if state == TrackedDownloadState::Failed && td.burned_by_import_gate
        {
            let warning_timeout = td
                .status_messages
                .iter()
                .find(|message| message.starts_with(WARNING_TIMEOUT_STATUS_MESSAGE_PREFIX));
            (
                Some(if warning_timeout.is_some() {
                    WARNING_TIMEOUT_TRACKED_STATE_REASON
                } else {
                    IMPORT_GATE_REJECTED_TRACKED_STATE_REASON
                }),
                warning_timeout
                    .map(String::as_str)
                    .or_else(|| td.status_messages.first().map(String::as_str)),
            )
        } else {
            (None, None)
        };
        persist_tracked_download_state_marker(app, td, state, reason, detail).await
    }

    // ── Title Resolution ─────────────────────────────────────────────────

    async fn resolve_title(app: &AppUseCase, td: &mut TrackedDownload) {
        let can_clear_stale_unmatched_state = should_clear_stale_unmatched_state_on_submission(td);
        let mut existing_submission = app
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&ClientJobLocator::new(
                Some(td.client_id.as_str()),
                &td.client_type,
                &td.client_item.download_client_item_id,
            ))
            .await
            .ok()
            .flatten();
        let should_try_download_id_lookup = existing_submission
            .as_ref()
            .is_none_or(|submission| !title_id_present(Some(submission.title_id.as_str())));
        if should_try_download_id_lookup
            && let Some(download_id_submission) =
                download_id_submission_for_tracked_download_for_download(
                    app,
                    td.canonical_download_id(),
                    td,
                )
                .await
            && title_id_present(Some(download_id_submission.title_id.as_str()))
        {
            existing_submission = Some(download_id_submission);
        }

        // 1. download_submissions lookup (highest confidence).
        if let Some(sub) = existing_submission.as_ref()
            && !sub.title_id.is_empty()
        {
            td.title_id = Some(sub.title_id.clone());
            td.facet = Some(sub.facet.clone());
            td.source_title = sub.source_title.clone();
            td.match_type = TitleMatchType::Submission;
            if can_clear_stale_unmatched_state {
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                if td.state == TrackedDownloadState::ImportBlocked
                    && !td.import_attempted
                    && td.path_missing_since.is_none()
                {
                    td.state = TrackedDownloadState::Downloading;
                }
            }
            return;
        }

        // 2. Embedded client parameters (*scryer_title_id).
        if let Some(title_id) = td.client_item.title_id.as_deref().filter(|s| !s.is_empty()) {
            // Cross-validate: does this title still exist?
            if let Ok(Some(_)) = app.services.catalog.titles.get_by_id(title_id).await {
                td.title_id = Some(title_id.to_string());
                td.match_type = TitleMatchType::ClientParameter;
                return;
            }
        }

        // Persist an observation row before accepting a provisional parse match.
        // It carries durable tracked state without claiming Scryer provenance.
        let category_admission = app.download_client_category_admission_snapshot().await;
        if existing_submission.is_none()
            && crate::services::download_observation_is_admitted(
                false,
                td.client_item.category.as_deref(),
                category_admission.as_deref(),
            )
            && let Err(error) = app
                .services
                .workflow
                .download_submissions
                .record_submission(DownloadSubmission {
                    download_id: scryer_domain::download_identity::DownloadId::new(),
                    title_id: String::new(),
                    purpose: crate::DownloadSubmissionPurpose::Standard,
                    facet: td.facet.clone().unwrap_or_default(),
                    download_client_id: Some(td.client_id.clone())
                        .filter(|value| !value.is_empty()),
                    download_client_type: td.client_type.clone(),
                    download_client_item_id: td.client_item.download_client_item_id.clone(),
                    // Adopted from the client, so there is no announced size.
                    release_size_bytes: None,
                    source_hint: None,
                    source_provider_id: None,
                    source_provider_name: None,
                    source_kind: None,
                    source_title: None,
                    info_hash: None,
                    request_signature: None,
                    scope: SubmissionScope::Orphan,
                })
                .await
        {
            tracing::warn!(error = %error, id = %td.id, "failed to record tracked download stub submission");
        }

        // 3. Parse-based monitored title resolution for downloader observations.
        let release_title = td
            .source_title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(td.client_item.title_name.as_str());
        let parsed = crate::parse_release_metadata(release_title);
        if let Ok(matcher) = app.monitored_title_matcher().await {
            let matched = if parsed.episode.is_some() {
                matcher.resolve_episode(
                    &parsed,
                    td.client_item.facet.as_deref().or(td.facet.as_deref()),
                )
            } else {
                matcher.resolve_movie(&parsed)
            };

            if let Some(resolved) = matched {
                td.title_id = Some(resolved.title.id.clone());
                td.facet = Some(resolved.title.facet.as_str().to_string());
                if td.source_title.is_none() {
                    td.source_title = Some(release_title.to_string());
                }
                td.match_type = resolved.match_type;
            }
        }

        // 4. No trustworthy title match found — completed handler will block
        // auto-import until the user assigns the title manually.
    }

    /// Reconstruct state from persistent storage after restart.
    async fn reconstruct_state(app: &AppUseCase, td: &mut TrackedDownload) {
        let observed_identity = observed_queue_item_identity(&td.client_item);
        let observed_source_identity = queue_item_source_identity(&td.client_item);
        let terminal_source_identity = ClientJobLocator::new(
            Some(td.client_id.as_str()),
            &td.client_type,
            &td.client_item.download_client_item_id,
        );
        // Mirror of the write side in `persist_tracked_download_state_marker`:
        // the durable row is found by canonical download id, so a token-less
        // item (plugin download client) has one too and must be able to read
        // its own terminal state back on the first see after a restart.
        if let Ok(Some(tracked_state)) = app
            .services
            .workflow
            .download_submissions
            .get_identity_tracked_state_for_download(
                td.canonical_download_id(),
                &observed_identity,
                Some(&observed_source_identity),
            )
            .await
            && let Some(state) = TrackedDownloadState::from_str_opt(&tracked_state)
            // `ImportedSeeding` is not terminal but must still survive a
            // restart: re-deriving it would re-import the payload and then
            // remove a torrent that is still working off its seeding goal.
            && (state.is_import_settled() || state == TrackedDownloadState::ImportBlocked)
        {
            td.state = state;
            let terminal_failure_reason = if state == TrackedDownloadState::Failed {
                app.services
                    .workflow
                    .download_submissions
                    .get_identity_tracked_state_reason_for_download(
                        td.canonical_download_id(),
                        &observed_identity,
                        Some(&terminal_source_identity),
                    )
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };
            td.burned_by_import_gate = terminal_failure_reason.as_deref().is_some_and(|reason| {
                matches!(
                    reason,
                    IMPORT_GATE_REJECTED_TRACKED_STATE_REASON
                        | WARNING_TIMEOUT_TRACKED_STATE_REASON
                )
            });
            if state == TrackedDownloadState::Failed && td.burned_by_import_gate {
                let detail = app
                    .services
                    .workflow
                    .download_submissions
                    .get_identity_tracked_state_detail_for_download(
                        td.canonical_download_id(),
                        &observed_identity,
                        Some(&terminal_source_identity),
                    )
                    .await
                    .ok()
                    .flatten();
                td.status = TrackedDownloadStatus::Error;
                td.status_messages = detail.into_iter().collect();
            } else if state == TrackedDownloadState::ImportBlocked {
                let detail = app
                    .services
                    .workflow
                    .download_submissions
                    .get_identity_tracked_state_detail_for_download(
                        td.canonical_download_id(),
                        &observed_identity,
                        Some(&observed_source_identity),
                    )
                    .await
                    .ok()
                    .flatten();
                set_import_blocked_status(td, detail);
            }
            return;
        }

        let download_id_submission = download_id_submission_for_tracked_download_for_download(
            app,
            td.canonical_download_id(),
            td,
        )
        .await;
        // Check tracked state against the matched submission identity first.
        if let Some(submission) = download_id_submission.as_ref() {
            let submission_source_identity = ClientJobLocator::from_submission(submission);
            if let Ok(Some(tracked_state)) = app
                .services
                .workflow
                .download_submissions
                .get_tracked_state(&submission_source_identity)
                .await
                && let Some(state) = TrackedDownloadState::from_str_opt(&tracked_state)
                && (state.is_import_settled() || state == TrackedDownloadState::ImportBlocked)
            {
                td.state = state;
                if state == TrackedDownloadState::ImportBlocked {
                    set_import_blocked_status(td, None);
                }
            }
        }

        // Default: Downloading (will be re-evaluated by check cycle).
    }
}

async fn download_id_submission_for_tracked_download_for_download(
    app: &AppUseCase,
    canonical_download_id: Option<&DownloadId>,
    tracked: &TrackedDownload,
) -> Option<DownloadSubmission> {
    let observed_identity = observed_queue_item_identity(&tracked.client_item);
    if let Some(download_id) = observed_identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let matches = app
            .services
            .workflow
            .download_submissions
            .list_by_download_id_for_download(
                canonical_download_id,
                Some(tracked.client_id.as_str()),
                &tracked.client_type,
                download_id,
            )
            .await
            .ok()?;
        return crate::download_identity::coalesce_download_submissions_by_release_attempt(
            &matches,
        );
    }

    None
}

fn download_submission_identity_is_empty(identity: &DownloadSubmissionIdentity) -> bool {
    identity
        .download_id
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
}

fn should_clear_stale_unmatched_state_on_submission(td: &TrackedDownload) -> bool {
    !title_id_present(td.title_id.as_deref())
        && matches!(
            td.match_type,
            TitleMatchType::Unmatched | TitleMatchType::IdOnly | TitleMatchType::TitleParse
        )
}

pub(crate) async fn publish_runtime_tracked_download_snapshot(
    app: &AppUseCase,
    tracked: &TrackedDownload,
) {
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked.id.clone(),
            TrackedDownloadQueueMetadata::from(tracked),
        );
}

pub(crate) async fn publish_runtime_tracked_download_snapshot_cache(
    app: &AppUseCase,
    tracker: &TrackedDownloadService,
) {
    let snapshot = tracker
        .get_all()
        .into_iter()
        .filter(|tracked| tracked.is_trackable)
        .map(|tracked| {
            (
                tracked.id.clone(),
                TrackedDownloadQueueMetadata::from(tracked),
            )
        })
        .collect::<HashMap<_, _>>();
    *app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await = snapshot;
}

fn title_id_present(value: Option<&str>) -> bool {
    value.is_some_and(|id| !id.trim().is_empty())
}

fn normalize_title_signal(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut last_was_space = false;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            normalized.extend(ch.to_lowercase());
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }
    normalized.trim().to_string()
}

fn normalize_facet_signal(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn should_reresolve_title(
    existing: &TrackedDownload,
    incoming: &DownloadQueueItem,
    matcher_dirty: bool,
) -> bool {
    if matches!(
        existing.match_type,
        TitleMatchType::Submission | TitleMatchType::ClientParameter
    ) {
        return false;
    }

    if matcher_dirty
        && matches!(
            existing.match_type,
            TitleMatchType::Unmatched | TitleMatchType::IdOnly | TitleMatchType::TitleParse
        )
    {
        return true;
    }

    if should_retry_late_submission_resolution(existing, incoming) {
        return true;
    }

    if !title_id_present(existing.client_item.title_id.as_deref())
        && title_id_present(incoming.title_id.as_deref())
    {
        return true;
    }

    if !title_id_present(existing.title_id.as_deref())
        && title_id_present(incoming.title_id.as_deref())
    {
        return true;
    }

    if !existing.client_item.is_scryer_origin && incoming.is_scryer_origin {
        return true;
    }

    if normalize_title_signal(&existing.client_item.title_name)
        != normalize_title_signal(&incoming.title_name)
    {
        return true;
    }

    if normalize_facet_signal(existing.client_item.facet.as_deref())
        != normalize_facet_signal(incoming.facet.as_deref())
    {
        return true;
    }

    false
}

fn should_retry_late_submission_resolution(
    existing: &TrackedDownload,
    incoming: &DownloadQueueItem,
) -> bool {
    !title_id_present(existing.title_id.as_deref())
        && matches!(
            existing.match_type,
            TitleMatchType::Unmatched | TitleMatchType::IdOnly | TitleMatchType::TitleParse
        )
        && !download_submission_identity_is_empty(&observed_queue_item_identity(incoming))
}

/// Absence debounce for pruning: stamps the first tick an item goes missing
/// and only reports true once the absence has outlived the grace window. A
/// sighting (`seen_ids` hit) clears the stamp at the call sites.
fn snapshot_absence_exceeds_grace(td: &mut TrackedDownload, now: DateTime<Utc>) -> bool {
    match td.snapshot_missing_since {
        None => {
            td.snapshot_missing_since = Some(now);
            false
        }
        Some(since) => {
            (now - since).num_seconds() >= TrackedDownloadService::SNAPSHOT_ABSENCE_PRUNE_GRACE_SECS
        }
    }
}

fn import_blocked_fallback_message(td: &TrackedDownload) -> String {
    if !title_id_present(td.title_id.as_deref()) {
        return "Automatic import could not identify a library title. Assign a title to continue."
            .to_string();
    }

    match td.facet.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("series" | "anime") => "Automatic import could not determine a unique season and episode mapping. Open Manual Import and assign the correct season and episode."
            .to_string(),
        _ => "Automatic import needs operator review. Open Manual Import and confirm the file mapping to continue."
            .to_string(),
    }
}

fn set_import_blocked_status(td: &mut TrackedDownload, detail: Option<String>) {
    let message = detail
        .and_then(|detail| {
            let detail = detail.trim();
            (!detail.is_empty()).then(|| detail.to_string())
        })
        .unwrap_or_else(|| import_blocked_fallback_message(td));
    td.status = TrackedDownloadStatus::Warning;
    td.status_messages = vec![message];
}

pub(crate) async fn assign_title_to_tracked_download(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    title: &Title,
) {
    let was_blocked = td.state == TrackedDownloadState::ImportBlocked;
    td.title_id = Some(title.id.clone());
    td.facet = Some(title.facet.as_str().to_string());
    td.match_type = TitleMatchType::Submission;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
    td.import_attempted = false;

    // Assignment identifies the title but does not authorize import. Movies use
    // the same explicit manual-import decision point as episodic downloads.
    if was_blocked {
        set_import_blocked_status(td, None);
        persist_import_blocked_state_marker(
            app,
            td,
            ImportBlockedReason::ManualMappingRequired,
            td.status_messages.first().map(String::as_str),
        )
        .await;
        return;
    }

    td.state = TrackedDownloadState::Downloading;
    crate::failed_download_handler::check(td);
    crate::completed_download_handler::check(app, td).await;
}

// ── Command Channel ──────────────────────────────────────────────────────────

/// Result of a manual-import recovery attempt against a tracked download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualImportRecoveryOutcome {
    /// The tracked download was transitioned to `Imported`.
    Marked,
    /// The source is tracked but must be left alone: still downloading,
    /// already terminal (including already `Imported`), or not yet reported
    /// complete by the client. This is a final decision for the record; the
    /// caller must not treat it as success and need not retry.
    Unchanged,
    /// The source is not in the tracked-download cache (not observed since
    /// boot, or evicted). No decision was made; retry on a later tick.
    Untracked,
    /// The tracked download is mid background work; retry on a later tick.
    Busy,
}

/// What a completed manual-import record is allowed to do to the tracked
/// download that shares its (client, item id) identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualImportRecoveryVerdict {
    /// Client-complete and waiting on import: the record may terminalize it.
    MarkImported,
    /// Leave the tracked download alone.
    Leave,
}

/// Decide whether a completed manual-import record (completed at
/// `record_completed_at`) may mark `tracked` as `Imported`.
///
/// The record is keyed only by (client id, client type, item id), and item ids
/// are reused: qBittorrent ids are info-hashes (a re-grab or cross-seed of the
/// same release shares one), NZBGet ids restart after a queue-database reset.
/// A record from a previous life of that identity must therefore never touch a
/// download that is still `Downloading` or awaiting failure handling — that
/// download has not been imported and would silently never be. Only a download
/// the client has finished (`completed_source` retained) and that is waiting
/// on import (`ImportPending`, `Importing`, `ImportBlocked`) is eligible; a
/// terminal download (including one already `Imported`) is left as is.
///
/// The record can also only vouch for the download it was produced from: one
/// the client finished *before* the record completed. A same-id re-grab that
/// finished after the record (delete + re-grab inside the recovery window,
/// now sitting in `ImportBlocked`) is a different download and is left alone.
/// When the client reported no completion time the check cannot run and the
/// state rules alone decide.
pub fn manual_import_recovery_verdict(
    tracked: &TrackedDownload,
    record_completed_at: DateTime<Utc>,
) -> ManualImportRecoveryVerdict {
    let awaiting_import = matches!(
        tracked.state,
        TrackedDownloadState::ImportPending
            | TrackedDownloadState::Importing
            | TrackedDownloadState::ImportBlocked
    );
    let Some(completed_source) = tracked.completed_source.as_ref() else {
        return ManualImportRecoveryVerdict::Leave;
    };
    if !awaiting_import {
        return ManualImportRecoveryVerdict::Leave;
    }
    if completed_source
        .completed_at
        .is_some_and(|client_completed_at| client_completed_at > record_completed_at)
    {
        return ManualImportRecoveryVerdict::Leave;
    }
    ManualImportRecoveryVerdict::MarkImported
}

/// Commands sent from GraphQL mutations to the poller's TrackedDownloadService.
pub enum TrackedDownloadCommand {
    ReconcileManualImport {
        id: String,
        canonical_download_id: Option<DownloadId>,
        files_imported_this_pass: usize,
        expected_mapping_count: Option<usize>,
        reply: oneshot::Sender<AppResult<bool>>,
    },
    MarkImported {
        id: String,
        canonical_download_id: Option<DownloadId>,
        reply: oneshot::Sender<AppResult<()>>,
    },
    /// Recovery for a manual-import record that reached `Completed` (at
    /// `record_completed_at`) without terminalizing its tracked download
    /// (crash, dropped reply, restart). Only a download the client finished
    /// before that time and that is waiting on import may be marked; see
    /// [`manual_import_recovery_verdict`].
    MarkImportedIfAwaitingImport {
        source_identity: ClientJobLocator,
        canonical_download_id: Option<DownloadId>,
        record_completed_at: DateTime<Utc>,
        reply: oneshot::Sender<AppResult<ManualImportRecoveryOutcome>>,
    },
    Ignore {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    Forget {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    MarkFailed {
        id: String,
        skip_reacquire: bool,
        reply: oneshot::Sender<AppResult<()>>,
    },
    RetryImport {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    AssignTitle {
        id: String,
        title: Box<Title>,
        submission: Box<DownloadSubmission>,
        actor_snapshot: DownloadSubmissionActorSnapshot,
        reply: oneshot::Sender<AppResult<()>>,
    },
    CompletedSource {
        identity: ClientJobLocator,
        reply: oneshot::Sender<Option<CompletedDownload>>,
    },
    Snapshot {
        ids: Vec<String>,
        reply: oneshot::Sender<HashMap<String, TrackedDownloadQueueMetadata>>,
    },
}

#[derive(Clone, Debug)]
pub enum TrackedDownloadSnapshotScope {
    AuthoritativeForClient {
        client_id: Option<String>,
        client_type: String,
    },
    Delta,
}

#[derive(Clone, Debug)]
pub struct TrackedDownloadSnapshotUpdate {
    pub scope: TrackedDownloadSnapshotScope,
    pub items: Vec<DownloadQueueItem>,
    pub completed_downloads: Vec<CompletedDownload>,
    pub actor_id: Option<String>,
}

/// Handle for feeding download-client observations into the tracked runtime.
#[derive(Clone)]
pub struct TrackedDownloadSnapshotIngestHandle {
    tx: mpsc::Sender<TrackedDownloadSnapshotUpdate>,
}

impl TrackedDownloadSnapshotIngestHandle {
    pub fn new(tx: mpsc::Sender<TrackedDownloadSnapshotUpdate>) -> Self {
        Self { tx }
    }

    pub async fn publish(&self, update: TrackedDownloadSnapshotUpdate) -> AppResult<()> {
        self.tx.send(update).await.map_err(|_| {
            crate::AppError::Repository("tracked download snapshot ingest unavailable".into())
        })
    }
}

/// Client types currently covered by a realtime bridge, shared between the
/// bridge supervisor (writer) and the download queue poller (reader).
///
/// Bridge eligibility used to be resolved once at process startup, so a weaver
/// client added or promoted after boot never got its subscription bridge and
/// silently fell back to interval polling until the next restart. The
/// supervisor now flips coverage at runtime, and the poller consults this
/// handle every tick instead of a list fixed at construction.
///
/// The poller treats these types exactly like its static
/// `excluded_client_types`: it neither polls nor prunes them (the bridge is
/// authoritative), but still runs the periodic excluded-client history
/// reconciliation as a loss-tolerance backstop.
#[derive(Clone, Debug, Default)]
pub struct BridgedClientTypesHandle {
    types: std::sync::Arc<std::sync::RwLock<Vec<String>>>,
}

impl BridgedClientTypesHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the covered set. The writer is the bridge supervisor.
    pub fn set(&self, types: Vec<String>) {
        *self
            .types
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = types;
    }

    /// Drop all coverage — generic polling resumes for every client type.
    pub fn clear(&self) {
        self.set(Vec::new());
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.types
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Handle for sending commands to the tracked downloads poller.
#[derive(Clone)]
pub struct TrackedDownloadHandle {
    tx: mpsc::Sender<TrackedDownloadCommand>,
}

impl TrackedDownloadHandle {
    pub fn new(tx: mpsc::Sender<TrackedDownloadCommand>) -> Self {
        Self { tx }
    }

    pub async fn completed_source(
        &self,
        identity: ClientJobLocator,
    ) -> AppResult<Option<CompletedDownload>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::CompletedSource {
                identity,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })
    }

    pub async fn ignore(&self, id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::Ignore {
                id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn forget(&self, id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::Forget {
                id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn mark_imported(&self, id: String) -> AppResult<()> {
        self.mark_imported_for_download(id, None).await
    }

    pub async fn mark_imported_for_download(
        &self,
        id: String,
        canonical_download_id: Option<DownloadId>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::MarkImported {
                id,
                canonical_download_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn mark_imported_if_awaiting_import(
        &self,
        source_identity: ClientJobLocator,
        record_completed_at: DateTime<Utc>,
    ) -> AppResult<ManualImportRecoveryOutcome> {
        self.mark_imported_if_awaiting_import_for_download(
            source_identity,
            None,
            record_completed_at,
        )
        .await
    }

    pub async fn mark_imported_if_awaiting_import_for_download(
        &self,
        source_identity: ClientJobLocator,
        canonical_download_id: Option<DownloadId>,
        record_completed_at: DateTime<Utc>,
    ) -> AppResult<ManualImportRecoveryOutcome> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::MarkImportedIfAwaitingImport {
                source_identity,
                canonical_download_id,
                record_completed_at,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn reconcile_manual_import(
        &self,
        id: String,
        files_imported_this_pass: usize,
        expected_mapping_count: Option<usize>,
    ) -> AppResult<bool> {
        self.reconcile_manual_import_for_download(
            id,
            None,
            files_imported_this_pass,
            expected_mapping_count,
        )
        .await
    }

    pub async fn reconcile_manual_import_for_download(
        &self,
        id: String,
        canonical_download_id: Option<DownloadId>,
        files_imported_this_pass: usize,
        expected_mapping_count: Option<usize>,
    ) -> AppResult<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::ReconcileManualImport {
                id,
                canonical_download_id,
                files_imported_this_pass,
                expected_mapping_count,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn mark_failed(&self, id: String, skip_reacquire: bool) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::MarkFailed {
                id,
                skip_reacquire,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn retry_import(&self, id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::RetryImport {
                id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn assign_title(
        &self,
        id: String,
        title: Title,
        submission: DownloadSubmission,
        actor_snapshot: DownloadSubmissionActorSnapshot,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::AssignTitle {
                id,
                title: Box::new(title),
                submission: Box::new(submission),
                actor_snapshot,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn snapshot(
        &self,
        ids: Vec<String>,
    ) -> AppResult<HashMap<String, TrackedDownloadQueueMetadata>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::Snapshot {
                ids,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Write a durable tracked-state marker for a download to
/// download_submissions and download_identity_states.
///
/// Terminal states record the outcome. The non-terminal `import_blocked`
/// marker records that an operator decision is pending, so restarts don't
/// erase the fact and reconciliation sweeps don't re-offer the item; a later
/// successful import overwrites it with the terminal state.
pub(crate) async fn persist_import_blocked_state_marker(
    app: &AppUseCase,
    td: &TrackedDownload,
    reason: ImportBlockedReason,
    detail: Option<&str>,
) -> bool {
    persist_tracked_download_state_marker(
        app,
        td,
        TrackedDownloadState::ImportBlocked,
        Some(reason.as_str()),
        detail,
    )
    .await
}

pub(crate) async fn persist_tracked_download_state_marker(
    app: &AppUseCase,
    td: &TrackedDownload,
    state: TrackedDownloadState,
    reason: Option<&str>,
    detail: Option<&str>,
) -> bool {
    let state_identity = match download_id_submission_for_tracked_download_for_download(
        app,
        td.canonical_download_id(),
        td,
    )
    .await
    {
        Some(submission) => ClientJobLocator::from_submission(&submission),
        None => ClientJobLocator::new(
            Some(td.client_id.as_str()),
            &td.client_type,
            &td.client_item.download_client_item_id,
        ),
    };
    // The durable row is keyed by the canonical download, not by the legacy
    // wire token, so an item that carries no token still has somewhere to
    // record its outcome. Gating this on a non-empty observed identity meant
    // plugin download clients — which legally omit the token — never persisted
    // a terminal or blocked marker and re-entered processing after a restart.
    // The store itself is the one that decides there is nothing to key by: it
    // returns early when neither a canonical id nor an active binding resolves.
    let observed_identity = observed_queue_item_identity(&td.client_item);
    if let Err(e) = app
        .services
        .workflow
        .download_submissions
        .record_identity_tracked_state_for_download(
            td.canonical_download_id(),
            &observed_identity,
            Some(&ClientJobLocator::new(
                Some(td.client_id.as_str()),
                &td.client_type,
                &td.client_item.download_client_item_id,
            )),
            state.as_str(),
            reason,
            detail,
        )
        .await
    {
        tracing::warn!(
            error = %e,
            id = %td.id,
            client_id = td.client_id.as_str(),
            client_type = td.client_type.as_str(),
            download_client_item_id = td.client_item.download_client_item_id.as_str(),
            state = state.as_str(),
            "failed to persist durable tracked download state"
        );
        return false;
    }

    // Write the canonical state and its typed reason before updating the
    // legacy submission projection. If the second write fails, restart
    // reconstruction still sees the safe canonical reason instead of a stale
    // migrated `unverified_already_imported` marker.
    if let Err(e) = app
        .services
        .workflow
        .download_submissions
        .update_tracked_state(&state_identity, state.as_str())
        .await
    {
        tracing::warn!(
            error = %e,
            id = %td.id,
            tracked_state_client_item_id = state_identity.item_id.as_str(),
            state = state.as_str(),
            "failed to persist tracked download submission state"
        );
        return false;
    }

    true
}

pub(crate) async fn import_blocked_reason_for_tracked(
    app: &AppUseCase,
    td: &TrackedDownload,
) -> Option<ImportBlockedReason> {
    let observed_identity = observed_queue_item_identity(&td.client_item);
    let source_identity = queue_item_source_identity(&td.client_item);
    app.services
        .workflow
        .download_submissions
        .get_identity_tracked_state_reason_for_download(
            td.canonical_download_id(),
            &observed_identity,
            Some(&source_identity),
        )
        .await
        .ok()
        .flatten()
        .as_deref()
        .and_then(ImportBlockedReason::parse)
}

pub(crate) fn tracked_client_type_is_excluded(
    client_type: &str,
    excluded_client_types: &[&str],
) -> bool {
    excluded_client_types
        .iter()
        .any(|excluded| excluded.trim().eq_ignore_ascii_case(client_type.trim()))
}

fn tracked_matches_snapshot_scope(
    tracked: &TrackedDownload,
    client_id: Option<&str>,
    client_type: &str,
) -> bool {
    if !tracked
        .client_type
        .trim()
        .eq_ignore_ascii_case(client_type.trim())
    {
        return false;
    }

    let Some(client_id) = client_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };

    tracked.client_id.trim() == client_id
}

fn tracked_download_can_be_evicted_for_cache_pressure(tracked: &TrackedDownload) -> bool {
    if !tracked.is_trackable {
        return true;
    }

    tracked.state == TrackedDownloadState::Downloading
        && tracked.status == TrackedDownloadStatus::Ok
        && tracked.status_messages.is_empty()
        && tracked.title_id.is_none()
        && tracked.facet.is_none()
        && tracked.source_title.is_none()
        && tracked.match_type == TitleMatchType::Unmatched
        && !tracked.import_attempted
        && tracked.path_missing_since.is_none()
        && !tracked.notified_manual_interaction
        && !tracked.skip_reacquire_on_failure
}

fn tracked_download_cache_ttl() -> chrono::Duration {
    std::env::var("SCRYER_TRACKED_DOWNLOAD_CACHE_TTL_HOURS")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|hours| *hours > 0)
        .map(chrono::Duration::hours)
        .unwrap_or_else(|| chrono::Duration::hours(DEFAULT_TRACKED_DOWNLOAD_CACHE_TTL_HOURS))
}

fn tracked_download_cache_max_entries() -> usize {
    std::env::var("SCRYER_TRACKED_DOWNLOAD_CACHE_MAX_ENTRIES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|entries| *entries > 0)
        .unwrap_or(DEFAULT_TRACKED_DOWNLOAD_CACHE_MAX_ENTRIES)
}

pub(crate) fn observed_queue_item_identity(item: &DownloadQueueItem) -> DownloadSubmissionIdentity {
    crate::observed_download_identity(crate::ObservedDownloadIdentityInput {
        download_id: item.download_id.as_deref(),
        parameters: &[],
        info_hash_hint: None,
    })
}

fn queue_item_source_identity(item: &DownloadQueueItem) -> ClientJobLocator {
    ClientJobLocator::new(
        Some(item.client_id.as_str()),
        item.client_type.as_str(),
        item.download_client_item_id.as_str(),
    )
}

pub(crate) fn tracked_download_id_for_item(item: &DownloadQueueItem) -> String {
    let observed_identity = observed_queue_item_identity(item);
    if let Some(download_id) = observed_identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!(
            "download:{}:{}:{download_id}",
            item.client_id, item.client_type
        );
    }
    tracked_download_id(
        Some(item.client_id.as_str()),
        &item.client_type,
        &item.download_client_item_id,
    )
}

pub fn tracked_download_id(client_id: Option<&str>, client_type: &str, item_id: &str) -> String {
    let normalized_client_id = client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    if normalized_client_id.is_empty() {
        return format!("{client_type}:{item_id}");
    }

    format!("{normalized_client_id}:{item_id}")
}

fn tracked_download_matches_source_id(tracked: &TrackedDownload, requested_id: &str) -> bool {
    let requested_id = requested_id.trim();
    if requested_id.is_empty() {
        return false;
    }

    let item = &tracked.client_item;
    [
        tracked_download_id(
            Some(tracked.client_id.as_str()),
            &tracked.client_type,
            &item.download_client_item_id,
        ),
        tracked_download_id(
            Some(item.client_id.as_str()),
            &item.client_type,
            &item.download_client_item_id,
        ),
        tracked_download_id(None, &tracked.client_type, &item.download_client_item_id),
        tracked_download_id(None, &item.client_type, &item.download_client_item_id),
    ]
    .into_iter()
    .any(|id| id == requested_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use crate::{
        AppError, AppResult, AppServices, AppUseCase, ClientJobLocator, CreateTitleOutcome,
        DomainEventRepository, DownloadClient, DownloadClientAddRequest, DownloadGrabResult,
        DownloadRegistryRepository, DownloadSubmissionRepository, FacetRegistry, ImportRepository,
        IndexerConfigRepository, JwtAuthConfig, PendingTitleHydration, TitleMetadataUpdate,
        TitleRepository,
    };
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use scryer_domain::{
        CompletedDownload, DomainEvent, DomainEventFilter, DownloadQueueState, Id, ImportRecord,
        ImportStatus, ImportType, MediaFacet, NewDomainEvent, Title, TitleHistoryEventType,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestDownloadSubmissionRepo {
        submission: Option<crate::DownloadSubmission>,
        canonical_submission: Option<crate::DownloadSubmission>,
        submission_identity: Option<crate::DownloadSubmissionIdentity>,
        mutable_submission: Option<Arc<Mutex<Option<crate::DownloadSubmission>>>>,
        mutable_submission_identity: Option<Arc<Mutex<Option<crate::DownloadSubmissionIdentity>>>>,
        tracked_state: Option<String>,
        tracked_state_updates: Arc<Mutex<Vec<String>>>,
        recorded_submissions: Arc<Mutex<Vec<crate::DownloadSubmission>>>,
        download_id_submissions:
            Arc<Mutex<Vec<(crate::DownloadSubmission, crate::DownloadSubmissionIdentity)>>>,
        identity_tracked_states: Arc<Mutex<HashMap<String, String>>>,
        identity_tracked_state_reasons: Arc<Mutex<HashMap<String, String>>>,
        identity_tracked_state_details: Arc<Mutex<HashMap<String, String>>>,
        canonical_identity_tracked_states: Arc<Mutex<HashMap<DownloadId, String>>>,
        canonical_identity_tracked_state_reasons: Arc<Mutex<HashMap<DownloadId, String>>>,
        canonical_identity_tracked_state_details: Arc<Mutex<HashMap<DownloadId, String>>>,
    }

    struct TestDownloadRegistry {
        ids: Mutex<HashMap<String, DownloadId>>,
        failing_item_ids: HashSet<String>,
        conflicting_item_ids: HashMap<String, (DownloadId, DownloadId)>,
        fallback_download_ids: HashMap<String, DownloadId>,
    }

    #[async_trait]
    impl DownloadRegistryRepository for TestDownloadRegistry {
        async fn resolve_observation(
            &self,
            observation: &crate::ObservedClientJob,
        ) -> AppResult<crate::ObservationResolution> {
            if let Some(&(token_id, binding_download_id)) =
                self.conflicting_item_ids.get(&observation.locator.item_id)
            {
                return Ok(crate::ObservationResolution::Conflict {
                    token_id,
                    binding_download_id,
                });
            }
            if self.failing_item_ids.contains(&observation.locator.item_id) {
                return Err(AppError::Repository(
                    "injected registry resolution failure".to_string(),
                ));
            }
            let mut ids = self.ids.lock().await;
            let download_id = *ids
                .entry(observation.locator.item_id.clone())
                .or_insert_with(DownloadId::new);
            Ok(crate::ObservationResolution::Resolved {
                download_id,
                newly_foreign: false,
                attached: false,
            })
        }

        async fn load_download(&self, _: &DownloadId) -> AppResult<Option<crate::DownloadRecord>> {
            Ok(None)
        }

        async fn load_binding(
            &self,
            _: &DownloadId,
        ) -> AppResult<Option<crate::DownloadClientBindingRecord>> {
            Ok(None)
        }

        async fn find_active_binding_by_locator(
            &self,
            locator: &crate::ClientJobLocator,
        ) -> AppResult<Option<crate::DownloadClientBindingRecord>> {
            if let Some(&download_id) = self.fallback_download_ids.get(&locator.item_id) {
                return Ok(Some(crate::DownloadClientBindingRecord {
                    download_id,
                    client_config_id: locator.client_id.clone(),
                    client_type_snapshot: Some(locator.client_type.clone()),
                    client_name_snapshot: None,
                    native_item_id: Some(locator.item_id.clone()),
                    created_at: Utc::now(),
                    last_seen_at: None,
                    ended_at: None,
                }));
            }
            if self.failing_item_ids.contains(&locator.item_id) {
                return Err(AppError::Repository(
                    "injected registry binding failure".to_string(),
                ));
            }
            Ok(None)
        }

        async fn end_binding(&self, _: &DownloadId) -> AppResult<()> {
            Ok(())
        }
    }

    fn test_tracked_state_key(
        identity: &crate::DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> Option<String> {
        let download_id = identity
            .download_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        if download_id.starts_with("scryer-download:")
            || (matches!(download_id.len(), 40 | 64)
                && download_id.chars().all(|ch| ch.is_ascii_hexdigit()))
        {
            return Some(format!("download:{download_id}"));
        }

        let source_identity = source_identity?;
        let client_type = source_identity.client_type.trim();
        if client_type.is_empty() {
            return None;
        }

        Some(format!(
            "client:{}:{}:download:{}",
            source_identity
                .client_id
                .as_deref()
                .map(str::trim)
                .unwrap_or_default(),
            client_type.to_ascii_lowercase(),
            download_id
        ))
    }

    impl TestDownloadSubmissionRepo {
        async fn current_submission(&self) -> Option<crate::DownloadSubmission> {
            if let Some(submission) = self.mutable_submission.as_ref() {
                return submission.lock().await.clone();
            }

            self.submission.clone()
        }

        async fn current_submission_identity(&self) -> Option<crate::DownloadSubmissionIdentity> {
            if let Some(identity) = self.mutable_submission_identity.as_ref() {
                return identity.lock().await.clone();
            }

            self.submission_identity.clone()
        }
    }

    #[async_trait]
    impl DownloadSubmissionRepository for TestDownloadSubmissionRepo {
        async fn record_submission(&self, submission: crate::DownloadSubmission) -> AppResult<()> {
            self.recorded_submissions.lock().await.push(submission);
            Ok(())
        }

        async fn record_ambiguous_submission(
            &self,
            submission: crate::DownloadSubmission,
        ) -> AppResult<()> {
            self.record_submission(submission).await
        }

        async fn record_submission_with_identity(
            &self,
            submission: crate::DownloadSubmission,
            _: crate::DownloadSubmissionIdentity,
            _: Option<crate::PersistedSeedGoals>,
        ) -> AppResult<crate::CanonicalDownloadIdentityDisposition> {
            self.record_submission(submission).await?;
            Ok(crate::CanonicalDownloadIdentityDisposition::Requested)
        }

        async fn find_by_client_item_id(
            &self,
            identity: &ClientJobLocator,
        ) -> AppResult<Option<crate::DownloadSubmission>> {
            Ok(self
                .current_submission()
                .await
                .filter(|submission| ClientJobLocator::from_submission(submission) == *identity))
        }

        async fn find_by_client_item_id_for_download(
            &self,
            canonical_download_id: Option<&DownloadId>,
            identity: &ClientJobLocator,
        ) -> AppResult<Option<crate::DownloadSubmission>> {
            let canonical = canonical_download_id.and_then(|canonical_download_id| {
                self.canonical_submission
                    .as_ref()
                    .filter(|submission| submission.download_id == *canonical_download_id)
                    .cloned()
            });
            let legacy = self.find_by_client_item_id(identity).await?;
            Ok(legacy.or(canonical))
        }

        async fn list_by_download_id(
            &self,
            client_id: Option<&str>,
            client_type: &str,
            download_id: &str,
        ) -> AppResult<Vec<crate::DownloadSubmission>> {
            let explicit_matches = self
                .download_id_submissions
                .lock()
                .await
                .iter()
                .filter(|(submission, identity)| {
                    submission.download_client_id.as_deref().unwrap_or("")
                        == client_id.unwrap_or("")
                        && submission
                            .download_client_type
                            .eq_ignore_ascii_case(client_type)
                        && identity.download_id.as_deref() == Some(download_id)
                })
                .map(|(submission, _)| submission.clone())
                .collect::<Vec<_>>();
            if !explicit_matches.is_empty() {
                return Ok(explicit_matches);
            }

            let Some(submission) = self.current_submission().await else {
                return Ok(vec![]);
            };
            let matches_submission = submission.download_client_id.as_deref().unwrap_or("")
                == client_id.unwrap_or("")
                && submission
                    .download_client_type
                    .eq_ignore_ascii_case(client_type);
            let matches_identity = self
                .current_submission_identity()
                .await
                .as_ref()
                .and_then(|identity| identity.download_id.as_deref())
                == Some(download_id);
            Ok((matches_submission && matches_identity)
                .then_some(submission.clone())
                .into_iter()
                .collect())
        }

        async fn list_by_download_id_for_download(
            &self,
            canonical_download_id: Option<&DownloadId>,
            client_id: Option<&str>,
            client_type: &str,
            download_id: &str,
        ) -> AppResult<Vec<crate::DownloadSubmission>> {
            let canonical = canonical_download_id
                .and_then(|canonical_download_id| {
                    self.canonical_submission
                        .as_ref()
                        .filter(|submission| submission.download_id == *canonical_download_id)
                        .cloned()
                })
                .into_iter()
                .collect::<Vec<_>>();
            let legacy = self
                .list_by_download_id(client_id, client_type, download_id)
                .await?;
            Ok(if legacy.is_empty() { canonical } else { legacy })
        }

        async fn get_submission_identity(
            &self,
            _: &ClientJobLocator,
        ) -> AppResult<Option<crate::DownloadSubmissionIdentity>> {
            Ok(self.current_submission_identity().await)
        }

        async fn list_for_client_items(
            &self,
            _: &[ClientJobLocator],
        ) -> AppResult<Vec<crate::DownloadSubmission>> {
            Ok(self.current_submission().await.into_iter().collect())
        }

        async fn list_for_title(&self, _: &str) -> AppResult<Vec<crate::DownloadSubmission>> {
            Ok(vec![])
        }

        async fn find_by_title_and_request_signature(
            &self,
            _: &str,
            _: &str,
            _: crate::DownloadSubmissionPurpose,
            _: &crate::SubmissionScope,
        ) -> AppResult<Option<crate::DownloadSubmission>> {
            Ok(None)
        }

        async fn delete_for_title(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn delete_by_client_item_id(&self, _: &ClientJobLocator) -> AppResult<()> {
            Ok(())
        }

        async fn update_tracked_state(
            &self,
            _: &ClientJobLocator,
            tracked_state: &str,
        ) -> AppResult<()> {
            self.tracked_state_updates
                .lock()
                .await
                .push(tracked_state.to_string());
            Ok(())
        }

        async fn get_tracked_state(&self, _: &ClientJobLocator) -> AppResult<Option<String>> {
            Ok(self.tracked_state.clone())
        }

        async fn record_identity_tracked_state(
            &self,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&ClientJobLocator>,
            tracked_state: &str,
            reason: Option<&str>,
            detail: Option<&str>,
        ) -> AppResult<()> {
            if let Some(key) = test_tracked_state_key(identity, source_identity) {
                self.identity_tracked_states
                    .lock()
                    .await
                    .insert(key.clone(), tracked_state.to_string());
                if let Some(reason) = reason {
                    self.identity_tracked_state_reasons
                        .lock()
                        .await
                        .insert(key.clone(), reason.to_string());
                }
                if let Some(detail) = detail {
                    self.identity_tracked_state_details
                        .lock()
                        .await
                        .insert(key, detail.to_string());
                }
            }
            Ok(())
        }

        /// The real store keys the durable row by the canonical download id and
        /// keeps the legacy token only as a column, so a token-less item still
        /// gets a row. Mirror both halves here: the legacy map for the
        /// token-bearing assertions, and the canonical map the restart path
        /// actually reads back.
        async fn record_identity_tracked_state_for_download(
            &self,
            canonical_download_id: Option<&DownloadId>,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&ClientJobLocator>,
            tracked_state: &str,
            reason: Option<&str>,
            detail: Option<&str>,
        ) -> AppResult<()> {
            self.record_identity_tracked_state(
                identity,
                source_identity,
                tracked_state,
                reason,
                detail,
            )
            .await?;
            if let Some(canonical_download_id) = canonical_download_id {
                self.canonical_identity_tracked_states
                    .lock()
                    .await
                    .insert(*canonical_download_id, tracked_state.to_string());
                if let Some(reason) = reason {
                    self.canonical_identity_tracked_state_reasons
                        .lock()
                        .await
                        .insert(*canonical_download_id, reason.to_string());
                }
                if let Some(detail) = detail {
                    self.canonical_identity_tracked_state_details
                        .lock()
                        .await
                        .insert(*canonical_download_id, detail.to_string());
                }
            }
            Ok(())
        }

        /// The trait default is canonical-blind — it reads and writes through
        /// the legacy identity key only, which a token-less item does not have.
        /// The real store resolves the canonical id, preserves a terminal
        /// previous state, and writes the canonical row; mirror that here.
        async fn upsert_identity_tracked_state_for_download_returning_previous(
            &self,
            target: crate::IdentityTrackedStateTarget<'_>,
            tracked_state: &str,
            preserve_previous: &[&str],
            reason: Option<&str>,
            detail: Option<&str>,
        ) -> AppResult<Option<String>> {
            let previous = self
                .get_identity_tracked_state_for_download(
                    target.canonical_download_id,
                    target.identity,
                    target.source_identity,
                )
                .await?;
            if let Some(previous) = previous
                .as_deref()
                .filter(|previous| preserve_previous.contains(previous))
            {
                return Ok(Some(previous.to_string()));
            }
            self.record_identity_tracked_state_for_download(
                target.canonical_download_id,
                target.identity,
                target.source_identity,
                tracked_state,
                reason,
                detail,
            )
            .await?;
            Ok(previous)
        }

        async fn get_identity_tracked_state(
            &self,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&ClientJobLocator>,
        ) -> AppResult<Option<String>> {
            let Some(key) = test_tracked_state_key(identity, source_identity) else {
                return Ok(None);
            };
            Ok(self.identity_tracked_states.lock().await.get(&key).cloned())
        }

        async fn get_identity_tracked_state_for_download(
            &self,
            canonical_download_id: Option<&DownloadId>,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&ClientJobLocator>,
        ) -> AppResult<Option<String>> {
            if let Some(canonical_download_id) = canonical_download_id {
                return Ok(self
                    .canonical_identity_tracked_states
                    .lock()
                    .await
                    .get(canonical_download_id)
                    .cloned());
            }
            self.get_identity_tracked_state(identity, source_identity)
                .await
        }

        async fn get_identity_tracked_state_reason(
            &self,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&ClientJobLocator>,
        ) -> AppResult<Option<String>> {
            let Some(key) = test_tracked_state_key(identity, source_identity) else {
                return Ok(None);
            };
            Ok(self
                .identity_tracked_state_reasons
                .lock()
                .await
                .get(&key)
                .cloned())
        }

        async fn get_identity_tracked_state_reason_for_download(
            &self,
            canonical_download_id: Option<&DownloadId>,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&ClientJobLocator>,
        ) -> AppResult<Option<String>> {
            if let Some(canonical_download_id) = canonical_download_id {
                return Ok(self
                    .canonical_identity_tracked_state_reasons
                    .lock()
                    .await
                    .get(canonical_download_id)
                    .cloned());
            }
            self.get_identity_tracked_state_reason(identity, source_identity)
                .await
        }

        async fn get_identity_tracked_state_detail(
            &self,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&ClientJobLocator>,
        ) -> AppResult<Option<String>> {
            let Some(key) = test_tracked_state_key(identity, source_identity) else {
                return Ok(None);
            };
            Ok(self
                .identity_tracked_state_details
                .lock()
                .await
                .get(&key)
                .cloned())
        }

        async fn get_identity_tracked_state_detail_for_download(
            &self,
            canonical_download_id: Option<&DownloadId>,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&ClientJobLocator>,
        ) -> AppResult<Option<String>> {
            if let Some(canonical_download_id) = canonical_download_id {
                return Ok(self
                    .canonical_identity_tracked_state_details
                    .lock()
                    .await
                    .get(canonical_download_id)
                    .cloned());
            }
            self.get_identity_tracked_state_detail(identity, source_identity)
                .await
        }
    }

    struct TestImportStatusUpdate(ImportStatus, Option<String>);

    #[derive(Default)]
    struct TestImportRepo {
        import_record: Option<ImportRecord>,
        import_records: Vec<ImportRecord>,
        queue_error: Option<String>,
        status_updates: Arc<Mutex<Vec<TestImportStatusUpdate>>>,
    }

    impl TestImportRepo {
        fn stored_imports(&self) -> Vec<ImportRecord> {
            if !self.import_records.is_empty() {
                return self.import_records.clone();
            }

            self.import_record.clone().into_iter().collect()
        }
    }

    #[derive(Default)]
    struct TestDownloadClient {
        queue_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
        recent_activity: Arc<Mutex<Vec<DownloadQueueItem>>>,
        completed_downloads: Arc<Mutex<Vec<CompletedDownload>>>,
    }

    #[async_trait]
    impl DownloadClient for TestDownloadClient {
        async fn submit_download(
            &self,
            _: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
            Ok(self.completed_downloads.lock().await.clone())
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.queue_items.lock().await.clone())
        }

        async fn list_recent_activity(&self, _: usize) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.recent_activity.lock().await.clone())
        }
    }

    #[derive(Default)]
    struct TestDomainEventRepo {
        events: Arc<Mutex<Vec<DomainEvent>>>,
        subscriber_offsets: Arc<Mutex<HashMap<String, i64>>>,
    }

    #[derive(Default)]
    struct TestTitleRepo {
        titles: Vec<Title>,
    }

    struct MutableTitleRepo {
        titles: Arc<Mutex<Vec<Title>>>,
        list_for_matching_calls: Arc<Mutex<usize>>,
    }

    #[derive(Default)]
    struct TestIndexerConfigRepo;

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepo {
        async fn list(&self, _: Option<String>) -> AppResult<Vec<scryer_domain::IndexerConfig>> {
            Ok(vec![])
        }

        async fn get_by_id(&self, _: &str) -> AppResult<Option<scryer_domain::IndexerConfig>> {
            Ok(None)
        }

        async fn create(
            &self,
            _: scryer_domain::IndexerConfig,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn touch_last_error(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(
            &self,
            _: crate::IndexerConfigUpdate,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ImportRepository for TestImportRepo {
        async fn queue_import_request(
            &self,
            _: ClientJobLocator,
            _: String,
            _: String,
        ) -> AppResult<String> {
            if let Some(message) = self.queue_error.as_ref() {
                return Err(AppError::Repository(message.clone()));
            }
            Ok(String::new())
        }

        async fn get_import_by_id(&self, _: &str) -> AppResult<Option<ImportRecord>> {
            Ok(None)
        }

        async fn update_import_status(
            &self,
            _: &str,
            status: ImportStatus,
            result_json: Option<String>,
        ) -> AppResult<()> {
            self.status_updates
                .lock()
                .await
                .push(TestImportStatusUpdate(status, result_json));
            Ok(())
        }

        async fn update_import_transfer_progress(
            &self,
            _: &str,
            _: scryer_domain::ImportTransferPhase,
            _: i64,
            _: i64,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn recover_stale_processing_imports(&self, _: i64) -> AppResult<u64> {
            Ok(0)
        }

        async fn recover_stale_processing_imports_for_type(
            &self,
            _: ImportType,
            _: i64,
        ) -> AppResult<u64> {
            Ok(0)
        }

        async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
            Ok(vec![])
        }

        async fn list_pending_imports_for_type(
            &self,
            _: ImportType,
        ) -> AppResult<Vec<ImportRecord>> {
            Ok(vec![])
        }

        async fn list_imports_for_identities(
            &self,
            identities: &[ClientJobLocator],
        ) -> AppResult<Vec<ImportRecord>> {
            Ok(self
                .stored_imports()
                .into_iter()
                .filter(|record| {
                    identities.iter().any(|identity| {
                        record.source_client_id.as_deref().unwrap_or("")
                            == identity.client_id_or_empty()
                            && record.source_system == identity.client_type
                            && record.source_ref == identity.item_id
                    })
                })
                .collect())
        }

        async fn list_imports(&self, _: usize) -> AppResult<Vec<ImportRecord>> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl DomainEventRepository for TestDomainEventRepo {
        async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
            let mut events = self.events.lock().await;
            let sequence = events
                .last()
                .map(|existing| existing.sequence + 1)
                .unwrap_or(1);
            let stored = DomainEvent {
                sequence,
                event_id: event.event_id,
                occurred_at: event.occurred_at,
                actor_kind: event.actor_kind,
                actor_user_id: event.actor_user_id,
                actor_display_name: event.actor_display_name,
                title_id: event.title_id,
                facet: event.facet,
                correlation_id: event.correlation_id,
                causation_id: event.causation_id,
                schema_version: event.schema_version,
                stream: event.stream,
                payload: event.payload,
            };
            events.push(stored.clone());
            Ok(stored)
        }

        async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
            let mut stored = Vec::with_capacity(events.len());
            for event in events {
                stored.push(self.append(event).await?);
            }
            Ok(stored)
        }

        async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .filter(|event| {
                    filter
                        .after_sequence
                        .is_none_or(|after| event.sequence > after)
                        && filter
                            .before_sequence
                            .is_none_or(|before| event.sequence < before)
                        && filter.title_id.as_ref().is_none_or(|title_id| {
                            event.title_id.as_deref() == Some(title_id.as_str())
                        })
                        && filter
                            .facet
                            .as_ref()
                            .is_none_or(|facet| event.facet.as_ref() == Some(facet))
                        && filter.event_types.as_ref().is_none_or(|event_types| {
                            event_types
                                .iter()
                                .any(|event_type| &event.payload.event_type() == event_type)
                        })
                })
                .cloned()
                .collect())
        }

        async fn count_dashboard_activity_events(
            &self,
            _: &[String],
            _: chrono::DateTime<chrono::Utc>,
            _: chrono::DateTime<chrono::Utc>,
            _: chrono::DateTime<chrono::Utc>,
        ) -> AppResult<crate::DashboardActivityStats> {
            Ok(crate::DashboardActivityStats::default())
        }

        async fn count_title_history_page_events(
            &self,
            event_types: Option<&[TitleHistoryEventType]>,
            title_ids: Option<&[String]>,
            download_id: Option<&str>,
        ) -> AppResult<i64> {
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .rev()
                .filter_map(crate::event_views::title_history_record_from_domain_event)
                .filter(|record| {
                    event_types.is_none_or(|values| values.contains(&record.event_type))
                        && title_ids.is_none_or(|values| values.contains(&record.title_id))
                        && download_id
                            .is_none_or(|value| record.download_id.as_deref() == Some(value))
                })
                .count() as i64)
        }

        async fn list_title_history_page_events(
            &self,
            event_types: Option<&[TitleHistoryEventType]>,
            title_ids: Option<&[String]>,
            download_id: Option<&str>,
            limit: usize,
            offset: usize,
        ) -> AppResult<Vec<DomainEvent>> {
            let page_size = if limit == 0 { usize::MAX } else { limit };
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .rev()
                .filter(|event| {
                    crate::event_views::title_history_record_from_domain_event(event).is_some_and(
                        |record| {
                            event_types.is_none_or(|values| values.contains(&record.event_type))
                                && title_ids.is_none_or(|values| values.contains(&record.title_id))
                                && download_id.is_none_or(|value| {
                                    record.download_id.as_deref() == Some(value)
                                })
                        },
                    )
                })
                .skip(offset)
                .take(page_size)
                .cloned()
                .collect())
        }

        async fn list_after_sequence(
            &self,
            after_sequence: i64,
            limit: usize,
        ) -> AppResult<Vec<DomainEvent>> {
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .take(limit)
                .cloned()
                .collect())
        }

        async fn delete_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
            Ok(0)
        }

        async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
            let offsets = self.subscriber_offsets.lock().await;
            Ok(*offsets.get(subscriber).unwrap_or(&0))
        }

        async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
            let mut offsets = self.subscriber_offsets.lock().await;
            offsets.insert(subscriber.to_string(), sequence);
            Ok(())
        }
    }

    #[async_trait]
    impl TitleRepository for TestTitleRepo {
        async fn list(&self, _: Option<MediaFacet>, _: Option<String>) -> AppResult<Vec<Title>> {
            Ok(self.titles.clone())
        }

        async fn list_by_external_ids(
            &self,
            source: &str,
            values: &[String],
        ) -> AppResult<Vec<Title>> {
            let mut matches = Vec::new();
            let mut seen = HashSet::new();
            for value in values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                if let Some(title) = self.titles.iter().find(|title| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
                }) && seen.insert(title.id.clone())
                {
                    matches.push(title.clone());
                }
            }
            Ok(matches)
        }

        async fn list_for_matching(
            &self,
            _: Option<MediaFacet>,
            _: Option<String>,
        ) -> AppResult<Vec<Title>> {
            Ok(self.titles.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
            Ok(self.titles.iter().find(|title| title.id == id).cloned())
        }

        async fn get_by_facet_and_slug(
            &self,
            facet: MediaFacet,
            slug: &str,
        ) -> AppResult<Option<Title>> {
            let normalized_slug = slug.trim();
            if normalized_slug.is_empty() {
                return Ok(None);
            }

            let matches = self
                .titles
                .iter()
                .filter(|title| {
                    title.facet == facet
                        && title.slug.as_deref().is_some_and(|candidate| {
                            candidate.trim().eq_ignore_ascii_case(normalized_slug)
                        })
                })
                .cloned()
                .collect::<Vec<_>>();

            match matches.as_slice() {
                [] => Ok(None),
                [title] => Ok(Some(title.clone())),
                _ => Err(AppError::Validation(
                    "multiple titles found for slug lookup".into(),
                )),
            }
        }

        async fn find_by_external_id(&self, _: &str, _: &str) -> AppResult<Option<Title>> {
            Ok(None)
        }

        async fn find_by_external_id_in_facet(
            &self,
            _: MediaFacet,
            _: &str,
            _: &str,
        ) -> AppResult<Option<Title>> {
            Ok(None)
        }

        async fn create_or_get_existing(&self, _: Title) -> AppResult<CreateTitleOutcome> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn create(&self, _: Title) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn list_titles_due_for_hydration(
            &self,
            _: usize,
            _: &[MediaFacet],
        ) -> AppResult<Vec<PendingTitleHydration>> {
            Ok(vec![])
        }

        async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn schedule_title_metadata_hydration_retry(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update_monitored(&self, _: &str, _: bool) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_metadata(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<MediaFacet>,
            _: Option<Vec<String>>,
            _: Option<String>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_title_hydrated_metadata(
            &self,
            _: &str,
            _: TitleMetadataUpdate,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn replace_match_state(
            &self,
            _: &str,
            _: Vec<scryer_domain::ExternalId>,
            _: Vec<String>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_folder_path(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
            Ok(0)
        }
    }

    #[async_trait]
    impl TitleRepository for MutableTitleRepo {
        async fn list(&self, _: Option<MediaFacet>, _: Option<String>) -> AppResult<Vec<Title>> {
            Ok(self.titles.lock().await.clone())
        }

        async fn list_by_external_ids(
            &self,
            source: &str,
            values: &[String],
        ) -> AppResult<Vec<Title>> {
            let titles = self.titles.lock().await;
            let mut matches = Vec::new();
            let mut seen = HashSet::new();
            for value in values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                if let Some(title) = titles.iter().find(|title| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
                }) && seen.insert(title.id.clone())
                {
                    matches.push(title.clone());
                }
            }
            Ok(matches)
        }

        async fn list_for_matching(
            &self,
            _: Option<MediaFacet>,
            _: Option<String>,
        ) -> AppResult<Vec<Title>> {
            *self.list_for_matching_calls.lock().await += 1;
            Ok(self.titles.lock().await.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
            Ok(self
                .titles
                .lock()
                .await
                .iter()
                .find(|title| title.id == id)
                .cloned())
        }

        async fn get_by_facet_and_slug(
            &self,
            facet: MediaFacet,
            slug: &str,
        ) -> AppResult<Option<Title>> {
            let normalized_slug = slug.trim();
            if normalized_slug.is_empty() {
                return Ok(None);
            }

            let titles = self.titles.lock().await;
            let matches = titles
                .iter()
                .filter(|title| {
                    title.facet == facet
                        && title.slug.as_deref().is_some_and(|candidate| {
                            candidate.trim().eq_ignore_ascii_case(normalized_slug)
                        })
                })
                .cloned()
                .collect::<Vec<_>>();

            match matches.as_slice() {
                [] => Ok(None),
                [title] => Ok(Some(title.clone())),
                _ => Err(AppError::Validation(
                    "multiple titles found for slug lookup".into(),
                )),
            }
        }

        async fn find_by_external_id(&self, _: &str, _: &str) -> AppResult<Option<Title>> {
            Ok(None)
        }

        async fn find_by_external_id_in_facet(
            &self,
            _: MediaFacet,
            _: &str,
            _: &str,
        ) -> AppResult<Option<Title>> {
            Ok(None)
        }

        async fn create_or_get_existing(&self, _: Title) -> AppResult<CreateTitleOutcome> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn create(&self, _: Title) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn list_titles_due_for_hydration(
            &self,
            _: usize,
            _: &[MediaFacet],
        ) -> AppResult<Vec<PendingTitleHydration>> {
            Ok(vec![])
        }

        async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn schedule_title_metadata_hydration_retry(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update_monitored(&self, _: &str, _: bool) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_metadata(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<MediaFacet>,
            _: Option<Vec<String>>,
            _: Option<String>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_title_hydrated_metadata(
            &self,
            _: &str,
            _: TitleMetadataUpdate,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn replace_match_state(
            &self,
            _: &str,
            _: Vec<scryer_domain::ExternalId>,
            _: Vec<String>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_folder_path(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
            Ok(0)
        }
    }

    fn build_app(
        download_submissions: Arc<TestDownloadSubmissionRepo>,
        imports: Arc<TestImportRepo>,
    ) -> AppUseCase {
        build_app_with_title_repo(Arc::new(NullTitleRepository), download_submissions, imports)
    }

    fn build_app_with_title_repo(
        title_repo: Arc<dyn TitleRepository>,
        download_submissions: Arc<TestDownloadSubmissionRepo>,
        imports: Arc<TestImportRepo>,
    ) -> AppUseCase {
        build_app_with_title_repo_and_download_client(
            title_repo,
            Arc::new(NullDownloadClient),
            download_submissions,
            imports,
        )
    }

    fn build_app_with_title_repo_and_download_client(
        title_repo: Arc<dyn TitleRepository>,
        download_client: Arc<dyn DownloadClient>,
        download_submissions: Arc<TestDownloadSubmissionRepo>,
        imports: Arc<TestImportRepo>,
    ) -> AppUseCase {
        let services = AppServices::builder(
            title_repo,
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            download_client,
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::null_repositories::NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_download_submissions(download_submissions)
        .with_imports(imports)
        .with_domain_events(Arc::new(TestDomainEventRepo::default()))
        .build_partial_for_tests();

        let mut facet_registry = FacetRegistry::new();
        facet_registry.register(Arc::new(crate::catalog::facets::movie::MovieFacetHandler));
        facet_registry.register(Arc::new(
            crate::catalog::facets::series::SeriesFacetHandler::new(MediaFacet::Series),
        ));
        facet_registry.register(Arc::new(
            crate::catalog::facets::series::SeriesFacetHandler::new(MediaFacet::Anime),
        ));

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(facet_registry),
        )
        .with_test_overrides(|services| {
            services.with_download_registry(Arc::new(TestDownloadRegistry {
                ids: Mutex::new(HashMap::new()),
                failing_item_ids: HashSet::new(),
                conflicting_item_ids: HashMap::new(),
                fallback_download_ids: HashMap::new(),
            }))
        })
    }

    fn build_client_item() -> DownloadQueueItem {
        DownloadQueueItem {
            id: Id::new().0,
            title_id: None,
            episode_id: None,
            title_name: "Restart Recovery Show".to_string(),
            facet: Some("series".to_string()),
            category: None,
            client_id: "client-1".to_string(),
            client_name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            state: DownloadQueueState::Completed,
            progress_percent: 100,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "dl-1".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            source_provider: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: vec![],
            tracked_match_type: None,
            seeding: None,
        }
    }

    fn build_tracked_download(id: &str) -> TrackedDownload {
        let mut client_item = build_client_item();
        client_item.download_client_item_id = id.to_string();
        client_item.title_id = None;
        client_item.facet = None;
        client_item.title_name = id.to_string();
        client_item.state = DownloadQueueState::Downloading;
        client_item.progress_percent = 10;

        TrackedDownload {
            download_id: DownloadId::new(),
            id: id.to_string(),
            client_id: client_item.client_id.clone(),
            client_type: client_item.client_type.clone(),
            client_item,
            completed_source: None,
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: None,
            facet: None,
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Unmatched,
            is_trackable: true,
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_execution_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
            burned_by_import_gate: false,
            snapshot_missing_since: None,
        }
    }

    fn no_video_retry_state(
        attempts: u8,
        next_retry_at: chrono::DateTime<Utc>,
    ) -> NoVideoImportRetryState {
        NoVideoImportRetryState {
            signature: NoVideoImportSourceSignature {
                source_path: "/tmp/download".to_string(),
                file_count: 3,
                total_bytes: 1234,
                latest_mtime: Some(Utc::now()),
            },
            attempts,
            next_retry_at,
        }
    }

    #[test]
    fn merge_background_work_state_preserves_no_video_retry_state() {
        let mut tracked = build_tracked_download("dl-1");
        let mut finished = build_tracked_download("dl-1");
        let retry = no_video_retry_state(2, Utc::now() + Duration::seconds(120));

        finished.state = TrackedDownloadState::ImportPending;
        finished.status = TrackedDownloadStatus::Warning;
        finished.status_messages = vec!["retry later".to_string()];
        finished.title_id = Some("title-1".to_string());
        finished.match_type = TitleMatchType::Submission;
        finished.import_attempted = true;
        finished.path_missing_since = Some(Utc::now());
        finished.no_video_import_retry = Some(retry.clone());
        let execution_retry = ImportExecutionRetryState {
            attempts: 3,
            next_retry_at: Utc::now() + Duration::minutes(5),
        };
        finished.import_execution_retry = Some(execution_retry.clone());

        tracked.merge_background_work_state_from(finished);

        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
        assert_eq!(tracked.status, TrackedDownloadStatus::Warning);
        assert_eq!(tracked.status_messages, vec!["retry later"]);
        assert_eq!(tracked.title_id.as_deref(), Some("title-1"));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);
        assert!(tracked.import_attempted);
        assert!(tracked.path_missing_since.is_some());
        assert_eq!(tracked.no_video_import_retry, Some(retry));
        assert_eq!(tracked.import_execution_retry, Some(execution_retry));
    }

    #[test]
    fn reset_for_import_retry_clears_stale_no_video_retry_state() {
        let mut tracked = build_tracked_download("dl-1");
        tracked.state = TrackedDownloadState::ImportBlocked;
        tracked.status = TrackedDownloadStatus::Warning;
        tracked.status_messages = vec!["blocked".to_string()];
        tracked.import_attempted = true;
        tracked.path_missing_since = Some(Utc::now());
        tracked.no_video_import_retry =
            Some(no_video_retry_state(1, Utc::now() + Duration::seconds(30)));
        tracked.import_execution_retry = Some(ImportExecutionRetryState {
            attempts: 2,
            next_retry_at: Utc::now() + Duration::minutes(2),
        });
        tracked.skip_reacquire_on_failure = true;

        tracked.reset_for_import_retry();

        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
        assert_eq!(tracked.status, TrackedDownloadStatus::Ok);
        assert!(tracked.status_messages.is_empty());
        assert!(!tracked.import_attempted);
        assert!(tracked.path_missing_since.is_none());
        assert!(tracked.no_video_import_retry.is_none());
        assert!(tracked.import_execution_retry.is_none());
        assert!(!tracked.skip_reacquire_on_failure);
    }

    #[test]
    fn import_execution_retry_defers_dispatch_until_next_retry_at_and_backs_off() {
        let mut tracked = build_tracked_download("dl-1");
        let now = Utc::now();
        assert!(!tracked.import_retry_deferred(now));

        let attempts = tracked.schedule_import_execution_retry(now, |attempts, next_retry_at| {
            format!(
                "boom (attempt {attempts}) at {}",
                next_retry_at.to_rfc3339()
            )
        });
        assert_eq!(attempts, 1);
        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
        assert_eq!(tracked.status, TrackedDownloadStatus::Warning);
        assert!(!tracked.waiting_for_completed_history);
        assert!(tracked.status_messages[0].starts_with("boom (attempt 1) at "));
        let first = tracked
            .import_execution_retry
            .clone()
            .expect("retry scheduled");
        assert_eq!(first.next_retry_at, now + Duration::seconds(30));
        assert!(tracked.import_retry_deferred(now));
        assert!(tracked.import_retry_deferred(now + Duration::seconds(29)));
        assert!(!tracked.import_retry_deferred(now + Duration::seconds(30)));

        for (attempt, expected_delay) in [
            (2u32, Duration::minutes(2)),
            (3, Duration::minutes(5)),
            (4, Duration::minutes(15)),
            (5, Duration::minutes(15)),
        ] {
            let attempts = tracked.schedule_import_execution_retry(now, |_, _| String::new());
            assert_eq!(attempts, attempt);
            assert_eq!(
                tracked
                    .import_execution_retry
                    .as_ref()
                    .map(|retry| retry.next_retry_at),
                Some(now + expected_delay),
                "attempt {attempt}"
            );
        }

        // Either retry family defers dispatch on its own.
        tracked.clear_import_execution_retry();
        assert!(!tracked.import_retry_deferred(now));
        tracked.no_video_import_retry = Some(no_video_retry_state(1, now + Duration::seconds(30)));
        assert!(tracked.import_retry_deferred(now));
        assert!(!tracked.import_retry_deferred(now + Duration::seconds(30)));
    }

    #[test]
    fn tracked_download_id_for_item_prefers_download_id() {
        let mut item = build_client_item();
        item.download_client_item_id = "10010".to_string();
        item.download_id = Some(" scryer-download-10010 ".to_string());

        assert_eq!(
            tracked_download_id_for_item(&item),
            "download:client-1:nzbget:scryer-download-10010"
        );
    }

    #[tokio::test]
    async fn cache_coalesces_observation_shapes_by_canonical_download_id_and_tracks_foreign_items()
    {
        let canonical_download_id = DownloadId::new();
        let foreign_download_id = DownloadId::new();
        let registry = Arc::new(TestDownloadRegistry {
            ids: Mutex::new(HashMap::from([
                ("job-1".to_string(), canonical_download_id),
                ("foreign-job".to_string(), foreign_download_id),
            ])),
            failing_item_ids: HashSet::new(),
            conflicting_item_ids: HashMap::new(),
            fallback_download_ids: HashMap::new(),
        });
        let app = build_app(
            Arc::new(TestDownloadSubmissionRepo::default()),
            Arc::new(TestImportRepo::default()),
        )
        .with_test_overrides(|services| services.with_download_registry(registry));
        let mut tracker = TrackedDownloadService::new();

        let mut token_observation = build_client_item();
        token_observation.download_client_item_id = "job-1".to_string();
        token_observation.download_id = Some(canonical_download_id.to_wire());
        let legacy_token_id = tracked_download_id_for_item(&token_observation);
        tracker.track(&app, token_observation).await;

        let mut locator_observation = build_client_item();
        locator_observation.download_client_item_id = "job-1".to_string();
        locator_observation.download_id = None;
        tracker.track(&app, locator_observation).await;

        assert_eq!(tracker.cache.len(), 1);
        assert_eq!(
            tracker
                .find(&legacy_token_id)
                .map(|tracked| tracked.download_id),
            Some(canonical_download_id)
        );

        let mut foreign_observation = build_client_item();
        foreign_observation.download_client_item_id = "foreign-job".to_string();
        foreign_observation.is_scryer_origin = false;
        tracker.track(&app, foreign_observation).await;

        assert_eq!(tracker.cache.len(), 2);
        assert!(tracker.cache.contains_key(&foreign_download_id));
    }

    #[tokio::test]
    async fn resolver_conflict_skips_item_without_falling_back_to_the_active_locator_binding() {
        let token_id = DownloadId::new();
        let binding_download_id = DownloadId::new();
        let registry = Arc::new(TestDownloadRegistry {
            ids: Mutex::new(HashMap::new()),
            failing_item_ids: HashSet::new(),
            conflicting_item_ids: HashMap::from([(
                "conflicting-job".to_string(),
                (token_id, binding_download_id),
            )]),
            fallback_download_ids: HashMap::from([(
                "conflicting-job".to_string(),
                binding_download_id,
            )]),
        });
        let app = build_app(
            Arc::new(TestDownloadSubmissionRepo::default()),
            Arc::new(TestImportRepo::default()),
        )
        .with_test_overrides(|services| services.with_download_registry(registry));
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.download_client_item_id = "conflicting-job".to_string();
        item.download_id = Some(token_id.to_wire());
        let legacy_id = tracked_download_id_for_item(&item);
        tracker.track(&app, item).await;

        assert!(tracker.cache.is_empty());
        assert!(tracker.find(&legacy_id).is_none());
        assert!(!tracker.cache.contains_key(&binding_download_id));
    }

    #[tokio::test]
    async fn resolver_unavailability_falls_back_to_the_active_locator_binding() {
        let binding_download_id = DownloadId::new();
        let registry = Arc::new(TestDownloadRegistry {
            ids: Mutex::new(HashMap::new()),
            failing_item_ids: HashSet::from(["unavailable-job".to_string()]),
            conflicting_item_ids: HashMap::new(),
            fallback_download_ids: HashMap::from([(
                "unavailable-job".to_string(),
                binding_download_id,
            )]),
        });
        let app = build_app(
            Arc::new(TestDownloadSubmissionRepo::default()),
            Arc::new(TestImportRepo::default()),
        )
        .with_test_overrides(|services| services.with_download_registry(registry));
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.download_client_item_id = "unavailable-job".to_string();
        tracker.track(&app, item).await;

        assert_eq!(tracker.cache.len(), 1);
        assert!(tracker.cache.contains_key(&binding_download_id));
    }

    #[tokio::test]
    async fn resolver_failure_skips_tracked_state_without_erasing_the_visible_queue_item() {
        let unaffected_download_id = DownloadId::new();
        let registry = Arc::new(TestDownloadRegistry {
            ids: Mutex::new(HashMap::from([(
                "unaffected-job".to_string(),
                unaffected_download_id,
            )])),
            failing_item_ids: HashSet::from(["failing-job".to_string()]),
            conflicting_item_ids: HashMap::new(),
            fallback_download_ids: HashMap::new(),
        });
        let app = build_app(
            Arc::new(TestDownloadSubmissionRepo::default()),
            Arc::new(TestImportRepo::default()),
        )
        .with_test_overrides(|services| services.with_download_registry(registry));
        let mut tracker = TrackedDownloadService::new();

        let mut unaffected = build_client_item();
        unaffected.download_client_item_id = "unaffected-job".to_string();
        tracker.track(&app, unaffected).await;

        let mut failing = build_client_item();
        failing.download_client_item_id = "failing-job".to_string();
        let failing_visible_queue_item = failing.clone();
        let failing_id = tracked_download_id_for_item(&failing);
        tracker.track(&app, failing).await;

        assert!(tracker.cache.contains_key(&unaffected_download_id));
        assert!(tracker.find(&failing_id).is_none());
        assert_eq!(tracker.get_all().len(), 1);
        assert_eq!(
            failing_visible_queue_item.download_client_item_id, "failing-job",
            "queue projection input remains available when tracking is skipped"
        );
    }

    #[tokio::test]
    async fn restart_reconstruction_reads_state_reason_and_detail_by_canonical_download_id() {
        let canonical_download_id = DownloadId::new();
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        download_submissions
            .canonical_identity_tracked_states
            .lock()
            .await
            .insert(
                canonical_download_id,
                TrackedDownloadState::Failed.as_str().to_string(),
            );
        download_submissions
            .canonical_identity_tracked_state_reasons
            .lock()
            .await
            .insert(
                canonical_download_id,
                IMPORT_GATE_REJECTED_TRACKED_STATE_REASON.to_string(),
            );
        download_submissions
            .canonical_identity_tracked_state_details
            .lock()
            .await
            .insert(
                canonical_download_id,
                "persisted import gate detail".to_string(),
            );
        let registry = Arc::new(TestDownloadRegistry {
            ids: Mutex::new(HashMap::from([(
                "restart-job".to_string(),
                canonical_download_id,
            )])),
            failing_item_ids: HashSet::new(),
            conflicting_item_ids: HashMap::new(),
            fallback_download_ids: HashMap::new(),
        });
        let app = build_app(download_submissions, Arc::new(TestImportRepo::default()))
            .with_test_overrides(|services| services.with_download_registry(registry));
        let mut tracker = TrackedDownloadService::new();
        let mut item = build_client_item();
        item.download_client_item_id = "restart-job".to_string();
        item.download_id = Some(canonical_download_id.to_wire());
        let legacy_id = tracked_download_id_for_item(&item);

        tracker.track(&app, item).await;

        let tracked = tracker
            .find(&legacy_id)
            .expect("restart state should reconstruct");
        assert_eq!(tracked.download_id, canonical_download_id);
        assert_eq!(tracked.state, TrackedDownloadState::Failed);
        assert_eq!(tracked.status, TrackedDownloadStatus::Error);
        assert_eq!(
            tracked.status_messages,
            vec!["persisted import gate detail"]
        );
    }

    #[tokio::test]
    async fn a_refresh_without_an_observation_keeps_the_last_one_but_never_overrides_a_new_one() {
        // The same download arrives from more than one source: a queue row
        // carries the plugin's seeding fields, a history row framed as a
        // completed download carries none. Losing the observation on the
        // history refresh would read as "unknown" and hold the torrent forever.
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app(download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut observed = build_client_item();
        observed.seeding = Some(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(false),
            seed_ratio: Some(0.5),
            ..Default::default()
        });
        let id = tracked_download_id_for_item(&observed);
        tracker.track(&app, observed.clone()).await;

        let mut silent = build_client_item();
        silent.seeding = None;
        tracker.track(&app, silent).await;
        assert_eq!(
            tracker
                .find(&id)
                .and_then(|td| td.client_item.seeding.as_ref())
                .and_then(|seeding| seeding.seed_ratio),
            Some(0.5),
            "a refresh with nothing to say must not erase what the client already told us"
        );

        // A fresh observation always wins, so this retains rather than freezes.
        let mut moved_on = build_client_item();
        moved_on.seeding = Some(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(true),
            seed_ratio: Some(2.6),
            ..Default::default()
        });
        tracker.track(&app, moved_on).await;
        let seeding = tracker
            .find(&id)
            .and_then(|td| td.client_item.seeding.clone())
            .expect("observation");
        assert_eq!(seeding.seed_ratio, Some(2.6));
        assert_eq!(seeding.can_remove, Some(true));
    }

    #[tokio::test]
    async fn tracked_download_service_resolves_live_source_id_to_durable_cached_entry() {
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app(download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.download_client_item_id = "10010".to_string();
        item.download_id = Some("scryer-download-10010".to_string());

        let durable_id = tracked_download_id_for_item(&item);
        let live_id = tracked_download_id(
            Some(item.client_id.as_str()),
            &item.client_type,
            &item.download_client_item_id,
        );

        tracker.track(&app, item).await;

        assert!(tracker.find(&durable_id).is_some());
        assert!(tracker.find(&live_id).is_none());
        assert_eq!(
            tracker.resolve_cached_id(&live_id).as_deref(),
            Some(durable_id.as_str())
        );
        assert_eq!(
            tracker.resolve_cached_id(&durable_id).as_deref(),
            Some(durable_id.as_str())
        );

        tracker.update_trackable(&HashSet::from([durable_id.clone()]));
        assert!(tracker.find(&durable_id).is_some_and(|td| td.is_trackable));
    }

    #[tokio::test]
    async fn tracked_download_submission_lookup_uses_canonical_submission() {
        let mut tracked = build_tracked_download("legacy-observed-job");
        tracked.client_item.download_id = Some("legacy-observed-download-id".to_string());
        let canonical_submission = crate::DownloadSubmission {
            download_id: tracked.download_id,
            title_id: "title-canonical".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "canonical-bound-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Canonical Observed Release".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: Some("canonical-observed-request".to_string()),
            scope: crate::SubmissionScope::Title,
        };
        let app = build_app(
            Arc::new(TestDownloadSubmissionRepo {
                canonical_submission: Some(canonical_submission),
                ..Default::default()
            }),
            Arc::new(TestImportRepo::default()),
        );

        let resolved = download_id_submission_for_tracked_download_for_download(
            &app,
            tracked.canonical_download_id(),
            &tracked,
        )
        .await
        .expect("canonical submission should resolve");

        assert_eq!(resolved.download_id, tracked.download_id);
        assert_eq!(resolved.title_id, "title-canonical");
    }

    #[tokio::test]
    async fn failed_download_authorization_uses_canonical_submission() {
        let mut tracked = build_tracked_download("legacy-failed-job");
        tracked.client_item.is_scryer_origin = true;
        let canonical_download_id = tracked.download_id;
        let canonical_submission = crate::DownloadSubmission {
            download_id: canonical_download_id,
            title_id: "title-canonical".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "canonical-bound-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Canonical Failure Release".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: Some("canonical-failure-request".to_string()),
            scope: crate::SubmissionScope::Title,
        };
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            canonical_submission: Some(canonical_submission),
            ..Default::default()
        });
        let app = build_app(download_submissions, Arc::new(TestImportRepo::default()));

        assert!(
            crate::failed_download_handler::tracked_download_has_grabbed_submission(
                &app, &tracked,
            )
            .await,
            "the canonical submission must authorize mark-failed even when the legacy tuple moved"
        );
    }

    #[tokio::test]
    async fn failed_download_authorization_falls_back_to_legacy_submission() {
        let mut tracked = build_tracked_download("legacy-failed-job");
        tracked.client_item.is_scryer_origin = true;
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: Some(crate::DownloadSubmission {
                download_id: DownloadId::new(),
                title_id: "title-legacy".to_string(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "series".to_string(),
                download_client_id: Some("client-1".to_string()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "legacy-failed-job".to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Legacy Failure Release".to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: Some("legacy-failure-request".to_string()),
                scope: crate::SubmissionScope::Title,
            }),
            ..Default::default()
        });
        let app = build_app(download_submissions, Arc::new(TestImportRepo::default()));

        assert!(
            crate::failed_download_handler::tracked_download_has_grabbed_submission(
                &app, &tracked,
            )
            .await,
            "a legacy row without a canonical binding must continue to authorize mark-failed"
        );
    }

    #[tokio::test]
    async fn failed_download_authorization_prefers_legacy_submission_on_divergence() {
        let canonical_download_id = DownloadId::new();
        let canonical_submission = crate::DownloadSubmission {
            download_id: canonical_download_id,
            title_id: "title-canonical".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "canonical-bound-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Canonical Failure Release".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: Some("canonical-failure-request".to_string()),
            scope: crate::SubmissionScope::Title,
        };
        let legacy_submission = crate::DownloadSubmission {
            download_id: DownloadId::new(),
            title_id: "title-legacy".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "legacy-failed-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Legacy Failure Release".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: Some("legacy-failure-request".to_string()),
            scope: crate::SubmissionScope::Title,
        };
        let download_submissions = TestDownloadSubmissionRepo {
            submission: Some(legacy_submission.clone()),
            canonical_submission: Some(canonical_submission),
            ..Default::default()
        };

        let resolved = download_submissions
            .find_by_client_item_id_for_download(
                Some(&canonical_download_id),
                &ClientJobLocator::new(Some("client-1"), "nzbget", "legacy-failed-job"),
            )
            .await
            .expect("submission lookup should succeed")
            .expect("a submission should resolve");

        assert_eq!(resolved.download_id, legacy_submission.download_id);
        assert_eq!(resolved.title_id, "title-legacy");
    }

    #[tokio::test]
    async fn resolves_submission_by_download_id_when_client_item_id_differs() {
        let download_id = "cc025b54883bbdc61258e9d5627b3bd1613241b2";
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: Some(crate::DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: "title-1".to_string(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: Some("client-1".to_string()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: download_id.to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Paperman.2012.720p.WEB-DL".to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: crate::SubmissionScope::Title,
            }),
            canonical_submission: None,
            submission_identity: Some(crate::DownloadSubmissionIdentity {
                download_id: Some(download_id.to_string()),
            }),
            ..Default::default()
        });
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app(download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.client_type = "nzbget".to_string();
        item.download_client_item_id = "2".to_string();
        item.download_id = Some(download_id.to_string());
        item.title_name = "Paperman.2012.720p.WEB-DL".to_string();
        item.facet = None;

        let tracked_id = tracked_download_id_for_item(&item);
        tracker.track(&app, item).await;

        let tracked = tracker.find(&tracked_id).expect("tracked download");
        assert_eq!(tracked.title_id.as_deref(), Some("title-1"));
        assert_eq!(tracked.facet.as_deref(), Some("movie"));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);
        assert_eq!(
            tracked.source_title.as_deref(),
            Some("Paperman.2012.720p.WEB-DL")
        );
    }

    #[tokio::test]
    async fn exact_orphan_submission_does_not_block_download_id_promotion() {
        let download_id = "e9527810bc94e83401584069306f1064ca28762a";
        let orphan = crate::DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: String::new(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: String::new(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "qbittorrent".to_string(),
            download_client_item_id: download_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb.rar".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: crate::SubmissionScope::Orphan,
        };
        let managed = crate::DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "title-1".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "qbittorrent".to_string(),
            download_client_item_id: download_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: crate::SubmissionScope::Title,
        };
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: Some(orphan),
            download_id_submissions: Arc::new(Mutex::new(vec![(
                managed,
                crate::DownloadSubmissionIdentity {
                    download_id: Some(download_id.to_string()),
                },
            )])),
            ..Default::default()
        });
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(
            Arc::new(TestTitleRepo::default()),
            download_submissions,
            imports,
        );
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.client_type = "qbittorrent".to_string();
        item.client_name = "qbittorrent".to_string();
        item.download_client_item_id = download_id.to_string();
        item.download_id = Some(download_id.to_string());
        item.title_id = None;
        item.title_name = "Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb.rar".to_string();
        item.facet = None;
        item.is_scryer_origin = true;

        let tracked_id = tracked_download_id_for_item(&item);
        tracker.track(&app, item).await;

        let tracked = tracker.find(&tracked_id).expect("tracked download");
        assert_eq!(tracked.title_id.as_deref(), Some("title-1"));
        assert_eq!(tracked.facet.as_deref(), Some("movie"));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);
        assert_eq!(
            tracked.source_title.as_deref(),
            Some("Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb")
        );
    }

    #[tokio::test]
    async fn late_submission_promotes_titleless_unmatched_download_and_clears_stale_import_block() {
        let download_id = "e9527810bc94e83401584069306f1064ca28762a";
        let mutable_submission = Arc::new(Mutex::new(None));
        let mutable_submission_identity = Arc::new(Mutex::new(None));
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            mutable_submission: Some(mutable_submission.clone()),
            mutable_submission_identity: Some(mutable_submission_identity.clone()),
            ..Default::default()
        });
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(
            Arc::new(TestTitleRepo::default()),
            download_submissions,
            imports,
        );
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.client_type = "qbittorrent".to_string();
        item.client_name = "qbittorrent".to_string();
        item.download_client_item_id = download_id.to_string();
        item.download_id = Some(download_id.to_string());
        item.title_id = None;
        item.title_name = "Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb.rar".to_string();
        item.facet = None;
        item.is_scryer_origin = true;

        let tracked_id = tracked_download_id_for_item(&item);
        tracker.track(&app, item.clone()).await;
        let tracked = tracker.find(&tracked_id).expect("tracked download");
        assert!(tracked.title_id.is_none());
        assert!(matches!(
            tracked.match_type,
            TitleMatchType::Unmatched | TitleMatchType::IdOnly | TitleMatchType::TitleParse
        ));

        let tracked = tracker.find_mut(&tracked_id).expect("tracked download mut");
        tracked.state = TrackedDownloadState::ImportBlocked;
        tracked.warn("Unable to resolve title from completed download");

        *mutable_submission.lock().await = Some(crate::DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "title-1".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "qbittorrent".to_string(),
            download_client_item_id: download_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: crate::SubmissionScope::Title,
        });
        *mutable_submission_identity.lock().await = Some(crate::DownloadSubmissionIdentity {
            download_id: Some(download_id.to_string()),
        });

        tracker.track(&app, item).await;

        let tracked = tracker.find(&tracked_id).expect("tracked download");
        assert_eq!(tracked.title_id.as_deref(), Some("title-1"));
        assert_eq!(tracked.facet.as_deref(), Some("movie"));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);
        assert_eq!(
            tracked.source_title.as_deref(),
            Some("Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb")
        );
        assert_eq!(tracked.status, TrackedDownloadStatus::Ok);
        assert!(tracked.status_messages.is_empty());
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
        assert!(!tracked.import_attempted);
    }

    fn build_completed_download(
        client_type: &str,
        item_id: &str,
        name: &str,
        dest_dir: &str,
        category: Option<&str>,
    ) -> CompletedDownload {
        CompletedDownload {
            client_type: client_type.to_string(),
            client_id: "client-1".to_string(),
            download_client_item_id: item_id.to_string(),
            download_id: None,
            name: name.to_string(),
            release_name: None,
            dest_dir: dest_dir.to_string(),
            category: category.map(str::to_string),
            size_bytes: None,
            completed_at: None,
            parameters: vec![],
        }
    }

    fn build_title(name: &str, facet: MediaFacet, aliases: &[&str]) -> Title {
        Title {
            id: Id::new().0,
            name: name.to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            facet,
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: aliases.iter().map(|value| value.to_string()).collect(),
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    #[tokio::test]
    async fn reconstruct_state_does_not_trust_status_only_import_record() {
        let download_id = "scryer-download:restart-recovery";
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: Some(crate::DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: "title-1".to_string(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "series".to_string(),
                download_client_id: Some("client-1".to_string()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "dl-1".to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Restart Recovery Show".to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: crate::SubmissionScope::Title,
            }),
            canonical_submission: None,
            submission_identity: Some(crate::DownloadSubmissionIdentity {
                download_id: Some(download_id.to_string()),
            }),
            mutable_submission: None,
            mutable_submission_identity: None,
            tracked_state: None,
            tracked_state_updates: Arc::new(Mutex::new(vec![])),
            recorded_submissions: Arc::new(Mutex::new(vec![])),
            download_id_submissions: Arc::new(Mutex::new(vec![])),
            identity_tracked_states: Arc::new(Mutex::new(HashMap::new())),
            identity_tracked_state_reasons: Arc::new(Mutex::new(HashMap::new())),
            identity_tracked_state_details: Arc::new(Mutex::new(HashMap::new())),
            canonical_identity_tracked_states: Arc::new(Mutex::new(HashMap::new())),
            canonical_identity_tracked_state_reasons: Arc::new(Mutex::new(HashMap::new())),
            canonical_identity_tracked_state_details: Arc::new(Mutex::new(HashMap::new())),
        });
        let imports = Arc::new(TestImportRepo {
            import_record: Some(ImportRecord {
                id: Id::new().0,
                source_client_id: Some("client-1".to_string()),
                source_system: "nzbget".to_string(),
                source_ref: "dl-1".to_string(),
                import_type: ImportType::SeriesDownload,
                status: ImportStatus::Skipped,
                payload_json: "{}".to_string(),
                result_json: Some(r#"{"skip_reason":"already_imported"}"#.to_string()),
                download_id: Some(download_id.to_string()),
                import_transfer_phase: None,
                import_transfer_bytes: None,
                import_transfer_total_bytes: None,
                import_transfer_started_at: None,
                import_transfer_updated_at: None,
                started_at: None,
                finished_at: None,
                created_at: "now".to_string(),
                updated_at: "now".to_string(),
            }),
            ..Default::default()
        });
        let app = build_app(download_submissions.clone(), imports);
        let mut tracker = TrackedDownloadService::new();
        let mut item = build_client_item();
        item.download_id = Some(download_id.to_string());
        let tracked_id = tracked_download_id_for_item(&item);

        tracker.track(&app, item).await;

        let tracked = tracker.find(&tracked_id).expect("tracked download");
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
        assert!(
            download_submissions
                .tracked_state_updates
                .lock()
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn reconstruct_state_does_not_recover_client_local_state_from_other_client() {
        let download_id = "10010";
        let identity = crate::DownloadSubmissionIdentity {
            download_id: Some(download_id.to_string()),
        };
        let other_client_source = ClientJobLocator::new(Some("client-2"), "nzbget", "dl-1");
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        download_submissions
            .record_identity_tracked_state(
                &identity,
                Some(&other_client_source),
                TrackedDownloadState::Imported.as_str(),
                None,
                None,
            )
            .await
            .expect("other client state should record");
        let app = build_app(download_submissions, Arc::new(TestImportRepo::default()));
        let mut tracker = TrackedDownloadService::new();
        let mut item = build_client_item();
        item.download_id = Some(download_id.to_string());
        let tracked_id = tracked_download_id_for_item(&item);

        tracker.track(&app, item).await;

        let tracked = tracker.find(&tracked_id).expect("tracked download");
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
    }

    #[tokio::test]
    async fn reconstruct_state_ignores_item_id_import_record_without_download_id() {
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo {
            import_record: Some(ImportRecord {
                id: Id::new().0,
                source_client_id: Some("client-1".to_string()),
                source_system: "nzbget".to_string(),
                source_ref: "dl-1".to_string(),
                import_type: ImportType::SeriesDownload,
                status: ImportStatus::Completed,
                payload_json: "{}".to_string(),
                result_json: None,
                download_id: None,
                import_transfer_phase: None,
                import_transfer_bytes: None,
                import_transfer_total_bytes: None,
                import_transfer_started_at: None,
                import_transfer_updated_at: None,
                started_at: None,
                finished_at: None,
                created_at: "now".to_string(),
                updated_at: "now".to_string(),
            }),
            ..Default::default()
        });
        let app = build_app(download_submissions.clone(), imports);
        let mut tracker = TrackedDownloadService::new();

        tracker.track(&app, build_client_item()).await;

        let tracked = tracker.find("client-1:dl-1").expect("tracked download");
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
        assert!(
            download_submissions
                .tracked_state_updates
                .lock()
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn persist_terminal_state_marks_burned_import_gate_failure() {
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let app = build_app(
            download_submissions.clone(),
            Arc::new(TestImportRepo::default()),
        );
        let mut tracker = TrackedDownloadService::new();
        let mut item = build_client_item();
        item.download_id = Some("scryer-download:burned-import".to_string());
        let tracked_id = tracked_download_id_for_item(&item);
        tracker.track(&app, item).await;
        let tracked = tracker.find_mut(&tracked_id).expect("tracked download");
        tracked.burned_by_import_gate = true;
        tracked.status_messages = vec!["release language does not match the title".to_string()];

        assert!(
            tracker
                .persist_terminal_state(&app, &tracked_id, TrackedDownloadState::Failed)
                .await
        );
        assert!(
            download_submissions
                .identity_tracked_state_reasons
                .lock()
                .await
                .values()
                .any(|reason| reason == "import_gate_rejected")
        );
        assert!(
            download_submissions
                .identity_tracked_state_details
                .lock()
                .await
                .values()
                .any(|detail| detail == "release language does not match the title")
        );
    }

    #[tokio::test]
    async fn persist_terminal_state_returns_false_when_repository_write_fails() {
        #[derive(Default)]
        struct FailingDownloadSubmissionRepo;

        #[async_trait]
        impl DownloadSubmissionRepository for FailingDownloadSubmissionRepo {
            async fn record_submission(&self, _: crate::DownloadSubmission) -> AppResult<()> {
                Ok(())
            }

            async fn record_ambiguous_submission(
                &self,
                _: crate::DownloadSubmission,
            ) -> AppResult<()> {
                Ok(())
            }

            async fn record_submission_with_identity(
                &self,
                _: crate::DownloadSubmission,
                _: crate::DownloadSubmissionIdentity,
                _: Option<crate::PersistedSeedGoals>,
            ) -> AppResult<crate::CanonicalDownloadIdentityDisposition> {
                Ok(crate::CanonicalDownloadIdentityDisposition::Requested)
            }

            async fn find_by_client_item_id(
                &self,
                _: &ClientJobLocator,
            ) -> AppResult<Option<crate::DownloadSubmission>> {
                Ok(None)
            }

            async fn list_for_client_items(
                &self,
                _: &[ClientJobLocator],
            ) -> AppResult<Vec<crate::DownloadSubmission>> {
                Ok(vec![])
            }

            async fn list_for_title(&self, _: &str) -> AppResult<Vec<crate::DownloadSubmission>> {
                Ok(vec![])
            }

            async fn find_by_title_and_request_signature(
                &self,
                _: &str,
                _: &str,
                _: crate::DownloadSubmissionPurpose,
                _: &crate::SubmissionScope,
            ) -> AppResult<Option<crate::DownloadSubmission>> {
                Ok(None)
            }

            async fn delete_for_title(&self, _: &str) -> AppResult<()> {
                Ok(())
            }

            async fn delete_by_client_item_id(&self, _: &ClientJobLocator) -> AppResult<()> {
                Ok(())
            }

            async fn update_tracked_state(&self, _: &ClientJobLocator, _: &str) -> AppResult<()> {
                Err(AppError::Repository("boom".into()))
            }

            async fn get_tracked_state(&self, _: &ClientJobLocator) -> AppResult<Option<String>> {
                Ok(None)
            }
        }

        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::null_repositories::NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_download_submissions(Arc::new(FailingDownloadSubmissionRepo))
        .with_imports(Arc::new(TestImportRepo::default()))
        .build_partial_for_tests();

        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
        .with_test_overrides(|services| {
            services.with_download_registry(Arc::new(TestDownloadRegistry {
                ids: Mutex::new(HashMap::new()),
                failing_item_ids: HashSet::new(),
                conflicting_item_ids: HashMap::new(),
                fallback_download_ids: HashMap::new(),
            }))
        });

        let mut tracker = TrackedDownloadService::new();
        tracker.track(&app, build_client_item()).await;

        assert!(
            tracker.find("client-1:dl-1").is_some(),
            "tracked download should exist before persistence attempt"
        );

        let persisted = tracker
            .persist_terminal_state(&app, "client-1:dl-1", TrackedDownloadState::Failed)
            .await;

        assert!(!persisted, "persistence should report failure");
        assert!(
            tracker.find("client-1:dl-1").is_some(),
            "tracked download should remain cached when persistence fails"
        );
    }

    #[tokio::test]
    async fn completed_episode_download_uses_title_parse_to_become_import_pending() {
        let title = build_title(
            "House of Ravens",
            MediaFacet::Anime,
            &["RAVENCOURT The Last Regent"],
        );
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: None,
            canonical_submission: None,
            submission_identity: None,
            mutable_submission: None,
            mutable_submission_identity: None,
            tracked_state: None,
            tracked_state_updates: Arc::new(Mutex::new(vec![])),
            recorded_submissions: Arc::new(Mutex::new(vec![])),
            download_id_submissions: Arc::new(Mutex::new(vec![])),
            identity_tracked_states: Arc::new(Mutex::new(HashMap::new())),
            identity_tracked_state_reasons: Arc::new(Mutex::new(HashMap::new())),
            identity_tracked_state_details: Arc::new(Mutex::new(HashMap::new())),
            canonical_identity_tracked_states: Arc::new(Mutex::new(HashMap::new())),
            canonical_identity_tracked_state_reasons: Arc::new(Mutex::new(HashMap::new())),
            canonical_identity_tracked_state_details: Arc::new(Mutex::new(HashMap::new())),
        });
        let imports = Arc::new(TestImportRepo::default());
        let tempdir = tempfile::tempdir().expect("tempdir");
        let completed_dir = tempdir
            .path()
            .join("RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL");
        std::fs::create_dir_all(&completed_dir).expect("create completed download dir");
        // The directory alone is not importable: a non-Scryer-origin download
        // with no video is classified NoImportableVideo and parked at
        // Downloading, which masks the title-parse outcome under test.
        std::fs::write(
            completed_dir.join("RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL.mkv"),
            b"video",
        )
        .expect("write fixture video");
        let download_client = Arc::new(TestDownloadClient {
            completed_downloads: Arc::new(Mutex::new(vec![build_completed_download(
                "weaver",
                "job-1",
                "RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL",
                completed_dir.to_string_lossy().as_ref(),
                Some("anime"),
            )])),
            ..Default::default()
        });
        let app = build_app_with_title_repo_and_download_client(
            title_repo,
            download_client,
            download_submissions,
            imports,
        );
        let mut tracker = TrackedDownloadService::new();
        let mut item = build_client_item();
        item.client_type = "weaver".to_string();
        item.client_name = "weaver".to_string();
        item.download_client_item_id = "job-1".to_string();
        item.title_name = "RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL".to_string();
        item.facet = Some("anime".to_string());
        item.is_scryer_origin = false;

        tracker.track(&app, item).await;

        let tracked = tracker.find("client-1:job-1").expect("tracked download");
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::TitleParse);

        let tracked = tracker
            .find_mut("client-1:job-1")
            .expect("tracked download mut");
        crate::completed_download_handler::check(&app, tracked).await;

        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
        assert!(tracked.status_messages.is_empty());
    }

    #[tokio::test]
    async fn tracked_download_resolution_marks_embedded_external_id_matches_as_id_only() {
        let mut title = build_title("Paper Lantern", MediaFacet::Movie, &[]);
        title.external_ids.push(scryer_domain::ExternalId {
            source: "imdb".to_string(),
            value: "tt2388725".to_string(),
        });
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();
        let mut item = build_client_item();
        item.client_type = "weaver".to_string();
        item.client_name = "weaver".to_string();
        item.download_client_item_id = "job-imdb".to_string();
        item.title_name = "Paper.Lantern.2012.[tt2388725].1080p.BluRay.x264-GRP".to_string();
        item.facet = Some("movie".to_string());
        item.is_scryer_origin = false;

        tracker.track(&app, item).await;

        let tracked = tracker.find("client-1:job-imdb").expect("tracked download");
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::IdOnly);
    }

    #[tokio::test]
    async fn assigning_title_to_completed_observation_keeps_manual_import_actionable() {
        let title = build_title("Paper Lantern", MediaFacet::Movie, &[]);
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let tempdir = tempfile::tempdir().expect("tempdir");
        let completed_dir = tempdir.path().join("4f8e2c7a91b6d3e0");
        std::fs::create_dir_all(&completed_dir).expect("create completed download dir");
        std::fs::write(completed_dir.join("Paper.Lantern.2020.1080p.mkv"), b"video")
            .expect("write completed download video");
        let download_client = Arc::new(TestDownloadClient {
            completed_downloads: Arc::new(Mutex::new(vec![build_completed_download(
                "weaver",
                "job-manual-movie",
                "4f8e2c7a91b6d3e0",
                completed_dir.to_string_lossy().as_ref(),
                Some("movie"),
            )])),
            ..Default::default()
        });
        let app = build_app_with_title_repo_and_download_client(
            title_repo,
            download_client,
            download_submissions,
            imports,
        );
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.client_type = "weaver".to_string();
        item.client_name = "weaver".to_string();
        item.download_client_item_id = "job-manual-movie".to_string();
        item.title_name = "4f8e2c7a91b6d3e0".to_string();
        item.facet = Some("movie".to_string());
        item.is_scryer_origin = false;

        tracker.track(&app, item).await;
        let tracked = tracker
            .find_mut("client-1:job-manual-movie")
            .expect("tracked download mut");
        crate::completed_download_handler::check(&app, tracked).await;
        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));

        let tracked = tracker
            .find_mut("client-1:job-manual-movie")
            .expect("tracked download mut");
        assign_title_to_tracked_download(&app, tracked, &title).await;

        // Movie assignment records the operator's target while keeping the
        // observation actionable for an explicit manual import.
        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);

        crate::completed_download_handler::check(&app, tracked).await;
        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
    }

    #[tokio::test]
    async fn repeated_unmatched_snapshot_uses_cached_matcher_until_title_event_invalidates_it() {
        let titles = Arc::new(Mutex::new(Vec::new()));
        let list_for_matching_calls = Arc::new(Mutex::new(0usize));
        let title_repo = Arc::new(MutableTitleRepo {
            titles: titles.clone(),
            list_for_matching_calls: list_for_matching_calls.clone(),
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut initial = build_client_item();
        initial.client_type = "weaver".to_string();
        initial.client_name = "weaver".to_string();
        initial.download_client_item_id = "job-manual-movie-reresolve".to_string();
        initial.title_name = "Paper Lantern".to_string();
        initial.facet = Some("movie".to_string());
        initial.is_scryer_origin = false;

        tracker.track(&app, initial).await;
        let tracked = tracker
            .find("client-1:job-manual-movie-reresolve")
            .expect("tracked download");
        assert!(tracked.title_id.is_none());
        assert_eq!(tracked.match_type, TitleMatchType::Unmatched);
        assert_eq!(*list_for_matching_calls.lock().await, 1);

        let mut unchanged = build_client_item();
        unchanged.client_type = "weaver".to_string();
        unchanged.client_name = "weaver".to_string();
        unchanged.download_client_item_id = "job-manual-movie-reresolve".to_string();
        unchanged.title_name = "Paper Lantern".to_string();
        unchanged.facet = Some("movie".to_string());
        unchanged.is_scryer_origin = false;

        tracker.track(&app, unchanged).await;

        let tracked = tracker
            .find("client-1:job-manual-movie-reresolve")
            .expect("tracked download");
        assert!(tracked.title_id.is_none());
        assert_eq!(tracked.match_type, TitleMatchType::Unmatched);
        assert_eq!(
            *list_for_matching_calls.lock().await,
            1,
            "unchanged unmatched polls should reuse the cached matcher"
        );

        let title = build_title("Paper Lantern", MediaFacet::Movie, &[]);
        titles.lock().await.push(title.clone());
        app.append_domain_event(crate::domain_events::new_title_domain_event(
            None,
            &title,
            scryer_domain::DomainEventPayload::TitleUpdated(scryer_domain::TitleUpdatedEventData {
                title: crate::domain_events::title_context_snapshot(&title),
            }),
        ))
        .await
        .expect("invalidate matcher");

        let mut updated = build_client_item();
        updated.client_type = "weaver".to_string();
        updated.client_name = "weaver".to_string();
        updated.download_client_item_id = "job-manual-movie-reresolve".to_string();
        updated.title_name = "Paper Lantern".to_string();
        updated.facet = Some("movie".to_string());
        updated.is_scryer_origin = false;

        tracker.track(&app, updated).await;

        let tracked = tracker
            .find("client-1:job-manual-movie-reresolve")
            .expect("tracked download");
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::TitleParse);
        assert_eq!(
            *list_for_matching_calls.lock().await,
            2,
            "title events should invalidate the cached matcher"
        );
    }

    #[tokio::test]
    async fn unchanged_unmatched_snapshot_does_not_reresolve_every_poll() {
        let title_repo = Arc::new(TestTitleRepo::default());
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions.clone(), imports);
        let mut tracker = TrackedDownloadService::new();

        let mut initial = build_client_item();
        initial.client_type = "weaver".to_string();
        initial.client_name = "weaver".to_string();
        initial.download_client_item_id = "job-unmatched-repeat".to_string();
        initial.title_name = "Paper Lantern".to_string();
        initial.facet = Some("movie".to_string());
        initial.category = Some("movie".to_string());
        initial.is_scryer_origin = false;

        tracker.track(&app, initial.clone()).await;
        tracker.track(&app, initial).await;

        let recorded = download_submissions.recorded_submissions.lock().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].download_client_item_id,
            "job-unmatched-repeat".to_string()
        );
    }

    #[tokio::test]
    async fn assigning_title_to_blocked_download_keeps_manual_import_actionable_even_if_client_is_still_downloading()
     {
        let title = build_title("Paper Lantern", MediaFacet::Movie, &[]);
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.client_type = "weaver".to_string();
        item.client_name = "weaver".to_string();
        item.download_client_item_id = "job-manual-movie-downloading".to_string();
        item.title_name = "Paper Lantern".to_string();
        item.facet = Some("movie".to_string());
        item.state = DownloadQueueState::Downloading;
        item.is_scryer_origin = false;

        tracker.track(&app, item).await;
        let tracked = tracker
            .find_mut("client-1:job-manual-movie-downloading")
            .expect("tracked download mut");
        tracked.state = TrackedDownloadState::ImportBlocked;
        tracked.match_type = TitleMatchType::Unmatched;
        tracked.title_id = None;

        assign_title_to_tracked_download(&app, tracked, &title).await;

        // Assignment records the movie target but does not release a blocked
        // download into automatic import, even while the client is downloading.
        assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);
        assert!(!tracked.import_attempted);
        assert_eq!(
            tracked.status_messages,
            vec![
                "Automatic import needs operator review. Open Manual Import and confirm the file mapping to continue."
                    .to_string()
            ]
        );

        crate::completed_download_handler::check(&app, tracked).await;
        assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
    }

    #[test]
    fn import_blocked_fallback_explains_episode_mapping_for_assigned_anime() {
        let mut tracked = build_tracked_download("job-ambiguous-anime");
        tracked.title_id = Some("title-1".to_string());
        tracked.facet = Some("anime".to_string());

        set_import_blocked_status(&mut tracked, None);

        assert_eq!(
            tracked.status_messages,
            vec![
                "Automatic import could not determine a unique season and episode mapping. Open Manual Import and assign the correct season and episode."
                    .to_string()
            ]
        );
    }

    #[tokio::test]
    async fn track_reresolves_when_scryer_metadata_arrives_on_later_snapshot() {
        let title = build_title("House of Ravens", MediaFacet::Anime, &[]);
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut initial = build_client_item();
        initial.client_type = "weaver".to_string();
        initial.client_name = "weaver".to_string();
        initial.download_client_item_id = "job-2".to_string();
        initial.title_id = None;
        initial.facet = Some("anime".to_string());
        initial.title_name = "RAVENCOURT".to_string();
        initial.is_scryer_origin = false;

        tracker.track(&app, initial).await;
        let tracked = tracker.find("client-1:job-2").expect("tracked download");
        assert_eq!(tracked.match_type, TitleMatchType::Unmatched);
        assert!(tracked.title_id.is_none());

        let mut updated = build_client_item();
        updated.client_type = "weaver".to_string();
        updated.client_name = "weaver".to_string();
        updated.download_client_item_id = "job-2".to_string();
        updated.title_id = Some(title.id.clone());
        updated.facet = Some("anime".to_string());
        updated.title_name = "RAVENCOURT".to_string();
        updated.is_scryer_origin = true;

        tracker.track(&app, updated).await;

        let tracked = tracker.find("client-1:job-2").expect("tracked download");
        assert_eq!(tracked.match_type, TitleMatchType::ClientParameter);
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert!(tracked.client_item.is_scryer_origin);
    }

    #[tokio::test]
    async fn track_reresolves_when_facet_hint_arrives_on_later_snapshot() {
        let anime_title = build_title("Tidal Quest", MediaFacet::Anime, &[]);
        let series_title = build_title("Tidal Quest", MediaFacet::Series, &[]);
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![anime_title.clone(), series_title],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut initial = build_client_item();
        initial.client_type = "weaver".to_string();
        initial.client_name = "weaver".to_string();
        initial.download_client_item_id = "job-facet-reresolve".to_string();
        initial.title_name = "Tidal.Quest.S01E01.1080p.WEB-DL".to_string();
        initial.facet = None;
        initial.is_scryer_origin = false;

        tracker.track(&app, initial).await;

        let tracked = tracker
            .find("client-1:job-facet-reresolve")
            .expect("tracked download");
        assert_eq!(tracked.match_type, TitleMatchType::Unmatched);
        assert!(tracked.title_id.is_none());

        let mut updated = build_client_item();
        updated.client_type = "weaver".to_string();
        updated.client_name = "weaver".to_string();
        updated.download_client_item_id = "job-facet-reresolve".to_string();
        updated.title_name = "Tidal.Quest.S01E01.1080p.WEB-DL".to_string();
        updated.facet = Some("anime".to_string());
        updated.is_scryer_origin = false;

        tracker.track(&app, updated).await;

        let tracked = tracker
            .find("client-1:job-facet-reresolve")
            .expect("tracked download");
        assert_eq!(tracked.match_type, TitleMatchType::TitleParse);
        assert_eq!(tracked.title_id.as_deref(), Some(anime_title.id.as_str()));
    }

    #[test]
    fn update_trackable_preserves_operator_actionable_sources_missing_from_client_snapshot() {
        let mut tracker = TrackedDownloadService::new();

        for (suffix, state) in [
            ("pending", TrackedDownloadState::ImportPending),
            ("importing", TrackedDownloadState::Importing),
            ("blocked", TrackedDownloadState::ImportBlocked),
            ("failed", TrackedDownloadState::FailedPending),
        ] {
            let tracked = TrackedDownload {
                download_id: DownloadId::new(),
                id: format!("client-1:{suffix}"),
                client_id: "client-1".to_string(),
                client_type: "nzbget".to_string(),
                client_item: DownloadQueueItem {
                    download_client_item_id: suffix.to_string(),
                    ..build_client_item()
                },
                completed_source: None,
                state,
                status: TrackedDownloadStatus::Ok,
                status_messages: Vec::new(),
                title_id: None,
                facet: Some("series".to_string()),
                source_title: None,
                indexer: None,
                added_at: None,
                notified_manual_interaction: false,
                match_type: TitleMatchType::Unmatched,
                is_trackable: true,
                import_attempted: false,
                waiting_for_completed_history: false,
                path_missing_since: None,
                no_video_import_retry: None,
                import_execution_retry: None,
                import_hold: None,
                skip_reacquire_on_failure: false,
                burned_by_import_gate: false,
                snapshot_missing_since: None,
            };
            tracker.cache.insert(tracked.download_id, tracked);
        }

        tracker
            .find_mut("client-1:blocked")
            .expect("blocked tracked download")
            .completed_source = Some(build_completed_download(
            "nzbget",
            "blocked",
            "Blocked.Release",
            "/downloads/blocked",
            Some("series"),
        ));
        let unavailable_sources = tracker.update_trackable(&HashSet::new());

        for id in [
            "client-1:pending",
            "client-1:importing",
            "client-1:blocked",
            "client-1:failed",
        ] {
            assert!(tracker.find(id).is_some_and(|td| td.is_trackable));
        }
        assert!(unavailable_sources.is_empty());
        let blocked_identity = ClientJobLocator::new(Some("client-1"), "nzbget", "blocked");
        assert_eq!(
            tracker
                .completed_source_for_identity(&blocked_identity)
                .as_ref()
                .map(|completed| completed.dest_dir.as_str()),
            Some("/downloads/blocked")
        );
        tracker.stop_tracking("client-1:blocked");
        assert!(
            tracker
                .completed_source_for_identity(&blocked_identity)
                .is_none(),
            "terminal cleanup must dispose of the retained source"
        );
    }

    #[test]
    fn cached_id_for_source_identity_keeps_client_type_distinct() {
        let mut tracker = TrackedDownloadService::new();
        let mut nzbget = build_tracked_download("nzbget-entry");
        nzbget.client_item.download_client_item_id = "same-item".to_string();
        let mut qbittorrent = build_tracked_download("qbittorrent-entry");
        qbittorrent.client_type = "qbittorrent".to_string();
        qbittorrent.client_item.client_type = "qbittorrent".to_string();
        qbittorrent.client_item.download_client_item_id = "same-item".to_string();

        tracker.insert_for_tests(nzbget);
        tracker.insert_for_tests(qbittorrent);

        let qbittorrent_identity =
            ClientJobLocator::new(Some("client-1"), "qbittorrent", "same-item");
        assert_eq!(
            tracker.cached_id_for_source_identity(&qbittorrent_identity),
            Some("qbittorrent-entry".to_string())
        );
    }

    #[test]
    fn scoped_snapshot_pruning_reports_only_that_scope_after_grace() {
        let mut tracker = TrackedDownloadService::new();
        let queue_resident = build_tracked_download("queue-resident");
        let queue_resident_id = queue_resident.id.clone();
        tracker
            .cache
            .insert(queue_resident.download_id, queue_resident);
        let mut outside_scope = build_tracked_download("outside-scope");
        outside_scope.client_id = "client-2".to_string();
        outside_scope.client_item.client_id = "client-2".to_string();
        let outside_scope_id = outside_scope.id.clone();
        tracker
            .cache
            .insert(outside_scope.download_id, outside_scope);
        let scope = TrackedDownloadSnapshotScope::AuthoritativeForClient {
            client_id: Some("client-1".to_string()),
            client_type: "nzbget".to_string(),
        };

        let unavailable_sources = tracker.update_trackable_for_scope(&HashSet::new(), &scope);
        expire_snapshot_absence(&mut tracker, &queue_resident_id);
        let unavailable_sources_after_grace =
            tracker.update_trackable_for_scope(&HashSet::new(), &scope);

        assert!(
            tracker
                .find(&queue_resident_id)
                .is_some_and(|td| !td.is_trackable)
        );
        assert!(
            tracker
                .find(&outside_scope_id)
                .is_some_and(|td| td.is_trackable)
        );
        assert!(unavailable_sources.is_empty());
        assert_eq!(
            unavailable_sources_after_grace,
            vec![ClientJobLocator::new(
                Some("client-1"),
                "nzbget",
                "queue-resident"
            )]
        );
    }

    #[test]
    fn global_snapshot_pruning_reports_queue_resident_state_after_grace() {
        let mut tracker = TrackedDownloadService::new();
        let queue_resident = build_tracked_download("queue-resident");
        let queue_resident_id = queue_resident.id.clone();
        tracker
            .cache
            .insert(queue_resident.download_id, queue_resident);

        tracker.update_trackable_excluding_client_types(&HashSet::new(), &[]);
        expire_snapshot_absence(&mut tracker, &queue_resident_id);
        let unavailable_sources =
            tracker.update_trackable_excluding_client_types(&HashSet::new(), &[]);

        assert!(
            tracker
                .find(&queue_resident_id)
                .is_some_and(|td| !td.is_trackable)
        );
        assert_eq!(
            unavailable_sources,
            vec![ClientJobLocator::new(
                Some("client-1"),
                "nzbget",
                "queue-resident"
            )]
        );
    }

    #[test]
    fn authoritative_snapshot_preserves_post_queue_states_after_grace() {
        let mut tracker = TrackedDownloadService::new();
        let states = [
            ("pending", TrackedDownloadState::ImportPending),
            ("importing", TrackedDownloadState::Importing),
            ("blocked", TrackedDownloadState::ImportBlocked),
            ("failed-pending", TrackedDownloadState::FailedPending),
        ];
        let mut tracked_ids = Vec::new();
        for (suffix, state) in states {
            let mut tracked = build_tracked_download(suffix);
            tracked.state = state;
            tracked_ids.push(tracked.id.clone());
            tracker.cache.insert(tracked.download_id, tracked);
        }
        for id in &tracked_ids {
            expire_snapshot_absence(&mut tracker, id);
        }

        let authoritative_client_ids = HashSet::from(["client-1".to_string()]);
        let unavailable_sources = tracker
            .update_trackable_excluding_client_types_for_authoritative_clients(
                &HashSet::new(),
                &[],
                Some(&authoritative_client_ids),
            );

        assert!(unavailable_sources.is_empty());
        for id in tracked_ids {
            assert!(tracker.find(&id).is_some_and(|td| td.is_trackable));
            assert!(
                tracker
                    .find(&id)
                    .is_some_and(|td| td.snapshot_missing_since.is_none())
            );
        }
    }

    #[test]
    fn partial_snapshot_pruning_only_sweeps_authoritative_clients() {
        let mut tracker = TrackedDownloadService::new();
        let healthy = build_tracked_download("healthy-absent");
        let healthy_id = healthy.id.clone();
        tracker.cache.insert(healthy.download_id, healthy);
        let mut unavailable = build_tracked_download("unavailable-client");
        unavailable.client_id = "client-2".to_string();
        unavailable.client_item.client_id = "client-2".to_string();
        let unavailable_id = unavailable.id.clone();
        tracker.cache.insert(unavailable.download_id, unavailable);

        expire_snapshot_absence(&mut tracker, &healthy_id);
        expire_snapshot_absence(&mut tracker, &unavailable_id);
        let authoritative_client_ids = HashSet::from(["client-1".to_string()]);
        let unavailable_sources = tracker
            .update_trackable_excluding_client_types_for_authoritative_clients(
                &HashSet::new(),
                &[],
                Some(&authoritative_client_ids),
            );

        assert_eq!(
            unavailable_sources,
            vec![ClientJobLocator::new(
                Some("client-1"),
                "nzbget",
                "healthy-absent"
            )]
        );
        assert!(tracker.find(&healthy_id).is_some_and(|td| !td.is_trackable));
        assert!(
            tracker
                .find(&unavailable_id)
                .is_some_and(|td| td.is_trackable)
        );
        assert!(
            tracker
                .find(&unavailable_id)
                .is_some_and(|td| td.snapshot_missing_since.is_none())
        );
    }

    #[test]
    fn tracked_client_type_is_excluded_trims_and_ignores_case() {
        assert!(tracked_client_type_is_excluded("weaver", &[" Weaver "]));
        assert!(tracked_client_type_is_excluded(" WEAVER ", &["weaver"]));
        assert!(!tracked_client_type_is_excluded("nzbget", &[" weaver "]));
    }

    #[test]
    fn update_trackable_excluding_client_types_preserves_excluded_realtime_clients() {
        let mut tracker = TrackedDownloadService::new();
        let mut weaver = build_tracked_download("weaver-active");
        weaver.client_type = "weaver".to_string();
        weaver.client_item.client_type = "weaver".to_string();
        let weaver_id = weaver.id.clone();
        let nzb = build_tracked_download("nzbget-active");
        let nzb_id = nzb.id.clone();
        tracker.cache.insert(weaver.download_id, weaver);
        tracker.cache.insert(nzb.download_id, nzb);

        // First absence only STAMPS (absence debounce); prune happens once the
        // absence outlives the grace window.
        tracker.update_trackable_excluding_client_types(&HashSet::new(), &["weaver"]);
        assert!(tracker.find(&weaver_id).is_some_and(|td| td.is_trackable));
        assert!(tracker.find(&nzb_id).is_some_and(|td| td.is_trackable));

        expire_snapshot_absence(&mut tracker, &nzb_id);
        tracker.update_trackable_excluding_client_types(&HashSet::new(), &["weaver"]);
        assert!(tracker.find(&weaver_id).is_some_and(|td| td.is_trackable));
        if let Some(td) = tracker.find(&nzb_id) {
            assert!(!td.is_trackable);
        }

        tracker.update_trackable_excluding_client_types(&HashSet::new(), &[]);
        expire_snapshot_absence(&mut tracker, &weaver_id);
        tracker.update_trackable_excluding_client_types(&HashSet::new(), &[]);
        if let Some(td) = tracker.find(&weaver_id) {
            assert!(!td.is_trackable);
        }
    }

    /// Backdate a tracked download's absence stamp past the prune grace window.
    fn expire_snapshot_absence(tracker: &mut TrackedDownloadService, id: &str) {
        if let Some(td) = tracker.find_mut(id) {
            td.snapshot_missing_since = Some(
                Utc::now()
                    - chrono::Duration::seconds(
                        TrackedDownloadService::SNAPSHOT_ABSENCE_PRUNE_GRACE_SECS + 1,
                    ),
            );
        }
    }

    #[test]
    fn snapshot_absence_debounce_survives_transient_client_blackouts() {
        // The router degrades per client: a feedback timeout starts an
        // exponential backoff during which that client's reads are silently
        // skipped, so its items are absent from otherwise-successful
        // snapshots. Pruning on first absence erased live downloads during
        // such blackouts — anything that completed inside one was never
        // imported. Absence must persist beyond the grace window to prune,
        // and one sighting must fully reset the clock.
        let mut tracker = TrackedDownloadService::new();
        let td = build_tracked_download("blip-item");
        let id = td.id.clone();
        tracker.cache.insert(td.download_id, td);

        // Several consecutive absent snapshots inside the grace window: the
        // item survives every one of them.
        for _ in 0..3 {
            tracker.update_trackable_excluding_client_types(&HashSet::new(), &[]);
            assert!(tracker.find(&id).is_some_and(|t| t.is_trackable));
        }
        assert!(
            tracker
                .find(&id)
                .is_some_and(|t| t.snapshot_missing_since.is_some()),
            "absence must be stamped"
        );

        // A sighting clears the stamp entirely.
        let mut seen = HashSet::new();
        seen.insert(id.clone());
        tracker.update_trackable_excluding_client_types(&seen, &[]);
        assert!(
            tracker
                .find(&id)
                .is_some_and(|t| t.snapshot_missing_since.is_none()),
            "a sighting must reset the absence clock"
        );

        // Absence that outlives the grace window prunes.
        tracker.update_trackable_excluding_client_types(&HashSet::new(), &[]);
        expire_snapshot_absence(&mut tracker, &id);
        tracker.update_trackable_excluding_client_types(&HashSet::new(), &[]);
        if let Some(t) = tracker.find(&id) {
            assert!(!t.is_trackable);
        }
    }

    #[test]
    fn update_trackable_excluding_client_types_trims_excluded_client_type() {
        let mut tracker = TrackedDownloadService::new();
        let mut weaver = build_tracked_download("weaver-active");
        weaver.client_type = " Weaver ".to_string();
        weaver.client_item.client_type = " Weaver ".to_string();
        let weaver_id = weaver.id.clone();
        let nzb = build_tracked_download("nzbget-active");
        let nzb_id = nzb.id.clone();
        tracker.cache.insert(weaver.download_id, weaver);
        tracker.cache.insert(nzb.download_id, nzb);

        // First absence stamps; expiry past the grace window prunes. The
        // trimmed exclusion must hold across both passes.
        tracker.update_trackable_excluding_client_types(&HashSet::new(), &[" weaver "]);
        expire_snapshot_absence(&mut tracker, &nzb_id);
        tracker.update_trackable_excluding_client_types(&HashSet::new(), &[" weaver "]);

        assert!(tracker.find(&weaver_id).is_some_and(|td| td.is_trackable));
        if let Some(td) = tracker.find(&nzb_id) {
            assert!(!td.is_trackable);
        }
    }

    #[test]
    fn prune_cache_evicts_oldest_low_value_unmatched_download_under_pressure() {
        let mut tracker = TrackedDownloadService::new();
        let old = build_tracked_download("old-unmatched");
        let recent = build_tracked_download("recent-unmatched");
        let old_download_id = old.download_id;
        let recent_download_id = recent.download_id;
        tracker.cache.insert(old_download_id, old);
        tracker.cache.insert(recent_download_id, recent);
        let now = Utc::now();
        tracker
            .last_seen_at
            .insert(old_download_id, now - chrono::Duration::minutes(10));
        tracker.last_seen_at.insert(recent_download_id, now);

        tracker.prune_cache_with_limits(now - chrono::Duration::hours(1), 1);

        assert!(tracker.find("old-unmatched").is_none());
        assert!(tracker.find("recent-unmatched").is_some());
    }

    #[test]
    fn prune_cache_keeps_actionable_entries_even_when_over_limit() {
        let mut tracker = TrackedDownloadService::new();
        let mut actionable = build_tracked_download("actionable");
        actionable.state = TrackedDownloadState::ImportPending;
        actionable.title_id = Some("title-1".to_string());
        actionable.facet = Some("series".to_string());
        let mut failed = build_tracked_download("failed-actionable");
        failed.state = TrackedDownloadState::FailedPending;
        failed.status = TrackedDownloadStatus::Error;
        failed.status_messages = vec!["failure needs processing".to_string()];
        let low_value = build_tracked_download("low-value");
        let actionable_download_id = actionable.download_id;
        let failed_download_id = failed.download_id;
        let low_value_download_id = low_value.download_id;
        tracker.cache.insert(actionable_download_id, actionable);
        tracker.cache.insert(failed_download_id, failed);
        tracker.cache.insert(low_value_download_id, low_value);
        let now = Utc::now();
        tracker
            .last_seen_at
            .insert(actionable_download_id, now - chrono::Duration::minutes(10));
        tracker
            .last_seen_at
            .insert(failed_download_id, now - chrono::Duration::minutes(5));
        tracker.last_seen_at.insert(low_value_download_id, now);

        tracker.prune_cache_with_limits(now - chrono::Duration::hours(1), 1);

        assert!(tracker.find("actionable").is_some());
        assert!(tracker.find("failed-actionable").is_some());
        assert!(tracker.find("low-value").is_none());
        assert_eq!(tracker.cache.len(), 2);
    }

    #[test]
    fn failed_download_check_preempts_import_pending_state() {
        let mut client_item = build_client_item();
        client_item.state = DownloadQueueState::Failed;
        client_item.attention_reason = Some("health below critical".to_string());
        let mut tracked = TrackedDownload {
            download_id: DownloadId::new(),
            id: "client-1:failed-import-pending".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item,
            completed_source: None,
            state: TrackedDownloadState::ImportPending,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_execution_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
            burned_by_import_gate: false,
            snapshot_missing_since: None,
        };

        crate::failed_download_handler::check(&mut tracked);

        assert_eq!(tracked.state, TrackedDownloadState::FailedPending);
        assert_eq!(tracked.status, TrackedDownloadStatus::Error);
    }

    #[test]
    fn warning_client_state_never_enters_failed_download_handling() {
        // A recoverable client condition (disk full, files moved, tracker
        // error) must keep the download where it is: no FailedPending, so no
        // blocklist, no removal and no re-search.
        for state in [
            TrackedDownloadState::Downloading,
            TrackedDownloadState::ImportPending,
            TrackedDownloadState::ImportBlocked,
        ] {
            let mut client_item = build_client_item();
            client_item.state = DownloadQueueState::Warning;
            client_item.attention_required = true;
            client_item.attention_reason = Some("files are missing".to_string());
            let mut tracked = TrackedDownload {
                download_id: DownloadId::new(),
                id: "client-1:warned".to_string(),
                client_id: "client-1".to_string(),
                client_type: "qbittorrent".to_string(),
                client_item,
                completed_source: None,
                state,
                status: TrackedDownloadStatus::Ok,
                status_messages: Vec::new(),
                title_id: Some("title-1".to_string()),
                facet: Some("movie".to_string()),
                source_title: None,
                indexer: None,
                added_at: None,
                notified_manual_interaction: false,
                match_type: TitleMatchType::Submission,
                is_trackable: true,
                import_attempted: false,
                waiting_for_completed_history: false,
                path_missing_since: None,
                no_video_import_retry: None,
                import_execution_retry: None,
                import_hold: None,
                skip_reacquire_on_failure: false,
                burned_by_import_gate: false,
                snapshot_missing_since: None,
            };

            crate::failed_download_handler::check(&mut tracked);

            assert_eq!(tracked.state, state, "{state:?} must survive a warning");
            assert_eq!(tracked.status, TrackedDownloadStatus::Ok);
            assert_eq!(
                tracked.client_item.attention_reason.as_deref(),
                Some("files are missing")
            );
        }
    }

    #[test]
    fn persistent_warning_becomes_failed_pending_after_the_timeout() {
        let mut tracker = TrackedDownloadService::new();
        let mut tracked = build_tracked_download("warned-timeout");
        tracked.client_item.state = DownloadQueueState::Warning;
        tracked.client_item.attention_reason = Some("files are missing".to_string());
        let id = tracked.id.clone();
        let download_id = tracker.insert_for_tests(tracked);
        let now = Utc::now();
        tracker.warning_since.insert(
            download_id,
            now - TrackedDownloadService::WARNING_FAILURE_TIMEOUT,
        );

        assert!(tracker.fail_persistent_warning(&id, now, true));
        let tracked = tracker.find(&id).expect("tracked download");
        assert_eq!(tracked.state, TrackedDownloadState::FailedPending);
        assert_eq!(tracked.status, TrackedDownloadStatus::Error);
        assert_eq!(
            tracked.status_messages,
            vec!["download client warning persisted for 24h: files are missing"]
        );
    }

    #[test]
    fn foreign_origin_warning_is_never_timed_out() {
        let mut tracker = TrackedDownloadService::new();
        let mut tracked = build_tracked_download("foreign-warning");
        tracked.client_item.is_scryer_origin = false;
        tracked.client_item.state = DownloadQueueState::Warning;
        let id = tracked.id.clone();
        let download_id = tracker.insert_for_tests(tracked);
        let now = Utc::now();
        tracker.warning_since.insert(
            download_id,
            now - TrackedDownloadService::WARNING_FAILURE_TIMEOUT,
        );

        assert!(!tracker.fail_persistent_warning(&id, now, true));
        let tracked = tracker.find(&id).expect("tracked download");
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
        assert!(tracked.status_messages.is_empty());
        assert!(!tracker.warning_since.contains_key(&download_id));
    }

    #[test]
    fn timed_out_warning_clears_its_clock_before_a_new_window_starts() {
        let mut tracker = TrackedDownloadService::new();
        let mut tracked = build_tracked_download("warning-clears-clock");
        tracked.client_item.state = DownloadQueueState::Warning;
        let id = tracked.id.clone();
        let download_id = tracker.insert_for_tests(tracked);
        let now = Utc::now();
        tracker.warning_since.insert(
            download_id,
            now - TrackedDownloadService::WARNING_FAILURE_TIMEOUT,
        );

        assert!(tracker.fail_persistent_warning(&id, now, true));
        assert!(!tracker.warning_since.contains_key(&download_id));
        assert!(!tracker.fail_persistent_warning(&id, now, true));
        assert_eq!(
            tracker
                .find(&id)
                .expect("tracked download")
                .status_messages
                .len(),
            1
        );
    }

    #[test]
    fn younger_warning_is_not_timed_out() {
        let mut tracker = TrackedDownloadService::new();
        let mut tracked = build_tracked_download("warned-young");
        tracked.client_item.state = DownloadQueueState::Warning;
        let id = tracked.id.clone();
        let download_id = tracker.insert_for_tests(tracked);
        let now = Utc::now();
        tracker.warning_since.insert(
            download_id,
            now - TrackedDownloadService::WARNING_FAILURE_TIMEOUT + chrono::Duration::seconds(1),
        );

        assert!(!tracker.fail_persistent_warning(&id, now, true));
        assert_eq!(
            tracker.find(&id).expect("tracked download").state,
            TrackedDownloadState::Downloading
        );
    }

    #[test]
    fn leaving_warning_clears_its_timeout_clock() {
        let mut tracker = TrackedDownloadService::new();
        let mut tracked = build_tracked_download("warning-cleared");
        tracked.client_item.state = DownloadQueueState::Warning;
        let id = tracked.id.clone();
        let download_id = tracker.insert_for_tests(tracked);
        let now = Utc::now();
        tracker.warning_since.insert(download_id, now);
        tracker
            .find_mut(&id)
            .expect("tracked download")
            .client_item
            .state = DownloadQueueState::Downloading;

        assert!(!tracker.fail_persistent_warning(&id, now, true));
        assert!(!tracker.warning_since.contains_key(&download_id));
    }

    #[test]
    fn imported_rows_are_never_timed_out_for_a_client_warning() {
        for state in [
            TrackedDownloadState::Imported,
            TrackedDownloadState::ImportedSeeding,
        ] {
            let mut tracker = TrackedDownloadService::new();
            let mut tracked = build_tracked_download("imported-warning");
            tracked.state = state;
            tracked.client_item.state = DownloadQueueState::Warning;
            let id = tracked.id.clone();
            let download_id = tracker.insert_for_tests(tracked);
            let now = Utc::now();
            tracker.warning_since.insert(
                download_id,
                now - TrackedDownloadService::WARNING_FAILURE_TIMEOUT,
            );

            assert!(!tracker.fail_persistent_warning(&id, now, true));
            assert_eq!(tracker.find(&id).expect("tracked download").state, state);
            assert!(!tracker.warning_since.contains_key(&download_id));
        }
    }

    #[test]
    fn a_warned_download_under_a_seeding_profile_is_never_timed_out() {
        let mut tracker = TrackedDownloadService::new();
        let mut tracked = build_tracked_download("warned-seeding-profile");
        tracked.client_item.state = DownloadQueueState::Warning;
        tracked.client_item.attention_reason = Some("stalled".to_string());
        let id = tracked.id.clone();
        let download_id = tracker.insert_for_tests(tracked);
        let now = Utc::now();
        tracker.warning_since.insert(
            download_id,
            now - TrackedDownloadService::WARNING_FAILURE_TIMEOUT,
        );

        // The caller decided the timeout does not apply (a seeding profile owns
        // this torrent): nothing fails, the clock is dropped, the warning stays.
        assert!(!tracker.fail_persistent_warning(&id, now, false));
        let tracked = tracker.find(&id).expect("tracked download");
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
        assert!(tracked.status_messages.is_empty());
        assert!(!tracker.warning_since.contains_key(&download_id));
    }

    #[test]
    fn failed_download_check_skips_parse_matched_downloader_observation() {
        let mut client_item = build_client_item();
        client_item.state = DownloadQueueState::Failed;
        client_item.attention_reason = Some("health below critical".to_string());
        client_item.is_scryer_origin = false;
        let mut tracked = TrackedDownload {
            download_id: DownloadId::new(),
            id: "client-1:failed-observation".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item,
            completed_source: None,
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: Some("Observed.Show.S01E01.1080p.WEB-DL".to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::TitleParse,
            is_trackable: true,
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_execution_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
            burned_by_import_gate: false,
            snapshot_missing_since: None,
        };

        crate::failed_download_handler::check(&mut tracked);

        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
        assert_eq!(tracked.status, TrackedDownloadStatus::Warning);
        assert!(
            tracked
                .status_messages
                .iter()
                .any(|message| message.contains("wasn't grabbed by Scryer"))
        );
    }

    #[tokio::test]
    async fn failed_source_invalidates_active_manual_import_request() {
        let payload = crate::ManualImportRequestPayload {
            requested_by_user_id: Some("user-1".to_string()),
            title_id: Some("title-1".to_string()),
            download_client_item_id: "job-active-manual".to_string(),
            client_id: Some("client-1".to_string()),
            client_type: "weaver".to_string(),
            files: Vec::new(),
            selection_id: None,
            release_evidence: None,
            trusted_source_root: None,
            archive_workspace_root: None,
            requested_at: Utc::now().to_rfc3339(),
        };
        let imports = Arc::new(TestImportRepo {
            import_record: Some(ImportRecord {
                id: "import-1".to_string(),
                source_client_id: Some("client-1".to_string()),
                source_system: "weaver".to_string(),
                source_ref: "job-active-manual".to_string(),
                import_type: ImportType::ManualImport,
                status: ImportStatus::Pending,
                payload_json: serde_json::to_string(&payload).expect("serialize payload"),
                result_json: None,
                download_id: None,
                import_transfer_phase: None,
                import_transfer_bytes: None,
                import_transfer_total_bytes: None,
                import_transfer_started_at: None,
                import_transfer_updated_at: None,
                started_at: None,
                finished_at: None,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            }),
            ..Default::default()
        });
        let app = build_app(
            Arc::new(TestDownloadSubmissionRepo::default()),
            imports.clone(),
        );
        let mut client_item = build_client_item();
        client_item.client_type = "weaver".to_string();
        client_item.download_client_item_id = "job-active-manual".to_string();
        let tracked = TrackedDownload {
            download_id: DownloadId::new(),
            id: "client-1:job-active-manual".to_string(),
            client_id: "client-1".to_string(),
            client_type: "weaver".to_string(),
            client_item,
            completed_source: None,
            state: TrackedDownloadState::FailedPending,
            status: TrackedDownloadStatus::Error,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_execution_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
            burned_by_import_gate: false,
            snapshot_missing_since: None,
        };

        crate::fail_active_manual_import_for_source(&app, &tracked, "health below critical").await;

        let updates = imports.status_updates.lock().await;
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, ImportStatus::Failed);
        assert!(
            updates[0]
                .1
                .as_deref()
                .is_some_and(|json| json.contains("source_job_failed"))
        );
    }

    #[tokio::test]
    async fn active_manual_import_lookup_matches_the_exact_client() {
        let payload_other = crate::ManualImportRequestPayload {
            requested_by_user_id: Some("user-1".to_string()),
            title_id: Some("title-1".to_string()),
            download_client_item_id: "job-shared".to_string(),
            client_id: Some("client-2".to_string()),
            client_type: "weaver".to_string(),
            files: Vec::new(),
            selection_id: None,
            release_evidence: None,
            trusted_source_root: None,
            archive_workspace_root: None,
            requested_at: Utc::now().to_rfc3339(),
        };
        let payload_match = crate::ManualImportRequestPayload {
            client_id: Some("client-1".to_string()),
            ..payload_other.clone()
        };
        let imports = Arc::new(TestImportRepo {
            import_records: vec![
                ImportRecord {
                    id: "import-other".to_string(),
                    source_client_id: Some("client-2".to_string()),
                    source_system: "weaver".to_string(),
                    source_ref: "job-shared".to_string(),
                    import_type: ImportType::ManualImport,
                    status: ImportStatus::Pending,
                    payload_json: serde_json::to_string(&payload_other)
                        .expect("serialize other payload"),
                    result_json: None,
                    download_id: None,
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    started_at: None,
                    finished_at: None,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: Utc::now().to_rfc3339(),
                },
                ImportRecord {
                    id: "import-match".to_string(),
                    source_client_id: Some("client-1".to_string()),
                    source_system: "weaver".to_string(),
                    source_ref: "job-shared".to_string(),
                    import_type: ImportType::ManualImport,
                    status: ImportStatus::Pending,
                    payload_json: serde_json::to_string(&payload_match)
                        .expect("serialize matching payload"),
                    result_json: None,
                    download_id: None,
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    started_at: None,
                    finished_at: None,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339(),
                },
            ],
            ..Default::default()
        });
        let download_client = Arc::new(TestDownloadClient {
            recent_activity: Arc::new(Mutex::new(vec![DownloadQueueItem {
                client_id: "client-1".to_string(),
                client_name: "weaver".to_string(),
                client_type: "weaver".to_string(),
                download_client_item_id: "job-shared".to_string(),
                state: DownloadQueueState::Completed,
                ..build_client_item()
            }])),
            ..Default::default()
        });
        let app = build_app_with_title_repo_and_download_client(
            Arc::new(NullTitleRepository),
            download_client,
            Arc::new(TestDownloadSubmissionRepo::default()),
            imports,
        );

        let import_id = crate::import_workflow::find_active_manual_import_for_source(
            &app,
            Some("client-1"),
            "weaver",
            "job-shared",
        )
        .await
        .expect("lookup should succeed")
        .expect("matching request should be found")
        .id;

        assert_eq!(import_id, "import-match");
    }

    #[tokio::test]
    async fn failed_source_invalidates_only_matching_client_request() {
        let payload_other = crate::ManualImportRequestPayload {
            requested_by_user_id: Some("user-1".to_string()),
            title_id: Some("title-1".to_string()),
            download_client_item_id: "job-shared".to_string(),
            client_id: Some("client-2".to_string()),
            client_type: "weaver".to_string(),
            files: Vec::new(),
            selection_id: None,
            release_evidence: None,
            trusted_source_root: None,
            archive_workspace_root: None,
            requested_at: Utc::now().to_rfc3339(),
        };
        let payload_match = crate::ManualImportRequestPayload {
            client_id: Some("client-1".to_string()),
            ..payload_other.clone()
        };
        let imports = Arc::new(TestImportRepo {
            import_records: vec![
                ImportRecord {
                    id: "import-other".to_string(),
                    source_client_id: Some("client-2".to_string()),
                    source_system: "weaver".to_string(),
                    source_ref: "job-shared".to_string(),
                    import_type: ImportType::ManualImport,
                    status: ImportStatus::Pending,
                    payload_json: serde_json::to_string(&payload_other)
                        .expect("serialize other payload"),
                    result_json: None,
                    download_id: None,
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    started_at: None,
                    finished_at: None,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: Utc::now().to_rfc3339(),
                },
                ImportRecord {
                    id: "import-match".to_string(),
                    source_client_id: Some("client-1".to_string()),
                    source_system: "weaver".to_string(),
                    source_ref: "job-shared".to_string(),
                    import_type: ImportType::ManualImport,
                    status: ImportStatus::Pending,
                    payload_json: serde_json::to_string(&payload_match)
                        .expect("serialize matching payload"),
                    result_json: None,
                    download_id: None,
                    import_transfer_phase: None,
                    import_transfer_bytes: None,
                    import_transfer_total_bytes: None,
                    import_transfer_started_at: None,
                    import_transfer_updated_at: None,
                    started_at: None,
                    finished_at: None,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339(),
                },
            ],
            ..Default::default()
        });
        let app = build_app(
            Arc::new(TestDownloadSubmissionRepo::default()),
            imports.clone(),
        );
        let tracked = TrackedDownload {
            download_id: DownloadId::new(),
            id: "client-1:job-shared".to_string(),
            client_id: "client-1".to_string(),
            client_type: "weaver".to_string(),
            client_item: DownloadQueueItem {
                client_id: "client-1".to_string(),
                client_name: "weaver".to_string(),
                client_type: "weaver".to_string(),
                download_client_item_id: "job-shared".to_string(),
                ..build_client_item()
            },
            completed_source: None,
            state: TrackedDownloadState::FailedPending,
            status: TrackedDownloadStatus::Error,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_execution_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
            burned_by_import_gate: false,
            snapshot_missing_since: None,
        };

        crate::fail_active_manual_import_for_source(&app, &tracked, "health below critical").await;

        let updates = imports.status_updates.lock().await;
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, ImportStatus::Failed);
        assert!(
            updates[0]
                .1
                .as_deref()
                .is_some_and(|json| json.contains("\"import_id\":\"import-match\""))
        );
    }

    #[test]
    fn manual_import_recovery_marks_only_client_complete_downloads_awaiting_import() {
        for state in [
            TrackedDownloadState::ImportPending,
            TrackedDownloadState::Importing,
            TrackedDownloadState::ImportBlocked,
        ] {
            let mut tracked = build_tracked_download("awaiting-import");
            tracked.state = state;
            tracked.completed_source = Some(build_completed_download(
                "nzbget",
                "awaiting-import",
                "Release",
                "/downloads/release",
                Some("movies"),
            ));
            assert_eq!(
                manual_import_recovery_verdict(&tracked, Utc::now()),
                ManualImportRecoveryVerdict::MarkImported,
                "{state:?} with a completed source"
            );

            // Without the client's completion the download has not finished.
            tracked.completed_source = None;
            assert_eq!(
                manual_import_recovery_verdict(&tracked, Utc::now()),
                ManualImportRecoveryVerdict::Leave,
                "{state:?} without a completed source"
            );
        }
    }

    #[test]
    fn manual_import_recovery_leaves_fresh_downloads_that_reuse_an_item_id() {
        // A re-grab or cross-seed of the same release shares the old item id
        // (qBittorrent info-hash; NZBGet ids restart after a queue reset). A
        // completed manual-import record from the previous life of that id
        // must not terminalize the new download.
        for state in [
            TrackedDownloadState::Downloading,
            TrackedDownloadState::FailedPending,
        ] {
            let mut tracked = build_tracked_download("reused-item-id");
            tracked.state = state;
            assert_eq!(
                manual_import_recovery_verdict(&tracked, Utc::now()),
                ManualImportRecoveryVerdict::Leave,
                "{state:?}"
            );
            tracked.completed_source = Some(build_completed_download(
                "qbittorrent",
                "reused-item-id",
                "Release",
                "/downloads/release",
                Some("movies"),
            ));
            assert_eq!(
                manual_import_recovery_verdict(&tracked, Utc::now()),
                ManualImportRecoveryVerdict::Leave,
                "{state:?} even with a retained completed source"
            );
        }
    }

    #[test]
    fn manual_import_recovery_leaves_terminal_downloads_including_already_imported() {
        for state in [
            TrackedDownloadState::Imported,
            TrackedDownloadState::Failed,
            TrackedDownloadState::Ignored,
        ] {
            let mut tracked = build_tracked_download("terminal");
            tracked.state = state;
            tracked.completed_source = Some(build_completed_download(
                "nzbget",
                "terminal",
                "Release",
                "/downloads/release",
                Some("movies"),
            ));
            assert_eq!(
                manual_import_recovery_verdict(&tracked, Utc::now()),
                ManualImportRecoveryVerdict::Leave,
                "{state:?}"
            );
        }
    }

    fn awaiting_import_with_client_completion(
        completed_at: Option<chrono::DateTime<Utc>>,
    ) -> TrackedDownload {
        let mut tracked = build_tracked_download("same-info-hash");
        tracked.state = TrackedDownloadState::ImportBlocked;
        let mut completed = build_completed_download(
            "qbittorrent",
            "same-info-hash",
            "Release",
            "/downloads/release",
            Some("movies"),
        );
        completed.completed_at = completed_at;
        tracked.completed_source = Some(completed);
        tracked
    }

    #[test]
    fn manual_import_recovery_marks_a_download_the_client_finished_before_the_record() {
        let record_completed_at = Utc::now();
        let tracked =
            awaiting_import_with_client_completion(Some(record_completed_at - Duration::hours(1)));
        assert_eq!(
            manual_import_recovery_verdict(&tracked, record_completed_at),
            ManualImportRecoveryVerdict::MarkImported
        );

        // Completion at the very same instant still predates the record.
        let tracked = awaiting_import_with_client_completion(Some(record_completed_at));
        assert_eq!(
            manual_import_recovery_verdict(&tracked, record_completed_at),
            ManualImportRecoveryVerdict::MarkImported
        );
    }

    #[test]
    fn manual_import_recovery_leaves_a_same_id_download_that_finished_after_the_record() {
        // Import completed at 10:00; the user deleted and re-grabbed the same
        // release (same info-hash); the re-grab completed at 11:30 and sits in
        // ImportBlocked inside the recovery window. The old record must not
        // mark it Imported.
        let record_completed_at = Utc::now() - Duration::hours(2);
        let tracked = awaiting_import_with_client_completion(Some(
            record_completed_at + Duration::minutes(90),
        ));
        assert_eq!(
            manual_import_recovery_verdict(&tracked, record_completed_at),
            ManualImportRecoveryVerdict::Leave
        );
    }

    #[test]
    fn manual_import_recovery_without_a_client_completion_time_uses_the_state_rules_alone() {
        let record_completed_at = Utc::now() - Duration::hours(2);
        let tracked = awaiting_import_with_client_completion(None);
        assert_eq!(
            manual_import_recovery_verdict(&tracked, record_completed_at),
            ManualImportRecoveryVerdict::MarkImported
        );

        let mut downloading = awaiting_import_with_client_completion(None);
        downloading.state = TrackedDownloadState::Downloading;
        assert_eq!(
            manual_import_recovery_verdict(&downloading, record_completed_at),
            ManualImportRecoveryVerdict::Leave
        );
    }

    /// Track an item, drive it to `state` through the durable marker, then
    /// track the same item again on a *fresh* tracker — the restart.
    async fn terminal_state_round_trip(
        carries_legacy_wire_token: bool,
        state: TrackedDownloadState,
    ) -> (Arc<TestDownloadSubmissionRepo>, DownloadId, TrackedDownload) {
        let canonical_download_id = DownloadId::new();
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let registry = Arc::new(TestDownloadRegistry {
            ids: Mutex::new(HashMap::from([(
                "round-trip-job".to_string(),
                canonical_download_id,
            )])),
            failing_item_ids: HashSet::new(),
            conflicting_item_ids: HashMap::new(),
            fallback_download_ids: HashMap::new(),
        });
        let app = build_app(
            download_submissions.clone(),
            Arc::new(TestImportRepo::default()),
        )
        .with_test_overrides(|services| services.with_download_registry(registry));

        let mut item = build_client_item();
        item.download_client_item_id = "round-trip-job".to_string();
        item.download_id = carries_legacy_wire_token.then(|| canonical_download_id.to_wire());
        let tracked_id = tracked_download_id_for_item(&item);

        let mut tracker = TrackedDownloadService::new();
        tracker.track(&app, item.clone()).await;
        let tracked = tracker
            .find(&tracked_id)
            .expect("first tracking pass should cache the download")
            .clone();
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
        assert!(
            persist_tracked_download_state_marker(&app, &tracked, state, None, None).await,
            "durable marker should persist"
        );

        let mut restarted = TrackedDownloadService::new();
        restarted.track(&app, item).await;
        let recovered = restarted
            .find(&tracked_id)
            .expect("restart should re-track the download")
            .clone();
        (download_submissions, canonical_download_id, recovered)
    }

    #[tokio::test]
    async fn a_token_less_plugin_download_keeps_its_terminal_state_across_a_restart() {
        // Plugin download clients legally omit the legacy wire token. The
        // durable row is keyed by the canonical download id, so the terminal
        // outcome must still be written and read back; otherwise the item
        // re-enters processing on the first see after a restart.
        let (download_submissions, canonical_download_id, recovered) =
            terminal_state_round_trip(false, TrackedDownloadState::Imported).await;

        assert_eq!(
            download_submissions
                .canonical_identity_tracked_states
                .lock()
                .await
                .get(&canonical_download_id)
                .map(String::as_str),
            Some(TrackedDownloadState::Imported.as_str()),
            "a token-less item must still write the canonical durable row"
        );
        assert_eq!(recovered.download_id, canonical_download_id);
        assert_eq!(recovered.state, TrackedDownloadState::Imported);
    }

    #[tokio::test]
    async fn a_token_bearing_download_keeps_the_same_restart_behaviour() {
        let (download_submissions, canonical_download_id, recovered) =
            terminal_state_round_trip(true, TrackedDownloadState::Failed).await;

        assert_eq!(
            download_submissions
                .canonical_identity_tracked_states
                .lock()
                .await
                .get(&canonical_download_id)
                .map(String::as_str),
            Some(TrackedDownloadState::Failed.as_str())
        );
        assert_eq!(recovered.state, TrackedDownloadState::Failed);
    }

    /// The ignore counterpart of the terminal-state round trip.
    ///
    /// A plugin item carries no legacy wire token, and a submission bound by
    /// observation is only reachable through its canonical id — so the durable
    /// identity-state write must not be gated on the legacy tuple resolving a
    /// token. Without the durable row the item re-enters processing on the
    /// first see after a restart.
    #[tokio::test]
    async fn a_token_less_plugin_download_keeps_an_ignore_across_a_restart() {
        let canonical_download_id = DownloadId::new();
        let source_identity = ClientJobLocator::new(Some("client-1"), "nzbget", "ignored-job");
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            // Only the canonical lookup finds it; `get_submission_identity`
            // (the legacy tuple) returns nothing, exactly as it does for a
            // submission whose client item id was filled in by observation.
            canonical_submission: Some(crate::DownloadSubmission {
                download_id: canonical_download_id,
                title_id: "title-plugin".to_string(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "series".to_string(),
                download_client_id: Some("client-1".to_string()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "ignored-job".to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Plugin Ignored Release".to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: crate::SubmissionScope::Title,
            }),
            ..Default::default()
        });
        let registry = Arc::new(TestDownloadRegistry {
            ids: Mutex::new(HashMap::from([(
                "ignored-job".to_string(),
                canonical_download_id,
            )])),
            failing_item_ids: HashSet::new(),
            conflicting_item_ids: HashMap::new(),
            fallback_download_ids: HashMap::new(),
        });
        let app = build_app(
            download_submissions.clone(),
            Arc::new(TestImportRepo::default()),
        )
        .with_test_overrides(|services| services.with_download_registry(registry));

        assert!(matches!(
            crate::integration::workflow::finalize_scryer_download_ignored_for_download(
                &app,
                crate::domain_events::DomainEventActor::system(),
                Some(&canonical_download_id),
                source_identity.clone(),
            )
            .await
            .expect("a token-less ignore should finalize"),
            crate::integration::workflow::FinalizeIgnoredOutcome::Finalized
        ));
        assert_eq!(
            download_submissions
                .canonical_identity_tracked_states
                .lock()
                .await
                .get(&canonical_download_id)
                .map(String::as_str),
            Some(TrackedDownloadState::Ignored.as_str()),
            "the durable row must be written without a legacy wire token"
        );

        // Idempotent: re-ignoring reports the same outcome and leaves the
        // durable row on ignored.
        assert!(matches!(
            crate::integration::workflow::finalize_scryer_download_ignored_for_download(
                &app,
                crate::domain_events::DomainEventActor::system(),
                Some(&canonical_download_id),
                source_identity.clone(),
            )
            .await
            .expect("a repeated ignore should finalize"),
            crate::integration::workflow::FinalizeIgnoredOutcome::Finalized
        ));
        assert_eq!(
            download_submissions
                .canonical_identity_tracked_states
                .lock()
                .await
                .get(&canonical_download_id)
                .map(String::as_str),
            Some(TrackedDownloadState::Ignored.as_str())
        );

        // Restart: a fresh tracker re-tracks the same client item and must
        // reconstruct the ignore instead of resuming the download.
        let mut item = build_client_item();
        item.download_client_item_id = "ignored-job".to_string();
        item.download_id = None;
        let tracked_id = tracked_download_id_for_item(&item);
        let mut restarted = TrackedDownloadService::new();
        restarted.track(&app, item).await;
        let recovered = restarted
            .find(&tracked_id)
            .expect("restart should re-track the download");
        assert_eq!(recovered.download_id, canonical_download_id);
        assert_eq!(recovered.state, TrackedDownloadState::Ignored);
    }
}
