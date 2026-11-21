//! The root-move execution path: the three runner seams, wired end to end.
//!
//! [`crate::location::executor::LocationOperationRunner`] owns the state
//! machine, the checkpoints, the cancel points, and the resume cursor. This
//! module supplies what it deliberately does not know:
//!
//! | Seam | Implementation here | What it guarantees |
//! |---|---|---|
//! | [`TitleFileMover`] | [`RootMoveFileMover`] | Same-filesystem link placement without copying bytes; otherwise a verified streaming copy at the operation's depth, or a proof of a destination an interrupted run already placed. The source survives until the journal and catalog are durable. Configured permissions land on the destination straight after verification. |
//! | [`TitleReconciler`] | [`RootMoveReconciler`] | Catalog ownership flips only after every planned file for the title is verified; then sources are removed, then *empty* source directories. For a title that merges into an existing destination title, the flip is replaced by the merge engine's single catalog transaction. |
//! | [`TitleAdmissionCheck`] | [`RootMoveAdmission`] | The scope rule: a changed catalog input or an unprocessed source that vanished is stale; this operation's own partial destination content is resumable. |
//!
//! # Why the source is never touched by the mover
//!
//! A cross-volume move copies. Removing the source is [`RootMoveReconciler`]'s
//! cleanup step, and it reads the *persisted verification records* to decide —
//! not the absence of an error. A file whose record says anything other than
//! `verified` keeps its source, which is what makes a byte flipped after
//! write end with both copies intact rather than with a corrupted
//! survivor.
//!
//! # Why cleanup is idempotent
//!
//! Resume re-enters a title at its checkpoint, so cleanup can run twice for the
//! same file. Every step here treats "already gone" as success: a missing
//! source is a completed removal, and a non-empty directory is simply left
//! alone.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use scryer_domain::MediaFacet;

use crate::disk::reason::DISK_PATH_MISSING;
use crate::disk::{
    Disk, Done, Doubt, EntryKind, LinkAfter, LinkProof, Mark, Parents, PathReading, Placed,
    Removal, Remove, Sameness, Unproven,
};
use crate::location::executor::{
    FileMoveRequest, PlannedTitle, TitleAdmission, TitleAdmissionCheck, TitleAdmissionContext,
    TitleFileMover, TitleReconciler, TitleStepOutcome,
};
use crate::location::merge::engine::{TitleMergeRepository, execute_merge, plan_merge};
use crate::location::model::LocationOperation;
use crate::location::preview::PlanInputChange;
use crate::location::root_move::{RootMoveExecutionPlan, RootMoveTitleExecution};
use crate::location::verify::{DestinationClaim, VerifiedFile, same_filesystem};
use crate::ports::LocationOperationRepository;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppError, AppResult};

// ── Seam: configured destination permissions ────────────────────────────────

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
/// check compares it against the confirmed plan.
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
    async fn admit_conflict(
        &self,
        _operation: &LocationOperation,
        _plan: &RootMoveExecutionPlan,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "Folder conflict validation is unavailable".into(),
        ))
    }
    /// Apply the confirmed anonymous-folder media associations after verification.
    async fn reconcile_conflict(
        &self,
        _operation: &LocationOperation,
        _plan: &RootMoveExecutionPlan,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "Folder conflict reconciliation is unavailable".into(),
        ))
    }
    /// Dispose only the confirmed conflict allowlist after catalog reconciliation.
    async fn clean_up_conflict(
        &self,
        _operation: &LocationOperation,
        _plan: &RootMoveExecutionPlan,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "Folder conflict cleanup is unavailable".into(),
        ))
    }
    async fn has_other_media_at_path(
        &self,
        _title_id: &str,
        _path: &str,
        _excluding_id: &str,
    ) -> AppResult<bool> {
        Ok(false)
    }
    /// Every `(media file id, path)` row of the title, read once so the
    /// reconciler can answer [`Self::has_other_media_at_path`] for each of the
    /// title's files without reading the list again. `None` means the catalog
    /// cannot list them, and each question goes to
    /// [`Self::has_other_media_at_path`] instead.
    async fn media_file_paths_for_title(
        &self,
        _title_id: &str,
    ) -> AppResult<Option<Vec<(String, String)>>> {
        Ok(None)
    }
    /// Where the catalog currently says the title lives. `None` when the title
    /// no longer exists.
    async fn title_placement(&self, title_id: &str) -> AppResult<Option<TitlePlacementSnapshot>>;

    /// Point a tracked media file at its new path.
    async fn set_media_file_path(&self, media_file_id: &str, stored_path: &str) -> AppResult<()>;

    /// Point several media files at their new paths, in order. The default
    /// writes them one at a time and stops at the first error; a SQL catalog
    /// writes them in one transaction, so on an error nothing is kept and the
    /// caller can replay.
    async fn set_media_file_paths(&self, paths: &[(String, String)]) -> AppResult<()> {
        for (media_file_id, stored_path) in paths {
            self.set_media_file_path(media_file_id, stored_path).await?;
        }
        Ok(())
    }

    /// Drop the catalog row for a source copy the collision engine proved
    /// redundant.
    ///
    /// The destination's row is the survivor and keeps every association; this
    /// removes the duplicate that would otherwise point at a dropped file.
    /// A row that is already gone is success — cleanup runs again on resume.
    async fn delete_media_file(&self, media_file_id: &str) -> AppResult<()>;

    /// Assign the folder the title now owns.
    async fn set_title_folder_path(&self, title_id: &str, stored_path: &str) -> AppResult<()>;

    /// Run the checks [`Self::set_title_folder_path`] would run, writing
    /// nothing, so a folder the title cannot own is refused before any file
    /// moves.
    async fn check_title_folder_path(&self, title_id: &str, stored_path: &str) -> AppResult<()>;

    /// Update the title's root reference (the synthetic id).
    async fn set_title_root(&self, title_id: &str, root_folder_id: &str) -> AppResult<()>;

    /// Transfer the title into another library, together with the root it lands
    /// on, in **one** transaction.
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
    /// destination-title detection reads. A title transferred without
    /// it would keep answering identity lookups from the library it left.
    ///
    /// `converted_facet` is the third half of the same indivisible write.
    /// A title's facet and its library's facet are one invariant, not
    /// two values that may briefly disagree: `title_root_folder_path` refuses a
    /// title whose facet does not match its library
    /// (`crate::catalog::workflow::roots`), so a series that reached an anime
    /// library without its facet would fail every subsequent path lookup. It
    /// belongs to this call for exactly the reason the root does.
    /// `drop_tag_prefixes` are the reserved prefixes whose values were derived
    /// under the old facet and must not survive it.
    async fn set_title_library_and_root(
        &self,
        title_id: &str,
        library_id: &str,
        root_folder_id: &str,
        converted_facet: Option<&MediaFacet>,
        drop_tag_prefixes: &[String],
    ) -> AppResult<()>;

    /// Persist the hashes the copy pass produced onto the moved media file
    /// (migration 0205).
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

// ── Seam: retiring the merged source title ───────────────────────────────────

/// A source title whose catalog row the merge transaction has just removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergedSourceRetirement {
    pub operation_id: String,
    pub source_title_id: String,
    pub destination_title_id: String,
    /// The source→destination map the merge committed with. The source
    /// title's downloads follow it to the destination.
    pub identity_map: crate::location::merge::map::MergeIdentityMap,
}

/// Runs the ordinary title-delete path's logical cleanup for a merged source
/// title.
///
/// A seam because that cleanup reaches subsystems the merge engine is
/// deliberately blind to — the download queue, the acquisition ledger, the
/// probe-signature cache — and because it is *record-only*: the source's files
/// were repointed, never removed. It runs after the transaction because a
/// merge that fails must not have already stripped the source title's rows, and
/// losing this call to a crash leaves rows nothing points at rather than a wrong
/// catalog — so a failure here is a warning, not a failed title.
#[async_trait]
pub trait MergedSourceRetirer: Send + Sync {
    async fn retire_merged_source(&self, request: MergedSourceRetirement) -> AppResult<()>;
}

/// Retires nothing. The right default for a reconciler wired without the
/// use-case layer.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoMergedSourceRetirement;

