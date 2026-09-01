//! The root-move execution path: the three runner seams, wired end to end
//! (US2, FR-030–033, FR-044, FR-076, FR-089).
//!
//! [`crate::location::executor::LocationOperationRunner`] owns the state
//! machine, the checkpoints, the cancel points, and the resume cursor. This
//! module supplies what it deliberately does not know:
//!
//! | Seam | Implementation here | What it guarantees |
//! |---|---|---|
//! | [`TitleFileMover`] | [`RootMoveFileMover`] | Same-filesystem rename with no verification pass (FR-032); otherwise a verified streaming copy at the operation's depth (FR-040–043), or a proof of a destination an interrupted run already placed. Configured permissions land on the destination straight after verification (FR-031). |
//! | [`TitleReconciler`] | [`RootMoveReconciler`] | Catalog ownership flips only after every planned file for the title is verified; then sources are recycled and *empty* source directories removed, in that order (FR-031, FR-044). |
//! | [`TitleAdmissionCheck`] | [`RootMoveAdmission`] | The FR-089 scope rule: a changed catalog input or an unprocessed source that vanished is stale; this operation's own partial destination content is resumable. |
//!
//! # Why the source is never touched by the mover
//!
//! A cross-volume move copies. Removing the source is [`RootMoveReconciler`]'s
//! cleanup step, and it reads the *persisted verification records* to decide —
//! not the absence of an error. A file whose record says anything other than
//! `verified` keeps its source, which is what makes SC-006 (a byte flipped
//! after write) end with both copies intact rather than with a corrupted
//! survivor.
//!
//! # Why cleanup is idempotent
//!
//! Resume re-enters a title at its checkpoint, so cleanup can run twice for the
//! same file. Every step here treats "already gone" as success: a missing
//! source is a completed removal, and a non-empty directory is simply left
//! alone.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::location::executor::{
    FileMoveRequest, PlannedTitle, TitleAdmission, TitleAdmissionCheck, TitleAdmissionContext,
    TitleFileMover, TitleReconciler, TitleStepOutcome,
};
use crate::location::model::{LocationOperation, VerificationDepth};
use crate::location::preview::PlanInputChange;
use crate::location::root_move::{RootMoveExecutionPlan, RootMoveTitleExecution};
use crate::location::verify::{
    DestinationClaim, VerifiedCopier, VerifiedCopyRequest, VerifiedFile, same_filesystem,
};
use crate::ports::LocationOperationRepository;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppError, AppResult};

// ── Seam: configured destination permissions (FR-031) ───────────────────────

/// Applies the operator's configured file and folder permissions to content a
/// location operation just placed.
///
/// A separate seam because only the settings layer can read the configuration,
/// and only the infrastructure layer can chown/chmod. Both halves are supplied
/// by the composition root; the runner path stays testable without touching
/// real modes.
#[async_trait]
pub trait PlacedContentPermissions: Send + Sync {
    async fn apply_to_file(&self, path: &Path) -> AppResult<()>;
    async fn apply_to_directory(&self, path: &Path) -> AppResult<()>;
}

/// Applies nothing. The right default for a workflow with no configured
/// permissions: a move must not change access an operator did not configure.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPlacedContentPermissions;

#[async_trait]
impl PlacedContentPermissions for NoPlacedContentPermissions {
    async fn apply_to_file(&self, _path: &Path) -> AppResult<()> {
        Ok(())
    }

    async fn apply_to_directory(&self, _path: &Path) -> AppResult<()> {
        Ok(())
    }
}

/// The production applier: the operator's resolved
/// [`crate::ports::ImportFilePermissions`] put on placed content through the
/// same file-importer seam imports use, so a moved file ends up with exactly
/// the modes an imported one would.
pub struct ImportFilePermissionsApplier {
    importer: Arc<dyn crate::ports::FileImporter>,
    permissions: crate::ports::ImportFilePermissions,
}

