//! The use-case half of **change root** (US4, T060–T063, T065): the preview and
//! start entry points, and the tail sequence that retires the source location
//! and flips the root's configured path once every title has landed.
//!
//! The planning rules live in [`crate::location::root_change`] and are pure.
//! Everything here is the IO the planner refuses to do — the scan, the `stat`s,
//! the catalog reads, the path flip — plus the one piece of ordering FR-087
//! insists on and no other operation type needs.
//!
//! # The shape of a root change
//!
//! ```text
//! preview_root_change ─▶ every title assigned to the root (FR-023, no selection)
//!                        the source root's filesystem inventory (FR-027)
//!                        destination admissibility (FR-020)
//!                        ─▶ LocationPlan + RootMoveExecutionPlan + RootChangeTail
//!
//! start_root_change   ─▶ re-plan, compare fingerprints (FR-081), typed
//!                        confirmation (FR-029), persist, spawn the shared runner
//!
//! …the runner walks the titles exactly as it walks a root move…
//!
//! retire_changed_root ─▶ relocate the recycle bin that lived under the source
//!                        prune empty source directories (FR-028)
//!                        flip the root's path (FR-021, FR-078)
//!                        assert the identity post-conditions
//!                        mirror the legacy default-root settings
//! ```
//!
//! # Why the tail is here and not in the runner
//!
//! [`crate::location::executor::LocationOperationRunner`] is per-title by
//! construction: it walks a work plan, checkpoints each title, and knows nothing
//! about roots. A root change's last act is not about a title at all — it is one
//! configuration write that must happen after the *last* title recycled
//! (FR-087). Teaching the runner a root-scoped epilogue would put a US4 concept
//! in the one component every workflow shares; running it here, on the terminal
//! outcome, keeps the runner exactly as generic as it was.
//!
//! # The tail is shared with consolidation (US5)
//!
//! FR-020's **Change root** has two branches, and both end in the same
//! sequence: move the recycle bin that lived under the source, prune the empty
//! source directories, then — and only then, FR-087 — write the library's root
//! configuration. Only that last write differs, and
//! [`crate::location::root_change::RootChangeTail::consolidation`] selects it:
//! absent, the root's path is flipped (US4); present, the source root is removed
//! from the library's root list by
//! [`AppUseCase::retire_consolidated_root_configuration`] (US5). The runner is
//! given one epilogue either way.
//!
//! # Why every step of the tail is idempotent
//!
//! A restart can interrupt the tail as easily as it can interrupt a copy, and
//! the resumed run re-walks the remaining titles and then arrives here again.
//! So each step asks the filesystem or the catalog what is already true rather
//! than assuming it runs once: a bin that is no longer under the source has
//! already moved, a root whose stored path is already the destination has
//! already flipped, and an absent directory has already been pruned.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use scryer_domain::{LibraryPermission, Title, User};

use crate::location::classify::reason_codes;
use crate::location::execution::{DirectoryPrune, remove_directory_if_empty};
use crate::location::hardlinks::detect_hardlinks;
use crate::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationCounters, LocationOperationState,
    LocationOperationType,
};
use crate::location::operations::{
    LOCATION_OPERATION_VERIFICATION_DEPTH, LocationJobRunActor, LocationOperationAccepted,
    collect_source_files, confirmation_error,
};
use crate::location::ownership_guard::OwnedEntity;
use crate::location::preview::{
    FreeSpaceRequest, LocationPlan, PlanConfirmationRequest, SystemVolumeProbe, estimate_free_space,
};
use crate::location::root_change::{
    DestinationPathState, PlannedRootChange, RootChangePathFacts, RootChangePlanRequest,
    RootChangeTail, RootChangeTitleDraft, RootContentInventory, RootEntry, RootIdentityRetention,
    RootRetentionFacts, RootRetirementContract, TitleAccounting, build_root_change_plan,
    check_root_change_paths,
};
use crate::location::verify::same_filesystem;
use crate::services::AppUseCase;
use crate::settings::keys::SETTINGS_SOURCE_TYPED_GRAPHQL;
use crate::settings::runtime::root_folder_entries_from_library_roots;
use crate::stored_paths::stored_path_to_path_buf;
use crate::{AppError, AppResult};