#[async_trait]
impl MergedSourceRetirer for NoMergedSourceRetirement {
    async fn retire_merged_source(&self, _request: MergedSourceRetirement) -> AppResult<()> {
        Ok(())
    }
}

// ── TitleFileMover ───────────────────────────────────────────────────────────

/// Moves one file for a root move.
///
/// Same filesystem → an exclusive hard link, recorded using the existing
/// no-copy verification result: no bytes were copied, and the source survives
/// until the journal and catalog are durable. Symbolic links retain their target.
///
/// Different filesystem → [`VerifiedCopier::copy_and_verify`] at the
/// operation's configured depth. The source is left exactly where it is; only
/// [`RootMoveReconciler`] removes it, and only against a persisted verification
/// record.
#[derive(Clone)]
pub struct RootMoveFileMover {
    resolver: Option<Arc<super::resolution::ConflictResolver>>,
    copier: Disk,
    permissions: Arc<dyn PlacedContentPermissions>,
    /// Where the streamed hashes are persisted (migration 0205).
    ///
    /// The mover is the only place that ever holds them, so it is the only
    /// place that can hand them to the read model. `None` for a mover wired
    /// without a catalog, in which case the backfill job picks the files up
    /// later — a missing hash costs a re-read, never correctness.
    catalog: Option<Arc<dyn RootMoveCatalog>>,
    /// Skip the rename fast path and always copy.
    ///
    /// The mover decides rename-vs-copy from the actual device ids, so a test
    /// living inside one temp directory can never reach the copy path — which
    /// is the path with the partial-copy staging, the read-back, and the
    /// interrupted-destination proof on it. This is the only way to exercise
    /// those against real files; production never sets it.
    force_copies: bool,
    /// Set on the placement handed to the resolver: the resolver may still
    /// refuse a copy after it lands, so it persists the hashes itself once it
    /// has decided, through [`TitleFileMover::persist_deferred_hashes`].
    defer_hash_persist: bool,
}

impl RootMoveFileMover {
    pub fn with_resolver(mut self, resolver: Arc<super::resolution::ConflictResolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }
    pub fn new(copier: Disk, permissions: Arc<dyn PlacedContentPermissions>) -> Self {
        Self {
            copier,
            permissions,
            resolver: None,
            catalog: None,
            force_copies: false,
            defer_hash_persist: false,
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
    /// the backfill queue instead of being read a second time.
    pub fn with_catalog(mut self, catalog: Arc<dyn RootMoveCatalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// A mover that changes no permissions — the right shape for a deployment
    /// with no configured modes, and for tests.
    pub fn without_permissions() -> Self {
        Self::new(Disk::process(), Arc::new(NoPlacedContentPermissions))
    }

    /// The hashes the copy already computed go onto the media file
    /// (0205) as well as into the operation's audit record (0206).
    ///
    /// Only for a verified destination — a hash of bytes that failed their
    /// check describes content we are about to refuse to trust. A
    /// same-filesystem rename carries no hashes at all, and a
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

        let mark = verified
            .destination_mark
            .as_deref()
            .and_then(crate::disk::Mark::from_stored);
        let persisted = crate::location::model::PersistedContentHashes::from_verified(
            hashes,
            &verified.depth,
            mark.as_ref(),
            chrono::Utc::now(),
        );
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
    fn file_abandoned(
        &self,
        operation_id: &str,
        title: &PlannedTitle,
        file: &crate::location::executor::PlannedFile,
        abandonment: crate::location::executor::FileAbandonment,
    ) {
        if let Some(resolver) = &self.resolver {
            resolver.media_abandoned(operation_id, title, file, abandonment);
        }
    }

    async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
        if let Some(resolver) = &self.resolver {
            let mut placement = self.clone();
            placement.resolver = None;
            placement.defer_hash_persist = true;
            return resolver.resolve(&placement, request).await;
        }
        let source = &request.file.source_path;
        let destination = &request.file.destination_path;

        if let Some(parent) = destination.parent()
            && !parent.as_os_str().is_empty()
        {
            Disk::process()
                .create_dir(parent, Parents::Create)
                .await
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to create destination directory {}: {error}",
                        parent.display()
                    ))
                })?;
            // The directory is destination content too, so the operator's
            // configured folder modes apply to it.
            self.permissions.apply_to_directory(parent).await?;
        }

        let verified = if !self.force_copies
            && same_filesystem(source, destination).await
            && stage_same_filesystem_file(source, destination).await?
        {
            VerifiedFile::same_filesystem_rename(source.clone(), destination.clone(), request.depth)
        } else if Disk::process().uncached(destination).await.exists() == Some(true) {
            // The runner only asks for files it has no verification record for,
            // so a destination that is already there is the crash window
            // between a completed copy's promotion and the record that should
            // have followed it — or a file that appeared from outside. Either
            // way it is proven against the source rather than replaced or
            // trusted.
            super::resolution::compare_existing(
                source,
                destination,
                request.progress,
                request.file.source_content.as_ref(),
                request.depth,
            )
            .await?
        } else {
            super::verify::copy_and_verify(
                &self.copier,
                source,
                destination,
                crate::disk::placement::verified_copy(request.depth, DestinationClaim::ClaimHere),
                request.progress,
            )
            .await?
        };

        // Only a destination that is actually there gets its modes changed; a
        // mismatch is left byte-for-byte as written so it can be inspected.
        if verified.permits_source_removal() {
            self.permissions.apply_to_file(destination).await?;
        }
        if !self.defer_hash_persist {
            self.persist_content_hashes(&request, &verified).await;
        }
        Ok(verified)
    }

    async fn persist_deferred_hashes(&self, request: FileMoveRequest<'_>, verified: &VerifiedFile) {
        if self.defer_hash_persist {
            self.persist_content_hashes(&request, verified).await;
        }
    }
}

/// Place a second name for the same regular file without copying bytes. The
/// source survives a crash before the verification journal is durable; cleanup
/// removes that name only after the catalog has moved, and only when the two
/// names read as one file that has another name. A symbolic link is staged
/// as a link of its own on unix; anything else that is not a regular file,
/// a symbolic link on Windows included, takes the ordinary verified-copy
/// path, as every source does on other platforms. A regular file is linked
/// only where its read tells it from every other file; on a volume whose
/// reads cannot (FAT, exFAT, ReFS, a share without file numbers, a
/// filesystem that numbers every file zero) cleanup could never prove the
/// second name, so it is copied. A destination already there is not linked
/// over: it is staged only as a proven second name of the source, refused
/// as one entry under two spellings, and otherwise copied. Each name is read
/// once, links not followed.
#[allow(clippy::disallowed_methods)] // disk-guard: symlink placement
async fn stage_same_filesystem_file(source: &Path, destination: &Path) -> AppResult<bool> {
    let seen = Disk::process()
        .fingerprint(source)
        .await
        .map_err(|reading| {
            AppError::Repository(format!(
                "cannot inspect {}: {}",
                source.display(),
                reading.absence()
            ))
        })?;
    #[cfg(unix)]
    if seen.kind() == Some(EntryKind::Symlink) {
        let target = tokio::fs::read_link(source).await.map_err(|error| {
            AppError::Repository(format!("cannot read link {}: {error}", source.display()))
        })?;
        if tokio::fs::symlink_metadata(destination).await.is_ok() {
            return Ok(tokio::fs::read_link(destination).await.ok().as_ref() == Some(&target));
        }
        tokio::fs::symlink(&target, destination)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "cannot place link {}: {error}",
                    destination.display()
                ))
            })?;
        return Ok(true);
    }
    if seen.kind() != Some(EntryKind::File) || !cfg!(any(unix, windows)) {
        return Ok(false);
    }
    if let Ok(destination_read) = Disk::process().fingerprint(destination).await {
        let resolved = Resolved::of(source, destination).await;
        if resolved.one_path() {
            return Err(AppError::validation_refused(
                crate::error_reasons::location::SOURCE_DESTINATION_SAME_PATH,
                "source and destination are the same path",
            ));
        }
        return match shared_inode(source, destination, &seen, &destination_read).await {
            SharedInode::SecondLink => Ok(true),
            SharedInode::OneEntry => Err(one_entry_refusal()),
            SharedInode::Distinct => Ok(false),
        };
    }
    // Only a read that tells this file from every other can prove the
    // second name at cleanup; anything else is copied.
    if seen.same_file(&seen) != Sameness::Same {
        return Ok(false);
    }
    // The new name is read once and must be the file read above. A link that
    // does not prove is removed by the disk service before it answers.
    let linked = Disk::process()
        .place_proven(
            source,
            destination,
            crate::disk::placement::proven_link(),
            seen,
            LinkProof::Name,
        )
        .await;
    let refusal = match linked {
        Ok(Placed {
            done: Done::HardLinked,
            ..
        }) => return Ok(true),
        Ok(_) => {
            return Err(AppError::Repository(format!(
                "the link to {} was not made as a hard link",
                destination.display()
            )));
        }
        Err(refusal) => refusal,
    };
    unproven_link_answer(destination, refusal.unproven)
}

