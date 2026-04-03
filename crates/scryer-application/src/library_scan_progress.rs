use chrono::{DateTime, Utc};
use scryer_domain::MediaFacet;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify, broadcast};

use crate::{AppError, AppResult, Id, LibraryFile, LibraryScanSummary};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryScanStatus {
    Discovering,
    Running,
    Completed,
    Warning,
    Failed,
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

#[derive(Default)]
struct LibraryScanRuntimeState {
    sessions: HashMap<String, LibraryScanSession>,
    facet_sessions: HashMap<MediaFacet, String>,
    title_sessions: HashMap<String, String>,
    pre_scanned_title_files: HashMap<String, Vec<LibraryFile>>,
    pending_title_pre_scans: HashSet<String>,
}

#[derive(Clone)]
pub struct LibraryScanTracker {
    state: Arc<Mutex<LibraryScanRuntimeState>>,
    broadcast: broadcast::Sender<LibraryScanSession>,
    pre_scan_notify: Arc<Notify>,
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
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LibraryScanSession> {
        self.broadcast.subscribe()
    }

    pub async fn list_active(&self) -> Vec<LibraryScanSession> {
        let state = self.state.lock().await;
        let mut sessions = state.sessions.values().cloned().collect::<Vec<_>>();
        sessions.sort_by(|left, right| left.started_at.cmp(&right.started_at));
        sessions
    }

    pub async fn start_session(&self, facet: MediaFacet) -> AppResult<LibraryScanSession> {
        let snapshot = {
            let mut state = self.state.lock().await;
            if state.facet_sessions.contains_key(&facet) {
                return Err(AppError::Validation(format!(
                    "{} library scan already running",
                    facet.as_str()
                )));
            }

            let snapshot = LibraryScanSession::new(facet.clone());
            state
                .facet_sessions
                .insert(facet, snapshot.session_id.clone());
            state
                .sessions
                .insert(snapshot.session_id.clone(), snapshot.clone());
            snapshot
        };
        let _ = self.broadcast.send(snapshot.clone());
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

            let existing_session_id = state.title_sessions.get(title_id).cloned();
            let new_title = existing_session_id.is_none();
            if new_title {
                state
                    .title_sessions
                    .insert(title_id.to_string(), session_id.to_string());
            } else if existing_session_id.as_deref() != Some(session_id) {
                return None;
            }

            let mut added_file_count = 0usize;
            let mut notify_pre_scan = false;
            if let Some(files) =
                pre_scanned_files.filter(|_| !state.pre_scanned_title_files.contains_key(title_id))
            {
                added_file_count = files.len();
                state
                    .pre_scanned_title_files
                    .insert(title_id.to_string(), files);
                notify_pre_scan = state.pending_title_pre_scans.remove(title_id);
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
        let _ = self.broadcast.send(snapshot);
        Some(result)
    }

    pub async fn mark_title_pre_scan_started(&self, title_id: &str) {
        let mut state = self.state.lock().await;
        state.pending_title_pre_scans.insert(title_id.to_string());
    }

    pub async fn mark_title_pre_scan_finished(&self, title_id: &str) {
        let removed = {
            let mut state = self.state.lock().await;
            state.pending_title_pre_scans.remove(title_id)
        };

        if removed {
            self.pre_scan_notify.notify_waiters();
        }
    }

    pub async fn is_title_pre_scan_pending(&self, title_id: &str) -> bool {
        let state = self.state.lock().await;
        state.pending_title_pre_scans.contains(title_id)
    }

    pub async fn wait_for_title_pre_scan_update(&self) {
        self.pre_scan_notify.notified().await;
    }

    pub async fn session_for_title(&self, title_id: &str) -> Option<String> {
        let state = self.state.lock().await;
        state.title_sessions.get(title_id).cloned()
    }

    pub async fn take_title_files(&self, title_id: &str) -> Option<Vec<LibraryFile>> {
        let mut state = self.state.lock().await;
        state.pre_scanned_title_files.remove(title_id)
    }

    pub async fn release_title(&self, title_id: &str) {
        let removed = {
            let mut state = self.state.lock().await;
            state.title_sessions.remove(title_id);
            state.pre_scanned_title_files.remove(title_id);
            state.pending_title_pre_scans.remove(title_id)
        };
        if removed {
            self.pre_scan_notify.notify_waiters();
        }
    }

    pub async fn mark_title_metadata_completed(&self, title_id: &str) -> Option<String> {
        let session_id = {
            let state = self.state.lock().await;
            state.title_sessions.get(title_id).cloned()
        }?;
        self.increment_metadata_completed(&session_id, 1).await;
        Some(session_id)
    }

    pub async fn mark_title_metadata_failed(&self, title_id: &str) -> Option<String> {
        let (session_id, cached_file_count, removed_pending) = {
            let mut state = self.state.lock().await;
            let session_id = state.title_sessions.remove(title_id)?;
            let removed_pending = state.pending_title_pre_scans.remove(title_id);
            let cached_file_count = state
                .pre_scanned_title_files
                .remove(title_id)
                .map(|files| files.len())
                .unwrap_or(0);
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
        let snapshot = {
            let mut state = self.state.lock().await;
            let Some(session) = state.sessions.get(session_id) else {
                return None;
            };
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
            state.title_sessions.retain(|_, value| value != session_id);
            let active_title_ids: HashSet<String> = state.title_sessions.keys().cloned().collect();
            state
                .pre_scanned_title_files
                .retain(|title_id, _| active_title_ids.contains(title_id));
            session
        };
        let _ = self.broadcast.send(snapshot.clone());
        Some(snapshot)
    }

    pub async fn fail_session(&self, session_id: &str) -> Option<LibraryScanSession> {
        let snapshot = {
            let mut state = self.state.lock().await;
            let mut session = state.sessions.remove(session_id)?;
            session.updated_at = Utc::now();
            session.metadata_total_known = true;
            session.file_total_known = true;
            session.status = LibraryScanStatus::Failed;
            state.facet_sessions.remove(&session.facet);
            state.title_sessions.retain(|_, value| value != session_id);
            let active_title_ids: HashSet<String> = state.title_sessions.keys().cloned().collect();
            state
                .pre_scanned_title_files
                .retain(|title_id, _| active_title_ids.contains(title_id));
            session
        };
        let _ = self.broadcast.send(snapshot.clone());
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
        let _ = self.broadcast.send(snapshot.clone());
        Some(snapshot)
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
}
