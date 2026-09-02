//! The use-case half of **Change root** (US4 + US5): one preview, one start,
//! one plan assembly, and the one tail sequence that retires the source
//! location and then writes the library's root configuration once every title
//! has landed.
//!
//! The planning rules live in [`crate::location::root_scope`] and are pure.
//! Everything here is the IO the planner refuses to do — the scans, the
//! `stat`s, the catalog reads, the identity detection, the merge snapshots, and
//! the one configuration write each branch ends in — plus the piece of ordering
//! FR-087 insists on and no other operation type needs.
//!
//! # The shape of a root-scoped operation
//!
//! ```text
//! preview_root_scope ─▶ every title assigned to the root (FR-023, no selection)
//!                       the source root's filesystem inventory (FR-027)
//!                       destination admissibility (FR-020)
//!                       ─▶ LocationPlan + RootMoveExecutionPlan + RootScopeTail
//!
//! start_root_scope   ─▶ re-plan, compare fingerprints (FR-081), typed
//!                       confirmation (FR-029), persist, spawn the shared runner
//!
//! …the runner walks the titles exactly as it walks a root move…
//!
//! retire_changed_root ─▶ prune empty source directories (FR-028)
//!                        flip the root's path, or retire it (FR-021, FR-078,
//!                        FR-022)
//!                        assert the post-conditions the preview promised
//!                        mirror the legacy default-root settings
//! ```
//!
//! FR-020 is one action with two destinations, so this is one path with two
//! destinations. The branch decides three things and nothing else: which
//! destination the admissibility rules are asked about, whether there are
//! destination titles to detect identities against and folders to resolve
//! around (a new, unconfigured path has neither by definition), and which
//! configuration write the tail ends in. There is one public entry point per
//! phase — [`AppUseCase::preview_root_scope`] and
//! [`AppUseCase::start_root_scope`] — and the GraphQL surface has one name for
//! each of them.
//!
//! # Why the tail is here and not in the runner
//!
//! [`crate::location::executor::LocationOperationRunner`] is per-title by
//! construction: it walks a work plan, checkpoints each title, and knows nothing
//! about roots. A root-scoped operation's last act is not about a title at all —
//! it is one configuration write that must happen after the *last* title
//! recycled (FR-087). Teaching the runner a root-scoped epilogue would put a US4
//! concept in the one component every workflow shares; running it here, on the
//! terminal outcome, keeps the runner exactly as generic as it was.
//!
//! Only that last write differs, and
//! [`crate::location::root_scope::RootScopeTail::consolidation`] selects it:
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
//! than assuming it runs once: a root whose stored path is already the
//! destination has already flipped, and an absent directory has already been
//! pruned.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use scryer_domain::{LibraryPermission, Title, User};

use crate::LibraryRootDraft;
use crate::library::recycle_bin::RECYCLE_DIR_NAME;
use crate::location::classify::reason_codes;
use crate::location::collisions::{CollisionNaming, PathCaseRule, RecycleAvailability};
use crate::location::execution::{DirectoryPrune, remove_directory_if_empty};
use crate::location::identity::{
    DestinationIdentityOutcome, DestinationTitleCandidate, IdentityRedirects, SourceTitleIdentity,
    detect_destination_title,
};
use crate::location::merge::engine::plan_merge;
use crate::location::merge::summary::MergePreviewSummary;
use crate::location::model::{LocationExecutionMode, LocationOperation, LocationOperationType};
use crate::location::operations::{
    LOCATION_OPERATION_VERIFICATION_DEPTH, LocationOperationAccepted, LocationOperationAdmission,
    TitleMoveFacts, confirmation_error,
};
use crate::location::ownership_guard::OwnedEntity;
use crate::location::preview::{
    FreeSpaceRequest, PlanConfirmationRequest, SystemVolumeProbe, estimate_free_space,
};
use crate::location::root_scope::{
    ConsolidationTail, DefaultRootTransfer, DestinationPathState, FolderResolutionRequest,
    FolderResolutionTitle, PlannedRootScope, ResolvedFolder, RootEntry, RootIdentityRetention,
    RootRetentionFacts, RootScopePathFacts, RootScopePathVariant, RootScopePlanRequest,
    RootScopeTail, RootScopeTitleDraft, RootScopeVariant, build_root_scope_plan,
    check_root_scope_paths, resolve_root_scope_folders,
};
use crate::location::verify::same_filesystem;
use crate::services::AppUseCase;
use crate::settings::keys::SETTINGS_SOURCE_TYPED_GRAPHQL;
use crate::settings::runtime::root_folder_entries_from_library_roots;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppError, AppResult};