impl ImportFilePermissionsApplier {
    pub fn new(
        importer: Arc<dyn crate::ports::FileImporter>,
        permissions: crate::ports::ImportFilePermissions,
    ) -> Self {
        Self {
            importer,
            permissions,
        }
    }
}

#[async_trait]
impl PlacedContentPermissions for ImportFilePermissionsApplier {
    async fn apply_to_file(&self, path: &Path) -> AppResult<()> {
        self.importer
            .apply_placed_file_permissions(path, &self.permissions)
            .await
    }

    async fn apply_to_directory(&self, path: &Path) -> AppResult<()> {
        self.importer
            .apply_placed_directory_permissions(path, &self.permissions)
            .await
    }
}

// ── Seam: catalog writes and reads ───────────────────────────────────────────

/// Where a title's content sits according to the catalog, as the admission
/// check compares it against the confirmed plan (FR-089).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitlePlacementSnapshot {
    pub root_folder_id: String,
    pub library_id: String,
    /// Stored folder path, or `None` when the title owns no folder.
    pub folder_path: Option<String>,
    /// Stored paths of every tracked media file, sorted.
    pub media_file_paths: BTreeSet<String>,
}

/// The catalog operations a root move performs, behind a seam so the executor
/// path can be exercised against temp directories and a fake catalog.
#[async_trait]
pub trait RootMoveCatalog: Send + Sync {
    /// Where the catalog currently says the title lives. `None` when the title
    /// no longer exists.
    async fn title_placement(&self, title_id: &str) -> AppResult<Option<TitlePlacementSnapshot>>;

    /// Point a tracked media file at its new path.
    async fn set_media_file_path(&self, media_file_id: &str, stored_path: &str) -> AppResult<()>;

    /// Assign the folder the title now owns.
    async fn set_title_folder_path(&self, title_id: &str, stored_path: &str) -> AppResult<()>;

    /// Update the title's root reference (FR-078 synthetic id).
    async fn set_title_root(&self, title_id: &str, root_folder_id: &str) -> AppResult<()>;

    /// Transfer the title into another library, together with the root it lands
    /// on, in **one** transaction (FR-056).
    ///
    /// The two halves cannot be written separately: a title whose `library_id`
    /// says library B while its `root_folder_id` names a root of library A is a
    /// catalog state nothing else in Scryer knows how to read, and an
    /// interruption between two statements would leave exactly that. The root
    /// move's own flip is three ordered statements precisely because each of
    /// them is individually valid; this one is not, so it is atomic instead.
    ///
    /// Implementations must also carry the title's library projections across —
    /// `title_external_ids.library_id` above all, since that column is what
    /// destination-title detection reads (FR-055). A title transferred without
    /// it would keep answering identity lookups from the library it left.
    async fn set_title_library_and_root(
        &self,
        title_id: &str,
        library_id: &str,
        root_folder_id: &str,
    ) -> AppResult<()>;

    /// Persist the hashes the copy pass produced onto the moved media file
    /// (FR-041, migration 0205).
    ///
    /// The operation's own per-file record (0206) is the audit trail; this is
    /// the *read model* the dedup gate and the backfill queue consult. Without
    /// it a verified move would hand its expensive hash to an audit table and
    /// leave the media file queued for the backfill job to read all over again.
    ///
    /// Defaulted to a no-op so a catalog that only exercises placement does not
    /// have to care.
    async fn set_media_file_content_hashes(
        &self,
        media_file_id: &str,
        hashes: &crate::location::model::PersistedContentHashes,
    ) -> AppResult<()> {
        let _ = (media_file_id, hashes);
        Ok(())
    }
}

// ── Seam: recycling ──────────────────────────────────────────────────────────

/// What happened to a source file the operation no longer needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDisposal {
    /// The source went to the recycle bin.
    Recycled,
    /// The source was already gone (a rename moved it, or a previous run
    /// removed it).
    AlreadyAbsent,
    /// Recycling was not available, so the verified-redundant source was
    /// removed. Always warned about, never silent (C3).
    RemovedRecycleUnavailable(&'static str),
}