/// What staging answers for a refused link: `Ok(false)` when no link was
/// made and the ordinary verified copy takes over, and a refusal for a link
/// that was made and did not prove.
pub(super) fn unproven_link_answer(
    destination: &Path,
    unproven: Option<Unproven>,
) -> AppResult<bool> {
    match unproven {
        // No link was made: the ordinary verified copy takes over.
        None => Ok(false),
        // A link that did not prove and is still at the name, whatever the
        // doubt: the path it was left at travels as its own fact.
        Some(Unproven {
            after: LinkAfter::NotRemoved,
            ..
        }) => Err(AppError::validation_refused_with_facts(
            crate::error_reasons::location::UNPROVEN_LINK_NOT_REMOVED,
            format!(
                "the link made at {} could not be proven and could not be removed; the source was kept",
                destination.display()
            ),
            vec![crate::RefusalFact::new(
                crate::error_reasons::location::LEFT_BEHIND_PATH_FACT,
                path_to_stored_string(destination),
            )],
        )),
        // The read that failed is the answer, when the name did not stay.
        Some(Unproven {
            doubt: Doubt::Unread(error),
            ..
        }) => Err(AppError::Repository(format!(
            "cannot inspect {}: {}",
            destination.display(),
            error.words()
        ))),
        // Another file at the name, removed, already gone or left alone.
        Some(_) => Err(AppError::validation_refused(
            crate::error_reasons::location::SOURCE_CHANGED_DURING_FILE_PLACEMENT,
            "source changed during file placement",
        )),
    }
}

/// What a source/destination pair that may share one inode is to the mover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SharedInode {
    /// Not provably one file: the pair is compared by content.
    Distinct,
    /// Two directory entries for one file; removing the source keeps the bytes.
    SecondLink,
    /// One directory entry reached under two spellings (a case-folding or
    /// normalizing mount, or an alias path). Removing either name removes the
    /// only entry, so the pair is refused.
    OneEntry,
}

/// Decide a pair from the facts alone. One file number is not trusted by
/// itself: mounts that synthesize inodes (CIFS without serverino, mergerfs,
/// rclone) can report it for two different files, so a length mismatch sends
/// the pair to the content comparison. A shared file counts as a second link
/// only when the read showed the destination to have another name; a count
/// the read did not give is never assumed to be two.
pub(super) fn shared_inode_decision(
    same_identity: bool,
    source_len: u64,
    destination_len: u64,
    other_names: bool,
) -> SharedInode {
    if !same_identity || source_len != destination_len {
        return SharedInode::Distinct;
    }
    match other_names {
        true => SharedInode::SecondLink,
        false => SharedInode::OneEntry,
    }
}

/// Classify a pair by the disk service's reads of both names, links not
/// followed, then prove a claimed second link is really a second directory
/// entry: two names in one directory must both be listed under their exact
/// spelling, which a case-folding mount or a case-insensitive volume does not
/// do.
pub(super) async fn shared_inode(
    source: &Path,
    destination: &Path,
    source_read: &Mark,
    destination_read: &Mark,
) -> SharedInode {
    let sameness = regular_file_sameness(source_read, destination_read);
    let decision = shared_inode_decision(
        sameness == Sameness::Same,
        source_read.len().unwrap_or_default(),
        destination_read.len().unwrap_or_default(),
        destination_read.has_other_names(),
    );
    // A pair the file numbers cannot tell apart is never a second link, and
    // goes to the content comparison only as two listed entries: one entry
    // under two spellings would be its own match.
    let undecided = sameness == Sameness::CannotTell && source_read.len() == destination_read.len();
    if (decision == SharedInode::SecondLink || undecided)
        && !distinct_directory_entries(source, destination).await
    {
        return SharedInode::OneEntry;
    }
    decision
}

pub(super) fn one_entry_refusal() -> AppError {
    AppError::validation_refused(
        crate::error_reasons::location::SOURCE_DESTINATION_SAME_PATH,
        "source and destination are one directory entry under two spellings",
    )
}

/// Both paths as the platform resolves them, links and aliases followed;
/// `None` for one that does not resolve. Used only to find one path: two
/// resolutions that differ prove nothing, since one folder reached through
/// two server-side spellings (a host name and its address, two shares over
/// one folder) resolves to two strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Resolved {
    source: Option<PathBuf>,
    destination: Option<PathBuf>,
}

impl Resolved {
    /// One resolution of each path.
    pub(super) async fn of(source: &Path, destination: &Path) -> Self {
        let disk = Disk::process();
        Self {
            source: disk.canonicalize(source).await.ok(),
            destination: disk.canonicalize(destination).await.ok(),
        }
    }

    /// Whether both resolve to one path, or neither resolves: either way the
    /// pair is refused as the same path.
    pub(super) fn one_path(&self) -> bool {
        self.source == self.destination
    }
}

/// Whether two names are in two folders, as the disk service's reads of the
/// folders say. Folders the reads cannot tell apart are not two, whatever
/// their paths resolve to. No probe.
pub(super) fn in_two_folders(parents: Sameness) -> bool {
    parents == Sameness::Different
}

/// Whether two paths are two directory entries. Names in two folders the
/// disk service reads as different folders are; names in one folder, or in
/// folders it cannot tell apart, are only when they differ and the listing of
/// the source's folder holds both exactly.
async fn distinct_directory_entries(source: &Path, destination: &Path) -> bool {
    let (Some(source_parent), Some(destination_parent)) = (source.parent(), destination.parent())
    else {
        return false;
    };
    let (Ok(source_dir), Ok(destination_dir)) = (
        Disk::process().fingerprint_followed(source_parent).await,
        Disk::process()
            .fingerprint_followed(destination_parent)
            .await,
    ) else {
        return false;
    };
    distinct_entries_in(source_dir.same_file(&destination_dir), source, destination).await
}

/// [`distinct_directory_entries`] once the folders are read: `parents` is
/// how the disk service's reads of the two folders compare.
#[allow(clippy::disallowed_methods)] // disk-guard: early-exit listing
pub(super) async fn distinct_entries_in(
    parents: Sameness,
    source: &Path,
    destination: &Path,
) -> bool {
    if in_two_folders(parents) {
        return true;
    }
    let Some(source_parent) = source.parent() else {
        return false;
    };
    let (Some(source_name), Some(destination_name)) = (source.file_name(), destination.file_name())
    else {
        return false;
    };
    if source_name == destination_name {
        return false;
    }
    let Ok(mut entries) = tokio::fs::read_dir(source_parent).await else {
        return false;
    };
    let (mut saw_source, mut saw_destination) = (false, false);
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        saw_source |= name == source_name;
        saw_destination |= name == destination_name;
        if saw_source && saw_destination {
            return true;
        }
    }
    false
}

/// Whether two reads are one regular file, proven. A pair that cannot be
/// told apart, such as two names on a filesystem that numbers every file
/// zero, answers `false`, so no caller treats two files as one on a guess.
pub(super) fn same_regular_file(left: &Mark, right: &Mark) -> bool {
    regular_file_sameness(left, right) == Sameness::Same
}