/// One root-scoped request, in the currency FR-020 actually has: one root, one
/// destination, one mode.
///
/// FR-020's **Change root** is one settings action with two destinations, so it
/// is one request with two destinations. There is no title selection — FR-023
/// forbids one; the root *is* the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootScopeCall {
    pub library_id: String,
    /// The root the operation acts on. Its synthetic id survives either
    /// destination (FR-021, FR-078): a path change repoints it, a fold retires
    /// its configuration.
    pub root_id: String,
    pub destination: RootScopeCallDestination,
    pub mode: LocationExecutionMode,
}

/// FR-020's two destinations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootScopeCallDestination {
    /// US4: a new, unconfigured path the user typed.
    Path(String),
    /// US5: another configured root of the same library.
    Root(String),
}

impl RootScopeCall {
    fn folds(&self) -> bool {
        matches!(self.destination, RootScopeCallDestination::Root(_))
    }

    /// The sentence a refused start is reported with. The two destinations read
    /// differently to the user even though they are one code path.
    fn start_failure_message(&self) -> &'static str {
        if self.folds() {
            "The consolidation could not be started."
        } else {
            "The root change could not be started."
        }
    }
}

/// The confirmation a client sends back: the call it previewed, the fingerprint
/// that preview carried, and the typed phrase FR-029 requires of a root-wide
/// operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRootScopeRequest {
    pub call: RootScopeCall,
    pub confirmation: PlanConfirmationRequest,
}

impl AppUseCase {
    /// Re-plan, confirm, persist, and spawn — for either of FR-020's branches.
    ///
    /// The plan is rebuilt from scratch rather than round-tripped through the
    /// client (C2), so the fingerprint the caller confirms is compared against
    /// a plan built from the world as it is now (FR-081).
    pub async fn start_root_scope(
        &self,
        actor: &User,
        request: StartRootScopeRequest,
    ) -> AppResult<LocationOperationAccepted> {
        let failure_message = request.call.start_failure_message();
        let planned = self.preview_root_scope(actor, &request.call).await?;
        let plan = planned.plan;

        // FR-029's typed phrase and FR-023's blocked titles both come out of
        // here: the plan header makes the confirmation stronger, and a blocked
        // title is a `NeedsResolution` classification, which `confirm` refuses
        // before anything is persisted.
        plan.confirm(&request.confirmation)
            .map_err(confirmation_error)?;

        // A root-scoped operation with no titles at all is a legitimate request
        // — an empty root moving to a new disk, or a root nothing lives on being
        // folded away — so, unlike a title selection, it is never refused for
        // having nothing to move. Activity still reports the ledger, which is
        // what makes "0 of 0 titles" readable.
        self.admit_location_operation(LocationOperationAdmission {
            actor,
            plan,
            titles_total: planned.accounting.assigned_total,
            execution: planned.execution,
            failure_message,
        })
        .await
    }

