use chrono::{DateTime, Utc};
use scryer_domain::MediaFacet;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify, broadcast};
use tokio::time::{Duration, Sleep};

use crate::{AppError, AppResult, Id, JobRunTracker, LibraryFile, LibraryScanSummary};

const LIBRARY_SCAN_PROGRESS_PUSH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryScanStatus {
    Discovering,
    Running,
    Completed,
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
            Self::Warning => "warning",
            Self::Failed => "failed",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Warning | Self::Failed)
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
    pub mode: LibraryScanMode,
    pub status: LibraryScanStatus,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub found_titles: usize,
    pub metadata_total_known: bool,
    pub file_total_known: bool,
    pub metadata_progress: LibraryScanPhaseProgress,
    pub file_progress: LibraryScanPhaseProgress,
    pub summary: Option<LibraryScanSummary>,
}

impl LibraryScanSession {
    fn new(facet: MediaFacet) -> Self {
        let now = Utc::now();
        Self {
            session_id: Id::new().0,
            facet,
            mode: LibraryScanMode::Full,
            status: LibraryScanStatus::Discovering,
            started_at: now,
            updated_at: now,
            found_titles: 0,
            metadata_total_known: false,
            file_total_known: false,
            metadata_progress: LibraryScanPhaseProgress::default(),
            file_progress: LibraryScanPhaseProgress::default(),
            summary: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryScanTitleAttachResult {
    pub new_title: bool,
    pub added_file_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TrackedTitlePreScanState {
    Pending,
    Ready(Vec<LibraryFile>),
    Failed,
    Abandoned,
    Consumed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrackedTitleState {
    session_id: String,
    pre_scan_state: TrackedTitlePreScanState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackedTitleFilesConsumption {
    Pending,
    Ready(Vec<LibraryFile>),
    FallbackRequired,
    NotTracked,
}

#[derive(Default)]
struct LibraryScanRuntimeState {
    sessions: HashMap<String, LibraryScanSession>,
    facet_sessions: HashMap<MediaFacet, String>,
    tracked_titles: HashMap<String, TrackedTitleState>,
}

#[derive(Clone)]
pub struct LibraryScanTracker {
    state: Arc<Mutex<LibraryScanRuntimeState>>,
    broadcast: broadcast::Sender<LibraryScanSession>,
    pre_scan_notify: Arc<Notify>,
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
            pre_scan_notify: Arc::new(Notify::new()),
            job_run_tracker: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_job_run_tracker(&self, tracker: JobRunTracker) {
        let mut slot = self.job_run_tracker.lock().await;
        *slot = Some(tracker);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LibraryScanSession> {
        let mut source = self.broadcast.subscribe();
        let (tx, rx) = broadcast::channel(256);

        tokio::spawn(async move {
            let mut pending: Option<LibraryScanSession> = None;
            let mut flush_timer: Option<Pin<Box<Sleep>>> = None;

            loop {
                if let Some(timer) = flush_timer.as_mut() {
                    tokio::select! {
                        recv_result = source.recv() => {
                            match recv_result {
                                Ok(session) => {
                                    if session.status.is_terminal() {
                                        pending = None;
                                        flush_timer = None;
                                        if tx.send(session).is_err() {
                                            break;
                                        }
                                    } else {
                                        pending = Some(session);
                                    }
                                }
                                Err(broadcast::error::RecvError::Lagged(n)) => {
                                    tracing::debug!(
                                        "library_scan_progress: receiver lagged, skipped {n} messages"
                                    );
                                }
                                Err(broadcast::error::RecvError::Closed) => {
                                    if let Some(session) = pending.take()
                                        && tx.send(session).is_err()
                                    {
                                        break;
                                    }
                                    break;
                                }
                            }
                        }
                        _ = timer.as_mut() => {
                            flush_timer = None;
                            if let Some(session) = pending.take()
                                && tx.send(session).is_err()
                            {
                                break;
                            }
                        }
                    }
                    continue;
                }

                match source.recv().await {
                    Ok(session) => {
                        if session.status.is_terminal() {
                            if tx.send(session).is_err() {
                                break;
                            }
                        } else {
                            pending = Some(session);
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
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        rx
    }

    pub async fn list_active(&self) -> Vec<LibraryScanSession> {
        let state = self.state.lock().await;
        let mut sessions = state.sessions.values().cloned().collect::<Vec<_>>();
        sessions.sort_by(|left, right| left.started_at.cmp(&right.started_at));
        sessions
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

    pub async fn start_session(&self, facet: MediaFacet) -> AppResult<LibraryScanSession> {
        self.start_session_with_id(Id::new().0, facet, LibraryScanMode::Full)
            .await
    }

    pub async fn start_session_with_id(
        &self,
        session_id: String,
        facet: MediaFacet,
        mode: LibraryScanMode,
    ) -> AppResult<LibraryScanSession> {
        let snapshot = {
            let mut state = self.state.lock().await;
            if state.facet_sessions.contains_key(&facet) {
                return Err(AppError::Validation(format!(
                    "{} library scan already running",
                    facet.as_str()
                )));
            }

            let mut snapshot = LibraryScanSession::new(facet.clone());
            snapshot.session_id = session_id;
            snapshot.mode = mode;
            state
                .facet_sessions
                .insert(facet, snapshot.session_id.clone());
            state
                .sessions
                .insert(snapshot.session_id.clone(), snapshot.clone());
            snapshot
        };
        self.notify_snapshot(snapshot.clone()).await;
        Ok(snapshot)
    }

    pub async fn set_found_titles(
        &self,
        session_id: &str,
        found_titles: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.found_titles = found_titles;
            if matches!(session.status, LibraryScanStatus::Discovering) {
                session.status = LibraryScanStatus::Running;
            }
        })
        .await
    }

    pub async fn add_found_titles(
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

    pub async fn add_metadata_total(
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

    pub async fn mark_metadata_total_known(&self, session_id: &str) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.metadata_total_known = true;
        })
        .await
    }

    pub async fn add_file_total(
        &self,
        session_id: &str,
        additional: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.file_progress.add_total(additional);
            if matches!(session.status, LibraryScanStatus::Discovering) {
                session.status = LibraryScanStatus::Running;
            }
        })
        .await
    }

    pub async fn mark_file_total_known(&self, session_id: &str) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.file_total_known = true;
        })
        .await
    }

    pub async fn increment_metadata_completed(
        &self,
        session_id: &str,
        additional: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.metadata_progress.mark_completed(additional);
        })
        .await
    }

    pub async fn increment_metadata_failed(
        &self,
        session_id: &str,
        additional: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.metadata_progress.mark_failed(additional);
        })
        .await
    }

    pub async fn increment_file_completed(
        &self,
        session_id: &str,
        additional: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.file_progress.mark_completed(additional);
        })
        .await
    }