/// Whether two reads are one regular file, two, or cannot be told apart.
/// Anything but two regular files is [`Sameness::Different`]. The reads are
/// the disk service's, so a Windows read carries the volume serial and file
/// index where the volume keeps them, and cannot tell where it does not.
pub(super) fn regular_file_sameness(left: &Mark, right: &Mark) -> Sameness {
    if left.kind() != Some(EntryKind::File) || right.kind() != Some(EntryKind::File) {
        return Sameness::Different;
    }
    left.same_file(right)
}

/// The disk service's read of a name a caller has already read through std,
/// links not followed: the metadata's own mark on unix, with no probe, and
/// one handle read on Windows, where std's metadata names no volume or file.
/// The reading when that read fails. Built from std metadata, the mark never
/// proves a link.
pub(super) async fn mark_beside(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<Mark, PathReading> {
    if cfg!(windows) {
        Disk::process().fingerprint(path).await
    } else {
        Ok(Mark::of(metadata))
    }
}

// ── TitleReconciler ──────────────────────────────────────────────────────────

/// Flips the catalog and cleans up the source for a root move.
///
/// The runner has already proven every planned file for the title before this
/// runs (it refuses to continue otherwise), so `reconcile_title` never has to
/// re-check verification. `clean_up_title` does, because it is the step that
/// touches the source: it reads the persisted verification records and skips
/// any file that is not recorded `verified`.
pub struct RootMoveReconciler<'a> {
    plan: &'a RootMoveExecutionPlan,
    catalog: &'a dyn RootMoveCatalog,
    store: &'a dyn LocationOperationRepository,
    permissions: Arc<dyn PlacedContentPermissions>,
    merges: Option<&'a dyn TitleMergeRepository>,
    retirer: Arc<dyn MergedSourceRetirer>,
}

impl<'a> RootMoveReconciler<'a> {
    pub fn new(
        plan: &'a RootMoveExecutionPlan,
        catalog: &'a dyn RootMoveCatalog,
        store: &'a dyn LocationOperationRepository,
    ) -> Self {
        Self {
            plan,
            catalog,
            store,
            permissions: Arc::new(NoPlacedContentPermissions),
            merges: None,
            retirer: Arc::new(NoMergedSourceRetirement),
        }
    }

    pub fn with_permissions(mut self, permissions: Arc<dyn PlacedContentPermissions>) -> Self {
        self.permissions = permissions;
        self
    }

    /// The merge engine. Required for a plan holding any merge title; a
    /// plan with none never reaches it.
    pub fn with_merges(mut self, merges: &'a dyn TitleMergeRepository) -> Self {
        self.merges = Some(merges);
        self
    }

    pub fn with_merged_source_retirer(mut self, retirer: Arc<dyn MergedSourceRetirer>) -> Self {
        self.retirer = retirer;
        self
    }

    /// Write the waiting media-file paths. When the grouped write fails, they
    /// are written again one at a time, so the rows before the failing one are
    /// kept and the same error is returned, as writing each in turn would.
    async fn flush_media_file_paths(&self, pending: &mut Vec<(String, String)>) -> AppResult<()> {
        match pending.len() {
            0 => return Ok(()),
            1 => {
                let (media_file_id, stored_path) = &pending[0];
                self.catalog
                    .set_media_file_path(media_file_id, stored_path)
                    .await?;
            }
            _ => {
                if let Err(error) = self.catalog.set_media_file_paths(pending).await {
                    tracing::debug!(
                        error = %error,
                        "grouped media-file path write failed; writing each path in turn"
                    );
                    for (media_file_id, stored_path) in pending.iter() {
                        self.catalog
                            .set_media_file_path(media_file_id, stored_path)
                            .await?;
                    }
                }
            }
        }
        pending.clear();
        Ok(())
    }

    fn title(&self, title_id: &str) -> AppResult<&RootMoveTitleExecution> {
        self.plan.title(title_id).ok_or_else(|| {
            AppError::validation_refused(
                crate::error_reasons::location::CONFIRMED_PLAN_NO_INSTRUCTIONS_TITLE,
                format!("the confirmed plan has no instructions for title {title_id}"),
            )
        })
    }

    /// The merge's catalog step: the merge transaction, then the source title's
    /// logical retirement handed to the delete path.
    ///
    /// # Why the plan is re-derived here rather than read off the confirmed one
    ///
    /// The confirmed plan carries the *preview's* merge summary, which is what
    /// the user agreed to and what the fingerprint covers. The transaction has
    /// to run against the catalog as it is at this instant — this title's media
    /// files have just been repointed at their new paths. Re-deriving is also
    /// what makes the confirmation fingerprint meaningful: if the two disagreed, start
    /// would have refused the confirmation before reaching here.
    ///
    /// # Resume idempotence
    ///
    /// `execute_title_merge` is one transaction, so a crash leaves either no
    /// merge or a complete one. A complete merge with an unsettled checkpoint
    /// is handled by the caller, which asks [`Self::merge_already_applied`]
    /// before doing anything at all. Re-planning would fail anyway — the read
    /// phase reads the source title and cannot find it — which is exactly the
    /// failure mode that check avoids turning into a failed title.
    async fn merge_title(
        &self,
        operation: &LocationOperation,
        planned: &RootMoveTitleExecution,
        destination_title_id: &str,
        outcome: &mut TitleStepOutcome,
    ) -> AppResult<()> {
        let merges = self.merges.ok_or_else(|| {
            AppError::validation_refused(
                crate::error_reasons::location::MERGE_UNAVAILABLE,
                format!(
                    "\"{}\" merges into {destination_title_id}, but no merge engine is configured",
                    planned.title_name
                ),
            )
        })?;

        let snapshot = merges
            .load_merge_snapshot(
                &planned.title_id,
                destination_title_id,
                // The resumable-operation block is about a *second* operation
                // still holding the source title. This one legitimately holds it
                // — it is the operation performing the merge — so it excludes
                // itself.
                Some(&operation.id),
            )
            .await?;
        let plan = plan_merge(&snapshot);
        // Checked again, at the last possible moment: the preview refused a
        // blocked merge, and anything that became unmappable since is a failed
        // title rather than a merge attached to a guess. A failed title is
        // contained — the rest of the operation runs on.
        if plan.is_blocked() {
            return Err(AppError::validation_refused(
                crate::error_reasons::location::MERGE_UNAVAILABLE,
                format!(
                    "\"{}\" can no longer merge into {destination_title_id}: {}",
                    planned.title_name,
                    plan.summary
                        .blocked_reason()
                        .unwrap_or_else(|| "unmappable records".to_string())
                ),
            ));
        }

        let identity_map = plan.require_identity_map()?.clone();
        execute_merge(merges, &plan).await?;
        // The merge is the plan's designed outcome, so it is stated, not warned
        // about: the user confirmed it in the preview.
        outcome.notes.push(merge_note(
            &planned.title_name,
            plan.summary.destination_title_name.as_deref(),
            plan.summary.source_records_dropped,
        ));
        self.retire_merged_source(operation, planned, destination_title_id, identity_map)
            .await;
        Ok(())
    }

    /// Whether this title's merge already committed.
    ///
    /// The source title row is the evidence, and it is sound evidence: this
    /// operation holds exclusive ownership of the title for its whole
    /// duration, so nothing else can have removed it. Requiring the destination
    /// to be present too keeps the answer from being "yes" for a catalog that
    /// lost both.
    async fn merge_already_applied(
        &self,
        source_title_id: &str,
        destination_title_id: &str,
    ) -> AppResult<bool> {
        if self
            .catalog
            .title_placement(source_title_id)
            .await?
            .is_some()
        {
            return Ok(false);
        }
        Ok(self
            .catalog
            .title_placement(destination_title_id)
            .await?
            .is_some())
    }

