//! The use-case half of **consolidate root** (US5, T071): the preview and start
//! entry points, and the last step of the shared root-scoped tail — retiring the
//! source root's *configuration* once every title has landed on the destination
//! root.
//!
//! The planning rules live in [`crate::location::consolidation`] and are pure.
//! Everything here is the IO the planner refuses to do: the two scans, the
//! `stat`s, the identity detection, the merge snapshots, and the one
//! configuration write FR-020's second branch ends in.
//!
//! # The shape of a consolidation
//!
//! ```text
//! preview_root_consolidation ─▶ every title assigned to the source root
//!                               (FR-023, no selection)
//!                               destination-title identity for each of them
//!                               (FR-055) and its merge plan (FR-066/071)
//!                               the destination root's folder occupancy
//!                               folder resolution (FR-025/FR-026)
//!                               the destination folders' listings (FR-072–075)
//!                               the source root's inventory (FR-027)
//!                               ─▶ LocationPlan + RootMoveExecutionPlan
//!                                  + RootChangeTail{consolidation}
//!
//! start_root_consolidation   ─▶ re-plan, compare fingerprints (FR-081), typed
//!                               confirmation (FR-029), persist, spawn the
//!                               shared runner
//!
//! …the runner walks the titles exactly as it walks a root move: move, verify,
//!   reconcile — and, for a title that merges, run the merge engine's Groups 1–5
//!   transaction instead of the plain root flip…
//!
//! retire_consolidated_root_configuration
//!                            ─▶ (after the shared tail moved the recycle bin
//!                                and pruned the empty source directories)
//!                               remove the source root from the library's root
//!                               list, transfer the default (FR-022), assert the
//!                               post-conditions, mirror the legacy settings
//! ```
//!
//! # Why so little of this is new
//!
//! A consolidation is a root change whose destination happens to be another
//! configured root. Everything about *how* content moves — the mover, the
//! verifier, the reconciler, the recycler, the checkpoints, resume, the
//! ownership guard, the media-server refresh — is the shared runner, unchanged.
//! Everything about *retiring the source location* — the travelling recycle bin,
//! the empty-directory prune, the "configuration last, after all recycling"
//! ordering of FR-087 — is [`crate::location::root_change_execution`]'s tail,
//! reused by carrying a [`ConsolidationTail`] inside
//! [`RootChangeTail`]. What is genuinely new is the last step, and it is below.
//!
//! # "Retiring the configuration", concretely
//!
//! FR-087 says the source root's configuration is retired only after all
//! recycling completes; it does not say what retiring it *is*. For a root change
//! WP43 answered "flip the path" — the root survives, pointing somewhere else.
//! For a consolidation the root does not survive at all: its titles now belong
//! to a root that already existed, so the source root is removed from the
//! library's root list.
//!
//! The mechanism is [`crate::ports::LibraryRepository::update`], the existing
//! replace-on-write of a library's root list, and it is the right one here for
//! the same reasons it was the wrong one for a root change:
//!
//! - it re-keys root identity **by normalized path**, so every root whose path
//!   is resubmitted keeps its synthetic id (FR-078) — including the destination
//!   root, which is what the post-conditions assert;
//! - a root absent from the submitted list is a *removed* root, which is exactly
//!   the intent here;
//! - `reject_referenced_root_removals_tx` refuses to remove a root any title
//!   still references. That guard is a feature: it is the catalog's own
//!   statement of "titles first, configuration last", and this code asks the
//!   same question itself first so the user gets a sentence rather than a
//!   constraint violation.
//!
//! # What blocks the removal (FR-028)
//!
//! > **FR-028**: A root MUST NOT be removed while unexplained content remains at
//! > the source.
//!
//! For a root change that sentence governs a directory. Here it governs the
//! configured root itself, which is the literal reading. So a consolidation over
//! a source root holding files Scryer cannot explain still moves every title —
//! nothing about the destination changes — but leaves the source root
//! configured, with a warning naming what has to be resolved. The default
//! transfer (FR-022) happens anyway: which root new content lands on is a
//! separate question from whether an old root can be deleted, and leaving the
//! default on a root the user has just emptied would be the worse answer.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use scryer_domain::{LibraryPermission, Title, User};

