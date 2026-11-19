//! One archive extraction per tracked download.
//!
//! A download's archives are extracted once, into a workspace at a path
//! derived from the download client and the client's item id, and every import
//! pass of that download reads the same output. A record per download says
//! where the workspace is and what state it is in, so a retry or a restart
//! never extracts the same download again. Import moves files out of the
//! workspace, so what remains in it is what is still to import.
//!
//! Nothing is written for a download without archives, or while no installed
//! plugin can extract them.
//!
//! A download gets [`MAX_ATTEMPTS`] attempts, spaced by a backoff; after that
//! it is blocked until an operator retries. The workspace lives as long as the
//! download is unsettled: it goes when the download is imported, and a day
//! after the download ends any other way.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::archive_extractor::{self, ArchiveExtractionDestination, DownloadWorkspacePresence};
use crate::error_reasons::archive_extraction as reasons;
use crate::{AppError, AppResult, AppUseCase, ArchiveExtractorPluginProvider, ClientJobLocator};

/// The folder in a library root that holds download workspaces. It is
/// Scryer's scratch and never library content.
pub const ARCHIVE_EXTRACTION_DIR_NAME: &str = ".scry-ex";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveExtractionState {
    /// Archives found, not yet extracted.
    Pending,
    /// An extraction is running, or was when the process stopped.
    Extracting,
    /// The complete output is on disk.
    Extracted,
    /// The archives refused every password tried.
    NeedsPassword,
    /// The last attempt failed; a later pass tries again.
    Failed,
    /// No further attempt is made until an operator acts.
    Blocked,
    /// The workspace is gone.
    Removed,
}