    /// The delete path's logical cleanup for the retired source title, run after
    /// the transaction and never allowed to fail the title.
    async fn retire_merged_source(
        &self,
        operation: &LocationOperation,
        planned: &RootMoveTitleExecution,
        destination_title_id: &str,
        identity_map: crate::location::merge::map::MergeIdentityMap,
    ) {
        let request = MergedSourceRetirement {
            operation_id: operation.id.clone(),
            source_title_id: planned.title_id.clone(),
            destination_title_id: destination_title_id.to_string(),
            identity_map,
        };
        if let Err(error) = self.retirer.retire_merged_source(request).await {
            tracing::warn!(
                operation_id = %operation.id,
                title_id = %planned.title_id,
                error = %error,
                "the merge committed but the retired title's logical cleanup could not be completed; its remaining rows reference a title that no longer exists"
            );
        }
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
        // configured folder modes apply before the catalog points at them.
        if operation.mode != crate::location::model::LocationExecutionMode::UserMovedFiles
            && let Some(folder) = planned.destination_folder_path.as_deref()
        {
            let folder = stored_path_to_path_buf(folder);
            if Disk::process().fresh(&folder).await.exists() == Some(true) {
                self.permissions.apply_to_directory(&folder).await?;
            }
        }

        // A merge, before anything else: one whose transaction already
        // committed has nothing left to reconcile. Its media-file paths were
        // written *before* that transaction, so re-writing them here would
        // reach for rows the merge has already repointed. Asking first is what
        // makes a resumed merge a settle rather than a second attempt.
        if let Some(destination_title_id) = planned.merge_target_title_id.as_deref()
            && self
                .merge_already_applied(&title.title_id, destination_title_id)
                .await?
        {
            tracing::info!(
                operation_id = %operation.id,
                title_id = %title.title_id,
                destination_title_id,
                "the merge for this title already committed; settling its checkpoint without re-planning"
            );
            outcome.notes.extend(planned.warnings.iter().cloned());
            return Ok(outcome);
        }

        // Media-file paths first, then the folder, then the root: each step is
        // individually correct, and an interruption between them leaves rows
        // pointing at real files rather than at a folder nothing lives in.
        let resolutions = self
            .store
            .file_resolutions(&operation.id, &title.title_id)
            .await?;
        for resolution in resolutions.iter().filter(|row| !row.is_disposal()) {
            if !resolution.proof_is_current().await {
                return Err(AppError::validation_refused(
                    crate::error_reasons::location::VERIFICATION_MISMATCH,
                    "A file changed before the catalog update. Its source was preserved; retry to compare it again.",
                ));
            }
            if let Some(warning) = &resolution.warning {
                // A transfer-time rename is conflict resolution doing its job, not a
                // departure from the plan.
                outcome.notes.push(warning.clone());
            }
        }
        if self.plan.conflict.is_some() {
            self.catalog
                .reconcile_conflict(operation, self.plan)
                .await?;
            return Ok(outcome);
        }
        // The first resolution for a source path answers for it, as a scan of
        // the list in order would.
        let mut resolution_by_source: HashMap<&str, &super::resolution::FileResolution> =
            HashMap::with_capacity(resolutions.len());
        for row in &resolutions {
            resolution_by_source
                .entry(row.source_path.as_str())
                .or_insert(row);
        }
        let owner_title_id = planned
            .merge_target_title_id
            .as_deref()
            .unwrap_or(&title.title_id);
        // The owner's media rows, read once on the first identical copy and
        // kept in step with this loop's own writes, so each identical copy is
        // judged against the catalog as the writes before it left it.
        let mut owner_media: Option<Option<Vec<(String, String)>>> = None;
        // Path writes wait here and go out together, ahead of any removal, so
        // the catalog sees them in the order the loop decided them.
        let mut pending_paths: Vec<(String, String)> = Vec::new();
        for file in planned.files.iter().chain(
            self.plan
                .catalog_paths
                .get(&title.title_id)
                .into_iter()
                .flatten(),
        ) {
            let Some(media_file_id) = file.media_file_id.as_deref() else {
                continue;
            };
            let resolution = resolution_by_source.get(file.source_path.as_str()).copied();
            let redundant = match resolution {
                Some(row)
                    if row.disposition == super::resolution::ResolutionDisposition::Identical =>
                {
                    if owner_media.is_none() {
                        // The read must see the writes decided before it.
                        self.flush_media_file_paths(&mut pending_paths).await?;
                        owner_media = Some(
                            self.catalog
                                .media_file_paths_for_title(owner_title_id)
                                .await?,
                        );
                    }
                    match owner_media.as_ref().and_then(Option::as_ref) {
                        Some(rows) => rows
                            .iter()
                            .any(|(id, path)| id != media_file_id && *path == row.destination_path),
                        None => {
                            self.flush_media_file_paths(&mut pending_paths).await?;
                            self.catalog
                                .has_other_media_at_path(
                                    owner_title_id,
                                    &row.destination_path,
                                    media_file_id,
                                )
                                .await?
                        }
                    }
                }
                _ => false,
            };
            if redundant {
                self.flush_media_file_paths(&mut pending_paths).await?;
                self.catalog.delete_media_file(media_file_id).await?;
                if let Some(Some(rows)) = owner_media.as_mut() {
                    rows.retain(|(id, _)| id != media_file_id);
                }
            } else {
                let destination = resolution
                    .map(|row| row.destination_path.as_str())
                    .unwrap_or(&file.destination_path);
                if let Some(Some(rows)) = owner_media.as_mut() {
                    for (id, path) in rows.iter_mut() {
                        if id == media_file_id {
                            *path = destination.to_string();
                        }
                    }
                }
                pending_paths.push((media_file_id.to_string(), destination.to_string()));
            }
        }
        self.flush_media_file_paths(&mut pending_paths).await?;

        // For a merge there is no source title left to point anywhere —
        // the destination title owns the folder and keeps the one it
        // already had, which is the folder this title's files just moved into.
        // Groups 1–5 repoint everything else, in one transaction, and the
        // checkpoint's `merged_into_title_id` records where the title went.
        if let Some(destination_title_id) = planned.merge_target_title_id.as_deref() {
            self.merge_title(operation, planned, destination_title_id, &mut outcome)
                .await?;
            outcome.notes.extend(planned.warnings.iter().cloned());
            return Ok(outcome);
        }

        if let Some(folder) = planned.destination_folder_path.as_deref() {
            self.catalog
                .set_title_folder_path(&title.title_id, folder)
                .await?;
        }

        // The transfer's catalog flip is library + root + facet
        // together, after the completeness gate the runner already applied. A
        // same-library root move keeps its narrower write, so nothing about it
        // changes.
        if planned.crosses_libraries() {
            self.catalog
                .set_title_library_and_root(
                    &title.title_id,
                    &planned.destination_library_id,
                    &planned.destination_root_id,
                    planned.converted_facet.as_ref(),
                    &planned.dropped_tag_prefixes,
                )
                .await?;
        } else {
            self.catalog
                .set_title_root(&title.title_id, &planned.destination_root_id)
                .await?;
        }

        // What the preview said would happen — a companion kept under a new
        // name, a hardlinked source, a same-named neighbour — happened. Those
        // are the plan's own statements, confirmed by the user, so they are
        // stated with the outcome rather than raised as warnings.
        outcome.notes.extend(planned.warnings.iter().cloned());
        Ok(outcome)
    }

