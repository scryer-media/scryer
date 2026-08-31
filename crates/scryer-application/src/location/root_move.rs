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

use crate::location::classify::{
    ClassificationCounts, DestinationLibraryFacts, DestinationRequest, SelectionClassification,
    TitleClassificationFacts, TitleLocationClass, classify_selection,
};
use crate::location::collisions::{
    CollisionDisposition, CollisionNaming, CollisionPlan, CollisionPlanRequest, ContentFacts,
    DestinationItem, IncomingItem, PathCaseRule, RecycleAvailability, plan_collisions,
};
use crate::location::executor::{OperationWorkPlan, PlannedFile, PlannedTitle};
use crate::location::hardlinks::{HardlinkFact, hardlink_warnings};
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
    /// Directories cleanup may remove — but only when they are actually empty
    /// (FR-031). Deepest first.
    pub prune_directories: Vec<String>,
    /// Warnings the preview showed and the completion summary repeats.
    pub warnings: Vec<String>,
}

impl RootMoveTitleExecution {
    pub fn placement(&self) -> TitleCheckpointPlacement {
        TitleCheckpointPlacement {
            source_library_id: Some(self.source_library_id.clone()),
            source_root_id: Some(self.source_root_id.clone()),
            source_folder_path: self.source_folder_path.clone(),
            destination_library_id: Some(self.destination_library_id.clone()),
            destination_root_id: Some(self.destination_root_id.clone()),
            destination_folder_path: self.destination_folder_path.clone(),
            merged_into_title_id: None,
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
}

impl RootMoveExecutionPlan {
    /// The runner's work plan, in confirmed order.
    pub fn to_work_plan(&self) -> OperationWorkPlan {
        OperationWorkPlan::new(
            self.titles
                .iter()
                .map(RootMoveTitleExecution::to_planned_title)
                .collect(),
        )
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
}

impl RootMoveTitleDraft {
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
    let header = LocationPlanHeader::new(
        LocationOperationType::RootMove,
        execution_mode_for(&request.titles),
    )
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

    let mut execution = RootMoveExecutionPlan::default();
    let mut plan_warnings: Vec<String> = Vec::new();

    for (index, draft) in request.titles.iter().enumerate() {
        let (title_execution, items, warnings) = plan_title(request, draft, index as i64);
        builder.extend(items);
        plan_warnings.extend(warnings.iter().cloned());
        if let Some(title_execution) = title_execution {
            execution.titles.push(title_execution);
        }
    }

    PlannedRootMove {
        plan: builder.build(),
        execution,
        warnings: plan_warnings,
    }
}

/// A plan with no file-bearing title needs no move mode; the catalog-only mode
/// is what FR-076 asks the UI to skip the chooser for.
fn execution_mode_for(titles: &[RootMoveTitleDraft]) -> LocationExecutionMode {
    if titles.iter().any(|title| title.class.moves_files()) {
        LocationExecutionMode::MoveWithScryer
    } else {
        LocationExecutionMode::CatalogOnly
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
                    .with_detail(format!(
                        "\"{}\" has no tracked files, so only its root reference changes",
                        draft.title_name
                    )),
            );
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
                prune_directories: Vec::new(),
                warnings: Vec::new(),
            };
            return (Some(execution), items, warnings);
        }
        TitleLocationClass::RootMove | TitleLocationClass::CrossLibraryTransfer => {}
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
                            Some(path_to_stored_string(&destination_folder.join(&final_name))),
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
        prune_directories: prune_directories_for(draft),
        warnings: warnings.clone(),
    };

    (Some(execution), items, warnings)
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
            item.with_content(ContentFacts::new(file.size_bytes))
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
        }
    }

    fn request(titles: Vec<RootMoveTitleDraft>) -> RootMovePlanRequest {
        let selection = titles.iter().map(|title| title.title_id.clone()).collect();
        RootMovePlanRequest {
            source_library_id: Some("lib-1".to_string()),
            destination_library_id: Some("lib-1".to_string()),
            source_root_id: None,
            destination_root_id: Some("root-b".to_string()),
            selection,
            titles,
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

        let planned = build_root_move_plan(&request(vec![blocked, no_op]));

        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Blocked), 1);
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::NoOp), 1);
        assert!(planned.plan.blocks_start());
        // Neither contributes executable work.
        assert!(planned.execution.titles.is_empty());

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
                destination_library_id: "lib".to_string(),
                destination_root_id: "root".to_string(),
                reason_code: None,
                reason: None,
            }],
            counts: ClassificationCounts {
                root_move: 1,
                ..ClassificationCounts::default()
            },
        };

        let indexed = classes_by_title(&classification);

        assert_eq!(indexed.get("t1"), Some(&TitleLocationClass::RootMove));
    }
}