/// What the caller asks to preview.
///
/// There is no title selection: FR-023 forbids one. The root *is* the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootChangePreviewRequest {
    pub library_id: String,
    /// The root's synthetic id (FR-078). It does not change.
    pub root_id: String,
    /// The new path. FR-020's first branch: a new, unconfigured location.
    pub destination_path: String,
    pub mode: LocationExecutionMode,
}

/// The confirmation a client sends back with the fingerprint it previewed and
/// the typed phrase FR-029 requires of a root-wide operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRootChangeRequest {
    pub library_id: String,
    pub root_id: String,
    pub destination_path: String,
    pub mode: LocationExecutionMode,
    pub confirmation: PlanConfirmationRequest,
}

/// Everything a root-change preview returns.
///
/// Beyond the shared fingerprinted plan: the every-title ledger FR-023 demands,
/// the identity statement FR-021 promises, the three content buckets of FR-027,
/// and the retirement contract of FR-028/FR-087.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootChangePreview {
    pub plan: LocationPlan,
    pub accounting: TitleAccounting,
    pub retention: RootIdentityRetention,
    pub content: RootContentInventory,
    pub retirement: RootRetirementContract,
    pub warnings: Vec<String>,
    /// Kept out of the client payload; start rebuilds it rather than trusting a
    /// round-trip (C2, FR-081).
    pub execution: crate::location::root_move::RootMoveExecutionPlan,
}

impl AppUseCase {
    /// Preview replacing one root's path (US4, FR-020–FR-029).
    pub async fn preview_root_change(
        &self,
        actor: &User,
        request: RootChangePreviewRequest,
    ) -> AppResult<RootChangePreview> {
        let planned = self.plan_root_change(actor, &request).await?;
        Ok(RootChangePreview {
            plan: planned.plan,
            accounting: planned.accounting,
            retention: planned.retention,
            content: planned.content,
            retirement: planned.retirement,
            warnings: planned.warnings,
            execution: planned.execution,
        })
    }

