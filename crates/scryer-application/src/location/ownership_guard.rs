//! Operation-ownership registry (D7, FR-084).
//!
//! While an operation owns a title or a root, every conflicting entry point —
//! library scans, imports, renames, title deletion, media-file mutation, other
//! location operations, root removal/configuration, and policy automation or
//! maintenance jobs — consults one choke-point helper and refuses. Unrelated
//! titles and libraries keep operating normally; there is no global lock.
//!
//! # Registering a new mutating entry point
//!
//! Every guarded call site goes through [`AppUseCase::ensure_location_ownership_allows`]
//! (or one of its entity-shaped wrappers) and names a [`GuardedEntry`] constant
//! declared in this module. Adding a new mutating entry point therefore means:
//!
//! 1. declare a `GuardedEntry` constant here, naming the module path and
//!    function that carries the check;
//! 2. add it to [`GUARDED_ENTRIES`];
//! 3. call the choke point with that constant at the entry point's admission.
//!
//! [`guarded_entries_are_wired`] reads the source of every registered module and
//! fails when a declared entry's function or constant is missing from it, so the
//! declaration and the wiring cannot drift apart silently.
//!
//! # Layering
//!
//! [`LocationOwnershipRegistry`] is the in-process fast path: the operation
//! runner writes claims into it as it takes them and clears them when the
//! operation reaches a terminal state, so a conflict inside this process is
//! answered without a query. Persisted claims (`LocationOperationRepository`)
//! remain the source of truth and are consulted whenever the registry is silent,
//! so a restart — which empties the registry but not the claim rows — still
//! refuses.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::{AppError, AppResult};

/// An entity an active operation holds for its duration. Persisted so ownership
/// survives a restart alongside the operation itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum OwnedEntity {
    /// One catalog title.
    Title(String),
    /// One root, by synthetic root id (FR-078).
    Root(String),
}

impl OwnedEntity {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Title(_) => "title",
            Self::Root(_) => "root",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Title(id) | Self::Root(id) => id,
        }
    }
}

/// The kinds of work the guard refuses while an entity is owned (FR-084). The
/// audit test in T016 enumerates one guarded entry point per variant.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GuardedAction {
    TitleEdit,
    Download,
    LibraryScan,
    Import,
    Rename,
    TitleDelete,
    MediaFileMutation,
    LocationOperation,
    RootConfiguration,
    MaintenanceJob,
    FolderMatch,
}

impl GuardedAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LibraryScan => "library_scan",
            Self::TitleEdit => "title_edit",
            Self::Download => "download",
            Self::Import => "import",
            Self::Rename => "rename",
            Self::TitleDelete => "title_delete",
            Self::MediaFileMutation => "media_file_mutation",
            Self::LocationOperation => "location_operation",
            Self::RootConfiguration => "root_configuration",
            Self::MaintenanceJob => "maintenance_job",
            Self::FolderMatch => "folder_match",
        }
    }

    /// How the refusal names the blocked work to an operator.
    pub fn label(&self) -> &'static str {
        match self {
            Self::LibraryScan => "a library scan",
            Self::TitleEdit => "editing this title or its episodes",
            Self::Download => "downloading or upgrading this title",
            Self::Import => "an import",
            Self::Rename => "a rename",
            Self::TitleDelete => "deleting this title",
            Self::MediaFileMutation => "changing this title's media files",
            Self::LocationOperation => "another location operation",
            Self::RootConfiguration => "changing this root configuration",
            Self::MaintenanceJob => "this maintenance job",
            Self::FolderMatch => "correcting this title's folder match",
        }
    }
}

/// A refusal from the choke-point helper, carrying enough context for an
/// actionable error (C6: typed, actionable, never silent reinterpretation).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnershipConflict {
    /// Operation currently holding the entity.
    pub operation_id: String,
    /// The entity that is owned.
    pub entity: OwnedEntity,
    /// What the caller was trying to do.
    pub action: GuardedAction,
}

/// In-process mirror of the persisted claims (D7).
///
/// The operation runner writes into it when a claim succeeds and clears the
/// operation's rows when the operation stops, so a same-process conflict is
/// refused without a query. It is a fast path, never the authority: an empty
/// registry (a fresh process, a claim taken before this table existed) still
/// falls through to the persisted claims.
#[derive(Clone, Default)]
pub struct LocationOwnershipRegistry {
    claims: Arc<RwLock<HashMap<OwnedEntity, String>>>,
    admissions: Arc<std::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::RwLock<()>>>>>,
}

impl LocationOwnershipRegistry {
    fn admission_lock(&self, title_id: &str) -> Arc<tokio::sync::RwLock<()>> {
        let mut locks = self
            .admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(lock) = locks.get(title_id).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        if locks.len() >= 256 && locks.len().is_multiple_of(256) {
            locks.retain(|_, lock| lock.strong_count() > 0);
        }
        let lock = Arc::new(tokio::sync::RwLock::new(()));
        locks.insert(title_id.to_string(), Arc::downgrade(&lock));
        lock
    }

    /// Drain work admitted before ownership is claimed. Sorting prevents two
    /// overlapping operations from waiting on each other's title locks.
    pub async fn drain_title_mutations(
        &self,
        entities: &[OwnedEntity],
    ) -> Vec<tokio::sync::OwnedRwLockWriteGuard<()>> {
        let mut ids = entities
            .iter()
            .filter_map(|entity| match entity {
                OwnedEntity::Title(id) => Some(id.as_str()),
                OwnedEntity::Root(_) => None,
            })
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        let mut guards = Vec::with_capacity(ids.len());
        for id in ids {
            guards.push(self.admission_lock(id).write_owned().await);
        }
        guards
    }

    fn admit_title_mutation(
        &self,
        title_id: &str,
    ) -> AppResult<tokio::sync::OwnedRwLockReadGuard<()>> {
        // Never wait behind a move while retaining an outer mutation lease:
        // nested workflows must refuse instead of deadlocking the drain.
        self.admission_lock(title_id).try_read_owned().map_err(|_| AppError::LocationOperationBusy(
            format!("title {title_id} is being locked for a location operation; retry after the operation finishes")
        ))
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Mirrors a successful claim. Called only after the store accepted it, so
    /// the registry can never invent ownership the datastore does not hold.
    pub fn claim_all(&self, operation_id: &str, entities: &[OwnedEntity]) {
        let Ok(mut claims) = self.claims.write() else {
            return;
        };
        for entity in entities {
            claims.insert(entity.clone(), operation_id.to_string());
        }
    }

    /// Drops every entity this operation holds.
    pub fn release_operation(&self, operation_id: &str) {
        let Ok(mut claims) = self.claims.write() else {
            return;
        };
        claims.retain(|_, holder| holder != operation_id);
    }

    pub fn holder(&self, entity: &OwnedEntity) -> Option<String> {
        self.claims.read().ok()?.get(entity).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.claims
            .read()
            .map(|claims| claims.is_empty())
            .unwrap_or(true)
    }
}

/// Which operations have a runner alive *in this process* right now.
///
/// The persisted ownership claim is idempotent for the same operation — that is
/// what makes a resume able to re-claim what it already holds — so it cannot
/// tell a second runner apart from the same runner picking its work back up. A
/// second runner over one set of checkpoints would double-walk the plan, so
/// this is the guard that refuses it.
///
/// Scryer runs as a single process, which is what makes an in-process set the
/// right authority here: a runner is either in this process or it is not
/// running at all. The boot resume runs before anything is spawned, so it is
/// never refused by its own predecessor.
#[derive(Clone, Default)]
pub struct LocationRunnerRegistry {
    pub transfers: super::live::TransferHub,
    pub transfer_generation: Arc<tokio::sync::OnceCell<i64>>,
    live: Arc<std::sync::Mutex<HashSet<String>>>,
    execution: Arc<tokio::sync::Mutex<()>>,
    completed: Arc<tokio::sync::Notify>,
    /// Set once the boot resume has spawned every runner it intends to. Until
    /// then a Queued row without a live runner is simply not picked up yet.
    boot_resume_settled: Arc<std::sync::atomic::AtomicBool>,
    /// Queued rows this registry failed because their runner was lost. The
    /// runtime drains these to close their Activity runs and release their
    /// ownership claims, which the registry cannot reach.
    stranded: Arc<std::sync::Mutex<Vec<StrandedOperation>>>,
}

/// A Queued operation whose runner disappeared before it ever started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrandedOperation {
    pub operation_id: String,
    pub job_run_id: Option<String>,
    pub counters: super::model::LocationOperationCounters,
    pub detail: String,
}

