//! TrackedDownloads — scryer-side download lifecycle state machine.
//!
//! Maintains an in-memory cache of active downloads, each enriched with title
//! resolution metadata and driven through a workflow state machine independent
//! of the download client's reported status.

use chrono::{DateTime, Utc};
use scryer_domain::{
    CompletedDownload, DownloadQueueItem, Title, TitleMatchType, TrackedDownloadState,
    TrackedDownloadStatus,
};
use std::collections::{HashMap, HashSet};
use tokio::sync::{mpsc, oneshot};

use crate::{
    AppResult, AppUseCase, DownloadSourceIdentity, DownloadSubmission,
    DownloadSubmissionActorSnapshot, DownloadSubmissionIdentity, SubmissionScope,
};

const DEFAULT_TRACKED_DOWNLOAD_CACHE_TTL_HOURS: i64 = 24;
const DEFAULT_TRACKED_DOWNLOAD_CACHE_MAX_ENTRIES: usize = 5_000;

// ── TrackedDownload ──────────────────────────────────────────────────────────

/// A download being tracked through scryer's import workflow.
#[derive(Clone, Debug)]
pub struct TrackedDownload {
    /// Composite key scoped to the configured client when available.
    pub id: String,
    pub client_id: String,
    pub client_type: String,
    /// Latest snapshot from the download client.
    pub client_item: DownloadQueueItem,
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
    /// Runtime-only reason this completed download is held back and hidden.
    /// Never persisted as a tracked-download outcome.
    pub import_hold: Option<ImportHold>,
    /// Manual failure actions can record the failure without reacquiring.
    pub skip_reacquire_on_failure: bool,
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

/// Why a completed download is held out of automatic import and hidden from
/// user-facing download activity.
///
/// TWO INDEPENDENT AXES, deliberately kept apart. The predecessor type
/// (`ForeignDownloadClassification`) put both in one flat enum, so a payload
/// with no video was reported through a field whose name asserted whose
/// download it was. Readers — several, repeatedly — took `NoImportableVideo`
/// to mean "another application owns this" and reasoned about ownership from
/// it. Nesting makes the misreading unrepresentable: you cannot reach a
/// provenance verdict without going through `Unmanaged`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportHold {
    /// PROVENANCE. Scryer did not submit this download.
    ///
    /// Says nothing about whether the payload is importable.
    Unmanaged(UnmanagedDownloadReason),
    /// CONTENT. The payload holds nothing importable.
    ///
    /// Says nothing about who submitted it — Scryer's own grabs land here too.
    NoImportableVideo,
}