/// Recycles source copies a verified destination has made redundant.
///
/// Recycling is a seam because the recycle bin's configuration is per media
/// root and lives in settings, and because the executor path has to be
/// testable without a configured bin.
#[async_trait]
pub trait SourceRecycler: Send + Sync {
    /// Recycle `source`, attributing the entry to `operation_id` and `title_id`
    /// so the recycle manifest links back to the operation.
    async fn recycle_source(
        &self,
        operation_id: &str,
        title: &RootMoveTitleExecution,
        source: &Path,
        media_file_id: Option<&str>,
        size_bytes: u64,
    ) -> AppResult<SourceDisposal>;
}

// ── TitleFileMover ───────────────────────────────────────────────────────────

/// Moves one file for a root move.
///
/// Same filesystem → an exclusive rename through [`crate::fs_safety`], recorded
/// as [`VerifiedFile::same_filesystem_rename`]: no bytes were copied, so there
/// is nothing a verification pass could compare (FR-032).
///
/// Different filesystem → [`VerifiedCopier::copy_and_verify`] at the
/// operation's configured depth. The source is left exactly where it is; only
/// [`RootMoveReconciler`] removes it, and only against a persisted verification
/// record (FR-044).
pub struct RootMoveFileMover {
    copier: VerifiedCopier,
    permissions: Arc<dyn PlacedContentPermissions>,
    /// Where the streamed hashes are persisted (FR-041, migration 0205).
    ///
    /// The mover is the only place that ever holds them, so it is the only
    /// place that can hand them to the read model. `None` for a mover wired
    /// without a catalog, in which case the backfill job picks the files up
    /// later — a missing hash costs a re-read, never correctness.
    catalog: Option<Arc<dyn RootMoveCatalog>>,
    /// Skip the FR-032 rename fast path and always copy.
    ///
    /// The mover decides rename-vs-copy from the actual device ids, so a test
    /// living inside one temp directory can never reach the copy path — which
    /// is the path with the partial-copy staging, the read-back, and the
    /// interrupted-destination proof on it. This is the only way to exercise
    /// those against real files; production never sets it.
    force_copies: bool,
}

impl RootMoveFileMover {
    pub fn new(copier: VerifiedCopier, permissions: Arc<dyn PlacedContentPermissions>) -> Self {
        Self {
            copier,
            permissions,
            catalog: None,
            force_copies: false,
        }
    }

    /// Take the cross-filesystem copy path whatever the devices say. Test-only;
    /// see [`RootMoveFileMover::force_copies`].
    #[cfg(test)]
    pub fn forcing_copies(mut self) -> Self {
        self.force_copies = true;
        self
    }

    /// Persist each copy's hashes onto its media file, so a moved file leaves
    /// the backfill queue instead of being read a second time (D4).
    pub fn with_catalog(mut self, catalog: Arc<dyn RootMoveCatalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// A mover that changes no permissions — the right shape for a deployment
    /// with no configured modes, and for tests.
    pub fn without_permissions() -> Self {
        Self::new(VerifiedCopier::new(), Arc::new(NoPlacedContentPermissions))
    }

    /// FR-041/D4: the hashes the copy already computed go onto the media file
    /// (0205) as well as into the operation's audit record (0206).
    ///
    /// Only for a verified destination — a hash of bytes that failed their
    /// check describes content we are about to refuse to trust. A
    /// same-filesystem rename carries no hashes at all (FR-032), and a
    /// companion asset has no media-file row.
    async fn persist_content_hashes(&self, request: &FileMoveRequest<'_>, verified: &VerifiedFile) {
        let Some(catalog) = self.catalog.as_ref() else {
            return;
        };
        if !verified.permits_source_removal() {
            return;
        }
        let (Some(media_file_id), Some(hashes)) = (
            request.file.media_file_id.as_deref(),
            verified.hashes.as_ref(),
        ) else {
            return;
        };

        let persisted =
            crate::location::model::PersistedContentHashes::from_streamed(hashes, chrono::Utc::now());
        if let Err(error) = catalog
            .set_media_file_content_hashes(media_file_id, &persisted)
            .await
        {
            // Not fatal: the bytes are placed and proven, and the backfill job
            // recomputes whatever did not land.
            tracing::warn!(
                error = %error,
                media_file_id,
                "failed to persist move content hashes; the backfill job will recompute them"
            );
        }
    }
}

#[async_trait]
impl TitleFileMover for RootMoveFileMover {
    async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
        let source = &request.file.source_path;
        let destination = &request.file.destination_path;

