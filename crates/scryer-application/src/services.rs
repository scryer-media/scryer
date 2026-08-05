use super::*;
use crate::ports::IndexerCapsSnapshotRefresher;
use scryer_runtime_info::{BinaryClass, BinaryLane};
use std::io::{Read, Write};

/// In-process guard table for download-submission dedupe and scope ownership.
///
/// Scryer is intentionally single-instance, so the database lookup remains the
/// authoritative duplicate check while this table serializes same-process races.
#[derive(Clone, Default)]
pub struct DownloadSubmissionGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl DownloadSubmissionGuardTable {
    async fn acquire_key(&self, key: String) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }

    pub async fn acquire(
        &self,
        title_id: &str,
        request_signature: Option<&str>,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let signature = request_signature?;
        let key = format!("{title_id}:{signature}");
        Some(self.acquire_key(key).await)
    }

    pub async fn acquire_scope(
        &self,
        title_id: &str,
        _scope: &SubmissionScope,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        self.acquire_key(format!("{title_id}:scope")).await
    }
}

/// In-process guard table for failed-download handling dedupe.
///
/// This serializes same-process races between the grabbed-item failure sweep and
/// tracked-download failure processing while the persisted blocklist row remains
/// the authoritative record of whether failure side effects already ran.
#[derive(Clone, Default)]
pub struct DownloadFailureGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl DownloadFailureGuardTable {
    async fn acquire_key(&self, key: String) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }

    pub async fn acquire(
        &self,
        title_id: Option<&str>,
        client_id: &str,
        client_type: &str,
        client_item_id: &str,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let title_id = title_id.map(str::trim).filter(|value| !value.is_empty())?;
        let key = format!(
            "{title_id}:{}:{}:{}",
            client_id.trim(),
            client_type.trim().to_ascii_lowercase(),
            client_item_id.trim()
        );
        Some(self.acquire_key(key).await)
    }

    pub async fn acquire_release_or_client_item(
        &self,
        title_id: Option<&str>,
        source_title: Option<&str>,
        client_id: &str,
        client_type: &str,
        client_item_id: &str,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let title_id = title_id.map(str::trim).filter(|value| !value.is_empty())?;
        if let Some(source_title) = source_title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
        {
            return Some(
                self.acquire_key(format!("release:{title_id}:{source_title}"))
                    .await,
            );
        }

        self.acquire(Some(title_id), client_id, client_type, client_item_id)
            .await
    }
}

#[derive(Clone, Default)]
pub struct BackupExecutionGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

pub type InteractiveOperationGuardTable = BackupExecutionGuardTable;

impl BackupExecutionGuardTable {
    async fn lock_for_key(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
            existing
        } else {
            let created = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(key, Arc::downgrade(&created));
            created
        }
    }

    pub async fn try_acquire(&self, key: &str) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let lock = self.lock_for_key(key.to_string()).await;
        lock.try_lock_owned().ok()
    }
}

#[derive(Clone, Default)]
pub struct PluginOperationGuardTable {
    locks: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl PluginOperationGuardTable {
    pub async fn acquire(&self, plugin_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let key = plugin_id.trim().to_ascii_lowercase();
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(existing) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&created));
                created
            }
        };

        lock.lock_owned().await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginInstallOperationKind {
    Install,
    Upgrade,
}

impl PluginInstallOperationKind {
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginInstallState {
    Downloading,
    Verifying,
    Installing,
    Succeeded,
    Failed,
}

impl PluginInstallState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Downloading => "Downloading",
            Self::Verifying => "Verifying",
            Self::Installing => "Installing",
            Self::Succeeded => "Plugin installed",
            Self::Failed => "Plugin install failed",
        }
    }

    pub const fn step_index(self) -> i32 {
        match self {
            Self::Downloading => 1,
            Self::Verifying => 2,
            Self::Installing => 3,
            Self::Succeeded | Self::Failed => 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginInstallProgressSnapshot {
    pub plugin_id: String,
    pub operation_kind: PluginInstallOperationKind,
    pub state: PluginInstallState,
    pub label: String,
    pub step_index: i32,
    pub step_count: i32,
    pub message: Option<String>,
    pub error: Option<String>,
}

impl PluginInstallProgressSnapshot {
    const STEP_COUNT: i32 = 3;

    fn new(
        plugin_id: String,
        operation_kind: PluginInstallOperationKind,
        state: PluginInstallState,
        message: Option<String>,
        error: Option<String>,
    ) -> Self {
        Self {
            plugin_id,
            operation_kind,
            state,
            label: state.label().to_string(),
            step_index: state.step_index(),
            step_count: Self::STEP_COUNT,
            message,
            error,
        }
    }

    fn with_state(
        &self,
        state: PluginInstallState,
        message: Option<String>,
        error: Option<String>,
    ) -> Self {
        Self::new(
            self.plugin_id.clone(),
            self.operation_kind,
            state,
            message,
            error,
        )
    }
}

#[derive(Debug)]
struct ActivePluginInstallOperation {
    actor_user_id: String,
    snapshot_key: (String, String),
    generation: u64,
}

#[derive(Clone, Debug)]
struct PluginInstallSnapshotHandle {
    generation: u64,
    active: bool,
    tx: tokio::sync::watch::Sender<PluginInstallProgressSnapshot>,
}

#[derive(Default)]
struct PluginInstallOrchestratorState {
    next_generation: u64,
    active_by_plugin: HashMap<String, ActivePluginInstallOperation>,
    snapshots_by_actor_plugin: HashMap<(String, String), PluginInstallSnapshotHandle>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginInstallInProgressError {
    pub plugin_id: String,
}

#[derive(Clone, Default)]
pub struct PluginInstallOrchestrator {
    state: Arc<tokio::sync::Mutex<PluginInstallOrchestratorState>>,
}

impl PluginInstallOrchestrator {
    const FINISHED_SNAPSHOT_TTL: tokio::time::Duration = tokio::time::Duration::from_secs(15);

    fn normalize_plugin_key(plugin_id: &str) -> String {
        plugin_id.trim().to_ascii_lowercase()
    }

    fn actor_snapshot_key(actor_user_id: &str, plugin_key: &str) -> (String, String) {
        (actor_user_id.to_string(), plugin_key.to_string())
    }

    pub async fn begin(
        &self,
        actor_user_id: &str,
        plugin_id: &str,
        operation_kind: PluginInstallOperationKind,
    ) -> Result<PluginInstallProgressSnapshot, PluginInstallInProgressError> {
        let plugin_key = Self::normalize_plugin_key(plugin_id);
        let snapshot_key = Self::actor_snapshot_key(actor_user_id, &plugin_key);
        let mut state = self.state.lock().await;
        if state.active_by_plugin.contains_key(&plugin_key) {
            return Err(PluginInstallInProgressError {
                plugin_id: plugin_key,
            });
        }

        state.next_generation += 1;
        let generation = state.next_generation;
        let snapshot = PluginInstallProgressSnapshot::new(
            plugin_key.clone(),
            operation_kind,
            PluginInstallState::Downloading,
            None,
            None,
        );
        let (tx, _rx) = tokio::sync::watch::channel(snapshot.clone());
        state.snapshots_by_actor_plugin.insert(
            snapshot_key.clone(),
            PluginInstallSnapshotHandle {
                generation,
                active: true,
                tx,
            },
        );
        state.active_by_plugin.insert(
            plugin_key,
            ActivePluginInstallOperation {
                actor_user_id: actor_user_id.to_string(),
                snapshot_key,
                generation,
            },
        );
        Ok(snapshot)
    }

    pub async fn subscribe(
        &self,
        actor_user_id: &str,
        plugin_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<PluginInstallProgressSnapshot>> {
        let plugin_key = Self::normalize_plugin_key(plugin_id);
        let snapshot_key = Self::actor_snapshot_key(actor_user_id, &plugin_key);
        let state = self.state.lock().await;
        state
            .snapshots_by_actor_plugin
            .get(&snapshot_key)
            .map(|handle| handle.tx.subscribe())
    }

    pub async fn active_plugin_ids_for_actor(&self, actor_user_id: &str) -> HashSet<String> {
        let state = self.state.lock().await;
        state
            .snapshots_by_actor_plugin
            .iter()
            .filter(|((snapshot_actor, _), handle)| {
                snapshot_actor == actor_user_id && handle.active
            })
            .map(|((_, plugin_id), _)| plugin_id.clone())
            .collect()
    }

    pub async fn transition(
        &self,
        actor_user_id: &str,
        plugin_id: &str,
        next_state: PluginInstallState,
        message: Option<String>,
        error: Option<String>,
    ) {
        let plugin_key = Self::normalize_plugin_key(plugin_id);
        let snapshot_key = Self::actor_snapshot_key(actor_user_id, &plugin_key);
        let mut state = self.state.lock().await;
        let generation = {
            let Some(handle) = state.snapshots_by_actor_plugin.get_mut(&snapshot_key) else {
                return;
            };
            let current = handle.tx.borrow().clone();
            let _ = handle
                .tx
                .send(current.with_state(next_state, message, error));
            if matches!(
                next_state,
                PluginInstallState::Succeeded | PluginInstallState::Failed
            ) {
                handle.active = false;
                Some(handle.generation)
            } else {
                None
            }
        };
        if let Some(generation) = generation {
            let should_release = state
                .active_by_plugin
                .get(&plugin_key)
                .is_some_and(|active| {
                    active.actor_user_id == actor_user_id
                        && active.generation == generation
                        && active.snapshot_key == snapshot_key
                });
            if should_release {
                state.active_by_plugin.remove(&plugin_key);
            }
            drop(state);
            self.schedule_finished_snapshot_cleanup(snapshot_key, generation);
        }
    }

    fn schedule_finished_snapshot_cleanup(&self, snapshot_key: (String, String), generation: u64) {
        let orchestrator = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Self::FINISHED_SNAPSHOT_TTL).await;
            let mut state = orchestrator.state.lock().await;
            if state
                .snapshots_by_actor_plugin
                .get(&snapshot_key)
                .is_some_and(|handle| handle.generation == generation && !handle.active)
            {
                state.snapshots_by_actor_plugin.remove(&snapshot_key);
            }
        });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalImportMonitorWarmupStatus {
    Queued,
    Running,
    Completed,
    Canceled,
    Failed,
}

impl ExternalImportMonitorWarmupStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Canceled | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalImportMonitorWarmupPhase {
    LoadingIndexers,
    LoadingMovies,
    LoadingSeries,
    LoadingEpisodes,
    BuildingSnapshot,
    Ready,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExternalImportMonitorWarmupPhaseProgress {
    pub total: i32,
    pub completed: i32,
    pub failed: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalImportMonitorWarmupProgressSnapshot {
    pub session_id: String,
    pub status: ExternalImportMonitorWarmupStatus,
    pub phase: ExternalImportMonitorWarmupPhase,
    pub started_at: String,
    pub updated_at: String,
    pub overall_total_known: bool,
    pub overall_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub movies_total_known: bool,
    pub movies_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub series_total_known: bool,
    pub series_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub episode_fetch_total_known: bool,
    pub episode_fetch_expected_total: Option<i32>,
    pub episode_fetch_expected_monitored_total: Option<i32>,
    pub episode_fetch_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub snapshot_build_total_known: bool,
    pub snapshot_build_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub matched_movie_count: i32,
    pub matched_series_count: i32,
    pub unmatched_movie_count: i32,
    pub unmatched_series_count: i32,
    pub ambiguous_movie_count: i32,
    pub ambiguous_series_count: i32,
    pub error_message: Option<String>,
}

impl ExternalImportMonitorWarmupProgressSnapshot {
    pub fn new(session_id: String) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            session_id,
            status: ExternalImportMonitorWarmupStatus::Queued,
            phase: ExternalImportMonitorWarmupPhase::LoadingMovies,
            started_at: now.clone(),
            updated_at: now,
            overall_total_known: false,
            overall_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            movies_total_known: false,
            movies_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            series_total_known: false,
            series_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            episode_fetch_total_known: false,
            episode_fetch_expected_total: None,
            episode_fetch_expected_monitored_total: None,
            episode_fetch_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            snapshot_build_total_known: false,
            snapshot_build_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            matched_movie_count: 0,
            matched_series_count: 0,
            unmatched_movie_count: 0,
            unmatched_series_count: 0,
            ambiguous_movie_count: 0,
            ambiguous_series_count: 0,
            error_message: None,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now().to_rfc3339();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ExternalImportArrSourceKind {
    Sonarr,
    Radarr,
}

impl ExternalImportArrSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sonarr => "sonarr",
            Self::Radarr => "radarr",
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ExternalImportArrSourceSeriesEntry {
    pub series: crate::external_import::ArrSeries,
    pub episodes: Vec<crate::external_import::ArrEpisode>,
}

#[derive(Clone, Debug)]
pub struct ExternalImportArrSourceWarmupResult {
    pub source_key: String,
    pub kind: ExternalImportArrSourceKind,
    pub base_url: String,
    pub version: Option<String>,
    pub root_folders: Vec<crate::external_import::ArrRootFolder>,
    pub title_root_paths: Vec<String>,
    pub naming_config: Option<crate::external_import::ArrNamingConfig>,
    pub media_management_config: Option<crate::external_import::ArrMediaManagementConfig>,
    pub metadata_providers: Vec<crate::external_import::ArrMetadataProvider>,
    pub quality_profiles: Vec<crate::external_import::ArrQualityProfile>,
    pub signal_warnings: Vec<String>,
    pub download_clients: Vec<crate::external_import::ArrDownloadClient>,
    pub indexers: Vec<crate::external_import::ArrIndexer>,
}

#[derive(Clone, Debug)]
pub struct ExternalImportProwlarrWarmupResult {
    pub base_url: String,
    /// The operator-entered Prowlarr API key the discovery ran with. Preview
    /// merges it into the import group so downstream consumers see the real
    /// credential, never a placeholder.
    pub api_key: String,
    pub version: Option<String>,
    pub plan: crate::IndexerSyncPlan,
}

#[derive(Clone)]
pub struct ExternalImportMonitorWarmupBeginResult {
    pub snapshot: ExternalImportMonitorWarmupProgressSnapshot,
    pub created: bool,
    pub cancel_token: tokio_util::sync::CancellationToken,
    pub replaced_session_id: Option<String>,
}

#[derive(Clone)]
struct ExternalImportMonitorWarmupSessionHandle {
    actor_user_id: String,
    connection_fingerprint: String,
    claimed: bool,
    cancel_token: tokio_util::sync::CancellationToken,
    tx: tokio::sync::watch::Sender<ExternalImportMonitorWarmupProgressSnapshot>,
    scan_hints: Option<crate::LibraryScanHintSet>,
    arr_source_result: Option<ExternalImportArrSourceWarmupResult>,
    prowlarr_result: Option<ExternalImportProwlarrWarmupResult>,
}

#[derive(Default)]
struct ExternalImportMonitorWarmupOrchestratorState {
    session_ids_by_actor_fingerprint: HashMap<(String, String), String>,
    sessions_by_id: HashMap<String, ExternalImportMonitorWarmupSessionHandle>,
}

#[derive(Clone, Default)]
pub struct ExternalImportMonitorWarmupOrchestrator {
    state: Arc<tokio::sync::Mutex<ExternalImportMonitorWarmupOrchestratorState>>,
}

impl ExternalImportMonitorWarmupOrchestrator {
    pub async fn begin(
        &self,
        actor_user_id: &str,
        connection_fingerprint: &str,
        initial_snapshot: ExternalImportMonitorWarmupProgressSnapshot,
    ) -> ExternalImportMonitorWarmupBeginResult {
        let actor_key = (
            actor_user_id.to_string(),
            connection_fingerprint.to_string(),
        );
        let mut state = self.state.lock().await;
        let mut replaced_session_id = None;

        if let Some(existing_session_id) = state
            .session_ids_by_actor_fingerprint
            .get(&actor_key)
            .cloned()
        {
            if let Some(existing_handle) = state.sessions_by_id.get(&existing_session_id) {
                let existing_snapshot = existing_handle.tx.borrow().clone();
                if matches!(
                    existing_snapshot.status,
                    ExternalImportMonitorWarmupStatus::Queued
                        | ExternalImportMonitorWarmupStatus::Running
                ) {
                    return ExternalImportMonitorWarmupBeginResult {
                        snapshot: existing_snapshot,
                        created: false,
                        cancel_token: existing_handle.cancel_token.clone(),
                        replaced_session_id: None,
                    };
                }
            }

            state.session_ids_by_actor_fingerprint.remove(&actor_key);
            state.sessions_by_id.remove(&existing_session_id);
            replaced_session_id = Some(existing_session_id);
        }

        let cancel_token = tokio_util::sync::CancellationToken::new();
        let (tx, _rx) = tokio::sync::watch::channel(initial_snapshot.clone());
        state
            .session_ids_by_actor_fingerprint
            .insert(actor_key, initial_snapshot.session_id.clone());
        state.sessions_by_id.insert(
            initial_snapshot.session_id.clone(),
            ExternalImportMonitorWarmupSessionHandle {
                actor_user_id: actor_user_id.to_string(),
                connection_fingerprint: connection_fingerprint.to_string(),
                claimed: false,
                cancel_token: cancel_token.clone(),
                tx,
                scan_hints: None,
                arr_source_result: None,
                prowlarr_result: None,
            },
        );

        ExternalImportMonitorWarmupBeginResult {
            snapshot: initial_snapshot,
            created: true,
            cancel_token,
            replaced_session_id,
        }
    }

    pub async fn subscribe(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<ExternalImportMonitorWarmupProgressSnapshot>> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id).then(|| handle.tx.subscribe())
        })
    }

    pub async fn snapshot(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<ExternalImportMonitorWarmupProgressSnapshot> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id).then(|| handle.tx.borrow().clone())
        })
    }

    pub async fn update(
        &self,
        session_id: &str,
        snapshot: ExternalImportMonitorWarmupProgressSnapshot,
    ) -> bool {
        let state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get(session_id) else {
            return false;
        };
        handle.tx.send_replace(snapshot);
        true
    }

    pub async fn set_scan_hints(
        &self,
        actor_user_id: &str,
        session_id: &str,
        scan_hints: crate::LibraryScanHintSet,
    ) -> bool {
        let mut state = self.state.lock().await;
        if !state.sessions_by_id.contains_key(session_id) {
            if scan_hints.is_empty() {
                return false;
            }
            let mut snapshot =
                ExternalImportMonitorWarmupProgressSnapshot::new(session_id.to_string());
            snapshot.status = ExternalImportMonitorWarmupStatus::Completed;
            snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
            let (tx, _rx) = tokio::sync::watch::channel(snapshot);
            state.sessions_by_id.insert(
                session_id.to_string(),
                ExternalImportMonitorWarmupSessionHandle {
                    actor_user_id: actor_user_id.to_string(),
                    connection_fingerprint: session_id.to_string(),
                    claimed: true,
                    cancel_token: tokio_util::sync::CancellationToken::new(),
                    tx,
                    scan_hints: None,
                    arr_source_result: None,
                    prowlarr_result: None,
                },
            );
        }
        let Some(handle) = state.sessions_by_id.get_mut(session_id) else {
            return false;
        };
        if handle.actor_user_id != actor_user_id {
            return false;
        }
        handle.scan_hints = (!scan_hints.is_empty()).then_some(scan_hints);
        true
    }

    pub async fn scan_hints(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<crate::LibraryScanHintSet> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id)
                .then(|| handle.scan_hints.clone())
                .flatten()
        })
    }

    pub async fn set_arr_source_result(
        &self,
        session_id: &str,
        result: ExternalImportArrSourceWarmupResult,
    ) -> bool {
        let mut state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get_mut(session_id) else {
            return false;
        };
        handle.arr_source_result = Some(result);
        true
    }

    pub async fn arr_source_result(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<ExternalImportArrSourceWarmupResult> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id)
                .then(|| handle.arr_source_result.clone())
                .flatten()
        })
    }

    pub async fn set_prowlarr_result(
        &self,
        session_id: &str,
        result: ExternalImportProwlarrWarmupResult,
    ) -> bool {
        let mut state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get_mut(session_id) else {
            return false;
        };
        handle.prowlarr_result = Some(result);
        true
    }

    pub async fn prowlarr_result(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<ExternalImportProwlarrWarmupResult> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id)
                .then(|| handle.prowlarr_result.clone())
                .flatten()
        })
    }

    pub async fn cancel(&self, actor_user_id: &str, session_id: &str) -> bool {
        let mut state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get_mut(session_id) else {
            return false;
        };
        if handle.actor_user_id != actor_user_id || handle.claimed {
            return false;
        }

        let mut snapshot = handle.tx.borrow().clone();
        if !snapshot.status.is_terminal() {
            snapshot.status = ExternalImportMonitorWarmupStatus::Canceled;
            snapshot.error_message = None;
            snapshot.touch();
            handle.tx.send_replace(snapshot);
        }
        handle.cancel_token.cancel();

        state
            .session_ids_by_actor_fingerprint
            .retain(|_, existing_session_id| existing_session_id != session_id);
        true
    }

    pub async fn claim(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<ExternalImportMonitorWarmupProgressSnapshot> {
        let mut state = self.state.lock().await;
        let handle = state.sessions_by_id.get_mut(session_id)?;
        if handle.actor_user_id != actor_user_id {
            return None;
        }
        handle.claimed = true;
        Some(handle.tx.borrow().clone())
    }

    pub async fn connection_fingerprint(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<String> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id).then(|| handle.connection_fingerprint.clone())
        })
    }

    pub async fn remove(&self, actor_user_id: &str, session_id: &str) -> bool {
        let mut state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get(session_id) else {
            return false;
        };
        if handle.actor_user_id != actor_user_id {
            return false;
        }
        state.sessions_by_id.remove(session_id);
        state
            .session_ids_by_actor_fingerprint
            .retain(|_, existing_session_id| existing_session_id != session_id);
        true
    }

    pub async fn prune_terminal_older_than(&self, max_age: chrono::Duration) -> Vec<String> {
        let mut state = self.state.lock().await;
        let now = Utc::now();
        let mut removed = Vec::new();
        let session_ids = state.sessions_by_id.keys().cloned().collect::<Vec<_>>();

        for session_id in session_ids {
            let Some(handle) = state.sessions_by_id.get(&session_id) else {
                continue;
            };
            if !handle.connection_fingerprint.starts_with("arr-source=")
                && !handle
                    .connection_fingerprint
                    .starts_with("prowlarr-source=")
            {
                continue;
            }
            let snapshot = handle.tx.borrow().clone();
            if !snapshot.status.is_terminal() {
                continue;
            }
            let Ok(updated_at) = chrono::DateTime::parse_from_rfc3339(&snapshot.updated_at) else {
                continue;
            };
            if now.signed_duration_since(updated_at.with_timezone(&Utc)) < max_age {
                continue;
            }

            state.sessions_by_id.remove(&session_id);
            state
                .session_ids_by_actor_fingerprint
                .retain(|_, existing_session_id| existing_session_id != &session_id);
            removed.push(session_id);
        }

        removed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ProviderCatalogFamily {
    Subtitle,
    Notification,
    Indexer,
    DownloadClient,
    ArchiveExtractor,
}

impl ProviderCatalogFamily {
    pub const fn all() -> [Self; 5] {
        [
            Self::Subtitle,
            Self::Notification,
            Self::Indexer,
            Self::DownloadClient,
            Self::ArchiveExtractor,
        ]
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Subtitle => "subtitle",
            Self::Notification => "notification",
            Self::Indexer => "indexer",
            Self::DownloadClient => "download_client",
            Self::ArchiveExtractor => "archive_extractor",
        }
    }
}