    async fn clean_up_title(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
    ) -> AppResult<TitleStepOutcome> {
        if operation.mode == crate::location::model::LocationExecutionMode::UserMovedFiles {
            return Ok(TitleStepOutcome::clean());
        }
        let planned = self.title(&title.title_id)?;
        let mut outcome = TitleStepOutcome::clean();

        // The persisted records are the gate, not the absence of an error: only
        // a file recorded `verified` may have its source touched.
        let records = self
            .store
            .list_location_file_verifications(&operation.id, Some(&title.title_id))
            .await?;
        let verified: BTreeMap<String, &crate::location::model::FileVerificationRecord> = records
            .iter()
            .filter(|record| record.outcome.permits_source_removal())
            .map(|record| (record.destination_path.clone(), record))
            .collect();

        // 1. Retire sources only after durable verification and catalog writes.
        // A no-copy placement keeps both hard links until this point.
        let resolutions = self
            .store
            .file_resolutions(&operation.id, &title.title_id)
            .await?;
        for file in &planned.files {
            let resolution = resolutions
                .iter()
                .find(|row| row.source_path == file.source_path);
            let mut resolved = file.clone();
            // Whether this pass renamed the source back from its set-aside
            // name: only then is a moved change time explained, and only then
            // are the stored versions held to content rather than whole.
            let mut put_back = false;
            if let Some(row) = resolution {
                // A source missing at its name may be one an interrupted
                // cleanup set aside: it is put back and checked like any
                // other, and only a source that is really gone is skipped.
                if Disk::process().uncached(file.source()).await.exists() == Some(false) {
                    match read_copied_source(&file.source()).await? {
                        None => continue,
                        Some(read) => put_back = read.put_back,
                    }
                }
                if !row.proof_holds_by(version_check(put_back)).await {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::location::FILE_CHANGED_BEFORE_SOURCE_CLEANUP,
                        "A file changed before source cleanup. The source was preserved; retry to compare it again.",
                    ));
                }
                resolved.destination_path = row.destination_path.clone();
            }
            let file = &resolved;
            let Some(record) = verified.get(&file.destination_path) else {
                continue;
            };
            if let Some(row) = resolution.filter(|row| {
                row.disposition == super::resolution::ResolutionDisposition::Identical
            }) {
                // The transfer proved this source byte-identical to the
                // destination's copy while both existed, and the proof was
                // re-checked above. The source is a redundant copy of
                // content the library already holds, not user data that can be
                // lost, so it is dropped outright rather than recycled
                // — the destination copy is the survivor.
                //
                // "The destination copy is the survivor" is the whole
                // justification, so it is proven rather than assumed: the file
                // versions checked above are the fast negative, and the bytes
                // still have to hash to the identity the transfer recorded
                // before this source is dropped without a recycle bin behind
                // it.
                if !row
                    .proof_supports_disposal(operation.verification_depth, version_check(put_back))
                    .await?
                {
                    return Err(AppError::validation_refused(crate::error_reasons::location::DESTINATION_COPY_NO_LONGER_HOLDS_CONTENT_MOVE_PROVED_IDENTICAL_SOURCE, "The destination copy no longer holds the content this move proved identical to the source. The source was preserved; retry to compare the files again."
                            ));
                }
                let source = stored_path_to_path_buf(&file.source_path);
                match Disk::process().remove(&source, Remove::File).await {
                    Ok(Removal::Removed) => tracing::info!(
                        operation_id = %operation.id,
                        title_id = %title.title_id,
                        source = %file.source_path,
                        destination = %file.destination_path,
                        "dropped a source copy proven identical to the destination's"
                    ),
                    Ok(Removal::AlreadyAbsent) => {}
                    Err(error) => {
                        return Err(AppError::Repository(format!(
                            "failed to remove the identical source {}: {error}",
                            file.source_path
                        )));
                    }
                }
                continue;
            }
            // A record without hashes or a content match placed the source's
            // own file; one the quick check matched is cleaned up below, as a
            // copy is.
            if record.is_link() {
                let source = stored_path_to_path_buf(&file.source_path);
                if source == file.destination() {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::location::SOURCE_DESTINATION_SAME_PATH,
                        "source and destination are the same path",
                    ));
                }
                retire_linked_source(&source, &file.destination(), &file.source_path).await?;
                continue;
            }
            let source = stored_path_to_path_buf(&file.source_path);
            // A durable copy receipt describes the bytes copied, not whatever
            // occupies the source name after an interruption. Recheck both
            // copies and their versions before handing this path to disposal.
            // The source's own read, a link not followed, is also the mark its
            // removal below is settled against.
            let Some(source_read) = read_copied_source(&source).await? else {
                continue;
            };
            put_back |= source_read.put_back;
            let source_mark = source_read.mark;
            // A linked source that took a copy is proven by the file it
            // names (the proofs below read through the link); once proven,
            // only the link's own name is removed, never the file it names.
            if !matches!(
                source_mark.kind(),
                Some(EntryKind::File | EntryKind::Symlink)
            ) {
                return Err(AppError::validation_refused(
                    crate::error_reasons::location::SOURCE_CHANGED_BEFORE_CLEANUP,
                    "Source changed before cleanup; preserved it.",
                ));
            }
            // A link is read once more, followed: one more probe, for a link
            // source only.
            let linked = source_mark.kind() == Some(EntryKind::Symlink);
            if linked {
                let named = Disk::process().follow(&source).await;
                match named.exists() {
                    // The link names nothing, so its name holds no data: after
                    // the verified copy it goes, whatever it pointed at.
                    Some(false) => {
                        match Disk::process().remove(&source, Remove::File).await {
                            Ok(Removal::Removed) => tracing::info!(
                                operation_id = %operation.id,
                                title_id = %title.title_id,
                                source = %file.source_path,
                                destination = %file.destination_path,
                                "removed a source link that names nothing, beside its verified copy"
                            ),
                            Ok(Removal::AlreadyAbsent) => {}
                            Err(error) => {
                                return Err(AppError::Repository(format!(
                                    "failed to remove the source link {}: {error}",
                                    file.source_path
                                )));
                            }
                        }
                        continue;
                    }
                    // A link that cannot be followed for a reason that is not
                    // the volume going quiet names something that is no
                    // longer the file it was: Windows refuses to open a
                    // folder through a link made for a file rather than
                    // reading it as a folder. A timeout or an unreachable
                    // volume is retried.
                    None if !matches!(
                        named.unreachable_cause(),
                        Some(
                            crate::disk::reason::DISK_TIMED_OUT
                                | crate::disk::reason::DISK_PATH_UNREACHABLE
                        )
                    ) =>
                    {
                        return Err(AppError::validation_refused(
                            crate::error_reasons::location::SOURCE_CHANGED_BEFORE_CLEANUP,
                            "Source changed before cleanup; preserved it.",
                        ));
                    }
                    None => return Err(AppError::Repository(named.absence().into())),
                    Some(true) if !named.is(EntryKind::File) => {
                        return Err(AppError::validation_refused(
                            crate::error_reasons::location::SOURCE_CHANGED_BEFORE_CLEANUP,
                            "Source changed before cleanup; preserved it.",
                        ));
                    }
                    Some(true) => {}
                }
            }
            let destination = file.destination();
            // The resolution row recorded both files' versions right after the
            // copy was proven; a file that no longer holds its version's
            // content has changed since, and the source is kept. A link's
            // version is the link's own.
            let versions_held = match resolution.and_then(|row| {
                row.source_version
                    .as_deref()
                    .zip(row.destination_version.as_deref())
            }) {
                Some((row_source_version, row_destination_version)) => {
                    let holds = version_check(put_back).holds();
                    holds(
                        Some(&super::resolution::file_version(&source).await?),
                        row_source_version,
                    ) && holds(
                        Some(&super::resolution::file_version(&destination).await?),
                        row_destination_version,
                    )
                }
                None => true,
            };
            // The source and the destination are then compared now, at the
            // depth the copy was proven at: the quick check samples both
            // live, and the full check holds their full hashes to each other.
            // A link is compared as the file it names. A file that changed
            // while it was read, or one that cannot be read, keeps the source.
            let check = match versions_held {
                true => {
                    Disk::process()
                        .check(
                            // At full depth the hashes the copy streamed and
                            // read back answer while both files still fit
                            // them; a file that moved is read again.
                            crate::disk::CheckSide::new(source.as_path()),
                            crate::disk::CheckSide::new(destination.as_path()),
                            super::verify::disk_depth(record.depth.applied),
                        )
                        .await
                }
                false => crate::disk::Check::Changed,
            };
            if record.source_path != file.source_path || check != crate::disk::Check::Same {
                return Err(AppError::validation_refused(
                    crate::error_reasons::location::SOURCE_DESTINATION_CHANGED_BEFORE_CLEANUP,
                    "Source or destination changed before cleanup; preserved the source.",
                ));
            }
            // The destination was just proven to hold the source's bytes, so the
            // source is the redundant half of a finished move and is removed,
            // not recycled: a recycle entry would only be a second copy of
            // content the library already holds. Equal bytes do not show two
            // entries, though: one entry reached under two spellings checks as
            // the same content, and removing the source would remove the only
            // copy. See [`remove_copied_source`].
            if remove_copied_source(&source, &source_mark, &destination).await? {
                tracing::info!(
                    operation_id = %operation.id,
                    title_id = %title.title_id,
                    source = %file.source_path,
                    destination = %file.destination_path,
                    "removed a source copy verified at its destination"
                );
            }
        }

        if self.plan.conflict.is_some() {
            // Historical hardlink content proofs remain usable after retiring
            // transfer sources. Inferior-file disposal rechecks the survivors.
            self.catalog.clean_up_conflict(operation, self.plan).await?;
        }

        // 2. Only *empty* source directories, deepest first. A
        //    directory that still holds anything — unmanaged content, a file
        //    this operation never planned — is left exactly as it is.
        let mut kept = Vec::new();
        for directory in &planned.prune_directories {
            let path = stored_path_to_path_buf(directory);
            match remove_directory_if_empty(&path).await {
                DirectoryPrune::Removed | DirectoryPrune::AlreadyAbsent => {}
                DirectoryPrune::NotEmpty => kept.push(directory.as_str()),
                DirectoryPrune::Failed(error) => outcome.warnings.push(format!(
                    "The old folder {directory} could not be removed: {error}"
                )),
            }
        }
        if let Some(warning) = kept_folders_warning(&kept) {
            outcome.warnings.push(warning);
        }

        Ok(outcome)
    }
}

