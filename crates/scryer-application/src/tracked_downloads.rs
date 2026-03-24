//! TrackedDownloads — scryer-side download lifecycle state machine (plan 055).
//!
//! Maintains an in-memory cache of active downloads, each enriched with title
//! resolution metadata and driven through a workflow state machine independent
//! of the download client's reported status.

use chrono::{DateTime, Utc};
use scryer_domain::{
    DownloadQueueItem, ImportStatus, TitleMatchType, TrackedDownloadState,
    TrackedDownloadStatus,
};
use std::collections::{HashMap, HashSet};
use tokio::sync::{mpsc, oneshot};

use crate::{AppResult, AppUseCase, DownloadSubmission};

// ── TrackedDownload ──────────────────────────────────────────────────────────

/// A download being tracked through scryer's import workflow.
#[derive(Clone, Debug)]
pub struct TrackedDownload {
    /// Composite key: "{client_type}:{download_client_item_id}".
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
}

// ── TrackedDownloadService ───────────────────────────────────────────────────

/// In-memory cache of tracked downloads with title resolution and state management.
pub struct TrackedDownloadService {
    cache: HashMap<String, TrackedDownload>,
}

impl TrackedDownloadService {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Create or update a tracked download from a client item snapshot.
    ///
    /// On first see: resolves title, checks for terminal state in DB.
    /// On update: refreshes client_item but preserves scryer state if past Downloading.
    pub async fn track(
        &mut self,
        app: &AppUseCase,
        client_item: DownloadQueueItem,
    ) {
        let id = tracked_download_id(
            &client_item.client_type,
            &client_item.download_client_item_id,
        );

        if self.cache.contains_key(&id) {
            let existing = self.cache.get_mut(&id).unwrap();
            // Update the client snapshot but preserve scryer state if not Downloading.
            if existing.state == TrackedDownloadState::Downloading {
                existing.status = TrackedDownloadStatus::Ok;
                existing.status_messages.clear();
            }
            existing.client_item = client_item;
            existing.is_trackable = true;
            return;
        }

        // First time seeing this download — build, resolve, and insert.
        let td = Self::build_new_tracked_download(app, id.clone(), client_item).await;
        self.cache.insert(id, td);
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
        };

        Self::resolve_title(app, &mut td).await;
        Self::reconstruct_state(app, &mut td).await;
        td
    }

    pub fn find(&self, id: &str) -> Option<&TrackedDownload> {
        self.cache.get(id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut TrackedDownload> {
        self.cache.get_mut(id)
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

    /// Mark downloads no longer visible in any client as untrackable.
    pub fn update_trackable(&mut self, seen_ids: &HashSet<String>) {
        for td in self.cache.values_mut() {
            if !seen_ids.contains(&td.id) {
                td.is_trackable = false;
            }
        }
    }

    /// Remove a download from the cache (after terminal state).
    pub fn stop_tracking(&mut self, id: &str) {
        self.cache.remove(id);
    }

    /// Persist a terminal state to download_submissions.
    pub async fn persist_terminal_state(
        &self,
        app: &AppUseCase,
        id: &str,
        state: TrackedDownloadState,
    ) {
        if !state.is_terminal() {
            return;
        }
        let Some(td) = self.cache.get(id) else {
            return;
        };
        if let Err(e) = app
            .services
            .download_submissions
            .update_tracked_state(
                &td.client_type,
                &td.client_item.download_client_item_id,
                state.as_str(),
            )
            .await
        {
            tracing::warn!(
                error = %e,
                id,
                state = state.as_str(),
                "failed to persist tracked download terminal state"
            );
        }
    }

    // ── Title Resolution ─────────────────────────────────────────────────

    async fn resolve_title(app: &AppUseCase, td: &mut TrackedDownload) {
        // 1. download_submissions lookup (highest confidence).
        if let Ok(Some(sub)) = app
            .services
            .download_submissions
            .find_by_client_item_id(
                &td.client_type,
                &td.client_item.download_client_item_id,
            )
            .await
        {
            if !sub.title_id.is_empty() {
                td.title_id = Some(sub.title_id.clone());
                td.facet = Some(sub.facet.clone());
                td.source_title = sub.source_title.clone();
                td.match_type = TitleMatchType::Submission;
                return;
            }
        }

        // 2. Embedded client parameters (*scryer_title_id).
        if let Some(title_id) = td.client_item.title_id.as_deref().filter(|s| !s.is_empty()) {
            // Cross-validate: does this title still exist?
            if let Ok(Some(_)) = app.services.titles.get_by_id(title_id).await {
                td.title_id = Some(title_id.to_string());
                td.match_type = TitleMatchType::ClientParameter;
                return;
            }
        }

        // 3 + 4: parse_release_metadata and ID-based lookup are more complex
        //         and depend on the facet handlers. Leave as Unmatched for now;
        //         the completed handler will block auto-import.
        //
        // Insert a stub download_submissions row for foreign downloads so they
        // get a tracked_state column for restart reconstruction.
        let _ = app
            .services
            .download_submissions
            .record_submission(DownloadSubmission {
                title_id: String::new(),
                facet: td.facet.clone().unwrap_or_default(),
                download_client_type: td.client_type.clone(),
                download_client_item_id: td.client_item.download_client_item_id.clone(),
                source_title: Some(td.client_item.title_name.clone()),
                collection_id: None,
            })
            .await;
    }

    /// Reconstruct state from persistent storage after restart.
    async fn reconstruct_state(app: &AppUseCase, td: &mut TrackedDownload) {
        // Check download_submissions.tracked_state for terminal states.
        if let Ok(Some(tracked_state)) = app
            .services
            .download_submissions
            .get_tracked_state(
                &td.client_type,
                &td.client_item.download_client_item_id,
            )
            .await
        {
            if let Some(state) = TrackedDownloadState::from_str_opt(&tracked_state) {
                if state.is_terminal() {
                    td.state = state;
                    return;
                }
            }
        }

        // Check imports table for prior completion.
        if let Ok(Some(import_record)) = app
            .services
            .imports
            .get_import_by_source_ref(
                &td.client_type,
                &td.client_item.download_client_item_id,
            )
            .await
        {
            if import_record.status == ImportStatus::Completed {
                td.state = TrackedDownloadState::Imported;
                // Persist terminal state that wasn't persisted before restart.
                let _ = app
                    .services
                    .download_submissions
                    .update_tracked_state(
                        &td.client_type,
                        &td.client_item.download_client_item_id,
                        "imported",
                    )
                    .await;
            }
        }

        // Default: Downloading (will be re-evaluated by check cycle).
    }
}

// ── Command Channel ──────────────────────────────────────────────────────────

/// Commands sent from GraphQL mutations to the poller's TrackedDownloadService.
pub enum TrackedDownloadCommand {
    Ignore {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    MarkFailed {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    RetryImport {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    AssignTitle {
        id: String,
        title_id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
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
            .map_err(|_| crate::AppError::Repository("tracked download service unavailable".into()))?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn mark_failed(&self, id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::MarkFailed {
                id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| crate::AppError::Repository("tracked download service unavailable".into()))?;
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
            .map_err(|_| crate::AppError::Repository("tracked download service unavailable".into()))?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn assign_title(&self, id: String, title_id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::AssignTitle {
                id,
                title_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| crate::AppError::Repository("tracked download service unavailable".into()))?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

pub fn tracked_download_id(client_type: &str, item_id: &str) -> String {
    format!("{client_type}:{item_id}")
}