#[derive(Clone)]
pub struct AppRuntimeEventState {
    pub domain_event_broadcast: broadcast::Sender<i64>,
    /// Wake-only high-water hints for the notification dispatcher. Send-side filtering keeps
    /// operational bursts from waking it, while persisted filtered replay remains authoritative.
    pub notification_event_broadcast: broadcast::Sender<i64>,
    pub import_history_broadcast: broadcast::Sender<()>,
    pub indexers_changed_broadcast: broadcast::Sender<()>,
    pub provider_catalog_changed_broadcast: broadcast::Sender<Vec<ProviderCatalogFamily>>,
    pub settings_changed_broadcast: broadcast::Sender<Vec<String>>,
}

#[derive(Clone)]
pub struct AppRuntimeCatalogState {
    pub(crate) monitored_title_matcher:
        Arc<RwLock<crate::import_title_resolution::MonitoredTitleMatcherCache>>,
    pub poster_wake: Arc<tokio::sync::Notify>,
    pub fanart_wake: Arc<tokio::sync::Notify>,
    pub(crate) title_hydration_wake: Arc<tokio::sync::Notify>,
    pub(crate) title_recommendation_refresh_queue:
        Arc<tokio::sync::Mutex<crate::catalog_workflow::TitleRecommendationRefreshQueue>>,
    pub(crate) title_recommendation_refresh_wake: Arc<tokio::sync::Notify>,
    pub image_processing_limit: Arc<Semaphore>,
    pub title_image_maintenance_lock: Arc<tokio::sync::RwLock<()>>,
    pub title_image_cache_clear_scheduled: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone)]
pub struct AppRuntimeAcquisitionState {
    pub acquisition_wake: Arc<tokio::sync::Notify>,
    pub download_submission_guards: DownloadSubmissionGuardTable,
    pub download_failure_guards: DownloadFailureGuardTable,
    pub(crate) release_candidate_passwords:
        Arc<std::sync::Mutex<HashMap<String, ReleaseCandidatePasswordTicket>>>,
    pub rss_seen_guids: Arc<tokio::sync::RwLock<HashSet<String>>>,
    pub tracked_download_handle: Option<tracked_downloads::TrackedDownloadHandle>,
    pub tracked_download_snapshot:
        Arc<tokio::sync::RwLock<HashMap<String, tracked_downloads::TrackedDownloadQueueMetadata>>>,
    pub(crate) download_client_category_ownership:
        Arc<tokio::sync::RwLock<DownloadClientCategoryOwnershipCache>>,
    /// Cancellation tokens for in-flight interactive acquisition-search jobs
    ///, keyed by job-run id — mirrors the library-scan cancel map.
    pub acquisition_search_cancellation_tokens:
        Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    /// In-memory registry of interactive release-search jobs (hotfix 0.17.1),
    /// keyed by job id. Ephemeral by design — see
    /// `catalog::interactive_release_search` for the eviction rules.
    pub(crate) interactive_release_searches: Arc<
        Mutex<
            HashMap<
                String,
                crate::catalog::interactive_release_search::InteractiveReleaseSearchJobEntry,
            >,
        >,
    >,
}

#[derive(Clone, Default)]
pub(crate) struct DownloadClientCategoryOwnershipSnapshot {
    pub(crate) default_categories: HashSet<String>,
    pub(crate) categories_by_client: HashMap<String, HashSet<String>>,
}

/// Fold a download-client category to its comparison form.
///
/// Download clients treat category names case-insensitively and echo back their
/// OWN canonical spelling: configure Scryer with `movies` and NZBGet — whose
/// `Category1.Name` is `Movies` — accepts the grab, files it under `Movies`,
/// and reports `Movies` in its history. Comparing those raw made Scryer's own
/// download look like it carried a category Scryer had never configured, so it
/// was classified foreign and filtered out of the tracked snapshot entirely:
/// never imported, never shown, and re-grabbed minutes later by the RSS sweep
/// because the wanted item was still unfilled.
pub(crate) fn normalize_owned_download_category(category: &str) -> String {
    category.trim().to_ascii_lowercase()
}

impl DownloadClientCategoryOwnershipSnapshot {
    pub(crate) fn owns_category(&self, client_id: &str, category: &str) -> bool {
        let category = normalize_owned_download_category(category);
        self.categories_by_client
            .get(client_id)
            .unwrap_or(&self.default_categories)
            .contains(&category)
    }

    /// Whether Scryer configured this category ANYWHERE — for any client, or as
    /// a default.
    ///
    /// Distinct from [`Self::owns_category`], which asks the narrower question
    /// "is this category assigned to THIS client". Both questions are useful,
    /// but only this one answers "did this download come from something Scryer
    /// set up". Users routinely move a download between clients, or point two
    /// clients at one category set, so a category that fails the per-client
    /// test is very often still Scryer's own work — treating that as foreign
    /// hides the user's download from their own activity view.
    pub(crate) fn knows_category(&self, category: &str) -> bool {
        let category = normalize_owned_download_category(category);
        self.default_categories.contains(&category)
            || self
                .categories_by_client
                .values()
                .any(|categories| categories.contains(&category))
    }
}

#[derive(Clone, Default)]
pub(crate) enum DownloadClientCategoryOwnershipCache {
    #[default]
    Uninitialized,
    Available(Arc<DownloadClientCategoryOwnershipSnapshot>),
    Unavailable,
}

pub(crate) struct ReleaseCandidatePasswordTicket {
    pub actor_id: String,
    pub title_id: String,
    pub scope_kind: String,
    pub scope_id: Option<String>,
    pub source_hint: String,
    pub source_title: String,
    pub password: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AppRuntimeImportState {
    pub external_import_warmup_orchestrator: ExternalImportMonitorWarmupOrchestrator,
    pub external_import_apply_lock: Arc<tokio::sync::Mutex<()>>,
    pub external_import_source_chunk_cleanup_done: Arc<tokio::sync::Mutex<bool>>,
    pub(crate) same_path_upgrade_guard_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone)]