/// Longer than this without a live runner and an accepted row is not merely
/// between `create` and `spawn`; whatever should have run it is gone.
const STRANDED_GRACE: chrono::Duration = chrono::Duration::seconds(30);

pub(crate) const STRANDED_DETAIL: &str = "The operation's runner was lost before any file moved, \
so it was marked failed to unblock the queue. Nothing was changed on disk; plan it again.";

/// The same stranding for a retry that never got its runner: files did move
/// once, so nothing about the disk is claimed and the way out is Retry.
pub(crate) const STRANDED_RETRY_DETAIL: &str = "The operation's runner was lost before its retry \
moved any file, so it was marked failed to unblock the queue. Finished titles are unchanged; retry it again.";

/// Which stranding text fits: a row that once started is a retry that never
/// got going, not a fresh operation nothing touched.
fn stranded_detail(started_before: bool) -> &'static str {
    if started_before {
        STRANDED_RETRY_DETAIL
    } else {
        STRANDED_DETAIL
    }
}

impl LocationRunnerRegistry {
    /// The boot resume has spawned every runner it intends to; from now on a
    /// Queued row with no live runner is stranded, not pending pickup.
    pub fn mark_boot_resume_settled(&self) {
        self.boot_resume_settled
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Hand back the stranded rows failed since the last call.
    pub fn take_stranded(&self) -> Vec<StrandedOperation> {
        self.stranded
            .lock()
            .map(|mut stranded| std::mem::take(&mut *stranded))
            .unwrap_or_default()
    }

    /// Fail a Queued row at the head of the FIFO whose runner is gone.
    ///
    /// Returns whether the head was failed, so the caller re-reads the FIFO.
    async fn fail_if_stranded(
        &self,
        store: &dyn crate::LocationOperationRepository,
        head: &super::model::LocationOperation,
    ) -> AppResult<bool> {
        if head.state != super::model::LocationOperationState::Queued
            || self.is_live(&head.id)
            || !self
                .boot_resume_settled
                .load(std::sync::atomic::Ordering::Acquire)
            // A reopened row is as new as its reopen: `updated_at` is when it
            // went back to queued, and its runner is as far behind as a
            // fresh row's.
            || chrono::Utc::now() - head.created_at.max(head.updated_at) < STRANDED_GRACE
        {
            return Ok(false);
        }
        // Re-read under the slot: a runner may have registered since the list.
        let Some(current) = store.get_location_operation(&head.id).await? else {
            return Ok(false);
        };
        if current.state != super::model::LocationOperationState::Queued || self.is_live(&head.id) {
            return Ok(false);
        }
        tracing::warn!(
            operation_id = %current.id,
            operation_type = current.operation_type.as_str(),
            "an accepted location operation has no runner; failing it so the queue moves"
        );
        let now = chrono::Utc::now();
        store
            .update_location_operation_progress(&crate::LocationOperationProgress {
                operation_id: current.id.clone(),
                state: super::model::LocationOperationState::Failed,
                counters: current.counters,
                verification_fallback_count: current.verification_fallback_count,
                detail: Some(stranded_detail(current.started_at.is_some()).to_owned()),
                clear_detail: false,
                reason_code: Some(super::model::LocationReasonCode::Stranded),
                started_at: current.started_at,
                completed_at: Some(now),
            })
            .await?;
        if let Ok(mut stranded) = self.stranded.lock() {
            stranded.push(StrandedOperation {
                operation_id: current.id.clone(),
                job_run_id: current.job_run_id.clone(),
                counters: current.counters,
                detail: stranded_detail(current.started_at.is_some()).to_owned(),
            });
        }
        Ok(true)
    }

    /// Held through draining and cleanup, not merely while copies run.
    pub async fn execution_slot(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.execution.clone().lock_owned().await
    }

    pub async fn wait_for_turn(
        &self,
        store: &dyn crate::LocationOperationRepository,
        operation_id: &str,
    ) -> AppResult<tokio::sync::OwnedMutexGuard<()>> {
        loop {
            // Register before reading FIFO state so completion between the
            // read and the wait cannot be lost.
            let completed = self.completed.notified();
            tokio::pin!(completed);
            completed.as_mut().enable();
            let slot = self.execution_slot().await;
            let mut pending = store.list_active_location_operations().await?;
            // Include accepted requests whose compact summaries are still
            // being persisted. Task wake order cannot overtake durable FIFO.
            pending.retain(|operation| {
                operation.state == super::model::LocationOperationState::Queued
                    || self.is_live(&operation.id)
                    || operation.id == operation_id
            });
            pending.sort_by_key(|operation| {
                (
                    operation.started_at.is_none(),
                    operation.created_at,
                    operation.id.clone(),
                )
            });
            if pending
                .first()
                .is_none_or(|operation| operation.id == operation_id)
                || store
                    .get_location_operation(operation_id)
                    .await?
                    .is_some_and(|operation| operation.state.is_terminal())
            {
                return Ok(slot);
            }
            if let Some(head) = pending.first()
                && self.fail_if_stranded(store, head).await?
            {
                drop(slot);
                continue;
            }
            drop(slot);
            // An accepted row can outlive its runner. Recheck that durable
            // state periodically without polling the store at file-loop rates.
            tokio::select! {
                () = completed => {},
                () = tokio::time::sleep(std::time::Duration::from_secs(2)) => {},
            }
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Register a runner for `operation_id`, unless one is already live.
    ///
    /// The returned guard deregisters on drop, so a runner that panics does not
    /// leave the operation permanently unresumable.
    pub fn begin(&self, operation_id: &str) -> Option<LiveRunnerGuard> {
        let mut live = self.live.lock().ok()?;
        if !live.insert(operation_id.to_string()) {
            return None;
        }
        Some(LiveRunnerGuard {
            registry: self.clone(),
            operation_id: operation_id.to_string(),
        })
    }

    pub fn is_live(&self, operation_id: &str) -> bool {
        self.live
            .lock()
            .map(|live| live.contains(operation_id))
            .unwrap_or(false)
    }

    fn finish(&self, operation_id: &str) {
        if let Ok(mut live) = self.live.lock() {
            live.remove(operation_id);
        }
        self.completed.notify_waiters();
    }
}

impl std::fmt::Debug for LocationRunnerRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocationRunnerRegistry")
            .field(
                "live",
                &self.live.lock().map(|live| live.len()).unwrap_or(0),
            )
            .finish()
    }
}

/// Holds an operation's in-process runner slot for as long as the runner runs.
pub struct LiveRunnerGuard {
    registry: LocationRunnerRegistry,
    operation_id: String,
}

impl Drop for LiveRunnerGuard {
    fn drop(&mut self) {
        self.registry.finish(&self.operation_id);
    }
}

impl std::fmt::Debug for LocationOwnershipRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocationOwnershipRegistry")
            .field(
                "claims",
                &self.claims.read().map(|claims| claims.len()).unwrap_or(0),
            )
            .finish()
    }
}