use crate::LibraryRootDraft;

use crate::location::classify::reason_codes;
use crate::location::collisions::{
    CollisionNaming, ContentFacts, DestinationItem, FullHash, PathCaseRule, RecycleAvailability,
};
use crate::location::consolidation::{
    ConsolidationClassification, ConsolidationPathFacts, ConsolidationTail,
    ConsolidationTitleDraft, DefaultRootTransfer, DestinationFolderState, FolderResolutionRequest,
    FolderResolutionTitle, PlannedRootConsolidation, ResolvedFolder, RootConsolidationPlanRequest,
    build_root_consolidation_plan, check_consolidation_paths, resolve_consolidation_folders,
};
use crate::location::hardlinks::detect_hardlinks;
use crate::location::identity::{
    DestinationIdentityOutcome, DestinationTitleCandidate, IdentityRedirects, SourceTitleIdentity,
    detect_destination_title,
};
use crate::location::merge::engine::plan_merge;
use crate::location::merge::summary::MergePreviewSummary;
use crate::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationCounters, LocationOperationState,
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
    RootChangeTail, RootContentInventory, RootIdentityRetention, RootRetirementContract,
    TitleAccounting,
};
use crate::location::root_change_execution::{canonical_or_lexical, scan_root_entries};
use crate::location::verify::same_filesystem;
use crate::services::AppUseCase;
use crate::settings::keys::SETTINGS_SOURCE_TYPED_GRAPHQL;
use crate::settings::runtime::root_folder_entries_from_library_roots;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppError, AppResult};

/// What the caller asks to preview.
///
/// Two root ids, one library. There is no title selection: FR-023 forbids one,
/// and there is no destination *path* either — the destination is an existing
/// root, which is what makes this consolidation rather than a root change
/// (FR-020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootConsolidationPreviewRequest {
    pub library_id: String,
    /// The root being folded away.
    pub source_root_id: String,
    /// The root that absorbs it. Must already be configured in this library.
    pub destination_root_id: String,
    pub mode: LocationExecutionMode,
}

/// The confirmation a client sends back with the fingerprint it previewed and
/// the typed phrase FR-029 requires of a root-wide operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRootConsolidationRequest {
    pub library_id: String,
    pub source_root_id: String,
    pub destination_root_id: String,
    pub mode: LocationExecutionMode,
    pub confirmation: PlanConfirmationRequest,
}

/// Everything a consolidation preview returns.
///
/// Beyond the shared fingerprinted plan: the every-title ledger FR-023 demands,
/// FR-024's seven classifications, FR-022's default-root statement, the three
/// content buckets of FR-027, and the retirement contract of FR-028/FR-087.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootConsolidationPreview {
    pub plan: LocationPlan,
    pub accounting: TitleAccounting,
    pub classification: ConsolidationClassification,
    pub default_transfer: DefaultRootTransfer,
    pub content: RootContentInventory,
    pub retirement: RootRetirementContract,
    pub warnings: Vec<String>,
    /// Kept out of the client payload; start rebuilds it rather than trusting a
    /// round-trip (C2, FR-081).
    pub execution: crate::location::root_move::RootMoveExecutionPlan,
}

impl AppUseCase {
    /// Preview folding one root into another root of the same library (US5,
    /// FR-020, FR-022, FR-024–FR-029).
    pub async fn preview_root_consolidation(
        &self,
        actor: &User,
        request: RootConsolidationPreviewRequest,
    ) -> AppResult<RootConsolidationPreview> {
        let (planned, _) = self.plan_root_consolidation(actor, &request).await?;
        Ok(RootConsolidationPreview {
            plan: planned.plan,
            accounting: planned.accounting,
            classification: planned.classification,
            default_transfer: planned.default_transfer,
            content: planned.content,
            retirement: planned.retirement,
            warnings: planned.warnings,
            execution: planned.execution,
        })
    }