/// How a cleanup holds a row's stored versions: whole, unless it renamed the
/// source back from its set-aside name in this pass.
fn version_check(put_back: bool) -> super::resolution::VersionCheck {
    match put_back {
        true => super::resolution::VersionCheck::SourcePutBack,
        false => super::resolution::VersionCheck::Exact,
    }
}

/// A copied source as its cleanup starts: its read, a link not followed, and
/// whether this read renamed it back from its set-aside name first.
struct CopiedSource {
    mark: Mark,
    put_back: bool,
}

/// Read a copied source at the start of its cleanup, a link not followed.
/// `None` when the source is gone, so there is nothing left to remove.
///
/// A cleanup interrupted after it set the source aside to tell it from its
/// destination leaves the file at its set-aside name. A source name that
/// reads missing is therefore put back first, as an import's start-of-run
/// read does, and read again; where the two names were one entry, that is
/// also what fills the destination's name again. A nonempty file at the
/// set-aside name that cannot go back is kept there and blocks the cleanup,
/// named by [`location::LEFT_BEHIND_PATH_FACT`].
///
/// Probes: one read of the source. Only when it reads missing, one read of
/// the set-aside name, then one rename when it holds a nonempty file (and one
/// more read of the source after it), or one removal of an empty one.
///
/// [`location::LEFT_BEHIND_PATH_FACT`]: crate::error_reasons::location::LEFT_BEHIND_PATH_FACT
async fn read_copied_source(source: &Path) -> AppResult<Option<CopiedSource>> {
    match Disk::process().fingerprint(source).await {
        Ok(mark) => {
            return Ok(Some(CopiedSource {
                mark,
                put_back: false,
            }));
        }
        Err(reading) if reading.exists() == Some(false) => {}
        Err(reading) => return Err(AppError::Repository(reading.absence().into())),
    }
    let owned = source.to_path_buf();
    let answer = tokio::task::spawn_blocking(move || {
        Disk::process().put_back_set_aside_source_blocking(&owned)
    })
    .await
    .map_err(|error| {
        AppError::Repository(format!(
            "putting back the set-aside source {} stopped: {error}",
            source.display()
        ))
    })?;
    match answer {
        crate::disk::PutBackAside::Nothing => Ok(None),
        crate::disk::PutBackAside::PutBack => match Disk::process().fingerprint(source).await {
            Ok(mark) => Ok(Some(CopiedSource {
                mark,
                put_back: true,
            })),
            Err(reading) if reading.exists() == Some(false) => Ok(None),
            Err(reading) => Err(AppError::Repository(reading.absence().into())),
        },
        crate::disk::PutBackAside::Failed { at, error } => {
            tracing::error!(
                source = %source.display(),
                at = %at.display(),
                error = %error,
                "a move's source set aside by an interrupted cleanup could not be put back; it is kept at its set-aside name"
            );
            Err(set_aside_source_kept(source, &at))
        }
        crate::disk::PutBackAside::SourceNameTaken { at } => {
            Err(set_aside_source_kept(source, &at))
        }
    }
}

/// The blocked cleanup of a source whose file is kept at its set-aside name
/// `at`, which the refusal names for the operator. Nothing was removed.
fn set_aside_source_kept(source: &Path, at: &Path) -> AppError {
    AppError::ManualReconciliationRefused {
        message: format!(
            "the move's source {} is kept at {}, where a cleanup set it aside, and could not be put back; nothing was removed",
            source.display(),
            at.display()
        ),
        reason: crate::error_reasons::location::SET_ASIDE_SOURCE_NOT_PUT_BACK,
        facts: vec![crate::RefusalFact {
            key: crate::error_reasons::location::LEFT_BEHIND_PATH_FACT,
            value: crate::RefusalFactValue::Text(at.display().to_string()),
        }],
    }
}

/// Remove a copied source once its destination was proven to hold its bytes,
/// only when the two are separate entries. `source_mark` is the read of the
/// source itself taken before the comparison. `Ok(false)` when the source was
/// already gone, or when the two are one entry or one file: the library then
/// holds the source's own file, so nothing is removed and the file is done.
///
/// The destination is read once, a link not followed, and the disk service's
/// entry alias answers from the two reads.
/// Where the reads carry nonzero file numbers on one known volume, neither
/// path is on a share and the destination is no link, distinct numbers are
/// two entries and the source is removed. Anywhere else the filesystem
/// settles it as an import's cleanup does: the source is set aside, the
/// destination read again, and the set-aside name removed only while the
/// destination still holds its file; otherwise the source is put back.
///
/// Probes, on this cleanup path only: one read of the destination; then
/// either one removal, or one rename, one read of the destination (a second
/// when it is a link), one listing of its folder unless both are numbered on
/// one volume off a share, and one removal, with one more rename to put the
/// source back when it is not removed.
async fn remove_copied_source(
    source: &Path,
    source_mark: &Mark,
    destination: &Path,
) -> AppResult<bool> {
    let disk = Disk::process();
    let destination_mark = disk
        .fingerprint(destination)
        .await
        .map_err(|reading| AppError::Repository(reading.absence().into()))?;
    let one_entry = || {
        tracing::info!(
            source = %source.display(),
            destination = %destination.display(),
            "a moved file's source and destination are one file; nothing was removed"
        );
        Ok(false)
    };
    if Disk::entry_alias_of(source, source_mark, destination, &destination_mark)
        == crate::disk::EntryAlias::Same
        || source_mark.same_file(&destination_mark) == Sameness::Same
    {
        return one_entry();
    }
    if !disk.adoption_needs_set_aside(source, source_mark, destination, &destination_mark) {
        return match disk.remove(source, Remove::File).await {
            Ok(Removal::Removed) => Ok(true),
            Ok(Removal::AlreadyAbsent) => Ok(false),
            Err(error) => Err(AppError::Repository(format!(
                "failed to remove the verified source {}: {error}",
                source.display()
            ))),
        };
    }
    use crate::disk::AsideRemoval;
    match disk
        .remove_source_beside(source, source_mark, destination, &destination_mark)
        .await
    {
        AsideRemoval::Removed => Ok(true),
        AsideRemoval::SameEntry => one_entry(),
        AsideRemoval::DestinationChanged => Err(AppError::validation_refused(
            crate::error_reasons::location::SOURCE_DESTINATION_CHANGED_BEFORE_CLEANUP,
            "Source or destination changed before cleanup; preserved the source.",
        )),
        AsideRemoval::PutBackFailed { at, error } | AsideRemoval::SetAsideMoved { at, error } => {
            tracing::error!(
                source = %source.display(),
                destination = %destination.display(),
                at = %at.display(),
                error = %error,
                "a move's verified source set aside to tell it from its copy could not be put back"
            );
            Err(set_aside_source_kept(source, &at))
        }
        AsideRemoval::AsideNameTaken { at, error } => Err(AppError::Repository(format!(
            "the verified source {} could not be set aside to tell it from {}: the name {} is taken: {error}; nothing was removed",
            source.display(),
            destination.display(),
            at.display()
        ))),
        AsideRemoval::DestinationUnread(reading) => Err(AppError::Repository(format!(
            "could not tell the verified source {} from {}: {}; nothing was removed",
            source.display(),
            destination.display(),
            reading.describe_absence()
        ))),
        AsideRemoval::CannotTell | AsideRemoval::Apart => Err(AppError::Repository(format!(
            "could not tell the verified source {} from {}; nothing was removed",
            source.display(),
            destination.display()
        ))),
        AsideRemoval::NotSetAside(error)
        | AsideRemoval::NotRemoved(error)
        | AsideRemoval::RemovalUnconfirmed(error) => Err(AppError::Repository(format!(
            "failed to remove the verified source {}: {error}",
            source.display()
        ))),
    }
}