pub struct AppRuntimeLibraryState {
    pub library_scan_tracker: LibraryScanTracker,
    pub library_scan_cancellation_tokens:
        Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    pub library_scan_title_walk_limit: Arc<Semaphore>,
    pub library_scan_analysis_limit: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct AppRuntimeJobState {
    pub job_run_tracker: JobRunTracker,
    pub discovery_sync_wake: Arc<tokio::sync::Notify>,
    pub backup_execution_guards: BackupExecutionGuardTable,
    pub interactive_operation_guards: InteractiveOperationGuardTable,
    pub title_deletion_lock: Arc<tokio::sync::Mutex<()>>,
    /// Single-flight guard for the interactive acquisition-search job — mirrors `title_deletion_lock`.
    pub acquisition_search_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone)]
pub struct AppRuntimeHealthState {
    pub results: Arc<tokio::sync::RwLock<Vec<HealthCheckResult>>>,
}

#[derive(Clone)]
pub struct AppRuntimePluginState {
    pub plugin_operation_guards: PluginOperationGuardTable,
    pub plugin_install_orchestrator: PluginInstallOrchestrator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePerformanceClass {
    Slow,
    Fast,
}

impl std::fmt::Display for RuntimePerformanceClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Slow => f.write_str("slow"),
            Self::Fast => f.write_str("fast"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePerformanceSnapshot {
    pub cpu_class: RuntimePerformanceClass,
    pub config_io_class: RuntimePerformanceClass,
    pub cpu_probe_elapsed_ms: Option<u64>,
    pub config_io_probe_elapsed_ms: Option<u64>,
}

impl RuntimePerformanceSnapshot {
    pub fn slow() -> Self {
        Self {
            cpu_class: RuntimePerformanceClass::Slow,
            config_io_class: RuntimePerformanceClass::Slow,
            cpu_probe_elapsed_ms: None,
            config_io_probe_elapsed_ms: None,
        }
    }
}

#[derive(Clone)]
pub struct AppRuntimeEnvironmentState {
    pub build_lane: BinaryLane,
    pub build_class: BinaryClass,
    pub(crate) supported_plugin_required_features: Arc<HashSet<String>>,
    pub(crate) config_dir: Arc<PathBuf>,
    pub(crate) performance_snapshot: Arc<OnceCell<RuntimePerformanceSnapshot>>,
    fixed_now: Arc<std::sync::RwLock<Option<DateTime<Utc>>>>,
}

impl AppRuntimeEnvironmentState {
    pub fn new<I, S>(
        build_lane: BinaryLane,
        config_dir: impl Into<PathBuf>,
        supported_plugin_required_features: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            build_lane,
            build_class: build_lane.binary_class(),
            supported_plugin_required_features: normalize_supported_plugin_required_features(
                supported_plugin_required_features,
            ),
            config_dir: Arc::new(config_dir.into()),
            performance_snapshot: Arc::new(OnceCell::new()),
            fixed_now: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.fixed_now
            .read()
            .ok()
            .and_then(|guard| *guard)
            .unwrap_or_else(Utc::now)
    }

    pub fn set_fixed_now_for_tests(&self, now: Option<DateTime<Utc>>) {
        if let Ok(mut guard) = self.fixed_now.write() {
            *guard = now;
        }
    }
}

fn normalize_supported_plugin_required_features<I, S>(features: I) -> Arc<HashSet<String>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    Arc::new(
        features
            .into_iter()
            .map(Into::into)
            .map(|feature| feature.trim().to_ascii_lowercase())
            .filter(|feature| !feature.is_empty())
            .collect::<HashSet<_>>(),
    )
}

#[derive(Clone)]
pub struct AppRuntimeIntegrationState {
    pub managed_indexer_sync_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone)]
pub struct AppRuntimeSecurityState {
    recovery_admin_login_enabled: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone)]
pub struct AppRuntimeState {
    pub environment: AppRuntimeEnvironmentState,
    pub security: AppRuntimeSecurityState,
    pub events: AppRuntimeEventState,
    pub catalog: AppRuntimeCatalogState,
    pub acquisition: AppRuntimeAcquisitionState,
    pub imports: AppRuntimeImportState,
    pub library: AppRuntimeLibraryState,
    pub jobs: AppRuntimeJobState,
    pub health: AppRuntimeHealthState,
    pub plugins: AppRuntimePluginState,
    pub integrations: AppRuntimeIntegrationState,
}

impl AppRuntimeState {
    pub fn new<I, S>(
        build_lane: BinaryLane,
        config_dir: impl Into<PathBuf>,
        supported_plugin_required_features: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let (domain_event_tx, _domain_event_rx) = broadcast::channel(256);
        // Match the main domain-event buffer so short notification bursts can queue wake hints
        // while the dispatcher catches up from persisted offsets.
        let (notification_event_tx, _notification_event_rx) = broadcast::channel(256);
        let (import_history_tx, _) = broadcast::channel::<()>(16);
        let (indexers_changed_tx, _) = broadcast::channel::<()>(16);
        let (provider_catalog_changed_tx, _) = broadcast::channel::<Vec<ProviderCatalogFamily>>(16);
        let (settings_changed_tx, _) = broadcast::channel::<Vec<String>>(16);

        Self {
            environment: AppRuntimeEnvironmentState::new(
                build_lane,
                config_dir,
                supported_plugin_required_features,
            ),
            security: AppRuntimeSecurityState {
                recovery_admin_login_enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
            events: AppRuntimeEventState {
                domain_event_broadcast: domain_event_tx,
                notification_event_broadcast: notification_event_tx,
                import_history_broadcast: import_history_tx,
                indexers_changed_broadcast: indexers_changed_tx,
                provider_catalog_changed_broadcast: provider_catalog_changed_tx,
                settings_changed_broadcast: settings_changed_tx,
            },
            catalog: AppRuntimeCatalogState {
                monitored_title_matcher: Arc::new(RwLock::new(
                    crate::import_title_resolution::MonitoredTitleMatcherCache::default(),
                )),
                poster_wake: Arc::new(tokio::sync::Notify::new()),
                fanart_wake: Arc::new(tokio::sync::Notify::new()),
                title_hydration_wake: Arc::new(tokio::sync::Notify::new()),
                title_recommendation_refresh_queue: Arc::new(tokio::sync::Mutex::new(
                    crate::catalog_workflow::TitleRecommendationRefreshQueue::default(),
                )),
                title_recommendation_refresh_wake: Arc::new(tokio::sync::Notify::new()),
                image_processing_limit: Arc::new(Semaphore::new(4)),
                title_image_maintenance_lock: Arc::new(tokio::sync::RwLock::new(())),
                title_image_cache_clear_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(
                    false,
                )),
            },
            acquisition: AppRuntimeAcquisitionState {
                acquisition_wake: Arc::new(tokio::sync::Notify::new()),
                download_submission_guards: DownloadSubmissionGuardTable::default(),
                download_failure_guards: DownloadFailureGuardTable::default(),
                release_candidate_passwords: Arc::new(std::sync::Mutex::new(HashMap::new())),
                rss_seen_guids: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
                tracked_download_handle: None,
                tracked_download_snapshot: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
                download_client_category_ownership: Arc::new(tokio::sync::RwLock::new(
                    DownloadClientCategoryOwnershipCache::default(),
                )),
                acquisition_search_cancellation_tokens: Arc::new(Mutex::new(HashMap::new())),
                interactive_release_searches: Arc::new(Mutex::new(HashMap::new())),
            },
            imports: AppRuntimeImportState {
                external_import_warmup_orchestrator:
                    ExternalImportMonitorWarmupOrchestrator::default(),
                external_import_apply_lock: Arc::new(tokio::sync::Mutex::new(())),
                external_import_source_chunk_cleanup_done: Arc::new(tokio::sync::Mutex::new(false)),
                same_path_upgrade_guard_lock: Arc::new(tokio::sync::Mutex::new(())),
            },
            library: AppRuntimeLibraryState {
                library_scan_tracker: LibraryScanTracker::new(),
                library_scan_cancellation_tokens: Arc::new(Mutex::new(HashMap::new())),
                library_scan_title_walk_limit: Arc::new(Semaphore::new(
                    LIBRARY_SCAN_GLOBAL_TITLE_WALK_CONCURRENCY,
                )),
                library_scan_analysis_limit: Arc::new(Semaphore::new(
                    GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY,
                )),
            },
            jobs: AppRuntimeJobState {
                job_run_tracker: JobRunTracker::new(),
                discovery_sync_wake: Arc::new(tokio::sync::Notify::new()),
                backup_execution_guards: BackupExecutionGuardTable::default(),
                interactive_operation_guards: InteractiveOperationGuardTable::default(),
                title_deletion_lock: Arc::new(tokio::sync::Mutex::new(())),
                acquisition_search_lock: Arc::new(tokio::sync::Mutex::new(())),
            },
            health: AppRuntimeHealthState {
                results: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            },
            plugins: AppRuntimePluginState {
                plugin_operation_guards: PluginOperationGuardTable::default(),
                plugin_install_orchestrator: PluginInstallOrchestrator::default(),
            },
            integrations: AppRuntimeIntegrationState {
                managed_indexer_sync_lock: Arc::new(tokio::sync::Mutex::new(())),
            },
        }
    }
}

impl Default for AppRuntimeState {
    fn default() -> Self {
        Self::new(
            BinaryLane::Portable,
            PathBuf::from("."),
            Vec::<String>::new(),
        )
    }
}

#[derive(Clone)]
pub struct AppAssembly {
    pub services: AppServices,
    pub runtime: AppRuntimeState,
}

#[derive(Clone)]
pub struct AppCatalogServices {
    pub(crate) titles: Arc<dyn TitleRepository>,
    pub(crate) shows: Arc<dyn ShowRepository>,
    pub(crate) libraries: Arc<dyn LibraryRepository>,
    pub(crate) media_requests: Arc<dyn MediaRequestRepository>,
}

#[derive(Clone)]
pub struct AppIdentityServices {
    pub(crate) users: Arc<dyn UserRepository>,
    pub(crate) ui_settings: Arc<dyn UserUiSettingsRepository>,
    pub(crate) external_accounts: Arc<dyn UserExternalAccountRepository>,
    pub(crate) webauthn: Arc<dyn WebauthnRepository>,
    pub(crate) totp: Arc<dyn TotpRepository>,
    pub(crate) oauth: Arc<dyn OAuthRepository>,
}

#[derive(Clone)]
pub struct AppEventServices {
    pub(crate) domain_events: Arc<dyn DomainEventRepository>,
    pub(crate) job_runs: Arc<dyn JobRunRepository>,
}

#[derive(Clone, Default)]
pub enum RuntimeFeature<T> {
    #[default]
    Disabled,
    Enabled(T),
}

impl<T> RuntimeFeature<T> {
    pub fn enabled(value: T) -> Self {
        Self::Enabled(value)
    }

    pub fn available(&self) -> Option<&T> {
        match self {
            Self::Disabled => None,
            Self::Enabled(value) => Some(value),
        }
    }
}

#[derive(Clone)]
pub struct AppLibraryServices {
    pub(crate) metadata_gateway: Arc<dyn MetadataGateway>,
    pub(crate) discovery: Arc<dyn DiscoveryRepository>,
    pub(crate) library_scanner: Arc<dyn LibraryScanner>,
    pub(crate) library_renamer: Arc<dyn LibraryRenamer>,
    pub(crate) media_files: Arc<dyn MediaFileRepository>,
    pub(crate) media_analyzer: Arc<dyn MediaAnalyzer>,
    pub(crate) title_images: Arc<dyn TitleImageRepository>,
    pub(crate) image_proxy: Arc<dyn ImageProxyRepository>,
    pub(crate) image_proxy_cache_control: Arc<dyn ImageProxyCacheControl>,
    pub(crate) title_image_processor: Arc<dyn TitleImageProcessor>,
    pub(crate) library_probe_signatures: Arc<dyn LibraryProbeRepository>,
    pub(crate) library_scan_unmatched_items: Arc<dyn LibraryScanUnmatchedItemRepository>,
}

#[derive(Clone)]
pub struct AppIntegrationServices {
    pub(crate) indexer_configs: Arc<dyn IndexerConfigRepository>,
    pub(crate) indexer_proxy_configs: Arc<dyn IndexerProxyConfigRepository>,
    pub(crate) scope_indexer_coverage: Arc<dyn ScopeIndexerCoverageRepository>,
    pub(crate) indexer_caps_refresher: RuntimeFeature<Arc<dyn IndexerCapsSnapshotRefresher>>,
    pub(crate) indexer_client: Arc<dyn IndexerClient>,
    pub(crate) download_client: Arc<dyn DownloadClient>,
    pub(crate) builtin_download_client_connection_tester:
        Arc<dyn BuiltinDownloadClientConnectionTester>,
    pub(crate) download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    pub(crate) subtitle_provider_configs: RuntimeFeature<Arc<dyn SubtitleProviderConfigRepository>>,
    pub(crate) external_identity_verifier: Arc<dyn ExternalIdentityVerifier>,
    pub(crate) media_server_connections: Arc<dyn MediaServerConnectionRepository>,
    pub(crate) indexer_stats: Arc<dyn IndexerStatsTracker>,
    pub(crate) upstream_scheduler: Arc<dyn UpstreamScheduler>,
    pub(crate) plugin_provider: RuntimeFeature<Arc<dyn IndexerPluginProvider>>,
    pub(crate) download_client_plugin_provider:
        RuntimeFeature<Arc<dyn DownloadClientPluginProvider>>,
    pub(crate) subtitle_plugin_provider: RuntimeFeature<Arc<dyn SubtitlePluginProvider>>,
    pub(crate) archive_extractor_plugin_provider:
        RuntimeFeature<Arc<dyn ArchiveExtractorPluginProvider>>,
}