/// The typed refusal the choke point returns (C6: actionable, never a silent
/// reinterpretation of what the caller asked for).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationOwnershipDenied {
    /// The entry point that was refused.
    pub entry: &'static GuardedEntry,
    /// Every overlapping entity, in the order the caller listed them.
    pub conflicts: Vec<OwnershipConflict>,
}

impl LocationOwnershipDenied {
    pub fn action(&self) -> GuardedAction {
        self.entry.action
    }

    /// The operation ids holding the overlapping entities, deduplicated.
    pub fn holding_operation_ids(&self) -> Vec<String> {
        let mut ids = self
            .conflicts
            .iter()
            .map(|conflict| conflict.operation_id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn message(&self) -> String {
        let held = self
            .conflicts
            .iter()
            .map(|conflict| {
                format!(
                    "{} {} (operation {})",
                    conflict.entity.kind_str(),
                    conflict.entity.id(),
                    conflict.operation_id
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} is blocked while a location operation owns {}; it can run again once that operation finishes or is canceled",
            self.entry.action.label(),
            held
        )
    }
}

impl std::fmt::Display for LocationOwnershipDenied {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl LocationOwnershipDenied {
    /// Surfaces through each entry point's existing error convention: a refusal
    /// the operator can act on, not a repository failure.
    ///
    /// Deliberately an inherent method rather than a `From` impl — a blanket
    /// conversion into [`AppError`] makes `Ok(())` ambiguous in the crate's many
    /// inferred-error closures.
    pub fn into_app_error(self) -> AppError {
        AppError::Validation(self.message())
    }
}

/// One registered mutating entry point (FR-084).
///
/// `module` and `function` are what [`guarded_entries_are_wired`] checks against
/// the real source, and what an operator-facing diagnostic can print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardedEntry {
    pub action: GuardedAction,
    /// Path of the module carrying the call, relative to
    /// `crates/scryer-application/src/`.
    pub module: &'static str,
    /// The function whose admission performs the check.
    pub function: &'static str,
    /// This constant's own name, so the audit can find the call site.
    pub constant: &'static str,
}

/// Library scans: every scan session, however it was triggered, funnels here.
pub const LIBRARY_SCAN_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::LibraryScan,
    module: "library/library.rs",
    function: "run_started_library_scan_session",
    constant: "LIBRARY_SCAN_ENTRY",
};

/// Automatic and manual-review imports of a completed download, checked once the
/// target title is resolved and before any file is touched.
pub const COMPLETED_IMPORT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::Import,
    module: "import/workflow/series_movie.rs",
    function: "dispatch_completed_import_target",
    constant: "COMPLETED_IMPORT_ENTRY",
};

/// Operator-driven manual imports, which do not pass through the completed
/// download dispatcher.
pub const MANUAL_IMPORT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::Import,
    module: "import/workflow/manual.rs",
    function: "execute_manual_import_with_release_evidence",
    constant: "MANUAL_IMPORT_ENTRY",
};

/// The rename apply path, shared by the per-title and per-facet entry points.
pub const RENAME_APPLY_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::Rename,
    module: "library/rename.rs",
    function: "apply_rename_plan",
    constant: "RENAME_APPLY_ENTRY",
};

/// Single-title deletion.
pub const TITLE_DELETE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleDelete,
    module: "catalog/workflow/delete.rs",
    function: "delete_title",
    constant: "TITLE_DELETE_ENTRY",
};

/// The bulk deletion job's per-title work, which does not route through
/// [`TITLE_DELETE_ENTRY`].
pub const TITLE_DELETE_JOB_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleDelete,
    module: "catalog/workflow/delete.rs",
    function: "delete_title_job_item",
    constant: "TITLE_DELETE_JOB_ENTRY",
};

/// Media-file deletion, including the per-item work of the bulk deletion job.
pub const MEDIA_FILE_DELETE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::MediaFileMutation,
    module: "catalog/workflow/delete.rs",
    function: "delete_media_file",
    constant: "MEDIA_FILE_DELETE_ENTRY",
};

/// Primary media-file changes, which rewrite which file a title serves.
pub const MEDIA_FILE_PRIMARY_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::MediaFileMutation,
    module: "catalog/workflow/metadata.rs",
    function: "set_primary_movie_file",
    constant: "MEDIA_FILE_PRIMARY_ENTRY",
};

/// Root configuration changes on an existing library.
pub const LIBRARY_ROOTS_UPDATE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::RootConfiguration,
    module: "library/library.rs",
    function: "update_library",
    constant: "LIBRARY_ROOTS_UPDATE_ENTRY",
};

/// Library removal, which retires every root the library configures.
pub const LIBRARY_DELETE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::RootConfiguration,
    module: "library/library.rs",
    function: "delete_library",
    constant: "LIBRARY_DELETE_ENTRY",
};

/// Recycle-bin restore, the maintenance job that writes files back into a
/// title's folder.
pub const RECYCLE_RESTORE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::MaintenanceJob,
    module: "jobs/housekeeping.rs",
    function: "restore_recycled_item_from_context",
    constant: "RECYCLE_RESTORE_ENTRY",
};

/// Folder-match correction reassigns the very folder ownership a running
/// operation's plan was built against (US1 is itself a location workflow, just
/// a synchronous catalog-only one that registers no persisted operation).
pub const FOLDER_MATCH_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::FolderMatch,
    module: "location/folder_match.rs",
    function: "apply_title_folder_change",
    constant: "FOLDER_MATCH_ENTRY",
};

/// The maintenance executor's policy delete, which does not route through
/// [`TITLE_DELETE_ENTRY`]: a scheduled destructive action is a delete like any
/// other, and a title an operation is mid-move stays out of its reach.
pub const MAINTENANCE_DELETE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::MaintenanceJob,
    module: "catalog/workflow/delete.rs",
    function: "delete_title_by_policy",
    constant: "MAINTENANCE_DELETE_ENTRY",
};

