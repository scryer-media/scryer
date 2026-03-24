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
    /// Whether import() has been called at least once. Prevents check() from
    /// re-evaluating a post-import ImportBlocked back to ImportPending.
    pub import_attempted: bool,
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
#[derive(Default)]
pub struct TrackedDownloadService {
    cache: HashMap<String, TrackedDownload>,
}

impl TrackedDownloadService {
    pub fn new() -> Self {
        Self::default()
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
            import_attempted: false,
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

    pub fn get_trackable_ids(&self) -> Vec<String> {
        self.cache
            .values()
            .filter(|td| td.is_trackable && !td.state.is_terminal())
            .map(|td| td.id.clone())
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
            && !sub.title_id.is_empty()
        {
            td.title_id = Some(sub.title_id.clone());
            td.facet = Some(sub.facet.clone());
            td.source_title = sub.source_title.clone();
            td.match_type = TitleMatchType::Submission;
            return;
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
            && let Some(state) = TrackedDownloadState::from_str_opt(&tracked_state)
            && state.is_terminal()
        {
            td.state = state;
            return;
        }

        // Fall back to the latest import record for restart recovery if the
        // tracked state was not persisted before shutdown.
        if let Ok(Some(import_record)) = app
            .services
            .imports
            .get_import_by_source_ref(
                &td.client_type,
                &td.client_item.download_client_item_id,
            )
            .await
            && import_record.status == ImportStatus::Completed
        {
            td.state = TrackedDownloadState::Imported;
            let _ = app
                .services
                .download_submissions
                .update_tracked_state(
                    &td.client_type,
                    &td.client_item.download_client_item_id,
                    TrackedDownloadState::Imported.as_str(),
                )
                .await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullEventRepository,
        NullIndexerClient, NullQualityProfileRepository, NullReleaseAttemptRepository,
        NullShowRepository, NullTitleRepository, NullUserRepository,
    };
    use crate::{
        AppError, AppResult, AppServices, AppUseCase, DownloadSubmissionRepository,
        FacetRegistry, ImportRepository, IndexerConfigRepository, JwtAuthConfig,
    };
    use async_trait::async_trait;
    use scryer_domain::{
        DownloadQueueState, Id, ImportRecord, ImportType,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestDownloadSubmissionRepo {
        submission: Option<crate::DownloadSubmission>,
        tracked_state: Option<String>,
        tracked_state_updates: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl DownloadSubmissionRepository for TestDownloadSubmissionRepo {
        async fn record_submission(&self, _: crate::DownloadSubmission) -> AppResult<()> {
            Ok(())
        }

        async fn find_by_client_item_id(
            &self,
            _: &str,
            _: &str,
        ) -> AppResult<Option<crate::DownloadSubmission>> {
            Ok(self.submission.clone())
        }

        async fn list_for_title(&self, _: &str) -> AppResult<Vec<crate::DownloadSubmission>> {
            Ok(vec![])
        }

        async fn delete_for_title(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn delete_by_client_item_id(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update_tracked_state(&self, _: &str, _: &str, tracked_state: &str) -> AppResult<()> {
            self.tracked_state_updates
                .lock()
                .await
                .push(tracked_state.to_string());
            Ok(())
        }

        async fn get_tracked_state(&self, _: &str, _: &str) -> AppResult<Option<String>> {
            Ok(self.tracked_state.clone())
        }
    }

    #[derive(Default)]
    struct TestImportRepo {
        import_record: Option<ImportRecord>,
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
            _: &str,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<i64>,
            _: Option<i64>,
            _: Option<bool>,
            _: Option<bool>,
            _: Option<bool>,
            _: Option<String>,
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
            _: String,
            _: String,
            _: String,
            _: String,
        ) -> AppResult<String> {
            Ok(String::new())
        }

        async fn get_import_by_id(&self, _: &str) -> AppResult<Option<ImportRecord>> {
            Ok(None)
        }

        async fn get_import_by_source_ref(&self, _: &str, _: &str) -> AppResult<Option<ImportRecord>> {
            Ok(self.import_record.clone())
        }

        async fn update_import_status(
            &self,
            _: &str,
            _: ImportStatus,
            _: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn recover_stale_processing_imports(&self, _: i64) -> AppResult<u64> {
            Ok(0)
        }

        async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
            Ok(vec![])
        }

        async fn is_already_imported(&self, _: &str, _: &str) -> AppResult<bool> {
            Ok(false)
        }

        async fn list_imports(&self, _: usize) -> AppResult<Vec<ImportRecord>> {
            Ok(vec![])
        }
    }

    fn build_app(
        download_submissions: Arc<TestDownloadSubmissionRepo>,
        imports: Arc<TestImportRepo>,
    ) -> AppUseCase {
        let mut services = AppServices::with_default_channels(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(NullEventRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::null_repositories::NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        );
        services.download_submissions = download_submissions;
        services.imports = imports;

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn build_client_item() -> DownloadQueueItem {
        DownloadQueueItem {
            id: Id::new().0,
            title_id: None,
            title_name: "Restart Recovery Show".to_string(),
            facet: Some("series".to_string()),
            client_id: "client-1".to_string(),
            client_name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            state: DownloadQueueState::Completed,
            progress_percent: 100,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "dl-1".to_string(),
            import_status: None,
            import_error_message: None,
            imported_at: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: vec![],
            tracked_match_type: None,
        }
    }

    #[tokio::test]
    async fn reconstruct_state_recovers_imported_from_completed_import_record() {
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: Some(crate::DownloadSubmission {
                title_id: "title-1".to_string(),
                facet: "series".to_string(),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "dl-1".to_string(),
                source_title: Some("Restart Recovery Show".to_string()),
                collection_id: None,
            }),
            tracked_state: None,
            tracked_state_updates: Arc::new(Mutex::new(vec![])),
        });
        let imports = Arc::new(TestImportRepo {
            import_record: Some(ImportRecord {
                id: Id::new().0,
                source_system: "nzbget".to_string(),
                source_ref: "dl-1".to_string(),
                import_type: ImportType::TvDownload,
                status: ImportStatus::Completed,
                payload_json: "{}".to_string(),
                result_json: None,
                started_at: None,
                finished_at: None,
                created_at: "now".to_string(),
                updated_at: "now".to_string(),
            }),
        });
        let app = build_app(download_submissions.clone(), imports);
        let mut tracker = TrackedDownloadService::new();

        tracker.track(&app, build_client_item()).await;

        let tracked = tracker.find("nzbget:dl-1").expect("tracked download");
        assert_eq!(tracked.state, TrackedDownloadState::Imported);
        assert_eq!(
            download_submissions.tracked_state_updates.lock().await.as_slice(),
            ["imported"]
        );
    }
}