impl ArchiveExtractionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Extracting => "extracting",
            Self::Extracted => "extracted",
            Self::NeedsPassword => "needs_password",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Removed => "removed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "extracting" => Self::Extracting,
            "extracted" => Self::Extracted,
            "needs_password" => Self::NeedsPassword,
            "failed" => Self::Failed,
            "blocked" => Self::Blocked,
            "removed" => Self::Removed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveExtractionRecord {
    pub client_id: String,
    pub client_item_id: String,
    pub state: ArchiveExtractionState,
    pub workspace_path: String,
    pub archive_digest: Option<String>,
    /// Extraction attempts started, including one a restart interrupted.
    pub attempts: u32,
    pub last_reason_code: Option<String>,
    pub output_count: u64,
    pub output_bytes: u64,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One row per download whose workspace is on disk.
#[async_trait]
pub trait ArchiveExtractionRepository: Send + Sync {
    async fn get(
        &self,
        client_id: &str,
        client_item_id: &str,
    ) -> AppResult<Option<ArchiveExtractionRecord>>;
    /// Insert or replace the download's row.
    async fn save(&self, record: &ArchiveExtractionRecord) -> AppResult<()>;
    async fn delete(&self, client_id: &str, client_item_id: &str) -> AppResult<()>;
    /// Record that these downloads ended at `at`; one already settled keeps
    /// its time.
    async fn settle(&self, keys: &[(String, String)], at: DateTime<Utc>) -> AppResult<()>;
    /// Move these downloads' settled time to `at`, so the removal is tried
    /// again later.
    async fn defer_settled(&self, keys: &[(String, String)], at: DateTime<Utc>) -> AppResult<()>;
    /// Downloads settled at or before `before`, oldest first, at most `limit`.
    async fn list_settled_before(
        &self,
        before: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<ArchiveExtractionRecord>>;
    async fn delete_many(&self, keys: &[(String, String)]) -> AppResult<()>;
}

/// The repository a service bundle starts with. Assembly replaces it with
/// the datastore's.
#[derive(Default)]
pub struct InMemoryArchiveExtractionRepository {
    rows: tokio::sync::Mutex<std::collections::HashMap<(String, String), ArchiveExtractionRecord>>,
}

#[async_trait]
impl ArchiveExtractionRepository for InMemoryArchiveExtractionRepository {
    async fn get(
        &self,
        client_id: &str,
        client_item_id: &str,
    ) -> AppResult<Option<ArchiveExtractionRecord>> {
        Ok(self
            .rows
            .lock()
            .await
            .get(&(client_id.to_string(), client_item_id.to_string()))
            .cloned())
    }

    async fn save(&self, record: &ArchiveExtractionRecord) -> AppResult<()> {
        self.rows.lock().await.insert(
            (record.client_id.clone(), record.client_item_id.clone()),
            record.clone(),
        );
        Ok(())
    }

    async fn delete(&self, client_id: &str, client_item_id: &str) -> AppResult<()> {
        self.rows
            .lock()
            .await
            .remove(&(client_id.to_string(), client_item_id.to_string()));
        Ok(())
    }

    async fn settle(&self, keys: &[(String, String)], at: DateTime<Utc>) -> AppResult<()> {
        let mut rows = self.rows.lock().await;
        for key in keys {
            if let Some(row) = rows.get_mut(key)
                && row.settled_at.is_none()
            {
                row.settled_at = Some(at);
                row.updated_at = at;
            }
        }
        Ok(())
    }

    async fn defer_settled(&self, keys: &[(String, String)], at: DateTime<Utc>) -> AppResult<()> {
        let mut rows = self.rows.lock().await;
        for key in keys {
            if let Some(row) = rows.get_mut(key) {
                row.settled_at = Some(at);
                row.updated_at = at;
            }
        }
        Ok(())
    }

    async fn list_settled_before(
        &self,
        before: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<ArchiveExtractionRecord>> {
        let rows = self.rows.lock().await;
        let mut due: Vec<_> = rows
            .values()
            .filter(|row| row.settled_at.is_some_and(|at| at <= before))
            .cloned()
            .collect();
        due.sort_by_key(|row| row.settled_at);
        due.truncate(limit);
        Ok(due)
    }

    async fn delete_many(&self, keys: &[(String, String)]) -> AppResult<()> {
        let mut rows = self.rows.lock().await;
        for key in keys {
            rows.remove(key);
        }
        Ok(())
    }
}

/// Extraction attempts a download gets before it is blocked.
pub(crate) const MAX_ATTEMPTS: u32 = 3;

/// How long after the `attempts`th failed attempt the next one may start.
fn attempt_backoff(attempts: u32) -> chrono::Duration {
    match attempts {
        0 | 1 => chrono::Duration::minutes(5),
        _ => chrono::Duration::minutes(20),
    }
}

/// How long a download that ended without being imported keeps its
/// workspace.
pub(crate) const SETTLED_WORKSPACE_RETENTION: chrono::Duration = chrono::Duration::hours(24);

/// Settled downloads one housekeeping pass handles.
const SETTLED_SWEEP_LIMIT: usize = 100;

/// A tracked download, as its extraction is keyed: the download client id
/// and the client's item id, compared the way import sources are.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DownloadExtractionKey {
    client_id: String,
    client_item_id: String,
}

impl DownloadExtractionKey {
    #[cfg(test)]
    pub(crate) async fn record_in(&self, app: &AppUseCase) -> Option<ArchiveExtractionRecord> {
        app.services
            .workflow
            .archive_extractions
            .get(&self.client_id, &self.client_item_id)
            .await
            .unwrap()
    }

    #[cfg(test)]
    pub(crate) async fn save_in(&self, app: &AppUseCase, record: &ArchiveExtractionRecord) {
        app.services
            .workflow
            .archive_extractions
            .save(record)
            .await
            .unwrap();
    }

    /// A record of this download in `state` for `workspace`.
    #[cfg(test)]
    pub(crate) fn test_record(
        &self,
        workspace: &Path,
        state: ArchiveExtractionState,
        now: DateTime<Utc>,
    ) -> ArchiveExtractionRecord {
        ArchiveExtractionRecord {
            client_id: self.client_id.clone(),
            client_item_id: self.client_item_id.clone(),
            state,
            workspace_path: crate::stored_paths::path_to_stored_string(workspace),
            archive_digest: None,
            attempts: 1,
            last_reason_code: None,
            output_count: 0,
            output_bytes: 0,
            next_attempt_at: None,
            settled_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// None when the client gave no item id, so the download has no identity
    /// to key a workspace on.
    pub(crate) fn new(client_id: Option<&str>, client_item_id: &str) -> Option<Self> {
        let client_item_id = client_item_id.trim();
        (!client_item_id.is_empty()).then(|| Self {
            client_id: client_id.unwrap_or("").trim().to_ascii_lowercase(),
            client_item_id: client_item_id.to_string(),
        })
    }

    pub(crate) fn for_completed(completed: &crate::CompletedDownload) -> Option<Self> {
        Self::new(
            Some(completed.client_id.as_str()),
            &completed.download_client_item_id,
        )
    }

    pub(crate) fn for_source(source: &ClientJobLocator) -> Option<Self> {
        Self::new(source.client_id.as_deref(), &source.item_id)
    }

    pub(crate) fn for_tracked(td: &crate::tracked_downloads::TrackedDownload) -> Option<Self> {
        Self::new(
            Some(td.client_id.as_str()),
            &td.client_item.download_client_item_id,
        )
    }

    fn of_record(record: &ArchiveExtractionRecord) -> Self {
        Self {
            client_id: record.client_id.clone(),
            client_item_id: record.client_item_id.clone(),
        }
    }

    fn pair(&self) -> (String, String) {
        (self.client_id.clone(), self.client_item_id.clone())
    }

    /// 64 bits of an unkeyed hash, so the name is the same on every run and
    /// short, and downloads sharing one library root do not collide in
    /// practice; both parts are length-prefixed.
    fn hex(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"scryer-archive-extraction-v1");
        for part in [&self.client_id, &self.client_item_id] {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
        hasher.finalize().to_hex()[..16].to_string()
    }

    pub(crate) fn workspace_name(&self) -> String {
        archive_extractor::download_workspace_name(&self.hex())
    }

    /// The owner its workspace's marker records.
    pub(crate) fn owner_id(&self) -> String {
        format!("download:{}", self.hex())
    }

    pub(crate) fn destination(&self, library_root: &Path) -> ArchiveExtractionDestination {
        ArchiveExtractionDestination::for_download(
            library_root,
            self.workspace_name(),
            self.owner_id(),
        )
    }

    /// Whether `root` is the path this download's workspace has.
    #[cfg(test)]
    pub(crate) fn names_workspace(&self, root: &Path) -> bool {
        root.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == self.workspace_name())
            && root
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == ARCHIVE_EXTRACTION_DIR_NAME)
    }
}

/// Whether `root` is a download's workspace, whichever download's: such a
/// workspace goes only when its download settles.
pub(crate) fn is_download_workspace_path(root: &Path) -> bool {
    root.parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == ARCHIVE_EXTRACTION_DIR_NAME)
}

/// Downloads whose extraction runs in this process. A record left
/// `Extracting` by a download not listed here was interrupted.
#[derive(Clone, Default)]
pub(crate) struct ArchiveExtractionsInFlight(Arc<std::sync::Mutex<HashSet<DownloadExtractionKey>>>);

impl ArchiveExtractionsInFlight {
    fn claim(&self, key: &DownloadExtractionKey) -> Option<InFlightClaim> {
        let mut keys = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        keys.insert(key.clone()).then(|| InFlightClaim {
            set: self.clone(),
            key: key.clone(),
        })
    }

    fn contains(&self, key: &DownloadExtractionKey) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(key)
    }
}

