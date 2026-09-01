//! Root-move planner: the first end-to-end consumer of the shared location
//! machinery (US2, FR-012–013, FR-076, FR-080–082).
//!
//! A root move relocates one or more title folders between roots **inside one
//! library**. The planner's job is to turn a selection plus a destination into
//! two artefacts that must agree exactly (SC-004):
//!
//! - a [`LocationPlan`] the user sees and confirms, built through
//!   [`LocationPlanBuilder`] so counts, sampling, fingerprint, free space,
//!   depth statement, and confirmation rules are the shared ones; and
//! - a [`RootMoveExecutionPlan`], the serialized instruction set the runner
//!   resumes from, so a restarted operation never has to re-derive the plan.
//!
//! # Destination folders are calculated, never copied
//!
//! FR-013: the destination folder name comes from the destination library's
//! active folder-naming policy, calculated fresh from
//! [`crate::library::rename::configured_title_folder_path`] — the same function
//! imports and renames use. Folder-name repair therefore falls out for free: a
//! title whose current folder is stale against the policy lands under the
//! policy's name, and the preview shows that rename before confirmation
//! (US2.2). Nothing here recalculates *file* names; that is out of scope for
//! this feature and stays with the existing rename flow (FR-058).
//!
//! # Layout is preserved beneath the title folder
//!
//! Everything under the title's folder moves with it, at its existing relative
//! path: season folders, specials folders, subtitles, artwork, trickplay
//! directories. The planner walks the source folder rather than the media-file
//! table, so companion assets are never left behind (FR-027). A tracked media
//! file that lives *outside* the title folder is still moved — into the
//! destination folder root — and the preview carries a warning saying so,
//! because silently leaving it on the old root would strand catalog rows on a
//! root the title no longer belongs to.
//!
//! # What the planner does not do
//!
//! It performs no mutation and reaches no download client. Everything that
//! could go stale between preview and execution is re-checked by the admission
//! seam in [`crate::location::execution`] (FR-089).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use scryer_domain::MediaFacet;

use crate::location::adoption::{
    AdoptionAccounting, AdoptionFileProof, TitleAdoptionAccounting,
    plan_reasons as adoption_reasons,
};
use crate::location::classify::{
    ClassificationCounts, DestinationLibraryFacts, DestinationRequest, SelectionClassification,
    TitleClassificationFacts, TitleLocationClass, classify_selection,
};
use crate::location::collisions::{
    CollisionDisposition, CollisionNaming, CollisionPlan, CollisionPlanRequest, ContentFacts,
    DestinationItem, FullHash, IncomingItem, PathCaseRule, RecycleAvailability, plan_collisions,
};
use crate::location::executor::{
    ClassifiedTitleBaseline, OperationWorkPlan, PlannedFile, PlannedTitle, TitleOutcomeCounts,
};
use crate::location::hardlinks::{HardlinkFact, hardlink_warnings};
use crate::location::identity::DestinationIdentityOutcome;
use crate::location::merge::summary::MergePreviewSummary;
use crate::location::transfer_effects::{
    FacetConversion, SettingDisposition, TitleAssociationFacts, collection_statement,
    series_movie_link_statement,
};
use crate::location::model::{
    LocationExecutionMode, LocationOperationType, TitleCheckpointPlacement, VerificationDepth,
};
use crate::location::preview::{
    FreeSpaceEstimate, LocationPlan, LocationPlanBuilder, LocationPlanHeader, PlanItem,
    PlanItemKind,
};
use crate::stored_paths::path_to_stored_string;

/// Reason codes on the plan items this planner emits, so the UI groups and
/// translates rather than parsing prose (C3).
pub mod plan_reasons {
    /// The destination folder name differs from the source folder name because
    /// the naming policy calculated it fresh (FR-013).
    pub const FOLDER_NAME_REPAIR: &str = "folder_name_repair";
    /// The title's catalog record changes root with no filesystem work
    /// (FR-076).
    pub const CATALOG_ONLY_REASSIGNMENT: &str = "catalog_only_reassignment";
    /// A tracked media file lives outside the title's folder and is being
    /// relocated into the destination folder root.
    pub const FILE_OUTSIDE_TITLE_FOLDER: &str = "file_outside_title_folder";
    /// Source files share their inode with another directory entry (FR-085).
    pub const HARDLINKED_SOURCE: &str = "hardlinked_source";
    /// The destination folder already exists and its contents were planned
    /// against (FR-072–075).
    pub const DESTINATION_FOLDER_EXISTS: &str = "destination_folder_exists";
    /// The title changes library: its catalog ownership moves, and settings it
    /// inherited from the source library are replaced by the destination
    /// library's (FR-056).
    pub const LIBRARY_TRANSFER: &str = "library_transfer";
    /// A destination title carries the same name but shares no metadata
    /// identity. It is never merged into (FR-055); the preview says it exists so
    /// two same-named titles in one library are not a surprise.
    pub const SAME_NAMED_DESTINATION_TITLE: &str = "same_named_destination_title";
    /// The destination library's facet differs across the series↔anime
    /// boundary, so the transfer converts the title's facet and recalculates the
    /// folder name — files keep theirs (FR-057, FR-058).
    pub const FACET_CONVERSION: &str = "facet_conversion";
    /// One title-level setting the facet conversion stops anything from reading
    /// (FR-057).
    pub const FACET_SETTING_INVALID: &str = "facet_setting_becomes_invalid";
    /// One title-level setting the facet conversion removes (FR-057).
    pub const FACET_SETTING_RESET: &str = "facet_setting_resets";
    /// One title-level setting the facet conversion leaves in place with a
    /// different consequence (FR-057).
    pub const FACET_SETTING_MEANING_CHANGE: &str = "facet_setting_changes_meaning";
    /// The title's series-movie links and their disposition (FR-060).
    pub const SERIES_MOVIE_LINKS: &str = "series_movie_links";
    /// What a transfer does to the title's collections, when it does anything
    /// worth stating (FR-062).
    pub const COLLECTION_PRESERVATION: &str = "collection_preservation";
    /// The title merges into an existing destination title rather than
    /// transferring into a new one (US7, FR-055, FR-063).
    pub const TITLE_MERGE: &str = "title_merge";
    /// One setting the destination keeps and the source loses (FR-063).
    pub const MERGE_DESTINATION_WINS: &str = "merge_destination_wins";
    /// One reserved `scryer:` setting whose two sides disagreed (OQ9, FR-071).
    pub const MERGE_RESERVED_TAG_CONFLICT: &str = "merge_reserved_tag_conflict";
    /// One category the merge deliberately does not carry (FR-071).
    pub const MERGE_DROPPED_DATA: &str = "merge_dropped_data";
    /// One media-file role the merge resolves per logical slot; never silent
    /// (FR-068–FR-070).
    pub const MERGE_ROLE_CHANGE: &str = "merge_role_change";
    /// The merge cannot run: episode-scoped records reference source identities
    /// that do not map onto the destination (FR-066).
    pub const MERGE_RECORDS_UNMAPPED: &str = "merge_records_unmapped";
}

/// One file of planned work, in the serialized form a resumed run reads back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootMoveFileExecution {
    /// Tracked media file, or `None` for a companion asset.
    pub media_file_id: Option<String>,
    /// Stored path of the source file.
    pub source_path: String,
    /// Stored path of the destination file.
    pub destination_path: String,
    pub size_bytes: u64,
}

impl RootMoveFileExecution {
    pub fn source(&self) -> PathBuf {
        crate::stored_paths::stored_path_to_path_buf(&self.source_path)
    }

    pub fn destination(&self) -> PathBuf {
        crate::stored_paths::stored_path_to_path_buf(&self.destination_path)
    }
}

/// One title's instruction set: what to move, what the catalog becomes, and
/// what cleanup may touch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootMoveTitleExecution {
    pub title_id: String,
    pub title_name: String,
    /// Position in the confirmed plan; the runner walks ascending.
    pub sequence: i64,
    pub class: TitleLocationClass,
    pub source_library_id: String,
    pub source_root_id: String,
    /// Stored path of the folder the title owns today.
    pub source_folder_path: Option<String>,
    pub destination_library_id: String,
    pub destination_root_id: String,
    /// Stored path of the calculated destination folder (FR-013).
    pub destination_folder_path: Option<String>,
    /// Path of the destination root, used to resolve the recycle-bin allowlist.
    pub destination_root_path: Option<String>,
    /// Path of the source root, used to resolve the recycle-bin allowlist.
    pub source_root_path: Option<String>,
    /// `Some(true)` for a same-volume move (rename fast path, FR-032).
    pub same_volume: Option<bool>,
    pub files: Vec<RootMoveFileExecution>,
    /// Source files proven redundant against identical destination content, so
    /// they are recycled instead of copied (FR-073).
    pub deduplicated_sources: Vec<String>,
    /// Media-file ids of those redundant sources, when they were tracked media
    /// rather than companion assets.
    ///
    /// The file is recycled and the destination's copy survives, so the source
    /// row is a duplicate of the survivor's and is removed with it (FR-073:
    /// "retain or merge catalog associations onto the survivor"). Kept beside
    /// the paths rather than folded into them because cleanup acts on paths and
    /// the catalog acts on ids, and defaulted so a plan serialized before the
    /// field existed still loads.
    #[serde(default)]
    pub deduplicated_media_file_ids: Vec<String>,
    /// Destination paths whose file name the collision planner changed so
    /// destination content keeps its own name (FR-074/075). Kept beside
    /// `deduplicated_sources` because Activity counts both (FR-091), and
    /// defaulted so a plan serialized before the field existed still loads.
    #[serde(default)]
    pub renamed_destinations: Vec<String>,
    /// Directories cleanup may remove — but only when they are actually empty
    /// (FR-031). Deepest first.
    pub prune_directories: Vec<String>,
    /// Warnings the preview showed and the completion summary repeats.
    pub warnings: Vec<String>,
    /// The facet the title carries *after* this transfer, when the destination
    /// library's facet differs across the series↔anime boundary (FR-057).
    /// `None` leaves the facet alone, which is every same-facet move.
    ///
    /// Carried on the instruction set rather than re-derived at execution time:
    /// the flip is what the user confirmed, and a resumed run must perform the
    /// conversion the preview described even if the destination library were
    /// edited in between. Defaulted so a plan serialized before FR-057 landed
    /// still reads back.
    #[serde(default)]
    pub converted_facet: Option<MediaFacet>,
    /// Reserved tag prefixes the facet conversion strips at the same write
    /// (`REMATCH_DERIVED_TAG_PREFIXES` precedent, FR-057).
    #[serde(default)]
    pub dropped_tag_prefixes: Vec<String>,
    /// The existing destination title this title merges into (US7, FR-055).
    ///
    /// This is the one field that turns the transfer's catalog flip into the
    /// merge engine's Groups 1–5 transaction: with it set the reconciler runs
    /// [`crate::location::merge::engine::execute_merge`] instead of
    /// `set_title_library_and_root`, and the checkpoint records
    /// `merged_into_title_id`. Defaulted so a plan serialized before US7 landed
    /// still reads back as a plain transfer.
    #[serde(default)]
    pub merge_target_title_id: Option<String>,
}

impl RootMoveTitleExecution {
    /// Whether the catalog flip for this title also changes its library, which
    /// is the difference between a root move and a cross-library transfer
    /// (FR-056). Derived rather than stored so it can never disagree with the
    /// placement the checkpoint records.
    pub fn crosses_libraries(&self) -> bool {
        self.destination_library_id != self.source_library_id
    }

    /// Whether this title's catalog step is the US7 merge rather than the
    /// FR-056 flip.
    pub fn merges(&self) -> bool {
        self.merge_target_title_id.is_some()
    }

    pub fn placement(&self) -> TitleCheckpointPlacement {
        TitleCheckpointPlacement {
            source_library_id: Some(self.source_library_id.clone()),
            source_root_id: Some(self.source_root_id.clone()),
            source_folder_path: self.source_folder_path.clone(),
            destination_library_id: Some(self.destination_library_id.clone()),
            destination_root_id: Some(self.destination_root_id.clone()),
            destination_folder_path: self.destination_folder_path.clone(),
            // FR-091: the executor's `merge_count` is derived from this, counted
            // once the title settles, exactly the way the dedup and rename
            // counters are counted off the plan.
            merged_into_title_id: self.merge_target_title_id.clone(),
            // Read-side only: 0206 has no column for the name, so the store
            // resolves it when it reads the checkpoint back. Writing it here
            // would be writing a field the upsert discards.
            merged_into_title_name: None,
        }
    }

