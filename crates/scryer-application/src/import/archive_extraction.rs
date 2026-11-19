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
//! after the download ends any other way. A download the tracker stopped
//! following, or whose client is no longer configured, counts as ended.
//!
//! Housekeeping also removes an owned workspace in a library root's
//! extraction folder that no record names, a day after it was made, and
//! leaves and counts anything there it cannot prove it made.

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

    /// Whether a download in this state has extraction output on disk: a
    /// failed, blocked, password-waiting or pending one's was removed.
    pub fn has_output_on_disk(self) -> bool {
        matches!(self, Self::Extracted | Self::Extracting)
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

/// An unsettled download's record, as the pass that settles downloads no
/// longer tracked reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsettledArchiveExtraction {
    pub client_id: String,
    pub client_item_id: String,
    pub updated_at: DateTime<Utc>,
}

/// The scratch the records account for.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveScratchSummary {
    /// Downloads with extraction output on disk: extracted, or extracting.
    pub workspaces: u64,
    /// Bytes their completed extractions wrote.
    pub bytes: u64,
    /// Downloads whose extraction is blocked until an operator retries.
    pub blocked: u64,
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
    /// Unsettled downloads last changed at or before `changed_before` whose
    /// key sorts after `after` (from the start when `None`), in key order, at
    /// most `limit`.
    async fn list_unsettled_after(
        &self,
        after: Option<(String, String)>,
        changed_before: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<UnsettledArchiveExtraction>>;
    /// The workspace names, of those given, that end some record's
    /// workspace path.
    async fn recorded_workspace_names(&self, names: &[String]) -> AppResult<HashSet<String>>;
    async fn scratch_summary(&self) -> AppResult<ArchiveScratchSummary>;
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

    async fn list_unsettled_after(
        &self,
        after: Option<(String, String)>,
        changed_before: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<UnsettledArchiveExtraction>> {
        let rows = self.rows.lock().await;
        let mut unsettled: Vec<_> = rows
            .iter()
            .filter(|(key, row)| {
                row.settled_at.is_none()
                    && row.updated_at <= changed_before
                    && after.as_ref().is_none_or(|after| *key > after)
            })
            .map(|(key, row)| UnsettledArchiveExtraction {
                client_id: key.0.clone(),
                client_item_id: key.1.clone(),
                updated_at: row.updated_at,
            })
            .collect();
        unsettled.sort_by(|a, b| {
            (&a.client_id, &a.client_item_id).cmp(&(&b.client_id, &b.client_item_id))
        });
        unsettled.truncate(limit);
        Ok(unsettled)
    }

    async fn recorded_workspace_names(&self, names: &[String]) -> AppResult<HashSet<String>> {
        let rows = self.rows.lock().await;
        Ok(names
            .iter()
            .filter(|name| {
                rows.values()
                    .any(|row| row.workspace_path.ends_with(name.as_str()))
            })
            .cloned()
            .collect())
    }

    async fn scratch_summary(&self) -> AppResult<ArchiveScratchSummary> {
        let rows = self.rows.lock().await;
        let on_disk = || rows.values().filter(|row| row.state.has_output_on_disk());
        Ok(ArchiveScratchSummary {
            workspaces: on_disk().count() as u64,
            bytes: on_disk().map(|row| row.output_bytes).sum(),
            blocked: rows
                .values()
                .filter(|row| row.state == ArchiveExtractionState::Blocked)
                .count() as u64,
        })
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

    /// None when the download names no configured client or the client gave
    /// no item id: without both it has no identity the tracker settles it
    /// by, and two clients' item ids could meet. Such a download extracts per
    /// pass instead.
    pub(crate) fn new(client_id: Option<&str>, client_item_id: &str) -> Option<Self> {
        let client_id = client_id.unwrap_or("").trim();
        let client_item_id = client_item_id.trim();
        (!client_id.is_empty() && !client_item_id.is_empty()).then(|| Self {
            client_id: client_id.to_ascii_lowercase(),
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

    pub(crate) fn pair(&self) -> (String, String) {
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

/// Downloads whose workspace is in use in this process: an extraction or a
/// removal holds a download exclusively, and import passes and manual imports
/// reading its workspace hold it as readers. A record left `Extracting` by a
/// download no extraction here holds was interrupted.
#[derive(Clone, Default)]
pub(crate) struct ArchiveExtractionsInFlight(
    Arc<std::sync::Mutex<std::collections::HashMap<DownloadExtractionKey, WorkspaceHolders>>>,
);

#[derive(Debug, Default, Clone, Copy)]
struct WorkspaceHolders {
    exclusive: bool,
    readers: usize,
}

type HoldersGuard<'a> =
    std::sync::MutexGuard<'a, std::collections::HashMap<DownloadExtractionKey, WorkspaceHolders>>;

impl ArchiveExtractionsInFlight {
    fn holders(&self) -> HoldersGuard<'_> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Hold the download for an extraction. Readers do not stop one: they
    /// read only a completed extraction, which an extraction reuses.
    pub(crate) fn claim(&self, key: &DownloadExtractionKey) -> Option<InFlightClaim> {
        let mut holders = self.holders();
        let entry = holders.entry(key.clone()).or_default();
        if entry.exclusive {
            return None;
        }
        entry.exclusive = true;
        Some(InFlightClaim {
            set: self.clone(),
            key: key.clone(),
        })
    }

    /// Hold the download to remove its workspace: refused while anything
    /// extracts or reads it.
    fn claim_for_removal(&self, key: &DownloadExtractionKey) -> Option<InFlightClaim> {
        let mut holders = self.holders();
        if holders.contains_key(key) {
            return None;
        }
        holders.insert(
            key.clone(),
            WorkspaceHolders {
                exclusive: true,
                readers: 0,
            },
        );
        Some(InFlightClaim {
            set: self.clone(),
            key: key.clone(),
        })
    }

    /// Read the download's workspace until the reader is dropped; its
    /// removal waits for a later sweep meanwhile.
    pub(crate) fn read(&self, key: &DownloadExtractionKey) -> WorkspaceReader {
        self.holders().entry(key.clone()).or_default().readers += 1;
        WorkspaceReader {
            set: self.clone(),
            key: key.clone(),
        }
    }

    /// Whether an extraction or a removal holds the download.
    fn contains(&self, key: &DownloadExtractionKey) -> bool {
        self.holders()
            .get(key)
            .is_some_and(|holders| holders.exclusive)
    }

    /// Whether anything holds the download whose workspace has this name.
    fn holds_workspace(&self, name: &str) -> bool {
        self.holders()
            .keys()
            .any(|key| key.workspace_name() == name)
    }

    fn release(&self, key: &DownloadExtractionKey, exclusive: bool) {
        let mut holders = self.holders();
        if let Some(entry) = holders.get_mut(key) {
            if exclusive {
                entry.exclusive = false;
            } else {
                entry.readers = entry.readers.saturating_sub(1);
            }
            if !entry.exclusive && entry.readers == 0 {
                holders.remove(key);
            }
        }
    }
}

/// A download's workspace, read by an import pass or a manual import.
pub(crate) struct WorkspaceReader {
    set: ArchiveExtractionsInFlight,
    key: DownloadExtractionKey,
}

impl Drop for WorkspaceReader {
    fn drop(&mut self) {
        self.set.release(&self.key, false);
    }
}

/// Hold `completed`'s workspace as a reader for as long as the result lives;
/// `None` for a download without a key.
pub(crate) fn read_download_workspace(
    app: &AppUseCase,
    completed: &crate::CompletedDownload,
) -> Option<WorkspaceReader> {
    DownloadExtractionKey::for_completed(completed)
        .map(|key| app.runtime.imports.archive_extractions_in_flight.read(&key))
}

/// Process-local state of workspace upkeep.
#[derive(Clone, Default)]
pub(crate) struct ArchiveWorkspaceUpkeep {
    /// What the last orphan sweep found, for the health check.
    last_orphan_sweep: Arc<std::sync::Mutex<Option<OrphanSweep>>>,
    /// Where the pass settling untracked downloads resumes.
    untracked_cursor: Arc<std::sync::Mutex<Option<(String, String)>>>,
    /// Removals of imported downloads' workspaces still running.
    removals: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// For each download whose last extraction here ended waiting for a
    /// password, a fingerprint of the candidates it tried. Kept in memory
    /// only, so no password is persisted in any form; after a restart the
    /// first pass with candidates tries them once.
    passwords_tried:
        Arc<std::sync::Mutex<std::collections::HashMap<DownloadExtractionKey, blake3::Hash>>>,
}

impl ArchiveWorkspaceUpkeep {
    pub(crate) fn last_orphan_sweep(&self) -> Option<OrphanSweep> {
        *self
            .last_orphan_sweep
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn passwords_tried(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<DownloadExtractionKey, blake3::Hash>>
    {
        self.passwords_tried
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn track_removal(&self, removal: tokio::task::JoinHandle<()>) {
        let mut removals = self
            .removals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        removals.retain(|removal| !removal.is_finished());
        removals.push(removal);
    }

    /// Wait for every removal started so far.
    #[cfg(test)]
    pub(crate) async fn drain_removals(&self) {
        let removals = std::mem::take(
            &mut *self
                .removals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for removal in removals {
            removal.await.expect("workspace removal task");
        }
    }
}

pub(crate) struct InFlightClaim {
    set: ArchiveExtractionsInFlight,
    key: DownloadExtractionKey,
}

impl Drop for InFlightClaim {
    fn drop(&mut self) {
        self.set.release(&self.key, true);
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

fn password_required_error() -> AppError {
    AppError::ArchivePasswordRequired {
        message: "the download's archives need a password none of its candidates opened; \
                  retry the import with the password"
            .to_string(),
    }
}

/// The candidates a pass offers, in order, as one value to compare with the
/// ones an earlier attempt tried.
fn password_fingerprint(
    passwords: &crate::import::archive_passwords::ArchivePasswordCandidates,
) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    for candidate in passwords.iter() {
        for part in [candidate.source().as_str(), candidate.value()] {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
    }
    hasher.finalize()
}

/// Mark a download live again: a settled one is unsettled, so its workspace
/// is not swept while it is used. Writes only when it was settled.
async fn unsettle(app: &AppUseCase, record: &ArchiveExtractionRecord) -> AppResult<()> {
    if record.settled_at.is_none() {
        return Ok(());
    }
    app.services
        .workflow
        .archive_extractions
        .save(&ArchiveExtractionRecord {
            settled_at: None,
            updated_at: app.runtime.environment.now(),
            ..record.clone()
        })
        .await
}

fn in_progress_error() -> AppError {
    AppError::validation_refused(
        reasons::IN_PROGRESS,
        "the download's archives are being extracted; the import waits for them",
    )
}

/// The completed extraction an import pass reads, without extracting. `None`
/// when the download has none yet, or when the archive sets in `dir` are not
/// the ones it was made from; [`extract_download_archives`] then replaces it.
/// A download whose extraction is running waits for it. A settled download
/// handed to a pass is live again.
pub(crate) async fn completed_download_extraction(
    app: &AppUseCase,
    key: &DownloadExtractionKey,
    dir: &Path,
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
    if archive_extractor::download_workspace_presence(&root, &key.owner_id()).await
        != DownloadWorkspacePresence::Owned
    {
        return Ok(None);
    }
    let current = archive_extractor::download_archive_digest(dir).await?;
    if current.is_none() || current != record.archive_digest {
        return Ok(None);
    }
    unsettle(app, &record).await?;
    Ok(Some(root))
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
                    let current = archive_extractor::download_archive_digest(dir).await?;
                    if current.is_some() && current == record.archive_digest {
                        unsettle(app, record).await?;
                        return Ok(Some(recorded));
                    }
                    // Made from other archives than the download holds now:
                    // that output goes, and the download starts afresh.
                    if archive_extractor::remove_download_workspace(&recorded, &owner_id).await {
                        repository
                            .delete(&key.client_id, &key.client_item_id)
                            .await?;
                        previous = None;
                    }
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
            // Still waiting for a password: tried again only with candidates
            // the last attempt here did not try. An operator retry resets the
            // record to pending first, so it always runs.
            ArchiveExtractionState::NeedsPassword => {
                let tried = app
                    .runtime
                    .imports
                    .archive_workspace_upkeep
                    .passwords_tried()
                    .get(key)
                    .copied();
                if passwords.is_empty() || tried == Some(password_fingerprint(passwords)) {
                    return Err(password_required_error());
                }
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
        // This download's own scratch, which no completed extraction still
        // reads: a failure's leftover, or one whose record is gone (a backup
        // restore). It is replaced.
        DownloadWorkspacePresence::Owned => {
            !archive_extractor::remove_download_workspace(&root, &owner_id).await
        }
        // Not Scryer's: left in place.
        DownloadWorkspacePresence::Foreign => true,
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
        // An attempt makes the download live again.
        settled_at: None,
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

    let outcome =
        archive_extractor::extract_planned_download(plan, &destination, passwords, &provider).await;
    {
        let mut tried = app
            .runtime
            .imports
            .archive_workspace_upkeep
            .passwords_tried();
        match &outcome {
            Err(error) if archive_extractor::is_password_required_error(error) => {
                tried.insert(key.clone(), password_fingerprint(passwords));
            }
            Err(AppError::Canceled(_) | AppError::ArchiveExtractionPluginRequired { .. }) => {}
            _ => {
                tried.remove(key);
            }
        }
    }
    match outcome {
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
            if record.state == ArchiveExtractionState::Failed && error.reason().is_none() {
                // Attempts remain, so the import retries after the backoff
                // instead of stopping at the first failure. An error that
                // already carries a code is returned as it is.
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
/// pending again with its attempts and backoff cleared, and a settled
/// download is live again. Nothing is written for a download with no record,
/// or with one neither retryable nor settled.
pub(crate) async fn reset_download_extraction(app: &AppUseCase, key: &DownloadExtractionKey) {
    let repository = &app.services.workflow.archive_extractions;
    let Ok(Some(record)) = repository.get(&key.client_id, &key.client_item_id).await else {
        return;
    };
    let retryable = matches!(
        record.state,
        ArchiveExtractionState::Blocked
            | ArchiveExtractionState::Failed
            | ArchiveExtractionState::NeedsPassword
    );
    if !retryable && record.settled_at.is_none() {
        return;
    }
    let reset = if retryable {
        ArchiveExtractionRecord {
            state: ArchiveExtractionState::Pending,
            attempts: 0,
            last_reason_code: None,
            next_attempt_at: None,
            settled_at: None,
            updated_at: app.runtime.environment.now(),
            ..record
        }
    } else {
        ArchiveExtractionRecord {
            settled_at: None,
            updated_at: app.runtime.environment.now(),
            ..record
        }
    };
    if repository.save(&reset).await.is_err() {
        tracing::warn!("archive extraction record could not be reset for the retry");
    }
}

/// The download has settled. Imported: it is marked due for removal at once
/// and its workspace is removed off the caller's path; should that removal
/// not finish, housekeeping's sweep removes it. Ended any other way: it is
/// marked settled, and housekeeping removes the workspace
/// [`SETTLED_WORKSPACE_RETENTION`] later. Nothing is written for a download
/// without a record, or for one already settled that is not imported.
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
    let now = app.runtime.environment.now();
    if imported {
        let due = now - SETTLED_WORKSPACE_RETENTION;
        if record.settled_at.is_none_or(|at| at > due)
            && repository.defer_settled(&[key.pair()], due).await.is_err()
        {
            tracing::warn!("archive extraction settlement could not be recorded");
            return;
        }
        let removal = tokio::spawn(remove_imported_workspace(app.clone(), key.clone()));
        app.runtime
            .imports
            .archive_workspace_upkeep
            .track_removal(removal);
        return;
    }
    if record.settled_at.is_none() && repository.settle(&[key.pair()], now).await.is_err() {
        tracing::warn!("archive extraction settlement could not be recorded");
    }
}

/// Remove an imported download's workspace and forget its record, unless an
/// extraction of it runs now or a new one has unsettled it. A workspace that
/// stays is left to the sweep, where the record is already due.
async fn remove_imported_workspace(app: AppUseCase, key: DownloadExtractionKey) {
    // A pass reading it or an extraction keeps it; the record stays due.
    let Some(_claim) = app
        .runtime
        .imports
        .archive_extractions_in_flight
        .claim_for_removal(&key)
    else {
        return;
    };
    let Ok(Some(record)) = app
        .services
        .workflow
        .archive_extractions
        .get(&key.client_id, &key.client_item_id)
        .await
    else {
        return;
    };
    if record.settled_at.is_none() {
        return;
    }
    let root = crate::stored_paths::stored_path_to_path_buf(&record.workspace_path);
    if remove_settled_workspace(&root, &key).await != SettledRemoval::Retry {
        forget_download_extraction(&app, &key).await;
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
    /// The workspace is gone: removed now, or already missing.
    Gone,
    /// What holds the path is not Scryer's and stays where it is.
    LeftForeign,
    /// The workspace is still there; try again later.
    Retry,
}

async fn remove_settled_workspace(root: &Path, key: &DownloadExtractionKey) -> SettledRemoval {
    let owner_id = key.owner_id();
    if archive_extractor::remove_download_workspace(root, &owner_id).await {
        return SettledRemoval::Gone;
    }
    match archive_extractor::download_workspace_presence(root, &owner_id).await {
        DownloadWorkspacePresence::Missing => SettledRemoval::Gone,
        DownloadWorkspacePresence::Foreign if archive_extractor::PROVES_WORKSPACE_OWNERSHIP => {
            SettledRemoval::LeftForeign
        }
        // Owned but not removed, or a build that cannot tell whose it is.
        _ => SettledRemoval::Retry,
    }
}

/// What one settled-workspace sweep did. The records of removed and foreign
/// paths are deleted; a deferred one is tried again on a later pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SettledSweep {
    /// Workspaces removed, or found already gone.
    pub removed: usize,
    /// Paths held by something Scryer did not make, left in place.
    pub left_foreign: usize,
    /// Workspaces that stay for now: removal failed, or an extraction of
    /// the download runs.
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
    let in_flight = &app.runtime.imports.archive_extractions_in_flight;
    let mut sweep = SettledSweep::default();
    let mut done = Vec::new();
    let mut retry = Vec::new();
    for record in &due {
        let key = DownloadExtractionKey::of_record(record);
        // A running extraction or a reader keeps its workspace; still due
        // next pass.
        let Some(_claim) = in_flight.claim_for_removal(&key) else {
            sweep.deferred += 1;
            continue;
        };
        let root = crate::stored_paths::stored_path_to_path_buf(&record.workspace_path);
        match remove_settled_workspace(&root, &key).await {
            SettledRemoval::Gone => {
                sweep.removed += 1;
                done.push(key.pair());
            }
            SettledRemoval::LeftForeign => {
                sweep.left_foreign += 1;
                done.push(key.pair());
            }
            SettledRemoval::Retry => {
                sweep.deferred += 1;
                retry.push(key.pair());
            }
        }
    }
    repository.delete_many(&done).await?;
    repository.defer_settled(&retry, now).await?;
    Ok(sweep)
}

/// Unsettled records the pass settling untracked downloads reads at a time,
/// and pages it reads per pass; the next pass resumes where it stopped.
const UNTRACKED_PAGE: usize = 500;
const UNTRACKED_PAGES_PER_PASS: usize = 20;

/// Settle downloads the tracker no longer follows, and downloads of clients
/// no longer configured, as ended without an import, so their workspaces go
/// [`SETTLED_WORKSPACE_RETENTION`] later. A record changed within that
/// period is left: a manual import of a download the tracker never listed
/// may be using it. Before a client has delivered a listing since startup,
/// none of its downloads is settled. Returns how many were settled; a pass
/// that settles none writes nothing.
pub(crate) async fn settle_untracked_download_extractions(app: &AppUseCase) -> AppResult<usize> {
    let Some(tracker) = app.runtime.acquisition.tracked_download_handle.clone() else {
        return Ok(0);
    };
    // Each configured client by its key's spelling: its id as configured
    // and its type, which name its downloads' imports.
    let configured: std::collections::HashMap<String, (String, String)> = app
        .services
        .integrations
        .download_client_configs
        .list(None)
        .await?
        .into_iter()
        .map(|config| {
            (
                config.id.trim().to_ascii_lowercase(),
                (config.id.trim().to_string(), config.client_type),
            )
        })
        .collect();
    let repository = &app.services.workflow.archive_extractions;
    let upkeep = &app.runtime.imports.archive_workspace_upkeep;
    let now = app.runtime.environment.now();
    let changed_before = now - SETTLED_WORKSPACE_RETENTION;
    let mut after = upkeep
        .untracked_cursor
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let mut settled = 0;
    for _ in 0..UNTRACKED_PAGES_PER_PASS {
        let page = repository
            .list_unsettled_after(after.clone(), changed_before, UNTRACKED_PAGE)
            .await?;
        after = page
            .last()
            .filter(|_| page.len() == UNTRACKED_PAGE)
            .map(|last| (last.client_id.clone(), last.client_item_id.clone()));
        let (mut ended, of_configured): (Vec<_>, Vec<_>) = page
            .into_iter()
            .map(|row| (row.client_id, row.client_item_id))
            .partition(|(client_id, _)| !configured.contains_key(client_id));
        if !of_configured.is_empty() {
            let untracked = tracker.untracked_extractions(of_configured).await?;
            // A download an import still holds the sources of (subtitles
            // pending, an unfinished cleanup) keeps its workspace however it
            // left its client; it settles once the hold is released.
            let sources: Vec<_> = untracked
                .iter()
                .filter_map(|(client_id, item_id)| {
                    let (id, client_type) = configured.get(client_id)?;
                    Some(ClientJobLocator::new(Some(id), client_type, item_id))
                })
                .collect();
            let held = crate::import::workflow::downloads_holding_sources(app, &sources).await?;
            ended.extend(untracked.into_iter().filter(|key| !held.contains(key)));
        }
        if !ended.is_empty() {
            repository.settle(&ended, now).await?;
            settled += ended.len();
        }
        if after.is_none() {
            break;
        }
    }
    *upkeep
        .untracked_cursor
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = after;
    Ok(settled)
}

/// What one sweep of the library roots' extraction folders found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OrphanSweep {
    /// Owned workspaces no record names, made a day ago or more: removed.
    pub removed: usize,
    /// Owned workspaces kept for now: younger than a day (whether or not a
    /// record names them), an extraction runs in them, or removal failed.
    pub waiting: usize,
    /// Directories named like a workspace whose marker does not prove Scryer
    /// made them for the download their name names. They are never removed;
    /// entries named otherwise are not counted.
    pub unmarked: usize,
}

/// The library roots, each once, in configuration order.
async fn library_roots(app: &AppUseCase) -> AppResult<Vec<PathBuf>> {
    let mut seen = HashSet::new();
    Ok(app
        .services
        .catalog
        .libraries
        .list(None)
        .await?
        .into_iter()
        .flat_map(|library| library.roots)
        .map(|root| PathBuf::from(root.path))
        .filter(|root| !root.as_os_str().is_empty() && seen.insert(root.clone()))
        .collect())
}

/// Remove owned download workspaces that no record names, a day after they
/// were made, from each library root's extraction folder.
///
/// Per root: one listing of the extraction folder, and nothing more when it
/// is missing or empty. Per directory named like a workspace: one directory
/// open, one reading and one read of its marker; any other entry costs
/// nothing. One batched lookup per root of the old ones' names, and
/// for each one removed, the owner check and release of the removal itself.
///
/// It never races an extraction: a record is saved before its workspace is
/// made and deleted only after the workspace is gone, a workspace being
/// extracted is in the in-flight set and is skipped, and a fresh one is
/// younger than the day of grace.
pub(crate) async fn sweep_orphaned_download_workspaces(app: &AppUseCase) -> AppResult<OrphanSweep> {
    let mut sweep = OrphanSweep::default();
    if !archive_extractor::PROVES_WORKSPACE_OWNERSHIP {
        return Ok(sweep);
    }
    let in_flight = &app.runtime.imports.archive_extractions_in_flight;
    let made_before =
        std::time::SystemTime::from(app.runtime.environment.now() - SETTLED_WORKSPACE_RETENTION);
    for root in library_roots(app).await? {
        let folder = root.join(ARCHIVE_EXTRACTION_DIR_NAME);
        let Ok(entries) = crate::disk::Disk::process().list(&folder).await else {
            continue;
        };
        let mut old = Vec::new();
        for entry in entries {
            let name = entry
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string);
            // Only a directory named like a workspace can be one; anything
            // else (a desktop's metadata file, a NAS index folder) is not
            // Scryer's, is not counted, and is never touched.
            let Some(name) =
                name.filter(|name| archive_extractor::is_download_workspace_name(name))
            else {
                continue;
            };
            if entry.reading.kind() != Some(crate::disk::EntryKind::Directory) {
                continue;
            }
            if in_flight.holds_workspace(&name) {
                sweep.waiting += 1;
                continue;
            }
            let owner_id = format!("download:{name}");
            match archive_extractor::download_workspace_marked_at(&entry.path, &owner_id).await {
                None => sweep.unmarked += 1,
                Some(made) if made <= made_before => old.push((entry.path, name, owner_id)),
                Some(_) => sweep.waiting += 1,
            }
        }
        if old.is_empty() {
            continue;
        }
        let names: Vec<_> = old.iter().map(|(_, name, _)| name.clone()).collect();
        let recorded = app
            .services
            .workflow
            .archive_extractions
            .recorded_workspace_names(&names)
            .await?;
        for (path, name, owner_id) in old {
            // A recorded workspace goes when its download settles.
            if recorded.contains(&name) {
                continue;
            }
            if !in_flight.holds_workspace(&name)
                && archive_extractor::remove_download_workspace(&path, &owner_id).await
            {
                sweep.removed += 1;
            } else {
                sweep.waiting += 1;
            }
        }
    }
    *app.runtime
        .imports
        .archive_workspace_upkeep
        .last_orphan_sweep
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sweep);
    Ok(sweep)
}

/// Remove per-pass workspaces staged directly in each library root under
/// their usual rule: owned, released, and released a day ago or more. One
/// listing per root; entries not named like a workspace are not opened.
pub(crate) async fn sweep_library_root_pass_workspaces(app: &AppUseCase) -> AppResult<usize> {
    let mut removed = 0;
    for root in library_roots(app).await? {
        removed += archive_extractor::sweep_stale_archive_workspaces_in(&root).await;
    }
    Ok(removed)
}

/// Folders a library scan noted at most. When it is full, new notes are
/// dropped; a later scan notes them again.
const NOTED_FOLDER_CAP: usize = 4_096;

/// Noted folders one housekeeping pass sweeps; the rest stay noted.
const NOTED_FOLDER_SWEEP_LIMIT: usize = 100;

/// Folders a library scan saw holding a directory named like a per-pass
/// workspace, held in memory for this process only, for housekeeping to
/// sweep.
static NOTED_PASS_WORKSPACE_FOLDERS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeSet<PathBuf>>,
> = std::sync::LazyLock::new(Default::default);

fn noted_pass_workspace_folders()
-> std::sync::MutexGuard<'static, std::collections::BTreeSet<PathBuf>> {
    NOTED_PASS_WORKSPACE_FOLDERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Note `parent` for housekeeping when `child`, an entry a scan has just
/// listed in it, is named like a per-pass workspace. A string test on the
/// name in hand: nothing is read from disk.
pub(crate) fn note_pass_workspace_parent(parent: &Path, child: &Path) {
    if !child
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(archive_extractor::is_pass_workspace_name)
    {
        return;
    }
    let mut noted = noted_pass_workspace_folders();
    if noted.len() < NOTED_FOLDER_CAP && !noted.contains(parent) {
        noted.insert(parent.to_path_buf());
    }
}

#[cfg(test)]
pub(crate) fn noted_pass_workspace_folders_for_test() -> Vec<PathBuf> {
    noted_pass_workspace_folders().iter().cloned().collect()
}

/// Up to `limit` noted folders inside one of `roots`, taken out of the
/// notes. A noted folder outside every root is dropped unswept, and a root
/// itself is dropped since the root sweep covers it; neither counts against
/// the limit.
fn take_noted_pass_workspace_folders(roots: &[PathBuf], limit: usize) -> Vec<PathBuf> {
    let mut noted = noted_pass_workspace_folders();
    let mut taken = Vec::new();
    while taken.len() < limit {
        let Some(folder) = noted.pop_first() else {
            break;
        };
        if roots
            .iter()
            .any(|root| folder.starts_with(root) && folder != *root)
        {
            taken.push(folder);
        }
    }
    taken
}

/// Remove per-pass workspaces in the folders a library scan noted, under
/// their usual rule: owned, released, and released a day ago or more. With
/// nothing noted it reads nothing. Per folder swept: one listing, and only
/// entries named like a workspace are opened.
pub(crate) async fn sweep_noted_pass_workspaces(app: &AppUseCase) -> AppResult<usize> {
    if noted_pass_workspace_folders().is_empty() {
        return Ok(0);
    }
    let roots = library_roots(app).await?;
    let mut removed = 0;
    for folder in take_noted_pass_workspace_folders(&roots, NOTED_FOLDER_SWEEP_LIMIT) {
        removed += archive_extractor::sweep_stale_archive_workspaces_in(&folder).await;
    }
    Ok(removed)
}

/// What housekeeping's workspace upkeep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArchiveWorkspaceHousekeeping {
    pub untracked_settled: usize,
    pub settled: SettledSweep,
    pub orphans: OrphanSweep,
    pub pass_workspaces_removed: usize,
    /// Per-pass workspaces removed from folders a library scan noted.
    pub noted_pass_workspaces_removed: usize,
}

/// Housekeeping's workspace upkeep, each step on its own: a step that fails
/// is logged and the rest still run.
pub(crate) async fn archive_workspace_housekeeping(
    app: &AppUseCase,
) -> ArchiveWorkspaceHousekeeping {
    let mut report = ArchiveWorkspaceHousekeeping::default();
    match settle_untracked_download_extractions(app).await {
        Ok(settled) => report.untracked_settled = settled,
        Err(error) => {
            tracing::warn!(error = %error, "untracked downloads' archive extractions could not be settled")
        }
    }
    match sweep_settled_download_extractions(app).await {
        Ok(sweep) => report.settled = sweep,
        Err(error) => {
            tracing::warn!(error = %error, "archive workspaces of settled downloads could not be swept")
        }
    }
    match sweep_orphaned_download_workspaces(app).await {
        Ok(sweep) => report.orphans = sweep,
        Err(error) => {
            tracing::warn!(error = %error, "orphaned archive workspaces could not be swept")
        }
    }
    match sweep_library_root_pass_workspaces(app).await {
        Ok(removed) => report.pass_workspaces_removed = removed,
        Err(error) => {
            tracing::warn!(error = %error, "stale archive workspaces in library roots could not be swept")
        }
    }
    match sweep_noted_pass_workspaces(app).await {
        Ok(removed) => report.noted_pass_workspaces_removed = removed,
        Err(error) => {
            tracing::warn!(error = %error, "stale archive workspaces in noted title folders could not be swept")
        }
    }
    report
}

/// The startup sweep: orphaned download workspaces and stale per-pass
/// workspaces in the library roots. Downloads are not settled here, since
/// no client has been listed yet.
pub async fn sweep_archive_workspaces_at_startup(app: &AppUseCase) {
    let orphans = match sweep_orphaned_download_workspaces(app).await {
        Ok(sweep) => sweep,
        Err(error) => {
            tracing::warn!(error = %error, "orphaned archive workspaces could not be swept at startup");
            OrphanSweep::default()
        }
    };
    let pass_workspaces_removed = sweep_library_root_pass_workspaces(app).await.unwrap_or_else(|error| {
        tracing::warn!(error = %error, "stale archive workspaces in library roots could not be swept at startup");
        0
    });
    if orphans != OrphanSweep::default() || pass_workspaces_removed > 0 {
        tracing::info!(
            orphans_removed = orphans.removed,
            orphans_waiting = orphans.waiting,
            unmarked = orphans.unmarked,
            pass_workspaces_removed,
            "swept archive workspaces at startup"
        );
    }
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
    fn a_download_without_a_client_id_has_no_key() {
        assert!(DownloadExtractionKey::new(None, "item-1").is_none());
        assert!(DownloadExtractionKey::new(Some(""), "item-1").is_none());
        assert!(DownloadExtractionKey::new(Some("  "), "item-1").is_none());
        let completed = crate::CompletedDownload {
            client_type: "nzbget".to_string(),
            client_id: String::new(),
            download_client_item_id: "10495".to_string(),
            download_id: None,
            name: "Synthetic.Release".to_string(),
            release_name: None,
            dest_dir: "/downloads/Synthetic.Release".to_string(),
            category: None,
            size_bytes: None,
            completed_at: None,
            parameters: Vec::new(),
            posted_file_names: Vec::new(),
            subtitle_paths: Vec::new(),
        };
        assert!(DownloadExtractionKey::for_completed(&completed).is_none());
        let source = ClientJobLocator::new(None, "nzbget", "10495");
        assert!(DownloadExtractionKey::for_source(&source).is_none());
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