    /// Confirm a previewed root change and start it (FR-029, FR-030, FR-081).
    pub async fn start_root_change(
        &self,
        actor: &User,
        request: StartRootChangeRequest,
    ) -> AppResult<LocationOperationAccepted> {
        let planned = self
            .plan_root_change(
                actor,
                &RootChangePreviewRequest {
                    library_id: request.library_id.clone(),
                    root_id: request.root_id.clone(),
                    destination_path: request.destination_path.clone(),
                    mode: request.mode,
                },
            )
            .await?;
        let plan = planned.plan;

        // FR-029's typed phrase and FR-023's blocked titles both come out of
        // here: the plan header makes the confirmation stronger, and a blocked
        // title is a `NeedsResolution` classification, which `confirm` refuses
        // before anything is persisted.
        plan.confirm(&request.confirmation)
            .map_err(confirmation_error)?;

        let operation_id = scryer_domain::Id::new().0;
        // A root change with no titles at all is a legitimate request — an
        // empty root moving to a new disk — so, unlike a title selection, it is
        // never refused for having nothing to move. Activity still reports the
        // ledger, which is what makes "0 of 0 titles" readable.
        let titles_total = planned.accounting.assigned_total;
        let job_run = self
            .open_location_operation_job_run(
                &operation_id,
                titles_total,
                LocationJobRunActor::Confirmed(actor),
            )
            .await?;

        let now = chrono::Utc::now();
        let operation = LocationOperation {
            id: operation_id,
            operation_type: plan.header.operation_type,
            mode: plan.header.mode,
            state: LocationOperationState::Queued,
            initiated_by_user_id: Some(actor.id.clone()),
            source_library_id: plan.header.source_library_id.clone(),
            destination_library_id: plan.header.destination_library_id.clone(),
            // FR-021/FR-078: the same root on both sides. Activity reads a root
            // change as an operation on one root, which is what it is.
            source_root_id: plan.header.source_root_id.clone(),
            destination_root_id: plan.header.destination_root_id.clone(),
            plan_fingerprint: plan.fingerprint.0.clone(),
            verification_depth: plan.verification.depth,
            verification_fallback_count: 0,
            counters: LocationOperationCounters {
                titles_total,
                files_total: planned
                    .execution
                    .titles
                    .iter()
                    .map(|title| title.files.len() as i64)
                    .sum(),
                bytes_total: planned.execution.moved_bytes() as i64,
                no_ops: planned.execution.no_op_titles,
                unresolved: planned.execution.unresolved_titles,
                ..LocationOperationCounters::default()
            },
            detail: None,
            job_run_id: Some(job_run.id.clone()),
            workflow_operation_id: None,
            cancel_requested: false,
            cancel_requested_at: None,
            confirmed_at: Some(now),
            started_at: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        let persisted = async {
            // The tail rides inside this JSON (see `RootChangeTail`): resume
            // needs the recycle allowlist and cleanup needs the directory list,
            // and neither can be re-derived once the path has flipped.
            let plan_json = serde_json::to_string(&planned.execution)
                .map_err(|error| AppError::Repository(error.to_string()))?;
            self.services
                .library
                .location_operations
                .create_location_operation(&operation, Some(&plan_json))
                .await
        }
        .await;
        if let Err(error) = persisted {
            self.close_location_operation_job_run(
                &job_run,
                crate::JobRunStatus::Failed,
                "The root change could not be started.".to_string(),
                Some(error.to_string()),
                None,
            )
            .await;
            return Err(error);
        }

        self.spawn_location_operation(operation.id.clone(), planned.execution);

        Ok(LocationOperationAccepted { operation, plan })
    }

    /// Assemble every fact [`build_root_change_plan`] needs, then plan.
    async fn plan_root_change(
        &self,
        actor: &User,
        request: &RootChangePreviewRequest,
    ) -> AppResult<PlannedRootChange> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&request.library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {}", request.library_id)))?;
        let root = library
            .roots
            .iter()
            .find(|root| root.id == request.root_id)
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "root {} in library {}",
                    request.root_id, request.library_id
                ))
            })?
            .clone();

        // FR-083, before any filesystem work is planned.
        self.require_library_permission(actor, &library.id, LibraryPermission::ManageTitles)
            .await?;

        let source_root = stored_path_to_path_buf(root.path.trim());
        let destination_path = request.destination_path.trim();
        if destination_path.is_empty() {
            return Err(AppError::Validation(
                "choose a new path for this root".to_string(),
            ));
        }
        // Lexically normalized only: `.` and `..` are resolved so the stored
        // configuration is predictable, while symlinks and platform aliases are
        // left exactly as the user gave them.
        let destination_root = lexically_normalize(&stored_path_to_path_buf(destination_path));

        // FR-020's admissibility rules, asked of the filesystem here and asked
        // again when the operation is admitted, because the destination is an
        // unmanaged path that anything could write to in between.
        let facts = self
            .root_change_path_facts(
                &request.root_id,
                &source_root,
                &destination_root,
                request.mode,
            )
            .await?;
        check_root_change_paths(&facts).map_err(|refusal| AppError::LocationRootRefused {
            message: refusal.detail,
            code: refusal.code,
        })?;

        // FR-023: every title assigned to the root, in a stable order. Not a
        // selection — there is no way to express one.
        let mut titles: Vec<Title> = self
            .services
            .catalog
            .titles
            .list_for_libraries(None, std::slice::from_ref(&library.id), None)
            .await?
            .into_iter()
            .filter(|title| title.root_folder_id == request.root_id)
            .collect();
        titles.sort_by(|left, right| left.id.cmp(&right.id));

        let mut drafts = Vec::with_capacity(titles.len());
        let mut moved_bytes = 0_u64;
        for title in &titles {
            let media_files = self
                .services
                .library
                .media_files
                .list_media_files_for_title(&title.id)
                .await?;
            let source_folder_path = title
                .folder_path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(stored_path_to_path_buf);

            // The same two blockers `classify_title` reads, from the same
            // sources: an active download or import (FR-086), and another
            // operation already owning the title (FR-084).
            let (blocked_reason, blocked_reason_code) =
                match self.active_work_blocking_a_move(title).await? {
                    Some(detail) => (
                        Some(detail),
                        Some(reason_codes::ACTIVE_DOWNLOAD_OR_IMPORT.to_string()),
                    ),
                    None => match self
                        .services
                        .library
                        .location_operations
                        .location_ownership_holder(&OwnedEntity::Title(title.id.clone()))
                        .await?
                    {
                        Some(operation_id) => (
                            Some(format!(
                                "\"{}\" is already owned by location operation {operation_id}",
                                title.name
                            )),
                            Some(reason_codes::OWNED_BY_LOCATION_OPERATION.to_string()),
                        ),
                        None => (None, None),
                    },
                };

            let (files, source_directories) = if blocked_reason.is_some() {
                // A blocked title never enters the operation, so walking its
                // folder would be work whose only product is a plan item the
                // planner discards. It is still counted and named (FR-023).
                (Vec::new(), Vec::new())
            } else {
                collect_source_files(source_folder_path.as_deref(), &media_files).await?
            };
            let hardlinks =
                detect_hardlinks(files.iter().map(|file| file.path.clone()).collect()).await?;
            moved_bytes = moved_bytes.saturating_add(
                files
                    .iter()
                    .fold(0_u64, |total, file| total.saturating_add(file.size_bytes)),
            );

            drafts.push(RootChangeTitleDraft {
                title_id: title.id.clone(),
                title_name: title.name.clone(),
                source_folder_path,
                files,
                source_directories,
                hardlinks,
                blocked_reason,
                blocked_reason_code,
            });
        }

        let same_volume = Some(same_filesystem(&source_root, &destination_root).await);
        let recycle_config = self
            .recycle_bin_config_for_media_root(Some(root.path.trim()))
            .await;

        // FR-027's inventory: everything under the source root, so the planner
        // can put each entry in exactly one of the three buckets — except
        // Scryer's own recycle bin, which is not content the catalog failed to
        // explain. Left in, it would report every recycled file as unexplained,
        // block the source location's retirement on Scryer's own bookkeeping,
        // and put the bin's contents in the fingerprint so that any background
        // purge voided the user's confirmation. The bin is accounted for by the
        // tail instead, which moves it (see
        // `relocate_recycle_bin_for_root_change`).
        let entries = scan_root_entries(&source_root, Some(&recycle_config.base_path)).await?;
        let free_space = estimate_free_space(
            &FreeSpaceRequest {
                source_path: source_root.clone(),
                destination_path: destination_root.clone(),
                // A same-volume root change renames; nothing is written twice.
                moved_bytes: if same_volume == Some(true) {
                    0
                } else {
                    moved_bytes
                },
                recycled_bytes: if same_volume == Some(true) {
                    0
                } else {
                    moved_bytes
                },
                recycle_base_path: recycle_config
                    .enabled
                    .then(|| recycle_config.base_path.clone()),
            },
            &SystemVolumeProbe,
        );

        let mut planned = build_root_change_plan(&RootChangePlanRequest {
            library_id: library.id.clone(),
            root_id: request.root_id.clone(),
            // The *configured* source path and the path the user typed, not the
            // canonicalized forms the admissibility check compared. Every title
            // folder and every media-file row in the catalog is stored against
            // the configured path, and on macOS the canonical form of `/var/x`
            // is `/private/var/x` — re-anchoring against the resolved form
            // would fail to strip a single prefix and place every file "outside
            // the root being changed". Canonicalization answers "do these two
            // paths overlap?"; stored paths are what the catalog is written in.
            source_root_path: source_root.clone(),
            destination_root_path: destination_root.clone(),
            mode: request.mode,
            retention: RootRetentionFacts {
                is_library_default: root.is_default,
                role: None,
            },
            titles: drafts,
            entries,
            // Forced full for every location operation; the operator's
            // depth preference governs import copies only
            // (`LOCATION_OPERATION_VERIFICATION_DEPTH`).
            verification_depth: LOCATION_OPERATION_VERIFICATION_DEPTH,
            free_space,
            same_volume,
        });
        planned.execution.root_change = Some(planned.tail(
            &library.id,
            titles.iter().map(|title| title.id.clone()).collect(),
        ));
        Ok(planned)
    }

    /// The `stat`s FR-020's rules are asked about.
    async fn root_change_path_facts(
        &self,
        root_id: &str,
        source_root: &Path,
        destination_root: &Path,
        mode: LocationExecutionMode,
    ) -> AppResult<RootChangePathFacts> {
        let source_metadata = tokio::fs::symlink_metadata(source_root).await;
        let source_root_is_symlink = source_metadata
            .as_ref()
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        let source_root_is_directory = tokio::fs::metadata(source_root)
            .await
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);

        let destination = match tokio::fs::metadata(destination_root).await {
            Ok(metadata) if metadata.is_dir() => DestinationPathState::Directory {
                empty: directory_is_empty(destination_root).await,
            },
            Ok(_) => DestinationPathState::NotADirectory,
            Err(_) => DestinationPathState::Missing {
                parent_exists: match destination_root.parent() {
                    Some(parent) => tokio::fs::metadata(parent)
                        .await
                        .map(|metadata| metadata.is_dir())
                        .unwrap_or(false),
                    None => false,
                },
            },
        };

        let mut configured_roots = Vec::new();
        for library in self.services.catalog.libraries.list(None).await? {
            for root in library.roots {
                let path = root.path.trim();
                if path.is_empty() {
                    continue;
                }
                configured_roots.push((
                    root.id,
                    canonical_or_lexical(&stored_path_to_path_buf(path)).await,
                ));
            }
        }

        Ok(RootChangePathFacts {
            source_root: canonical_or_lexical(source_root).await,
            destination_root: canonical_or_lexical(destination_root).await,
            source_root_is_symlink,
            source_root_is_directory,
            destination,
            configured_roots,
            root_id: root_id.to_string(),
            mode,
        })
    }

    /// FR-087's tail: retire the source location, then the configuration.
    ///
    /// Bound to the runner as its [`OperationEpilogue`], so it runs after the
    /// last title has moved, verified, reconciled and recycled — and *before*
    /// the operation is reported finished, while it still owns its titles and
    /// its root (FR-084). Returns the warnings the retirement raised; an `Err`
    /// fails the operation.
    ///
    /// Every step asks what is already true, because a resumed run reaches this
    /// point again.
    async fn retire_changed_root(&self, tail: &RootChangeTail) -> AppResult<Vec<String>> {
        let mut warnings: Vec<String> = Vec::new();

        // 1. The bin travels with the operation. Before the prune (it lives
        //    under the source root, so it would block the source's removal) and
        //    before the flip (after it, housekeeping would never look there
        //    again).
        if let Err(warning) = self.relocate_recycle_bin_for_root_change(tail).await {
            warnings.push(warning);
        }

        // 2. FR-028: empty source directories only, deepest first, and only
        //    what the confirmed plan named.
        warnings.extend(self.retire_source_location(tail).await);

        // 3. The last step, and the only one FR-020's two branches do not share
        //    (see `RootChangeTail::consolidation`):
        //
        //    - a **root change** repoints its root at the new path, keeping the
        //      root's identity, role, and default status (FR-021, FR-078);
        //    - a **consolidation** removes the source root's configuration
        //      entirely, because its titles now live on a different, already
        //      configured root (FR-020, FR-022).
        //
        // A failure here is the operation's failure either way. The bytes are at
        // the destination and the catalog points at them, but the library's root
        // configuration still describes the world before the operation — a state
        // no later pass repairs on its own, and one that would be a lie to
        // report as a completed move.
        let configuration = match tail.consolidation.as_ref() {
            Some(consolidation) => {
                self.retire_consolidated_root_configuration(tail, consolidation)
                    .await
            }
            None => self.flip_changed_root_path(tail).await,
        };
        let assertions = configuration.map_err(|error| {
            tracing::error!(
                root_id = %tail.root_id,
                error = %error,
                "a root-scoped operation moved its content but could not write the library's root configuration"
            );
            AppError::Repository(format!(
                "the content moved to {} but the library's root configuration could not be updated: {error}",
                tail.destination_root_path
            ))
        })?;
        warnings.extend(assertions);

        Ok(warnings)
    }

    /// Move the recycle bin that lived under the source root to the matching
    /// place under the destination, and re-anchor the entries inside it.
    ///
    /// # Why the bin moves at all
    ///
    /// The bin's default location is `<root>/.scryer-recycle`
    /// (`recycle_bin_config_from_path_values`), i.e. *inside* the root. Housekeeping
    /// enumerates configured library roots to find bins to sweep and to restore
    /// from, so a bin left at a path that is no longer a configured root would
    /// never be purged and never be restorable — every file this operation
    /// recycled would be stranded, silently, forever. Moving it is what keeps
    /// the retention policy and the restore affordance working across a root
    /// change.
    ///
    /// A bin configured to a **custom** path outside the source root is shared
    /// installation-wide and is not this root's to move; it stays where it is,
    /// and its entries are re-anchored in place instead so a pre-flip entry is
    /// still restorable.
    ///
    /// `Err` is a warning, never a failure: the content has already moved and
    /// been verified, and losing the operation over a bin that could not be
    /// relocated would be worse than naming the path it was left at.
    async fn relocate_recycle_bin_for_root_change(
        &self,
        tail: &RootChangeTail,
    ) -> Result<(), String> {
        let source_root = tail.source_root();
        let destination_root = tail.destination_root();
        let config = self
            .recycle_bin_config_for_media_root(Some(&tail.source_root_path))
            .await;
        let bin = config.base_path.clone();

        // A custom bin outside the root does not travel, but its entries still
        // record source-root paths that restore would refuse after the flip.
        let Some(destination_bin) = tail
            .rebase_onto_destination(&bin)
            .filter(|_| bin != source_root)
        else {
            return match crate::recycle_bin::reanchor_recycled_entries(
                &bin,
                &source_root,
                &destination_root,
            )
            .await
            {
                Ok(_) => Ok(()),
                Err(error) => Err(format!(
                    "the recycle bin at {} could not be re-anchored onto the new root path, so entries recycled during this operation may not restore: {error}",
                    bin.display()
                )),
            };
        };

        if !path_exists(&bin).await {
            // Nothing was recycled — a same-volume root change renames and
            // leaves no source copies — or a previous attempt already moved it.
            return Ok(());
        }

        move_directory_tree(&bin, &destination_bin)
            .await
            .map_err(|error| {
                format!(
                    "the recycle bin at {} could not be moved to {}, so entries recycled during this operation are still at the old location: {error}",
                    bin.display(),
                    destination_bin.display()
                )
            })?;

        crate::recycle_bin::reanchor_recycled_entries(
            &destination_bin,
            &source_root,
            &destination_root,
        )
        .await
        .map_err(|error| {
            format!(
                "the recycle bin moved to {} but its entries could not be re-anchored onto the new root path, so they may not restore: {error}",
                destination_bin.display()
            )
        })?;
        Ok(())
    }

    /// FR-028: remove the empty directories the confirmed plan named, and the
    /// source root itself only when nothing unexplained is left standing.
    async fn retire_source_location(&self, tail: &RootChangeTail) -> Vec<String> {
        let mut warnings = Vec::new();

        for directory in &tail.retirement.removable_directories {
            let path = stored_path_to_path_buf(directory);
            match remove_directory_if_empty(&path).await {
                DirectoryPrune::Removed | DirectoryPrune::AlreadyAbsent => {}
                // Best effort by design: these are root-level directories the
                // titles did not own, and one that turned out to hold something
                // is one FR-027 wants left exactly as it is.
                DirectoryPrune::NotEmpty | DirectoryPrune::Failed(_) => tracing::debug!(
                    directory = %directory,
                    "a source directory a root change offered to prune was left in place"
                ),
            }
        }

        if !tail.retirement.permits_source_removal() {
            for blocker in &tail.retirement.blockers {
                warnings.push(blocker.detail.clone());
            }
            return warnings;
        }

        match remove_directory_if_empty(&tail.source_root()).await {
            DirectoryPrune::Removed | DirectoryPrune::AlreadyAbsent => {}
            DirectoryPrune::NotEmpty => warnings.push(format!(
                "{} still holds content, so the old location was left in place",
                tail.source_root_path
            )),
            // A root that is its own mount point cannot be removed and should
            // not be: that is a normal, healthy outcome, not something to make
            // the user read.
            DirectoryPrune::Failed(error) => tracing::info!(
                source_root = %tail.source_root_path,
                error = %error,
                "the old root location was left in place"
            ),
        }
        warnings
    }

    /// The path flip and its post-conditions (FR-021, FR-078, US4.2).
    ///
    /// Returns the warnings the assertions raised; an empty vector is the
    /// expected result.
    async fn flip_changed_root_path(&self, tail: &RootChangeTail) -> AppResult<Vec<String>> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&tail.library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {}", tail.library_id)))?;
        let root = library
            .roots
            .iter()
            .find(|root| root.id == tail.root_id)
            .ok_or_else(|| AppError::NotFound(format!("library root {}", tail.root_id)))?;

        let already_flipped = scryer_domain::normalize_library_root_path(root.path.trim())
            == scryer_domain::normalize_library_root_path(tail.destination_root_path.trim());
        let library = if already_flipped {
            // A resumed run reaching the tail a second time.
            library
        } else {
            self.services
                .catalog
                .libraries
                .set_root_path(&tail.root_id, &tail.destination_root_path)
                .await?
        };

        let mut warnings = Vec::new();
        let Some(root) = library.roots.iter().find(|root| root.id == tail.root_id) else {
            // Unreachable through `set_root_path`, which never deletes a row —
            // which is exactly why it is asserted rather than assumed.
            warnings.push(format!(
                "root {} no longer exists after its path was changed",
                tail.root_id
            ));
            return Ok(warnings);
        };
        if scryer_domain::normalize_library_root_path(root.path.trim())
            != scryer_domain::normalize_library_root_path(tail.destination_root_path.trim())
        {
            warnings.push(format!(
                "root {} reads back as {} rather than {}",
                tail.root_id, root.path, tail.destination_root_path
            ));
        }
        if root.is_default != tail.retention.remains_library_default {
            warnings.push(format!(
                "root {} {} the library default across the path change",
                tail.root_id,
                if root.is_default {
                    "unexpectedly became"
                } else {
                    "unexpectedly stopped being"
                }
            ));
        }

        // FR-021's last post-condition: every title still points at this root.
        let retained = self
            .services
            .catalog
            .titles
            .list_for_libraries(None, std::slice::from_ref(&library.id), None)
            .await?
            .into_iter()
            .filter(|title| title.root_folder_id == tail.root_id)
            .count() as i64;
        if retained != tail.retention.retained_title_assignments {
            warnings.push(format!(
                "root {} holds {retained} title assignment(s) after the path change, not the {} the preview promised",
                tail.root_id, tail.retention.retained_title_assignments
            ));
        }

        for warning in &warnings {
            tracing::error!(
                root_id = %tail.root_id,
                warning = %warning,
                "a root change's identity post-condition did not hold"
            );
        }

        // Compatibility plumbing, not spec: the legacy per-facet root-folder
        // settings keys mirror the default library's roots, and nothing else in
        // the location subsystem changes a root's path, so nothing else would
        // update them. A stale mirror would point scanning and import at the
        // retired location.
        if library.is_default
            && let Err(error) = self
                .mirror_default_library_roots_to_legacy_settings(
                    &library.facet,
                    &root_folder_entries_from_library_roots(&library.roots),
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    None,
                )
                .await
        {
            warnings.push(format!(
                "the legacy root-folder settings still name the old path: {error}"
            ));
        }

        Ok(warnings)
    }
}