    /// The runner's view of this title.
    pub fn to_planned_title(&self) -> PlannedTitle {
        PlannedTitle {
            title_id: self.title_id.clone(),
            sequence: self.sequence,
            classification: Some(self.class),
            placement: self.placement(),
            files: self
                .files
                .iter()
                .map(|file| PlannedFile {
                    media_file_id: file.media_file_id.clone(),
                    source_path: file.source(),
                    destination_path: file.destination(),
                    size_bytes: file.size_bytes,
                })
                .collect(),
            outcomes: TitleOutcomeCounts {
                dedups: self.deduplicated_sources.len() as i64,
                renames: self.renamed_destinations.len() as i64,
            },
        }
    }

    pub fn bytes_total(&self) -> u64 {
        self.files
            .iter()
            .fold(0_u64, |total, file| total.saturating_add(file.size_bytes))
    }
}

/// The whole confirmed instruction set, persisted as the operation's plan JSON
/// so a restart resumes without rebuilding a preview (FR-033).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RootMoveExecutionPlan {
    pub titles: Vec<RootMoveTitleExecution>,
    /// Titles the selection classified as already at their destination. They
    /// carry no instructions, so they are counted here rather than dropped
    /// (FR-091). Defaulted so a plan serialized before the field existed loads.
    #[serde(default)]
    pub no_op_titles: i64,
    /// Titles the selection could not resolve into work: an outstanding user
    /// decision (FR-016/FR-086) or an incompatible destination (FR-017). A start
    /// refuses a selection holding any of these (FR-016), so this is normally
    /// zero on a running operation and non-zero only on a preview.
    #[serde(default)]
    pub unresolved_titles: i64,
    /// What adoption matched each adopted file on, keyed by its stored
    /// destination path (FR-050–053).
    ///
    /// Carried on the instruction set rather than re-derived at execution time
    /// for the same reason the facet flip is: it is what the user confirmed,
    /// and a resumed run must prove exactly what the preview promised (FR-089).
    /// Empty for every managed move, and defaulted so a plan serialized before
    /// adoption existed still reads back.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adoption_proofs: BTreeMap<String, AdoptionFileProof>,
    /// The root-scoped half of a **change root** operation (US4): the identity
    /// the executor asserts after the path flip, the three content buckets, and
    /// the retirement ordering contract (FR-021, FR-027, FR-028, FR-087).
    ///
    /// It rides on the persisted plan rather than in a column of its own
    /// because resume needs every one of those facts and the plan JSON is
    /// already the operation's recovery journal. `None` for every other
    /// operation type, and defaulted so an operation persisted before root
    /// change existed still resumes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_change: Option<crate::location::root_change::RootChangeTail>,
}

impl RootMoveExecutionPlan {
    /// The runner's work plan, in confirmed order.
    pub fn to_work_plan(&self) -> OperationWorkPlan {
        let mut work_plan = OperationWorkPlan::new(
            self.titles
                .iter()
                .map(RootMoveTitleExecution::to_planned_title)
                .collect(),
        )
        .with_baseline(ClassifiedTitleBaseline {
            no_ops: self.no_op_titles,
            unresolved: self.unresolved_titles,
        });
        // FR-084 for a root change: the operation owns *every* title assigned
        // to the root, not only the ones that produce instructions. A
        // catalog-only title has no files and a blocked title has no execution
        // at all, and both would otherwise stay open to a scan or an import
        // that moved them out from under an operation whose whole subject is
        // the root they sit on.
        if let Some(tail) = self.root_change.as_ref() {
            work_plan = work_plan.with_additional_owned_titles(tail.assigned_title_ids.clone());
        }
        work_plan
    }

    pub fn title(&self, title_id: &str) -> Option<&RootMoveTitleExecution> {
        self.titles.iter().find(|title| title.title_id == title_id)
    }

    /// Bytes this plan will write at the destination, excluding proven
    /// duplicates that are recycled rather than copied.
    pub fn moved_bytes(&self) -> u64 {
        self.titles
            .iter()
            .fold(0_u64, |total, title| {
                total.saturating_add(title.bytes_total())
            })
    }
}

// ── Planner input ────────────────────────────────────────────────────────────

/// One file the planner found beneath (or belonging to) a title's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Tracked media file id, or `None` for a companion asset.
    pub media_file_id: Option<String>,
    /// The file's persisted full-BLAKE3 state (D4, FR-047). Carried on the
    /// draft rather than looked up during planning: planning performs no IO,
    /// and the dedup gate needs the *persisted* hash, never a fresh read.
    /// Companion assets are always [`FullHash::Absent`] — nothing hashes them.
    pub full_blake3: FullHash,
    pub path: PathBuf,
    /// Path relative to the title's folder. `None` when the file lives outside
    /// the folder, in which case it is placed in the destination folder root.
    pub relative_path: Option<PathBuf>,
    pub size_bytes: u64,
}

/// Everything the planner needs about one title. Assembled by
/// [`crate::services::AppUseCase`]; the planning function itself performs no IO
/// so every rule below is testable from literals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootMoveTitleDraft {
    pub title_id: String,
    pub title_name: String,
    pub class: TitleLocationClass,
    pub source_library_id: String,
    pub source_root_id: String,
    pub source_root_path: Option<PathBuf>,
    pub source_folder_path: Option<PathBuf>,
    pub destination_library_id: String,
    pub destination_root_id: String,
    pub destination_root_path: Option<PathBuf>,
    /// The folder the destination naming policy calculates (FR-013).
    pub destination_folder_path: Option<PathBuf>,
    pub files: Vec<SourceFile>,
    /// Directories beneath the source folder, deepest first, that cleanup may
    /// remove once they are empty.
    pub source_directories: Vec<PathBuf>,
    /// `Some(true)` for a same-volume move, `Some(false)` for cross-volume,
    /// `None` when the relationship could not be probed.
    pub same_volume: Option<bool>,
    pub hardlinks: Vec<HardlinkFact>,
    /// Entries already present at the destination folder, when it exists.
    pub destination_entries: Vec<DestinationItem>,
    /// Whether recycling is usable for this title's source root.
    pub recycle: RecycleAvailability,
    /// The explanation for a blocked or unresolved title (FR-016/FR-017).
    pub blocked_reason: Option<String>,
    /// What destination-title detection concluded (FR-055), when the caller ran
    /// it. `None` for a same-library root move, where there is no destination
    /// library to detect against.
    pub destination_identity: Option<DestinationIdentityOutcome>,
    /// The series↔anime conversion this destination performs, with the settings
    /// it affects (FR-057). `None` for every same-facet move.
    pub facet_conversion: Option<FacetConversion>,
    /// Link, collection, and episode counts behind the FR-060–FR-062
    /// statements.
    pub associations: TitleAssociationFacts,
    /// The merge the engine planned for this title at preview time (US7,
    /// FR-066, FR-071), or `None` when the title is not a merge candidate.
    ///
    /// A summary whose [`MergePreviewSummary::is_blocked`] is set never reaches
    /// this planner as a startable title — the caller has already classified it
    /// `NeedsResolution` — but it is carried anyway so the preview can name the
    /// blocking records beside the title rather than only in the class reason.
    pub merge_summary: Option<MergePreviewSummary>,
    /// What the destination already holds for this title, matched against the
    /// catalog (FR-050/051). Set only for a
    /// [`LocationExecutionMode::FilesAlreadyThere`] request; `None` on every
    /// managed move, and `None` on an adoption whose destination folder could
    /// not be read, which the planner reports as a blocked title rather than as
    /// an empty accounting.
    pub adoption: Option<TitleAdoptionAccounting>,
}

impl RootMoveTitleDraft {
    /// Whether this title changes library, which is what makes it a transfer
    /// (FR-056) rather than a root move.
    pub fn crosses_libraries(&self) -> bool {
        self.destination_library_id != self.source_library_id
    }

    /// The existing destination title this title merges into (US7, FR-055).
    ///
    /// Read off the detection outcome rather than stored beside it, so the
    /// planner and the classifier can never disagree about which titles merge.
    pub fn merge_target_title_id(&self) -> Option<&str> {
        self.destination_identity
            .as_ref()
            .and_then(DestinationIdentityOutcome::merge_target)
    }

    /// The destination folder name differs from the source folder name, so the
    /// naming policy repaired it (FR-013, US2.2).
    pub fn repairs_folder_name(&self) -> bool {
        match (
            self.source_folder_path.as_ref(),
            self.destination_folder_path.as_ref(),
        ) {
            (Some(source), Some(destination)) => source.file_name() != destination.file_name(),
            _ => false,
        }
    }
}

/// The whole planning request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootMovePlanRequest {
    pub source_library_id: Option<String>,
    pub destination_library_id: Option<String>,
    pub source_root_id: Option<String>,
    pub destination_root_id: Option<String>,
    /// The user's selection, in the order it was submitted. Every id appears in
    /// the fingerprint (FR-081).
    pub selection: Vec<String>,
    pub titles: Vec<RootMoveTitleDraft>,
    /// The mode the user chose in the move workflow (FR-011). Only
    /// [`LocationExecutionMode::MoveWithScryer`] and
    /// [`LocationExecutionMode::FilesAlreadyThere`] are ever *requested*;
    /// [`LocationExecutionMode::CatalogOnly`] is derived by the planner for a
    /// selection with nothing on disk (FR-076).
    pub mode: LocationExecutionMode,
    pub classification: ClassificationCounts,
    pub verification_depth: VerificationDepth,
    pub free_space: FreeSpaceEstimate,
    /// Case rule of the destination filesystem (FR-090, C7).
    pub case_rule: PathCaseRule,
    /// Label used in collision renames (FR-074).
    pub naming: CollisionNaming,
}

/// A planner result: the plan the user confirms and the instructions the runner
/// executes, built together so they cannot drift (SC-004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRootMove {
    pub plan: LocationPlan,
    pub execution: RootMoveExecutionPlan,
    /// Warnings the preview surfaces above the item list.
    pub warnings: Vec<String>,
}

impl PlannedRootMove {
    pub fn work_plan(&self) -> OperationWorkPlan {
        self.execution.to_work_plan()
    }
}

/// Build the shared preview and the execution plan for a root move.
///
/// Pure: the caller supplies every fact, including the volume relationship, the
/// hardlink facts, and the destination directory listing. The only ordering
/// guarantee callers may rely on is that `execution.titles` follows the input
/// order and carries ascending sequences starting at zero.
pub fn build_root_move_plan(request: &RootMovePlanRequest) -> PlannedRootMove {
    let mode = execution_mode_for(request.mode, &request.titles);
    let header = LocationPlanHeader::new(operation_type_for(mode, &request.titles), mode)
    .with_source(
        request.source_library_id.clone(),
        request.source_root_id.clone(),
    )
    .with_destination(
        request.destination_library_id.clone(),
        request.destination_root_id.clone(),
    )
    .with_selection(request.selection.clone());

    let mut builder = LocationPlanBuilder::new(header);
    builder.classification(request.classification);
    builder.free_space(request.free_space.clone());
    builder.verification_depth(request.verification_depth);

    // The classification counts are the operation's starting truth for the two
    // counters no instruction set can carry: a no-op and an unresolved title
    // produce no work, so without this they would vanish between the preview and
    // Activity (FR-091).
    let mut execution = RootMoveExecutionPlan {
        no_op_titles: request.classification.no_op,
        unresolved_titles: request.classification.needs_resolution
            + request.classification.incompatible,
        ..RootMoveExecutionPlan::default()
    };
    let mut plan_warnings: Vec<String> = Vec::new();

    for (index, draft) in request.titles.iter().enumerate() {
        let (title_execution, items, warnings) = plan_title(request, draft, index as i64);
        builder.extend(items);
        plan_warnings.extend(warnings.iter().cloned());
        // FR-071 + FR-081: the merge summary is both what the preview shows and
        // part of what the fingerprint covers, so it is recorded for every
        // merge candidate — including a blocked one, whose records the user has
        // to see before deciding anything.
        if let Some(summary) = draft.merge_summary.clone() {
            builder.merge(summary);
        }
        if let Some(title_execution) = title_execution {
            // US3: the proof each adopted file was matched on rides the
            // instruction set, so a resumed run re-proves what the preview
            // promised rather than whatever the catalog says later (FR-089).
            if let Some(accounting) = draft.adoption.as_ref() {
                for adopted in &accounting.adopted {
                    execution
                        .adoption_proofs
                        .insert(adopted.destination_path.clone(), adopted.proof.clone());
                }
            }
            execution.titles.push(title_execution);
        }
    }

    PlannedRootMove {
        plan: builder.build(),
        execution,
        warnings: plan_warnings,
    }
}