        if let Some(parent) = destination.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                AppError::Repository(format!(
                    "failed to create destination directory {}: {error}",
                    parent.display()
                ))
            })?;
            // The directory is destination content too, so the operator's
            // configured folder modes apply to it (FR-031).
            self.permissions.apply_to_directory(parent).await?;
        }

        let verified = if !self.force_copies && same_filesystem(source, destination).await {
            move_with_rename(source, destination, request.depth).await?
        } else if tokio::fs::symlink_metadata(destination).await.is_ok() {
            // The runner only asks for files it has no verification record for,
            // so a destination that is already there is the crash window
            // between a completed copy's promotion and the record that should
            // have followed it — or a file that appeared from outside. Either
            // way it is proven against the source rather than replaced or
            // trusted (FR-033, C4).
            self.copier
                .verify_existing_destination(source, destination, request.depth)
                .await?
        } else {
            self.copier
                .copy_and_verify(VerifiedCopyRequest {
                    source: source.clone(),
                    destination: destination.clone(),
                    depth: request.depth,
                    claim: DestinationClaim::ClaimHere,
                    progress: request.progress.clone(),
                })
                .await?
        };

        // Only a destination that is actually there gets its modes changed; a
        // mismatch is left byte-for-byte as written so it can be inspected.
        if verified.permits_source_removal() {
            self.permissions.apply_to_file(destination).await?;
        }
        self.persist_content_hashes(&request, &verified).await;
        Ok(verified)
    }
}

/// The FR-032 fast path: an exclusive rename that never replaces a destination
/// the caller did not ask to replace.
async fn move_with_rename(
    source: &Path,
    destination: &Path,
    depth: VerificationDepth,
) -> AppResult<VerifiedFile> {
    // A resumed run can find the rename already done: the source is gone and
    // the destination is there. That is this operation's own completed work,
    // not a collision (FR-089).
    if tokio::fs::symlink_metadata(source).await.is_err()
        && tokio::fs::symlink_metadata(destination).await.is_ok()
    {
        return Ok(VerifiedFile::same_filesystem_rename(
            source.to_path_buf(),
            destination.to_path_buf(),
            depth,
        ));
    }

    crate::fs_safety::move_file_exclusive(
        source,
        destination,
        crate::fs_safety::MoveOptions::default(),
    )
    .await
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to move {} to {}: {error}",
            source.display(),
            destination.display()
        ))
    })?;

    Ok(VerifiedFile::same_filesystem_rename(
        source.to_path_buf(),
        destination.to_path_buf(),
        depth,
    ))
}

// ── TitleReconciler ──────────────────────────────────────────────────────────

/// Flips the catalog and cleans up the source for a root move.
///
/// The runner has already proven every planned file for the title before this
/// runs (it refuses to continue otherwise), so `reconcile_title` never has to
/// re-check verification. `clean_up_title` does, because it is the step that
/// touches the source: it reads the persisted verification records and skips
/// any file that is not recorded `verified` (FR-044).
pub struct RootMoveReconciler<'a> {
    plan: &'a RootMoveExecutionPlan,
    catalog: &'a dyn RootMoveCatalog,
    store: &'a dyn LocationOperationRepository,
    recycler: &'a dyn SourceRecycler,
    permissions: Arc<dyn PlacedContentPermissions>,
}

