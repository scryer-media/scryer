use chrono::{DateTime, Utc};
use scryer_domain::{
    DomainEvent, DomainEventPayload, LibraryScanCanceledEventData, LibraryScanCompletedEventData,
    LibraryScanDeltaRecordedEventData, LibraryScanFailedEventData, LibraryScanProgressedEventData,
    LibraryScanStartedEventData, LibraryScanTitleDiscoveredEventData, MediaFacet,
};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tokio::time::{Duration, Sleep};
use tracing::{debug, trace};

use crate::{AppError, AppResult, Id, JobRunTracker, LibraryScanSummary};

const LIBRARY_SCAN_PROGRESS_PUSH_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryScanStatus {
    Discovering,
    Running,
    Completed,
    Canceled,
    Warning,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryScanMode {
    Full,
    Additive,
}

impl LibraryScanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovering => "discovering",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
            Self::Warning => "warning",
            Self::Failed => "failed",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Canceled | Self::Warning | Self::Failed
        )
    }
}

impl LibraryScanMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Additive => "additive",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryScanPhaseProgress {
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
}

impl LibraryScanPhaseProgress {
    fn add_total(&mut self, additional: usize) {
        self.total = self.total.saturating_add(additional);
    }

    fn mark_completed(&mut self, additional: usize) {
        let remaining = self
            .total
            .saturating_sub(self.completed.saturating_add(self.failed));
        self.completed = self.completed.saturating_add(additional.min(remaining));
    }

    fn mark_failed(&mut self, additional: usize) {
        let remaining = self
            .total
            .saturating_sub(self.completed.saturating_add(self.failed));
        self.failed = self.failed.saturating_add(additional.min(remaining));
    }