    /// Assemble every fact [`build_root_scope_plan`] needs, then plan — for
    /// either of FR-020's branches.
    ///
    /// The branch decides three things and nothing else: which destination the
    /// admissibility rules are asked about, whether there are destination
    /// titles to detect identities and resolve folders against (a new,
    /// unconfigured path has none by definition), and which last step the tail
    /// takes. Everything between — the every-title ledger, the per-title walk,
    /// the source inventory, the free-space estimate — is one path.
    pub async fn preview_root_scope(
        &self,
        actor: &User,
        call: &RootScopeCall,
    ) -> AppResult<PlannedRootScope> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&call.library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {}", call.library_id)))?;
        let source_root = library
            .roots
            .iter()
            .find(|root| root.id == call.root_id)
            .cloned()
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "root {} in library {}",
                    call.root_id, call.library_id
                ))
            })?;

        // FR-083, before any filesystem work is planned.
        self.require_library_permission(actor, &library.id, LibraryPermission::ManageTitles)
            .await?;

        let source_root_path = stored_path_to_path_buf(source_root.path.trim());
        let (destination_root_path, destination_root) =
            self.resolve_root_scope_destination(&library, call).await?;

        // FR-020's admissibility rules, asked of the filesystem here and asked
        // again when the operation is admitted, because the destination is a
        // path anything could write to in between.
        let facts = self
            .root_scope_path_facts(
                call,
                &library,
                destination_root.as_ref(),
                &source_root_path,
                &destination_root_path,
            )
            .await?;
        check_root_scope_paths(&facts).map_err(|refusal| AppError::LocationRootRefused {
            message: refusal.detail,
            code: refusal.code,
        })?;

        // One read of the library's titles answers both questions: which titles
        // the operation accounts for (FR-023) and which titles the destination
        // root already holds (FR-024's merge candidates).
        let all_titles: Vec<Title> = self
            .services
            .catalog
            .titles
            .list_for_libraries(None, std::slice::from_ref(&library.id), None)
            .await?;
        // FR-023: every title assigned to the root, in a stable order. Not a
        // selection — there is no way to express one.
        let mut titles: Vec<Title> = all_titles
            .iter()
            .filter(|title| title.root_folder_id == call.root_id)
            .cloned()
            .collect();
        titles.sort_by(|left, right| left.id.cmp(&right.id));

        // A new, unconfigured path holds nothing, so there is nothing to detect
        // an identity against, nothing to merge with, and no folder to collide
        // over. Every read below is the fold's.
        let destination_titles: Vec<Title> = match &destination_root {
            Some(root) => all_titles
                .iter()
                .filter(|title| title.root_folder_id == root.id)
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        let destination_titles_by_id: BTreeMap<&str, &Title> = destination_titles
            .iter()
            .map(|title| (title.id.as_str(), title))
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
        let mut identities: BTreeMap<String, DestinationIdentityOutcome> = BTreeMap::new();
        if !candidates.is_empty() {
            for title in &titles {
                let source = SourceTitleIdentity::new(title.id.clone(), title.facet.clone())
                    .with_name(title.name.clone())
                    .with_external_ids(&title.external_ids);
                identities.insert(
                    title.id.clone(),
                    detect_destination_title(&source, &candidates, &redirects),
                );
            }
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
        // (FR-024 (3), FR-025). A path change resolves nothing: the re-anchored
        // path is always free, and the planner's adapter re-anchors it.
        let (case_rule, naming) = match &destination_root {
            Some(_) => (
                PathCaseRule::platform_default(),
                CollisionNaming::from_source_library(root_label(&source_root_path)),
            ),
            None => (
                PathCaseRule::CaseSensitive,
                CollisionNaming::from_source_library(&library.name),
            ),
        };
        let resolved: Vec<ResolvedFolder> = match &destination_root {
            Some(root) => {
                let destination_occupants = self
                    .destination_folder_occupancy(
                        &stored_path_to_path_buf(root.path.trim()),
                        &destination_titles,
                    )
                    .await?;
                let resolution_titles: Vec<FolderResolutionTitle> = titles
                    .iter()
                    .map(|title| {
                        let merge_target = identities
                            .get(&title.id)
                            .and_then(DestinationIdentityOutcome::merge_target);
                        let merge_title =
                            merge_target.and_then(|id| destination_titles_by_id.get(id));
                        FolderResolutionTitle {
                            title_id: title.id.clone(),
                            title_name: title.name.clone(),
                            source_folder_path: folder_path_of(title),
                            merge_target_title_id: merge_target.map(str::to_string),
                            merge_target_title_name: merge_title.map(|title| title.name.clone()),
                            merge_target_folder_path: merge_title
                                .and_then(|title| folder_path_of(title)),
                        }
                    })
                    .collect();
                resolve_root_scope_folders(&FolderResolutionRequest {
                    source_root: source_root_path.clone(),
                    destination_root: destination_root_path.clone(),
                    case_rule,
                    naming: naming.clone(),
                    titles: resolution_titles,
                    destination_occupants,
                })
            }
            None => Vec::new(),
        };
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
            let source_folder_path = folder_path_of(title);
            let (blocked_reason, blocked_reason_code) = self
                .root_scope_title_blockers(title, merge_summaries.get(&title.id))
                .await?;

            let resolved = resolved_by_title
                .get(title.id.as_str())
                .map(|f| (*f).clone());
            // A blocked title never enters the operation, so walking its folder
            // would be work whose only product is a plan item the planner
            // discards. It is still counted and named (FR-023).
            let facts = if blocked_reason.is_some() {
                TitleMoveFacts::default()
            } else {
                let media_files = self
                    .services
                    .library
                    .media_files
                    .list_media_files_for_title(&title.id)
                    .await?;
                self.title_move_facts(
                    source_folder_path.as_deref(),
                    &media_files,
                    resolved
                        .as_ref()
                        .and_then(|resolved| resolved.destination_folder.as_deref()),
                    identities
                        .get(&title.id)
                        .and_then(DestinationIdentityOutcome::merge_target),
                )
                .await?
            };
            moved_bytes = moved_bytes.saturating_add(
                facts
                    .files
                    .iter()
                    .fold(0_u64, |total, file| total.saturating_add(file.size_bytes)),
            );

            drafts.push(RootScopeTitleDraft {
                source_folder_path,
                files: facts.files,
                source_directories: facts.source_directories,
                hardlinks: facts.hardlinks,
                resolved,
                destination_entries: facts.destination_entries,
                recycle: recycle.clone(),
                destination_identity: identities.get(&title.id).cloned(),
                merge_summary: merge_summaries.get(&title.id).cloned(),
                blocked_reason,
                blocked_reason_code,
                ..RootScopeTitleDraft::new(title.id.clone(), title.name.clone())
            });
        }

        // FR-027's inventory: everything under the source root, so the planner
        // can put each entry in exactly one of the three buckets — except
        // Scryer's own recycle bin, which is not content the catalog failed to
        // explain. Left in, it would report every recycled file as unexplained,
        // block the source location's retirement on Scryer's own bookkeeping,
        // and put the bin's contents in the fingerprint so that any background
        // purge voided the user's confirmation. The bin never moves; the tail
        // names the source directory it keeps standing.
        let entries = scan_root_entries(&source_root_path, Some(&recycle_config.base_path)).await?;
        let free_space = estimate_free_space(
            &FreeSpaceRequest {
                source_path: source_root_path.clone(),
                destination_path: destination_root_path.clone(),
                // A same-volume root-scoped operation renames; nothing is
                // written twice.
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

        let default_transfer = match &destination_root {
            Some(root) => DefaultRootTransfer {
                source_was_default: source_root.is_default,
                destination_was_default: root.is_default,
            },
            None => DefaultRootTransfer::default(),
        };
        let variant = match &destination_root {
            Some(root) => RootScopeVariant::FoldInto {
                destination_root_id: root.id.clone(),
                default_transfer,
            },
            None => RootScopeVariant::ChangePath {
                retention: RootRetentionFacts {
                    is_library_default: source_root.is_default,
                    role: None,
                },
            },
        };

        let mut planned = build_root_scope_plan(&RootScopePlanRequest {
            library_id: library.id.clone(),
            root_id: call.root_id.clone(),
            // The *configured* source path and the path the user typed, not the
            // canonicalized forms the admissibility check compared. Every title
            // folder and every media-file row in the catalog is stored against
            // the configured path, and on macOS the canonical form of `/var/x`
            // is `/private/var/x` — re-anchoring against the resolved form
            // would fail to strip a single prefix and place every file "outside
            // the root being changed". Canonicalization answers "do these two
            // paths overlap?"; stored paths are what the catalog is written in.
            source_root_path: source_root_path.clone(),
            destination_root_path: destination_root_path.clone(),
            variant,
            // A fold only ever runs **Move with Scryer**; a path change carries
            // the mode the user chose (FR-076 downgrades either to
            // catalog-only when nothing is on disk).
            mode: if call.folds() {
                LocationExecutionMode::MoveWithScryer
            } else {
                call.mode
            },
            titles: drafts,
            entries,
            // Forced full for every location operation; the operator's depth
            // preference governs import copies only
            // (`LOCATION_OPERATION_VERIFICATION_DEPTH`).
            verification_depth: LOCATION_OPERATION_VERIFICATION_DEPTH,
            free_space,
            same_volume,
            case_rule,
            naming,
        });

        let assigned_title_ids: Vec<String> = titles.iter().map(|title| title.id.clone()).collect();
        planned.execution.root_change = Some(match &destination_root {
            Some(root) => {
                // The destination root's title count once this finishes: the
                // titles it already holds, plus every arriving title that does
                // *not* merge (a merging title's row is folded into a
                // destination row that is already counted).
                let merging = planned
                    .execution
                    .titles
                    .iter()
                    .filter(|title| title.merges())
                    .count() as i64;
                RootScopeTail {
                    library_id: library.id.clone(),
                    // The root being retired.
                    root_id: call.root_id.clone(),
                    source_root_path: path_to_stored_string(&source_root_path),
                    destination_root_path: path_to_stored_string(&destination_root_path),
                    assigned_title_ids,
                    // For a fold this describes the *destination* root: the one
                    // that keeps its synthetic id and may gain the library
                    // default.
                    retention: RootIdentityRetention {
                        root_id: root.id.clone(),
                        keeps_root_id: true,
                        was_library_default: root.is_default,
                        remains_library_default: default_transfer.destination_becomes_default(),
                        retained_role: None,
                        retained_title_assignments: destination_titles.len() as i64
                            + planned.accounting.assigned_total
                            - merging,
                    },
                    content: planned.content.clone(),
                    retirement: planned.retirement.clone(),
                    consolidation: Some(ConsolidationTail {
                        destination_root_id: root.id.clone(),
                        default_transfer,
                    }),
                }
            }
            // US4's branch: the root's path is flipped, not retired.
            None => planned.tail(&library.id, &call.root_id, assigned_title_ids, None),
        });
        Ok(planned)
    }

    /// Why this title cannot enter the operation yet, if it cannot (FR-023).
    ///
    /// The same blockers `classify_title` reads, from the same sources: an
    /// active download or import (FR-086), another operation already owning the
    /// title (FR-084) — plus a merge the engine refuses to plan (FR-066), which
    /// only a fold can have.
    async fn root_scope_title_blockers(
        &self,
        title: &Title,
        merge_summary: Option<&MergePreviewSummary>,
    ) -> AppResult<(Option<String>, Option<String>)> {
        if let Some(detail) = self.active_work_blocking_a_move(title).await? {
            return Ok((
                Some(detail),
                Some(reason_codes::ACTIVE_DOWNLOAD_OR_IMPORT.to_string()),
            ));
        }
        if let Some(operation_id) = self
            .services
            .library
            .location_operations
            .location_ownership_holder(&OwnedEntity::Title(title.id.clone()))
            .await?
        {
            return Ok((
                Some(format!(
                    "\"{}\" is already owned by location operation {operation_id}",
                    title.name
                )),
                Some(reason_codes::OWNED_BY_LOCATION_OPERATION.to_string()),
            ));
        }
        if let Some(summary) = merge_summary.filter(|summary| summary.is_blocked()) {
            return Ok((
                Some(format!(
                    "\"{}\" cannot merge into the destination title yet: {}",
                    title.name,
                    summary
                        .blocked_reason()
                        .unwrap_or_else(|| "unmappable records".to_string())
                )),
                Some(reason_codes::MERGE_RECORDS_UNMAPPED.to_string()),
            ));
        }
        Ok((None, None))
    }

    /// The `stat`s and configuration facts FR-020's rules are asked about.
    /// FR-020's one destination, resolved to the branch it actually is.
    ///
    /// The client names either a new path or an existing root. A path that
    /// resolves to a configured root of this library *is* that root — the same
    /// request said the other way — so it is planned as a fold rather than
    /// refused for arriving in the wrong shape. Everything else is a path
    /// change.
    ///
    /// Returns the destination's path and, when the destination is a configured
    /// root, that root's configuration.
    async fn resolve_root_scope_destination(
        &self,
        library: &scryer_domain::Library,
        call: &RootScopeCall,
    ) -> AppResult<(PathBuf, Option<scryer_domain::LibraryRoot>)> {
        match &call.destination {
            RootScopeCallDestination::Path(path) => {
                let path = path.trim();
                if path.is_empty() {
                    return Err(AppError::Validation(
                        "choose a new path for this root".to_string(),
                    ));
                }
                // Lexically normalized only: `.` and `..` are resolved so the
                // stored configuration is predictable, while symlinks and
                // platform aliases are left exactly as the user gave them.
                let destination =
                    crate::stored_paths::lexically_normalize(&stored_path_to_path_buf(path));
                // Compared the way the admissibility rules compare, so a path
                // that would have been refused for being a root of this library
                // is the one that becomes a fold.
                let canonical = canonical_or_lexical(&destination).await;
                for root in &library.roots {
                    // The source root's own path is not a fold into itself; the
                    // overlap rule refuses it, by name.
                    if root.id == call.root_id {
                        continue;
                    }
                    let configured = root.path.trim();
                    if configured.is_empty() {
                        continue;
                    }
                    let configured_path = stored_path_to_path_buf(configured);
                    if canonical_or_lexical(&configured_path).await == canonical {
                        return Ok((configured_path, Some(root.clone())));
                    }
                }
                Ok((destination, None))
            }
            RootScopeCallDestination::Root(root_id) => {
                let root = library
                    .roots
                    .iter()
                    .find(|root| &root.id == root_id)
                    .cloned()
                    .ok_or_else(|| {
                        AppError::NotFound(format!("root {root_id} in library {}", library.id))
                    })?;
                let path = stored_path_to_path_buf(root.path.trim());
                Ok((path, Some(root)))
            }
        }
    }

    async fn root_scope_path_facts(
        &self,
        call: &RootScopeCall,
        library: &scryer_domain::Library,
        destination_root_config: Option<&scryer_domain::LibraryRoot>,
        source_root: &Path,
        destination_root: &Path,
    ) -> AppResult<RootScopePathFacts> {
        let source_root_is_symlink = tokio::fs::symlink_metadata(source_root)
            .await
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

        // Which branch this is was settled by `resolve_root_scope_destination`:
        // a destination that resolved to a configured root of this library is a
        // fold, whichever of FR-020's two forms named it.
        let variant = match destination_root_config {
            Some(destination) => RootScopePathVariant::FoldInto {
                source_root_id: call.root_id.clone(),
                destination_root_id: destination.id.clone(),
            },
            None => {
                // Only *other* libraries' roots: one of this library's would
                // have resolved to a fold above, so a match here is the
                // genuine "two libraries cannot share a root" refusal.
                let mut configured_roots = Vec::new();
                for other in self.services.catalog.libraries.list(None).await? {
                    if other.id == library.id {
                        continue;
                    }
                    for root in other.roots {
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
                RootScopePathVariant::ChangePath { configured_roots }
            }
        };

        Ok(RootScopePathFacts {
            variant,
            source_root: canonical_or_lexical(source_root).await,
            destination_root: canonical_or_lexical(destination_root).await,
            source_root_is_symlink,
            source_root_is_directory,
            destination,
            mode: call.mode,
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
    async fn retire_changed_root(&self, tail: &RootScopeTail) -> AppResult<Vec<String>> {
        // 1. FR-028: empty source directories only, deepest first, and only
        //    what the confirmed plan named. A recycle bin under the source root
        //    is not moved and not removed, so the source directory it sits in
        //    is left standing and named in the warnings.
        let mut warnings = self.retire_source_location(tail).await;

        // 2. The last step, and the only one FR-020's two branches do not share
        //    (see `RootScopeTail::consolidation`):
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

    /// FR-028: remove the empty directories the confirmed plan named, and the
    /// source root itself only when nothing unexplained is left standing.
    async fn retire_source_location(&self, tail: &RootScopeTail) -> Vec<String> {
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
            // Scryer's own recycle bin never travels with a root (it is the one
            // thing under the source root the operation deliberately leaves
            // behind), so a source root left standing *only* because the bin is
            // inside it is not content the user should go looking for.
            DirectoryPrune::NotEmpty => {
                warnings.push(if only_the_recycle_bin_remains(&tail.source_root()).await {
                    format!(
                        "{} was kept because it holds Scryer's recycle bin",
                        tail.source_root_path
                    )
                } else {
                    format!(
                        "{} still holds content, so the old location was left in place",
                        tail.source_root_path
                    )
                });
            }
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
    async fn flip_changed_root_path(&self, tail: &RootScopeTail) -> AppResult<Vec<String>> {
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

        self.assert_root_post_conditions(
            &library,
            &tail.root_id,
            tail,
            tail.retention.remains_library_default,
            "the path change",
            "the legacy root-folder settings still name the old path",
        )
        .await
    }

    /// The post-conditions both branches of FR-020 end with, checked against the
    /// library as it reads back.
    ///
    /// The surviving root — the repointed one (FR-021, FR-078) or the one that
    /// absorbed the fold (FR-022) — has to be at the destination path, hold the
    /// default status the preview promised, and hold the title assignments the
    /// preview counted. Each is a promise the user confirmed, so each is
    /// asserted rather than assumed, and each failure is a warning rather than
    /// an error: the bytes have already moved and been verified.
    ///
    /// The last step is compatibility plumbing rather than spec: the legacy
    /// per-facet root-folder settings keys mirror the default library's roots,
    /// and nothing else in the location subsystem rewrites a library's root
    /// list, so nothing else would update them. A stale mirror would point
    /// scanning and import at a location the library no longer has.
    async fn assert_root_post_conditions(
        &self,
        library: &scryer_domain::Library,
        surviving_root_id: &str,
        tail: &RootScopeTail,
        expected_default: bool,
        subject: &str,
        legacy_settings_failure: &str,
    ) -> AppResult<Vec<String>> {
        let mut warnings = Vec::new();
        match library
            .roots
            .iter()
            .find(|root| root.id == surviving_root_id)
        {
            None => warnings.push(format!(
                "root {surviving_root_id} no longer exists after {subject}, so its titles have no configured root"
            )),
            Some(root) => {
                if scryer_domain::normalize_library_root_path(root.path.trim())
                    != scryer_domain::normalize_library_root_path(
                        tail.destination_root_path.trim(),
                    )
                {
                    warnings.push(format!(
                        "root {surviving_root_id} reads back as {} rather than {}",
                        root.path, tail.destination_root_path
                    ));
                }
                if root.is_default != expected_default {
                    warnings.push(format!(
                        "root {surviving_root_id} {} the library default across {subject}",
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
            .list_for_libraries(None, std::slice::from_ref(&library.id), None)
            .await?
            .into_iter()
            .filter(|title| title.root_folder_id == surviving_root_id)
            .count() as i64;
        if retained != tail.retention.retained_title_assignments {
            warnings.push(format!(
                "root {surviving_root_id} holds {retained} title assignment(s) after {subject}, not the {} the preview promised",
                tail.retention.retained_title_assignments
            ));
        }

        for warning in &warnings {
            tracing::error!(
                root_id = %surviving_root_id,
                warning = %warning,
                "a root-scoped operation's post-condition did not hold"
            );
        }

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
            warnings.push(format!("{legacy_settings_failure}: {error}"));
        }

        Ok(warnings)
    }
}

/// Binds [`AppUseCase::retire_changed_root`] onto the runner's epilogue seam.
///
/// Borrowed rather than owned so the runner keeps a plain `&dyn` for the whole
/// run, the way its mover and reconciler are bound.
pub(super) struct RootScopeEpilogue<'a> {
    pub(super) app: &'a AppUseCase,
    pub(super) tail: &'a RootScopeTail,
}

#[async_trait::async_trait]
impl crate::location::executor::OperationEpilogue for RootScopeEpilogue<'_> {
    async fn finish_operation(&self, _operation: &LocationOperation) -> AppResult<Vec<String>> {
        self.app.retire_changed_root(self.tail).await
    }
}

impl AppUseCase {
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
    ) -> AppResult<BTreeMap<String, Option<String>>> {
        let owners: BTreeMap<String, String> = destination_titles
            .iter()
            .filter_map(|title| {
                folder_path_of(title)
                    .map(|folder| (path_to_stored_string(&folder), title.id.clone()))
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

        // Only what is *not* free is recorded: an empty directory nothing owns
        // is nothing to collide with, so the layout is preserved and the
        // directory is reused as it stands.
        let mut occupants: BTreeMap<String, Option<String>> = BTreeMap::new();
        for (path, holds_files) in directories {
            match owners.get(&path) {
                Some(title_id) => {
                    occupants.insert(path, Some(title_id.clone()));
                }
                None if holds_files || has_children.contains(&path) => {
                    occupants.insert(path, None);
                }
                None => {}
            }
        }

        // A destination title may own a folder the walk could not see (an
        // unreadable subtree, or a folder recorded but never created). Its name
        // is still claimed: nothing else may take it.
        for (path, title_id) in owners {
            occupants.entry(path).or_insert(Some(title_id));
        }
        Ok(occupants)
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
        tail: &RootScopeTail,
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
            .list_for_libraries(None, std::slice::from_ref(&library.id), None)
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
        let source_present = library.roots.iter().any(|root| root.id == tail.root_id);
        if still_on_source > 0 {
            warnings.push(format!(
                "{} title(s) still reference {}, so it stays a configured root",
                still_on_source, tail.source_root_path
            ));
        }
        let remove_source =
            source_present && still_on_source == 0 && tail.retirement.permits_source_removal();

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
            && desired
                .iter()
                .zip(library.roots.iter())
                .all(|(want, has)| want.path == has.path && want.is_default == has.is_default);

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

        // The one post-condition only this branch has: the root it retired is
        // gone. FR-078 gives the destination the rest, and they are the shared
        // ones.
        if remove_source && library.roots.iter().any(|root| root.id == tail.root_id) {
            warnings.push(format!(
                "{} is still a configured root after the consolidation completed",
                tail.source_root_path
            ));
        }
        warnings.extend(
            self.assert_root_post_conditions(
                &library,
                &consolidation.destination_root_id,
                tail,
                becomes_default,
                "the consolidation",
                "the legacy root-folder settings still name the retired root",
            )
            .await?,
        );

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

// ── Filesystem helpers ───────────────────────────────────────────────────────

/// The source root's inventory, as [`crate::location::root_scope::classify_root_content`]
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

async fn directory_is_empty(path: &Path) -> bool {
    match tokio::fs::read_dir(path).await {
        Ok(mut entries) => matches!(entries.next_entry().await, Ok(None)),
        Err(_) => false,
    }
}

/// True when the only thing standing between `path` and removal is Scryer's own
/// recycle bin (D6): every remaining entry is the bin directory.
///
/// Unreadable directories answer `false` — the honest fallback is the generic
/// "still holds content" sentence, not a claim about a bin we could not see.
async fn only_the_recycle_bin_remains(path: &Path) -> bool {
    let Ok(mut entries) = tokio::fs::read_dir(path).await else {
        return false;
    };
    let mut saw_the_bin = false;
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => {
                if entry.file_name() == RECYCLE_DIR_NAME {
                    saw_the_bin = true;
                } else {
                    return false;
                }
            }
            Ok(None) => return saw_the_bin,
            Err(_) => return false,
        }
    }
}

/// The path as the filesystem resolves it, or lexically normalized when it does
/// not exist yet.
///
/// A root-scoped operation compares a configured path against a path the user
/// typed, and `/mnt/media` and `/mnt/./media` are the same directory. Resolving
/// what exists and normalizing what does not is what makes "these two overlap"
/// and "this is already a configured root" answerable.
pub(super) async fn canonical_or_lexical(path: &Path) -> PathBuf {
    if let Ok(resolved) = tokio::fs::canonicalize(path).await {
        return resolved;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(resolved) = tokio::fs::canonicalize(parent).await
    {
        return resolved.join(name);
    }
    crate::stored_paths::lexically_normalize(path)
}

/// The operation types the shared root-move runner is allowed to resume.
///
/// A root change resumes through it because it *is* a root move in plan
/// currency: the same instruction set, the same checkpoints, the same
/// reconciler — plus a tail that re-runs harmlessly.
pub(super) fn resumes_through_root_move_runner(operation_type: LocationOperationType) -> bool {
    matches!(
        operation_type,
        LocationOperationType::RootMove
            | LocationOperationType::RootChange
            | LocationOperationType::RootConsolidation
            | LocationOperationType::CrossLibraryTransfer
            | LocationOperationType::Adoption
    )
}