impl<'a> RootMoveReconciler<'a> {
    pub fn new(
        plan: &'a RootMoveExecutionPlan,
        catalog: &'a dyn RootMoveCatalog,
        store: &'a dyn LocationOperationRepository,
        recycler: &'a dyn SourceRecycler,
    ) -> Self {
        Self {
            plan,
            catalog,
            store,
            recycler,
            permissions: Arc::new(NoPlacedContentPermissions),
        }
    }

    pub fn with_permissions(mut self, permissions: Arc<dyn PlacedContentPermissions>) -> Self {
        self.permissions = permissions;
        self
    }

    fn title(&self, title_id: &str) -> AppResult<&RootMoveTitleExecution> {
        self.plan.title(title_id).ok_or_else(|| {
            AppError::Validation(format!(
                "the confirmed plan has no instructions for title {title_id}"
            ))
        })
    }
}

#[async_trait]
impl TitleReconciler for RootMoveReconciler<'_> {
    async fn reconcile_title(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
    ) -> AppResult<TitleStepOutcome> {
        let planned = self.title(&title.title_id)?;
        let mut outcome = TitleStepOutcome::clean();

        // Destination directories are destination content: the operator's
        // configured folder modes apply before the catalog points at them
        // (FR-031).
        if let Some(folder) = planned.destination_folder_path.as_deref() {
            let folder = stored_path_to_path_buf(folder);
            if tokio::fs::symlink_metadata(&folder).await.is_ok() {
                self.permissions.apply_to_directory(&folder).await?;
            }
        }

        // Media-file paths first, then the folder, then the root: each step is
        // individually correct, and an interruption between them leaves rows
        // pointing at real files rather than at a folder nothing lives in.
        for file in &planned.files {
            let Some(media_file_id) = file.media_file_id.as_deref() else {
                continue;
            };
            self.catalog
                .set_media_file_path(media_file_id, &file.destination_path)
                .await?;
        }

        if let Some(folder) = planned.destination_folder_path.as_deref() {
            self.catalog
                .set_title_folder_path(&title.title_id, folder)
                .await?;
        }

        // FR-056: the transfer's catalog flip is library + root together, after
        // the completeness gate the runner already applied. A same-library root
        // move keeps its narrower write, so nothing about US2 changes.
        if planned.crosses_libraries() {
            self.catalog
                .set_title_library_and_root(
                    &title.title_id,
                    &planned.destination_library_id,
                    &planned.destination_root_id,
                )
                .await?;
        } else {
            self.catalog
                .set_title_root(&title.title_id, &planned.destination_root_id)
                .await?;
        }

        let _ = operation;
        for warning in &planned.warnings {
            outcome.warnings.push(warning.clone());
        }
        Ok(outcome)
    }

    async fn clean_up_title(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
    ) -> AppResult<TitleStepOutcome> {
        let planned = self.title(&title.title_id)?;
        let mut outcome = TitleStepOutcome::clean();

        // The persisted records are the gate, not the absence of an error: only
        // a file recorded `verified` may have its source touched (FR-044).
        let records = self
            .store
            .list_location_file_verifications(&operation.id, Some(&title.title_id))
            .await?;
        let verified: BTreeMap<String, &crate::location::model::FileVerificationRecord> = records
            .iter()
            .filter(|record| record.outcome.permits_source_removal())
            .map(|record| (record.destination_path.clone(), record))
            .collect();

        // 1. Proven duplicates: the destination copy survives, the redundant
        //    source is recycled (FR-073).
        for source in &planned.deduplicated_sources {
            let disposal = self
                .recycler
                .recycle_source(
                    &operation.id,
                    planned,
                    &stored_path_to_path_buf(source),
                    None,
                    0,
                )
                .await?;
            if let SourceDisposal::RemovedRecycleUnavailable(reason) = disposal {
                outcome.warnings.push(format!(
                    "the duplicate source {source} was removed rather than recycled: {reason}"
                ));
            }
        }

        // 2. Sources a cross-volume copy made redundant. A same-filesystem
        //    rename left nothing behind, and its record carries no hashes,
        //    which is exactly how the two cases are told apart (FR-032).
        for file in &planned.files {
            let Some(record) = verified.get(&file.destination_path) else {
                continue;
            };
            if record.hashes.is_none() {
                continue;
            }
            let source = stored_path_to_path_buf(&file.source_path);
            let disposal = self
                .recycler
                .recycle_source(
                    &operation.id,
                    planned,
                    &source,
                    file.media_file_id.as_deref(),
                    file.size_bytes,
                )
                .await?;
            if let SourceDisposal::RemovedRecycleUnavailable(reason) = disposal {
                outcome.warnings.push(format!(
                    "the copied source {} was removed rather than recycled: {reason}",
                    file.source_path
                ));
            }
        }

        // 3. Only *empty* source directories, deepest first (FR-031). A
        //    directory that still holds anything — unmanaged content, a file
        //    this operation never planned — is left exactly as it is.
        for directory in &planned.prune_directories {
            let path = stored_path_to_path_buf(directory);
            match remove_directory_if_empty(&path).await {
                DirectoryPrune::Removed | DirectoryPrune::AlreadyAbsent => {}
                DirectoryPrune::NotEmpty => outcome.warnings.push(format!(
                    "the source directory {directory} still holds content, so it was left in place"
                )),
                DirectoryPrune::Failed(error) => outcome.warnings.push(format!(
                    "the source directory {directory} could not be removed: {error}"
                )),
            }
        }

        Ok(outcome)
    }
}