#[derive(Clone)]
pub struct AppWorkflowServices {
    pub(crate) imports: Arc<dyn ImportRepository>,
    pub(crate) external_import_monitor_snapshots: Arc<dyn ExternalImportMonitorSnapshotRepository>,
    pub(crate) external_import_setup_secret_drafts:
        Arc<dyn ExternalImportSetupSecretDraftRepository>,
    pub(crate) download_queue_commands: Arc<dyn DownloadQueueCommandRepository>,
    pub(crate) workflow_operations: Arc<dyn WorkflowOperationRepository>,
    pub(crate) file_importer: Arc<dyn FileImporter>,
    pub(crate) import_artifacts: Arc<dyn ImportArtifactRepository>,
    pub(crate) release_attempts: Arc<dyn ReleaseAttemptRepository>,
    pub(crate) acquisition_state: Arc<dyn AcquisitionStateRepository>,
    pub(crate) download_submissions: Arc<dyn DownloadSubmissionRepository>,
    pub(crate) acquisition_scope_states: Arc<dyn AcquisitionScopeStateRepository>,
    pub(crate) housekeeping: Arc<dyn HousekeepingRepository>,
    pub(crate) pending_releases: Arc<dyn PendingReleaseRepository>,
    pub(crate) blocklist_repo: Arc<dyn BlocklistRepository>,
    pub(crate) subtitle_downloads: Arc<dyn SubtitleDownloadRepository>,
    pub(crate) staged_nzb_store: Arc<dyn StagedNzbStore>,
    pub(crate) staged_nzb_pipeline_limit: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct AppConfigServices {
    pub(crate) settings: Arc<dyn SettingsRepository>,
    pub(crate) quality_profiles: Arc<dyn QualityProfileRepository>,
    pub(crate) system_info: Arc<dyn SystemInfoProvider>,
    pub(crate) plugin_http_trust_runtime: RuntimeFeature<Arc<dyn PluginHttpTrustConfigRuntime>>,
    pub(crate) logical_backup_exporter: Arc<dyn LogicalBackupExporter>,
    pub(crate) backup_dir: PathBuf,
    pub(crate) smg_registration_secret: Option<String>,
    pub(crate) smg_gateway_url: Option<String>,
}

#[derive(Clone)]
pub struct AppCustomizationServices {
    pub(crate) rule_sets: Arc<dyn RuleSetRepository>,
    pub(crate) pp_scripts: Arc<dyn PostProcessingScriptRepository>,
    pub(crate) plugin_installations: Arc<dyn PluginInstallationRepository>,
    pub(crate) plugin_descriptor_loader: Arc<dyn PluginDescriptorLoader>,
    pub(crate) user_rules: Arc<std::sync::RwLock<scryer_rules::UserRulesEngine>>,
}

#[derive(Clone)]
pub enum AppNotificationServices {
    Disabled,
    Store {
        notification_channels: Arc<dyn NotificationChannelRepository>,
        notification_subscriptions: Arc<dyn NotificationSubscriptionRepository>,
    },
    Provider {
        notification_provider: Arc<dyn NotificationPluginProvider>,
    },
    Runtime {
        notification_channels: Arc<dyn NotificationChannelRepository>,
        notification_subscriptions: Arc<dyn NotificationSubscriptionRepository>,
        notification_provider: Arc<dyn NotificationPluginProvider>,
    },
}

impl AppNotificationServices {
    pub fn notification_channels(&self) -> Option<&Arc<dyn NotificationChannelRepository>> {
        match self {
            Self::Store {
                notification_channels,
                ..
            }
            | Self::Runtime {
                notification_channels,
                ..
            } => Some(notification_channels),
            Self::Disabled | Self::Provider { .. } => None,
        }
    }

    pub fn notification_subscriptions(
        &self,
    ) -> Option<&Arc<dyn NotificationSubscriptionRepository>> {
        match self {
            Self::Store {
                notification_subscriptions,
                ..
            }
            | Self::Runtime {
                notification_subscriptions,
                ..
            } => Some(notification_subscriptions),
            Self::Disabled | Self::Provider { .. } => None,
        }
    }

    pub fn notification_provider(&self) -> Option<&Arc<dyn NotificationPluginProvider>> {
        match self {
            Self::Provider {
                notification_provider,
            }
            | Self::Runtime {
                notification_provider,
                ..
            } => Some(notification_provider),
            Self::Disabled | Self::Store { .. } => None,
        }
    }
}

#[derive(Clone)]
pub struct AppServices {
    pub(crate) catalog: AppCatalogServices,
    pub(crate) identity: AppIdentityServices,
    pub(crate) events: AppEventServices,
    pub(crate) library: AppLibraryServices,
    pub(crate) integrations: AppIntegrationServices,
    pub(crate) workflow: AppWorkflowServices,
    pub(crate) config: AppConfigServices,
    pub(crate) customization: AppCustomizationServices,
    pub(crate) notifications: AppNotificationServices,
}

impl AppServices {
    #[expect(
        clippy::too_many_arguments,
        reason = "service assembly intentionally enumerates each root dependency explicitly"
    )]
    pub fn builder(
        titles: Arc<dyn TitleRepository>,
        shows: Arc<dyn ShowRepository>,
        users: Arc<dyn UserRepository>,
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        indexer_client: Arc<dyn IndexerClient>,
        download_client: Arc<dyn DownloadClient>,
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        release_attempts: Arc<dyn ReleaseAttemptRepository>,
        settings: Arc<dyn SettingsRepository>,
        quality_profiles: Arc<dyn QualityProfileRepository>,
        backup_dir: impl Into<PathBuf>,
    ) -> AppServicesBuilder {
        AppServicesBuilder {
            services: Self::with_placeholder_defaults(
                titles,
                shows,
                users,
                indexer_configs,
                indexer_client,
                download_client,
                download_client_configs,
                release_attempts,
                settings,
                quality_profiles,
                backup_dir.into(),
            ),
            runtime: AppRuntimeState::default(),
            configured: AppServicesBuildConfiguration::default(),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "placeholder wiring intentionally follows the full service dependency surface"
    )]
    fn with_placeholder_defaults(
        titles: Arc<dyn TitleRepository>,
        shows: Arc<dyn ShowRepository>,
        users: Arc<dyn UserRepository>,
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        indexer_client: Arc<dyn IndexerClient>,
        download_client: Arc<dyn DownloadClient>,
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        release_attempts: Arc<dyn ReleaseAttemptRepository>,
        settings: Arc<dyn SettingsRepository>,
        quality_profiles: Arc<dyn QualityProfileRepository>,
        backup_dir: PathBuf,
    ) -> Self {
        Self {
            catalog: AppCatalogServices {
                titles,
                shows,
                libraries: Arc::new(NullLibraryRepository),
                media_requests: Arc::new(NullMediaRequestRepository),
            },
            identity: AppIdentityServices {
                users,
                ui_settings: Arc::new(null_repositories::NullUserUiSettingsRepository),
                external_accounts: Arc::new(null_repositories::NullUserExternalAccountRepository),
                webauthn: Arc::new(null_repositories::NullWebauthnRepository),
                totp: Arc::new(null_repositories::NullTotpRepository),
                oauth: Arc::new(null_repositories::NullOAuthRepository),
            },
            events: AppEventServices {
                domain_events: Arc::new(NullDomainEventRepository),
                job_runs: Arc::new(null_repositories::NullJobRunRepository),
            },
            library: AppLibraryServices {
                metadata_gateway: Arc::new(crate::library_scan::NullMetadataGateway),
                discovery: Arc::new(null_repositories::NullDiscoveryRepository),
                library_scanner: Arc::new(crate::library_scan::NullLibraryScanner),
                library_renamer: Arc::new(crate::library_rename::NullLibraryRenamer),
                media_files: Arc::new(NullMediaFileRepository),
                media_analyzer: Arc::new(NativeMediaAnalyzer),
                title_images: Arc::new(NullTitleImageRepository),
                image_proxy: Arc::new(null_repositories::NullImageProxyRepository),
                image_proxy_cache_control: Arc::new(null_repositories::NullImageProxyCacheControl),
                title_image_processor: Arc::new(NullTitleImageProcessor),
                library_probe_signatures: Arc::new(null_repositories::NullLibraryProbeRepository),
                library_scan_unmatched_items: Arc::new(
                    null_repositories::NullLibraryScanUnmatchedItemRepository,
                ),
            },
            integrations: AppIntegrationServices {
                indexer_configs,
                indexer_proxy_configs: Arc::new(
                    null_repositories::NullIndexerProxyConfigRepository,
                ),
                scope_indexer_coverage: Arc::new(
                    null_repositories::NullScopeIndexerCoverageRepository,
                ),
                indexer_caps_refresher: RuntimeFeature::Disabled,
                indexer_client,
                download_client,
                builtin_download_client_connection_tester: Arc::new(
                    null_repositories::NullBuiltinDownloadClientConnectionTester,
                ),
                download_client_configs,
                subtitle_provider_configs: RuntimeFeature::Disabled,
                external_identity_verifier: Arc::new(
                    null_repositories::NullExternalIdentityVerifier,
                ),
                media_server_connections: Arc::new(
                    null_repositories::NullMediaServerConnectionRepository,
                ),
                indexer_stats: Arc::new(NullIndexerStatsTracker),
                upstream_scheduler: Arc::new(NullUpstreamScheduler),
                plugin_provider: RuntimeFeature::Disabled,
                download_client_plugin_provider: RuntimeFeature::Disabled,
                subtitle_plugin_provider: RuntimeFeature::Disabled,
                archive_extractor_plugin_provider: RuntimeFeature::Disabled,
            },
            workflow: AppWorkflowServices {
                imports: Arc::new(NullImportRepository),
                external_import_monitor_snapshots: Arc::new(
                    null_repositories::NullExternalImportMonitorSnapshotRepository,
                ),
                external_import_setup_secret_drafts: Arc::new(
                    null_repositories::NullExternalImportSetupSecretDraftRepository,
                ),
                download_queue_commands: Arc::new(
                    null_repositories::NullDownloadQueueCommandRepository,
                ),
                workflow_operations: Arc::new(NullWorkflowOperationRepository),
                file_importer: Arc::new(NullFileImporter),
                import_artifacts: Arc::new(null_repositories::NullImportArtifactRepository),
                release_attempts,
                acquisition_state: Arc::new(NullAcquisitionStateRepository),
                download_submissions: Arc::new(NullDownloadSubmissionRepository),
                acquisition_scope_states: Arc::new(NullAcquisitionScopeStateRepository),
                housekeeping: Arc::new(NullHousekeepingRepository),
                pending_releases: Arc::new(NullPendingReleaseRepository),
                blocklist_repo: Arc::new(NullBlocklistRepository),
                subtitle_downloads: Arc::new(null_repositories::NullSubtitleDownloadRepository),
                staged_nzb_store: Arc::new(null_repositories::NullStagedNzbStore),
                staged_nzb_pipeline_limit: Arc::new(Semaphore::new(4)),
            },
            config: AppConfigServices {
                settings,
                quality_profiles,
                system_info: Arc::new(NullSystemInfoProvider),
                plugin_http_trust_runtime: RuntimeFeature::Disabled,
                logical_backup_exporter: Arc::new(NullLogicalBackupExporter),
                backup_dir,
                smg_registration_secret: None,
                smg_gateway_url: None,
            },
            customization: AppCustomizationServices {
                rule_sets: Arc::new(NullRuleSetRepository),
                pp_scripts: Arc::new(NullPostProcessingScriptRepository),
                plugin_installations: Arc::new(NullPluginInstallationRepository),
                plugin_descriptor_loader: Arc::new(NullPluginDescriptorLoader),
                user_rules: Arc::new(std::sync::RwLock::new(
                    scryer_rules::UserRulesEngine::empty(),
                )),
            },
            notifications: AppNotificationServices::Disabled,
        }
    }
}

macro_rules! app_services_builder_setter {
    ($name:ident, $($field:ident).+, $ty:ty) => {
        pub fn $name(mut self, value: $ty) -> Self {
            self.services.$($field).+ = value;
            self
        }
    };
}

macro_rules! app_services_builder_required_setter {
    ($name:ident, $($field:ident).+, $config_field:ident, $ty:ty) => {
        pub fn $name(mut self, value: $ty) -> Self {
            self.services.$($field).+ = value;
            self.configured.$config_field = true;
            self
        }
    };
}

macro_rules! app_services_builder_runtime_feature_setter {
    ($name:ident, $($field:ident).+, $ty:ty) => {
        pub fn $name(mut self, value: $ty) -> Self {
            self.services.$($field).+ = RuntimeFeature::enabled(value);
            self
        }
    };
}

pub struct AppServicesBuilder {
    services: AppServices,
    runtime: AppRuntimeState,
    configured: AppServicesBuildConfiguration,
}

#[derive(Default)]
struct AppServicesBuildConfiguration {
    domain_events: bool,
    metadata_gateway: bool,
    library_scanner: bool,
    imports: bool,
    workflow_operations: bool,
    import_artifacts: bool,
    media_files: bool,
    acquisition_state: bool,
    download_submissions: bool,
    acquisition_scope_states: bool,
    rule_sets: bool,
    pp_scripts: bool,
    plugin_installations: bool,
    system_info: bool,
    title_images: bool,
    housekeeping: bool,
    pending_releases: bool,
    blocklist_repo: bool,
    subtitle_downloads: bool,
    job_runs: bool,
    library_probe_signatures: bool,
    library_scan_unmatched_items: bool,
}

impl AppServicesBuildConfiguration {
    fn missing_runtime_services(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();

        if !self.domain_events {
            missing.push("domain_events");
        }
        if !self.metadata_gateway {
            missing.push("metadata_gateway");
        }
        if !self.library_scanner {
            missing.push("library_scanner");
        }
        if !self.imports {
            missing.push("imports");
        }
        if !self.workflow_operations {
            missing.push("workflow_operations");
        }
        if !self.import_artifacts {
            missing.push("import_artifacts");
        }
        if !self.media_files {
            missing.push("media_files");
        }
        if !self.acquisition_state {
            missing.push("acquisition_state");
        }
        if !self.download_submissions {
            missing.push("download_submissions");
        }
        if !self.acquisition_scope_states {
            missing.push("acquisition_scope_states");
        }
        if !self.rule_sets {
            missing.push("rule_sets");
        }
        if !self.pp_scripts {
            missing.push("pp_scripts");
        }
        if !self.plugin_installations {
            missing.push("plugin_installations");
        }
        if !self.system_info {
            missing.push("system_info");
        }
        if !self.title_images {
            missing.push("title_images");
        }
        if !self.housekeeping {
            missing.push("housekeeping");
        }
        if !self.pending_releases {
            missing.push("pending_releases");
        }
        if !self.blocklist_repo {
            missing.push("blocklist_repo");
        }
        if !self.subtitle_downloads {
            missing.push("subtitle_downloads");
        }
        if !self.job_runs {
            missing.push("job_runs");
        }
        if !self.library_probe_signatures {
            missing.push("library_probe_signatures");
        }
        if !self.library_scan_unmatched_items {
            missing.push("library_scan_unmatched_items");
        }

        missing
    }
}

impl AppServicesBuilder {
    app_services_builder_runtime_feature_setter!(
        with_plugin_http_trust_runtime,
        config.plugin_http_trust_runtime,
        Arc<dyn PluginHttpTrustConfigRuntime>
    );