    pub async fn increment_file_failed(
        &self,
        session_id: &str,
        additional: usize,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.file_progress.mark_failed(additional);
        })
        .await
    }

    pub async fn attach_title(
        &self,
        session_id: &str,
        title_id: &str,
        pre_scanned_files: Option<Vec<LibraryFile>>,
    ) -> Option<LibraryScanTitleAttachResult> {
        let (snapshot, result, notify_pre_scan) = {
            let mut state = self.state.lock().await;
            if !state.sessions.contains_key(session_id) {
                return None;
            }

            let new_title = !state.tracked_titles.contains_key(title_id);
            let tracked_title = state
                .tracked_titles
                .entry(title_id.to_string())
                .or_insert_with(|| TrackedTitleState {
                    session_id: session_id.to_string(),
                    pre_scan_state: TrackedTitlePreScanState::Consumed,
                });
            if tracked_title.session_id != session_id {
                return None;
            }

            let mut added_file_count = 0usize;
            let mut notify_pre_scan = false;
            if let Some(files) = pre_scanned_files {
                match tracked_title.pre_scan_state {
                    TrackedTitlePreScanState::Pending | TrackedTitlePreScanState::Consumed => {
                        added_file_count = files.len();
                        tracked_title.pre_scan_state = TrackedTitlePreScanState::Ready(files);
                        notify_pre_scan = true;
                    }
                    TrackedTitlePreScanState::Failed
                    | TrackedTitlePreScanState::Ready(_)
                    | TrackedTitlePreScanState::Abandoned => {}
                }
            }

            let session = state
                .sessions
                .get_mut(session_id)
                .expect("session existence checked above");
            if added_file_count > 0 {
                session.file_progress.add_total(added_file_count);
            }
            session.updated_at = Utc::now();
            if matches!(session.status, LibraryScanStatus::Discovering) {
                session.status = LibraryScanStatus::Running;
            }
            (
                session.clone(),
                LibraryScanTitleAttachResult {
                    new_title,
                    added_file_count,
                },
                notify_pre_scan,
            )
        };

        if notify_pre_scan {
            self.pre_scan_notify.notify_waiters();
        }
        self.notify_snapshot(snapshot).await;
        Some(result)
    }

    pub async fn mark_title_pre_scan_started(&self, title_id: &str) {
        let mut state = self.state.lock().await;
        if let Some(tracked_title) = state.tracked_titles.get_mut(title_id)
            && matches!(
                tracked_title.pre_scan_state,
                TrackedTitlePreScanState::Consumed
            )
        {
            tracked_title.pre_scan_state = TrackedTitlePreScanState::Pending;
        }
    }

    pub async fn mark_title_pre_scan_finished(&self, title_id: &str) {
        let removed = {
            let mut state = self.state.lock().await;
            let Some(tracked_title) = state.tracked_titles.get_mut(title_id) else {
                return;
            };
            if matches!(
                tracked_title.pre_scan_state,
                TrackedTitlePreScanState::Pending
            ) {
                tracked_title.pre_scan_state = TrackedTitlePreScanState::Consumed;
                true
            } else {
                false
            }
        };

        if removed {
            self.pre_scan_notify.notify_waiters();
        }
    }

    pub async fn mark_title_pre_scan_failed(&self, title_id: &str) {
        let removed = {
            let mut state = self.state.lock().await;
            let Some(tracked_title) = state.tracked_titles.get_mut(title_id) else {
                return;
            };
            if matches!(
                tracked_title.pre_scan_state,
                TrackedTitlePreScanState::Pending
            ) {
                tracked_title.pre_scan_state = TrackedTitlePreScanState::Failed;
                true
            } else {
                false
            }
        };

        if removed {
            self.pre_scan_notify.notify_waiters();
        }
    }

    pub async fn wait_for_title_pre_scan_update(&self) {
        self.pre_scan_notify.notified().await;
    }

    pub async fn abandon_title_pre_scan(&self, title_id: &str) {
        let removed_pending = {
            let mut state = self.state.lock().await;
            let Some(tracked_title) = state.tracked_titles.get_mut(title_id) else {
                return;
            };
            let removed_pending = matches!(
                tracked_title.pre_scan_state,
                TrackedTitlePreScanState::Pending | TrackedTitlePreScanState::Failed
            );
            tracked_title.pre_scan_state = TrackedTitlePreScanState::Abandoned;
            removed_pending
        };

        if removed_pending {
            self.pre_scan_notify.notify_waiters();
        }
    }

    pub async fn has_pending_title_pre_scans_for_session(&self, session_id: &str) -> bool {
        let state = self.state.lock().await;
        state.tracked_titles.values().any(|tracked_title| {
            tracked_title.session_id == session_id
                && matches!(
                    tracked_title.pre_scan_state,
                    TrackedTitlePreScanState::Pending | TrackedTitlePreScanState::Failed
                )
        })
    }

    pub async fn session_for_title(&self, title_id: &str) -> Option<String> {
        let state = self.state.lock().await;
        state
            .tracked_titles
            .get(title_id)
            .map(|tracked_title| tracked_title.session_id.clone())
    }

    pub async fn consume_tracked_title_files(
        &self,
        title_id: &str,
    ) -> TrackedTitleFilesConsumption {
        let mut state = self.state.lock().await;
        let Some(tracked_title) = state.tracked_titles.get_mut(title_id) else {
            return TrackedTitleFilesConsumption::NotTracked;
        };

        match std::mem::replace(
            &mut tracked_title.pre_scan_state,
            TrackedTitlePreScanState::Consumed,
        ) {
            TrackedTitlePreScanState::Pending => {
                tracked_title.pre_scan_state = TrackedTitlePreScanState::Pending;
                TrackedTitleFilesConsumption::Pending
            }
            TrackedTitlePreScanState::Ready(files) => TrackedTitleFilesConsumption::Ready(files),
            TrackedTitlePreScanState::Failed
            | TrackedTitlePreScanState::Abandoned
            | TrackedTitlePreScanState::Consumed => TrackedTitleFilesConsumption::FallbackRequired,
        }
    }

    pub async fn release_title(&self, title_id: &str) {
        let removed = {
            let mut state = self.state.lock().await;
            state
                .tracked_titles
                .remove(title_id)
                .is_some_and(|tracked_title| {
                    matches!(
                        tracked_title.pre_scan_state,
                        TrackedTitlePreScanState::Pending | TrackedTitlePreScanState::Failed
                    )
                })
        };
        if removed {
            self.pre_scan_notify.notify_waiters();
        }
    }

    pub async fn mark_title_metadata_completed(&self, title_id: &str) -> Option<String> {
        let session_id = {
            let state = self.state.lock().await;
            state
                .tracked_titles
                .get(title_id)
                .map(|tracked_title| tracked_title.session_id.clone())
        }?;
        self.increment_metadata_completed(&session_id, 1).await;
        Some(session_id)
    }

    pub async fn mark_title_metadata_failed(&self, title_id: &str) -> Option<String> {
        let (session_id, cached_file_count, removed_pending) = {
            let mut state = self.state.lock().await;
            let tracked_title = state.tracked_titles.remove(title_id)?;
            let session_id = tracked_title.session_id.clone();
            let removed_pending = matches!(
                tracked_title.pre_scan_state,
                TrackedTitlePreScanState::Pending | TrackedTitlePreScanState::Failed
            );
            let cached_file_count = match tracked_title.pre_scan_state {
                TrackedTitlePreScanState::Ready(files) => files.len(),
                _ => 0,
            };
            (session_id, cached_file_count, removed_pending)
        };
        if removed_pending {
            self.pre_scan_notify.notify_waiters();
        }
        self.increment_metadata_failed(&session_id, 1).await;
        if cached_file_count > 0 {
            self.increment_file_failed(&session_id, cached_file_count)
                .await;
        }
        Some(session_id)
    }

    pub async fn set_summary(
        &self,
        session_id: &str,
        summary: LibraryScanSummary,
    ) -> Option<LibraryScanSession> {
        self.update_session(session_id, move |session| {
            session.summary = Some(summary.clone());
        })
        .await
    }

    pub async fn complete_if_finished(&self, session_id: &str) -> Option<LibraryScanSession> {
        let (snapshot, removed_pending) = {
            let mut state = self.state.lock().await;
            let session = state.sessions.get(session_id)?;
            if session.summary.is_none()
                || !session.metadata_progress.is_finished()
                || !session.file_progress.is_finished()
            {
                return None;
            }

            let mut session = state.sessions.remove(session_id)?;
            session.updated_at = Utc::now();
            session.metadata_total_known = true;
            session.file_total_known = true;
            session.status =
                if session.metadata_progress.failed > 0 || session.file_progress.failed > 0 {
                    LibraryScanStatus::Warning
                } else {
                    LibraryScanStatus::Completed
                };
            state.facet_sessions.remove(&session.facet);
            let removed_pending = state
                .tracked_titles
                .extract_if(|_, tracked_title| tracked_title.session_id == session_id)
                .any(|(_, tracked_title)| {
                    matches!(
                        tracked_title.pre_scan_state,
                        TrackedTitlePreScanState::Pending | TrackedTitlePreScanState::Failed
                    )
                });
            (session, removed_pending)
        };
        if removed_pending {
            self.pre_scan_notify.notify_waiters();
        }
        self.notify_snapshot(snapshot.clone()).await;
        Some(snapshot)
    }

    pub async fn fail_session(&self, session_id: &str) -> Option<LibraryScanSession> {
        let (snapshot, removed_pending) = {
            let mut state = self.state.lock().await;
            let mut session = state.sessions.remove(session_id)?;
            session.updated_at = Utc::now();
            session.metadata_total_known = true;
            session.file_total_known = true;
            session.status = LibraryScanStatus::Failed;
            state.facet_sessions.remove(&session.facet);
            let removed_pending = state
                .tracked_titles
                .extract_if(|_, tracked_title| tracked_title.session_id == session_id)
                .any(|(_, tracked_title)| {
                    matches!(
                        tracked_title.pre_scan_state,
                        TrackedTitlePreScanState::Pending | TrackedTitlePreScanState::Failed
                    )
                });
            (session, removed_pending)
        };
        if removed_pending {
            self.pre_scan_notify.notify_waiters();
        }
        self.notify_snapshot(snapshot.clone()).await;
        Some(snapshot)
    }

    pub async fn get_session(&self, session_id: &str) -> Option<LibraryScanSession> {
        let state = self.state.lock().await;
        state.sessions.get(session_id).cloned()
    }

    pub async fn mark_file_total_known_if_resolved(
        &self,
        session_id: &str,
    ) -> Option<LibraryScanSession> {
        let snapshot = {
            let mut state = self.state.lock().await;
            let has_unresolved = state.tracked_titles.values().any(|tracked_title| {
                tracked_title.session_id == session_id
                    && matches!(
                        tracked_title.pre_scan_state,
                        TrackedTitlePreScanState::Pending | TrackedTitlePreScanState::Failed
                    )
            });
            if has_unresolved {
                return None;
            }

            let session = state.sessions.get_mut(session_id)?;
            if session.file_total_known {
                return None;
            }
            session.file_total_known = true;
            session.updated_at = Utc::now();
            session.clone()
        };
        self.notify_snapshot(snapshot.clone()).await;
        Some(snapshot)
    }

    async fn update_session(
        &self,
        session_id: &str,
        mutator: impl FnOnce(&mut LibraryScanSession),
    ) -> Option<LibraryScanSession> {
        let snapshot = {
            let mut state = self.state.lock().await;
            let session = state.sessions.get_mut(session_id)?;
            mutator(session);
            session.updated_at = Utc::now();
            session.clone()
        };
        self.notify_snapshot(snapshot.clone()).await;
        Some(snapshot)
    }

    async fn notify_snapshot(&self, snapshot: LibraryScanSession) {
        let _ = self.broadcast.send(snapshot.clone());
        if let Some(tracker) = self.job_run_tracker.lock().await.clone() {
            tracker.merge_library_scan_progress(snapshot).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn attach_title_counts_files_once_and_completes_with_warning() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Anime)
            .await
            .expect("start session");

        let attach = tracker
            .attach_title(
                &session.session_id,
                "title-1",
                Some(vec![
                    LibraryFile {
                        path: "/tmp/A.mkv".into(),
                        display_name: "A".into(),
                        nfo_path: None,
                        size_bytes: None,
                        source_signature_scheme: None,
                        source_signature_value: None,
                    },
                    LibraryFile {
                        path: "/tmp/B.mkv".into(),
                        display_name: "B".into(),
                        nfo_path: None,
                        size_bytes: None,
                        source_signature_scheme: None,
                        source_signature_value: None,
                    },
                ]),
            )
            .await
            .expect("attach files");
        assert!(attach.new_title);
        assert_eq!(attach.added_file_count, 2);

        let attach_again = tracker
            .attach_title(&session.session_id, "title-1", None)
            .await
            .expect("reattach title");
        assert!(!attach_again.new_title);
        assert_eq!(attach_again.added_file_count, 0);

        tracker.add_metadata_total(&session.session_id, 1).await;
        tracker
            .increment_file_completed(&session.session_id, 1)
            .await;
        let failed_session_id = tracker
            .mark_title_metadata_failed("title-1")
            .await
            .expect("mark failed");
        assert_eq!(failed_session_id, session.session_id);
        tracker
            .set_summary(
                &session.session_id,
                LibraryScanSummary {
                    scanned: 2,
                    matched: 2,
                    imported: 1,
                    skipped: 0,
                    unmatched: 0,
                },
            )
            .await;

        let final_snapshot = tracker
            .complete_if_finished(&session.session_id)
            .await
            .expect("complete session");
        assert_eq!(final_snapshot.status, LibraryScanStatus::Warning);
        assert_eq!(final_snapshot.metadata_progress.failed, 1);
        assert_eq!(final_snapshot.file_progress.completed, 1);
        assert_eq!(final_snapshot.file_progress.failed, 1);
        assert!(tracker.list_active().await.is_empty());
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
    async fn wait_until_idle_returns_immediately_without_sessions() {
        let tracker = LibraryScanTracker::new();

        tokio::time::timeout(Duration::from_millis(100), tracker.wait_until_idle())
            .await
            .expect("idle tracker should resolve immediately");
    }

    #[tokio::test]
    async fn wait_until_idle_blocks_until_terminal_session() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Anime)
            .await
            .expect("start session");

        let waiter = tokio::spawn({
            let tracker = tracker.clone();
            async move { tracker.wait_until_idle().await }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !waiter.is_finished(),
            "waiter should block while scan is active"
        );

        tracker
            .fail_session(&session.session_id)
            .await
            .expect("session should fail");

        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("waiter should resolve once scan finishes")
            .expect("waiter task should not panic");
    }

    #[tokio::test]
    async fn abandoned_pre_scan_does_not_add_late_file_totals() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Anime)
            .await
            .expect("start session");

        tracker
            .attach_title(&session.session_id, "title-1", None)
            .await
            .expect("attach title");
        tracker.mark_title_pre_scan_started("title-1").await;
        assert!(
            tracker
                .has_pending_title_pre_scans_for_session(&session.session_id)
                .await
        );

        tracker.abandon_title_pre_scan("title-1").await;
        assert!(
            !tracker
                .has_pending_title_pre_scans_for_session(&session.session_id)
                .await
        );

        let late_attach = tracker
            .attach_title(
                &session.session_id,
                "title-1",
                Some(vec![LibraryFile {
                    path: "/tmp/C.mkv".into(),
                    display_name: "C".into(),
                    nfo_path: None,
                    size_bytes: None,
                    source_signature_scheme: None,
                    source_signature_value: None,
                }]),
            )
            .await
            .expect("late attach");

        assert_eq!(late_attach.added_file_count, 0);
        let snapshot = tracker
            .get_session(&session.session_id)
            .await
            .expect("session exists");
        assert_eq!(snapshot.file_progress.total, 0);
    }

    #[tokio::test]
    async fn consume_tracked_title_files_transitions_ready_to_fallback_required() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Series)
            .await
            .expect("start session");

        tracker
            .attach_title(&session.session_id, "title-1", None)
            .await
            .expect("attach title");
        tracker.mark_title_pre_scan_started("title-1").await;

        match tracker.consume_tracked_title_files("title-1").await {
            TrackedTitleFilesConsumption::Pending => {}
            other => panic!("expected pending consumption, got {other:?}"),
        }

        tracker
            .attach_title(
                &session.session_id,
                "title-1",
                Some(vec![LibraryFile {
                    path: "/tmp/D.mkv".into(),
                    display_name: "D".into(),
                    nfo_path: None,
                    size_bytes: None,
                    source_signature_scheme: None,
                    source_signature_value: None,
                }]),
            )
            .await
            .expect("attach pre-scanned files");

        assert!(
            !tracker
                .has_pending_title_pre_scans_for_session(&session.session_id)
                .await
        );

        match tracker.consume_tracked_title_files("title-1").await {
            TrackedTitleFilesConsumption::Ready(files) => {
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].display_name, "D");
            }
            other => panic!("expected ready files, got {other:?}"),
        }

        match tracker.consume_tracked_title_files("title-1").await {
            TrackedTitleFilesConsumption::FallbackRequired => {}
            other => panic!("expected fallback required after consume, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mark_title_pre_scan_finished_clears_pending_state_without_files() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Anime)
            .await
            .expect("start session");

        tracker
            .attach_title(&session.session_id, "title-1", None)
            .await
            .expect("attach title");
        tracker.mark_title_pre_scan_started("title-1").await;
        assert!(
            tracker
                .has_pending_title_pre_scans_for_session(&session.session_id)
                .await
        );

        tracker.mark_title_pre_scan_finished("title-1").await;

        assert!(
            !tracker
                .has_pending_title_pre_scans_for_session(&session.session_id)
                .await
        );
        match tracker.consume_tracked_title_files("title-1").await {
            TrackedTitleFilesConsumption::FallbackRequired => {}
            other => panic!("expected fallback required after empty pre-scan, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn failed_pre_scan_keeps_file_total_unresolved_until_abandoned() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Series)
            .await
            .expect("start session");

        tracker
            .attach_title(&session.session_id, "title-1", None)
            .await
            .expect("attach title");
        tracker.mark_title_pre_scan_started("title-1").await;
        tracker.mark_title_pre_scan_failed("title-1").await;

        assert!(
            tracker
                .has_pending_title_pre_scans_for_session(&session.session_id)
                .await
        );
        assert!(
            tracker
                .mark_file_total_known_if_resolved(&session.session_id)
                .await
                .is_none()
        );
        assert!(
            !tracker
                .get_session(&session.session_id)
                .await
                .expect("session exists")
                .file_total_known
        );

        tracker.abandon_title_pre_scan("title-1").await;

        let snapshot = tracker
            .mark_file_total_known_if_resolved(&session.session_id)
            .await
            .expect("file total should become known once fallback owns the title");
        assert!(snapshot.file_total_known);
    }

    #[tokio::test]
    async fn release_failed_pre_scan_title_allows_file_total_to_settle() {
        let tracker = LibraryScanTracker::new();
        let session = tracker
            .start_session(MediaFacet::Anime)
            .await
            .expect("start session");

        tracker
            .attach_title(&session.session_id, "title-1", None)
            .await
            .expect("attach title");
        tracker.mark_title_pre_scan_started("title-1").await;
        tracker.mark_title_pre_scan_failed("title-1").await;

        tracker.release_title("title-1").await;

        let snapshot = tracker
            .mark_file_total_known_if_resolved(&session.session_id)
            .await
            .expect("release should clear the unresolved title");
        assert!(snapshot.file_total_known);
    }
}