/// The operation type Activity and the resume path see (FR-091).
///
/// Root move and cross-library transfer share one planner and one runner, so
/// between those two this is a label rather than a branch — but it is a label
/// the user reads, and a selection that changes library is a cross-library
/// transfer, not a root move. A mixed selection (US6.3: libraries A, B, C into
/// A) contains transfers, so the operation is one; nothing about a title's own
/// class changes because of it.
///
/// Adoption is the one type this *does* branch on, because it is the mode the
/// user chose rather than a shape the selection happens to have (US3). It is
/// read off the **effective** mode, so a selection that collapsed to the
/// catalog-only fast path (FR-076) is not filed as an adoption: nothing was
/// adopted, and Activity should not say otherwise.
fn operation_type_for(
    mode: LocationExecutionMode,
    titles: &[RootMoveTitleDraft],
) -> LocationOperationType {
    if mode == LocationExecutionMode::FilesAlreadyThere {
        return LocationOperationType::Adoption;
    }
    if titles.iter().any(RootMoveTitleDraft::crosses_libraries) {
        LocationOperationType::CrossLibraryTransfer
    } else {
        LocationOperationType::RootMove
    }
}

/// A plan with no file-bearing title needs no move mode; the catalog-only mode
/// is what FR-076 asks the UI to skip the chooser for — including for a request
/// that asked for adoption, since there is nothing on disk to adopt.
fn execution_mode_for(
    requested: LocationExecutionMode,
    titles: &[RootMoveTitleDraft],
) -> LocationExecutionMode {
    if !titles.iter().any(|title| title.class.moves_files()) {
        return LocationExecutionMode::CatalogOnly;
    }
    match requested {
        LocationExecutionMode::FilesAlreadyThere => LocationExecutionMode::FilesAlreadyThere,
        _ => LocationExecutionMode::MoveWithScryer,
    }
}

fn plan_title(
    request: &RootMovePlanRequest,
    draft: &RootMoveTitleDraft,
    sequence: i64,
) -> (Option<RootMoveTitleExecution>, Vec<PlanItem>, Vec<String>) {
    let mut items = Vec::new();
    let mut warnings = Vec::new();

    match draft.class {
        // Blocking classes are represented, never omitted (FR-015): the preview
        // lists them so the user can resolve or deselect them (FR-016).
        TitleLocationClass::Incompatible | TitleLocationClass::NeedsResolution => {
            items.push(
                PlanItem::new(PlanItemKind::Blocked)
                    .with_title(draft.title_id.clone())
                    .with_detail(
                        draft
                            .blocked_reason
                            .clone()
                            .unwrap_or_else(|| format!("\"{}\" needs a decision", draft.title_name)),
                    ),
            );
            // FR-066: an unmappable merge is refused per *record*, and the
            // records are what the user acts on. One line each, so the preview
            // can be read rather than parsed.
            items.extend(blocked_merge_items(draft));
            return (None, items, warnings);
        }
        TitleLocationClass::NoOp => {
            items.push(
                PlanItem::new(PlanItemKind::NoOp)
                    .with_title(draft.title_id.clone())
                    .with_detail(format!(
                        "\"{}\" already lives on the destination root",
                        draft.title_name
                    )),
            );
            return (None, items, warnings);
        }
        TitleLocationClass::CatalogOnly => {
            // FR-076 / T033: no move mode, no filesystem work — one catalog item
            // and a checkpoint so the title still appears in Activity.
            items.push(
                PlanItem::new(PlanItemKind::CatalogChange)
                    .with_title(draft.title_id.clone())
                    .with_reason_code(plan_reasons::CATALOG_ONLY_REASSIGNMENT)
                    .with_detail(if draft.crosses_libraries() {
                        format!(
                            "\"{}\" has no tracked files, so only its library and root references change",
                            draft.title_name
                        )
                    } else {
                        format!(
                            "\"{}\" has no tracked files, so only its root reference changes",
                            draft.title_name
                        )
                    }),
            );
            // A fileless title still changes library, and the same-name warning
            // still applies to it (FR-055/FR-056).
            items.extend(transfer_items(draft));
            let transfer_warnings = transfer_warnings(draft);
            warnings.extend(transfer_warnings.iter().cloned());
            let execution = RootMoveTitleExecution {
                title_id: draft.title_id.clone(),
                title_name: draft.title_name.clone(),
                sequence,
                class: draft.class,
                source_library_id: draft.source_library_id.clone(),
                source_root_id: draft.source_root_id.clone(),
                source_folder_path: draft.source_folder_path.as_deref().map(path_to_stored_string),
                destination_library_id: draft.destination_library_id.clone(),
                destination_root_id: draft.destination_root_id.clone(),
                // A fileless title keeps no folder: there is nothing on disk to
                // own, and inventing a folder here would make the next scan
                // claim an empty directory.
                destination_folder_path: None,
                destination_root_path: draft
                    .destination_root_path
                    .as_deref()
                    .map(path_to_stored_string),
                source_root_path: draft.source_root_path.as_deref().map(path_to_stored_string),
                same_volume: draft.same_volume,
                files: Vec::new(),
                deduplicated_sources: Vec::new(),
                deduplicated_media_file_ids: Vec::new(),
                renamed_destinations: Vec::new(),
                prune_directories: Vec::new(),
                warnings: transfer_warnings,
                // FR-076 does not exempt a fileless title from FR-057: it has no
                // bytes to move, and its facet still has to match the library
                // it lands in.
                converted_facet: converted_facet(draft),
                dropped_tag_prefixes: dropped_tag_prefixes(draft),
                // FR-076 does not exempt a fileless title from US7 either: a
                // catalog-only title with a unique destination identity is
                // still a merge, and folding its rows in is the whole change.
                merge_target_title_id: draft.merge_target_title_id().map(str::to_string),
            };
            return (Some(execution), items, warnings);
        }
        TitleLocationClass::RootMove | TitleLocationClass::CrossLibraryTransfer => {}
    }

    // US3: the files are already where they are going. Everything above this
    // line is mode-independent — a blocked title is blocked either way, and a
    // fileless title takes the FR-076 fast path either way.
    if request.mode == LocationExecutionMode::FilesAlreadyThere {
        return plan_adopted_title(draft, sequence, items, warnings);
    }

    let Some(destination_folder) = draft.destination_folder_path.clone() else {
        items.push(
            PlanItem::new(PlanItemKind::Blocked)
                .with_title(draft.title_id.clone())
                .with_detail(format!(
                    "no destination folder could be calculated for \"{}\"",
                    draft.title_name
                )),
        );
        return (None, items, warnings);
    };

    // FR-056: a transfer is a root move that also changes catalog ownership.
    // The preview states the library change in its own right, because the
    // consequences the user cannot see on disk — destination-library defaults
    // replacing the source library's, the destination naming policy calculating
    // the folder — all follow from it.
    items.extend(transfer_items(draft));
    warnings.extend(transfer_warnings(draft));

    // The folder itself: a move, and a rename when the naming policy calculated
    // a different name than the source carries today (FR-013, US2.2).
    let source_folder_display = draft
        .source_folder_path
        .as_deref()
        .map(path_to_stored_string);
    let destination_folder_display = path_to_stored_string(&destination_folder);
    if draft.repairs_folder_name() {
        items.push(
            PlanItem::new(PlanItemKind::Rename)
                .with_title(draft.title_id.clone())
                .with_paths(
                    source_folder_display.clone(),
                    Some(destination_folder_display.clone()),
                )
                .with_reason_code(plan_reasons::FOLDER_NAME_REPAIR)
                .with_detail(format!(
                    "the destination naming policy renames this folder to \"{}\"",
                    destination_folder
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default()
                )),
        );
    }

    // Collisions are only possible where the destination folder already holds
    // something (FR-072–075).
    let collisions = plan_title_collisions(request, draft, &destination_folder);
    if !draft.destination_entries.is_empty() {
        items.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_paths(Option::<String>::None, Some(destination_folder_display.clone()))
                .with_reason_code(plan_reasons::DESTINATION_FOLDER_EXISTS)
                .with_detail(format!(
                    "the destination folder already holds {} item(s); destination content keeps its names",
                    draft.destination_entries.len()
                )),
        );
    }

    let mut files = Vec::new();
    let mut deduplicated_sources = Vec::new();
    let mut deduplicated_media_file_ids = Vec::new();
    let mut renamed_destinations = Vec::new();
    for file in &draft.files {
        let item_id = collision_item_id(file);
        let decision = collisions
            .as_ref()
            .and_then(|plan| plan.decision(&item_id).cloned());
        let final_name = decision
            .as_ref()
            .map(|decision| decision.final_name.clone())
            .unwrap_or_else(|| file_name_of(&file.path));

        let destination_path = match file.relative_path.as_ref() {
            Some(relative) => match relative.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => {
                    destination_folder.join(parent).join(&final_name)
                }
                _ => destination_folder.join(&final_name),
            },
            None => {
                warnings.push(format!(
                    "\"{}\" tracks {} outside its folder; it moves into the destination folder root",
                    draft.title_name,
                    file.path.display()
                ));
                items.push(
                    PlanItem::new(PlanItemKind::Warning)
                        .with_title(draft.title_id.clone())
                        .with_paths(
                            Some(path_to_stored_string(&file.path)),
                            Some(path_to_stored_string(destination_folder.join(&final_name))),
                        )
                        .with_reason_code(plan_reasons::FILE_OUTSIDE_TITLE_FOLDER)
                        .with_detail(
                            "this tracked file is not inside the title's folder; it is placed in the destination folder"
                                .to_string(),
                        ),
                );
                destination_folder.join(&final_name)
            }
        };

        let source_display = path_to_stored_string(&file.path);
        let destination_display = path_to_stored_string(&destination_path);

        match decision.as_ref().map(|decision| decision.disposition) {
            Some(CollisionDisposition::DedupRecycleSource) => {
                // Proven duplicate: no bytes are written, the source copy is
                // recycled (FR-073). The preview says so before confirmation.
                deduplicated_sources.push(source_display.clone());
                if let Some(media_file_id) = file.media_file_id.as_deref() {
                    deduplicated_media_file_ids.push(media_file_id.to_string());
                }
                items.push(
                    PlanItem::new(PlanItemKind::Dedup)
                        .with_title(draft.title_id.clone())
                        .with_paths(Some(source_display), Some(destination_display))
                        .with_size(file.size_bytes)
                        .with_detail(
                            "identical content already exists at the destination; the source copy is recycled"
                                .to_string(),
                        ),
                );
                continue;
            }
            Some(disposition) if disposition.is_rename() => {
                renamed_destinations.push(destination_display.clone());
                items.push(
                    PlanItem::new(PlanItemKind::Rename)
                        .with_title(draft.title_id.clone())
                        .with_paths(Some(source_display.clone()), Some(destination_display.clone()))
                        .with_size(file.size_bytes)
                        .with_detail(format!(
                            "renamed to \"{final_name}\" so destination content keeps its name"
                        )),
                );
            }
            _ => {}
        }

        for warning in decision
            .as_ref()
            .map(|decision| decision.warnings.clone())
            .unwrap_or_default()
        {
            warnings.push(warning.message());
        }

        let mut item = PlanItem::new(PlanItemKind::Move)
            .with_title(draft.title_id.clone())
            .with_paths(Some(source_display.clone()), Some(destination_display.clone()))
            .with_size(file.size_bytes);
        item.media_file_id = file.media_file_id.clone();
        if let Some(same_volume) = draft.same_volume {
            item = item.with_same_volume(same_volume);
        }
        items.push(item);

        files.push(RootMoveFileExecution {
            media_file_id: file.media_file_id.clone(),
            source_path: source_display,
            destination_path: destination_display,
            size_bytes: file.size_bytes,
        });
    }

    // FR-085: hardlink warnings are built from the same facts the completion
    // summary uses, so preview and outcome cannot disagree.
    let recycles_source = draft.same_volume != Some(true) || !deduplicated_sources.is_empty();
    for warning in hardlink_warnings(&draft.hardlinks, draft.same_volume, recycles_source) {
        items.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::HARDLINKED_SOURCE)
                .with_detail(warning.message()),
        );
        warnings.push(warning.message());
    }

    let execution = RootMoveTitleExecution {
        title_id: draft.title_id.clone(),
        title_name: draft.title_name.clone(),
        sequence,
        class: draft.class,
        source_library_id: draft.source_library_id.clone(),
        source_root_id: draft.source_root_id.clone(),
        source_folder_path: source_folder_display,
        destination_library_id: draft.destination_library_id.clone(),
        destination_root_id: draft.destination_root_id.clone(),
        destination_folder_path: Some(destination_folder_display),
        destination_root_path: draft
            .destination_root_path
            .as_deref()
            .map(path_to_stored_string),
        source_root_path: draft.source_root_path.as_deref().map(path_to_stored_string),
        same_volume: draft.same_volume,
        files,
        deduplicated_sources,
        deduplicated_media_file_ids,
        renamed_destinations,
        prune_directories: prune_directories_for(draft),
        warnings: warnings.clone(),
        converted_facet: converted_facet(draft),
        dropped_tag_prefixes: dropped_tag_prefixes(draft),
        merge_target_title_id: draft.merge_target_title_id().map(str::to_string),
    };

    (Some(execution), items, warnings)
}