    pub fn with_runtime_environment<I, S>(
        mut self,
        build_lane: BinaryLane,
        config_dir: impl Into<PathBuf>,
        supported_plugin_required_features: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.runtime =
            AppRuntimeState::new(build_lane, config_dir, supported_plugin_required_features);
        self
    }

    pub fn with_supported_plugin_required_features<I, S>(mut self, features: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.runtime.environment.supported_plugin_required_features =
            normalize_supported_plugin_required_features(features);
        self
    }
}

impl AppServicesBuilder {
    app_services_builder_setter!(with_shows, catalog.shows, Arc<dyn ShowRepository>);
    app_services_builder_setter!(
        with_libraries,
        catalog.libraries,
        Arc<dyn LibraryRepository>
    );
    app_services_builder_setter!(
        with_media_requests,
        catalog.media_requests,
        Arc<dyn MediaRequestRepository>
    );
    app_services_builder_setter!(
        with_webauthn_store,
        identity.webauthn,
        Arc<dyn WebauthnRepository>
    );
    app_services_builder_setter!(with_totp_store, identity.totp, Arc<dyn TotpRepository>);
    app_services_builder_setter!(
        with_user_ui_settings_store,
        identity.ui_settings,
        Arc<dyn UserUiSettingsRepository>
    );
    app_services_builder_setter!(
        with_external_account_store,
        identity.external_accounts,
        Arc<dyn UserExternalAccountRepository>
    );
    app_services_builder_setter!(with_oauth_store, identity.oauth, Arc<dyn OAuthRepository>);
    pub fn with_customization_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: PluginInstallationRepository
            + PostProcessingScriptRepository
            + RuleSetRepository
            + Send
            + Sync
            + 'static,
    {
        self.services.customization.rule_sets = store.clone();
        self.services.customization.pp_scripts = store.clone();
        self.services.customization.plugin_installations = store;
        self.configured.rule_sets = true;
        self.configured.pp_scripts = true;
        self.configured.plugin_installations = true;
        self
    }

    pub fn with_rule_set_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: RuleSetRepository + Send + Sync + 'static,
    {
        self.services.customization.rule_sets = store;
        self.configured.rule_sets = true;
        self
    }

    pub fn with_post_processing_script_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: PostProcessingScriptRepository + Send + Sync + 'static,
    {
        self.services.customization.pp_scripts = store;
        self.configured.pp_scripts = true;
        self
    }

    pub fn with_plugin_installation_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: PluginInstallationRepository + Send + Sync + 'static,
    {
        self.services.customization.plugin_installations = store;
        self.configured.plugin_installations = true;
        self
    }

    pub fn with_plugin_descriptor_loader<T>(mut self, loader: Arc<T>) -> Self
    where
        T: PluginDescriptorLoader + Send + Sync + 'static,
    {
        self.services.customization.plugin_descriptor_loader = loader;
        self
    }

    pub fn with_notification_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: NotificationChannelRepository
            + NotificationSubscriptionRepository
            + Send
            + Sync
            + 'static,
    {
        let notification_channels: Arc<dyn NotificationChannelRepository> = store.clone();
        let notification_subscriptions: Arc<dyn NotificationSubscriptionRepository> = store;
        self.services.notifications = match self.services.notifications {
            AppNotificationServices::Disabled | AppNotificationServices::Store { .. } => {
                AppNotificationServices::Store {
                    notification_channels,
                    notification_subscriptions,
                }
            }
            AppNotificationServices::Provider {
                notification_provider,
            }
            | AppNotificationServices::Runtime {
                notification_provider,
                ..
            } => AppNotificationServices::Runtime {
                notification_channels,
                notification_subscriptions,
                notification_provider,
            },
        };
        self
    }

    app_services_builder_setter!(
        with_builtin_download_client_connection_tester,
        integrations.builtin_download_client_connection_tester,
        Arc<dyn BuiltinDownloadClientConnectionTester>
    );
    app_services_builder_setter!(
        with_indexer_proxy_config_store,
        integrations.indexer_proxy_configs,
        Arc<dyn IndexerProxyConfigRepository>
    );
    app_services_builder_setter!(
        with_scope_indexer_coverage_store,
        integrations.scope_indexer_coverage,
        Arc<dyn ScopeIndexerCoverageRepository>
    );
    app_services_builder_setter!(
        with_external_identity_verifier,
        integrations.external_identity_verifier,
        Arc<dyn ExternalIdentityVerifier>
    );
    app_services_builder_setter!(
        with_media_server_connection_store,
        integrations.media_server_connections,
        Arc<dyn MediaServerConnectionRepository>
    );
    app_services_builder_required_setter!(
        with_metadata_gateway,
        library.metadata_gateway,
        metadata_gateway,
        Arc<dyn MetadataGateway>
    );
    app_services_builder_setter!(
        with_discovery_store,
        library.discovery,
        Arc<dyn DiscoveryRepository>
    );
    app_services_builder_required_setter!(
        with_library_scanner,
        library.library_scanner,
        library_scanner,
        Arc<dyn LibraryScanner>
    );
    app_services_builder_setter!(
        with_library_renamer,
        library.library_renamer,
        Arc<dyn LibraryRenamer>
    );
    app_services_builder_setter!(
        with_media_analyzer,
        library.media_analyzer,
        Arc<dyn MediaAnalyzer>
    );
    app_services_builder_required_setter!(
        with_domain_events,
        events.domain_events,
        domain_events,
        Arc<dyn DomainEventRepository>
    );
    app_services_builder_required_setter!(
        with_imports,
        workflow.imports,
        imports,
        Arc<dyn ImportRepository>
    );
    app_services_builder_setter!(
        with_external_import_monitor_snapshots,
        workflow.external_import_monitor_snapshots,
        Arc<dyn ExternalImportMonitorSnapshotRepository>
    );
    app_services_builder_setter!(
        with_external_import_setup_secret_drafts,
        workflow.external_import_setup_secret_drafts,
        Arc<dyn ExternalImportSetupSecretDraftRepository>
    );
    app_services_builder_setter!(
        with_download_queue_commands,
        workflow.download_queue_commands,
        Arc<dyn DownloadQueueCommandRepository>
    );
    app_services_builder_required_setter!(
        with_workflow_operations,
        workflow.workflow_operations,
        workflow_operations,
        Arc<dyn WorkflowOperationRepository>
    );
    app_services_builder_required_setter!(
        with_import_artifacts,
        workflow.import_artifacts,
        import_artifacts,
        Arc<dyn ImportArtifactRepository>
    );
    app_services_builder_setter!(
        with_file_importer,
        workflow.file_importer,
        Arc<dyn FileImporter>
    );
    app_services_builder_required_setter!(
        with_media_files,
        library.media_files,
        media_files,
        Arc<dyn MediaFileRepository>
    );
    app_services_builder_required_setter!(
        with_download_submissions,
        workflow.download_submissions,
        download_submissions,
        Arc<dyn DownloadSubmissionRepository>
    );
    app_services_builder_required_setter!(
        with_acquisition_state,
        workflow.acquisition_state,
        acquisition_state,
        Arc<dyn AcquisitionStateRepository>
    );
    app_services_builder_required_setter!(
        with_acquisition_scope_states,
        workflow.acquisition_scope_states,
        acquisition_scope_states,
        Arc<dyn AcquisitionScopeStateRepository>
    );
    app_services_builder_required_setter!(
        with_pending_releases,
        workflow.pending_releases,
        pending_releases,
        Arc<dyn PendingReleaseRepository>
    );
    app_services_builder_required_setter!(
        with_blocklist_repo,
        workflow.blocklist_repo,
        blocklist_repo,
        Arc<dyn BlocklistRepository>
    );
    app_services_builder_required_setter!(
        with_rule_sets,
        customization.rule_sets,
        rule_sets,
        Arc<dyn RuleSetRepository>
    );
    app_services_builder_required_setter!(
        with_pp_scripts,
        customization.pp_scripts,
        pp_scripts,
        Arc<dyn PostProcessingScriptRepository>
    );
    app_services_builder_required_setter!(
        with_plugin_installations,
        customization.plugin_installations,
        plugin_installations,
        Arc<dyn PluginInstallationRepository>
    );
    app_services_builder_required_setter!(
        with_system_info,
        config.system_info,
        system_info,
        Arc<dyn SystemInfoProvider>
    );
    app_services_builder_setter!(
        with_logical_backup_exporter,
        config.logical_backup_exporter,
        Arc<dyn LogicalBackupExporter>
    );
    app_services_builder_setter!(with_backup_dir, config.backup_dir, PathBuf);
    app_services_builder_setter!(
        with_smg_registration_secret,
        config.smg_registration_secret,
        Option<String>
    );
    app_services_builder_setter!(with_smg_gateway_url, config.smg_gateway_url, Option<String>);
    app_services_builder_required_setter!(
        with_job_runs,
        events.job_runs,
        job_runs,
        Arc<dyn JobRunRepository>
    );
    app_services_builder_required_setter!(
        with_library_probe_signatures,
        library.library_probe_signatures,
        library_probe_signatures,
        Arc<dyn LibraryProbeRepository>
    );
    app_services_builder_required_setter!(
        with_library_scan_unmatched_items,
        library.library_scan_unmatched_items,
        library_scan_unmatched_items,
        Arc<dyn LibraryScanUnmatchedItemRepository>
    );
    app_services_builder_required_setter!(
        with_title_images,
        library.title_images,
        title_images,
        Arc<dyn TitleImageRepository>
    );
    app_services_builder_setter!(
        with_image_proxy,
        library.image_proxy,
        Arc<dyn ImageProxyRepository>
    );
    app_services_builder_setter!(
        with_image_proxy_cache_control,
        library.image_proxy_cache_control,
        Arc<dyn ImageProxyCacheControl>
    );
    app_services_builder_setter!(
        with_title_image_processor,
        library.title_image_processor,
        Arc<dyn TitleImageProcessor>
    );
    app_services_builder_required_setter!(
        with_housekeeping,
        workflow.housekeeping,
        housekeeping,
        Arc<dyn HousekeepingRepository>
    );
    app_services_builder_required_setter!(
        with_subtitle_downloads,
        workflow.subtitle_downloads,
        subtitle_downloads,
        Arc<dyn SubtitleDownloadRepository>
    );
    app_services_builder_setter!(
        with_staged_nzb_store,
        workflow.staged_nzb_store,
        Arc<dyn StagedNzbStore>
    );
    app_services_builder_setter!(
        with_staged_nzb_pipeline_limit,
        workflow.staged_nzb_pipeline_limit,
        Arc<Semaphore>
    );
    app_services_builder_setter!(
        with_indexer_stats,
        integrations.indexer_stats,
        Arc<dyn IndexerStatsTracker>
    );
    app_services_builder_setter!(
        with_upstream_scheduler,
        integrations.upstream_scheduler,
        Arc<dyn UpstreamScheduler>
    );
    app_services_builder_runtime_feature_setter!(
        with_indexer_caps_refresher,
        integrations.indexer_caps_refresher,
        Arc<dyn IndexerCapsSnapshotRefresher>
    );
    app_services_builder_runtime_feature_setter!(
        with_plugin_provider,
        integrations.plugin_provider,
        Arc<dyn IndexerPluginProvider>
    );
    app_services_builder_runtime_feature_setter!(
        with_download_client_plugin_provider,
        integrations.download_client_plugin_provider,
        Arc<dyn DownloadClientPluginProvider>
    );
    app_services_builder_runtime_feature_setter!(
        with_subtitle_provider_configs,
        integrations.subtitle_provider_configs,
        Arc<dyn SubtitleProviderConfigRepository>
    );
    app_services_builder_runtime_feature_setter!(
        with_subtitle_plugin_provider,
        integrations.subtitle_plugin_provider,
        Arc<dyn SubtitlePluginProvider>
    );
    app_services_builder_runtime_feature_setter!(
        with_archive_extractor_plugin_provider,
        integrations.archive_extractor_plugin_provider,
        Arc<dyn ArchiveExtractorPluginProvider>
    );
    pub fn with_notification_provider(
        mut self,
        value: Arc<dyn NotificationPluginProvider>,
    ) -> Self {
        self.services.notifications = match self.services.notifications {
            AppNotificationServices::Disabled | AppNotificationServices::Provider { .. } => {
                AppNotificationServices::Provider {
                    notification_provider: value,
                }
            }
            AppNotificationServices::Store {
                notification_channels,
                notification_subscriptions,
            }
            | AppNotificationServices::Runtime {
                notification_channels,
                notification_subscriptions,
                ..
            } => AppNotificationServices::Runtime {
                notification_channels,
                notification_subscriptions,
                notification_provider: value,
            },
        };
        self
    }
    pub fn with_tracked_download_handle(
        mut self,
        value: tracked_downloads::TrackedDownloadHandle,
    ) -> Self {
        self.runtime.acquisition.tracked_download_handle = Some(value);
        self
    }

    pub fn build(self) -> AppAssembly {
        let missing = self.configured.missing_runtime_services();
        assert!(
            missing.is_empty(),
            "AppServicesBuilder missing required runtime services: {}. Use build_partial_for_tests() only for intentionally partial test assemblies.",
            missing.join(", ")
        );
        self.finish()
    }

    fn finish(self) -> AppAssembly {
        AppAssembly {
            services: self.services,
            runtime: self.runtime,
        }
    }

    pub(crate) fn build_partial_for_tests(self) -> AppAssembly {
        self.finish()
    }
}

#[derive(Clone)]
pub struct AppUseCase {
    pub(crate) services: AppServices,
    pub(crate) runtime: AppRuntimeState,
    pub auth: JwtAuthConfig,
    pub facet_registry: Arc<FacetRegistry>,
    pub(crate) pending_import_resolution_locks: Arc<std::sync::Mutex<HashSet<String>>>,
    pub(crate) jwt_signing_keys: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    pub(crate) jwt_signing_keys_loaded: Arc<OnceCell<()>>,
    pub(crate) jwt_signing_keys_seed_lock: Arc<Mutex<()>>,
    pub webauthn: RuntimeFeature<Arc<webauthn_rs::Webauthn>>,
}

impl AppUseCase {
    pub async fn upstream_scheduler_snapshot(
        &self,
        filter: SchedulerSnapshotFilter,
    ) -> AppResult<SchedulerSnapshot> {
        self.services
            .integrations
            .upstream_scheduler
            .snapshot(filter)
            .await
    }

    pub async fn flush_upstream_scheduler(&self) -> AppResult<()> {
        self.services
            .integrations
            .upstream_scheduler
            .flush_pending()
            .await
    }