/// Binds [`AppUseCase::retire_changed_root`] onto the runner's epilogue seam.
///
/// Borrowed rather than owned so the runner keeps a plain `&dyn` for the whole
/// run, the way its mover and reconciler are bound.
pub(super) struct RootChangeEpilogue<'a> {
    pub(super) app: &'a AppUseCase,
    pub(super) tail: &'a RootChangeTail,
}

#[async_trait::async_trait]
impl crate::location::executor::OperationEpilogue for RootChangeEpilogue<'_> {
    async fn finish_operation(&self, _operation: &LocationOperation) -> AppResult<Vec<String>> {
        self.app.retire_changed_root(self.tail).await
    }
}

// ── Filesystem helpers ───────────────────────────────────────────────────────

/// The source root's inventory, as [`crate::location::root_change::classify_root_content`]
/// wants it: one entry per directory and one per file, beneath the root.
pub(super) async fn scan_root_entries(
    source_root: &Path,
    recycle_bin: Option<&Path>,
) -> AppResult<Vec<RootEntry>> {
    let root = source_root.to_path_buf();
    let walked = tokio::task::spawn_blocking({
        let root = root.clone();
        move || {
            crate::library::filesystem_walk::FilesystemWalker::new()
                .skip_unreadable_subdirectories()
                .skip_symlinked_directories()
                .confine_to_root()
                .walk(&root)
        }
    })
    .await
    .map_err(|error| AppError::Repository(format!("root scan task panicked: {error}")))??;

    let excluded = |path: &Path| -> bool {
        recycle_bin.is_some_and(|bin| path == bin || path.starts_with(bin))
    };

    let mut entries = Vec::new();
    let mut sizes: BTreeMap<PathBuf, u64> = BTreeMap::new();
    for listing in walked {
        if excluded(&listing.path) {
            continue;
        }
        if listing.path != root {
            entries.push(RootEntry::directory(listing.path.clone()));
        }
        for path in listing.files {
            if excluded(&path) {
                continue;
            }
            let size = tokio::fs::symlink_metadata(&path)
                .await
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            sizes.insert(path, size);
        }
    }
    for (path, size) in sizes {
        entries.push(RootEntry::file(path, size));
    }
    Ok(entries)
}