/// The maintenance executor's pre-action recheck. A destructive candidate whose
/// title an operation owns *holds* rather than fails — the operation releases
/// its claim on every terminal path, so the action simply retries later.
pub const MAINTENANCE_ACTION_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::MaintenanceJob,
    module: "maintenance_rules/action_execution.rs",
    function: "maintenance_execution_safety_checks",
    constant: "MAINTENANCE_ACTION_ENTRY",
};

pub const TITLE_METADATA_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/metadata.rs",
    function: "update_title_metadata_with_root_folder_id",
    constant: "TITLE_METADATA_EDIT_ENTRY",
};

pub const TITLE_LANGUAGE_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/metadata.rs",
    function: "set_title_metadata_language_override",
    constant: "TITLE_LANGUAGE_EDIT_ENTRY",
};

pub const TITLE_REMATCH_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/metadata.rs",
    function: "fix_title_match",
    constant: "TITLE_REMATCH_ENTRY",
};

pub const TITLE_MONITOR_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/monitoring.rs",
    function: "set_title_monitored",
    constant: "TITLE_MONITOR_EDIT_ENTRY",
};

pub const COLLECTION_MONITOR_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/monitoring.rs",
    function: "set_collection_monitored",
    constant: "COLLECTION_MONITOR_EDIT_ENTRY",
};

pub const EPISODE_MONITOR_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/monitoring.rs",
    function: "set_episode_monitored",
    constant: "EPISODE_MONITOR_EDIT_ENTRY",
};

pub const COLLECTION_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/monitoring.rs",
    function: "update_collection",
    constant: "COLLECTION_EDIT_ENTRY",
};

pub const EPISODE_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/monitoring.rs",
    function: "update_episode",
    constant: "EPISODE_EDIT_ENTRY",
};

pub const SERIES_MOVIE_MONITOR_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/monitoring.rs",
    function: "set_series_movie_monitored",
    constant: "SERIES_MOVIE_MONITOR_EDIT_ENTRY",
};

pub const TITLE_MONITOR_SELECTION_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/monitoring.rs",
    function: "set_title_monitor_selection",
    constant: "TITLE_MONITOR_SELECTION_ENTRY",
};

pub const TITLE_TAG_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/title_tags.rs",
    function: "update_title_tags",
    constant: "TITLE_TAG_EDIT_ENTRY",
};

pub const SERIES_MOVIE_TAG_EDIT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/title_tags.rs",
    function: "update_series_movie_tags",
    constant: "SERIES_MOVIE_TAG_EDIT_ENTRY",
};

pub const TITLE_HYDRATION_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/hydration.rs",
    function: "apply_hydration_result",
    constant: "TITLE_HYDRATION_ENTRY",
};

pub const TITLE_DOWNLOAD_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::Download,
    module: "acquisition/submission.rs",
    function: "submit_canonical_download",
    constant: "TITLE_DOWNLOAD_ENTRY",
};

pub const COLLECTION_CREATE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/collections.rs",
    function: "create_collection",
    constant: "COLLECTION_CREATE_ENTRY",
};

pub const EPISODE_CREATE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleEdit,
    module: "catalog/workflow/collections.rs",
    function: "create_episode",
    constant: "EPISODE_CREATE_ENTRY",
};

/// Every registered mutating entry point. A new one is added here and nowhere
/// else; [`guarded_entries_are_wired`] fails when this list and the code drift.
pub const GUARDED_ENTRIES: &[&GuardedEntry] = &[
    &COLLECTION_CREATE_ENTRY,
    &EPISODE_CREATE_ENTRY,
    &TITLE_METADATA_EDIT_ENTRY,
    &TITLE_LANGUAGE_EDIT_ENTRY,
    &TITLE_REMATCH_ENTRY,
    &TITLE_MONITOR_EDIT_ENTRY,
    &COLLECTION_MONITOR_EDIT_ENTRY,
    &EPISODE_MONITOR_EDIT_ENTRY,
    &COLLECTION_EDIT_ENTRY,
    &EPISODE_EDIT_ENTRY,
    &SERIES_MOVIE_MONITOR_EDIT_ENTRY,
    &TITLE_MONITOR_SELECTION_ENTRY,
    &TITLE_TAG_EDIT_ENTRY,
    &SERIES_MOVIE_TAG_EDIT_ENTRY,
    &TITLE_HYDRATION_ENTRY,
    &TITLE_DOWNLOAD_ENTRY,
    &LIBRARY_SCAN_ENTRY,
    &COMPLETED_IMPORT_ENTRY,
    &MANUAL_IMPORT_ENTRY,
    &RENAME_APPLY_ENTRY,
    &TITLE_DELETE_ENTRY,
    &TITLE_DELETE_JOB_ENTRY,
    &MEDIA_FILE_DELETE_ENTRY,
    &MEDIA_FILE_PRIMARY_ENTRY,
    &LIBRARY_ROOTS_UPDATE_ENTRY,
    &LIBRARY_DELETE_ENTRY,
    &RECYCLE_RESTORE_ENTRY,
    &FOLDER_MATCH_ENTRY,
    &MAINTENANCE_DELETE_ENTRY,
    &MAINTENANCE_ACTION_ENTRY,
];

/// Actions that need no registered entry point because exclusivity is enforced
/// elsewhere.
///
/// [`GuardedAction::LocationOperation`] is the only one: two operations cannot
/// hold the same entity because
/// [`crate::ports::LocationOperationRepository::claim_location_operation_ownership`]
/// claims all-or-nothing behind the store's partial unique index on unreleased
/// claims, and reports the loser's conflicts directly.
pub const ACTIONS_GUARDED_BY_CLAIM: &[GuardedAction] = &[GuardedAction::LocationOperation];

/// Every action the guard knows about; kept beside the enum so the audit test
/// notices a variant nobody registered an entry point for.
pub const ALL_GUARDED_ACTIONS: &[GuardedAction] = &[
    GuardedAction::TitleEdit,
    GuardedAction::Download,
    GuardedAction::LibraryScan,
    GuardedAction::Import,
    GuardedAction::Rename,
    GuardedAction::TitleDelete,
    GuardedAction::MediaFileMutation,
    GuardedAction::LocationOperation,
    GuardedAction::RootConfiguration,
    GuardedAction::MaintenanceJob,
    GuardedAction::FolderMatch,
];

/// The choke point (D7): the one place that answers "may this action proceed
/// against these entities?".
pub struct LocationOwnershipGuard<'a> {
    store: &'a dyn crate::ports::LocationOperationRepository,
    registry: Option<&'a LocationOwnershipRegistry>,
}

impl<'a> LocationOwnershipGuard<'a> {
    pub fn new(
        store: &'a dyn crate::ports::LocationOperationRepository,
        registry: &'a LocationOwnershipRegistry,
    ) -> Self {
        Self {
            store,
            registry: Some(registry),
        }
    }

    /// Persisted claims only — for callers with no runtime state at hand.
    pub fn persisted_only(store: &'a dyn crate::ports::LocationOperationRepository) -> Self {
        Self {
            store,
            registry: None,
        }
    }