/// One title's adoption plan (T051, FR-050–053).
///
/// Rides the shared preview core exactly as a managed move does — same items,
/// same fingerprint, same complete counts (FR-051: "apply the same
/// title/folder/library/merge preview as a managed move"). Three things differ,
/// and each one is a spec rule rather than a shortcut:
///
/// - **Nothing is copied.** Adopted files are still [`PlanItemKind::Move`]
///   items, because the content really did move and the verification statement
///   has to cover them (FR-080), but the caller contributes no bytes to the
///   free-space estimate: the destination already holds them.
/// - **Unaccounted media refuses the confirmation.** A title with a missing or
///   ambiguous tracked file emits [`PlanItemKind::Blocked`] items and no
///   instructions at all, so [`LocationPlan::blocks_start`] refuses the start
///   (FR-052, US3.2). It is a refusal, not a warning.
/// - **The source is the user's.** No `prune_directories`, and no
///   `deduplicated_sources` decided here: whether a source copy the user kept is
///   provably redundant is settled at execution time by the verification record
///   (FR-053, and see [`crate::location::adoption::AdoptionFileVerifier`]).
fn plan_adopted_title(
    draft: &RootMoveTitleDraft,
    sequence: i64,
    mut items: Vec<PlanItem>,
    mut warnings: Vec<String>,
) -> (Option<RootMoveTitleExecution>, Vec<PlanItem>, Vec<String>) {
    let Some(destination_folder) = draft.destination_folder_path.clone() else {
        items.push(
            PlanItem::new(PlanItemKind::Blocked)
                .with_title(draft.title_id.clone())
                .with_reason_code(adoption_reasons::ADOPTION_DESTINATION_UNREADABLE)
                .with_detail(format!(
                    "no destination folder could be resolved for \"{}\", so nothing about it can be accounted for",
                    draft.title_name
                )),
        );
        return (None, items, warnings);
    };
    let destination_folder_display = path_to_stored_string(&destination_folder);

    let Some(accounting) = draft.adoption.as_ref() else {
        items.push(
            PlanItem::new(PlanItemKind::Blocked)
                .with_title(draft.title_id.clone())
                .with_paths(
                    Option::<String>::None,
                    Some(destination_folder_display.clone()),
                )
                .with_reason_code(adoption_reasons::ADOPTION_DESTINATION_UNREADABLE)
                .with_detail(format!(
                    "{destination_folder_display} could not be scanned, so \"{}\" cannot be adopted",
                    draft.title_name
                )),
        );
        return (None, items, warnings);
    };

    // FR-056/FR-055 apply to an adoption that also changes library, exactly as
    // they do to a managed transfer: the ownership flip is the same flip.
    items.extend(transfer_items(draft));
    warnings.extend(transfer_warnings(draft));

    let source_folder_display = draft
        .source_folder_path
        .as_deref()
        .map(path_to_stored_string);
    if draft.repairs_folder_name() {
        items.push(
            PlanItem::new(PlanItemKind::Rename)
                .with_title(draft.title_id.clone())
                .with_paths(
                    source_folder_display.clone(),
                    Some(destination_folder_display.clone()),
                )
                .with_reason_code(plan_reasons::FOLDER_NAME_REPAIR)
                .with_detail(
                    "the catalog's folder for this title becomes the folder the content was moved into"
                        .to_string(),
                ),
        );
    }

    // FR-051: every additional file is surfaced. Adoption neither adopts nor
    // removes them — they are the user's content sitting in a folder Scryer is
    // about to own, and silently ignoring them is what FR-027 exists against.
    for file in &accounting.additional {
        items.push(
            PlanItem::new(PlanItemKind::UnmanagedContent)
                .with_title(draft.title_id.clone())
                .with_paths(Option::<String>::None, Some(file.path.clone()))
                .with_size(file.size_bytes)
                .with_reason_code(adoption_reasons::ADOPTION_ADDITIONAL_FILE)
                .with_detail(format!(
                    "{} is at the destination but no tracked file claims it; it is left exactly as it is",
                    file.path
                )),
        );
    }

    // FR-052: a refusal, one item per unaccounted file so the user can act on
    // each, plus the title-level line the preview groups them under.
    if !accounting.unaccounted.is_empty() {
        for file in &accounting.unaccounted {
            let reason_code = match file.accounting {
                AdoptionAccounting::Ambiguous => adoption_reasons::ADOPTION_MEDIA_AMBIGUOUS,
                _ => adoption_reasons::ADOPTION_MEDIA_MISSING,
            };
            items.push(
                PlanItem::new(PlanItemKind::Blocked)
                    .with_title(draft.title_id.clone())
                    .with_paths(
                        Some(file.source_path.clone()),
                        Some(destination_folder_display.clone()),
                    )
                    .with_size(file.size_bytes)
                    .with_reason_code(reason_code)
                    .with_detail(file.detail.clone()),
            );
        }
        let counts = accounting.counts();
        let detail = format!(
            "\"{}\" cannot be adopted: {} tracked file(s) are missing at the destination and {} are ambiguous",
            draft.title_name, counts.missing, counts.ambiguous
        );
        items.push(
            PlanItem::new(PlanItemKind::Blocked)
                .with_title(draft.title_id.clone())
                .with_reason_code(adoption_reasons::ADOPTION_MEDIA_MISSING)
                .with_detail(detail.clone()),
        );
        warnings.push(detail);
        return (None, items, warnings);
    }

    let mut files = Vec::with_capacity(accounting.adopted.len());
    for adopted in &accounting.adopted {
        let mut item = PlanItem::new(PlanItemKind::Move)
            .with_title(draft.title_id.clone())
            .with_paths(
                Some(adopted.source_path.clone()),
                Some(adopted.destination_path.clone()),
            )
            .with_size(adopted.size_bytes)
            .with_reason_code(adoption_reasons::ADOPTED_AT_DESTINATION)
            .with_detail(format!(
                "already at the destination; matched on {} and verified in place",
                adopted.proof.strength.as_str()
            ));
        item.media_file_id = Some(adopted.media_file_id.clone());
        items.push(item);

        files.push(RootMoveFileExecution {
            media_file_id: Some(adopted.media_file_id.clone()),
            source_path: adopted.source_path.clone(),
            destination_path: adopted.destination_path.clone(),
            size_bytes: adopted.size_bytes,
        });
    }

    // FR-053, stated before confirmation: source cleanup is the user's, and the
    // one exception needs proof this preview does not have yet.
    if !files.is_empty() {
        let warning = format!(
            "adoption does not delete anything at \"{}\"'s old location; a source copy is only recycled when the destination is proven identical to the full hash the catalog holds",
            draft.title_name
        );
        items.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_reason_code(adoption_reasons::ADOPTION_REDUNDANT_SOURCE)
                .with_detail(warning.clone()),
        );
        warnings.push(warning);
    }

    let execution = RootMoveTitleExecution {
        title_id: draft.title_id.clone(),
        title_name: draft.title_name.clone(),
        sequence,
        class: draft.class,
        source_library_id: draft.source_library_id.clone(),
        source_root_id: draft.source_root_id.clone(),
        source_folder_path: source_folder_display,
        destination_library_id: draft.destination_library_id.clone(),
        destination_root_id: draft.destination_root_id.clone(),
        destination_folder_path: Some(destination_folder_display),
        destination_root_path: draft
            .destination_root_path
            .as_deref()
            .map(path_to_stored_string),
        source_root_path: draft.source_root_path.as_deref().map(path_to_stored_string),
        // Nothing is copied, so the rename-vs-copy question never arises.
        same_volume: None,
        files,
        // FR-053: the redundancy exception is decided at execution time against
        // a persisted verification record, never optimistically here.
        deduplicated_sources: Vec::new(),
        deduplicated_media_file_ids: Vec::new(),
        renamed_destinations: Vec::new(),
        // Adoption never removes the user's directories. The recycle exception
        // is about redundant *content*; a folder the user still has is theirs.
        prune_directories: Vec::new(),
        warnings: warnings.clone(),
        converted_facet: converted_facet(draft),
        dropped_tag_prefixes: dropped_tag_prefixes(draft),
        merge_target_title_id: draft.merge_target_title_id().map(str::to_string),
    };

    (Some(execution), items, warnings)
}

/// The plan items a cross-library transfer adds on top of the move itself
/// (FR-056, FR-055).
///
/// Two things the user cannot see on disk:
///
/// 1. **The library changes.** Everything the title held explicitly — its
///    reserved `scryer:*` settings, its tags, its monitored state, its history,
///    its requests, its media rows — travels with it, because the title keeps
///    its identity. What does *not* travel is behavior it never held: a quality
///    profile, a naming policy, or a monitoring default it was only inheriting
///    from the source library resolves against the destination library from the
///    moment the flip lands. That is FR-056's "inherited source-library behavior
///    is replaced by destination defaults", and it is a consequence of the flip
///    rather than a step the executor performs.
/// 2. **A same-named title may already be there.** FR-055 forbids merging on
///    title text, so the transfer proceeds — but silently landing a second
///    "The Gift" in one library is exactly the surprise C3 exists to prevent.
fn transfer_items(draft: &RootMoveTitleDraft) -> Vec<PlanItem> {
    if !draft.crosses_libraries() {
        return Vec::new();
    }

    // US7: a merge is not a transfer that also merges — it is a different
    // catalog outcome, and saying "its own settings travel with it" over a
    // merge would be the opposite of FR-063's destination-wins rule.
    let mut items = if let Some(merge_target) = draft.merge_target_title_id() {
        vec![
            PlanItem::new(PlanItemKind::Merge)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::TITLE_MERGE)
                .with_detail(format!(
                    "\"{}\" merges into the existing title {merge_target} in library {}; the destination keeps its title id, metadata identity, monitoring, settings, quality configuration, and naming, and this title's additive data is unioned onto it",
                    draft.title_name, draft.destination_library_id
                )),
        ]
    } else {
        vec![
            PlanItem::new(PlanItemKind::CatalogChange)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::LIBRARY_TRANSFER)
                .with_detail(format!(
                    "\"{}\" moves from library {} to library {}; its own settings, tags, monitoring, history, and requests travel with it, and anything it inherited from {} is replaced by the destination library's defaults",
                    draft.title_name,
                    draft.source_library_id,
                    draft.destination_library_id,
                    draft.source_library_id
                )),
        ]
    };

    if let Some(warning) = same_named_destination_warning(draft) {
        items.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::SAME_NAMED_DESTINATION_TITLE)
                .with_detail(warning),
        );
    }

    items.extend(merge_items(draft));
    items.extend(facet_conversion_items(draft));
    items.extend(association_items(draft));

    items
}