/// Why Scryer believes it did not submit a download.
///
/// "Unmanaged" rather than "foreign" because these downloads are usually ones
/// Scryer SHOULD adopt once a user resolves them — a user-uploaded NZB, or one
/// the download client's own RSS grabbed, is not another application's
/// property; it simply was not submitted by Scryer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnmanagedDownloadReason {
    /// The category is not one Scryer configured for any client.
    ///
    /// Weak evidence, and an ABSENCE of a signal rather than a positive one: it
    /// cannot distinguish a user upload from the download client's own RSS
    /// grab, because no client payload is read for a source/feed marker today
    /// (NZBGet exposes `URL`/`Kind` but Scryer does not read them; SABnzbd has
    /// no submitter channel; qBittorrent has none). Those two origins are
    /// indistinguishable and both land here.
    ///
    /// Because it is configuration-derived, this verdict is re-evaluated on
    /// later passes rather than trusted from cache.
    UnknownCategory,
    /// Another download manager submitted it.
    ///
    /// A POSITIVE identification, unlike `UnknownCategory` — but only of one
    /// specific marker: the `drone` parameter Sonarr/Radarr stamp on the item.
    /// A manager that does not stamp it will not be detected here, so treat
    /// this as "a known external manager" rather than "any external manager".
    ExternalManager,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoVideoImportRetryState {
    pub signature: NoVideoImportSourceSignature,
    pub attempts: u8,
    pub next_retry_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoVideoImportSourceSignature {
    pub source_path: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub latest_mtime: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
        self.import_hold = finished.import_hold;
    }

    pub(crate) fn reset_for_import_retry(&mut self) {
        self.state = TrackedDownloadState::ImportPending;
        self.status = TrackedDownloadStatus::Ok;
        self.status_messages.clear();
        self.import_attempted = false;
        self.waiting_for_completed_history = false;
        self.path_missing_since = None;
        self.no_video_import_retry = None;
        self.import_hold = None;
        self.skip_reacquire_on_failure = false;
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

/// In-memory cache of tracked downloads with title resolution and state management.
#[derive(Default)]
pub struct TrackedDownloadService {
    cache: HashMap<String, TrackedDownload>,
    last_seen_at: HashMap<String, DateTime<Utc>>,
}

impl TrackedDownloadService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create or update a tracked download from a client item snapshot.
    ///
    /// On first see: resolves title, checks for terminal state in DB.
    /// On update: refreshes client_item but preserves scryer state if past Downloading.
    pub async fn track(&mut self, app: &AppUseCase, client_item: DownloadQueueItem) {
        let id = tracked_download_id_for_item(&client_item);
        self.last_seen_at.insert(id.clone(), Utc::now());

        if self.cache.contains_key(&id) {
            let existing = self.cache.get_mut(&id).unwrap();
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
            existing.client_item = client_item;
            existing.is_trackable = true;
            if should_reresolve {
                Self::resolve_title(app, existing).await;
            }
            return;
        }

        // First time seeing this download — build, resolve, and insert.
        let td = Self::build_new_tracked_download(app, id.clone(), client_item).await;
        self.cache.insert(id, td);
        self.prune_cache();
    }

    /// Build a new TrackedDownload, resolving title and reconstructing state.
    async fn build_new_tracked_download(
        app: &AppUseCase,
        id: String,
        client_item: DownloadQueueItem,
    ) -> TrackedDownload {
        let mut td = TrackedDownload {
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
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
            snapshot_missing_since: None,
        };

        Self::resolve_title(app, &mut td).await;
        Self::reconstruct_state(app, &mut td).await;
        td
    }

    pub fn find(&self, id: &str) -> Option<&TrackedDownload> {
        self.cache.get(id)
    }

    #[cfg(test)]
    pub(crate) fn insert_for_tests(&mut self, tracked: TrackedDownload) {
        let id = tracked.id.clone();
        self.last_seen_at.insert(id.clone(), Utc::now());
        self.cache.insert(id, tracked);
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut TrackedDownload> {
        self.cache.get_mut(id)
    }

    pub fn resolve_cached_id(&self, requested_id: &str) -> Option<String> {
        if self.cache.contains_key(requested_id) {
            return Some(requested_id.to_string());
        }

        self.cache.iter().find_map(|(id, tracked)| {
            tracked_download_matches_source_id(tracked, requested_id).then(|| id.clone())
        })
    }

    pub fn get_all(&self) -> Vec<&TrackedDownload> {
        self.cache.values().collect()
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

    /// Mark downloads no longer visible in any client as untrackable.
    pub fn update_trackable(&mut self, seen_ids: &HashSet<String>) -> Vec<DownloadSourceIdentity> {
        let mut unavailable_sources = Vec::new();
        for td in self.cache.values_mut() {
            if td.is_trackable && !seen_ids.contains(&td.id) && !should_preserve_tracking(td.state)
            {
                td.is_trackable = false;
                unavailable_sources.push(DownloadSourceIdentity::new(
                    Some(&td.client_id),
                    &td.client_type,
                    &td.client_item.download_client_item_id,
                ));
            }
        }
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
    ) -> Vec<DownloadSourceIdentity> {
        let now = Utc::now();
        let mut unavailable_sources = Vec::new();
        for td in self.cache.values_mut() {
            if tracked_client_type_is_excluded(&td.client_type, excluded_client_types) {
                continue;
            }
            if seen_ids.contains(&td.id) {
                td.snapshot_missing_since = None;
                continue;
            }
            if td.is_trackable
                && !should_preserve_tracking(td.state)
                && snapshot_absence_exceeds_grace(td, now)
            {
                td.is_trackable = false;
                unavailable_sources.push(DownloadSourceIdentity::new(
                    Some(&td.client_id),
                    &td.client_type,
                    &td.client_item.download_client_item_id,
                ));
            }
        }
        self.prune_cache();
        unavailable_sources
    }

    /// Mark downloads absent from an authoritative client-scoped snapshot as untrackable.
    pub fn update_trackable_for_scope(
        &mut self,
        seen_ids: &HashSet<String>,
        scope: &TrackedDownloadSnapshotScope,
    ) -> Vec<DownloadSourceIdentity> {
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
            if seen_ids.contains(&td.id) {
                td.snapshot_missing_since = None;
                continue;
            }
            if td.is_trackable
                && !should_preserve_tracking(td.state)
                && snapshot_absence_exceeds_grace(td, now)
            {
                td.is_trackable = false;
                unavailable_sources.push(DownloadSourceIdentity::new(
                    Some(&td.client_id),
                    &td.client_type,
                    &td.client_item.download_client_item_id,
                ));
            }
        }
        self.prune_cache();
        unavailable_sources
    }

    /// Remove a download from the cache (after terminal state).
    pub fn stop_tracking(&mut self, id: &str) {
        self.cache.remove(id);
        self.last_seen_at.remove(id);
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
                        id.clone(),
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
        let Some(td) = self.cache.get(id) else {
            return false;
        };
        persist_tracked_download_state_marker(app, td, state, None, None).await
    }

    // ── Title Resolution ─────────────────────────────────────────────────

    async fn resolve_title(app: &AppUseCase, td: &mut TrackedDownload) {
        let can_clear_stale_unmatched_state = should_clear_stale_unmatched_state_on_submission(td);
        let mut existing_submission = app
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&DownloadSourceIdentity::new(
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
                download_id_submission_for_tracked_download(app, td).await
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

        // 3. Parse-based monitored title resolution for foreign downloads.
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
                return;
            }
        }

        // 4. No trustworthy title match found — completed handler will block
        // auto-import until the user assigns the title manually.
        //
        // Insert a stub download_submissions row for foreign downloads so they
        // get a tracked_state column for restart reconstruction.
        if existing_submission.is_none()
            && let Err(error) = app
                .services
                .workflow
                .download_submissions
                .record_submission(DownloadSubmission {
                    title_id: String::new(),
                    purpose: crate::DownloadSubmissionPurpose::Standard,
                    facet: td.facet.clone().unwrap_or_default(),
                    download_client_id: Some(td.client_id.clone())
                        .filter(|value| !value.is_empty()),
                    download_client_type: td.client_type.clone(),
                    download_client_item_id: td.client_item.download_client_item_id.clone(),
                    source_hint: None,
                    source_provider_id: None,
                    source_provider_name: None,
                    source_kind: None,
                    source_title: Some(td.client_item.title_name.clone()),
                    request_signature: None,
                    scope: SubmissionScope::Orphan,
                })
                .await
        {
            tracing::warn!(error = %error, id = %td.id, "failed to record tracked download stub submission");
        }
    }

    /// Reconstruct state from persistent storage after restart.
    async fn reconstruct_state(app: &AppUseCase, td: &mut TrackedDownload) {
        let observed_identity = observed_queue_item_identity(&td.client_item);
        let observed_source_identity = queue_item_source_identity(&td.client_item);
        if !download_submission_identity_is_empty(&observed_identity)
            && let Ok(Some(tracked_state)) = app
                .services
                .workflow
                .download_submissions
                .get_identity_tracked_state(&observed_identity, Some(&observed_source_identity))
                .await
            && let Some(state) = TrackedDownloadState::from_str_opt(&tracked_state)
            && (state.is_terminal() || state == TrackedDownloadState::ImportBlocked)
        {
            td.state = state;
            return;
        }

        let download_id_submission = download_id_submission_for_tracked_download(app, td).await;
        // Check tracked state against the matched submission identity first.
        if let Some(submission) = download_id_submission.as_ref() {
            let submission_source_identity = DownloadSourceIdentity::from_submission(submission);
            if let Ok(Some(tracked_state)) = app
                .services
                .workflow
                .download_submissions
                .get_tracked_state(&submission_source_identity)
                .await
                && let Some(state) = TrackedDownloadState::from_str_opt(&tracked_state)
                && (state.is_terminal() || state == TrackedDownloadState::ImportBlocked)
            {
                td.state = state;
                return;
            }
        }

        // Fall back to the latest import record for restart recovery if the
        // tracked state was not persisted before shutdown. This is only safe
        // after the current DownloadId resolves to a Scryer submission.
        if let Some(submission) = download_id_submission {
            let submission_identity = DownloadSourceIdentity::from_submission(&submission);
            let download_identity = app
                .services
                .workflow
                .download_submissions
                .get_submission_identity(&submission_identity)
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
            let imported = !download_submission_identity_is_empty(&download_identity)
                && app
                    .services
                    .workflow
                    .imports
                    .is_already_imported_by_download_id(&submission_identity, &download_identity)
                    .await
                    .unwrap_or(false);
            if imported {
                td.state = TrackedDownloadState::Imported;
                let _ = app
                    .services
                    .workflow
                    .download_submissions
                    .update_tracked_state(
                        &submission_identity,
                        TrackedDownloadState::Imported.as_str(),
                    )
                    .await;
            }
        }

        // Default: Downloading (will be re-evaluated by check cycle).
    }
}

async fn download_id_submission_for_tracked_download(
    app: &AppUseCase,
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
            .list_by_download_id(
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

/// Whether a manual import could express a target for this title.
///
/// Manual import can only map a file to an episode or a series-movie link, and
/// both belong to series-shaped titles. A movie has neither, so leaving a movie
/// download parked "for manual intervention" offers the user no action that can
/// actually complete it.
fn title_has_mappable_import_targets(title: &Title) -> bool {
    match title.facet {
        scryer_domain::MediaFacet::Series | scryer_domain::MediaFacet::Anime => true,
        scryer_domain::MediaFacet::Movie => false,
    }
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

pub(crate) async fn assign_title_to_tracked_download(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    title: &Title,
) {
    td.title_id = Some(title.id.clone());
    td.facet = Some(title.facet.as_str().to_string());
    td.match_type = TitleMatchType::Submission;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
    td.import_attempted = false;

    // A download that is already blocked for manual intervention should stay
    // manually actionable after title assignment instead of being pushed
    // straight back into auto-import.
    //
    // EXCEPT when the assigned title has no mappable import targets. Manual
    // import exists to answer one question — which episode does this file
    // belong to — and it can only express two targets: an episode, or a
    // series-movie link (see ManualImportMappingTarget and
    // manual_import_preview_targets, which returns exactly those two lists).
    // A plain movie has neither, so "stay manually actionable" strands it:
    // the user assigns the title and the only remaining action is a manual
    // import that cannot represent a movie at all.
    //
    // Movies were always meant to resolve via assignment instead — Submission
    // is in the high-confidence set that completed_download_allows_automatic_import
    // waves through — so fall through to the re-check for them. Series and
    // anime keep the early return, because for those the mapping decision is
    // real and still waiting on the user.
    if td.state == TrackedDownloadState::ImportBlocked && title_has_mappable_import_targets(title) {
        return;
    }

    td.state = TrackedDownloadState::Downloading;
    crate::failed_download_handler::check(td);
    crate::completed_download_handler::check(app, td).await;
}

// ── Command Channel ──────────────────────────────────────────────────────────

/// Commands sent from GraphQL mutations to the poller's TrackedDownloadService.
pub enum TrackedDownloadCommand {
    MarkImported {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    Ignore {
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

    pub async fn mark_imported(&self, id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::MarkImported {
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
pub(crate) async fn persist_tracked_download_state_marker(
    app: &AppUseCase,
    td: &TrackedDownload,
    state: TrackedDownloadState,
    reason: Option<&str>,
    detail: Option<&str>,
) -> bool {
    let state_identity = match download_id_submission_for_tracked_download(app, td).await {
        Some(submission) => DownloadSourceIdentity::from_submission(&submission),
        None => DownloadSourceIdentity::new(
            Some(td.client_id.as_str()),
            &td.client_type,
            &td.client_item.download_client_item_id,
        ),
    };
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
            "failed to persist tracked download state"
        );
        return false;
    }

    let observed_identity = observed_queue_item_identity(&td.client_item);
    if !download_submission_identity_is_empty(&observed_identity)
        && let Err(e) = app
            .services
            .workflow
            .download_submissions
            .record_identity_tracked_state(
                &observed_identity,
                Some(&DownloadSourceIdentity::new(
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

    true
}

pub(crate) fn tracked_client_type_is_excluded(
    client_type: &str,
    excluded_client_types: &[&str],
) -> bool {
    excluded_client_types
        .iter()
        .any(|excluded| excluded.trim().eq_ignore_ascii_case(client_type.trim()))
}

fn should_preserve_tracking(state: TrackedDownloadState) -> bool {
    matches!(
        state,
        TrackedDownloadState::ImportPending
            | TrackedDownloadState::Importing
            | TrackedDownloadState::ImportBlocked
            | TrackedDownloadState::FailedPending
    )
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

fn observed_queue_item_identity(item: &DownloadQueueItem) -> DownloadSubmissionIdentity {
    crate::observed_download_identity(crate::ObservedDownloadIdentityInput {
        download_id: item.download_id.as_deref(),
        parameters: &[],
        info_hash_hint: None,
    })
}

fn queue_item_source_identity(item: &DownloadQueueItem) -> DownloadSourceIdentity {
    DownloadSourceIdentity::new(
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
        AppError, AppResult, AppServices, AppUseCase, CreateTitleOutcome, DomainEventRepository,
        DownloadClient, DownloadClientAddRequest, DownloadGrabResult, DownloadSourceIdentity,
        DownloadSubmissionRepository, FacetRegistry, ImportRepository, IndexerConfigRepository,
        JwtAuthConfig, PendingTitleHydration, TitleMetadataUpdate, TitleRepository,
    };
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use scryer_domain::{
        CompletedDownload, DomainEvent, DomainEventFilter, DownloadQueueState, Id, ImportRecord,
        ImportStatus, ImportType, MediaFacet, NewDomainEvent, Title, TitleHistoryEventType, User,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestDownloadSubmissionRepo {
        submission: Option<crate::DownloadSubmission>,
        submission_identity: Option<crate::DownloadSubmissionIdentity>,
        mutable_submission: Option<Arc<Mutex<Option<crate::DownloadSubmission>>>>,
        mutable_submission_identity: Option<Arc<Mutex<Option<crate::DownloadSubmissionIdentity>>>>,
        tracked_state: Option<String>,
        tracked_state_updates: Arc<Mutex<Vec<String>>>,
        recorded_submissions: Arc<Mutex<Vec<crate::DownloadSubmission>>>,
        download_id_submissions:
            Arc<Mutex<Vec<(crate::DownloadSubmission, crate::DownloadSubmissionIdentity)>>>,
        identity_tracked_states: Arc<Mutex<HashMap<String, String>>>,
    }

    fn test_download_identity_state_key(
        identity: &crate::DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
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

        async fn find_by_client_item_id(
            &self,
            identity: &DownloadSourceIdentity,
        ) -> AppResult<Option<crate::DownloadSubmission>> {
            Ok(self.current_submission().await.filter(|submission| {
                DownloadSourceIdentity::from_submission(submission) == *identity
            }))
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

        async fn get_submission_identity(
            &self,
            _: &DownloadSourceIdentity,
        ) -> AppResult<Option<crate::DownloadSubmissionIdentity>> {
            Ok(self.current_submission_identity().await)
        }

        async fn list_for_client_items(
            &self,
            _: &[DownloadSourceIdentity],
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

        async fn delete_by_client_item_id(&self, _: &DownloadSourceIdentity) -> AppResult<()> {
            Ok(())
        }

        async fn update_tracked_state(
            &self,
            _: &DownloadSourceIdentity,
            tracked_state: &str,
        ) -> AppResult<()> {
            self.tracked_state_updates
                .lock()
                .await
                .push(tracked_state.to_string());
            Ok(())
        }

        async fn get_tracked_state(&self, _: &DownloadSourceIdentity) -> AppResult<Option<String>> {
            Ok(self.tracked_state.clone())
        }

        async fn record_identity_tracked_state(
            &self,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&DownloadSourceIdentity>,
            tracked_state: &str,
            _: Option<&str>,
            _: Option<&str>,
        ) -> AppResult<()> {
            if let Some(key) = test_download_identity_state_key(identity, source_identity) {
                self.identity_tracked_states
                    .lock()
                    .await
                    .insert(key, tracked_state.to_string());
            }
            Ok(())
        }

        async fn get_identity_tracked_state(
            &self,
            identity: &crate::DownloadSubmissionIdentity,
            source_identity: Option<&DownloadSourceIdentity>,
        ) -> AppResult<Option<String>> {
            let Some(key) = test_download_identity_state_key(identity, source_identity) else {
                return Ok(None);
            };
            Ok(self.identity_tracked_states.lock().await.get(&key).cloned())
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
            _: DownloadSourceIdentity,
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
            identities: &[DownloadSourceIdentity],
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

        async fn is_already_imported(&self, identity: &DownloadSourceIdentity) -> AppResult<bool> {
            Ok(self.stored_imports().iter().any(|record| {
                record.source_client_id.as_deref().unwrap_or("") == identity.client_id_or_empty()
                    && record.source_system == identity.client_type
                    && record.source_ref == identity.item_id
                    && matches!(
                        record.status,
                        ImportStatus::Completed | ImportStatus::Skipped
                    )
            }))
        }

        async fn is_already_imported_by_download_id(
            &self,
            source_identity: &DownloadSourceIdentity,
            identity: &crate::DownloadSubmissionIdentity,
        ) -> AppResult<bool> {
            let Some(download_id) = identity.download_id.as_deref() else {
                return Ok(false);
            };
            Ok(self.stored_imports().iter().any(|record| {
                record.source_client_id.as_deref().unwrap_or("")
                    == source_identity.client_id_or_empty()
                    && record.source_system == source_identity.client_type
                    && record.download_id.as_deref() == Some(download_id)
                    && matches!(
                        record.status,
                        ImportStatus::Completed | ImportStatus::Skipped
                    )
            }))
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
    }

    fn trigger_user() -> User {
        let mut libraries = HashMap::new();
        let permissions = scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
            scryer_domain::LibraryPermission::ResolveImports,
            scryer_domain::LibraryPermission::ManageLibrary,
        ]);
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            libraries.insert(
                scryer_domain::default_library_id_for_facet(&facet),
                permissions,
            );
        }
        User {
            id: "user-1".to_string(),
            username: "user@example.test".to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: scryer_domain::UserAuthorization {
                app: scryer_domain::AppPermissionMask::NONE,
                libraries,
                default_library: permissions,
                actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
                login_status: Default::default(),
                loaded: true,
            },
        }
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
            id: id.to_string(),
            client_id: client_item.client_id.clone(),
            client_type: client_item.client_type.clone(),
            client_item,
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
            import_hold: None,
            skip_reacquire_on_failure: false,
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

        tracked.merge_background_work_state_from(finished);

        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
        assert_eq!(tracked.status, TrackedDownloadStatus::Warning);
        assert_eq!(tracked.status_messages, vec!["retry later"]);
        assert_eq!(tracked.title_id.as_deref(), Some("title-1"));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);
        assert!(tracked.import_attempted);
        assert!(tracked.path_missing_since.is_some());
        assert_eq!(tracked.no_video_import_retry, Some(retry));
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
        tracked.skip_reacquire_on_failure = true;

        tracked.reset_for_import_retry();

        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
        assert_eq!(tracked.status, TrackedDownloadStatus::Ok);
        assert!(tracked.status_messages.is_empty());
        assert!(!tracked.import_attempted);
        assert!(tracked.path_missing_since.is_none());
        assert!(tracked.no_video_import_retry.is_none());
        assert!(!tracked.skip_reacquire_on_failure);
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
    async fn resolves_submission_by_download_id_when_client_item_id_differs() {
        let download_id = "cc025b54883bbdc61258e9d5627b3bd1613241b2";
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: Some(crate::DownloadSubmission {
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
                request_signature: None,
                scope: crate::SubmissionScope::Title,
            }),
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
            request_signature: None,
            scope: crate::SubmissionScope::Orphan,
        };
        let managed = crate::DownloadSubmission {
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
    async fn reconstruct_state_recovers_imported_from_completed_import_record() {
        let download_id = "scryer-download:restart-recovery";
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: Some(crate::DownloadSubmission {
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
                request_signature: None,
                scope: crate::SubmissionScope::Title,
            }),
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
        });
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
        assert_eq!(tracked.state, TrackedDownloadState::Imported);
        assert_eq!(
            download_submissions
                .tracked_state_updates
                .lock()
                .await
                .as_slice(),
            ["imported"]
        );
    }

    #[tokio::test]
    async fn reconstruct_state_does_not_recover_client_local_state_from_other_client() {
        let download_id = "10010";
        let identity = crate::DownloadSubmissionIdentity {
            download_id: Some(download_id.to_string()),
        };
        let other_client_source = DownloadSourceIdentity::new(Some("client-2"), "nzbget", "dl-1");
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
    async fn persist_terminal_state_returns_false_when_repository_write_fails() {
        #[derive(Default)]
        struct FailingDownloadSubmissionRepo;

        #[async_trait]
        impl DownloadSubmissionRepository for FailingDownloadSubmissionRepo {
            async fn record_submission(&self, _: crate::DownloadSubmission) -> AppResult<()> {
                Ok(())
            }

            async fn find_by_client_item_id(
                &self,
                _: &DownloadSourceIdentity,
            ) -> AppResult<Option<crate::DownloadSubmission>> {
                Ok(None)
            }

            async fn list_for_client_items(
                &self,
                _: &[DownloadSourceIdentity],
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

            async fn delete_by_client_item_id(&self, _: &DownloadSourceIdentity) -> AppResult<()> {
                Ok(())
            }

            async fn update_tracked_state(
                &self,
                _: &DownloadSourceIdentity,
                _: &str,
            ) -> AppResult<()> {
                Err(AppError::Repository("boom".into()))
            }

            async fn get_tracked_state(
                &self,
                _: &DownloadSourceIdentity,
            ) -> AppResult<Option<String>> {
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
        );

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
            submission_identity: None,
            mutable_submission: None,
            mutable_submission_identity: None,
            tracked_state: None,
            tracked_state_updates: Arc::new(Mutex::new(vec![])),
            recorded_submissions: Arc::new(Mutex::new(vec![])),
            download_id_submissions: Arc::new(Mutex::new(vec![])),
            identity_tracked_states: Arc::new(Mutex::new(HashMap::new())),
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
    async fn assigning_title_to_completed_blocked_download_keeps_manual_import_actionable() {
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
        assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
        assert!(tracked.title_id.is_none());

        let tracked = tracker
            .find_mut("client-1:job-manual-movie")
            .expect("tracked download mut");
        assign_title_to_tracked_download(&app, tracked, &title).await;

        // A MOVIE has no manual-import target (manual import maps files to
        // episodes or series-movie links only), so keeping it ImportBlocked
        // after assignment stranded it with no completable action. Assignment
        // now releases movies back into auto-import: the embedded re-check runs
        // against the completed download and, with the high-confidence
        // Submission match, moves it to ImportPending.
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

        // Movie: released from the manual-intervention park (see the
        // completed-blocked variant above). The client is still Downloading, so
        // the re-check is a no-op and the download simply resumes normal
        // tracking until the client reports completion.
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);
        assert!(!tracked.import_attempted);

        crate::completed_download_handler::check(&app, tracked).await;
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
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
            tracker.cache.insert(
                format!("client-1:{suffix}"),
                TrackedDownload {
                    id: format!("client-1:{suffix}"),
                    client_id: "client-1".to_string(),
                    client_type: "nzbget".to_string(),
                    client_item: build_client_item(),
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
                    import_hold: None,
                    skip_reacquire_on_failure: false,
                    snapshot_missing_since: None,
                },
            );
        }

        let unavailable_sources = tracker.update_trackable(&HashSet::new());

        assert!(
            tracker
                .find("client-1:pending")
                .is_some_and(|td| td.is_trackable)
        );
        assert!(
            tracker
                .find("client-1:importing")
                .is_some_and(|td| td.is_trackable)
        );
        assert!(
            tracker
                .find("client-1:blocked")
                .is_some_and(|td| td.is_trackable)
        );
        assert!(
            tracker
                .find("client-1:failed")
                .is_some_and(|td| td.is_trackable)
        );
        assert!(unavailable_sources.is_empty());
    }

    #[test]
    fn scoped_snapshot_pruning_preserves_import_blocked_sources_after_grace() {
        let mut tracker = TrackedDownloadService::new();
        let mut blocked = build_tracked_download("blocked");
        blocked.state = TrackedDownloadState::ImportBlocked;
        let blocked_id = blocked.id.clone();
        tracker.cache.insert(blocked_id.clone(), blocked);
        let scope = TrackedDownloadSnapshotScope::AuthoritativeForClient {
            client_id: Some("client-1".to_string()),
            client_type: "nzbget".to_string(),
        };

        let unavailable_sources = tracker.update_trackable_for_scope(&HashSet::new(), &scope);
        expire_snapshot_absence(&mut tracker, &blocked_id);
        let unavailable_sources_after_grace =
            tracker.update_trackable_for_scope(&HashSet::new(), &scope);

        assert!(tracker.find(&blocked_id).is_some_and(|td| td.is_trackable));
        assert!(unavailable_sources.is_empty());
        assert!(unavailable_sources_after_grace.is_empty());
    }

    #[test]
    fn excluded_client_snapshot_pruning_preserves_import_blocked_after_grace() {
        let mut tracker = TrackedDownloadService::new();
        let mut blocked = build_tracked_download("blocked");
        blocked.state = TrackedDownloadState::ImportBlocked;
        let blocked_id = blocked.id.clone();
        tracker.cache.insert(blocked_id.clone(), blocked);

        tracker.update_trackable_excluding_client_types(&HashSet::new(), &[]);
        expire_snapshot_absence(&mut tracker, &blocked_id);
        let unavailable_sources =
            tracker.update_trackable_excluding_client_types(&HashSet::new(), &[]);

        assert!(tracker.find(&blocked_id).is_some_and(|td| td.is_trackable));
        assert!(unavailable_sources.is_empty());
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
        tracker.cache.insert(weaver_id.clone(), weaver);
        tracker.cache.insert(nzb_id.clone(), nzb);

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
        if let Some(td) = tracker.cache.get_mut(id) {
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
        tracker.cache.insert(id.clone(), td);

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
        tracker.cache.insert(weaver_id.clone(), weaver);
        tracker.cache.insert(nzb_id.clone(), nzb);

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
        tracker.cache.insert(old.id.clone(), old);
        tracker.cache.insert(recent.id.clone(), recent);
        let now = Utc::now();
        tracker.last_seen_at.insert(
            "old-unmatched".to_string(),
            now - chrono::Duration::minutes(10),
        );
        tracker
            .last_seen_at
            .insert("recent-unmatched".to_string(), now);

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
        tracker.cache.insert(actionable.id.clone(), actionable);
        tracker.cache.insert(failed.id.clone(), failed);
        tracker.cache.insert(low_value.id.clone(), low_value);
        let now = Utc::now();
        tracker.last_seen_at.insert(
            "actionable".to_string(),
            now - chrono::Duration::minutes(10),
        );
        tracker.last_seen_at.insert(
            "failed-actionable".to_string(),
            now - chrono::Duration::minutes(5),
        );
        tracker.last_seen_at.insert("low-value".to_string(), now);

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
            id: "client-1:failed-import-pending".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item,
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
            import_hold: None,
            skip_reacquire_on_failure: false,
            snapshot_missing_since: None,
        };

        crate::failed_download_handler::check(&mut tracked);

        assert_eq!(tracked.state, TrackedDownloadState::FailedPending);
        assert_eq!(tracked.status, TrackedDownloadStatus::Error);
    }

    #[test]
    fn failed_download_check_skips_parse_matched_foreign_download() {
        let mut client_item = build_client_item();
        client_item.state = DownloadQueueState::Failed;
        client_item.attention_reason = Some("health below critical".to_string());
        client_item.is_scryer_origin = false;
        let mut tracked = TrackedDownload {
            id: "client-1:failed-foreign".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item,
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: Some("Foreign.Show.S01E01.1080p.WEB-DL".to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::TitleParse,
            is_trackable: true,
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
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
            id: "client-1:job-active-manual".to_string(),
            client_id: "client-1".to_string(),
            client_type: "weaver".to_string(),
            client_item,
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
            import_hold: None,
            skip_reacquire_on_failure: false,
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
            import_hold: None,
            skip_reacquire_on_failure: false,
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
}