    /// The typed answer. `Ok(None)` means nothing owns any of `entities`.
    ///
    /// Entities are checked individually, so an operation on an unrelated title
    /// or root never denies: only an actual overlap does.
    pub async fn check(
        &self,
        entry: &'static GuardedEntry,
        entities: &[OwnedEntity],
    ) -> AppResult<Option<LocationOwnershipDenied>> {
        if entities.is_empty() {
            return Ok(None);
        }

        let mut conflicts = Vec::new();
        for entity in entities {
            let holder = match self.registry.and_then(|registry| registry.holder(entity)) {
                Some(holder) => Some(holder),
                None => self.store.location_ownership_holder(entity).await?,
            };
            if let Some(operation_id) = holder {
                conflicts.push(OwnershipConflict {
                    operation_id,
                    entity: entity.clone(),
                    action: entry.action,
                });
            }
        }

        if conflicts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(LocationOwnershipDenied { entry, conflicts }))
        }
    }

    /// [`Self::check`] mapped onto the caller's error convention.
    pub async fn ensure_not_owned(
        &self,
        entry: &'static GuardedEntry,
        entities: &[OwnedEntity],
    ) -> AppResult<()> {
        match self.check(entry, entities).await? {
            None => Ok(()),
            Some(denied) => Err(denied.into_app_error()),
        }
    }

    /// Every open claim, for facet-wide callers that cannot enumerate their own
    /// entities up front.
    pub async fn open_claims(&self) -> AppResult<Vec<crate::ports::LocationOwnershipClaim>> {
        self.store.list_location_ownership_claims().await
    }
}