/// FR-066: one `Blocked` item per record the merge engine could not map, so the
/// preview names the table and the identity rather than only the count.
fn blocked_merge_items(draft: &RootMoveTitleDraft) -> Vec<PlanItem> {
    draft
        .merge_summary
        .iter()
        .flat_map(|summary| summary.blocked.iter())
        .map(|record| {
            PlanItem::new(PlanItemKind::Blocked)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::MERGE_RECORDS_UNMAPPED)
                .with_detail(format!(
                    "\"{}\" cannot merge into an existing destination title: {}",
                    draft.title_name,
                    record.summary_line()
                ))
        })
        .collect()
}

/// FR-071 as plan items: what the destination keeps, what disagreed, what is
/// dropped, and which roles change.
///
/// The summary itself rides on the plan (and on the fingerprint) as structured
/// data; these items are the readable form of the same decision, in the same
/// vocabulary every other preview line uses, so Activity and the confirmation
/// dialog do not need a second renderer for merges.
fn merge_items(draft: &RootMoveTitleDraft) -> Vec<PlanItem> {
    let Some(summary) = draft.merge_summary.as_ref() else {
        return Vec::new();
    };

    // A blocked merge performs none of this; its records are listed by
    // `blocked_merge_items` on the blocking branch instead.
    if summary.is_blocked() {
        return Vec::new();
    }

    let mut items = Vec::new();

    for entry in &summary.destination_wins {
        let detail = match (entry.destination_value.as_deref(), entry.source_value.as_deref()) {
            (Some(destination), Some(source)) => format!(
                "the destination's {} ({destination}) wins; \"{}\" loses its own ({source})",
                entry.setting, draft.title_name
            ),
            _ => format!(
                "the destination title's {} wins for the merged title",
                entry.setting
            ),
        };
        items.push(
            PlanItem::new(PlanItemKind::CatalogChange)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::MERGE_DESTINATION_WINS)
                .with_detail(detail),
        );
    }

    for conflict in &summary.reserved_tag_conflicts {
        let setting = conflict
            .setting
            .clone()
            .unwrap_or_else(|| conflict.prefix.clone());
        items.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::MERGE_RESERVED_TAG_CONFLICT)
                .with_detail(match conflict.destination_value.as_deref() {
                    Some(destination) => format!(
                        "{setting}: the destination keeps \"{destination}\" and \"{}\" loses \"{}\"",
                        draft.title_name,
                        conflict.source_value.clone().unwrap_or_default()
                    ),
                    None => format!(
                        "{setting}: the destination has no value, so \"{}\" loses \"{}\"",
                        draft.title_name,
                        conflict.source_value.clone().unwrap_or_default()
                    ),
                }),
        );
    }

    // FR-070: every role change appears, and none of them is silent.
    for change in &summary.role_changes {
        items.push(
            PlanItem::new(PlanItemKind::RoleChange)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::MERGE_ROLE_CHANGE)
                .with_detail(change.describe()),
        );
    }

    for dropped in &summary.dropped {
        if dropped.source_row_count == 0 {
            continue;
        }
        items.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::MERGE_DROPPED_DATA)
                .with_detail(format!(
                    "{} row(s) in {} are not carried over ({}): {}",
                    dropped.source_row_count, dropped.table, dropped.decision, dropped.reason
                )),
        );
    }

    items
}

/// FR-057/FR-058: the conversion itself, then one line per affected setting.
///
/// The per-setting lines are deliberately individual items rather than one
/// sentence with a list in it. FR-057 asks the preview to "show every setting
/// that becomes invalid, resets, or changes meaning" — a count of four with a
/// comma-joined string cannot be grouped, translated, or counted by Activity,
/// and the reason code is what tells the UI which of the three happened.
fn facet_conversion_items(draft: &RootMoveTitleDraft) -> Vec<PlanItem> {
    let Some(conversion) = draft.facet_conversion.as_ref() else {
        return Vec::new();
    };

    let mut items = vec![
        PlanItem::new(PlanItemKind::CatalogChange)
            .with_title(draft.title_id.clone())
            .with_reason_code(plan_reasons::FACET_CONVERSION)
            .with_detail(conversion.headline(&draft.title_name)),
    ];

    for setting in &conversion.settings {
        let reason_code = match setting.disposition {
            SettingDisposition::BecomesInvalid => plan_reasons::FACET_SETTING_INVALID,
            SettingDisposition::Resets => plan_reasons::FACET_SETTING_RESET,
            SettingDisposition::ChangesMeaning => plan_reasons::FACET_SETTING_MEANING_CHANGE,
        };
        items.push(
            PlanItem::new(PlanItemKind::CatalogChange)
                .with_title(draft.title_id.clone())
                .with_reason_code(reason_code)
                .with_detail(setting.detail.clone()),
        );
    }

    items
}

/// FR-060 and FR-062: the dispositions for what the title is linked to and what
/// it contains.
///
/// Both are statements of preservation, and both are emitted only when the
/// title actually has the association in question — a transfer of a title with
/// no links and no seasons says nothing about links or seasons (C3: nothing
/// silent, but also no noise).
fn association_items(draft: &RootMoveTitleDraft) -> Vec<PlanItem> {
    let mut items = Vec::new();

    if let Some(statement) = series_movie_link_statement(
        &draft.title_name,
        draft.associations,
        &draft.destination_library_id,
    ) {
        items.push(
            PlanItem::new(PlanItemKind::CatalogChange)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::SERIES_MOVIE_LINKS)
                .with_detail(statement),
        );
    }

    if let Some(statement) = collection_statement(
        &draft.title_name,
        draft.associations,
        draft.facet_conversion.as_ref(),
    ) {
        items.push(
            PlanItem::new(PlanItemKind::CatalogChange)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::COLLECTION_PRESERVATION)
                .with_detail(statement),
        );
    }

    items
}

/// The warnings a transfer repeats in the completion summary, so the preview and
/// the outcome say the same thing.
fn transfer_warnings(draft: &RootMoveTitleDraft) -> Vec<String> {
    if !draft.crosses_libraries() {
        return Vec::new();
    }
    same_named_destination_warning(draft).into_iter().collect()
}

/// The facet the catalog flip writes, or `None` to leave it alone.
fn converted_facet(draft: &RootMoveTitleDraft) -> Option<MediaFacet> {
    draft
        .facet_conversion
        .as_ref()
        .map(|conversion| conversion.to.clone())
}

/// The reserved tag prefixes the same write strips.
fn dropped_tag_prefixes(draft: &RootMoveTitleDraft) -> Vec<String> {
    draft
        .facet_conversion
        .as_ref()
        .map(|conversion| conversion.dropped_tag_prefixes.clone())
        .unwrap_or_default()
}

/// FR-055's same-name-without-identity statement, phrased once.
fn same_named_destination_warning(draft: &RootMoveTitleDraft) -> Option<String> {
    let outcome = draft.destination_identity.as_ref()?;
    let title_id = outcome.same_name_title_id.as_deref()?;
    let name = outcome
        .same_name_title_name
        .as_deref()
        .unwrap_or(&draft.title_name);
    Some(format!(
        "the destination library already holds a title called \"{name}\" ({title_id}); it shares no metadata identity with \"{}\", so the two are not merged and both will exist there",
        draft.title_name
    ))
}

/// Directories cleanup is allowed to consider, deepest first, with the title's
/// own folder last. Cleanup still removes a directory only when it is empty
/// (FR-031); this list is the *permission*, not the instruction.
fn prune_directories_for(draft: &RootMoveTitleDraft) -> Vec<String> {
    let mut directories: Vec<PathBuf> = draft.source_directories.clone();
    if let Some(folder) = draft.source_folder_path.as_ref()
        && !directories.iter().any(|path| path == folder)
    {
        directories.push(folder.clone());
    }
    // Deepest first, so a parent is only considered after its children were
    // given their chance to disappear.
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    directories.iter().map(path_to_stored_string).collect()
}

fn plan_title_collisions(
    request: &RootMovePlanRequest,
    draft: &RootMoveTitleDraft,
    destination_folder: &Path,
) -> Option<CollisionPlan> {
    if draft.destination_entries.is_empty() {
        return None;
    }

    let mut incoming = Vec::with_capacity(draft.files.len());
    for file in &draft.files {
        let name = file_name_of(&file.path);
        let id = collision_item_id(file);
        let item = if file.media_file_id.is_some() {
            IncomingItem::media(id, name, file.size_bytes)
        } else {
            IncomingItem::companion(id, name, file.size_bytes)
        };
        incoming.push(
            item.with_content(
                ContentFacts::new(file.size_bytes).with_full_hash(file.full_blake3.clone()),
            )
            .with_source_path(path_to_stored_string(&file.path)),
        );
    }

    let _ = destination_folder;
    Some(plan_collisions(
        &CollisionPlanRequest::new(request.case_rule, request.naming.clone())
            .with_recycle(draft.recycle.clone())
            .with_destination(draft.destination_entries.clone())
            .with_incoming(incoming),
    ))
}

/// Stable, collision-free id for one source file inside one title's collision
/// plan: the media file id when there is one, else the stored source path.
fn collision_item_id(file: &SourceFile) -> String {
    match file.media_file_id.as_deref() {
        Some(id) => format!("media:{id}"),
        None => format!("asset:{}", path_to_stored_string(&file.path)),
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "untitled".to_string())
}

/// Reduce a selection classification to the per-title classes the planner
/// consumes, keyed by title id.
pub fn classes_by_title(classification: &SelectionClassification) -> BTreeMap<String, TitleLocationClass> {
    classification
        .titles
        .iter()
        .map(|title| (title.title_id.clone(), title.class))
        .collect()
}

/// Classify a selection and keep the ordering the caller submitted.
///
/// Re-exported here so a planner caller has one entry point rather than two.
pub fn classify_for_root_move(
    titles: &[TitleClassificationFacts],
    destination: &DestinationRequest,
    destination_library: Option<&DestinationLibraryFacts>,
) -> SelectionClassification {
    classify_selection(titles, destination, destination_library)
}