    /// Confirm a previewed consolidation and start it (FR-029, FR-030, FR-081).
    pub async fn start_root_consolidation(
        &self,
        actor: &User,
        request: StartRootConsolidationRequest,
    ) -> AppResult<LocationOperationAccepted> {
        let (planned, _) = self
            .plan_root_consolidation(
                actor,
                &RootConsolidationPreviewRequest {
                    library_id: request.library_id.clone(),
                    source_root_id: request.source_root_id.clone(),
                    destination_root_id: request.destination_root_id.clone(),
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
        // A consolidation of an empty root is a legitimate request — folding
        // away a root nothing lives on — so, unlike a title selection, it is
        // never refused for having nothing to move.
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
            // Two real, different roots in one library (FR-020). Activity reads
            // a consolidation as an operation between them, which is what it is.
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
            // The tail rides inside this JSON: resume needs the recycle
            // allowlist and the retirement contract, and neither can be
            // re-derived once the source root has left the configuration.
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
                "The consolidation could not be started.".to_string(),
                Some(error.to_string()),
                None,
            )
            .await;
            return Err(error);
        }

        self.spawn_location_operation(operation.id.clone(), planned.execution);

        Ok(LocationOperationAccepted { operation, plan })
    }

    /// Assemble every fact [`build_root_consolidation_plan`] needs, then plan.
    ///
    /// Returns the plan and the tail that must ride on the persisted execution
    /// plan; `start` attaches it, `preview` discards it.
    async fn plan_root_consolidation(
        &self,
        actor: &User,
        request: &RootConsolidationPreviewRequest,
    ) -> AppResult<(PlannedRootConsolidation, RootChangeTail)> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&request.library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {}", request.library_id)))?;
        let source_root = library
            .roots
            .iter()
            .find(|root| root.id == request.source_root_id)
            .cloned()
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "root {} in library {}",
                    request.source_root_id, request.library_id
                ))
            })?;
        let destination_root = library
            .roots
            .iter()
            .find(|root| root.id == request.destination_root_id)
            .cloned();

        // FR-083, before any filesystem work is planned.
        self.require_library_permission(actor, &library.id, LibraryPermission::ManageTitles)
            .await?;

        let source_root_path = stored_path_to_path_buf(source_root.path.trim());
        let destination_root_path = destination_root
            .as_ref()
            .map(|root| stored_path_to_path_buf(root.path.trim()))
            .unwrap_or_default();

        // FR-020's admissibility rules, asked here and asked again when the
        // operation is started (which re-plans from scratch).
        let facts = self
            .consolidation_path_facts(
                request,
                &library,
                &source_root_path,
                &destination_root_path,
            )
            .await?;
        check_consolidation_paths(&facts).map_err(|refusal| {
            AppError::Validation(format!("{} [{}]", refusal.detail, refusal.code))
        })?;
        let destination_root = destination_root.expect("admissibility proved the destination root");

        // One read of the library's titles answers both questions: which titles
        // the operation accounts for (FR-023) and which titles the destination
        // root already holds (FR-024's merge candidates).
        let all_titles: Vec<Title> = self
            .services
            .catalog
            .titles
            .list_for_libraries(None, &[library.id.clone()], None)
            .await?;
        let mut titles: Vec<Title> = all_titles
            .iter()
            .filter(|title| title.root_folder_id == request.source_root_id)
            .cloned()
            .collect();
        titles.sort_by(|left, right| left.id.cmp(&right.id));
        let destination_titles: Vec<Title> = all_titles
            .iter()
            .filter(|title| title.root_folder_id == request.destination_root_id)
            .cloned()
            .collect();

        // FR-055: a merge is decided by canonical metadata identity and nothing
        // else. The candidate set is the *destination root's* titles — a title
        // on some third root of the same library is not a destination title.
        let candidates: Vec<DestinationTitleCandidate> = destination_titles
            .iter()
            .map(|title| {
                DestinationTitleCandidate::new(title.id.clone(), title.facet.clone())
                    .with_name(title.name.clone())
                    .with_external_ids(&title.external_ids)
            })
            .collect();
        let redirects = IdentityRedirects::new();
        let destination_titles_by_id: BTreeMap<&str, &Title> = destination_titles
            .iter()
            .map(|title| (title.id.as_str(), title))
            .collect();

        let mut identities: BTreeMap<String, DestinationIdentityOutcome> = BTreeMap::new();
        for title in &titles {
            let source = SourceTitleIdentity::new(title.id.clone(), title.facet.clone())
                .with_name(title.name.clone())
                .with_external_ids(&title.external_ids);
            identities.insert(
                title.id.clone(),
                detect_destination_title(&source, &candidates, &redirects),
            );
        }

        // FR-066/FR-071 at preview time, not at the checkpoint: the user must
        // confirm a plan whose central question — "can these two titles actually
        // be folded together?" — has already been asked.
        let mut merge_summaries: BTreeMap<String, MergePreviewSummary> = BTreeMap::new();
        for title in &titles {
            let Some(destination_title_id) = identities
                .get(&title.id)
                .and_then(DestinationIdentityOutcome::merge_target)
            else {
                continue;
            };
            let snapshot = self
                .services
                .library
                .title_merges
                .load_merge_snapshot(&title.id, destination_title_id, None)
                .await?;
            merge_summaries.insert(title.id.clone(), plan_merge(&snapshot).summary);
        }

        // Which folders the destination root already has, and who owns them
        // (FR-024 (3), FR-025).
        let case_rule = PathCaseRule::platform_default();
        let destination_states = self
            .destination_folder_occupancy(&destination_root_path, &destination_titles)
            .await?;

        let resolution_titles: Vec<FolderResolutionTitle> = titles
            .iter()
            .map(|title| {
                let merge_target = identities
                    .get(&title.id)
                    .and_then(DestinationIdentityOutcome::merge_target);
                let merge_title = merge_target.and_then(|id| destination_titles_by_id.get(id));
                FolderResolutionTitle {
                    title_id: title.id.clone(),
                    title_name: title.name.clone(),
                    source_folder_path: folder_path_of(title),
                    merge_target_title_id: merge_target.map(str::to_string),
                    merge_target_title_name: merge_title.map(|title| title.name.clone()),
                    merge_target_folder_path: merge_title.and_then(|title| folder_path_of(title)),
                }
            })
            .collect();
        let naming = CollisionNaming::from_source_library(root_label(&source_root_path));
        let resolved = resolve_consolidation_folders(&FolderResolutionRequest {
            source_root: source_root_path.clone(),
            destination_root: destination_root_path.clone(),
            case_rule,
            naming: naming.clone(),
            titles: resolution_titles,
            destination_states,
        });
        let resolved_by_title: BTreeMap<&str, &ResolvedFolder> = resolved
            .iter()
            .map(|folder| (folder.title_id.as_str(), folder))
            .collect();

        let recycle_config = self
            .recycle_bin_config_for_media_root(Some(source_root.path.trim()))
            .await;
        let recycle = if !recycle_config.enabled {
            RecycleAvailability::Disabled
        } else if let Some(error) = recycle_config.validation_error.clone() {
            RecycleAvailability::Unavailable(error)
        } else {
            RecycleAvailability::Available
        };
        let same_volume = Some(same_filesystem(&source_root_path, &destination_root_path).await);

        let mut drafts = Vec::with_capacity(titles.len());
        let mut moved_bytes = 0_u64;
        for title in &titles {
            let media_files = self
                .services
                .library
                .media_files
                .list_media_files_for_title(&title.id)
                .await?;
            let source_folder_path = folder_path_of(title);

            // The same blockers `classify_title` reads, from the same sources:
            // an active download or import (FR-086), another operation already
            // owning the title (FR-084) — plus, here, a merge the engine refuses
            // to plan (FR-066).
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
                        None => match merge_summaries
                            .get(&title.id)
                            .filter(|summary| summary.is_blocked())
                        {
                            Some(summary) => (
                                Some(format!(
                                    "\"{}\" cannot merge into the destination title yet: {}",
                                    title.name,
                                    summary.blocked_reason().unwrap_or_else(|| {
                                        "unmappable records".to_string()
                                    })
                                )),
                                Some(reason_codes::MERGE_RECORDS_UNMAPPED.to_string()),
                            ),
                            None => (None, None),
                        },
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

            let resolved = resolved_by_title
                .get(title.id.as_str())
                .map(|folder| (*folder).clone())
                .unwrap_or_else(|| ResolvedFolder {
                    title_id: title.id.clone(),
                    destination_folder: None,
                    placement: crate::location::consolidation::ConsolidationPlacement::NoFolder,
                    renamed_to: None,
                });

            // FR-073 needs a *proven* hash on both sides, and the only place a
            // destination file's hash exists without reading it again is the
            // catalog. For a merge the destination folder's contents are the
            // destination title's tracked media, so the persisted hashes are
            // right there; without them every identical episode would be renamed
            // beside its twin rather than deduplicated (D4).
            let destination_hashes = match resolved.placement.merge_target() {
                Some(destination_title_id) => {
                    self.persisted_hashes_for_consolidation(destination_title_id)
                        .await?
                }
                None => BTreeMap::new(),
            };
            let destination_entries = match resolved.destination_folder.as_deref() {
                Some(folder) if !blocked_reason.is_some() => {
                    read_destination_entries(folder, &destination_hashes).await
                }
                _ => Vec::new(),
            };

            drafts.push(ConsolidationTitleDraft {
                title_id: title.id.clone(),
                title_name: title.name.clone(),
                source_folder_path,
                files,
                source_directories,
                hardlinks,
                resolved,
                destination_entries,
                recycle: recycle.clone(),
                destination_identity: identities.get(&title.id).cloned(),
                merge_summary: merge_summaries.get(&title.id).cloned(),
                blocked_reason,
                blocked_reason_code,
            });
        }

        // FR-027's inventory: everything under the source root, except Scryer's
        // own recycle bin, which is not content the catalog failed to explain.
        // (The tail moves the bin; see `relocate_recycle_bin_for_root_change`.)
        let entries = scan_root_entries(&source_root_path, Some(&recycle_config.base_path)).await?;
        let free_space = estimate_free_space(
            &FreeSpaceRequest {
                source_path: source_root_path.clone(),
                destination_path: destination_root_path.clone(),
                // A same-volume consolidation renames; nothing is written twice.
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

        let default_transfer = DefaultRootTransfer {
            source_was_default: source_root.is_default,
            destination_was_default: destination_root.is_default,
        };

        let planned = build_root_consolidation_plan(&RootConsolidationPlanRequest {
            library_id: library.id.clone(),
            source_root_id: request.source_root_id.clone(),
            // The *configured* paths, not the canonicalized forms the
            // admissibility check compared: every title folder and every
            // media-file row is stored against the configured path, and on macOS
            // the canonical form of `/var/x` is `/private/var/x`.
            source_root_path: source_root_path.clone(),
            destination_root_id: request.destination_root_id.clone(),
            destination_root_path: destination_root_path.clone(),
            default_transfer,
            titles: drafts,
            entries,
            // Forced full for every location operation; the operator's depth
            // preference governs import copies only.
            verification_depth: LOCATION_OPERATION_VERIFICATION_DEPTH,
            free_space,
            same_volume,
            case_rule,
            naming,
        });

        // The destination root's title count once this finishes: the titles it
        // already holds, plus every arriving title that does *not* merge (a
        // merging title's row is folded into a destination row that is already
        // counted).
        let merging = planned
            .execution
            .titles
            .iter()
            .filter(|title| title.merges())
            .count() as i64;
        let retained_title_assignments =
            destination_titles.len() as i64 + planned.accounting.assigned_total - merging;

        let tail = RootChangeTail {
            library_id: library.id.clone(),
            // The root being retired. `RootChangeTail::rebase_onto_destination`
            // is what moves the recycle bin, and it re-anchors from this path
            // onto the destination root's — the same rebase the planner used for
            // content whose folder name survived.
            root_id: request.source_root_id.clone(),
            source_root_path: path_to_stored_string(&source_root_path),
            destination_root_path: path_to_stored_string(&destination_root_path),
            assigned_title_ids: titles.iter().map(|title| title.id.clone()).collect(),
            // For a consolidation this describes the *destination* root: the one
            // that keeps its synthetic id and may gain the library default.
            retention: RootIdentityRetention {
                root_id: request.destination_root_id.clone(),
                keeps_root_id: true,
                was_library_default: destination_root.is_default,
                remains_library_default: default_transfer.destination_becomes_default(),
                retained_role: None,
                retained_title_assignments,
            },
            content: planned.content.clone(),
            retirement: planned.retirement.clone(),
            consolidation: Some(ConsolidationTail {
                destination_root_id: request.destination_root_id.clone(),
                default_transfer,
            }),
        };

        let mut planned = planned;
        planned.execution.root_change = Some(tail.clone());
        Ok((planned, tail))
    }

    /// The `stat`s and configuration facts FR-020's rules are asked about.
    async fn consolidation_path_facts(
        &self,
        request: &RootConsolidationPreviewRequest,
        library: &scryer_domain::Library,
        source_root: &Path,
        destination_root: &Path,
    ) -> AppResult<ConsolidationPathFacts> {
        let source_root_is_symlink = tokio::fs::symlink_metadata(source_root)
            .await
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        let source_root_is_directory = tokio::fs::metadata(source_root)
            .await
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        let destination_root_is_directory = tokio::fs::metadata(destination_root)
            .await
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);

        Ok(ConsolidationPathFacts {
            source_root_id: request.source_root_id.clone(),
            destination_root_id: request.destination_root_id.clone(),
            source_root: canonical_or_lexical(source_root).await,
            destination_root: canonical_or_lexical(destination_root).await,
            source_root_is_symlink,
            source_root_is_directory,
            destination_root_is_directory,
            library_root_ids: library.roots.iter().map(|root| root.id.clone()).collect(),
            mode: request.mode,
        })
    }

    /// Which directories the destination root already holds, and who owns them
    /// (FR-024 (3), FR-025).
    ///
    /// A directory a destination title owns is a merge candidate's folder or an
    /// unrelated title's; a non-empty directory nobody owns is content Scryer
    /// must not write into either. Empty directories are reusable, which is what
    /// lets an interrupted run re-enter its own half-made folders (FR-089).
    async fn destination_folder_occupancy(
        &self,
        destination_root: &Path,
        destination_titles: &[Title],
    ) -> AppResult<BTreeMap<String, DestinationFolderState>> {
        let owners: BTreeMap<String, String> = destination_titles
            .iter()
            .filter_map(|title| {
                folder_path_of(title).map(|folder| {
                    (path_to_stored_string(&folder), title.id.clone())
                })
            })
            .collect();

        let root = destination_root.to_path_buf();
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
        .map_err(|error| {
            AppError::Repository(format!("destination root scan task panicked: {error}"))
        })??;

        // The walker reports one listing per directory with the files directly
        // inside it. A directory is "empty" only when it holds neither a file
        // nor a subdirectory, so the child listings are folded back onto their
        // parents rather than each answering for itself.
        let mut has_children: BTreeSet<String> = BTreeSet::new();
        let mut directories: BTreeMap<String, bool> = BTreeMap::new();
        for listing in &walked {
            if listing.path != root {
                directories.insert(
                    path_to_stored_string(&listing.path),
                    !listing.files.is_empty(),
                );
                if let Some(parent) = listing.path.parent() {
                    has_children.insert(path_to_stored_string(parent));
                }
            }
        }

        let mut states = BTreeMap::new();
        for (path, holds_files) in directories {
            let state = match owners.get(&path) {
                Some(title_id) => DestinationFolderState::OwnedByTitle {
                    title_id: title_id.clone(),
                },
                None if holds_files || has_children.contains(&path) => {
                    DestinationFolderState::Occupied
                }
                None => DestinationFolderState::Empty,
            };
            states.insert(path, state);
        }

        // A destination title may own a folder the walk could not see (an
        // unreadable subtree, or a folder recorded but never created). Its name
        // is still claimed: nothing else may take it.
        for (path, title_id) in owners {
            states
                .entry(path)
                .or_insert(DestinationFolderState::OwnedByTitle { title_id });
        }
        Ok(states)
    }

    /// Persisted full-BLAKE3 state for one destination title's tracked media,
    /// keyed by stored path, so the collision planner can prove a duplicate
    /// without reading a byte (D4, FR-047).
    async fn persisted_hashes_for_consolidation(
        &self,
        title_id: &str,
    ) -> AppResult<BTreeMap<String, FullHash>> {
        Ok(self
            .services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await?
            .into_iter()
            .map(|file| {
                let hash = FullHash::from_persisted(file.content_hashes.as_ref());
                (file.file_path, hash)
            })
            .collect())
    }

    /// FR-020's second branch, last step: remove the source root from the
    /// library's root list, and move the default if it held it (FR-022).
    ///
    /// Called by [`AppUseCase::retire_changed_root`] after the shared tail moved
    /// the recycle bin and pruned the empty source directories, and therefore
    /// after every title has moved, verified, reconciled, and recycled (FR-087).
    ///
    /// Idempotent, because a resumed run reaches this point again: it computes
    /// the root list it wants, compares it with the one the library has, and
    /// writes nothing when they already agree.
    pub(super) async fn retire_consolidated_root_configuration(
        &self,
        tail: &RootChangeTail,
        consolidation: &ConsolidationTail,
    ) -> AppResult<Vec<String>> {
        let mut warnings = Vec::new();
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&tail.library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {}", tail.library_id)))?;

        let titles = self
            .services
            .catalog
            .titles
            .list_for_libraries(None, &[library.id.clone()], None)
            .await?;
        let still_on_source = titles
            .iter()
            .filter(|title| title.root_folder_id == tail.root_id)
            .count();

        // Three independent gates, each of them a spec sentence:
        //  - FR-023: a title still on the source root means the operation did
        //    not finish accounting for it. (A blocked title cannot get here —
        //    it refuses the start — so this is the belt to that braces.)
        //  - FR-028: unexplained content at the source blocks removing the root.
        //  - and there is nothing to do at all if it is already gone (resume).
        let source_present = library
            .roots
            .iter()
            .any(|root| root.id == tail.root_id);
        if still_on_source > 0 {
            warnings.push(format!(
                "{} title(s) still reference {}, so it stays a configured root",
                still_on_source, tail.source_root_path
            ));
        }
        let remove_source = source_present
            && still_on_source == 0
            && tail.retirement.permits_source_removal();

        let becomes_default = consolidation.default_transfer.destination_becomes_default();
        let desired: Vec<LibraryRootDraft> = library
            .roots
            .iter()
            .filter(|root| !(remove_source && root.id == tail.root_id))
            .map(|root| LibraryRootDraft {
                path: root.path.clone(),
                is_default: if root.id == consolidation.destination_root_id {
                    becomes_default
                } else if root.id == tail.root_id {
                    // FR-022: a default source root hands the default to the
                    // destination. A source root that survives because of
                    // unexplained content must not keep it, or the library would
                    // have two defaults.
                    root.is_default && !becomes_default
                } else {
                    root.is_default
                },
            })
            .collect();

        let unchanged = desired.len() == library.roots.len()
            && desired.iter().zip(library.roots.iter()).all(|(want, has)| {
                want.path == has.path && want.is_default == has.is_default
            });

        let library = if unchanged {
            // A resumed run reaching the tail a second time.
            library
        } else {
            self.services
                .catalog
                .libraries
                .update(
                    &library.id,
                    library.name.clone(),
                    library.slug.clone(),
                    desired,
                )
                .await?
        };

        // Post-conditions. Each is a promise the preview made.
        if remove_source && library.roots.iter().any(|root| root.id == tail.root_id) {
            warnings.push(format!(
                "{} is still a configured root after the consolidation completed",
                tail.source_root_path
            ));
        }
        match library
            .roots
            .iter()
            .find(|root| root.id == consolidation.destination_root_id)
        {
            None => warnings.push(format!(
                "root {} no longer exists after the consolidation, so its titles have no configured root",
                consolidation.destination_root_id
            )),
            Some(root) => {
                // FR-078: the destination keeps the synthetic id it had, which
                // is what makes every repointed title's reference still valid.
                if scryer_domain::normalize_library_root_path(root.path.trim())
                    != scryer_domain::normalize_library_root_path(
                        tail.destination_root_path.trim(),
                    )
                {
                    warnings.push(format!(
                        "root {} reads back as {} rather than {}",
                        consolidation.destination_root_id, root.path, tail.destination_root_path
                    ));
                }
                if root.is_default != becomes_default {
                    warnings.push(format!(
                        "root {} {} the library default across the consolidation",
                        consolidation.destination_root_id,
                        if root.is_default {
                            "unexpectedly became"
                        } else {
                            "unexpectedly stopped being"
                        }
                    ));
                }
            }
        }

        let retained = self
            .services
            .catalog
            .titles
            .list_for_libraries(None, &[library.id.clone()], None)
            .await?
            .into_iter()
            .filter(|title| title.root_folder_id == consolidation.destination_root_id)
            .count() as i64;
        if retained != tail.retention.retained_title_assignments {
            warnings.push(format!(
                "root {} holds {retained} title assignment(s) after the consolidation, not the {} the preview promised",
                consolidation.destination_root_id, tail.retention.retained_title_assignments
            ));
        }

        for warning in &warnings {
            tracing::error!(
                source_root_id = %tail.root_id,
                destination_root_id = %consolidation.destination_root_id,
                warning = %warning,
                "a consolidation's post-condition did not hold"
            );
        }

        // Compatibility plumbing, not spec: the legacy per-facet root-folder
        // settings keys mirror the default library's roots, and a consolidation
        // removes one of them. A stale mirror would point scanning and import at
        // a root the library no longer has.
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
                "the legacy root-folder settings still name the retired root: {error}"
            ));
        }

        Ok(warnings)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn folder_path_of(title: &Title) -> Option<PathBuf> {
    title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(stored_path_to_path_buf)
}