/// Outcome of considering one source directory for removal.
#[derive(Debug)]
enum DirectoryPrune {
    Removed,
    AlreadyAbsent,
    NotEmpty,
    Failed(String),
}

/// Remove `path` only when it is a directory and holds nothing.
///
/// `remove_dir` already refuses a non-empty directory, but reading the entries
/// first is what lets the caller *report* "still holds content" rather than an
/// errno the user cannot act on (C3).
async fn remove_directory_if_empty(path: &Path) -> DirectoryPrune {
    let mut entries = match tokio::fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return DirectoryPrune::AlreadyAbsent;
        }
        Err(error) => return DirectoryPrune::Failed(error.to_string()),
    };
    match entries.next_entry().await {
        Ok(None) => {}
        Ok(Some(_)) => return DirectoryPrune::NotEmpty,
        Err(error) => return DirectoryPrune::Failed(error.to_string()),
    }
    match tokio::fs::remove_dir(path).await {
        Ok(()) => DirectoryPrune::Removed,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            DirectoryPrune::AlreadyAbsent
        }
        Err(error) => DirectoryPrune::Failed(error.to_string()),
    }
}

// ── TitleAdmissionCheck ──────────────────────────────────────────────────────

/// The FR-089 staleness rule for a root move.
///
/// Asked once per title, immediately before that title starts, and never for a
/// title that already settled. The two sides of the rule:
///
/// - **Stale**: the catalog inputs the plan was built from changed (the title's
///   root, folder, or tracked-file set moved underneath it), or a source file
///   this operation has not processed yet vanished. Both map onto
///   [`PlanInputChange::CatalogInput`] / [`PlanInputChange::UnprocessedSourceItem`],
///   whose [`PlanInputChange::is_stale`] is `true`.
/// - **Resumable**: destination content this operation itself wrote, whether
///   verified or a half-written partial. Those map onto
///   [`PlanInputChange::VerifiedDestinationFile`] and
///   [`PlanInputChange::ExpectedDestinationPartial`], which are not stale — a
///   source that is gone because *this* operation already renamed it into its
///   verified destination is its own footprint, not a foreign change.
pub struct RootMoveAdmission<'a> {
    plan: &'a RootMoveExecutionPlan,
    catalog: &'a dyn RootMoveCatalog,
}

impl<'a> RootMoveAdmission<'a> {
    pub fn new(plan: &'a RootMoveExecutionPlan, catalog: &'a dyn RootMoveCatalog) -> Self {
        Self { plan, catalog }
    }
}