struct InFlightClaim {
    set: ArchiveExtractionsInFlight,
    key: DownloadExtractionKey,
}

impl Drop for InFlightClaim {
    fn drop(&mut self) {
        self.set
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
    }
}

pub(crate) fn workspace_occupied_error() -> AppError {
    AppError::validation_refused(
        reasons::WORKSPACE_OCCUPIED,
        "the download's extraction folder is held by something Scryer did not create for it; \
         it is left in place",
    )
}

fn waiting_error() -> AppError {
    AppError::validation_refused(
        reasons::WAITING,
        "an earlier extraction of the download's archives failed; the next attempt is not due yet",
    )
}

pub(crate) fn blocked_error() -> AppError {
    AppError::validation_refused(
        reasons::BLOCKED,
        "the download's archives failed to extract on every attempt; retry the import to try again",
    )
}

fn in_progress_error() -> AppError {
    AppError::validation_refused(
        reasons::IN_PROGRESS,
        "the download's archives are being extracted; the import waits for them",
    )
}

/// The completed extraction an import pass reads, without extracting. `None`
/// when the download has none yet. A download whose extraction is running
/// waits for it.
pub(crate) async fn completed_download_extraction(
    app: &AppUseCase,
    key: &DownloadExtractionKey,
) -> AppResult<Option<PathBuf>> {
    if app
        .runtime
        .imports
        .archive_extractions_in_flight
        .contains(key)
    {
        return Err(in_progress_error());
    }
    let Some(record) = app
        .services
        .workflow
        .archive_extractions
        .get(&key.client_id, &key.client_item_id)
        .await?
    else {
        return Ok(None);
    };
    if record.state != ArchiveExtractionState::Extracted {
        return Ok(None);
    }
    let root = crate::stored_paths::stored_path_to_path_buf(&record.workspace_path);
    Ok(
        (archive_extractor::download_workspace_presence(&root, &key.owner_id()).await
            == DownloadWorkspacePresence::Owned)
            .then_some(root),
    )
}