    pub fn set_recovery_admin_login_enabled(&self, enabled: bool) {
        self.runtime
            .security
            .recovery_admin_login_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn recovery_admin_login_enabled(&self) -> bool {
        self.runtime
            .security
            .recovery_admin_login_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn invalidate_monitored_title_matcher(&self) {
        let mut state = self.runtime.catalog.monitored_title_matcher.write().await;
        state.dirty = true;
        state.generation = state.generation.wrapping_add(1);
    }

    pub(crate) async fn monitored_title_matcher(
        &self,
    ) -> AppResult<Arc<crate::import_title_resolution::MonitoredTitleMatcher>> {
        const MATCHER_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60);

        let observed_generation = {
            let state = self.runtime.catalog.monitored_title_matcher.read().await;
            let fresh = state
                .built_at
                .is_some_and(|built_at| built_at.elapsed() <= MATCHER_MAX_AGE);
            if !state.dirty
                && fresh
                && let Some(matcher) = state.matcher.clone()
            {
                return Ok(matcher);
            }
            state.generation
        };

        let titles = self
            .services
            .catalog
            .titles
            .list_for_matching(None, None)
            .await?;
        let matcher = Arc::new(crate::import_title_resolution::MonitoredTitleMatcher::new(
            titles,
        ));

        let mut state = self.runtime.catalog.monitored_title_matcher.write().await;
        state.matcher = Some(matcher.clone());
        state.built_at = Some(std::time::Instant::now());
        // Only clear dirty when no invalidation raced the rebuild; a bumped
        // generation means this matcher may already be stale, so the next
        // caller rebuilds again rather than trusting it.
        if state.generation == observed_generation {
            state.dirty = false;
        }
        Ok(matcher)
    }

    pub fn runtime_build_lane(&self) -> BinaryLane {
        self.runtime.environment.build_lane
    }

    pub fn runtime_build_class(&self) -> BinaryClass {
        self.runtime.environment.build_class
    }

    pub(crate) fn runtime_supported_plugin_required_features(&self) -> Arc<HashSet<String>> {
        self.runtime
            .environment
            .supported_plugin_required_features
            .clone()
    }

    pub async fn runtime_performance(&self) -> RuntimePerformanceSnapshot {
        let environment = self.runtime.environment.clone();
        initialize_runtime_performance_snapshot(
            environment.performance_snapshot.as_ref(),
            environment.config_dir.clone(),
            Arc::new(probe_runtime_performance_snapshot),
        )
        .await
    }

    pub fn warm_runtime_performance(&self) {
        let app = self.clone();
        tokio::spawn(async move {
            let _ = app.runtime_performance().await;
        });
    }

    /// Test-only escape hatch for selectively overriding already-assembled services.
    ///
    /// Production assembly should go through `AppServices::builder(...).build()`.
    pub(crate) fn with_test_overrides<F>(&self, configure: F) -> Self
    where
        F: FnOnce(AppServicesBuilder) -> AppServicesBuilder,
    {
        let assembly = configure(AppServicesBuilder {
            services: self.services.clone(),
            runtime: self.runtime.clone(),
            configured: AppServicesBuildConfiguration::default(),
        })
        .build_partial_for_tests();
        Self {
            services: assembly.services,
            runtime: assembly.runtime,
            auth: self.auth.clone(),
            facet_registry: self.facet_registry.clone(),
            pending_import_resolution_locks: self.pending_import_resolution_locks.clone(),
            jwt_signing_keys: self.jwt_signing_keys.clone(),
            jwt_signing_keys_loaded: self.jwt_signing_keys_loaded.clone(),
            jwt_signing_keys_seed_lock: self.jwt_signing_keys_seed_lock.clone(),
            webauthn: self.webauthn.clone(),
        }
    }

    pub async fn append_domain_event(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        let stored = self.services.events.domain_events.append(event).await?;
        self.publish_stored_domain_event(&stored).await;
        Ok(stored)
    }

    pub async fn publish_stored_domain_event(&self, stored: &DomainEvent) {
        if should_invalidate_monitored_title_matcher(&stored.payload) {
            self.invalidate_monitored_title_matcher().await;
        }
        let _ = self
            .runtime
            .events
            .domain_event_broadcast
            .send(stored.sequence);
        if crate::notifications::dispatcher::notification_event_type(&stored.payload).is_some() {
            tracing::debug!(
                sequence = stored.sequence,
                event_type = stored.payload.event_type().as_str(),
                "queued notification dispatcher wake for notification-relevant domain event"
            );
            let _ = self
                .runtime
                .events
                .notification_event_broadcast
                .send(stored.sequence);
        }
        self.maybe_accelerate_discovery_sync_for_scan_completion(stored)
            .await;
    }

    pub async fn append_domain_events(
        &self,
        events: Vec<NewDomainEvent>,
    ) -> AppResult<Vec<DomainEvent>> {
        let stored = self
            .services
            .events
            .domain_events
            .append_many(events)
            .await?;
        if stored
            .iter()
            .any(|event| should_invalidate_monitored_title_matcher(&event.payload))
        {
            self.invalidate_monitored_title_matcher().await;
        }
        if let Some(last) = stored.last() {
            let _ = self
                .runtime
                .events
                .domain_event_broadcast
                .send(last.sequence);
        }
        let notification_count = stored
            .iter()
            .filter(|event| {
                crate::notifications::dispatcher::notification_event_type(&event.payload).is_some()
            })
            .count();
        if notification_count > 0
            && let Some(last) = stored.last()
        {
            tracing::debug!(
                high_water_sequence = last.sequence,
                batch_len = stored.len(),
                notification_events = notification_count,
                "queued notification dispatcher wake for notification-relevant domain event batch"
            );
            let _ = self
                .runtime
                .events
                .notification_event_broadcast
                .send(last.sequence);
        }
        for event in &stored {
            self.maybe_accelerate_discovery_sync_for_scan_completion(event)
                .await;
        }
        Ok(stored)
    }

    async fn maybe_accelerate_discovery_sync_for_scan_completion(&self, event: &DomainEvent) {
        let scryer_domain::DomainEventPayload::LibraryScanCompleted(data) = &event.payload else {
            return;
        };
        if data.found_titles <= 0 {
            return;
        }

        match self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
        {
            Ok(Some(state)) if state.last_success_generation_id.is_some() => {}
            Ok(_) => {
                if let Err(error) = self
                    .schedule_discovery_sync_soon_silent(
                        "library_scan_completed_before_first_snapshot",
                    )
                    .await
                {
                    tracing::warn!(
                        error = %error,
                        sequence = event.sequence,
                        facet = event.facet.as_ref().map(MediaFacet::as_str),
                        "failed to accelerate discovery sync after library scan"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    sequence = event.sequence,
                    facet = event.facet.as_ref().map(MediaFacet::as_str),
                    "failed to inspect discovery sync state for scan acceleration"
                );
            }
        }
    }

    pub async fn update_import_status_and_notify(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        self.services
            .workflow
            .imports
            .update_import_status(import_id, status, result_json.clone())
            .await?;
        if matches!(status, ImportStatus::Completed | ImportStatus::Failed) {
            let _ = self.runtime.events.import_history_broadcast.send(());
        }

        if let Some(ref json) = result_json
            && let Ok(result) = serde_json::from_str::<ImportResult>(json)
            && matches!(status, ImportStatus::Failed | ImportStatus::Skipped)
        {
            let title = match result.title_id.as_ref() {
                Some(title_id) => self
                    .services
                    .catalog
                    .titles
                    .get_by_id(title_id)
                    .await?
                    .map(|title| crate::domain_events::title_context_snapshot(&title)),
                None => None,
            };
            let reason = result.error_message.clone().or_else(|| {
                result
                    .skip_reason
                    .as_ref()
                    .map(|reason| reason.as_str().to_string())
            });

            let event = if let Some(title_id) = result.title_id.as_ref() {
                let facet = title.as_ref().map(|snapshot| snapshot.facet.clone());
                NewDomainEvent {
                    event_id: Id::new().0,
                    occurred_at: Utc::now(),
                    actor_kind: scryer_domain::DomainEventActorKind::System,
                    actor_user_id: None,
                    actor_display_name: "System".to_string(),
                    title_id: Some(title_id.clone()),
                    facet,
                    correlation_id: None,
                    causation_id: None,
                    schema_version: 1,
                    stream: scryer_domain::DomainEventStream::Title {
                        title_id: title_id.clone(),
                    },
                    payload: scryer_domain::DomainEventPayload::ImportRejected(
                        scryer_domain::ImportRejectedEventData {
                            title,
                            status,
                            import_id: Some(result.import_id.clone()),
                            source_system: result.source_system.clone(),
                            source_ref: result.source_ref.clone(),
                            source_title: result.source_title.clone(),
                            source_path: Some(result.source_path.clone()),
                            dest_path: result.dest_path.clone(),
                            quality: result.quality.clone(),
                            reason,
                            skip_reason: result.skip_reason.clone(),
                            episode_ids: result.episode_ids.clone(),
                        },
                    ),
                }
            } else {
                crate::domain_events::new_global_domain_event(
                    None,
                    scryer_domain::DomainEventPayload::ImportRejected(
                        scryer_domain::ImportRejectedEventData {
                            title: None,
                            status,
                            import_id: Some(result.import_id.clone()),
                            source_system: result.source_system.clone(),
                            source_ref: result.source_ref.clone(),
                            source_title: result.source_title.clone(),
                            source_path: Some(result.source_path.clone()),
                            dest_path: result.dest_path.clone(),
                            quality: result.quality.clone(),
                            reason,
                            skip_reason: result.skip_reason.clone(),
                            episode_ids: result.episode_ids.clone(),
                        },
                    ),
                )
            };

            let _ = self.append_domain_event(event).await;
        }
        Ok(())
    }

    pub fn publish_settings_changed(&self, changed_keys: Vec<String>) {
        let _ = self
            .runtime
            .events
            .settings_changed_broadcast
            .send(changed_keys);
    }

    pub fn publish_indexers_changed(&self) {
        let _ = self.runtime.events.indexers_changed_broadcast.send(());
    }

    pub fn publish_provider_catalog_changed(&self, families: Vec<ProviderCatalogFamily>) {
        if families.is_empty() {
            return;
        }

        let _ = self
            .runtime
            .events
            .provider_catalog_changed_broadcast
            .send(families);
    }

    pub async fn indexer_query_stats(&self, actor: &User) -> AppResult<Vec<IndexerQueryStats>> {
        let settings_permissions = scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageSystemSettings,
            scryer_domain::AppPermission::ManageCatalogSettings,
        ]);
        if !self
            .has_any_app_permission(actor, settings_permissions)
            .await?
        {
            return Err(AppError::Unauthorized(
                "You do not have permission to perform this action".to_string(),
            ));
        }
        Ok(self.services.integrations.indexer_stats.all_stats())
    }