async fn path_exists(path: &Path) -> bool {
    tokio::fs::symlink_metadata(path).await.is_ok()
}

async fn directory_is_empty(path: &Path) -> bool {
    match tokio::fs::read_dir(path).await {
        Ok(mut entries) => matches!(entries.next_entry().await, Ok(None)),
        Err(_) => false,
    }
}

/// The path as the filesystem resolves it, or lexically normalized when it does
/// not exist yet.
///
/// A root change compares a configured path against a path the user typed, and
/// `/mnt/media` and `/mnt/./media` are the same directory. Resolving what exists
/// and normalizing what does not is what makes "these two overlap" and "this is
/// already a configured root" answerable.
pub(super) async fn canonical_or_lexical(path: &Path) -> PathBuf {
    if let Ok(resolved) = tokio::fs::canonicalize(path).await {
        return resolved;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(resolved) = tokio::fs::canonicalize(parent).await
    {
        return resolved.join(name);
    }
    lexically_normalize(path)
}

fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

/// Move a whole directory to `to`, across filesystems if necessary.
///
/// A root change's destination is very often a different disk — that is the
/// story — so `rename` is the fast path and never the only one. When `to`
/// already exists (a resumed run that got half way) the trees are merged rather
/// than replaced, so nothing that already arrived is lost.
async fn move_directory_tree(from: &Path, to: &Path) -> AppResult<()> {
    if !path_exists(from).await {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            AppError::Repository(format!(
                "failed to create {}: {error}",
                parent.display()
            ))
        })?;
    }
    if !path_exists(to).await {
        match tokio::fs::rename(from, to).await {
            Ok(()) => return Ok(()),
            Err(error) if crate::fs_safety::is_cross_device_error(&error) => {}
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "failed to move {} to {}: {error}",
                    from.display(),
                    to.display()
                )));
            }
        }
    }

    copy_directory_tree(from, to).await?;
    tokio::fs::remove_dir_all(from).await.map_err(|error| {
        AppError::Repository(format!(
            "the contents of {} were copied to {} but the old directory could not be removed: {error}",
            from.display(),
            to.display()
        ))
    })
}