/// Extract the download's archives into its workspace once.
///
/// Returns the workspace when the download's archives are extracted, now or
/// by an earlier pass, and `None` when the download holds no archive to
/// extract. Nothing is written for a download without archives or without an
/// extractor for them. A failed or interrupted extraction's workspace is
/// removed; a path held by anything this download does not own is left as it
/// is and refused.
pub(crate) async fn extract_download_archives(
    app: &AppUseCase,
    key: &DownloadExtractionKey,
    dir: &Path,
    is_sample: fn(&Path) -> bool,
    destination: ArchiveExtractionDestination,
    passwords: &crate::import::archive_passwords::ArchivePasswordCandidates,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<Option<PathBuf>> {
    let Some(_claim) = app.runtime.imports.archive_extractions_in_flight.claim(key) else {
        return Err(in_progress_error());
    };
    let Some(root) = destination.workspace_root() else {
        // Without archive support no destination names a workspace.
        return Ok(None);
    };
    let owner_id = key.owner_id();
    let repository = &app.services.workflow.archive_extractions;
    let now = app.runtime.environment.now();
    let mut previous = repository.get(&key.client_id, &key.client_item_id).await?;

    if let Some(record) = previous.as_mut() {
        let recorded = crate::stored_paths::stored_path_to_path_buf(&record.workspace_path);
        match record.state {
            ArchiveExtractionState::Extracted => {
                // Gone from disk, or taken by something else: planned again
                // below, where a taken path is refused.
                if archive_extractor::download_workspace_presence(&recorded, &owner_id).await
                    == DownloadWorkspacePresence::Owned
                {
                    return Ok(Some(recorded));
                }
            }
            ArchiveExtractionState::Extracting => {
                // Nothing in this process runs it, so the process that did
                // stopped. That attempt counts, and its partial output goes.
                archive_extractor::remove_download_workspace(&recorded, &owner_id).await;
                record.state = if record.attempts >= MAX_ATTEMPTS {
                    ArchiveExtractionState::Blocked
                } else {
                    ArchiveExtractionState::Failed
                };
                record.last_reason_code = Some(reasons::INTERRUPTED.to_string());
                record.updated_at = now;
                repository.save(record).await?;
                if record.state == ArchiveExtractionState::Blocked {
                    return Err(blocked_error());
                }
            }
            ArchiveExtractionState::Blocked => return Err(blocked_error()),
            ArchiveExtractionState::Failed
                if record.next_attempt_at.is_some_and(|due| due > now) =>
            {
                return Err(waiting_error());
            }
            _ => {}
        }
    }

    let Some(plan) =
        archive_extractor::plan_download_extraction(dir, is_sample, archive_provider.as_ref())
            .await?
    else {
        return Ok(None);
    };
    let Some(provider) = archive_provider else {
        return Err(AppError::archive_extraction_plugin_required(None));
    };

    let occupied = match archive_extractor::download_workspace_presence(&root, &owner_id).await {
        DownloadWorkspacePresence::Missing => false,
        // A recorded failure's leftover is this download's partial output.
        DownloadWorkspacePresence::Owned if previous.is_some() => {
            !archive_extractor::remove_download_workspace(&root, &owner_id).await
        }
        // An owned workspace with no record is not known to be partial, and
        // anything else is not Scryer's: both are left in place.
        DownloadWorkspacePresence::Owned | DownloadWorkspacePresence::Foreign => true,
    };
    // A completed extraction whose output went missing starts a fresh count.
    let attempts = previous
        .as_ref()
        .filter(|record| record.state != ArchiveExtractionState::Extracted)
        .map_or(0, |record| record.attempts)
        .saturating_add(1);
    let base = ArchiveExtractionRecord {
        client_id: key.client_id.clone(),
        client_item_id: key.client_item_id.clone(),
        state: ArchiveExtractionState::Extracting,
        workspace_path: crate::stored_paths::path_to_stored_string(&root),
        archive_digest: Some(plan.digest().to_string()),
        attempts,
        last_reason_code: previous
            .as_ref()
            .and_then(|record| record.last_reason_code.clone()),
        output_count: 0,
        output_bytes: 0,
        next_attempt_at: None,
        settled_at: previous.as_ref().and_then(|record| record.settled_at),
        created_at: previous.as_ref().map_or(now, |record| record.created_at),
        updated_at: now,
    };
    if occupied {
        // Only an operator can clear the path, so a taken workspace counts
        // as an attempt and ends blocked rather than being retried forever.
        let error = workspace_occupied_error();
        let saved = repository
            .save(&failed_attempt(base, reasons::WORKSPACE_OCCUPIED, now))
            .await;
        if saved.is_err() {
            tracing::warn!("archive extraction refusal could not be recorded");
        }
        return Err(error);
    }

    let extracting = base;
    repository.save(&extracting).await?;

    match archive_extractor::extract_planned_download(plan, &destination, passwords, &provider)
        .await
    {
        Ok(output) => {
            repository
                .save(&ArchiveExtractionRecord {
                    state: ArchiveExtractionState::Extracted,
                    last_reason_code: None,
                    output_count: output.files,
                    output_bytes: output.bytes,
                    updated_at: app.runtime.environment.now(),
                    ..extracting
                })
                .await?;
            Ok(Some(root))
        }
        // Not an attempt of this download: the pass was cancelled, or the
        // plugin went away mid-flight. The record is as it was.
        Err(error @ (AppError::Canceled(_) | AppError::ArchiveExtractionPluginRequired { .. })) => {
            let restored = match &previous {
                Some(record) => repository.save(record).await,
                None => repository.delete(&key.client_id, &key.client_item_id).await,
            };
            if restored.is_err() {
                tracing::warn!(
                    "archive extraction record could not be restored after a cancelled attempt"
                );
            }
            Err(error)
        }
        Err(error) => {
            let finished = app.runtime.environment.now();
            let failure_reason =
                error
                    .reason()
                    .unwrap_or(if archive_extractor::is_timeout_error(&error) {
                        reasons::TIMED_OUT
                    } else {
                        reasons::FAILED
                    });
            let record = if archive_extractor::is_password_required_error(&error) {
                // Waiting for a password is not a failed attempt: it costs
                // none and sets no backoff. The operator's password retry
                // runs it again.
                ArchiveExtractionRecord {
                    state: ArchiveExtractionState::NeedsPassword,
                    attempts: attempts - 1,
                    last_reason_code: Some(reasons::PASSWORD_REQUIRED.to_string()),
                    updated_at: finished,
                    ..extracting
                }
            } else {
                failed_attempt(extracting, failure_reason, finished)
            };
            let blocked = record.state == ArchiveExtractionState::Blocked;
            if repository.save(&record).await.is_err() {
                tracing::warn!("archive extraction failure could not be recorded");
            }
            if blocked {
                tracing::warn!(
                    reason = record.last_reason_code.as_deref().unwrap_or_default(),
                    attempts = record.attempts,
                    "archive extraction blocked after its last allowed attempt"
                );
                return Err(blocked_error());
            }
            if record.state == ArchiveExtractionState::Failed {
                // Attempts remain, so the import retries after the backoff
                // instead of stopping at the first failure.
                return Err(AppError::validation_refused(
                    failure_reason,
                    error.to_string(),
                ));
            }
            Err(error)
        }
    }
}

/// A failed attempt: retried after its backoff, or blocked once it was the
/// last allowed.
fn failed_attempt(
    record: ArchiveExtractionRecord,
    reason: &str,
    now: DateTime<Utc>,
) -> ArchiveExtractionRecord {
    let blocked = record.attempts >= MAX_ATTEMPTS;
    ArchiveExtractionRecord {
        state: if blocked {
            ArchiveExtractionState::Blocked
        } else {
            ArchiveExtractionState::Failed
        },
        last_reason_code: Some(reason.to_string()),
        next_attempt_at: (!blocked).then(|| now + attempt_backoff(record.attempts)),
        updated_at: now,
        ..record
    }
}

/// An operator retry: a blocked, failed or password-waiting extraction is
/// pending again with its attempts and backoff cleared. Nothing is written
/// for a download with no such record.
pub(crate) async fn reset_download_extraction(app: &AppUseCase, key: &DownloadExtractionKey) {
    let repository = &app.services.workflow.archive_extractions;
    let Ok(Some(record)) = repository.get(&key.client_id, &key.client_item_id).await else {
        return;
    };
    if !matches!(
        record.state,
        ArchiveExtractionState::Blocked
            | ArchiveExtractionState::Failed
            | ArchiveExtractionState::NeedsPassword
    ) {
        return;
    }
    let reset = ArchiveExtractionRecord {
        state: ArchiveExtractionState::Pending,
        attempts: 0,
        last_reason_code: None,
        next_attempt_at: None,
        updated_at: app.runtime.environment.now(),
        ..record
    };
    if repository.save(&reset).await.is_err() {
        tracing::warn!("archive extraction record could not be reset for the retry");
    }
}

/// The download has settled. Imported: its workspace and record go now.
/// Ended any other way: it is marked settled, and housekeeping removes the
/// workspace [`SETTLED_WORKSPACE_RETENTION`] later. Nothing is written for a
/// download without a record or one already settled.
pub(crate) async fn settle_download_extraction(
    app: &AppUseCase,
    key: &DownloadExtractionKey,
    imported: bool,
) {
    let repository = &app.services.workflow.archive_extractions;
    let record = match repository.get(&key.client_id, &key.client_item_id).await {
        Ok(Some(record)) => record,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(error = %error, "archive extraction record could not be read at settlement");
            return;
        }
    };
    if imported {
        let root = crate::stored_paths::stored_path_to_path_buf(&record.workspace_path);
        if remove_settled_workspace(&root, key).await == SettledRemoval::Done {
            forget_download_extraction(app, key).await;
            return;
        }
    }
    if record.settled_at.is_none()
        && repository
            .settle(&[key.pair()], app.runtime.environment.now())
            .await
            .is_err()
    {
        tracing::warn!("archive extraction settlement could not be recorded");
    }
}