    pub async fn cached_health_check_results(
        &self,
        actor: &User,
    ) -> AppResult<Vec<HealthCheckResult>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        Ok(self.runtime.health.results.read().await.clone())
    }

    pub async fn list_import_history(
        &self,
        actor: &User,
        limit: usize,
    ) -> AppResult<Vec<ImportRecord>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        let records = self.services.workflow.imports.list_imports(limit).await?;
        let mut title_library_cache = std::collections::HashMap::<String, Option<String>>::new();
        let mut visible = Vec::new();
        for record in records {
            let title_id = record
                .result_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<scryer_domain::ImportResult>(json).ok())
                .and_then(|result| result.title_id)
                .or_else(|| {
                    serde_json::from_str::<crate::ManualImportRequestPayload>(&record.payload_json)
                        .ok()
                        .and_then(|payload| payload.title_id)
                });
            let allowed = if let Some(title_id) = title_id {
                let library_id = if let Some(cached) = title_library_cache.get(&title_id) {
                    cached.clone()
                } else {
                    let library_id = self
                        .services
                        .catalog
                        .titles
                        .get_by_id(&title_id)
                        .await?
                        .map(|title| title.library_id);
                    title_library_cache.insert(title_id.clone(), library_id.clone());
                    library_id
                };
                library_id
                    .as_ref()
                    .is_some_and(|library_id| allowed_library_ids.contains(library_id))
            } else {
                false
            };
            if allowed {
                visible.push(record);
            }
        }
        Ok(visible)
    }

    async fn require_library_permission_for_title(
        &self,
        actor: &User,
        title_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(actor, &title.library_id, permission)
            .await
    }

    async fn require_any_library_permission_for_service(
        &self,
        actor: &User,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        if self
            .authorized_library_ids(actor, None, permission)
            .await?
            .is_empty()
        {
            Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    async fn derive_wanted_item_library_id(
        &self,
        wanted: &AcquisitionScopeState,
    ) -> AppResult<String> {
        if let Some(library_id) = wanted.library_id.as_deref() {
            return Ok(library_id.to_string());
        }
        self.services
            .catalog
            .titles
            .get_by_id(&wanted.title_id)
            .await?
            .map(|title| title.library_id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", wanted.title_id)))
    }

    /// Retain only the acquisition scope states whose owning library the actor
    /// holds `permission` on. Mirrors the per-item permission derivation used by
    /// `get_wanted_item` / `get_wanted_item_for_management` (the joined
    /// `library_id` is the title's library), silently dropping forbidden or
    /// orphaned rows for batch/dataloader callers.
    pub(crate) async fn filter_wanted_items_for_permission(
        &self,
        actor: &User,
        items: Vec<AcquisitionScopeState>,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(items
            .into_iter()
            .filter(|item| {
                item.library_id
                    .as_deref()
                    .is_some_and(|library_id| allowed_library_ids.contains(library_id))
            })
            .collect())
    }

    pub async fn find_download_submission_by_client_item_id(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        let submission = self
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&DownloadSourceIdentity::new(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        if let Some(submission) = submission.as_ref() {
            self.require_library_permission_for_title(
                actor,
                &submission.title_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(submission)
    }

    pub async fn search_metadata(
        &self,
        actor: &User,
        query: &str,
        type_hint: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb_rich(query, type_hint, limit, language, year)
            .await
    }

    pub async fn search_metadata_tvdb(
        &self,
        actor: &User,
        query: &str,
        type_hint: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb(query, type_hint, year)
            .await
    }

    pub async fn search_metadata_batch(
        &self,
        actor: &User,
        queries: &[MetadataSearchQuery],
        language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb_batch(queries, language)
            .await
    }

    pub async fn search_metadata_multi(
        &self,
        actor: &User,
        query: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .search_tvdb_multi(query, limit, language)
            .await
    }

    pub async fn get_metadata_movie(
        &self,
        actor: &User,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<MovieMetadata> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .get_movie(tvdb_id, language)
            .await
    }

    pub async fn get_metadata_series(
        &self,
        actor: &User,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<SeriesMetadata> {
        self.require_any_library_permission_for_service(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .metadata_gateway
            .get_series(tvdb_id, language)
            .await
    }

    pub async fn list_title_media_files(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<TitleMediaFile>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await
    }

    pub async fn list_episode_media_files(
        &self,
        actor: &User,
        title_id: &str,
        episode_id: &str,
    ) -> AppResult<Vec<TitleMediaFile>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;

        let episode_ids = vec![episode_id.to_string()];
        let scoped_files = self
            .services
            .library
            .media_files
            .list_live_media_files_for_episode_ids(title_id, &episode_ids)
            .await?;

        Ok(scoped_files
            .into_iter()
            .filter_map(|scoped_file| {
                if !scoped_file
                    .episode_ids
                    .iter()
                    .any(|scoped_episode_id| scoped_episode_id == episode_id)
                {
                    return None;
                }

                let mut media_file = scoped_file.media_file;
                media_file.episode_id = Some(episode_id.to_string());
                Some(media_file)
            })
            .collect())
    }

    /// Batch variant of [`Self::list_episode_media_files`] for one title:
    /// one permission check and one scoped-files fetch cover every requested
    /// episode id, grouped per episode. A missing or non-`View`-visible title
    /// yields an empty map (silent drop, matching the loader-facing batches).
    pub async fn list_episode_media_files_for_title(
        &self,
        actor: &User,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, Vec<TitleMediaFile>>> {
        if episode_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let Some(title) = self.services.catalog.titles.get_by_id(title_id).await? else {
            return Ok(std::collections::HashMap::new());
        };
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        if !allowed_library_ids.contains(&title.library_id) {
            return Ok(std::collections::HashMap::new());
        }
        let scoped_files = self
            .services
            .library
            .media_files
            .list_live_media_files_for_episode_ids(title_id, episode_ids)
            .await?;
        let mut files_by_episode: std::collections::HashMap<String, Vec<TitleMediaFile>> =
            std::collections::HashMap::new();
        for scoped_file in scoped_files {
            for episode_id in episode_ids {
                if scoped_file
                    .episode_ids
                    .iter()
                    .any(|scoped_episode_id| scoped_episode_id == episode_id)
                {
                    let mut media_file = scoped_file.media_file.clone();
                    media_file.episode_id = Some(episode_id.clone());
                    files_by_episode
                        .entry(episode_id.clone())
                        .or_default()
                        .push(media_file);
                }
            }
        }
        Ok(files_by_episode)
    }

    pub async fn get_title_wanted_item(
        &self,
        actor: &User,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_for_title(title_id, episode_id)
            .await
    }

    /// Batch variant of [`Self::get_title_wanted_item`]: returns every acquisition
    /// scope state for the `View`-visible subset of `title_ids`. Callers key the
    /// flat result by `(title_id, episode_id)`.
    pub async fn get_title_wanted_items_for_titles(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let visible_title_ids = self
            .get_titles_by_ids(actor, title_ids)
            .await?
            .into_iter()
            .map(|title| title.id)
            .collect::<Vec<_>>();
        if visible_title_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states_for_title_ids(&visible_title_ids)
            .await
    }

    pub async fn get_title_for_management(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Option<Title>> {
        let title = self.services.catalog.titles.get_by_id(title_id).await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        }
        Ok(title)
    }

    pub async fn get_wanted_item_for_management(
        &self,
        actor: &User,
        wanted_item_id: &str,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(wanted_item_id)
            .await?;
        if let Some(wanted) = wanted.as_ref() {
            let library_id = self.derive_wanted_item_library_id(wanted).await?;
            self.require_library_permission(
                actor,
                &library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        }
        Ok(wanted)
    }

    /// Batch variant of [`Self::get_wanted_item_for_management`]: loads wanted
    /// items by id and silently drops those the actor cannot manage.
    pub async fn get_wanted_items_by_ids_for_management(
        &self,
        actor: &User,
        ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states_by_ids(ids)
            .await?;
        self.filter_wanted_items_for_permission(
            actor,
            items,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await
    }

    pub async fn get_title_for_download_actions(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Option<Title>> {
        let title = self.services.catalog.titles.get_by_id(title_id).await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        }
        Ok(title)
    }

    pub async fn get_completed_download(
        &self,
        actor: &User,
        download_client_item_id: &str,
    ) -> AppResult<CompletedDownload> {
        if self
            .authorized_library_ids(
                actor,
                None,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?
            .is_empty()
        {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }
        let download_client_item_id = download_client_item_id.trim();
        if download_client_item_id.is_empty() {
            return Err(AppError::Validation(
                "download client item id is required".into(),
            ));
        }

        self.services
            .integrations
            .download_client
            .list_completed_downloads()
            .await?
            .into_iter()
            .find(|download| download.download_client_item_id == download_client_item_id)
            .ok_or_else(|| {
                AppError::NotFound(format!("completed download {download_client_item_id}"))
            })
    }

    pub async fn connect_library_scan_tracker(&self) {
        self.runtime
            .library
            .library_scan_tracker
            .set_job_run_tracker(self.runtime.jobs.job_run_tracker.clone())
            .await;
    }

    pub fn wake_title_image_loops(&self) {
        self.runtime.catalog.poster_wake.notify_one();
        self.runtime.catalog.fanart_wake.notify_one();
    }

    pub async fn primary_enabled_download_client_config(
        &self,
    ) -> AppResult<Option<DownloadClientConfig>> {
        Ok(self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .min_by_key(|config| config.client_priority))
    }

    pub async fn active_library_scan_sessions(&self) -> Vec<LibraryScanSession> {
        self.runtime
            .library
            .library_scan_tracker
            .list_active()
            .await
    }

    pub fn user_rules_engine_snapshot(&self) -> scryer_rules::UserRulesEngine {
        self.services
            .customization
            .user_rules
            .read()
            .unwrap()
            .clone()
    }
}

fn should_invalidate_monitored_title_matcher(payload: &scryer_domain::DomainEventPayload) -> bool {
    matches!(
        payload,
        scryer_domain::DomainEventPayload::TitleAdded(_)
            | scryer_domain::DomainEventPayload::TitleUpdated(_)
            | scryer_domain::DomainEventPayload::TitleDeleted(_)
    )
}

type RuntimePerformanceProbe =
    Arc<dyn Fn(PathBuf) -> RuntimePerformanceSnapshot + Send + Sync + 'static>;

async fn initialize_runtime_performance_snapshot(
    cell: &OnceCell<RuntimePerformanceSnapshot>,
    config_dir: Arc<PathBuf>,
    probe: RuntimePerformanceProbe,
) -> RuntimePerformanceSnapshot {
    cell.get_or_init(|| async move {
        let config_dir_for_probe = config_dir.as_ref().clone();
        let config_dir_for_log = config_dir_for_probe.clone();
        let snapshot =
            match tokio::task::spawn_blocking(move || (probe)(config_dir_for_probe)).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "runtime performance probe task failed; using conservative slow defaults"
                    );
                    RuntimePerformanceSnapshot::slow()
                }
            };
        tracing::info!(
            cpu_class = %snapshot.cpu_class,
            config_io_class = %snapshot.config_io_class,
            cpu_probe_elapsed_ms = snapshot.cpu_probe_elapsed_ms,
            config_io_probe_elapsed_ms = snapshot.config_io_probe_elapsed_ms,
            config_dir = %config_dir_for_log.display(),
            "runtime performance probe settled"
        );
        snapshot
    })
    .await
    .clone()
}

fn probe_runtime_performance_snapshot(config_dir: PathBuf) -> RuntimePerformanceSnapshot {
    let (cpu_class, cpu_probe_elapsed_ms) = probe_cpu_performance();
    let (config_io_class, config_io_probe_elapsed_ms) = probe_config_io_performance(&config_dir);
    RuntimePerformanceSnapshot {
        cpu_class,
        config_io_class,
        cpu_probe_elapsed_ms,
        config_io_probe_elapsed_ms,
    }
}

fn classify_cpu_elapsed(elapsed: std::time::Duration) -> RuntimePerformanceClass {
    if elapsed <= std::time::Duration::from_millis(125) {
        RuntimePerformanceClass::Fast
    } else {
        RuntimePerformanceClass::Slow
    }
}

fn probe_cpu_performance() -> (RuntimePerformanceClass, Option<u64>) {
    const CPU_PROBE_BYTES: usize = 8 * 1024 * 1024;
    const CPU_PROBE_PASSES: usize = 32;
    const SLOW_CAP: std::time::Duration = std::time::Duration::from_millis(250);

    let mut buffer = vec![0_u64; CPU_PROBE_BYTES / std::mem::size_of::<u64>()];
    let start = std::time::Instant::now();
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;

    for pass in 0..CPU_PROBE_PASSES {
        for word in &mut buffer {
            state = state
                .wrapping_add(0xA076_1D64_78BD_642F_u64 ^ (pass as u64))
                .rotate_left(13);
            let mixed = state ^ word.rotate_left((state & 31) as u32) ^ 0xE703_7ED1_A0B4_28DB_u64;
            *word = word.wrapping_add(mixed).rotate_left(7) ^ mixed;
            std::hint::black_box(*word);
        }

        if start.elapsed() > SLOW_CAP {
            let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            return (RuntimePerformanceClass::Slow, Some(elapsed_ms));
        }
    }

    std::hint::black_box(state);
    let elapsed = start.elapsed();
    let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    (classify_cpu_elapsed(elapsed), Some(elapsed_ms))
}

fn classify_config_io_elapsed(elapsed: std::time::Duration) -> RuntimePerformanceClass {
    if elapsed <= std::time::Duration::from_millis(200) {
        RuntimePerformanceClass::Fast
    } else {
        RuntimePerformanceClass::Slow
    }
}

fn probe_config_io_performance(config_dir: &Path) -> (RuntimePerformanceClass, Option<u64>) {
    const PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
    const CHUNK_BYTES: usize = 1024 * 1024;
    const SLOW_CAP: std::time::Duration = std::time::Duration::from_millis(500);

    if !config_dir.is_dir() && std::fs::create_dir_all(config_dir).is_err() {
        return (RuntimePerformanceClass::Slow, None);
    }

    let probe_name = format!(
        ".scryer-runtime-probe-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let probe_path = config_dir.join(probe_name);
    let chunk = vec![0x5Au8; CHUNK_BYTES];
    let start = std::time::Instant::now();

    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&probe_path)?;
        let mut written = 0;
        while written < PAYLOAD_BYTES {
            let to_write = std::cmp::min(CHUNK_BYTES, PAYLOAD_BYTES - written);
            file.write_all(&chunk[..to_write])?;
            written += to_write;
            if start.elapsed() > SLOW_CAP {
                return Ok(());
            }
        }
        file.flush()?;
        file.sync_all()?;

        let mut file = std::fs::File::open(&probe_path)?;
        let mut read_buffer = vec![0_u8; CHUNK_BYTES];
        loop {
            let bytes_read = file.read(&mut read_buffer)?;
            if bytes_read == 0 {
                break;
            }
            std::hint::black_box(&read_buffer[..bytes_read]);
            if start.elapsed() > SLOW_CAP {
                return Ok(());
            }
        }

        Ok(())
    })();

    let cleanup_result = std::fs::remove_file(&probe_path);

    if result.is_err() || cleanup_result.is_err() {
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).ok();
        return (RuntimePerformanceClass::Slow, elapsed_ms);
    }

    let elapsed = start.elapsed();
    let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    (classify_config_io_elapsed(elapsed), Some(elapsed_ms))
}

#[cfg(test)]
mod category_ownership_tests {
    use super::{DownloadClientCategoryOwnershipSnapshot, normalize_owned_download_category};
    use std::collections::{HashMap, HashSet};

    fn snapshot(
        defaults: &[&str],
        per_client: &[(&str, &[&str])],
    ) -> DownloadClientCategoryOwnershipSnapshot {
        DownloadClientCategoryOwnershipSnapshot {
            default_categories: defaults
                .iter()
                .map(|value| normalize_owned_download_category(value))
                .collect::<HashSet<_>>(),
            categories_by_client: per_client
                .iter()
                .map(|(client, categories)| {
                    (
                        (*client).to_string(),
                        categories
                            .iter()
                            .map(|value| normalize_owned_download_category(value))
                            .collect::<HashSet<_>>(),
                    )
                })
                .collect::<HashMap<_, _>>(),
        }
    }

    #[test]
    fn category_ownership_ignores_the_client_canonical_casing() {
        // The real failure: Scryer is configured with `movies`, NZBGet's own
        // Category1.Name is `Movies`, and NZBGet reports ITS spelling back in
        // history. Comparing raw made Scryer's own completed download look
        // foreign, so it was filtered out of the tracked snapshot, never
        // imported, and re-grabbed by the RSS sweep minutes later.
        let snapshot = snapshot(&["movies"], &[("client-1", &["movies"])]);

        assert!(snapshot.owns_category("client-1", "Movies"));
        assert!(snapshot.knows_category("Movies"));
        assert!(snapshot.owns_category("client-1", "MOVIES"));
        assert!(snapshot.knows_category("  Movies  "));
    }

    #[test]
    fn normalization_is_the_shared_contract_between_both_category_gates() {
        // `completed_download_allows_automatic_import` and `knows_category` are
        // documented as having to agree about what counts as Scryer's own work.
        // They compare in different places, so they must fold identically —
        // normalizing only one silently splits them, and a download would be
        // eligible by one gate and foreign by the other.
        for (configured, reported) in [
            ("movies", "Movies"),
            ("Series", "series"),
            ("anime", "ANIME"),
            ("movies", "  movies  "),
        ] {
            assert_eq!(
                normalize_owned_download_category(configured),
                normalize_owned_download_category(reported),
                "{configured} vs {reported}"
            );
        }

        // Different categories must stay different after folding.
        assert_ne!(
            normalize_owned_download_category("movies"),
            normalize_owned_download_category("radarr")
        );
    }

    #[test]
    fn a_genuinely_unconfigured_category_is_still_unknown() {
        // Case-folding must not turn the gate off: a category Scryer never
        // configured still marks the download as another app's work.
        let snapshot = snapshot(&["movies"], &[("client-1", &["movies"])]);

        assert!(!snapshot.knows_category("radarr"));
        assert!(!snapshot.owns_category("client-1", "radarr"));
    }

    #[test]
    fn a_category_owned_by_another_client_is_known_but_not_owned() {
        // Per-client mismatch stays eligible (knows_category), while the
        // narrower ownership question still answers honestly.
        let snapshot = snapshot(&[], &[("client-1", &["movies"]), ("client-2", &["Series"])]);

        assert!(snapshot.knows_category("SERIES"));
        assert!(!snapshot.owns_category("client-1", "series"));
        assert!(snapshot.owns_category("client-2", "series"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use async_trait::async_trait;
    use scryer_domain::IndexerConfig;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    struct TestIndexerConfigRepository;

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepository {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(Vec::new())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(&self, _update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    fn test_builder() -> AppServicesBuilder {
        AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepository),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
    }

    #[test]
    #[should_panic(expected = "AppServicesBuilder missing required runtime services")]
    fn build_requires_explicit_runtime_dependencies() {
        let _ = test_builder().build();
    }

    #[test]
    fn build_partial_for_tests_allows_partial_test_assemblies() {
        let _ = test_builder().build_partial_for_tests();
    }

    #[test]
    fn runtime_build_identity_defaults_to_portable() {
        let runtime = AppRuntimeState::default();
        assert_eq!(runtime.environment.build_lane, BinaryLane::Portable);
        assert_eq!(runtime.environment.build_class, BinaryClass::Portable);
        assert!(
            runtime
                .environment
                .supported_plugin_required_features
                .is_empty()
        );
    }

    #[test]
    fn runtime_environment_builder_sets_build_identity() {
        let assembly = test_builder()
            .with_runtime_environment(
                BinaryLane::Haswell,
                "/tmp/scryer-config",
                Vec::<String>::new(),
            )
            .build_partial_for_tests();
        assert_eq!(assembly.runtime.environment.build_lane, BinaryLane::Haswell);
        assert_eq!(
            assembly.runtime.environment.build_class,
            BinaryClass::Optimized
        );
        assert_eq!(
            assembly.runtime.environment.config_dir.as_ref(),
            &PathBuf::from("/tmp/scryer-config")
        );
    }

    #[test]
    fn runtime_environment_builder_sets_supported_plugin_required_features() {
        let assembly = test_builder()
            .with_runtime_environment(
                BinaryLane::Portable,
                "/tmp/scryer-config",
                ["simd128", " relaxed-simd ", ""],
            )
            .build_partial_for_tests();
        assert_eq!(
            assembly
                .runtime
                .environment
                .supported_plugin_required_features
                .as_ref(),
            &HashSet::from(["simd128".to_string(), "relaxed-simd".to_string()])
        );
    }

    #[tokio::test]
    async fn runtime_performance_initializer_shares_one_probe_run() {
        let cell = Arc::new(OnceCell::new());
        let config_dir = Arc::new(PathBuf::from("."));
        let probe_runs = Arc::new(AtomicUsize::new(0));
        let probe: RuntimePerformanceProbe = Arc::new({
            let probe_runs = probe_runs.clone();
            move |_path: PathBuf| {
                probe_runs.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(50));
                RuntimePerformanceSnapshot {
                    cpu_class: RuntimePerformanceClass::Fast,
                    config_io_class: RuntimePerformanceClass::Slow,
                    cpu_probe_elapsed_ms: Some(50),
                    config_io_probe_elapsed_ms: Some(5),
                }
            }
        });

        let left = {
            let cell = cell.clone();
            let config_dir = config_dir.clone();
            let probe = probe.clone();
            tokio::spawn(async move {
                initialize_runtime_performance_snapshot(cell.as_ref(), config_dir, probe).await
            })
        };
        let right = {
            let cell = cell.clone();
            let config_dir = config_dir.clone();
            let probe = probe.clone();
            tokio::spawn(async move {
                initialize_runtime_performance_snapshot(cell.as_ref(), config_dir, probe).await
            })
        };

        let first = left.await.expect("left probe");
        let second = right.await.expect("right probe");
        assert_eq!(probe_runs.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);

        let start = std::time::Instant::now();
        let cached =
            initialize_runtime_performance_snapshot(cell.as_ref(), config_dir, probe).await;
        assert_eq!(cached, first);
        assert!(start.elapsed() < std::time::Duration::from_millis(20));
    }

    #[test]
    fn cpu_elapsed_threshold_classification_is_stable() {
        assert_eq!(
            classify_cpu_elapsed(std::time::Duration::from_millis(125)),
            RuntimePerformanceClass::Fast
        );
        assert_eq!(
            classify_cpu_elapsed(std::time::Duration::from_millis(126)),
            RuntimePerformanceClass::Slow
        );
    }

    #[test]
    fn config_io_elapsed_threshold_classification_is_stable() {
        assert_eq!(
            classify_config_io_elapsed(std::time::Duration::from_millis(200)),
            RuntimePerformanceClass::Fast
        );
        assert_eq!(
            classify_config_io_elapsed(std::time::Duration::from_millis(201)),
            RuntimePerformanceClass::Slow
        );
    }

    #[test]
    fn config_io_probe_creates_missing_directory_before_measuring() {
        let temp = tempdir().expect("tempdir");
        let missing = temp.path().join("missing");
        let (class, elapsed_ms) = probe_config_io_performance(&missing);
        assert!(matches!(
            class,
            RuntimePerformanceClass::Slow | RuntimePerformanceClass::Fast
        ));
        assert!(missing.is_dir());
        assert!(elapsed_ms.is_some());
    }

    #[tokio::test]
    async fn external_import_warmup_begin_creates_new_session_after_completion() {
        let orchestrator = ExternalImportMonitorWarmupOrchestrator::default();
        let first = orchestrator
            .begin(
                "user-1",
                "fingerprint-1",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-1".into()),
            )
            .await;
        assert!(first.created);
        let _subscription = orchestrator
            .subscribe("user-1", &first.snapshot.session_id)
            .await
            .expect("subscribe to first session");

        let mut completed = first.snapshot.clone();
        completed.status = ExternalImportMonitorWarmupStatus::Completed;
        assert!(
            orchestrator
                .update(&completed.session_id, completed.clone())
                .await
        );

        let second = orchestrator
            .begin(
                "user-1",
                "fingerprint-1",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-2".into()),
            )
            .await;

        assert!(second.created);
        assert_ne!(second.snapshot.session_id, first.snapshot.session_id);
    }

    #[tokio::test]
    async fn external_import_prowlarr_warmup_deduplicates_per_actor_and_isolates_results() {
        let orchestrator = ExternalImportMonitorWarmupOrchestrator::default();
        let first = orchestrator
            .begin(
                "user-1",
                "prowlarr-source=http://prowlarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-1".into()),
            )
            .await;
        let reused = orchestrator
            .begin(
                "user-1",
                "prowlarr-source=http://prowlarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-2".into()),
            )
            .await;
        let other_actor = orchestrator
            .begin(
                "user-2",
                "prowlarr-source=http://prowlarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-3".into()),
            )
            .await;

        assert!(first.created);
        assert!(!reused.created);
        assert_eq!(reused.snapshot.session_id, first.snapshot.session_id);
        assert!(other_actor.created);
        assert_ne!(other_actor.snapshot.session_id, first.snapshot.session_id);

        assert!(
            orchestrator
                .set_prowlarr_result(
                    &first.snapshot.session_id,
                    ExternalImportProwlarrWarmupResult {
                        base_url: "http://prowlarr".into(),
                        api_key: "key".into(),
                        version: Some("2.0.0".into()),
                        plan: crate::IndexerSyncPlan {
                            children: Vec::new(),
                        },
                    },
                )
                .await
        );
        assert!(
            orchestrator
                .prowlarr_result("user-1", &first.snapshot.session_id)
                .await
                .is_some()
        );
        assert!(
            orchestrator
                .prowlarr_result("user-2", &first.snapshot.session_id)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn external_import_warmup_prune_only_removes_import_source_sessions() {
        let orchestrator = ExternalImportMonitorWarmupOrchestrator::default();
        let source = orchestrator
            .begin(
                "user-1",
                "arr-source=sonarr|http://sonarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("source-session".into()),
            )
            .await;
        let prowlarr_source = orchestrator
            .begin(
                "user-1",
                "prowlarr-source=http://prowlarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("prowlarr-session".into()),
            )
            .await;
        let apply = orchestrator
            .begin(
                "user-1",
                crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID,
                ExternalImportMonitorWarmupProgressSnapshot::new(
                    crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID.to_string(),
                ),
            )
            .await;
        let old_updated_at = (Utc::now() - chrono::Duration::hours(3)).to_rfc3339();

        for snapshot in [&source.snapshot, &prowlarr_source.snapshot, &apply.snapshot] {
            let mut completed = snapshot.clone();
            completed.status = ExternalImportMonitorWarmupStatus::Completed;
            completed.updated_at = old_updated_at.clone();
            let session_id = completed.session_id.clone();
            assert!(orchestrator.update(&session_id, completed).await);
        }

        let removed = orchestrator
            .prune_terminal_older_than(chrono::Duration::hours(2))
            .await;

        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"source-session".to_string()));
        assert!(removed.contains(&"prowlarr-session".to_string()));
        assert!(
            orchestrator
                .snapshot("user-1", crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID)
                .await
                .is_some(),
            "apply session should not be pruned as a source session"
        );
    }

    #[tokio::test]
    async fn external_import_warmup_update_persists_without_active_subscribers() {
        let orchestrator = ExternalImportMonitorWarmupOrchestrator::default();
        let begin = orchestrator
            .begin(
                "user-1",
                "fingerprint-1",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-1".into()),
            )
            .await;
        assert!(begin.created);

        let mut running = begin.snapshot.clone();
        running.status = ExternalImportMonitorWarmupStatus::Running;
        running.phase = ExternalImportMonitorWarmupPhase::LoadingSeries;
        running.series_total_known = true;
        running.series_progress.total = 42;
        running.series_progress.completed = 17;

        assert!(
            orchestrator
                .update(&running.session_id, running.clone())
                .await
        );

        let snapshot = orchestrator
            .snapshot("user-1", &running.session_id)
            .await
            .expect("stored snapshot");

        assert_eq!(snapshot.status, ExternalImportMonitorWarmupStatus::Running);
        assert_eq!(
            snapshot.phase,
            ExternalImportMonitorWarmupPhase::LoadingSeries
        );
        assert!(snapshot.series_total_known);
        assert_eq!(snapshot.series_progress.total, 42);
        assert_eq!(snapshot.series_progress.completed, 17);
    }

    #[tokio::test]
    async fn backup_execution_guards_allow_cross_trigger_overlap_but_block_same_trigger() {
        let guards = BackupExecutionGuardTable::default();

        let manual_guard = guards
            .try_acquire("manual")
            .await
            .expect("manual guard should acquire");
        let auto_guard = guards
            .try_acquire("auto")
            .await
            .expect("auto guard should acquire while manual is running");

        assert!(
            guards.try_acquire("manual").await.is_none(),
            "same-trigger manual backup should be blocked",
        );
        assert!(
            guards.try_acquire("auto").await.is_none(),
            "same-trigger automatic backup should be blocked",
        );

        drop(manual_guard);
        assert!(
            guards.try_acquire("manual").await.is_some(),
            "manual guard should be released after completion",
        );

        drop(auto_guard);
        assert!(
            guards.try_acquire("auto").await.is_some(),
            "automatic guard should be released after completion",
        );
    }

    #[tokio::test]
    async fn interactive_operation_guards_allow_distinct_resources_but_block_duplicates() {
        let guards = InteractiveOperationGuardTable::default();

        let media_file_guard = guards
            .try_acquire("media-file:file-1")
            .await
            .expect("media file guard should acquire");
        let recycle_entry_guard = guards
            .try_acquire("recycle-entry:entry-1")
            .await
            .expect("recycle entry guard should acquire independently");

        assert!(
            guards.try_acquire("media-file:file-1").await.is_none(),
            "the same media file must not queue a duplicate operation",
        );
        assert!(
            guards.try_acquire("recycle-entry:entry-1").await.is_none(),
            "the same recycle entry must not queue a duplicate operation",
        );

        drop(media_file_guard);
        drop(recycle_entry_guard);

        assert!(guards.try_acquire("media-file:file-1").await.is_some());
        assert!(guards.try_acquire("recycle-entry:entry-1").await.is_some());
    }

    #[tokio::test]
    async fn acquire_scope_serializes_overlapping_scopes_for_same_title() {
        let guards = DownloadSubmissionGuardTable::default();
        let title_guard = guards
            .acquire_scope("title-1", &SubmissionScope::Title)
            .await;

        let guards_clone = guards.clone();
        let waiting_guard = tokio::spawn(async move {
            guards_clone
                .acquire_scope(
                    "title-1",
                    &SubmissionScope::Episode {
                        episode_id: "episode-1".to_string(),
                    },
                )
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!waiting_guard.is_finished());

        drop(title_guard);

        tokio::time::timeout(std::time::Duration::from_secs(1), waiting_guard)
            .await
            .expect("overlapping scope guard should acquire after release")
            .expect("scope task should complete");
    }

    #[tokio::test]
    async fn plugin_install_orchestrator_rejects_second_begin_for_same_plugin_until_terminal() {
        let orchestrator = PluginInstallOrchestrator::default();
        let snapshot = orchestrator
            .begin("admin", "email", PluginInstallOperationKind::Install)
            .await
            .expect("first install should start");
        assert_eq!(snapshot.state, PluginInstallState::Downloading);

        let err = orchestrator
            .begin("viewer", "email", PluginInstallOperationKind::Upgrade)
            .await
            .expect_err("same plugin should be globally locked");
        assert_eq!(
            err,
            PluginInstallInProgressError {
                plugin_id: "email".to_string(),
            }
        );

        orchestrator
            .transition("admin", "email", PluginInstallState::Succeeded, None, None)
            .await;

        let upgrade = orchestrator
            .begin("viewer", "email", PluginInstallOperationKind::Upgrade)
            .await
            .expect("terminal state should release plugin lock");
        assert_eq!(upgrade.state, PluginInstallState::Downloading);
        assert_eq!(upgrade.operation_kind, PluginInstallOperationKind::Upgrade);
    }

    #[tokio::test]
    async fn plugin_install_orchestrator_scopes_progress_to_initiating_actor() {
        let orchestrator = PluginInstallOrchestrator::default();
        orchestrator
            .begin("admin", "email", PluginInstallOperationKind::Install)
            .await
            .expect("install should start");

        let admin_active = orchestrator.active_plugin_ids_for_actor("admin").await;
        assert!(admin_active.contains("email"));
        let viewer_active = orchestrator.active_plugin_ids_for_actor("viewer").await;
        assert!(viewer_active.is_empty());

        let admin_rx = orchestrator
            .subscribe("admin", "email")
            .await
            .expect("initiating actor should see snapshot");
        assert_eq!(admin_rx.borrow().state, PluginInstallState::Downloading);

        assert!(
            orchestrator.subscribe("viewer", "email").await.is_none(),
            "other actors should not see the snapshot"
        );

        orchestrator
            .transition(
                "admin",
                "email",
                PluginInstallState::Verifying,
                Some("verifying manifest".to_string()),
                None,
            )
            .await;

        let admin_rx = orchestrator
            .subscribe("admin", "email")
            .await
            .expect("snapshot should remain visible to initiator");
        let snapshot = admin_rx.borrow().clone();
        assert_eq!(snapshot.state, PluginInstallState::Verifying);
        assert_eq!(snapshot.message.as_deref(), Some("verifying manifest"));
    }
}