    fn is_finished(&self) -> bool {
        self.completed.saturating_add(self.failed) >= self.total
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryScanSession {
    pub session_id: String,
    pub facet: MediaFacet,
    pub library_id: Option<String>,
    pub mode: LibraryScanMode,
    pub status: LibraryScanStatus,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub found_titles: usize,
    pub title_match_total_known: bool,
    pub metadata_total_known: bool,
    pub file_total_known: bool,
    pub title_match_progress: LibraryScanPhaseProgress,
    pub metadata_progress: LibraryScanPhaseProgress,
    pub file_progress: LibraryScanPhaseProgress,
    pub summary: Option<LibraryScanSummary>,
    pub warning_message: Option<String>,
}

impl LibraryScanSession {
    fn new(facet: MediaFacet) -> Self {
        let now = Utc::now();
        Self {
            session_id: Id::new().0,
            facet,
            library_id: None,
            mode: LibraryScanMode::Full,
            status: LibraryScanStatus::Discovering,
            started_at: now,
            updated_at: now,
            found_titles: 0,
            title_match_total_known: false,
            metadata_total_known: false,
            file_total_known: false,
            title_match_progress: LibraryScanPhaseProgress::default(),
            metadata_progress: LibraryScanPhaseProgress::default(),
            file_progress: LibraryScanPhaseProgress::default(),
            summary: None,
            warning_message: None,
        }
    }

    pub(crate) fn with_id_for_library(
        session_id: String,
        facet: MediaFacet,
        library_id: Option<String>,
        mode: LibraryScanMode,
    ) -> Self {
        let mut session = Self::new(facet);
        session.session_id = session_id;
        session.library_id = library_id;
        session.mode = mode;
        session
    }

    pub(crate) fn is_ready_to_complete(&self) -> bool {
        self.summary.is_some()
            && self.title_match_progress.is_finished()
            && self.metadata_progress.is_finished()
            && self.file_progress.is_finished()
    }

    pub(crate) fn completion_status(&self) -> LibraryScanStatus {
        if self.title_match_progress.failed > 0
            || self.metadata_progress.failed > 0
            || self.file_progress.failed > 0
            || self.warning_message.is_some()
        {
            LibraryScanStatus::Warning
        } else {
            LibraryScanStatus::Completed
        }
    }
}

#[derive(Default)]
struct LibraryScanRuntimeState {
    sessions: HashMap<String, LibraryScanSession>,
    next_sequence: u64,
    /// At most one coalesced delta per session that has been folded into the
    /// live session already but has not been written to the event log yet.
    /// See [`stage_library_scan_delta`].
    pending_deltas: HashMap<String, PendingLibraryScanDelta>,
}

/// A run of per-title deltas merged into one event-log record.
struct PendingLibraryScanDelta {
    data: LibraryScanDeltaRecordedEventData,
    /// Captured when the run opens so a flush can build the event even after
    /// the session has left the tracker (completed/failed/canceled).
    facet: MediaFacet,
    merged: usize,
}

/// How many per-title deltas one pending record may absorb before it is
/// written out even though no progress publish is due.
///
/// A scan phase can walk thousands of unchanged titles without publishing
/// progress once (the background refresh's title-match loop does exactly
/// that), so the publish boundary alone is not a bound. This caps both the
/// staleness of the persisted log and what a crash can lose.
const LIBRARY_SCAN_PENDING_DELTA_MERGE_LIMIT: usize = 256;

#[derive(Clone)]
struct LibraryScanTrackerEvent {
    sequence: u64,
    session: LibraryScanSession,
}

/// The outcome of folding one delta into the live session.
pub(crate) struct StagedLibraryScanDelta {
    /// The live session after the delta was applied.
    pub(crate) session: LibraryScanSession,
    /// The delta records that must be appended to the event log now, in
    /// order. Usually empty: a coalescable delta is merged into the pending
    /// record instead and written at the next flush.
    pub(crate) to_persist: Vec<LibraryScanDeltaRecordedEventData>,
}

/// True when `delta` carries nothing but phase completion/failure counters.
///
/// Those are the only fields whose projection is a plain clamped addition
/// against state the delta itself does not move, which is what makes a run of
/// them safe to sum into one record. Anything that moves a total, latches a
/// `*_total_known` flag, or sets a summary changes how a *later* field in the
/// same record is projected, so it is never merged.
fn is_coalescable_library_scan_delta(delta: &LibraryScanDeltaRecordedEventData) -> bool {
    delta.found_titles_total.is_none()
        && delta.found_titles_delta == 0
        && delta.title_match_total_known.is_none()
        && delta.metadata_total_delta == 0
        && delta.metadata_total_known.is_none()
        && delta.file_total_delta == 0
        && delta.file_total_known.is_none()
        && delta.summary.is_none()
}

/// True when merging `incoming` into `pending` projects identically to
/// applying the two records in order.
///
/// `mark_completed`/`mark_failed` share one remaining-capacity budget per
/// phase, and the merged record always spends it on `completed` first, so a
/// phase that carries both in a single record would redistribute the clamp if
/// the original order was failure-first. Phases that only ever move one of the
/// two counters are unaffected (`min(a, r) + min(b, r - min(a, r))` is
/// `min(a + b, r)`), which covers every per-title loop.
fn can_merge_library_scan_deltas(
    pending: &LibraryScanDeltaRecordedEventData,
    incoming: &LibraryScanDeltaRecordedEventData,
) -> bool {
    fn phase_is_single_sided(
        pending_completed: i64,
        pending_failed: i64,
        incoming_completed: i64,
        incoming_failed: i64,
    ) -> bool {
        let completed = pending_completed
            .max(0)
            .saturating_add(incoming_completed.max(0));
        let failed = pending_failed.max(0).saturating_add(incoming_failed.max(0));
        completed == 0 || failed == 0
    }

    phase_is_single_sided(
        pending.title_match_completed_delta,
        pending.title_match_failed_delta,
        incoming.title_match_completed_delta,
        incoming.title_match_failed_delta,
    ) && phase_is_single_sided(
        pending.metadata_completed_delta,
        pending.metadata_failed_delta,
        incoming.metadata_completed_delta,
        incoming.metadata_failed_delta,
    ) && phase_is_single_sided(
        pending.file_completed_delta,
        pending.file_failed_delta,
        incoming.file_completed_delta,
        incoming.file_failed_delta,
    )
}

fn merge_library_scan_delta(
    pending: &mut LibraryScanDeltaRecordedEventData,
    incoming: &LibraryScanDeltaRecordedEventData,
) {
    pending.title_match_completed_delta = pending
        .title_match_completed_delta
        .saturating_add(incoming.title_match_completed_delta);
    pending.title_match_failed_delta = pending
        .title_match_failed_delta
        .saturating_add(incoming.title_match_failed_delta);
    pending.metadata_completed_delta = pending
        .metadata_completed_delta
        .saturating_add(incoming.metadata_completed_delta);
    pending.metadata_failed_delta = pending
        .metadata_failed_delta
        .saturating_add(incoming.metadata_failed_delta);
    pending.file_completed_delta = pending
        .file_completed_delta
        .saturating_add(incoming.file_completed_delta);
    pending.file_failed_delta = pending
        .file_failed_delta
        .saturating_add(incoming.file_failed_delta);
}

/// Folds `delta` into the session's pending record and returns whatever must
/// be written to the event log now.
///
/// An idle scheduled scan probes every title and records one delta per
/// unchanged title; persisting each one appended ~12k domain events per
/// library per cycle for a library where nothing changed. The event log only
/// has to carry the progress information, not one record per probe, so a run
/// of per-title counter deltas collapses into a single record that is written
/// at the next flush.
fn stage_library_scan_delta(
    pending: &mut HashMap<String, PendingLibraryScanDelta>,
    session_id: &str,
    facet: &MediaFacet,
    delta: LibraryScanDeltaRecordedEventData,
) -> Vec<LibraryScanDeltaRecordedEventData> {
    let mut to_persist = Vec::new();

    if !is_coalescable_library_scan_delta(&delta) {
        if let Some(previous) = pending.remove(session_id) {
            to_persist.push(previous.data);
        }
        to_persist.push(delta);
        return to_persist;
    }

    match pending.get_mut(session_id) {
        Some(existing) if can_merge_library_scan_deltas(&existing.data, &delta) => {
            merge_library_scan_delta(&mut existing.data, &delta);
            existing.merged = existing.merged.saturating_add(1);
            if existing.merged >= LIBRARY_SCAN_PENDING_DELTA_MERGE_LIMIT
                && let Some(flushed) = pending.remove(session_id)
            {
                to_persist.push(flushed.data);
            }
        }
        _ => {
            if let Some(previous) = pending.remove(session_id) {
                to_persist.push(previous.data);
            }
            pending.insert(
                session_id.to_string(),
                PendingLibraryScanDelta {
                    data: delta,
                    facet: facet.clone(),
                    merged: 1,
                },
            );
        }
    }

    to_persist
}

fn library_scan_scopes_conflict(
    active: &LibraryScanSession,
    facet: &MediaFacet,
    library_id: Option<&str>,
) -> bool {
    if &active.facet != facet {
        return false;
    }

    match (active.library_id.as_deref(), library_id) {
        (Some(active_library_id), Some(requested_library_id)) => {
            active_library_id == requested_library_id
        }
        _ => true,
    }
}

fn flush_pending_library_scan_sessions(
    pending: &mut HashMap<String, LibraryScanSession>,
    tx: &broadcast::Sender<LibraryScanSession>,
) -> bool {
    let mut sessions = pending
        .drain()
        .map(|(_, session)| session)
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    sessions.into_iter().all(|session| tx.send(session).is_ok())
}

#[derive(Clone)]
pub struct LibraryScanTracker {
    state: Arc<Mutex<LibraryScanRuntimeState>>,
    broadcast: broadcast::Sender<LibraryScanTrackerEvent>,
    job_run_tracker: Arc<Mutex<Option<JobRunTracker>>>,
}

impl Default for LibraryScanTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl LibraryScanTracker {
    pub fn new() -> Self {
        let (broadcast, _) = broadcast::channel(256);
        Self {
            state: Arc::new(Mutex::new(LibraryScanRuntimeState::default())),
            broadcast,
            job_run_tracker: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_job_run_tracker(&self, tracker: JobRunTracker) {
        let mut slot = self.job_run_tracker.lock().await;
        *slot = Some(tracker);
    }

    fn spawn_subscription(
        mut source: broadcast::Receiver<LibraryScanTrackerEvent>,
        min_sequence_exclusive: Option<u64>,
    ) -> broadcast::Receiver<LibraryScanSession> {
        let (tx, rx) = broadcast::channel(256);

        tokio::spawn(async move {
            let mut pending = HashMap::<String, LibraryScanSession>::new();
            let mut flush_timer: Option<Pin<Box<Sleep>>> = None;

            loop {
                if let Some(timer) = flush_timer.as_mut() {
                    tokio::select! {
                        recv_result = source.recv() => {
                            match recv_result {
                                Ok(event) => {
                                    if min_sequence_exclusive
                                        .is_some_and(|minimum| event.sequence <= minimum)
                                    {
                                        continue;
                                    }
                                    let session = event.session;
                                    if session.status.is_terminal() {
                                        pending.remove(&session.session_id);
                                        if pending.is_empty() {
                                            flush_timer = None;
                                        }
                                        if tx.send(session).is_err() {
                                            break;
                                        }
                                    } else {
                                        pending.insert(session.session_id.clone(), session);
                                    }
                                }
                                Err(broadcast::error::RecvError::Lagged(n)) => {
                                    tracing::debug!(
                                        "library_scan_progress: receiver lagged, skipped {n} messages"
                                    );
                                }
                                Err(broadcast::error::RecvError::Closed) => {
                                    flush_pending_library_scan_sessions(&mut pending, &tx);
                                    break;
                                }
                            }
                        }
                        _ = timer.as_mut() => {
                            flush_timer = None;
                            if !flush_pending_library_scan_sessions(&mut pending, &tx) {
                                break;
                            }
                        }
                    }
                    continue;
                }

                match source.recv().await {
                    Ok(event) => {
                        if min_sequence_exclusive.is_some_and(|minimum| event.sequence <= minimum) {
                            continue;
                        }
                        let session = event.session;
                        if session.status.is_terminal() {
                            pending.remove(&session.session_id);
                            if tx.send(session).is_err() {
                                break;
                            }
                        } else {
                            pending.insert(session.session_id.clone(), session);
                            flush_timer = Some(Box::pin(tokio::time::sleep(
                                LIBRARY_SCAN_PROGRESS_PUSH_INTERVAL,
                            )));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(
                            "library_scan_progress: receiver lagged, skipped {n} messages"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        flush_pending_library_scan_sessions(&mut pending, &tx);
                        break;
                    }
                }
            }
        });

        rx
    }

    /// Test seam: live subscriptions to the tracker's event stream, so a test
    /// can tell when a waiter such as [`Self::wait_until_idle`] has parked.
    #[cfg(test)]
    pub(crate) fn subscription_count(&self) -> usize {
        self.broadcast.receiver_count()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LibraryScanSession> {
        Self::spawn_subscription(self.broadcast.subscribe(), None)
    }

    pub async fn subscribe_with_initial_snapshot(
        &self,
    ) -> (
        Vec<LibraryScanSession>,
        broadcast::Receiver<LibraryScanSession>,
    ) {
        let source = self.broadcast.subscribe();
        let (initial_sessions, initial_sequence) = {
            let state = self.state.lock().await;
            let mut sessions = state.sessions.values().cloned().collect::<Vec<_>>();
            sessions.sort_by_key(|session| session.started_at);
            (sessions, state.next_sequence)
        };
        let receiver = Self::spawn_subscription(source, Some(initial_sequence));
        (initial_sessions, receiver)
    }

    pub async fn list_active(&self) -> Vec<LibraryScanSession> {
        let state = self.state.lock().await;
        let mut sessions = state.sessions.values().cloned().collect::<Vec<_>>();
        sessions.sort_by_key(|session| session.started_at);
        sessions
    }

    pub async fn active_facets(&self) -> Vec<MediaFacet> {
        let state = self.state.lock().await;
        let mut facets = state
            .sessions
            .values()
            .map(|session| session.facet.clone())
            .collect::<Vec<_>>();
        facets.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        facets.dedup();
        facets
    }

    /// Canonical gate for background workers that should yield while any
    /// library scan is active instead of open-coding their own polling loops.
    pub async fn wait_until_idle(&self) {
        let mut receiver = self.subscribe();

        loop {
            if self.list_active().await.is_empty() {
                return;
            }

            match receiver.recv().await {
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(
                        "library_scan_progress: idle waiter lagged, skipped {n} messages"
                    );
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    pub async fn wait_for_active_facets_change(&self, current_facets: &[MediaFacet]) {
        let mut receiver = self.subscribe();

        loop {
            if self.active_facets().await != current_facets {
                return;
            }

            match receiver.recv().await {
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(
                        "library_scan_progress: active facet waiter lagged, skipped {n} messages"
                    );
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    pub async fn wait_for_session_to_clear(&self, session_id: &str) {
        let mut receiver = self.subscribe();

        loop {
            let is_active = {
                let state = self.state.lock().await;
                state.sessions.contains_key(session_id)
            };

            if !is_active {
                return;
            }

            match receiver.recv().await {
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(
                        "library_scan_progress: session waiter lagged, skipped {n} messages"
                    );
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn start_session(&self, facet: MediaFacet) -> AppResult<LibraryScanSession> {
        self.start_session_with_id_for_library(Id::new().0, facet, None, LibraryScanMode::Full)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn start_session_with_id(
        &self,
        session_id: String,
        facet: MediaFacet,
        mode: LibraryScanMode,
    ) -> AppResult<LibraryScanSession> {
        self.start_session_with_id_for_library(session_id, facet, None, mode)
            .await
    }

    pub(crate) async fn start_session_with_id_for_library(
        &self,
        session_id: String,
        facet: MediaFacet,
        library_id: Option<String>,
        mode: LibraryScanMode,
    ) -> AppResult<LibraryScanSession> {
        let event =
            {
                let mut state = self.state.lock().await;
                if state.sessions.contains_key(&session_id) {
                    return Err(AppError::Validation(format!(
                        "library scan session {session_id} is already running"
                    )));
                }
                if state.sessions.values().any(|active| {
                    library_scan_scopes_conflict(active, &facet, library_id.as_deref())
                }) {
                    return Err(AppError::Validation(format!(
                        "{} library scan already running",
                        facet.as_str()
                    )));
                }

                let snapshot = LibraryScanSession::with_id_for_library(
                    session_id,
                    facet.clone(),
                    library_id,
                    mode,
                );
                state
                    .sessions
                    .insert(snapshot.session_id.clone(), snapshot.clone());
                // A session id is never reused, but never let a stale record
                // from an abandoned session leak into a new one.
                state.pending_deltas.remove(&snapshot.session_id);
                state.next_sequence = state.next_sequence.saturating_add(1);
                LibraryScanTrackerEvent {
                    sequence: state.next_sequence,
                    session: snapshot,
                }
            };
        self.notify_event(event.clone()).await;
        Ok(event.session)
    }

    pub(crate) async fn has_conflicting_session(
        &self,
        facet: &MediaFacet,
        library_id: Option<&str>,
    ) -> bool {
        let state = self.state.lock().await;
        state
            .sessions
            .values()
            .any(|active| library_scan_scopes_conflict(active, facet, library_id))
    }

    /// Applies `delta` to the live session and stages it for the event log.
    ///
    /// The live session - and therefore every GraphQL progress subscriber -
    /// sees the delta immediately, exactly as [`Self::apply_delta`] would.
    /// What changes is the persisted log: a run of per-title counter deltas is
    /// coalesced into one record, so `to_persist` is usually empty and the run
    /// is written by the next [`Self::take_pending_delta`] flush.
    pub(crate) async fn apply_and_stage_delta(
        &self,
        session_id: &str,
        delta: LibraryScanDeltaRecordedEventData,
    ) -> Option<StagedLibraryScanDelta> {
        let (event, to_persist) = {
            let mut state = self.state.lock().await;
            let snapshot = {
                let session = state.sessions.get_mut(session_id)?;
                apply_library_scan_delta_fields(session, &delta);
                session.updated_at = Utc::now();
                session.clone()
            };
            let to_persist = stage_library_scan_delta(
                &mut state.pending_deltas,
                session_id,
                &snapshot.facet,
                delta,
            );
            state.next_sequence = state.next_sequence.saturating_add(1);
            (
                LibraryScanTrackerEvent {
                    sequence: state.next_sequence,
                    session: snapshot,
                },
                to_persist,
            )
        };
        self.notify_event(event.clone()).await;
        Some(StagedLibraryScanDelta {
            session: event.session,
            to_persist,
        })
    }

    /// Takes the session's coalesced delta, if any, so the caller can append
    /// it before the next progress/terminal event.
    pub(crate) async fn take_pending_delta(
        &self,
        session_id: &str,
    ) -> Option<(LibraryScanDeltaRecordedEventData, MediaFacet)> {
        let mut state = self.state.lock().await;
        state
            .pending_deltas
            .remove(session_id)
            .map(|pending| (pending.data, pending.facet))
    }

    /// Per-delta append path, kept for the projection tests that assert the
    /// live session folds one delta at a time. Production records go through
    /// [`Self::apply_and_stage_delta`].
    #[cfg(test)]
    pub(crate) async fn apply_delta(
        &self,
        session_id: &str,
        delta: &LibraryScanDeltaRecordedEventData,
    ) -> Option<LibraryScanSession> {
        let event = {
            let mut state = self.state.lock().await;
            let session = state.sessions.get_mut(session_id)?;
            apply_library_scan_delta_fields(session, delta);
            session.updated_at = Utc::now();
            let snapshot = session.clone();
            state.next_sequence = state.next_sequence.saturating_add(1);
            LibraryScanTrackerEvent {
                sequence: state.next_sequence,
                session: snapshot,
            }
        };
        self.notify_event(event.clone()).await;
        Some(event.session)
    }

    #[cfg(test)]
    pub(crate) async fn add_found_titles(
        &self,
        session_id: &str,
        additional: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.found_titles = session.found_titles.saturating_add(additional);
            if matches!(session.status, LibraryScanStatus::Discovering) {
                session.status = LibraryScanStatus::Running;
            }
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn set_title_match_total(
        &self,
        session_id: &str,
        total: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.title_match_progress.total = total;
            if matches!(session.status, LibraryScanStatus::Discovering) {
                session.status = LibraryScanStatus::Running;
            }
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn mark_title_match_total_known(
        &self,
        session_id: &str,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.title_match_total_known = true;
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn add_metadata_total(
        &self,
        session_id: &str,
        additional: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.metadata_progress.add_total(additional);
            if matches!(session.status, LibraryScanStatus::Discovering) {
                session.status = LibraryScanStatus::Running;
            }
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn mark_metadata_total_known(
        &self,
        session_id: &str,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.metadata_total_known = true;
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn increment_title_match_completed(
        &self,
        session_id: &str,
        additional: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.title_match_progress.mark_completed(additional);
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn set_summary(
        &self,
        session_id: &str,
        summary: LibraryScanSummary,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.summary = Some(summary.clone());
        })
        .await
    }

    pub(crate) async fn set_warning_message(
        &self,
        session_id: &str,
        warning_message: Option<String>,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.warning_message = warning_message.clone();
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn apply_summary_delta(
        &self,
        session_id: &str,
        delta: LibraryScanSummary,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            let summary = session
                .summary
                .get_or_insert_with(LibraryScanSummary::default);
            summary.absorb(&delta);
        })
        .await
    }

    pub(crate) async fn complete_if_finished(
        &self,
        session_id: &str,
    ) -> Option<LibraryScanSession> {
        let event = {
            let mut state = self.state.lock().await;
            let session = state.sessions.get(session_id)?;
            if session.summary.is_none()
                || !session.title_match_progress.is_finished()
                || !session.metadata_progress.is_finished()
                || !session.file_progress.is_finished()
            {
                return None;
            }

            let mut session = state.sessions.remove(session_id)?;
            // The completion path flushes before it gets here; this only keeps
            // an unflushed record from outliving its session.
            state.pending_deltas.remove(session_id);
            session.updated_at = Utc::now();
            session.title_match_total_known = true;
            session.metadata_total_known = true;
            session.file_total_known = true;
            session.status = if session.title_match_progress.failed > 0
                || session.metadata_progress.failed > 0
                || session.file_progress.failed > 0
                || session.warning_message.is_some()
            {
                LibraryScanStatus::Warning
            } else {
                LibraryScanStatus::Completed
            };
            state.next_sequence = state.next_sequence.saturating_add(1);
            LibraryScanTrackerEvent {
                sequence: state.next_sequence,
                session,
            }
        };
        self.notify_event(event.clone()).await;
        Some(event.session)
    }

    pub(crate) async fn fail_session(&self, session_id: &str) -> Option<LibraryScanSession> {
        let event = {
            let mut state = self.state.lock().await;
            let mut session = state.sessions.remove(session_id)?;
            session.updated_at = Utc::now();
            session.title_match_total_known = true;
            session.metadata_total_known = true;
            session.file_total_known = true;
            state.pending_deltas.remove(session_id);
            session.status = LibraryScanStatus::Failed;
            state.next_sequence = state.next_sequence.saturating_add(1);
            LibraryScanTrackerEvent {
                sequence: state.next_sequence,
                session,
            }
        };
        self.notify_event(event.clone()).await;
        Some(event.session)
    }

    pub(crate) async fn cancel_session(&self, session_id: &str) -> Option<LibraryScanSession> {
        let event = {
            let mut state = self.state.lock().await;
            let mut session = state.sessions.remove(session_id)?;
            session.updated_at = Utc::now();
            session.title_match_total_known = true;
            session.metadata_total_known = true;
            session.file_total_known = true;
            state.pending_deltas.remove(session_id);
            session.status = LibraryScanStatus::Canceled;
            state.next_sequence = state.next_sequence.saturating_add(1);
            LibraryScanTrackerEvent {
                sequence: state.next_sequence,
                session,
            }
        };
        self.notify_event(event.clone()).await;
        Some(event.session)
    }

    pub(crate) async fn get_session(&self, session_id: &str) -> Option<LibraryScanSession> {
        let state = self.state.lock().await;
        state.sessions.get(session_id).cloned()
    }

    async fn update_session(
        &self,
        session_id: &str,
        mutator: impl FnOnce(&mut LibraryScanSession),
    ) -> Option<LibraryScanSession> {
        let event = {
            let mut state = self.state.lock().await;
            let session = state.sessions.get_mut(session_id)?;
            mutator(session);
            session.updated_at = Utc::now();
            let snapshot = session.clone();
            state.next_sequence = state.next_sequence.saturating_add(1);
            LibraryScanTrackerEvent {
                sequence: state.next_sequence,
                session: snapshot,
            }
        };
        self.notify_event(event.clone()).await;
        Some(event.session)
    }

    async fn notify_event(&self, event: LibraryScanTrackerEvent) {
        let _ = self.broadcast.send(event.clone());
        if let Some(tracker) = self.job_run_tracker.lock().await.clone() {
            tracker.merge_library_scan_progress(event.session).await;
        }
    }
}

pub fn replay_library_scan_projection(
    events: &[DomainEvent],
) -> HashMap<String, LibraryScanSession> {
    let mut sessions = HashMap::new();
    for event in events {
        reduce_library_scan_projection_event(&mut sessions, event);
    }
    sessions
}

pub fn reduce_library_scan_projection_event(
    sessions: &mut HashMap<String, LibraryScanSession>,
    event: &DomainEvent,
) -> Option<LibraryScanSession> {
    match &event.payload {
        DomainEventPayload::LibraryScanStarted(data) => {
            let session = library_scan_session_from_started(data, event);
            sessions.insert(data.session_id.clone(), session.clone());
            trace_session_snapshot("started", &session);
            Some(session)
        }
        DomainEventPayload::LibraryScanTitleDiscovered(data) => {
            let session = sessions
                .entry(data.session_id.clone())
                .or_insert_with(|| library_scan_session_from_title_discovered(data, event));
            session.updated_at = event.occurred_at;
            session.facet = data.facet.clone();
            if matches!(session.status, LibraryScanStatus::Discovering) {
                session.status = LibraryScanStatus::Running;
            }
            trace!(
                reason = "title_discovered",
                session_id = %session.session_id,
                title_id = %data.title_id,
                title_name = %data.title_name,
                discovered_file_count = data.discovered_file_count,
                "library scan projection title discovered"
            );
            trace_session_snapshot("title_discovered", session);
            Some(session.clone())
        }
        DomainEventPayload::LibraryScanDeltaRecorded(data) => {
            let session = sessions.get_mut(&data.session_id)?;
            apply_library_scan_delta_recorded(session, data, event);
            Some(session.clone())
        }
        DomainEventPayload::LibraryScanProgressed(data) => {
            let session = sessions
                .entry(data.session_id.clone())
                .or_insert_with(|| library_scan_session_from_progressed(data, event));
            apply_library_scan_progress(session, data, event);
            Some(session.clone())
        }
        DomainEventPayload::LibraryScanCompleted(data) => {
            let mut session = sessions
                .remove(&data.session_id)
                .unwrap_or_else(|| library_scan_session_from_completed(data, event));
            apply_library_scan_completed(&mut session, data, event);
            Some(session)
        }
        DomainEventPayload::LibraryScanCanceled(data) => {
            let mut session = sessions
                .remove(&data.session_id)
                .unwrap_or_else(|| library_scan_session_from_canceled(data, event));
            apply_library_scan_canceled(&mut session, data, event);
            Some(session)
        }
        DomainEventPayload::LibraryScanFailed(data) => {
            let mut session = sessions
                .remove(&data.session_id)
                .unwrap_or_else(|| library_scan_session_from_failed(data, event));
            session.updated_at = event.occurred_at;
            session.status = LibraryScanStatus::Failed;
            session.title_match_total_known = true;
            session.metadata_total_known = true;
            session.file_total_known = true;
            debug!(
                reason = "failed",
                session_id = %session.session_id,
                error_message = %data.error_message,
                "library scan projection marked session failed"
            );
            debug_session_snapshot("failed", &session);
            Some(session)
        }
        _ => None,
    }
}

fn library_scan_session_from_started(
    data: &LibraryScanStartedEventData,
    event: &DomainEvent,
) -> LibraryScanSession {
    LibraryScanSession {
        session_id: data.session_id.clone(),
        facet: event.facet.clone().unwrap_or(MediaFacet::Movie),
        library_id: data.library_id.clone(),
        mode: parse_library_scan_mode(&data.mode),
        status: LibraryScanStatus::Discovering,
        started_at: event.occurred_at,
        updated_at: event.occurred_at,
        found_titles: 0,
        title_match_total_known: false,
        metadata_total_known: false,
        file_total_known: false,
        title_match_progress: LibraryScanPhaseProgress::default(),
        metadata_progress: LibraryScanPhaseProgress::default(),
        file_progress: LibraryScanPhaseProgress::default(),
        summary: None,
        warning_message: None,
    }
}

fn library_scan_session_from_title_discovered(
    data: &LibraryScanTitleDiscoveredEventData,
    event: &DomainEvent,
) -> LibraryScanSession {
    LibraryScanSession {
        session_id: data.session_id.clone(),
        facet: data.facet.clone(),
        library_id: None,
        mode: LibraryScanMode::Full,
        status: LibraryScanStatus::Running,
        started_at: event.occurred_at,
        updated_at: event.occurred_at,
        found_titles: 0,
        title_match_total_known: false,
        metadata_total_known: false,
        file_total_known: false,
        title_match_progress: LibraryScanPhaseProgress::default(),
        metadata_progress: LibraryScanPhaseProgress::default(),
        file_progress: LibraryScanPhaseProgress::default(),
        summary: None,
        warning_message: None,
    }
}

fn library_scan_session_from_progressed(
    data: &LibraryScanProgressedEventData,
    event: &DomainEvent,
) -> LibraryScanSession {
    let mut session = LibraryScanSession {
        session_id: data.session_id.clone(),
        facet: event.facet.clone().unwrap_or(MediaFacet::Movie),
        library_id: None,
        mode: LibraryScanMode::Full,
        status: parse_library_scan_status(&data.status),
        started_at: event.occurred_at,
        updated_at: event.occurred_at,
        found_titles: 0,
        title_match_total_known: data.title_match_total_known,
        metadata_total_known: data.titles_total.is_some(),
        file_total_known: data.files_total.is_some(),
        title_match_progress: LibraryScanPhaseProgress::default(),
        metadata_progress: LibraryScanPhaseProgress::default(),
        file_progress: LibraryScanPhaseProgress::default(),
        summary: None,
        warning_message: None,
    };
    apply_library_scan_progress(&mut session, data, event);
    session
}

fn library_scan_session_from_completed(
    data: &LibraryScanCompletedEventData,
    event: &DomainEvent,
) -> LibraryScanSession {
    let mut session = LibraryScanSession {
        session_id: data.session_id.clone(),
        facet: event.facet.clone().unwrap_or(MediaFacet::Movie),
        library_id: None,
        mode: LibraryScanMode::Full,
        status: parse_library_scan_status(&data.status),
        started_at: event.occurred_at,
        updated_at: event.occurred_at,
        found_titles: data.found_titles.max(0) as usize,
        title_match_total_known: true,
        metadata_total_known: true,
        file_total_known: true,
        title_match_progress: LibraryScanPhaseProgress::default(),
        metadata_progress: LibraryScanPhaseProgress::default(),
        file_progress: LibraryScanPhaseProgress::default(),
        summary: None,
        warning_message: None,
    };
    apply_library_scan_completed(&mut session, data, event);
    session
}

fn library_scan_session_from_failed(
    data: &LibraryScanFailedEventData,
    event: &DomainEvent,
) -> LibraryScanSession {
    LibraryScanSession {
        session_id: data.session_id.clone(),
        facet: event.facet.clone().unwrap_or(MediaFacet::Movie),
        library_id: None,
        mode: LibraryScanMode::Full,
        status: LibraryScanStatus::Failed,
        started_at: event.occurred_at,
        updated_at: event.occurred_at,
        found_titles: 0,
        title_match_total_known: true,
        metadata_total_known: true,
        file_total_known: true,
        title_match_progress: LibraryScanPhaseProgress::default(),
        metadata_progress: LibraryScanPhaseProgress::default(),
        file_progress: LibraryScanPhaseProgress::default(),
        summary: None,
        warning_message: None,
    }
}

fn library_scan_session_from_canceled(
    data: &LibraryScanCanceledEventData,
    event: &DomainEvent,
) -> LibraryScanSession {
    let mut session = LibraryScanSession {
        session_id: data.session_id.clone(),
        facet: event.facet.clone().unwrap_or(MediaFacet::Movie),
        library_id: None,
        mode: LibraryScanMode::Full,
        status: LibraryScanStatus::Canceled,
        started_at: event.occurred_at,
        updated_at: event.occurred_at,
        found_titles: data.found_titles.max(0) as usize,
        title_match_total_known: true,
        metadata_total_known: true,
        file_total_known: true,
        title_match_progress: LibraryScanPhaseProgress::default(),
        metadata_progress: LibraryScanPhaseProgress::default(),
        file_progress: LibraryScanPhaseProgress::default(),
        summary: None,
        warning_message: None,
    };
    apply_library_scan_canceled(&mut session, data, event);
    session
}

fn apply_library_scan_progress(
    session: &mut LibraryScanSession,
    data: &LibraryScanProgressedEventData,
    event: &DomainEvent,
) {
    session.updated_at = event.occurred_at;
    session.status = parse_library_scan_status(&data.status);
    session.found_titles = data.found_titles.max(0) as usize;
    session.title_match_progress.total = data.found_titles.max(0) as usize;
    session.title_match_total_known = data.title_match_total_known;
    session.title_match_progress.completed =
        title_match_completed_from_event(data.found_titles, data.title_match_completed, false);
    if let Some(total) = data.titles_total {
        session.metadata_progress.total = total as usize;
        session.metadata_total_known = true;
    }
    session.metadata_progress.completed = data.titles_completed.max(0) as usize;
    if let Some(total) = data.files_total {
        session.file_progress.total = total as usize;
        session.file_total_known = true;
    }
    session.file_progress.completed = data.files_completed.max(0) as usize;
    session.warning_message = data.warning_message.clone();

    trace!(
        reason = "progressed",
        session_id = %session.session_id,
        status = %data.status,
        found_titles = data.found_titles,
        title_match_completed = data.title_match_completed,
        title_match_total_known = data.title_match_total_known,
        titles_completed = data.titles_completed,
        titles_total = ?data.titles_total,
        files_completed = data.files_completed,
        files_total = ?data.files_total,
        warning_message = ?data.warning_message,
        occurred_at = %event.occurred_at,
        "library scan projection applied progressed event"
    );
    trace_session_snapshot("progressed", session);
}

fn apply_library_scan_delta_recorded(
    session: &mut LibraryScanSession,
    data: &LibraryScanDeltaRecordedEventData,
    event: &DomainEvent,
) {
    session.updated_at = event.occurred_at;

    apply_library_scan_delta_fields(session, data);

    trace!(
        reason = "delta_recorded",
        session_id = %session.session_id,
        found_titles_total = ?data.found_titles_total,
        found_titles_delta = data.found_titles_delta,
        title_match_completed_delta = data.title_match_completed_delta,
        title_match_failed_delta = data.title_match_failed_delta,
        title_match_total_known = ?data.title_match_total_known,
        metadata_total_delta = data.metadata_total_delta,
        metadata_completed_delta = data.metadata_completed_delta,
        metadata_failed_delta = data.metadata_failed_delta,
        metadata_total_known = ?data.metadata_total_known,
        file_total_delta = data.file_total_delta,
        file_completed_delta = data.file_completed_delta,
        file_failed_delta = data.file_failed_delta,
        file_total_known = ?data.file_total_known,
        summary_present = data.summary.is_some(),
        summary_is_delta = data.summary_is_delta,
        occurred_at = %event.occurred_at,
        "library scan projection applied delta event"
    );
    trace_session_snapshot("delta_recorded", session);
}

fn apply_library_scan_delta_fields(
    session: &mut LibraryScanSession,
    data: &LibraryScanDeltaRecordedEventData,
) {
    if let Some(found_titles) = data.found_titles_total {
        session.found_titles = non_negative_usize(found_titles);
    } else if data.found_titles_delta != 0 {
        apply_signed_delta(&mut session.found_titles, data.found_titles_delta);
    }

    session.title_match_progress.total = session.found_titles;

    if let Some(total_known) = data.title_match_total_known {
        session.title_match_total_known = total_known;
    }
    if data.title_match_completed_delta > 0 {
        session
            .title_match_progress
            .mark_completed(data.title_match_completed_delta as usize);
    }
    if data.title_match_failed_delta > 0 {
        session
            .title_match_progress
            .mark_failed(data.title_match_failed_delta as usize);
    }

    if data.metadata_total_delta > 0 {
        session
            .metadata_progress
            .add_total(data.metadata_total_delta as usize);
    }
    if let Some(total_known) = data.metadata_total_known {
        session.metadata_total_known = total_known;
    }
    if data.metadata_completed_delta > 0 {
        session
            .metadata_progress
            .mark_completed(data.metadata_completed_delta as usize);
    }
    if data.metadata_failed_delta > 0 {
        session
            .metadata_progress
            .mark_failed(data.metadata_failed_delta as usize);
    }

    if data.file_total_delta > 0 && !session.file_total_known {
        session
            .file_progress
            .add_total(data.file_total_delta as usize);
    }
    if let Some(total_known) = data.file_total_known {
        session.file_total_known = total_known;
    }
    if data.file_completed_delta > 0 {
        session
            .file_progress
            .mark_completed(data.file_completed_delta as usize);
    }
    if data.file_failed_delta > 0 {
        session
            .file_progress
            .mark_failed(data.file_failed_delta as usize);
    }

    if let Some(summary) = data.summary.as_ref() {
        let summary = LibraryScanSummary {
            scanned: non_negative_usize(summary.scanned),
            matched: non_negative_usize(summary.matched),
            imported: non_negative_usize(summary.imported),
            skipped: non_negative_usize(summary.skipped),
            unmatched: non_negative_usize(summary.unmatched),
        };
        if data.summary_is_delta {
            session
                .summary
                .get_or_insert_with(LibraryScanSummary::default)
                .absorb(&summary);
        } else {
            session.summary = Some(summary);
        }
    }

    if matches!(session.status, LibraryScanStatus::Discovering) {
        session.status = LibraryScanStatus::Running;
    }
}

fn apply_library_scan_completed(
    session: &mut LibraryScanSession,
    data: &LibraryScanCompletedEventData,
    event: &DomainEvent,
) {
    session.updated_at = event.occurred_at;
    session.status = parse_library_scan_status(&data.status);
    session.found_titles = data.found_titles.max(0) as usize;
    session.title_match_total_known = true;
    session.title_match_progress.total = data.found_titles.max(0) as usize;
    session.title_match_progress.completed =
        title_match_completed_from_event(data.found_titles, data.title_match_completed, true);
    session.metadata_total_known = true;
    session.file_total_known = true;
    if let Some(total) = data.titles_total {
        session.metadata_progress.total = total as usize;
    }
    session.metadata_progress.completed = data.titles_completed.max(0) as usize;
    if let Some(total) = data.files_total {
        session.file_progress.total = total as usize;
    }
    session.file_progress.completed = data.files_completed.max(0) as usize;
    session.summary = data.summary.as_ref().map(|summary| LibraryScanSummary {
        scanned: summary.scanned.max(0) as usize,
        matched: summary.matched.max(0) as usize,
        imported: summary.imported.max(0) as usize,
        skipped: summary.skipped.max(0) as usize,
        unmatched: summary.unmatched.max(0) as usize,
    });
    session.warning_message = data.warning_message.clone();

    trace!(
        reason = "completed",
        session_id = %session.session_id,
        status = %data.status,
        found_titles = data.found_titles,
        title_match_completed = data.title_match_completed,
        titles_completed = data.titles_completed,
        titles_total = ?data.titles_total,
        files_completed = data.files_completed,
        files_total = ?data.files_total,
        summary_present = data.summary.is_some(),
        warning_message = ?data.warning_message,
        occurred_at = %event.occurred_at,
        "library scan projection applied completed event"
    );
    debug_session_snapshot("completed", session);
}

fn apply_library_scan_canceled(
    session: &mut LibraryScanSession,
    data: &LibraryScanCanceledEventData,
    event: &DomainEvent,
) {
    session.updated_at = event.occurred_at;
    session.status = LibraryScanStatus::Canceled;
    session.found_titles = data.found_titles.max(0) as usize;
    session.title_match_total_known = true;
    session.title_match_progress.total = data.found_titles.max(0) as usize;
    session.title_match_progress.completed =
        title_match_completed_from_event(data.found_titles, data.title_match_completed, true);
    session.metadata_total_known = true;
    session.file_total_known = true;
    if let Some(total) = data.titles_total {
        session.metadata_progress.total = total.max(0) as usize;
    }
    session.metadata_progress.completed = data.titles_completed.max(0) as usize;
    if let Some(total) = data.files_total {
        session.file_progress.total = total.max(0) as usize;
    }
    session.file_progress.completed = data.files_completed.max(0) as usize;
    if let Some(summary) = data.summary.as_ref() {
        session.summary = Some(LibraryScanSummary {
            scanned: non_negative_usize(summary.scanned),
            matched: non_negative_usize(summary.matched),
            imported: non_negative_usize(summary.imported),
            skipped: non_negative_usize(summary.skipped),
            unmatched: non_negative_usize(summary.unmatched),
        });
    }

    debug!(
        reason = "canceled",
        session_id = %session.session_id,
        found_titles = data.found_titles,
        title_match_completed = data.title_match_completed,
        titles_completed = data.titles_completed,
        titles_total = ?data.titles_total,
        files_completed = data.files_completed,
        files_total = ?data.files_total,
        occurred_at = %event.occurred_at,
        "library scan projection marked session canceled"
    );
    debug_session_snapshot("canceled", session);
}

fn debug_session_snapshot(reason: &str, session: &LibraryScanSession) {
    debug!(
        reason = reason,
        session_id = %session.session_id,
        facet = %session.facet.as_str(),
        status = %session.status.as_str(),
        found_titles = session.found_titles,
        title_match_total_known = session.title_match_total_known,
        title_match_total = session.title_match_progress.total,
        title_match_completed = session.title_match_progress.completed,
        title_match_failed = session.title_match_progress.failed,
        metadata_total_known = session.metadata_total_known,
        metadata_total = session.metadata_progress.total,
        metadata_completed = session.metadata_progress.completed,
        metadata_failed = session.metadata_progress.failed,
        file_total_known = session.file_total_known,
        file_total = session.file_progress.total,
        file_completed = session.file_progress.completed,
        file_failed = session.file_progress.failed,
        summary_present = session.summary.is_some(),
        ready_to_complete = session.is_ready_to_complete(),
        "library scan projection snapshot"
    );
}

fn trace_session_snapshot(reason: &str, session: &LibraryScanSession) {
    trace!(
        reason = reason,
        session_id = %session.session_id,
        facet = %session.facet.as_str(),
        status = %session.status.as_str(),
        found_titles = session.found_titles,
        title_match_total_known = session.title_match_total_known,
        title_match_total = session.title_match_progress.total,
        title_match_completed = session.title_match_progress.completed,
        title_match_failed = session.title_match_progress.failed,
        metadata_total_known = session.metadata_total_known,
        metadata_total = session.metadata_progress.total,
        metadata_completed = session.metadata_progress.completed,
        metadata_failed = session.metadata_progress.failed,
        file_total_known = session.file_total_known,
        file_total = session.file_progress.total,
        file_completed = session.file_progress.completed,
        file_failed = session.file_progress.failed,
        summary_present = session.summary.is_some(),
        ready_to_complete = session.is_ready_to_complete(),
        "library scan projection snapshot"
    );
}

fn title_match_completed_from_event(
    found_titles: i64,
    title_match_completed: i64,
    completed_event: bool,
) -> usize {
    if completed_event && title_match_completed <= 0 {
        return found_titles.max(0) as usize;
    }

    title_match_completed.max(0) as usize
}

fn parse_library_scan_status(value: &str) -> LibraryScanStatus {
    match value {
        "discovering" => LibraryScanStatus::Discovering,
        "running" => LibraryScanStatus::Running,
        "canceled" => LibraryScanStatus::Canceled,
        "warning" => LibraryScanStatus::Warning,
        "failed" => LibraryScanStatus::Failed,
        _ => LibraryScanStatus::Completed,
    }
}

fn parse_library_scan_mode(value: &str) -> LibraryScanMode {
    match value {
        "additive" => LibraryScanMode::Additive,
        _ => LibraryScanMode::Full,
    }
}

fn non_negative_usize(value: i64) -> usize {
    value.max(0) as usize
}

fn apply_signed_delta(target: &mut usize, delta: i64) {
    if delta >= 0 {
        *target = target.saturating_add(delta as usize);
    } else {
        *target = target.saturating_sub(delta.unsigned_abs() as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::{
        DomainEventStream, LibraryScanDeltaRecordedEventData, LibraryScanSummaryEventData,
    };

    fn test_library_scan_event(
        sequence: i64,
        session_id: &str,
        facet: MediaFacet,
        payload: DomainEventPayload,
    ) -> DomainEvent {
        DomainEvent {
            sequence,
            event_id: format!("event-{sequence}"),
            occurred_at: Utc::now(),
            actor_kind: scryer_domain::DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: None,
            facet: Some(facet),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::LibraryScan {
                session_id: session_id.to_string(),
            },
            payload,
        }
    }

    #[tokio::test]
    async fn start_session_rejects_duplicate_facet() {
        let tracker = LibraryScanTracker::new();

        let first = tracker
            .start_session(MediaFacet::Movie)
            .await
            .expect("start first session");
        let err = tracker
            .start_session(MediaFacet::Movie)
            .await
            .expect_err("reject duplicate movie scan");

        assert!(matches!(err, AppError::Validation(_)));
        assert_eq!(first.facet, MediaFacet::Movie);
    }

    #[tokio::test]
    async fn start_session_allows_distinct_libraries_in_same_facet() {
        let tracker = LibraryScanTracker::new();

        let first = tracker
            .start_session_with_id_for_library(
                "movie-scan-a".into(),
                MediaFacet::Movie,
                Some("movie-library-a".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start first movie library scan");
        let second = tracker
            .start_session_with_id_for_library(
                "movie-scan-b".into(),
                MediaFacet::Movie,
                Some("movie-library-b".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start second movie library scan");
        let duplicate = tracker
            .start_session_with_id_for_library(
                "movie-scan-a-duplicate".into(),
                MediaFacet::Movie,
                Some("movie-library-a".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect_err("reject duplicate scan for the same library");

        assert!(matches!(duplicate, AppError::Validation(_)));
        assert_eq!(first.library_id.as_deref(), Some("movie-library-a"));
        assert_eq!(second.library_id.as_deref(), Some("movie-library-b"));
        assert_eq!(tracker.list_active().await.len(), 2);
    }

    #[tokio::test]
    async fn duplicate_session_id_does_not_replace_an_unrelated_active_scan() {
        let tracker = LibraryScanTracker::new();
        let first = tracker
            .start_session_with_id_for_library(
                "shared-session-id".into(),
                MediaFacet::Movie,
                Some("movie-library-a".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start first scan");

        let duplicate_id = tracker
            .start_session_with_id_for_library(
                "shared-session-id".into(),
                MediaFacet::Series,
                Some("series-library-a".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect_err("reject a reused active session id");

        assert!(matches!(duplicate_id, AppError::Validation(_)));
        assert_eq!(tracker.list_active().await, vec![first]);
        assert!(
            tracker
                .has_conflicting_session(&MediaFacet::Movie, Some("movie-library-a"))
                .await
        );
    }

    #[tokio::test]
    async fn unscoped_facet_session_conflicts_with_scoped_sessions_in_both_start_orders() {
        let unscoped_first = LibraryScanTracker::new();
        unscoped_first
            .start_session(MediaFacet::Movie)
            .await
            .expect("start unscoped movie scan");
        let scoped_error = unscoped_first
            .start_session_with_id_for_library(
                "scoped-after-unscoped".into(),
                MediaFacet::Movie,
                Some("movie-library-a".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect_err("unscoped scan should block scoped scan");

        let scoped_first = LibraryScanTracker::new();
        scoped_first
            .start_session_with_id_for_library(
                "scoped-before-unscoped".into(),
                MediaFacet::Movie,
                Some("movie-library-a".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start scoped movie scan");
        let unscoped_error = scoped_first
            .start_session(MediaFacet::Movie)
            .await
            .expect_err("scoped scan should block unscoped scan");

        assert!(matches!(scoped_error, AppError::Validation(_)));
        assert!(matches!(unscoped_error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn terminal_sessions_release_only_their_own_library_scope() {
        let tracker = LibraryScanTracker::new();
        let completed = tracker
            .start_session_with_id_for_library(
                "completed-scan".into(),
                MediaFacet::Movie,
                Some("completed-library".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start scan to complete");
        let failed = tracker
            .start_session_with_id_for_library(
                "failed-scan".into(),
                MediaFacet::Movie,
                Some("failed-library".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start scan to fail");
        let canceled = tracker
            .start_session_with_id_for_library(
                "canceled-scan".into(),
                MediaFacet::Movie,
                Some("canceled-library".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start scan to cancel");

        tracker
            .set_summary(&completed.session_id, LibraryScanSummary::default())
            .await;
        tracker
            .complete_if_finished(&completed.session_id)
            .await
            .expect("complete first scan");
        tracker
            .fail_session(&failed.session_id)
            .await
            .expect("fail second scan");
        tracker
            .cancel_session(&canceled.session_id)
            .await
            .expect("cancel third scan");

        for library_id in ["completed-library", "failed-library", "canceled-library"] {
            tracker
                .start_session_with_id_for_library(
                    format!("replacement-{library_id}"),
                    MediaFacet::Movie,
                    Some(library_id.to_string()),
                    LibraryScanMode::Full,
                )
                .await
                .expect("terminal scan should release its library scope");
        }
        let still_active_error = tracker
            .start_session_with_id_for_library(
                "duplicate-replacement".into(),
                MediaFacet::Movie,
                Some("completed-library".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect_err("other active replacement should keep its scope locked");
        assert!(matches!(still_active_error, AppError::Validation(_)));
    }

    // The paused clock fires the coalescing flush timer only once every task
    // is idle, so both events are always in the same window on any runner.
    #[tokio::test(start_paused = true)]
    async fn subscription_coalesces_concurrent_sessions_independently() {
        let tracker = LibraryScanTracker::new();
        let mut receiver = tracker.subscribe();
        let first = tracker
            .start_session_with_id_for_library(
                "subscription-scan-a".into(),
                MediaFacet::Movie,
                Some("subscription-library-a".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start first subscribed scan");
        let second = tracker
            .start_session_with_id_for_library(
                "subscription-scan-b".into(),
                MediaFacet::Movie,
                Some("subscription-library-b".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start second subscribed scan");
        tracker.add_found_titles(&first.session_id, 3).await;
        tracker.add_found_titles(&second.session_id, 7).await;

        let mut received = Vec::new();
        for _ in 0..2 {
            received.push(
                crate::test_wait::within_deadline("a coalesced session", receiver.recv())
                    .await
                    .expect("subscription remains open"),
            );
        }
        received.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        assert_eq!(received[0].session_id, first.session_id);
        assert_eq!(received[0].found_titles, 3);
        assert_eq!(received[1].session_id, second.session_id);
        assert_eq!(received[1].found_titles, 7);
    }

    // The paused clock fires the coalescing flush timer only once every task
    // is idle, so both events are always in the same window on any runner.
    #[tokio::test(start_paused = true)]
    async fn terminal_event_does_not_discard_another_sessions_pending_progress() {
        let tracker = LibraryScanTracker::new();
        let mut receiver = tracker.subscribe();
        let first = tracker
            .start_session_with_id_for_library(
                "terminal-scan-a".into(),
                MediaFacet::Movie,
                Some("terminal-library-a".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start terminal scan");
        let second = tracker
            .start_session_with_id_for_library(
                "pending-scan-b".into(),
                MediaFacet::Movie,
                Some("pending-library-b".into()),
                LibraryScanMode::Full,
            )
            .await
            .expect("start pending scan");
        tracker.add_found_titles(&second.session_id, 11).await;
        tracker.fail_session(&first.session_id).await;

        let terminal = crate::test_wait::within_deadline("the terminal session", receiver.recv())
            .await
            .expect("subscription remains open");
        let pending =
            crate::test_wait::within_deadline("the other pending session", receiver.recv())
                .await
                .expect("subscription remains open");

        assert_eq!(terminal.session_id, first.session_id);
        assert_eq!(terminal.status, LibraryScanStatus::Failed);
        assert_eq!(pending.session_id, second.session_id);
        assert_eq!(pending.found_titles, 11);
    }

    #[tokio::test]
    async fn add_found_titles_accumulates_and_starts_running() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Movie)
            .await
            .expect("start session");

        let first = tracker
            .add_found_titles(&session.session_id, 10)
            .await
            .expect("add first batch");
        assert_eq!(first.found_titles, 10);
        assert_eq!(first.status, LibraryScanStatus::Running);

        let second = tracker
            .add_found_titles(&session.session_id, 90)
            .await
            .expect("add second batch");
        assert_eq!(second.found_titles, 100);
        assert_eq!(second.status, LibraryScanStatus::Running);
    }

    #[tokio::test]
    async fn add_metadata_total_keeps_total_indeterminate_until_marked_known() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Movie)
            .await
            .expect("start session");

        let snapshot = tracker
            .add_metadata_total(&session.session_id, 2)
            .await
            .expect("add metadata total");

        assert!(!snapshot.metadata_total_known);
        assert_eq!(snapshot.metadata_progress.total, 2);
        assert_eq!(snapshot.status, LibraryScanStatus::Running);

        let snapshot = tracker
            .mark_metadata_total_known(&session.session_id)
            .await
            .expect("mark metadata total known");

        assert!(snapshot.metadata_total_known);
    }

    #[tokio::test]
    async fn subscribe_with_initial_snapshot_filters_preexisting_buffered_events() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Movie)
            .await
            .expect("start session");

        let (initial_sessions, mut receiver) = tracker.subscribe_with_initial_snapshot().await;

        assert_eq!(initial_sessions.len(), 1);
        assert_eq!(initial_sessions[0].session_id, session.session_id);
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn title_match_progress_can_be_tracked_independently() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Series)
            .await
            .expect("start session");

        let snapshot = tracker
            .set_title_match_total(&session.session_id, 4)
            .await
            .expect("set title match total");
        assert_eq!(snapshot.title_match_progress.total, 4);
        assert!(!snapshot.title_match_total_known);

        let snapshot = tracker
            .mark_title_match_total_known(&session.session_id)
            .await
            .expect("mark title match known");
        assert!(snapshot.title_match_total_known);

        let snapshot = tracker
            .increment_title_match_completed(&session.session_id, 3)
            .await
            .expect("increment title match completed");
        assert_eq!(snapshot.title_match_progress.completed, 3);
    }

    #[tokio::test]
    async fn wait_until_idle_returns_immediately_without_sessions() {
        let tracker = LibraryScanTracker::new();

        let mut waiter = Box::pin(tokio::task::unconstrained(tracker.wait_until_idle()));
        assert!(
            futures_util::poll!(waiter.as_mut()).is_ready(),
            "idle tracker should resolve on its first poll"
        );
    }

    #[tokio::test]
    async fn wait_until_idle_blocks_until_terminal_session() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Anime)
            .await
            .expect("start session");

        // One unconstrained poll runs the waiter through its active-session
        // check to its park point, so Pending proves it saw the scan and
        // blocked rather than merely not having been scheduled yet.
        let mut waiter = Box::pin(tokio::task::unconstrained(tracker.wait_until_idle()));
        assert!(
            futures_util::poll!(waiter.as_mut()).is_pending(),
            "waiter should block while scan is active"
        );

        tracker
            .fail_session(&session.session_id)
            .await
            .expect("session should fail");

        crate::test_wait::within_deadline("the idle waiter to resolve after the scan ends", waiter)
            .await;
    }

    #[tokio::test]
    async fn apply_summary_delta_merges_into_existing_summary() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Movie)
            .await
            .expect("start session");

        tracker
            .set_summary(
                &session.session_id,
                LibraryScanSummary {
                    scanned: 2,
                    matched: 1,
                    imported: 1,
                    skipped: 0,
                    unmatched: 1,
                },
            )
            .await
            .expect("set base summary");

        let snapshot = tracker
            .apply_summary_delta(
                &session.session_id,
                LibraryScanSummary {
                    scanned: 3,
                    matched: 2,
                    imported: 2,
                    skipped: 1,
                    unmatched: 0,
                },
            )
            .await
            .expect("summary delta should apply");

        assert_eq!(
            snapshot.summary,
            Some(LibraryScanSummary {
                scanned: 5,
                matched: 3,
                imported: 3,
                skipped: 1,
                unmatched: 1,
            })
        );
    }

    #[tokio::test]
    async fn complete_if_finished_marks_warning_when_warning_message_is_present() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Series)
            .await
            .expect("start session");

        tracker
            .update_session(&session.session_id, |session| {
                session.title_match_total_known = true;
                session.title_match_progress.total = 2;
                session.title_match_progress.completed = 2;
                session.metadata_total_known = true;
                session.metadata_progress.total = 2;
                session.metadata_progress.completed = 2;
                session.file_total_known = true;
                session.file_progress.total = 2;
                session.file_progress.completed = 2;
                session.summary = Some(LibraryScanSummary {
                    scanned: 2,
                    matched: 2,
                    imported: 1,
                    skipped: 0,
                    unmatched: 0,
                });
                session.warning_message = Some(
                    "Imported Sonarr/Radarr monitored state could not be applied after this scan."
                        .to_string(),
                );
            })
            .await
            .expect("update session");

        let snapshot = tracker
            .complete_if_finished(&session.session_id)
            .await
            .expect("session should complete");

        assert_eq!(snapshot.status, LibraryScanStatus::Warning);
        assert_eq!(
            snapshot.warning_message.as_deref(),
            Some("Imported Sonarr/Radarr monitored state could not be applied after this scan.")
        );
    }

    #[tokio::test]
    async fn live_apply_delta_updates_session_without_projection_replay() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Series)
            .await
            .expect("start session");

        let snapshot = tracker
            .apply_delta(
                &session.session_id,
                &LibraryScanDeltaRecordedEventData {
                    session_id: session.session_id.clone(),
                    found_titles_total: Some(3),
                    found_titles_delta: 0,
                    title_match_completed_delta: 2,
                    title_match_failed_delta: 0,
                    title_match_total_known: Some(true),
                    metadata_total_delta: 2,
                    metadata_completed_delta: 1,
                    metadata_failed_delta: 1,
                    metadata_total_known: Some(true),
                    file_total_delta: 4,
                    file_completed_delta: 3,
                    file_failed_delta: 1,
                    file_total_known: Some(true),
                    summary: Some(LibraryScanSummaryEventData {
                        scanned: 4,
                        matched: 2,
                        imported: 2,
                        skipped: 1,
                        unmatched: 1,
                    }),
                    summary_is_delta: false,
                },
            )
            .await
            .expect("apply live delta");

        assert_eq!(snapshot.status, LibraryScanStatus::Running);
        assert_eq!(snapshot.found_titles, 3);
        assert!(snapshot.title_match_total_known);
        assert_eq!(snapshot.title_match_progress.total, 3);
        assert_eq!(snapshot.title_match_progress.completed, 2);
        assert_eq!(snapshot.metadata_progress.total, 2);
        assert_eq!(snapshot.metadata_progress.completed, 1);
        assert_eq!(snapshot.metadata_progress.failed, 1);
        assert_eq!(snapshot.file_progress.total, 4);
        assert_eq!(snapshot.file_progress.completed, 3);
        assert_eq!(snapshot.file_progress.failed, 1);
        assert_eq!(
            snapshot.summary,
            Some(LibraryScanSummary {
                scanned: 4,
                matched: 2,
                imported: 2,
                skipped: 1,
                unmatched: 1,
            })
        );
    }

    #[tokio::test]
    async fn live_file_total_delta_is_ignored_after_total_known() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Series)
            .await
            .expect("start session");

        tracker
            .apply_delta(
                &session.session_id,
                &LibraryScanDeltaRecordedEventData {
                    session_id: session.session_id.clone(),
                    found_titles_total: None,
                    found_titles_delta: 0,
                    title_match_completed_delta: 0,
                    title_match_failed_delta: 0,
                    title_match_total_known: None,
                    metadata_total_delta: 0,
                    metadata_completed_delta: 0,
                    metadata_failed_delta: 0,
                    metadata_total_known: None,
                    file_total_delta: 2,
                    file_completed_delta: 0,
                    file_failed_delta: 0,
                    file_total_known: Some(true),
                    summary: None,
                    summary_is_delta: false,
                },
            )
            .await
            .expect("seed known file total");

        let snapshot = tracker
            .apply_delta(
                &session.session_id,
                &LibraryScanDeltaRecordedEventData {
                    session_id: session.session_id.clone(),
                    found_titles_total: None,
                    found_titles_delta: 0,
                    title_match_completed_delta: 0,
                    title_match_failed_delta: 0,
                    title_match_total_known: None,
                    metadata_total_delta: 0,
                    metadata_completed_delta: 0,
                    metadata_failed_delta: 0,
                    metadata_total_known: None,
                    file_total_delta: 3,
                    file_completed_delta: 0,
                    file_failed_delta: 0,
                    file_total_known: None,
                    summary: None,
                    summary_is_delta: false,
                },
            )
            .await
            .expect("late file total delta");

        assert!(snapshot.file_total_known);
        assert_eq!(snapshot.file_progress.total, 2);
    }

    #[tokio::test]
    async fn live_delta_can_drive_terminal_completion() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Movie)
            .await
            .expect("start session");

        tracker
            .apply_delta(
                &session.session_id,
                &LibraryScanDeltaRecordedEventData {
                    session_id: session.session_id.clone(),
                    found_titles_total: Some(1),
                    found_titles_delta: 0,
                    title_match_completed_delta: 1,
                    title_match_failed_delta: 0,
                    title_match_total_known: Some(true),
                    metadata_total_delta: 1,
                    metadata_completed_delta: 1,
                    metadata_failed_delta: 0,
                    metadata_total_known: Some(true),
                    file_total_delta: 1,
                    file_completed_delta: 1,
                    file_failed_delta: 0,
                    file_total_known: Some(true),
                    summary: Some(LibraryScanSummaryEventData {
                        scanned: 1,
                        matched: 1,
                        imported: 1,
                        skipped: 0,
                        unmatched: 0,
                    }),
                    summary_is_delta: false,
                },
            )
            .await
            .expect("apply live delta");

        let terminal = tracker
            .complete_if_finished(&session.session_id)
            .await
            .expect("session should complete live");

        assert_eq!(terminal.status, LibraryScanStatus::Completed);
        assert!(tracker.get_session(&session.session_id).await.is_none());
    }

    #[test]
    fn delta_recorded_updates_projection_without_double_counting_title_discovery() {
        let session_id = "session-1";
        let mut sessions = HashMap::new();

        reduce_library_scan_projection_event(
            &mut sessions,
            &test_library_scan_event(
                1,
                session_id,
                MediaFacet::Movie,
                DomainEventPayload::LibraryScanStarted(LibraryScanStartedEventData {
                    session_id: session_id.to_string(),
                    library_id: None,
                    mode: "full".to_string(),
                }),
            ),
        );
        reduce_library_scan_projection_event(
            &mut sessions,
            &test_library_scan_event(
                2,
                session_id,
                MediaFacet::Movie,
                DomainEventPayload::LibraryScanTitleDiscovered(
                    LibraryScanTitleDiscoveredEventData {
                        session_id: session_id.to_string(),
                        title_id: "title-1".to_string(),
                        title_name: "Harbor Pals".to_string(),
                        facet: MediaFacet::Movie,
                        discovered_file_count: 1,
                        folder_path: None,
                    },
                ),
            ),
        );

        let snapshot = reduce_library_scan_projection_event(
            &mut sessions,
            &test_library_scan_event(
                3,
                session_id,
                MediaFacet::Movie,
                DomainEventPayload::LibraryScanDeltaRecorded(LibraryScanDeltaRecordedEventData {
                    session_id: session_id.to_string(),
                    found_titles_total: None,
                    found_titles_delta: 4,
                    title_match_completed_delta: 0,
                    title_match_failed_delta: 0,
                    title_match_total_known: None,
                    metadata_total_delta: 0,
                    metadata_completed_delta: 0,
                    metadata_failed_delta: 0,
                    metadata_total_known: None,
                    file_total_delta: 4,
                    file_completed_delta: 0,
                    file_failed_delta: 0,
                    file_total_known: None,
                    summary: None,
                    summary_is_delta: false,
                }),
            ),
        )
        .expect("delta should update active session");

        assert_eq!(snapshot.found_titles, 4);
        assert_eq!(snapshot.title_match_progress.total, 4);
        assert_eq!(snapshot.file_progress.total, 4);
    }

    #[test]
    fn delta_recorded_can_make_session_ready_for_completion() {
        let session_id = "session-2";
        let mut sessions = HashMap::new();

        reduce_library_scan_projection_event(
            &mut sessions,
            &test_library_scan_event(
                1,
                session_id,
                MediaFacet::Series,
                DomainEventPayload::LibraryScanStarted(LibraryScanStartedEventData {
                    session_id: session_id.to_string(),
                    library_id: None,
                    mode: "full".to_string(),
                }),
            ),
        );

        let snapshot = reduce_library_scan_projection_event(
            &mut sessions,
            &test_library_scan_event(
                2,
                session_id,
                MediaFacet::Series,
                DomainEventPayload::LibraryScanDeltaRecorded(LibraryScanDeltaRecordedEventData {
                    session_id: session_id.to_string(),
                    found_titles_total: Some(2),
                    found_titles_delta: 0,
                    title_match_completed_delta: 2,
                    title_match_failed_delta: 0,
                    title_match_total_known: Some(true),
                    metadata_total_delta: 1,
                    metadata_completed_delta: 1,
                    metadata_failed_delta: 0,
                    metadata_total_known: Some(true),
                    file_total_delta: 2,
                    file_completed_delta: 2,
                    file_failed_delta: 0,
                    file_total_known: Some(true),
                    summary: Some(LibraryScanSummaryEventData {
                        scanned: 2,
                        matched: 2,
                        imported: 1,
                        skipped: 0,
                        unmatched: 0,
                    }),
                    summary_is_delta: false,
                }),
            ),
        )
        .expect("delta should update active session");

        assert!(snapshot.is_ready_to_complete());
        assert_eq!(snapshot.completion_status(), LibraryScanStatus::Completed);
        assert_eq!(
            snapshot.summary.as_ref().map(|summary| summary.imported),
            Some(1)
        );
    }
}