/// Downloads that left their client: each is marked settled in one batch.
pub(crate) async fn settle_download_extractions(app: &AppUseCase, keys: &[DownloadExtractionKey]) {
    if keys.is_empty() {
        return;
    }
    let pairs: Vec<_> = keys.iter().map(DownloadExtractionKey::pair).collect();
    if app
        .services
        .workflow
        .archive_extractions
        .settle(&pairs, app.runtime.environment.now())
        .await
        .is_err()
    {
        tracing::warn!("archive extraction settlement could not be recorded");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettledRemoval {
    /// The workspace is gone, or what holds its path is not Scryer's and
    /// stays; either way the record is done with.
    Done,
    /// The workspace is still there; try again later.
    Retry,
}

async fn remove_settled_workspace(root: &Path, key: &DownloadExtractionKey) -> SettledRemoval {
    let owner_id = key.owner_id();
    if archive_extractor::remove_download_workspace(root, &owner_id).await {
        return SettledRemoval::Done;
    }
    match archive_extractor::download_workspace_presence(root, &owner_id).await {
        DownloadWorkspacePresence::Missing => SettledRemoval::Done,
        DownloadWorkspacePresence::Foreign if archive_extractor::PROVES_WORKSPACE_OWNERSHIP => {
            SettledRemoval::Done
        }
        // Owned but not removed, or a build that cannot tell whose it is.
        _ => SettledRemoval::Retry,
    }
}

/// What one settled-workspace sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SettledSweep {
    pub removed: usize,
    pub deferred: usize,
}

/// Remove the workspaces of downloads settled more than
/// [`SETTLED_WORKSPACE_RETENTION`] ago, oldest first and a bounded number per
/// pass. A workspace that cannot be removed now is tried again after another
/// retention period. A pass with nothing due only reads.
pub(crate) async fn sweep_settled_download_extractions(
    app: &AppUseCase,
) -> AppResult<SettledSweep> {
    let repository = &app.services.workflow.archive_extractions;
    let now = app.runtime.environment.now();
    let due = repository
        .list_settled_before(now - SETTLED_WORKSPACE_RETENTION, SETTLED_SWEEP_LIMIT)
        .await?;
    let mut done = Vec::new();
    let mut retry = Vec::new();
    for record in &due {
        let key = DownloadExtractionKey::of_record(record);
        let root = crate::stored_paths::stored_path_to_path_buf(&record.workspace_path);
        match remove_settled_workspace(&root, &key).await {
            SettledRemoval::Done => done.push(key.pair()),
            SettledRemoval::Retry => retry.push(key.pair()),
        }
    }
    repository.delete_many(&done).await?;
    repository.defer_settled(&retry, now).await?;
    Ok(SettledSweep {
        removed: done.len(),
        deferred: retry.len(),
    })
}

/// Delete the record of a download whose workspace was removed.
pub(crate) async fn forget_download_extraction(app: &AppUseCase, key: &DownloadExtractionKey) {
    if app
        .services
        .workflow
        .archive_extractions
        .delete(&key.client_id, &key.client_item_id)
        .await
        .is_err()
    {
        tracing::warn!(
            "archive extraction record could not be deleted after its workspace was removed"
        );
    }
}

/// The download's recorded workspace, when it is an owned workspace of that
/// download.
pub(crate) async fn owned_download_workspace(
    app: &AppUseCase,
    key: &DownloadExtractionKey,
) -> AppResult<Option<PathBuf>> {
    let Some(record) = app
        .services
        .workflow
        .archive_extractions
        .get(&key.client_id, &key.client_item_id)
        .await?
    else {
        return Ok(None);
    };
    let root = crate::stored_paths::stored_path_to_path_buf(&record.workspace_path);
    Ok(
        (archive_extractor::download_workspace_presence(&root, &key.owner_id()).await
            == DownloadWorkspacePresence::Owned)
            .then_some(root),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_round_trip_through_their_stored_names() {
        for state in [
            ArchiveExtractionState::Pending,
            ArchiveExtractionState::Extracting,
            ArchiveExtractionState::Extracted,
            ArchiveExtractionState::NeedsPassword,
            ArchiveExtractionState::Failed,
            ArchiveExtractionState::Blocked,
            ArchiveExtractionState::Removed,
        ] {
            assert_eq!(ArchiveExtractionState::parse(state.as_str()), Some(state));
        }
        assert_eq!(ArchiveExtractionState::parse("unknown"), None);
    }

    #[test]
    fn a_download_key_names_one_stable_workspace() {
        let key = DownloadExtractionKey::new(Some(" Client-A "), " item-1 ").unwrap();
        let same = DownloadExtractionKey::new(Some("client-a"), "item-1").unwrap();
        let other_item = DownloadExtractionKey::new(Some("client-a"), "item-2").unwrap();
        let other_client = DownloadExtractionKey::new(Some("client-b"), "item-1").unwrap();
        // The length prefix keeps the parts from running together.
        let shifted = DownloadExtractionKey::new(Some("client-ai"), "tem-1").unwrap();

        assert_eq!(key.workspace_name(), same.workspace_name());
        for different in [&other_item, &other_client, &shifted] {
            assert_ne!(key.workspace_name(), different.workspace_name());
        }
        let name = key.workspace_name();
        let hex = name.as_str();
        assert_eq!(hex.len(), 16);
        assert!(
            hex.bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        );
        assert_eq!(key.owner_id(), format!("download:{hex}"));
        assert!(DownloadExtractionKey::new(Some("client-a"), "  ").is_none());
    }

    #[test]
    fn a_key_names_only_its_own_path_in_the_extraction_folder() {
        let key = DownloadExtractionKey::new(Some("client-a"), "item-1").unwrap();
        let root = Path::new("/library");
        let staged = root
            .join(ARCHIVE_EXTRACTION_DIR_NAME)
            .join(key.workspace_name());
        assert!(key.names_workspace(&staged));
        assert!(!key.names_workspace(&root.join("Title").join(key.workspace_name())));
        let other = DownloadExtractionKey::new(Some("client-a"), "item-2").unwrap();
        assert!(!other.names_workspace(&staged));
    }

    #[test]
    fn one_claim_per_download_until_it_is_dropped() {
        let in_flight = ArchiveExtractionsInFlight::default();
        let key = DownloadExtractionKey::new(Some("client-a"), "item-1").unwrap();
        let claim = in_flight.claim(&key).expect("first claim");
        assert!(in_flight.contains(&key));
        assert!(in_flight.claim(&key).is_none());
        drop(claim);
        assert!(!in_flight.contains(&key));
        assert!(in_flight.claim(&key).is_some());
    }
}