#[async_trait]
impl TitleAdmissionCheck for RootMoveAdmission<'_> {
    async fn admit_title(&self, context: TitleAdmissionContext<'_>) -> AppResult<TitleAdmission> {
        let title_id = context.title.title_id.as_str();
        let Some(planned) = self.plan.title(title_id) else {
            return Ok(TitleAdmission::Skip(format!(
                "title {title_id} is not in the confirmed plan"
            )));
        };

        let Some(placement) = self.catalog.title_placement(title_id).await? else {
            return Ok(stale(
                PlanInputChange::CatalogInput,
                format!("title {title_id} no longer exists"),
            ));
        };

        // The catalog inputs the plan was built from. A transfer that already
        // flipped this title's library is this operation's own footprint, not a
        // foreign change (FR-089) — the same reasoning the root check below
        // uses, and what lets a run interrupted after a title's checkpoint
        // re-enter that title and converge.
        if placement.library_id != planned.source_library_id
            && placement.library_id != planned.destination_library_id
        {
            return Ok(stale(
                PlanInputChange::CatalogInput,
                format!(
                    "\"{}\" moved to library {} after the preview was taken",
                    planned.title_name, placement.library_id
                ),
            ));
        }
        if placement.root_folder_id != planned.source_root_id
            && placement.root_folder_id != planned.destination_root_id
        {
            return Ok(stale(
                PlanInputChange::CatalogInput,
                format!(
                    "\"{}\" moved to root {} after the preview was taken",
                    planned.title_name, placement.root_folder_id
                ),
            ));
        }
        if let Some(expected) = planned.source_folder_path.as_deref()
            && let Some(actual) = placement.folder_path.as_deref()
            && !crate::stored_paths::folder_paths_match(expected, actual)
            && planned
                .destination_folder_path
                .as_deref()
                .is_none_or(|destination| !crate::stored_paths::folder_paths_match(destination, actual))
        {
            return Ok(stale(
                PlanInputChange::CatalogInput,
                format!(
                    "\"{}\" now owns {actual}, not the folder the preview planned",
                    planned.title_name
                ),
            ));
        }

        // Tracked files that appeared or disappeared in the catalog change what
        // the plan would move, so the plan no longer describes reality.
        let planned_media: BTreeSet<String> = planned
            .files
            .iter()
            .filter(|file| file.media_file_id.is_some())
            .map(|file| file.source_path.clone())
            .collect();
        let planned_destinations: BTreeSet<String> = planned
            .files
            .iter()
            .filter(|file| file.media_file_id.is_some())
            .map(|file| file.destination_path.clone())
            .collect();
        for path in &placement.media_file_paths {
            if !planned_media.contains(path) && !planned_destinations.contains(path) {
                return Ok(stale(
                    PlanInputChange::CatalogInput,
                    format!(
                        "\"{}\" gained a tracked file at {path} after the preview was taken",
                        planned.title_name
                    ),
                ));
            }
        }

        // Unprocessed sources must still be there. A source that is gone
        // because this operation already verified its destination is the
        // operation's own footprint (FR-089), not a foreign change.
        for file in &planned.files {
            if context.verified_destinations.contains(&file.destination_path) {
                debug_assert!(!PlanInputChange::VerifiedDestinationFile.is_stale());
                continue;
            }
            let source = stored_path_to_path_buf(&file.source_path);
            if tokio::fs::symlink_metadata(&source).await.is_err() {
                return Ok(stale(
                    PlanInputChange::UnprocessedSourceItem,
                    format!(
                        "the source {} is gone and its destination was never verified",
                        file.source_path
                    ),
                ));
            }
        }

        Ok(TitleAdmission::Proceed)
    }
}

/// Turn an observed change into an admission verdict, honouring
/// [`PlanInputChange::is_stale`] rather than hardcoding the rule.
fn stale(change: PlanInputChange, detail: String) -> TitleAdmission {
    if change.is_stale() {
        TitleAdmission::Stale(detail)
    } else {
        TitleAdmission::Proceed
    }
}