impl crate::AppUseCase {
    fn location_ownership_guard(&self) -> LocationOwnershipGuard<'_> {
        LocationOwnershipGuard::new(
            self.services.library.location_operations.as_ref(),
            &self.runtime.library.location_ownership,
        )
    }

    /// Every open claim, for background work that cannot enumerate its own
    /// entities up front and only needs to *skip* what is owned rather than
    /// refuse (the full-hash backfill job, FR-047/SC-007).
    pub(crate) async fn location_ownership_open_claims(
        &self,
    ) -> AppResult<Vec<crate::ports::LocationOwnershipClaim>> {
        self.location_ownership_guard().open_claims().await
    }

    pub(crate) async fn location_owned_title_ids(&self) -> AppResult<HashSet<String>> {
        let mut ids = self
            .location_ownership_open_claims()
            .await?
            .into_iter()
            .filter_map(|claim| match claim.entity {
                OwnedEntity::Title(id) => Some(id),
                OwnedEntity::Root(_) => None,
            })
            .collect::<HashSet<_>>();
        if let Ok(claims) = self.runtime.library.location_ownership.claims.read() {
            ids.extend(claims.keys().filter_map(|entity| match entity {
                OwnedEntity::Title(id) => Some(id.clone()),
                OwnedEntity::Root(_) => None,
            }));
        }
        Ok(ids)
    }

    /// The choke point every mutating entry point calls (FR-084). `entry` names
    /// the [`GuardedEntry`] constant registered for this call site.
    pub(crate) async fn ensure_location_ownership_allows(
        &self,
        entry: &'static GuardedEntry,
        entities: &[OwnedEntity],
    ) -> AppResult<()> {
        self.location_ownership_guard()
            .ensure_not_owned(entry, entities)
            .await
    }

    /// Hold through the mutation, closing the gap between checking ownership
    /// and changing a title. A move drains these leases before taking claims.
    pub(crate) async fn acquire_location_title_mutation(
        &self,
        entry: &'static GuardedEntry,
        title_id: &str,
    ) -> AppResult<tokio::sync::OwnedRwLockReadGuard<()>> {
        let guard = self
            .runtime
            .library
            .location_ownership
            .admit_title_mutation(title_id)?;
        if let Some(denied) = self
            .location_ownership_denial_for_title(entry, title_id)
            .await?
        {
            return Err(AppError::LocationOperationBusy(denied.message()));
        }
        Ok(guard)
    }

    /// Title-scoped shorthand.
    pub(crate) async fn ensure_location_ownership_allows_title(
        &self,
        entry: &'static GuardedEntry,
        title_id: &str,
    ) -> AppResult<()> {
        self.ensure_location_ownership_allows(entry, &[OwnedEntity::Title(title_id.to_string())])
            .await
    }

    /// Title-scoped probe for a caller that holds rather than refuses.
    /// `Ok(None)` means nothing owns the title.
    pub(crate) async fn location_ownership_denial_for_title(
        &self,
        entry: &'static GuardedEntry,
        title_id: &str,
    ) -> AppResult<Option<LocationOwnershipDenied>> {
        self.location_ownership_guard()
            .check(entry, &[OwnedEntity::Title(title_id.to_string())])
            .await
    }

    /// Root-scoped shorthand covering every root a library configures.
    pub(crate) async fn ensure_location_ownership_allows_library_roots(
        &self,
        entry: &'static GuardedEntry,
        library_id: &str,
    ) -> AppResult<()> {
        let Some(library) = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
        else {
            return Ok(());
        };
        let entities = library
            .roots
            .iter()
            .map(|root| OwnedEntity::Root(root.id.clone()))
            .collect::<Vec<_>>();
        self.ensure_location_ownership_allows(entry, &entities)
            .await
    }

    /// Facet-wide shorthand for callers whose plan names no ids (bulk rename).
    ///
    /// Resolves the open claims instead of the caller's entities, then keeps
    /// only the ones inside this facet, so an operation on another facet's
    /// library never blocks the work.
    pub(crate) async fn ensure_location_ownership_allows_facet(
        &self,
        entry: &'static GuardedEntry,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<()> {
        let claims = self.location_ownership_guard().open_claims().await?;
        if claims.is_empty() {
            return Ok(());
        }

        let mut facet_root_ids = std::collections::HashSet::new();
        for library in self
            .services
            .catalog
            .libraries
            .list(Some(facet.clone()))
            .await?
        {
            for root in &library.roots {
                facet_root_ids.insert(root.id.clone());
            }
        }

        let mut overlapping = Vec::new();
        for claim in claims {
            let in_facet = match &claim.entity {
                OwnedEntity::Root(root_id) => facet_root_ids.contains(root_id),
                OwnedEntity::Title(title_id) => self
                    .services
                    .catalog
                    .titles
                    .get_by_id(title_id)
                    .await?
                    .is_some_and(|title| title.facet == *facet),
            };
            if in_facet {
                overlapping.push(claim.entity);
            }
        }

        self.ensure_location_ownership_allows(entry, &overlapping)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn orphaned_queued_row_backs_off_and_completion_wakes_waiter() {
        use crate::location::model::{
            LocationExecutionMode, LocationOperationState, LocationOperationType, VerificationDepth,
        };
        use crate::location::test_support::{InMemoryLocationOperationStore, queued_operation};
        let store = InMemoryLocationOperationStore::new();
        let mut first = queued_operation(
            "first",
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            VerificationDepth::Full,
        );
        let mut second = first.clone();
        second.id = "second".into();
        second.created_at += chrono::Duration::seconds(1);
        store.insert_operation(first.clone());
        store.insert_operation(second);
        let registry = LocationRunnerRegistry::new();
        let waiting = registry.wait_for_turn(&store, "second");
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(150), &mut waiting)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .active_reads
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        first.state = LocationOperationState::Completed;
        store.insert_operation(first);
        // Model a runner picked up by recovery and then completing.
        drop(registry.begin("first").unwrap());
        let _slot = tokio::time::timeout(std::time::Duration::from_millis(250), waiting)
            .await
            .expect("completion wakes without waiting for backoff")
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn stranded_queued_head_is_failed_with_a_reason_once_boot_resume_settled() {
        use crate::location::model::{
            LocationExecutionMode, LocationOperationState, LocationOperationType, VerificationDepth,
        };
        use crate::location::test_support::{InMemoryLocationOperationStore, queued_operation};
        let store = InMemoryLocationOperationStore::new();
        let mut first = queued_operation(
            "first",
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            VerificationDepth::Full,
        );
        first.created_at -= chrono::Duration::minutes(2);
        // Nothing has written to a stranded row since it was created.
        first.updated_at = first.created_at;
        first.job_run_id = Some("run-first".into());
        let mut second = first.clone();
        second.id = "second".into();
        second.created_at = chrono::Utc::now();
        second.job_run_id = None;
        store.insert_operation(first);
        store.insert_operation(second);
        let registry = LocationRunnerRegistry::new();
        let waiting = registry.wait_for_turn(&store, "second");
        tokio::pin!(waiting);
        // Until the boot resume has spawned its runners, a Queued row nobody
        // is running is simply not picked up yet.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(150), &mut waiting)
                .await
                .is_err()
        );
        assert!(registry.take_stranded().is_empty());
        assert_eq!(
            store
                .get_location_operation("first")
                .await
                .unwrap()
                .unwrap()
                .state,
            LocationOperationState::Queued
        );
        registry.mark_boot_resume_settled();
        let _slot = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .expect("the stranded head is failed and the queue moves")
            .unwrap();
        let failed = store
            .get_location_operation("first")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.state, LocationOperationState::Failed);
        assert_eq!(failed.detail.as_deref(), Some(STRANDED_DETAIL));
        assert!(failed.completed_at.is_some());
        let stranded = registry.take_stranded();
        assert_eq!(stranded.len(), 1);
        assert_eq!(stranded[0].operation_id, "first");
        assert_eq!(stranded[0].job_run_id.as_deref(), Some("run-first"));
        assert!(registry.take_stranded().is_empty());
    }

    /// A retry reopens an old row: its `created_at` is long past, its
    /// `updated_at` is the reopen. The sweep gives it the same head start a
    /// fresh row gets, and when a retry does strand, the row says so in
    /// retry terms rather than claiming nothing ever moved.
    #[tokio::test(start_paused = true)]
    async fn a_reopened_head_gets_a_fresh_grace_and_a_stranded_retry_says_so() {
        use crate::location::model::{
            LocationExecutionMode, LocationOperationState, LocationOperationType, VerificationDepth,
        };
        use crate::location::test_support::{InMemoryLocationOperationStore, queued_operation};
        let store = InMemoryLocationOperationStore::new();
        let mut reopened = queued_operation(
            "reopened",
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            VerificationDepth::Full,
        );
        reopened.created_at -= chrono::Duration::minutes(3);
        reopened.started_at = Some(reopened.created_at);
        reopened.updated_at = chrono::Utc::now();
        let mut second = reopened.clone();
        second.id = "second".into();
        second.created_at = chrono::Utc::now();
        second.started_at = None;
        store.insert_operation(reopened.clone());
        store.insert_operation(second);
        let registry = LocationRunnerRegistry::new();
        registry.mark_boot_resume_settled();
        let waiting = registry.wait_for_turn(&store, "second");
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(150), &mut waiting)
                .await
                .is_err(),
            "a just-reopened head is waiting for its runner, not stranded"
        );
        assert!(registry.take_stranded().is_empty());
        assert_eq!(
            store
                .get_location_operation("reopened")
                .await
                .unwrap()
                .unwrap()
                .state,
            LocationOperationState::Queued
        );

        // The reopen is now old too: the retry's runner never came.
        reopened.updated_at -= chrono::Duration::minutes(2);
        store.insert_operation(reopened);
        let _slot = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .expect("the stranded retry is failed and the queue moves")
            .unwrap();
        let failed = store
            .get_location_operation("reopened")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.state, LocationOperationState::Failed);
        assert_eq!(failed.detail.as_deref(), Some(STRANDED_RETRY_DETAIL));
        assert_eq!(
            failed.reason_code,
            Some(crate::location::model::LocationReasonCode::Stranded)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn fresh_or_live_queued_heads_are_not_stranded() {
        use crate::location::model::{
            LocationExecutionMode, LocationOperationState, LocationOperationType, VerificationDepth,
        };
        use crate::location::test_support::{InMemoryLocationOperationStore, queued_operation};
        for live in [false, true] {
            let store = InMemoryLocationOperationStore::new();
            let mut first = queued_operation(
                "first",
                LocationOperationType::RootMove,
                LocationExecutionMode::MoveWithScryer,
                VerificationDepth::Full,
            );
            if live {
                // Old enough to be stranded, but somebody is running it.
                first.created_at -= chrono::Duration::minutes(2);
                first.updated_at = first.created_at;
            }
            let mut second = first.clone();
            second.id = "second".into();
            second.created_at = chrono::Utc::now();
            store.insert_operation(first);
            store.insert_operation(second);
            let registry = LocationRunnerRegistry::new();
            registry.mark_boot_resume_settled();
            let _first_live = live.then(|| registry.begin("first").unwrap());
            let waiting = registry.wait_for_turn(&store, "second");
            tokio::pin!(waiting);
            assert!(
                tokio::time::timeout(std::time::Duration::from_secs(5), &mut waiting)
                    .await
                    .is_err(),
                "live = {live}"
            );
            assert_eq!(
                store
                    .get_location_operation("first")
                    .await
                    .unwrap()
                    .unwrap()
                    .state,
                LocationOperationState::Queued,
                "live = {live}"
            );
            assert!(registry.take_stranded().is_empty());
        }
    }

    #[tokio::test]
    async fn transfer_dispatch_is_fifo_and_holds_one_instance_slot() {
        use crate::location::model::{
            LocationExecutionMode, LocationOperationState, LocationOperationType, VerificationDepth,
        };
        use crate::location::test_support::{InMemoryLocationOperationStore, queued_operation};
        let store = InMemoryLocationOperationStore::new();
        let mut first = queued_operation(
            "first",
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
            VerificationDepth::Full,
        );
        let mut second = first.clone();
        second.id = "second".into();
        second.created_at += chrono::Duration::seconds(1);
        store.insert_operation(first.clone());
        store.insert_operation(second);
        let registry = LocationRunnerRegistry::new();
        let _first_live = registry.begin("first").unwrap();
        let _second_live = registry.begin("second").unwrap();
        let second_wait = registry.wait_for_turn(&store, "second");
        tokio::pin!(second_wait);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut second_wait)
                .await
                .is_err()
        );
        let first_slot = registry.wait_for_turn(&store, "first").await.unwrap();
        first.state = LocationOperationState::Completed;
        store.insert_operation(first);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut second_wait)
                .await
                .is_err()
        );
        drop(first_slot);
        drop(_first_live);
        let _second_slot = tokio::time::timeout(std::time::Duration::from_secs(1), second_wait)
            .await
            .unwrap()
            .unwrap();
    }
    use crate::ports::{
        LocationOperationRepository, LocationOwnershipClaim, LocationOwnershipOutcome,
    };
    use async_trait::async_trait;
    use chrono::Utc;

    /// Source of every module that registers a guarded entry point. The audit
    /// test reads these to prove a declared entry is actually wired.
    const GUARDED_ENTRY_SOURCES: &[(&str, &str)] = &[
        (
            "catalog/workflow/collections.rs",
            include_str!("../catalog/workflow/collections.rs"),
        ),
        (
            "catalog/workflow/monitoring.rs",
            include_str!("../catalog/workflow/monitoring.rs"),
        ),
        (
            "catalog/workflow/title_tags.rs",
            include_str!("../catalog/workflow/title_tags.rs"),
        ),
        (
            "catalog/workflow/hydration.rs",
            include_str!("../catalog/workflow/hydration.rs"),
        ),
        (
            "acquisition/submission.rs",
            include_str!("../acquisition/submission.rs"),
        ),
        ("location/folder_match.rs", include_str!("folder_match.rs")),
        ("library/library.rs", include_str!("../library/library.rs")),
        ("library/rename.rs", include_str!("../library/rename.rs")),
        (
            "import/workflow/series_movie.rs",
            include_str!("../import/workflow/series_movie.rs"),
        ),
        (
            "import/workflow/manual.rs",
            include_str!("../import/workflow/manual.rs"),
        ),
        (
            "catalog/workflow/delete.rs",
            include_str!("../catalog/workflow/delete.rs"),
        ),
        (
            "maintenance_rules/action_execution.rs",
            include_str!("../maintenance_rules/action_execution.rs"),
        ),
        (
            "catalog/workflow/metadata.rs",
            include_str!("../catalog/workflow/metadata.rs"),
        ),
        (
            "jobs/housekeeping.rs",
            include_str!("../jobs/housekeeping.rs"),
        ),
    ];

    #[derive(Default)]
    struct FakeOwnershipStore {
        claims: HashMap<OwnedEntity, String>,
    }

    impl FakeOwnershipStore {
        fn owning(entries: &[(OwnedEntity, &str)]) -> Self {
            Self {
                claims: entries
                    .iter()
                    .map(|(entity, operation_id)| (entity.clone(), (*operation_id).to_string()))
                    .collect(),
            }
        }
    }

    #[async_trait]
    impl LocationOperationRepository for FakeOwnershipStore {
        async fn create_location_operation(
            &self,
            _operation: &crate::location::model::LocationOperation,
            _plan_json: Option<&str>,
        ) -> AppResult<()> {
            unimplemented!("guard tests only read ownership")
        }

        async fn get_location_operation(
            &self,
            _operation_id: &str,
        ) -> AppResult<Option<crate::location::model::LocationOperation>> {
            unimplemented!("guard tests only read ownership")
        }

        async fn get_location_operation_plan_json(
            &self,
            _operation_id: &str,
        ) -> AppResult<Option<String>> {
            unimplemented!("guard tests only read ownership")
        }

        async fn list_active_location_operations(
            &self,
        ) -> AppResult<Vec<crate::location::model::LocationOperation>> {
            Ok(Vec::new())
        }

        async fn update_location_operation_progress(
            &self,
            _progress: &crate::ports::LocationOperationProgress,
        ) -> AppResult<()> {
            unimplemented!("guard tests only read ownership")
        }

        async fn set_location_operation_job_run(
            &self,
            _operation_id: &str,
            _job_run_id: &str,
        ) -> AppResult<()> {
            unimplemented!("guard tests only read ownership")
        }

        async fn request_location_operation_cancel(&self, _operation_id: &str) -> AppResult<bool> {
            unimplemented!("guard tests only read ownership")
        }

        async fn reopen_location_operation(&self, _operation_id: &str) -> AppResult<bool> {
            unimplemented!("guard tests only read ownership")
        }

        async fn location_operation_cancel_requested(
            &self,
            _operation_id: &str,
        ) -> AppResult<bool> {
            Ok(false)
        }

        async fn upsert_location_title_checkpoint(
            &self,
            _checkpoint: &crate::location::model::TitleCheckpoint,
        ) -> AppResult<()> {
            unimplemented!("guard tests only read ownership")
        }

        async fn list_location_title_checkpoints(
            &self,
            _operation_id: &str,
        ) -> AppResult<Vec<crate::location::model::TitleCheckpoint>> {
            Ok(Vec::new())
        }

        async fn record_location_file_verification(
            &self,
            _record: &crate::location::model::FileVerificationRecord,
        ) -> AppResult<()> {
            unimplemented!("guard tests only read ownership")
        }

        async fn list_location_file_verifications(
            &self,
            _operation_id: &str,
            _title_id: Option<&str>,
        ) -> AppResult<Vec<crate::location::model::FileVerificationRecord>> {
            Ok(Vec::new())
        }

        async fn verified_destination_paths(
            &self,
            _operation_id: &str,
            _title_id: &str,
        ) -> AppResult<std::collections::BTreeSet<String>> {
            Ok(std::collections::BTreeSet::new())
        }

        async fn claim_location_operation_ownership(
            &self,
            _operation_id: &str,
            _entities: &[OwnedEntity],
        ) -> AppResult<LocationOwnershipOutcome> {
            unimplemented!("guard tests only read ownership")
        }

        async fn release_location_operation_ownership(
            &self,
            _operation_id: &str,
        ) -> AppResult<u64> {
            Ok(0)
        }

        async fn location_ownership_holder(
            &self,
            entity: &OwnedEntity,
        ) -> AppResult<Option<String>> {
            Ok(self.claims.get(entity).cloned())
        }

        async fn list_location_ownership_claims(&self) -> AppResult<Vec<LocationOwnershipClaim>> {
            Ok(self
                .claims
                .iter()
                .map(|(entity, operation_id)| LocationOwnershipClaim {
                    operation_id: operation_id.clone(),
                    entity: entity.clone(),
                    acquired_at: Utc::now(),
                })
                .collect())
        }
    }

    fn title(id: &str) -> OwnedEntity {
        OwnedEntity::Title(id.to_string())
    }

    fn root(id: &str) -> OwnedEntity {
        OwnedEntity::Root(id.to_string())
    }

    #[tokio::test]
    async fn persisted_claim_denies_an_overlapping_entity() {
        let store = FakeOwnershipStore::owning(&[(title("title-1"), "op-1")]);
        let guard = LocationOwnershipGuard::persisted_only(&store);

        let denied = guard
            .check(&TITLE_DELETE_ENTRY, &[title("title-1")])
            .await
            .expect("guard query")
            .expect("overlapping entity must be denied");

        assert_eq!(denied.action(), GuardedAction::TitleDelete);
        assert_eq!(denied.holding_operation_ids(), vec!["op-1".to_string()]);
        assert!(denied.message().contains("op-1"));
        assert!(denied.message().contains("title-1"));
    }

    #[tokio::test]
    async fn title_mutation_drain_waits_for_admitted_work_and_preserves_other_titles() {
        let registry = LocationOwnershipRegistry::new();
        let mutation = registry.admit_title_mutation("moving").unwrap();
        let entities = [title("moving")];
        let drain = registry.drain_title_mutations(&entities);
        tokio::pin!(drain);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut drain)
                .await
                .is_err()
        );
        assert!(registry.admit_title_mutation("moving").is_err());
        assert!(registry.admit_title_mutation("unrelated").is_ok());
        drop(mutation);
        let exclusive = drain.await;
        registry.claim_all("move", &entities);
        drop(exclusive);
        let store = FakeOwnershipStore::default();
        let guard = LocationOwnershipGuard::new(&store, &registry);
        for entry in [
            &TITLE_METADATA_EDIT_ENTRY,
            &EPISODE_EDIT_ENTRY,
            &TITLE_DOWNLOAD_ENTRY,
        ] {
            assert!(guard.check(entry, &entities).await.unwrap().is_some());
        }
        registry.release_operation("move");
        assert!(
            guard
                .check(&TITLE_DOWNLOAD_ENTRY, &entities)
                .await
                .unwrap()
                .is_none()
        );
        assert!(registry.admit_title_mutation("moving").is_ok());
    }

    #[tokio::test]
    async fn persisted_title_locks_block_edits_and_downloads_after_restart() {
        let (mut app, _) = crate::lib_tests::bootstrap();
        app.services.library.location_operations =
            Arc::new(FakeOwnershipStore::owning(&[(title("moving"), "move")]));
        for entry in [
            &TITLE_METADATA_EDIT_ENTRY,
            &EPISODE_EDIT_ENTRY,
            &TITLE_DOWNLOAD_ENTRY,
        ] {
            let error = app
                .acquire_location_title_mutation(entry, "moving")
                .await
                .unwrap_err();
            assert!(matches!(error, AppError::LocationOperationBusy(_)));
            assert!(error.to_string().contains("move"));
        }
        assert!(
            app.acquire_location_title_mutation(&TITLE_DOWNLOAD_ENTRY, "unrelated")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn disjoint_entities_pass_through() {
        let store =
            FakeOwnershipStore::owning(&[(title("title-1"), "op-1"), (root("root-1"), "op-1")]);
        let guard = LocationOwnershipGuard::persisted_only(&store);

        guard
            .ensure_not_owned(&TITLE_DELETE_ENTRY, &[title("title-2")])
            .await
            .expect("an unrelated title must not be blocked");
        guard
            .ensure_not_owned(&LIBRARY_SCAN_ENTRY, &[root("root-2")])
            .await
            .expect("an unrelated root must not be blocked");
    }

    #[tokio::test]
    async fn released_claims_stop_denying() {
        let owned = FakeOwnershipStore::owning(&[(title("title-1"), "op-1")]);
        LocationOwnershipGuard::persisted_only(&owned)
            .ensure_not_owned(&RENAME_APPLY_ENTRY, &[title("title-1")])
            .await
            .expect_err("an owned title must be refused");

        // Release drops the row; the same query now answers "nothing owns it".
        let released = FakeOwnershipStore::default();
        LocationOwnershipGuard::persisted_only(&released)
            .ensure_not_owned(&RENAME_APPLY_ENTRY, &[title("title-1")])
            .await
            .expect("a released claim must stop denying");
    }

    #[tokio::test]
    async fn in_process_registry_denies_before_the_store_is_queried() {
        // The store knows nothing; only the in-process registry does.
        let store = FakeOwnershipStore::default();
        let registry = LocationOwnershipRegistry::new();
        registry.claim_all("op-7", &[title("title-9")]);
        let guard = LocationOwnershipGuard::new(&store, &registry);

        let denied = guard
            .check(&LIBRARY_SCAN_ENTRY, &[title("title-9")])
            .await
            .expect("guard query")
            .expect("the in-process fast path must deny");
        assert_eq!(denied.holding_operation_ids(), vec!["op-7".to_string()]);

        registry.release_operation("op-7");
        assert!(registry.is_empty());
        guard
            .ensure_not_owned(&LIBRARY_SCAN_ENTRY, &[title("title-9")])
            .await
            .expect("a released registry claim must stop denying");
    }

    #[tokio::test]
    async fn an_empty_entity_list_never_denies() {
        let store = FakeOwnershipStore::owning(&[(title("title-1"), "op-1")]);
        LocationOwnershipGuard::persisted_only(&store)
            .ensure_not_owned(&LIBRARY_ROOTS_UPDATE_ENTRY, &[])
            .await
            .expect("an entry point with no entities has nothing to overlap");
    }

    #[tokio::test]
    async fn every_conflicting_entity_is_reported() {
        let store =
            FakeOwnershipStore::owning(&[(title("title-1"), "op-1"), (root("root-1"), "op-2")]);
        let denied = LocationOwnershipGuard::persisted_only(&store)
            .check(
                &LIBRARY_DELETE_ENTRY,
                &[title("title-1"), title("title-2"), root("root-1")],
            )
            .await
            .expect("guard query")
            .expect("two owned entities must be denied");

        assert_eq!(denied.conflicts.len(), 2);
        assert_eq!(
            denied.holding_operation_ids(),
            vec!["op-1".to_string(), "op-2".to_string()]
        );
    }

    /// The plan's risk mitigation: the declared entry list and the wiring must
    /// not drift. A new mutating entry point registers a [`GuardedEntry`] here
    /// and calls the choke point with it; this test fails loudly otherwise.
    #[test]
    fn guarded_entries_are_wired() {
        for entry in GUARDED_ENTRIES {
            let source = GUARDED_ENTRY_SOURCES
                .iter()
                .find(|(module, _)| *module == entry.module)
                .map(|(_, source)| *source)
                .unwrap_or_else(|| {
                    panic!(
                        "guarded entry {} names module {} which GUARDED_ENTRY_SOURCES does not include; add an include_str! for it",
                        entry.constant, entry.module
                    )
                });

            assert!(
                source.contains(&format!("fn {}(", entry.function)),
                "guarded entry {} names {}::{}, which no longer exists",
                entry.constant,
                entry.module,
                entry.function
            );
            assert!(
                source.contains(entry.constant),
                "guarded entry {} is declared but {} never references it: the entry point is unguarded",
                entry.constant,
                entry.module
            );
        }
    }

    #[test]
    fn every_guarded_action_has_an_entry_point() {
        for action in ALL_GUARDED_ACTIONS {
            if ACTIONS_GUARDED_BY_CLAIM.contains(action) {
                assert!(
                    !GUARDED_ENTRIES.iter().any(|entry| entry.action == *action),
                    "{} is documented as guarded by the claim itself but also registers an entry point",
                    action.as_str()
                );
                continue;
            }
            assert!(
                GUARDED_ENTRIES.iter().any(|entry| entry.action == *action),
                "{} has no registered entry point; register one in GUARDED_ENTRIES or document it in ACTIONS_GUARDED_BY_CLAIM",
                action.as_str()
            );
        }
    }

    #[test]
    fn guarded_entry_constants_are_named_after_themselves() {
        for entry in GUARDED_ENTRIES {
            assert!(
                include_str!("ownership_guard.rs")
                    .contains(&format!("pub const {}: GuardedEntry", entry.constant)),
                "{} does not name its own constant",
                entry.constant
            );
        }
    }
}