/// Copy `from` onto `to`, creating directories and overwriting nothing that is
/// not already a copy of what is being written.
///
/// Iterative rather than recursive: an `async fn` that called itself would need
/// boxing on every directory, and a recycle bin is one flat level of entry
/// directories in practice.
async fn copy_directory_tree(from: &Path, to: &Path) -> AppResult<()> {
    let mut work = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((source, destination)) = work.pop() {
        tokio::fs::create_dir_all(&destination)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to create {}: {error}",
                    destination.display()
                ))
            })?;
        let mut entries = tokio::fs::read_dir(&source).await.map_err(|error| {
            AppError::Repository(format!("failed to read {}: {error}", source.display()))
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            AppError::Repository(format!("failed to walk {}: {error}", source.display()))
        })? {
            let file_type = entry.file_type().await.map_err(|error| {
                AppError::Repository(format!(
                    "failed to stat {}: {error}",
                    entry.path().display()
                ))
            })?;
            let target = destination.join(entry.file_name());
            if file_type.is_dir() {
                work.push((entry.path(), target));
            } else if file_type.is_file() {
                tokio::fs::copy(entry.path(), &target).await.map_err(|error| {
                    AppError::Repository(format!(
                        "failed to copy {} to {}: {error}",
                        entry.path().display(),
                        target.display()
                    ))
                })?;
            }
            // Symlinks inside a recycle bin are not content Scryer put there
            // and are deliberately not recreated at the new location.
        }
    }
    Ok(())
}

/// The operation types the shared root-move runner is allowed to resume.
///
/// A root change resumes through it because it *is* a root move in plan
/// currency: the same instruction set, the same checkpoints, the same
/// reconciler — plus a tail that re-runs harmlessly.
pub(super) fn resumes_through_root_move_runner(
    operation_type: LocationOperationType,
) -> bool {
    matches!(
        operation_type,
        LocationOperationType::RootMove
            | LocationOperationType::RootChange
            | LocationOperationType::RootConsolidation
            | LocationOperationType::CrossLibraryTransfer
            | LocationOperationType::Adoption
    )
}