// ── Recyclers ────────────────────────────────────────────────────────────────

/// A recycler that removes verified-redundant sources outright, warning every
/// time, for deployments with no recycle bin configured.
///
/// This is only ever reached for a file whose destination copy is *proven*
/// present and correct, which is the difference between this and a deletion
/// (C4: destruction requires proof). Collisions and duplicates never take this
/// path — [`crate::location::collisions`] preserves and renames those instead
/// (FR-073).
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoveVerifiedSource;

#[async_trait]
impl SourceRecycler for RemoveVerifiedSource {
    async fn recycle_source(
        &self,
        _operation_id: &str,
        _title: &RootMoveTitleExecution,
        source: &Path,
        _media_file_id: Option<&str>,
        _size_bytes: u64,
    ) -> AppResult<SourceDisposal> {
        match tokio::fs::remove_file(source).await {
            Ok(()) => Ok(SourceDisposal::RemovedRecycleUnavailable(
                "no recycle bin is configured for this root",
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(SourceDisposal::AlreadyAbsent)
            }
            Err(error) => Err(AppError::Repository(format!(
                "failed to remove the verified source {}: {error}",
                source.display()
            ))),
        }
    }
}

/// The production recycler: the configured recycle bin, with removal as the
/// explicit, warned fallback when the bin cannot take the file.
pub struct RecycleBinSourceRecycler {
    configs: BTreeMap<String, crate::recycle_bin::RecycleBinConfig>,
}

impl RecycleBinSourceRecycler {
    /// `configs` is keyed by the stored source-root path each title moves out
    /// of, because the recycle bin is configured per media root.
    pub fn new(configs: BTreeMap<String, crate::recycle_bin::RecycleBinConfig>) -> Self {
        Self { configs }
    }

    fn config_for(
        &self,
        title: &RootMoveTitleExecution,
    ) -> Option<&crate::recycle_bin::RecycleBinConfig> {
        self.configs.get(title.source_root_path.as_deref()?)
    }
}

#[async_trait]
impl SourceRecycler for RecycleBinSourceRecycler {
    async fn recycle_source(
        &self,
        operation_id: &str,
        title: &RootMoveTitleExecution,
        source: &Path,
        media_file_id: Option<&str>,
        size_bytes: u64,
    ) -> AppResult<SourceDisposal> {
        if tokio::fs::symlink_metadata(source).await.is_err() {
            return Ok(SourceDisposal::AlreadyAbsent);
        }

        let Some(config) = self.config_for(title).filter(|config| config.enabled) else {
            return RemoveVerifiedSource
                .recycle_source(operation_id, title, source, media_file_id, size_bytes)
                .await;
        };

        let manifest = crate::recycle_bin::RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: Some(operation_id.to_string()),
            recycled_at: chrono::Utc::now().to_rfc3339(),
            original_path: path_to_stored_string(source),
            original_file_id: media_file_id.map(str::to_string),
            size_bytes,
            title_id: Some(title.title_id.clone()),
            media_root: title.source_root_path.clone(),
            reason: "location_operation_source".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        };

        match crate::recycle_bin::recycle_file(config, source, manifest).await {
            Ok(Some(_)) => Ok(SourceDisposal::Recycled),
            Ok(None) => Ok(SourceDisposal::AlreadyAbsent),
            Err(_) => {
                // The bin refused the file. The destination copy is proven, so
                // removing the source is safe — but it is never silent.
                RemoveVerifiedSource
                    .recycle_source(operation_id, title, source, media_file_id, size_bytes)
                    .await
            }
        }
    }
}

/// Source paths a plan expects to disappear, for a caller that wants to assert
/// nothing else was touched.
pub fn planned_source_paths(plan: &RootMoveExecutionPlan) -> Vec<PathBuf> {
    plan.titles
        .iter()
        .flat_map(|title| title.files.iter())
        .map(|file| stored_path_to_path_buf(&file.source_path))
        .collect()
}

#[cfg(test)]
mod tests;