/// Retire a source whose placement was a second link (or the same symlink), so
/// no content hashes were recorded. Missing already is retired. The source
/// goes only when the disk service reads both names as one regular file that
/// has another name and the listing shows two entries, or when both are
/// symbolic links to one target. A server that fakes a hard link with a
/// symbolic link, or a volume whose reads cannot tell files apart, keeps it.
#[allow(clippy::disallowed_methods)] // disk-guard: symlink target comparison and removal
async fn retire_linked_source(
    source: &Path,
    destination: &Path,
    source_path: &str,
) -> AppResult<()> {
    let source_read = match Disk::process().fingerprint(source).await {
        Ok(read) => read,
        Err(reading) if reading.exists() == Some(false) => return Ok(()),
        Err(reading) => return Err(AppError::Repository(reading.absence().into())),
    };
    let destination_read = Disk::process()
        .fingerprint(destination)
        .await
        .map_err(|reading| AppError::Repository(reading.absence().into()))?;
    let same_link = if source_read.kind() == Some(EntryKind::Symlink)
        && destination_read.kind() == Some(EntryKind::Symlink)
    {
        match (
            tokio::fs::read_link(source).await,
            tokio::fs::read_link(destination).await,
        ) {
            (Ok(source_target), Ok(destination_target)) => source_target == destination_target,
            _ => false,
        }
    } else {
        false
    };
    let same_file = same_regular_file(&source_read, &destination_read);
    if same_file
        && shared_inode(source, destination, &source_read, &destination_read).await
            != SharedInode::SecondLink
    {
        // Removing the source would remove the only entry.
        return Err(one_entry_refusal());
    }
    if !same_file && !same_link {
        return Err(AppError::validation_refused(
            crate::error_reasons::location::SOURCE_DESTINATION_CHANGED_BEFORE_CLEANUP,
            format!(
                "source or destination changed before cleanup: {}",
                source_path
            ),
        ));
    }
    tokio::fs::remove_file(source)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))
}

/// The one line a merge leaves in the title's outcome.
fn merge_note(
    title_name: &str,
    destination_title_name: Option<&str>,
    source_records_dropped: i64,
) -> String {
    let destination = destination_title_name
        .map(|name| format!("the existing \"{name}\""))
        .unwrap_or_else(|| "the existing title".to_string());
    let mut note = format!("\"{title_name}\" was merged into {destination}.");
    if source_records_dropped > 0 {
        let records = if source_records_dropped == 1 {
            "1 record".to_string()
        } else {
            format!("{source_records_dropped} records")
        };
        note.push_str(&format!(
            " {records} that only belonged to the duplicate entry (tags, requests, \
             acquisition state, discovery history) were retired with it."
        ));
    }
    note
}

/// One warning for every old folder cleanup left standing, rather than one per
/// folder: a stray file deep in a season folder keeps every folder above it
/// too, and the user wants to know what was kept, not to read the same
/// sentence for each level.
fn kept_folders_warning(kept: &[&str]) -> Option<String> {
    match kept {
        [] => None,
        [folder] => Some(format!(
            "The old folder {folder} was kept because it still has files in it that were not part of this move."
        )),
        folders => Some(format!(
            "{} old folders were kept because they still have files in them that were not part of this move: {}",
            folders.len(),
            folders.join(", ")
        )),
    }
}

/// Outcome of considering one source directory for removal.
#[derive(Debug)]
pub(super) enum DirectoryPrune {
    Removed,
    AlreadyAbsent,
    NotEmpty,
    Failed(String),
}

/// Remove `path` only when it is a directory and holds nothing.
///
/// `remove_dir` already refuses a non-empty directory, but reading the entries
/// first is what lets the caller *report* "still holds content" rather than an
/// errno the user cannot act on.
pub(super) async fn remove_directory_if_empty(path: &Path) -> DirectoryPrune {
    let disk = Disk::process();
    match disk.has_entries(path).await {
        Ok(false) => {}
        Ok(true) => return DirectoryPrune::NotEmpty,
        Err(error) if error.reason() == DISK_PATH_MISSING => return DirectoryPrune::AlreadyAbsent,
        Err(error) => return DirectoryPrune::Failed(error.to_string()),
    }
    match disk.remove(path, Remove::EmptyDir).await {
        Ok(Removal::Removed) => DirectoryPrune::Removed,
        Ok(Removal::AlreadyAbsent) => DirectoryPrune::AlreadyAbsent,
        Err(error) => DirectoryPrune::Failed(error.to_string()),
    }
}

// ── TitleAdmissionCheck ──────────────────────────────────────────────────────

/// The staleness rule for a root move.
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
        if self.plan.conflict.is_some() {
            self.catalog
                .admit_conflict(context.operation, self.plan)
                .await?;
            return Ok(TitleAdmission::Proceed);
        }
        let Some(planned) = self.plan.title(title_id) else {
            return Ok(TitleAdmission::Skip(format!(
                "title {title_id} is not in the confirmed plan"
            )));
        };

        let Some(placement) = self.catalog.title_placement(title_id).await? else {
            // A merge title whose source row is gone is this operation's
            // own completed Groups 1–5 transaction, not a foreign deletion —
            // it holds exclusive ownership of the title, so nothing else
            // could have removed it. Admitting it lets the reconciler recognize
            // the merge as already applied and settle the checkpoint that the
            // crash left behind.
            if let Some(destination_title_id) = planned.merge_target_title_id.as_deref()
                && self
                    .catalog
                    .title_placement(destination_title_id)
                    .await?
                    .is_some()
            {
                debug_assert!(!PlanInputChange::SettledSourceItem.is_stale());
                return Ok(TitleAdmission::Proceed);
            }
            return Ok(stale(
                PlanInputChange::CatalogInput,
                format!("title {title_id} no longer exists"),
            ));
        };

        // The catalog inputs the plan was built from. A transfer that already
        // flipped this title's library is this operation's own footprint, not a
        // foreign change — the same reasoning the root check below
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
                .is_none_or(|destination| {
                    !crate::stored_paths::folder_paths_match(destination, actual)
                })
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
        //
        // A deduplicated source counts as planned even though it moves no
        // bytes: the plan decided at preview time that its destination twin
        // already exists, so its catalog row is content the plan accounted for
        // and is about to recycle — not a file that appeared underneath it.
        let planned_media: BTreeSet<String> = planned
            .files
            .iter()
            .chain(self.plan.catalog_paths.get(title_id).into_iter().flatten())
            .filter(|file| file.media_file_id.is_some())
            .map(|file| file.source_path.clone())
            .collect();
        let planned_destinations: BTreeSet<String> = planned
            .files
            .iter()
            .chain(self.plan.catalog_paths.get(title_id).into_iter().flatten())
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

        // Unprocessed content must still be there. Content that is gone because
        // this operation already verified its destination is the operation's own
        // footprint, not a foreign change.
        for file in &planned.files {
            if context
                .verified_destinations
                .contains(&file.destination_path)
            {
                debug_assert!(!PlanInputChange::VerifiedDestinationFile.is_stale());
                continue;
            }
            let (path, detail) = (
                stored_path_to_path_buf(&file.source_path),
                format!(
                    "the source {} is gone and its destination was never verified",
                    file.source_path
                ),
            );
            if Disk::process().uncached(&path).await.exists() != Some(true) {
                return Ok(stale(PlanInputChange::UnprocessedSourceItem, detail));
            }
        }

        // The folder this title will record must be one it can own, asked
        // before any of its files move. A merge records no folder for it.
        if planned.merge_target_title_id.is_none()
            && let Some(folder) = planned.destination_folder_path.as_deref()
        {
            self.catalog
                .check_title_folder_path(title_id, folder)
                .await?;
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