/// The readable label a uniqued folder name is suffixed with (FR-074's scheme,
/// applied to folders by FR-025/FR-026).
///
/// Both roots are in one library, so the source *library* name — what a
/// cross-library transfer uses — would say nothing at all. The source root's own
/// last path segment is the thing the user chose and recognizes; the full path
/// would not survive [`CollisionNaming::from_source_library`]'s sanitizer, which
/// strips separators for exactly the reason it should.
fn root_label(source_root: &Path) -> String {
    source_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "the old root".to_string())
}

/// What one destination folder already holds, as the collision engine wants it
/// (FR-072–FR-075).
///
/// `known_hashes` maps a stored destination path to the full BLAKE3 the catalog
/// has for it. Anything not in the map keeps [`FullHash::Absent`], which the
/// dedup gate reads as "unproven" and therefore as *not* a duplicate (D4).
async fn read_destination_entries(
    destination_folder: &Path,
    known_hashes: &BTreeMap<String, FullHash>,
) -> Vec<DestinationItem> {
    let Ok(mut entries) = tokio::fs::read_dir(destination_folder).await else {
        return Vec::new();
    };
    let mut items = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if metadata.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let path = path_to_stored_string(entry.path());
        let full_hash = known_hashes.get(&path).cloned().unwrap_or(FullHash::Absent);
        items.push(
            DestinationItem::companion(name, metadata.len())
                .with_content(ContentFacts::new(metadata.len()).with_full_hash(full_hash))
                .with_path(path),
        );
    }
    items
}