/// Every source path the plan reads, for a caller that wants to probe hardlinks
/// or free space over the whole selection at once.
pub fn source_paths(plan: &RootMoveExecutionPlan) -> Vec<PathBuf> {
    let mut paths: BTreeSet<PathBuf> = BTreeSet::new();
    for title in &plan.titles {
        for file in &title.files {
            paths.insert(file.source());
        }
    }
    paths.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::location::preview::PlanConfirmationRequest;

    fn file(id: Option<&str>, path: &str, relative: Option<&str>, size: u64) -> SourceFile {
        SourceFile {
            media_file_id: id.map(str::to_string),
            full_blake3: FullHash::Absent,
            path: PathBuf::from(path),
            relative_path: relative.map(PathBuf::from),
            size_bytes: size,
        }
    }

    fn draft(class: TitleLocationClass) -> RootMoveTitleDraft {
        RootMoveTitleDraft {
            title_id: "title-1".to_string(),
            title_name: "Some Movie".to_string(),
            class,
            source_library_id: "lib-1".to_string(),
            source_root_id: "root-a".to_string(),
            source_root_path: Some(PathBuf::from("/a")),
            source_folder_path: Some(PathBuf::from("/a/Some Movie")),
            destination_library_id: "lib-1".to_string(),
            destination_root_id: "root-b".to_string(),
            destination_root_path: Some(PathBuf::from("/b")),
            destination_folder_path: Some(PathBuf::from("/b/Some Movie (2024)")),
            files: vec![file(
                Some("mf-1"),
                "/a/Some Movie/movie.mkv",
                Some("movie.mkv"),
                1_000,
            )],
            source_directories: vec![PathBuf::from("/a/Some Movie")],
            same_volume: Some(false),
            hardlinks: Vec::new(),
            destination_entries: Vec::new(),
            recycle: RecycleAvailability::Available,
            blocked_reason: None,
            destination_identity: None,
            facet_conversion: None,
            associations: TitleAssociationFacts::default(),
            merge_summary: None,
            adoption: None,
        }
    }

    fn request(titles: Vec<RootMoveTitleDraft>) -> RootMovePlanRequest {
        request_in_mode(titles, LocationExecutionMode::MoveWithScryer)
    }

    fn request_in_mode(
        titles: Vec<RootMoveTitleDraft>,
        mode: LocationExecutionMode,
    ) -> RootMovePlanRequest {
        let selection = titles.iter().map(|title| title.title_id.clone()).collect();
        RootMovePlanRequest {
            source_library_id: Some("lib-1".to_string()),
            destination_library_id: Some("lib-1".to_string()),
            source_root_id: None,
            destination_root_id: Some("root-b".to_string()),
            selection,
            titles,
            mode,
            classification: ClassificationCounts::default(),
            verification_depth: VerificationDepth::Full,
            free_space: FreeSpaceEstimate::unknown(),
            case_rule: PathCaseRule::CaseSensitive,
            naming: CollisionNaming::from_source_library("Movies"),
        }
    }

    /// US2.2: a stale folder name is repaired by the destination naming policy,
    /// and the preview shows the rename before confirmation (FR-013).
    #[test]
    fn folder_name_repair_is_previewed_as_a_rename() {
        let planned = build_root_move_plan(&request(vec![draft(TitleLocationClass::RootMove)]));

        let renames = planned
            .plan
            .section(PlanItemKind::Rename)
            .expect("rename section");
        assert_eq!(renames.items.total, 1);
        let item = &renames.items.items[0];
        assert_eq!(
            item.reason_code.as_deref(),
            Some(plan_reasons::FOLDER_NAME_REPAIR)
        );
        assert_eq!(item.destination_path.as_deref(), Some("/b/Some Movie (2024)"));
        assert!(
            item.detail
                .as_deref()
                .expect("detail")
                .contains("Some Movie (2024)")
        );

        let execution = planned.execution.title("title-1").expect("title planned");
        assert_eq!(
            execution.destination_folder_path.as_deref(),
            Some("/b/Some Movie (2024)")
        );
        assert_eq!(
            execution.files[0].destination_path,
            "/b/Some Movie (2024)/movie.mkv"
        );
    }

    /// A title whose folder name already matches the policy produces no rename
    /// item — the preview never invents a change (SC-004).
    #[test]
    fn a_matching_folder_name_is_not_reported_as_a_rename() {
        let mut title = draft(TitleLocationClass::RootMove);
        title.destination_folder_path = Some(PathBuf::from("/b/Some Movie"));
        title.files = vec![file(
            Some("mf-1"),
            "/a/Some Movie/movie.mkv",
            Some("movie.mkv"),
            1_000,
        )];

        let planned = build_root_move_plan(&request(vec![title]));

        assert!(planned.plan.section(PlanItemKind::Rename).is_none());
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Move), 1);
    }

    /// Relative layout beneath the title folder is preserved: a season folder
    /// lands as a season folder, not as a flattened file.
    #[test]
    fn relative_layout_is_preserved_under_the_destination_folder() {
        let mut title = draft(TitleLocationClass::RootMove);
        title.title_name = "Some Show".to_string();
        title.source_folder_path = Some(PathBuf::from("/a/Some Show"));
        title.destination_folder_path = Some(PathBuf::from("/b/Some Show"));
        title.files = vec![
            file(
                Some("mf-1"),
                "/a/Some Show/Season 01/S01E01.mkv",
                Some("Season 01/S01E01.mkv"),
                10,
            ),
            file(None, "/a/Some Show/poster.jpg", Some("poster.jpg"), 2),
        ];
        title.source_directories = vec![
            PathBuf::from("/a/Some Show/Season 01"),
            PathBuf::from("/a/Some Show"),
        ];

        let planned = build_root_move_plan(&request(vec![title]));
        let execution = planned.execution.title("title-1").expect("planned");

        assert_eq!(
            execution.files[0].destination_path,
            "/b/Some Show/Season 01/S01E01.mkv"
        );
        assert_eq!(execution.files[1].destination_path, "/b/Some Show/poster.jpg");
        // Deepest first, so a parent is only pruned after its children.
        assert_eq!(
            execution.prune_directories,
            vec![
                "/a/Some Show/Season 01".to_string(),
                "/a/Some Show".to_string()
            ]
        );
    }

    /// FR-076 / T033: the fileless fast path plans a catalog change, no files,
    /// and no move mode.
    #[test]
    fn catalog_only_titles_plan_no_filesystem_work() {
        let mut title = draft(TitleLocationClass::CatalogOnly);
        title.files = Vec::new();
        title.source_folder_path = None;
        title.destination_folder_path = None;

        let planned = build_root_move_plan(&request(vec![title]));

        assert_eq!(planned.plan.header.mode, LocationExecutionMode::CatalogOnly);
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::CatalogChange), 1);
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Move), 0);
        assert!(!planned.plan.verification.applies());

        let execution = planned.execution.title("title-1").expect("planned");
        assert!(execution.files.is_empty());
        assert!(execution.prune_directories.is_empty());
        assert!(execution.destination_folder_path.is_none());
        assert_eq!(execution.destination_root_id, "root-b");
    }

    /// FR-015/FR-016: blocked and no-op titles appear in the plan, and a blocked
    /// item stops the confirmation.
    #[test]
    fn blocked_titles_appear_in_the_plan_and_stop_the_start() {
        let mut blocked = draft(TitleLocationClass::NeedsResolution);
        blocked.title_id = "blocked".to_string();
        blocked.blocked_reason = Some("a download is still importing".to_string());
        let mut no_op = draft(TitleLocationClass::NoOp);
        no_op.title_id = "settled".to_string();

        let mut plan_request = request(vec![blocked, no_op]);
        plan_request.classification = ClassificationCounts {
            no_op: 1,
            needs_resolution: 1,
            ..ClassificationCounts::default()
        };
        let planned = build_root_move_plan(&plan_request);

        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Blocked), 1);
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::NoOp), 1);
        assert!(planned.plan.blocks_start());
        // Neither contributes executable work.
        assert!(planned.execution.titles.is_empty());
        // But both are still counted, or Activity would report a selection
        // smaller than the one the user submitted (FR-091).
        assert_eq!(planned.execution.no_op_titles, 1);
        assert_eq!(planned.execution.unresolved_titles, 1);
        let baseline = planned.work_plan().baseline;
        assert_eq!(baseline.no_ops, 1);
        assert_eq!(baseline.unresolved, 1);

        let confirmation = PlanConfirmationRequest {
            fingerprint: planned.plan.fingerprint.clone(),
            typed_confirmation: None,
        };
        assert_eq!(
            planned.plan.confirm(&confirmation),
            Err(crate::location::preview::PlanConfirmationError::Blocked)
        );
    }

    /// A same-volume move needs no verification pass, so the depth statement
    /// covers no files (FR-032).
    #[test]
    fn same_volume_moves_are_excluded_from_the_verification_statement() {
        let mut title = draft(TitleLocationClass::RootMove);
        title.same_volume = Some(true);

        let planned = build_root_move_plan(&request(vec![title]));

        assert_eq!(planned.plan.verification.depth, VerificationDepth::Full);
        assert!(!planned.plan.verification.applies());
    }

    /// A cross-volume move verifies every copied file at the configured depth.
    #[test]
    fn cross_volume_moves_state_the_depth_that_will_apply() {
        let planned = build_root_move_plan(&request(vec![draft(TitleLocationClass::RootMove)]));

        assert_eq!(planned.plan.verification.depth, VerificationDepth::Full);
        assert_eq!(planned.plan.verification.files, 1);
        assert_eq!(planned.plan.verification.bytes, 1_000);
    }

    /// A proven-identical destination copy deduplicates: no bytes are written
    /// and the source copy is recycled (FR-073).
    #[test]
    fn identical_destination_content_deduplicates_instead_of_copying() {
        let mut title = draft(TitleLocationClass::RootMove);
        title.destination_folder_path = Some(PathBuf::from("/b/Some Movie"));
        title.files = vec![SourceFile {
            media_file_id: Some("mf-1".to_string()),
            full_blake3: FullHash::Absent,
            path: PathBuf::from("/a/Some Movie/movie.mkv"),
            relative_path: Some(PathBuf::from("movie.mkv")),
            size_bytes: 1_000,
        }];
        title.destination_entries = vec![DestinationItem::media("movie.mkv", 1_000)];

        let planned = build_root_move_plan(&request(vec![title]));
        let execution = planned.execution.title("title-1").expect("planned");

        // Content facts carry no full BLAKE3 on either side, so the dedup gate
        // refuses to call them identical (D4) and the incoming file is renamed
        // rather than deduplicated.
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Dedup), 0);
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Rename), 1);
        assert!(execution.deduplicated_sources.is_empty());
        assert_ne!(execution.files[0].destination_path, "/b/Some Movie/movie.mkv");
        // The rename reaches the runner as an outcome to count (FR-091).
        assert_eq!(
            execution.renamed_destinations,
            vec![execution.files[0].destination_path.clone()]
        );
        let work = planned.work_plan();
        assert_eq!(work.titles[0].outcomes.renames, 1);
        assert_eq!(work.titles[0].outcomes.dedups, 0);
    }

    /// D4, unlocked by FR-047: once the backfill job has hashed both sides, the
    /// same look-alike collision that could only be renamed above resolves as a
    /// proven duplicate. This is the whole point of persisting the hash — the
    /// planner does no IO, so a dedup it cannot read off the catalog is a dedup
    /// that never happens.
    #[test]
    fn matching_persisted_hashes_turn_a_look_alike_into_a_proven_dedup() {
        const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        let mut title = draft(TitleLocationClass::RootMove);
        title.destination_folder_path = Some(PathBuf::from("/b/Some Movie"));
        title.files = vec![SourceFile {
            media_file_id: Some("mf-1".to_string()),
            full_blake3: FullHash::known(HASH),
            path: PathBuf::from("/a/Some Movie/movie.mkv"),
            relative_path: Some(PathBuf::from("movie.mkv")),
            size_bytes: 1_000,
        }];
        title.destination_entries = vec![
            DestinationItem::media("movie.mkv", 1_000)
                .with_content(ContentFacts::new(1_000).with_full_blake3(HASH)),
        ];

        let planned = build_root_move_plan(&request(vec![title]));
        let execution = planned.execution.title("title-1").expect("planned");

        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Dedup), 1);
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Rename), 0);
        assert_eq!(
            execution.deduplicated_sources,
            vec!["/a/Some Movie/movie.mkv".to_string()]
        );
        assert!(execution.renamed_destinations.is_empty());
        let work = planned.work_plan();
        assert_eq!(work.titles[0].outcomes.dedups, 1);
        assert_eq!(work.titles[0].outcomes.renames, 0);
    }

    /// A stale hash proves nothing. The row is queued for backfill, and until it
    /// is rehashed the planner must treat the look-alike as unproven rather than
    /// deduplicate on a hash of bytes that are gone (FR-046).
    #[test]
    fn a_stale_persisted_hash_does_not_deduplicate() {
        const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        let mut title = draft(TitleLocationClass::RootMove);
        title.destination_folder_path = Some(PathBuf::from("/b/Some Movie"));
        title.files = vec![SourceFile {
            media_file_id: Some("mf-1".to_string()),
            full_blake3: FullHash::Stale,
            path: PathBuf::from("/a/Some Movie/movie.mkv"),
            relative_path: Some(PathBuf::from("movie.mkv")),
            size_bytes: 1_000,
        }];
        title.destination_entries = vec![
            DestinationItem::media("movie.mkv", 1_000)
                .with_content(ContentFacts::new(1_000).with_full_blake3(HASH)),
        ];

        let planned = build_root_move_plan(&request(vec![title]));

        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Dedup), 0);
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Rename), 1);
    }

    /// FR-085: hardlinked sources warn about the broken link and the recycle
    /// that frees nothing.
    #[test]
    fn hardlinked_sources_produce_the_documented_warnings() {
        let mut title = draft(TitleLocationClass::RootMove);
        title.hardlinks = vec![HardlinkFact {
            path: "/a/Some Movie/movie.mkv".to_string(),
            link_count: crate::location::hardlinks::LinkCount::Known(2),
            size_bytes: 1_000,
        }];

        let planned = build_root_move_plan(&request(vec![title]));

        assert_eq!(planned.warnings.len(), 2);
        assert!(planned.warnings[0].contains("crosses volumes"));
        assert!(planned.warnings[1].contains("frees no space"));
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Warning), 2);
    }

    /// FR-081: the fingerprint covers the selection, so adding a title to the
    /// selection voids an existing confirmation even when the items coincide.
    #[test]
    fn the_fingerprint_covers_the_selection() {
        let first = build_root_move_plan(&request(vec![draft(TitleLocationClass::RootMove)]));

        let mut wider = request(vec![draft(TitleLocationClass::RootMove)]);
        wider.selection.push("also-selected".to_string());
        let second = build_root_move_plan(&wider);

        assert_ne!(first.plan.fingerprint, second.plan.fingerprint);
    }

    /// The execution plan round-trips through JSON: a resumed run reads the
    /// same instructions the confirmation was taken over (FR-033).
    #[test]
    fn the_execution_plan_round_trips_through_json() {
        let planned = build_root_move_plan(&request(vec![draft(TitleLocationClass::RootMove)]));

        let json = serde_json::to_string(&planned.execution).expect("serialize");
        let restored: RootMoveExecutionPlan = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored, planned.execution);
        assert_eq!(restored.to_work_plan(), planned.work_plan());
        assert_eq!(restored.to_work_plan().files_total(), 1);
        assert_eq!(restored.to_work_plan().bytes_total(), 1_000);
        assert_eq!(source_paths(&restored).len(), 1);
    }

    // ── FR-057 / FR-058 / FR-060 / FR-062 ───────────────────────────────────

    /// A transfer draft crossing into another library, with whatever facet
    /// conversion and associations the case under test needs.
    fn transfer_draft(
        conversion: Option<FacetConversion>,
        associations: TitleAssociationFacts,
    ) -> RootMoveTitleDraft {
        let mut draft = draft(TitleLocationClass::CrossLibraryTransfer);
        draft.title_name = "Some Show".to_string();
        draft.destination_library_id = "lib-2".to_string();
        draft.facet_conversion = conversion;
        draft.associations = associations;
        draft
    }

    fn details_for<'a>(planned: &'a PlannedRootMove, reason_code: &str) -> Vec<&'a str> {
        planned
            .plan
            .section(PlanItemKind::CatalogChange)
            .map(|section| {
                section
                    .items
                    .items
                    .iter()
                    .filter(|item| item.reason_code.as_deref() == Some(reason_code))
                    .filter_map(|item| item.detail.as_deref())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// FR-057: the conversion is stated once, and every affected setting gets
    /// its own item carrying the reason code for what happened to it.
    #[test]
    fn a_facet_conversion_states_itself_and_every_setting_it_affects() {
        let conversion = crate::location::transfer_effects::plan_facet_conversion(
            &MediaFacet::Anime,
            &MediaFacet::Series,
            &[
                "scryer:filler-policy:skip_filler".to_string(),
                "scryer:recap-policy:skip_recap".to_string(),
                "scryer:mal-score:8.4".to_string(),
                "scryer:season-folder:enabled".to_string(),
            ],
            TitleAssociationFacts::default(),
        )
        .expect("anime → series converts");

        let planned = build_root_move_plan(&request(vec![transfer_draft(
            Some(conversion),
            TitleAssociationFacts::default(),
        )]));

        let headline = details_for(&planned, plan_reasons::FACET_CONVERSION);
        assert_eq!(headline.len(), 1, "the conversion is stated exactly once");
        assert!(
            headline[0]
                .contains(crate::location::transfer_effects::FILES_KEEP_THEIR_NAMES),
            "FR-058's statement rides the conversion item: {}",
            headline[0]
        );

        let invalid = details_for(&planned, plan_reasons::FACET_SETTING_INVALID);
        assert_eq!(
            invalid.len(),
            2,
            "filler and recap handling each get their own line: {invalid:?}"
        );
        assert!(invalid.iter().any(|detail| detail.contains("filler")));
        assert!(invalid.iter().any(|detail| detail.contains("recap")));

        let reset = details_for(&planned, plan_reasons::FACET_SETTING_RESET);
        assert_eq!(reset.len(), 1, "the derived MAL score resets: {reset:?}");

        let execution = planned.execution.title("title-1").expect("title planned");
        assert_eq!(
            execution.converted_facet,
            Some(MediaFacet::Series),
            "the instruction set carries the post-conversion facet to the flip"
        );
        assert!(
            execution
                .dropped_tag_prefixes
                .contains(&"scryer:mal-score:".to_string())
        );
    }

    /// A same-facet transfer says nothing about facets: SC-004's "the preview
    /// never invents a change" applies to prose as much as to file work.
    #[test]
    fn a_same_facet_transfer_states_no_conversion() {
        let planned = build_root_move_plan(&request(vec![transfer_draft(
            None,
            TitleAssociationFacts::default(),
        )]));

        assert!(details_for(&planned, plan_reasons::FACET_CONVERSION).is_empty());
        assert!(details_for(&planned, plan_reasons::COLLECTION_PRESERVATION).is_empty());
        let execution = planned.execution.title("title-1").expect("title planned");
        assert_eq!(execution.converted_facet, None);
        assert!(execution.dropped_tag_prefixes.is_empty());
    }

    /// FR-060: a title with series-movie links gets an explicit disposition, and
    /// a title with none is not told about links it does not have.
    #[test]
    fn series_movie_links_get_a_stated_disposition() {
        let planned = build_root_move_plan(&request(vec![transfer_draft(
            None,
            TitleAssociationFacts::new(2, 0, 0),
        )]));

        let stated = details_for(&planned, plan_reasons::SERIES_MOVIE_LINKS);
        assert_eq!(stated.len(), 1);
        assert!(stated[0].contains("2 series-movie links"));
        assert!(
            stated[0].contains("lib-2"),
            "the disposition names where they go: {}",
            stated[0]
        );

        let without = build_root_move_plan(&request(vec![transfer_draft(
            None,
            TitleAssociationFacts::default(),
        )]));
        assert!(details_for(&without, plan_reasons::SERIES_MOVIE_LINKS).is_empty());
    }

    /// FR-062: the collection note appears when the facet converts and the title
    /// has seasons, and nowhere else.
    #[test]
    fn collections_are_noted_when_a_conversion_changes_how_they_are_treated() {
        let conversion = crate::location::transfer_effects::plan_facet_conversion(
            &MediaFacet::Series,
            &MediaFacet::Anime,
            &[],
            TitleAssociationFacts::new(0, 3, 40),
        )
        .expect("series → anime converts");

        let planned = build_root_move_plan(&request(vec![transfer_draft(
            Some(conversion),
            TitleAssociationFacts::new(0, 3, 40),
        )]));

        let noted = details_for(&planned, plan_reasons::COLLECTION_PRESERVATION);
        assert_eq!(noted.len(), 1);
        assert!(noted[0].contains("3 of its seasons"));
        assert!(noted[0].contains("40 of its episodes"));
    }

    /// A fileless title still converts: FR-076 removes the filesystem work, not
    /// the catalog invariant that a title's facet matches its library's.
    #[test]
    fn a_catalog_only_transfer_still_carries_the_conversion() {
        let conversion = crate::location::transfer_effects::plan_facet_conversion(
            &MediaFacet::Series,
            &MediaFacet::Anime,
            &[],
            TitleAssociationFacts::default(),
        )
        .expect("converts");
        let mut draft = transfer_draft(Some(conversion), TitleAssociationFacts::default());
        draft.class = TitleLocationClass::CatalogOnly;
        draft.files = Vec::new();

        let planned = build_root_move_plan(&request(vec![draft]));

        let execution = planned.execution.title("title-1").expect("title planned");
        assert_eq!(execution.converted_facet, Some(MediaFacet::Anime));
        assert_eq!(details_for(&planned, plan_reasons::FACET_CONVERSION).len(), 1);
    }

    // ── US7 ─────────────────────────────────────────────────────────────────

    /// A transfer draft whose destination holds the same canonical title, with
    /// the merge the engine planned for it.
    fn merge_draft(summary: Option<MergePreviewSummary>) -> RootMoveTitleDraft {
        use crate::location::identity::{DestinationIdentityOutcome, IdentityCandidate};
        use crate::location::merge::DestinationIdentityMatch;

        let mut draft = transfer_draft(None, TitleAssociationFacts::default());
        draft.destination_identity = Some(DestinationIdentityOutcome {
            match_kind: DestinationIdentityMatch::Unique,
            matched_title_id: Some("destination".to_string()),
            candidates: vec![IdentityCandidate {
                title_id: "destination".to_string(),
                title_name: "Some Show".to_string(),
                shared_identities: Vec::new(),
            }],
            same_name_title_id: None,
            same_name_title_name: None,
        });
        draft.merge_summary = summary;
        draft
    }

    fn merge_summary() -> MergePreviewSummary {
        use crate::location::merge::MergedMediaRole;
        use crate::location::merge::roles::{MediaRoleChange, RoleChangeReason};
        use crate::location::merge::summary::{DestinationWinsEntry, DroppedCategory, ReservedTagConflict};

        MergePreviewSummary {
            source_title_id: "title-1".to_string(),
            destination_title_id: "destination".to_string(),
            destination_wins: vec![DestinationWinsEntry {
                setting: "title id".to_string(),
                destination_value: Some("destination".to_string()),
                source_value: Some("title-1".to_string()),
            }],
            role_changes: vec![MediaRoleChange {
                file_id: "file-in".to_string(),
                source_episode_id: "s-e1".to_string(),
                destination_episode_id: "d-e1".to_string(),
                previous_role: MergedMediaRole::Primary,
                new_role: MergedMediaRole::Additional,
                reason: RoleChangeReason::DestinationPrimaryRetained,
            }],
            reserved_tag_conflicts: vec![ReservedTagConflict {
                prefix: "scryer:quality-profile:".to_string(),
                setting: Some("quality profile".to_string()),
                destination_value: Some("dest-profile".to_string()),
                source_value: Some("source-profile".to_string()),
            }],
            dropped: vec![DroppedCategory {
                table: "pending_releases".to_string(),
                source_row_count: 2,
                decision: "OQ5".to_string(),
                reason: "the delay queue re-derives against the destination profile".to_string(),
            }],
            ..MergePreviewSummary::default()
        }
    }

    fn details_of<'a>(
        planned: &'a PlannedRootMove,
        kind: PlanItemKind,
        reason_code: &str,
    ) -> Vec<&'a str> {
        planned
            .plan
            .section(kind)
            .map(|section| {
                section
                    .items
                    .items
                    .iter()
                    .filter(|item| item.reason_code.as_deref() == Some(reason_code))
                    .filter_map(|item| item.detail.as_deref())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// US7: a merge is a startable transfer whose catalog outcome is the merge
    /// engine's. It is previewed as a merge, not as a library transfer, and its
    /// instruction set carries the target the reconciler and the checkpoint
    /// both read.
    #[test]
    fn a_merge_is_previewed_as_a_merge_and_carries_its_target() {
        let planned = build_root_move_plan(&request(vec![merge_draft(Some(merge_summary()))]));

        let merged = details_of(&planned, PlanItemKind::Merge, plan_reasons::TITLE_MERGE);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].contains("destination"));
        // FR-063 is the opposite of the transfer statement, so the transfer
        // statement must not be there.
        assert!(details_for(&planned, plan_reasons::LIBRARY_TRANSFER).is_empty());

        // FR-071: what wins, what disagreed, what is dropped, which roles move.
        assert_eq!(
            details_of(&planned, PlanItemKind::CatalogChange, plan_reasons::MERGE_DESTINATION_WINS)
                .len(),
            1
        );
        let conflicts = details_of(
            &planned,
            PlanItemKind::Warning,
            plan_reasons::MERGE_RESERVED_TAG_CONFLICT,
        );
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].contains("quality profile"));
        let dropped =
            details_of(&planned, PlanItemKind::Warning, plan_reasons::MERGE_DROPPED_DATA);
        assert_eq!(dropped.len(), 1);
        assert!(dropped[0].contains("pending_releases"));
        // FR-070: the demotion is a line item of its own.
        let roles =
            details_of(&planned, PlanItemKind::RoleChange, plan_reasons::MERGE_ROLE_CHANGE);
        assert_eq!(roles.len(), 1);
        assert!(roles[0].contains("additional"));

        let execution = planned.execution.title("title-1").expect("title planned");
        assert!(execution.merges());
        assert_eq!(
            execution.placement().merged_into_title_id.as_deref(),
            Some("destination")
        );
        // FR-091: the runner counts a merge off the plan, once the title settles.
        assert_eq!(
            planned.work_plan().titles[0]
                .placement
                .merged_into_title_id
                .as_deref(),
            Some("destination")
        );
        // FR-081: the summary is fingerprinted material, not decoration.
        assert_eq!(planned.plan.merges.len(), 1);
    }

    /// FR-066: a merge the engine refused names the blocking records in the
    /// preview, and the plan refuses to start.
    #[test]
    fn a_blocked_merge_names_its_records_and_stops_the_start() {
        use crate::location::merge::map::{MergeBlockReason, MergeBlockedRecord};

        let mut summary = merge_summary();
        summary.blocked = vec![MergeBlockedRecord {
            table: "wanted_items".to_string(),
            reason: MergeBlockReason::UnmappedEpisode,
            source_id: "s-e2".to_string(),
            detail: "no destination episode carries standard S01E02".to_string(),
        }];
        let mut draft = merge_draft(Some(summary));
        // The classifier has already refused it; the planner represents it.
        draft.class = TitleLocationClass::NeedsResolution;
        draft.blocked_reason = Some("wanted_items (unmapped_episode): s-e2".to_string());

        let mut plan_request = request(vec![draft]);
        plan_request.classification = ClassificationCounts {
            needs_resolution: 1,
            ..ClassificationCounts::default()
        };
        let planned = build_root_move_plan(&plan_request);

        assert!(planned.plan.blocks_start());
        assert!(planned.execution.titles.is_empty());
        assert_eq!(planned.execution.unresolved_titles, 1);
        let named =
            details_of(&planned, PlanItemKind::Blocked, plan_reasons::MERGE_RECORDS_UNMAPPED);
        assert_eq!(named.len(), 1);
        assert!(named[0].contains("wanted_items"));
        assert!(named[0].contains("s-e2"));
        // The refusal is still fingerprinted: re-previewing after the user maps
        // the episode has to produce a different plan.
        assert_eq!(planned.plan.merges.len(), 1);
        assert!(planned.plan.merges[0].is_blocked());
    }

    /// A root move is not root-wide, so it takes the simple confirmation
    /// (FR-029 applies to root changes and consolidations only).
    #[test]
    fn a_root_move_takes_the_simple_confirmation() {
        let planned = build_root_move_plan(&request(vec![draft(TitleLocationClass::RootMove)]));

        assert!(!planned.plan.confirmation.requires_typed_confirmation());
        assert_eq!(
            planned.plan.confirm(&PlanConfirmationRequest {
                fingerprint: planned.plan.fingerprint.clone(),
                typed_confirmation: None,
            }),
            Ok(())
        );
    }

    #[test]
    fn classes_by_title_indexes_the_classification() {
        let classification = SelectionClassification {
            titles: vec![crate::location::classify::TitleClassification {
                title_id: "t1".to_string(),
                class: TitleLocationClass::RootMove,
                source_library_id: "lib".to_string(),
                source_root_id: "other-root".to_string(),
                source_folder_path: Some("/media/other-root/T1".to_string()),
                destination_library_id: "lib".to_string(),
                destination_root_id: "root".to_string(),
                reason_code: None,
                reason: None,
                destination_identity: None,
                facet_conversion: None,
                associations: TitleAssociationFacts::default(),
            }],
            counts: ClassificationCounts {
                root_move: 1,
                ..ClassificationCounts::default()
            },
        };

        let indexed = classes_by_title(&classification);

        assert_eq!(indexed.get("t1"), Some(&TitleLocationClass::RootMove));
    }

    // ── US3: adoption plans (T051) ───────────────────────────────────────────

    use crate::location::adoption::{
        AdoptedMediaFile, AdoptionAccounting, AdoptionFileProof, AdoptionMatchStrength,
        TitleAdoptionAccounting, UnaccountedMediaFile,
    };

    fn adopted_draft(accounting: TitleAdoptionAccounting) -> RootMoveTitleDraft {
        let mut draft = draft(TitleLocationClass::RootMove);
        draft.adoption = Some(accounting);
        draft
    }

    fn adopted_file(strength: AdoptionMatchStrength) -> AdoptedMediaFile {
        AdoptedMediaFile {
            media_file_id: "mf-1".to_string(),
            source_path: "/a/Some Movie/movie.mkv".to_string(),
            destination_path: "/b/Some Movie (2024)/movie.mkv".to_string(),
            size_bytes: 1_000,
            proof: AdoptionFileProof {
                strength,
                full_blake3: matches!(strength, AdoptionMatchStrength::FullHash)
                    .then(|| "abcd".to_string()),
                signature: None,
            },
        }
    }

    /// US3.1: the adoption preview is the managed-move preview — same item
    /// vocabulary, same fingerprint machinery — with the mode and type the user
    /// chose (FR-051).
    #[test]
    fn an_adoption_plan_is_typed_as_an_adoption_and_moves_no_bytes_of_its_own() {
        let accounting = TitleAdoptionAccounting {
            adopted: vec![adopted_file(AdoptionMatchStrength::IdentityOnly)],
            ..TitleAdoptionAccounting::default()
        };
        let planned = build_root_move_plan(&request_in_mode(
            vec![adopted_draft(accounting)],
            LocationExecutionMode::FilesAlreadyThere,
        ));

        assert_eq!(
            planned.plan.header.operation_type,
            LocationOperationType::Adoption
        );
        assert_eq!(
            planned.plan.header.mode,
            LocationExecutionMode::FilesAlreadyThere
        );
        assert!(!planned.plan.blocks_start());
        assert_eq!(planned.execution.titles.len(), 1);
        assert_eq!(planned.execution.titles[0].files.len(), 1);
        // Nothing is copied, so nothing is recycled and nothing is pruned
        // (FR-053: source cleanup is the user's).
        assert!(planned.execution.titles[0].deduplicated_sources.is_empty());
        assert!(planned.execution.titles[0].prune_directories.is_empty());
        // The verification statement still covers the adopted bytes: they are
        // read at the operation's depth even though they were never written.
        assert_eq!(planned.plan.verification.files, 1);
        assert_eq!(planned.plan.verification.bytes, 1_000);
    }

    #[test]
    fn an_adoption_carries_the_match_proof_into_the_instruction_set() {
        let accounting = TitleAdoptionAccounting {
            adopted: vec![adopted_file(AdoptionMatchStrength::FullHash)],
            ..TitleAdoptionAccounting::default()
        };
        let planned = build_root_move_plan(&request_in_mode(
            vec![adopted_draft(accounting)],
            LocationExecutionMode::FilesAlreadyThere,
        ));

        let proof = planned
            .execution
            .adoption_proofs
            .get("/b/Some Movie (2024)/movie.mkv")
            .expect("the adopted file's proof rides the plan");
        assert_eq!(proof.strength, AdoptionMatchStrength::FullHash);
        assert_eq!(proof.full_blake3.as_deref(), Some("abcd"));
    }

    /// US3.2: a tracked file that is not accounted for at the destination
    /// refuses the confirmation — a refusal, not a warning (FR-052).
    #[test]
    fn unaccounted_tracked_media_blocks_the_confirmation() {
        let accounting = TitleAdoptionAccounting {
            unaccounted: vec![UnaccountedMediaFile {
                media_file_id: "mf-1".to_string(),
                source_path: "/a/Some Movie/movie.mkv".to_string(),
                size_bytes: 1_000,
                accounting: AdoptionAccounting::Missing,
                detail: "no file at the destination matches movie.mkv".to_string(),
            }],
            ..TitleAdoptionAccounting::default()
        };
        let planned = build_root_move_plan(&request_in_mode(
            vec![adopted_draft(accounting)],
            LocationExecutionMode::FilesAlreadyThere,
        ));

        assert!(planned.plan.blocks_start());
        assert!(
            planned.execution.titles.is_empty(),
            "a blocked title contributes no instructions"
        );
        let refusal = planned.plan.confirm(&PlanConfirmationRequest {
            fingerprint: planned.plan.fingerprint.clone(),
            typed_confirmation: None,
        });
        assert_eq!(
            refusal,
            Err(crate::location::preview::PlanConfirmationError::Blocked)
        );
    }

    #[test]
    fn an_ambiguous_match_blocks_with_its_own_reason_code() {
        let accounting = TitleAdoptionAccounting {
            unaccounted: vec![UnaccountedMediaFile {
                media_file_id: "mf-1".to_string(),
                source_path: "/a/Some Movie/movie.mkv".to_string(),
                size_bytes: 1_000,
                accounting: AdoptionAccounting::Ambiguous,
                detail: "two destination files could be movie.mkv".to_string(),
            }],
            ..TitleAdoptionAccounting::default()
        };
        let planned = build_root_move_plan(&request_in_mode(
            vec![adopted_draft(accounting)],
            LocationExecutionMode::FilesAlreadyThere,
        ));

        let blocked = planned
            .plan
            .section(PlanItemKind::Blocked)
            .expect("blocked section");
        assert!(blocked.items.items.iter().any(|item| {
            item.reason_code.as_deref()
                == Some(crate::location::adoption::plan_reasons::ADOPTION_MEDIA_AMBIGUOUS)
        }));
    }

    /// FR-051: a destination file nothing claims is surfaced, never ignored.
    #[test]
    fn additional_destination_files_are_surfaced_without_blocking() {
        let accounting = TitleAdoptionAccounting {
            adopted: vec![adopted_file(AdoptionMatchStrength::IdentityOnly)],
            additional: vec![crate::location::adoption::AdditionalDestinationFile {
                path: "/b/Some Movie (2024)/extra.mkv".to_string(),
                size_bytes: 42,
            }],
            ..TitleAdoptionAccounting::default()
        };
        let planned = build_root_move_plan(&request_in_mode(
            vec![adopted_draft(accounting)],
            LocationExecutionMode::FilesAlreadyThere,
        ));

        assert!(!planned.plan.blocks_start());
        let unmanaged = planned
            .plan
            .section(PlanItemKind::UnmanagedContent)
            .expect("unmanaged content section");
        assert_eq!(unmanaged.items.total, 1);
    }

    /// FR-076 outranks the mode: a fileless title has nothing to adopt, so the
    /// plan collapses to the catalog-only fast path and is not filed as an
    /// adoption.
    #[test]
    fn a_fileless_selection_in_adoption_mode_is_still_the_catalog_only_fast_path() {
        let planned = build_root_move_plan(&request_in_mode(
            vec![draft(TitleLocationClass::CatalogOnly)],
            LocationExecutionMode::FilesAlreadyThere,
        ));

        assert_eq!(planned.plan.header.mode, LocationExecutionMode::CatalogOnly);
        assert_eq!(
            planned.plan.header.operation_type,
            LocationOperationType::RootMove
        );
    }

    #[test]
    fn an_adoption_with_no_accounting_at_all_is_blocked_rather_than_empty() {
        let planned = build_root_move_plan(&request_in_mode(
            vec![draft(TitleLocationClass::RootMove)],
            LocationExecutionMode::FilesAlreadyThere,
        ));

        assert!(planned.plan.blocks_start());
        assert!(planned.execution.titles.is_empty());
    }
}
