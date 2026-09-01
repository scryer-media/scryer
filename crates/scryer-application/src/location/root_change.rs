//! Root-change planner: replacing one root's path with a new, unconfigured path
//! (US4, T060–T063, FR-020–FR-029, FR-087).
//!
//! A root change is the one location workflow whose selection is not the user's:
//! it is *every* title assigned to the root. FR-023 makes that explicit —
//! "A root change MUST account for every title assigned to the source root;
//! titles cannot be excluded" — so this planner has no notion of a selection to
//! filter. It takes the root's whole title set plus the filesystem inventory the
//! caller scanned, and produces a plan in which every title and every byte under
//! the source root is placed in exactly one bucket.
//!
//! # What this module is not
//!
//! Pure. No IO, no clock, no catalog access: the caller assembles the drafts and
//! the inventory, exactly as [`crate::location::root_move`] and
//! [`crate::location::transfer_effects`] do, so every rule below is testable from
//! literals. It also plans only the US4 half of the root-scoped workflow — a
//! change to a **new, unconfigured** path. Consolidating into an existing
//! non-empty root (US5, FR-024–FR-026) is a second planner: it needs the
//! collision engine, destination-title identity, and the merge engine, none of
//! which a change to an empty destination can encounter.
//!
//! # Layout is preserved, names are not recalculated
//!
//! FR-026: "Root replacement SHOULD preserve the source root's relative folder
//! layout where practical". So unlike a title-scoped root move (FR-013), this
//! planner never asks the naming policy for a folder name. Every path under the
//! source root lands at the same path relative to the destination root, however
//! deeply nested. That also means a title folder need not be a direct child of
//! the root — the absorbed prototype required that, the spec does not.
//!
//! # How this lowers onto the shared operation model (T060)
//!
//! The planner emits the same two artefacts the root-move planner does, so the
//! executor, Activity, resume, and the ownership guard are the ones already
//! written:
//!
//! - a [`LocationPlan`] built through [`LocationPlanBuilder`], with header type
//!   [`LocationOperationType::RootChange`]. Because the shared
//!   [`crate::location::preview::PlanConfirmation::for_operation`] derives the
//!   confirmation requirement from the operation type, FR-029's stronger typed
//!   confirmation falls out of the header alone — this module defines no
//!   confirmation rule of its own, and the absorbed prototype's separate
//!   `RELOCATION_CONFIRMATION` phrase is dropped in favour of the shared one.
//! - a [`RootMoveExecutionPlan`], the instruction set the runner resumes from.
//!   The root change states FR-021 *in that currency*: every
//!   [`RootMoveTitleExecution`] carries the **same** `source_root_id` and
//!   `destination_root_id`, the same library on both sides, and differing
//!   `source_root_path` / `destination_root_path`. A root whose path changes is,
//!   in plan terms, a move between two paths of one root — which is precisely
//!   what synthetic root ids (FR-078, T010/T013) made expressible.
//!
//! The facts the shared plan currency cannot carry — the identity/role retention
//! the executor asserts, the unmanaged-content buckets, and the retirement
//! ordering contract — ride beside the plan on [`PlannedRootChange`] rather than
//! being smuggled into plan-item prose.
//!
//! # Two gates, not one (T061, T062)
//!
//! The spec blocks two different things for two different reasons, and this
//! module keeps them apart:
//!
//! - **Start** is blocked by a blocked title. FR-086 keeps a title with an active
//!   download or import out of a move; FR-023 forbids excluding it from a root
//!   change. A root change holding one therefore cannot start at all, which is
//!   what the shared [`LocationPlan::blocks_start`] already does for a
//!   `NeedsResolution` classification. FR-023's "Blocked titles MUST be repaired
//!   before the source root is retired" is the same gate seen from the far end.
//! - **Source removal** is blocked by unexplained content. US4 scenario 3:
//!   unknown files "are never silently deleted or abandoned, and root removal is
//!   blocked until the user resolves them". That does not stop the titles from
//!   moving; it stops cleanup from taking the source location away underneath
//!   content Scryer cannot explain (FR-028).
//!
//! # Retirement ordering (T063, FR-087)
//!
//! FR-087: "the source root's configuration is retired only after all recycling
//! for the operation completes; resume treats an in-retirement root as still
//! allowlisted for recycling." The mechanism behind that requirement is that the
//! recycle bin's allowlist is derived from a configured media root
//! (`AppUseCase::recycle_bin_config_for_media_root`), so flipping the root's
//! configured path to the destination before the last source file is recycled
//! would make the bin reject every remaining source file — and FR-073 forbids
//! falling back to permanent deletion. [`RootRetirementContract`] carries the
//! source path the allowlist must keep accepting for the operation's whole life,
//! resume included, plus the blockers that must be empty before the configuration
//! flip may happen at all.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::location::classify::{ClassificationCounts, TitleLocationClass};
use crate::location::collisions::is_canonical_sidecar_name;
use crate::location::hardlinks::{HardlinkFact, hardlink_warnings};
use crate::location::model::{
    LocationExecutionMode, LocationOperationType, VerificationDepth,
};
use crate::location::preview::{
    FreeSpaceEstimate, LocationPlan, LocationPlanBuilder, LocationPlanHeader, PlanItem,
    PlanItemKind,
};
use crate::location::root_move::{
    RootMoveExecutionPlan, RootMoveFileExecution, RootMoveTitleExecution, SourceFile,
};
use crate::stored_paths::path_to_stored_string;

/// Reason codes on the plan items this planner emits, so the UI groups and
/// translates rather than parsing prose (C3).
pub mod plan_reasons {
    /// The root keeps its synthetic id, its role, its default status, and every
    /// title assignment; only its path changes (FR-021, FR-078).
    pub const ROOT_IDENTITY_RETAINED: &str = "root_identity_retained";
    /// The title has no folder to move, so only its stored paths change
    /// (FR-076).
    pub const CATALOG_ONLY_ROOT_CHANGE: &str = "catalog_only_root_change";
    /// The title cannot enter the root change until the user repairs it; it
    /// cannot be excluded either (FR-023, FR-086).
    pub const TITLE_BLOCKED_FOR_ROOT_CHANGE: &str = "title_blocked_for_root_change";
    /// Content at the source root the catalog does not explain (FR-027).
    pub const UNKNOWN_ROOT_CONTENT: &str = "unknown_root_content";
    /// Why the source location cannot be removed once the titles have moved
    /// (FR-028, FR-023).
    pub const SOURCE_RETIREMENT_BLOCKED: &str = "source_retirement_blocked";
    /// A tracked file lives outside its title's folder but inside the root; it
    /// keeps its root-relative position (FR-026).
    pub const FILE_OUTSIDE_TITLE_FOLDER: &str = "file_outside_title_folder";
    /// Source files share their inode with another directory entry (FR-085).
    pub const HARDLINKED_SOURCE: &str = "hardlinked_source";
}

/// Machine-readable codes for the reasons a source root cannot be retired.
pub mod retirement_blockers {
    /// At least one assigned title is blocked and must be repaired first
    /// (FR-023).
    pub const BLOCKED_TITLES: &str = "blocked_titles";
    /// Content Scryer cannot explain still sits at the source (FR-027, FR-028).
    pub const UNEXPLAINED_SOURCE_CONTENT: &str = "unexplained_source_content";
}

// ── Unmanaged-content classification (T062, FR-027) ──────────────────────────

/// What the catalog can say about one entry found beneath the source root.
///
/// The vocabulary is FR-027's: "managed title content, recognized companion
/// assets (NFO, subtitles, artwork, trickplay, etc. — which move with their
/// title), and unrelated root-level content."
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RootContentClass {
    /// A file the catalog tracks as media for a title assigned to this root.
    Managed,
    /// An untracked file inside a title's owned folder: sidecars, artwork,
    /// subtitles, trickplay. It travels with its title.
    Companion,
    /// Anything else. Never deleted, never abandoned, and it blocks removal of
    /// the source location (FR-027, FR-028).
    Unknown,
}

impl RootContentClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::Companion => "companion",
            Self::Unknown => "unknown",
        }
    }
}

/// What the caller's scan found at one path under the source root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootEntryKind {
    File { size_bytes: u64 },
    Directory,
}

/// One entry of the source root's filesystem inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootEntry {
    pub path: PathBuf,
    pub kind: RootEntryKind,
}

impl RootEntry {
    pub fn file(path: impl Into<PathBuf>, size_bytes: u64) -> Self {
        Self {
            path: path.into(),
            kind: RootEntryKind::File { size_bytes },
        }
    }

    pub fn directory(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: RootEntryKind::Directory,
        }
    }

    fn size_bytes(&self) -> u64 {
        match self.kind {
            RootEntryKind::File { size_bytes } => size_bytes,
            RootEntryKind::Directory => 0,
        }
    }

    fn is_directory(&self) -> bool {
        matches!(self.kind, RootEntryKind::Directory)
    }
}

/// One classified entry, in the stored-path form the preview and Activity show.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassifiedRootEntry {
    pub path: String,
    pub size_bytes: u64,
    pub class: RootContentClass,
    /// A canonical folder-level sidecar (`movie.nfo`, `tvshow.nfo`,
    /// `season.nfo`), recognized with the collision engine's own
    /// [`is_canonical_sidecar_name`] so the two can never disagree about what a
    /// sidecar is.
    pub canonical_sidecar: bool,
}

/// Every entry under the source root, in FR-027's three buckets, plus the
/// directory facts cleanup is allowed to act on (FR-028).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RootContentInventory {
    pub managed: Vec<ClassifiedRootEntry>,
    pub companions: Vec<ClassifiedRootEntry>,
    pub unknown: Vec<ClassifiedRootEntry>,
    /// Directories outside every title folder that hold nothing unexplained, so
    /// cleanup may remove them **once they are empty** — deepest first (FR-028).
    pub prunable_directories: Vec<String>,
    /// Directories outside every title folder that hold unexplained content and
    /// are therefore left standing, with their contents, forever (FR-027).
    pub retained_directories: Vec<String>,
}

impl RootContentInventory {
    /// FR-028: unexplained content at the source stops the source location from
    /// being removed.
    pub fn blocks_source_removal(&self) -> bool {
        !self.unknown.is_empty()
    }

    pub fn unknown_bytes(&self) -> u64 {
        self.unknown
            .iter()
            .fold(0_u64, |total, entry| total.saturating_add(entry.size_bytes))
    }

    /// Every scanned file, across the three buckets. The FR-027 counterpart of
    /// [`TitleAccounting::accounts_for_every_title`]: nothing found under the
    /// root is dropped on the way into a bucket.
    pub fn entry_count(&self) -> usize {
        self.managed.len() + self.companions.len() + self.unknown.len()
    }
}

/// Classify every scanned entry under `source_root` (T062, FR-027).
///
/// `title_folders` are the folders titles assigned to this root own;
/// `tracked_media_paths` are the paths the catalog tracks as media. Both come
/// from the caller — this function reads nothing.
///
/// Entries that are not under `source_root` are classified [`Unknown`] rather
/// than dropped. They should not exist (the caller scanned the root), and an
/// entry Scryer cannot place is exactly what "unexplained" means; classifying it
/// `Unknown` fails closed, because `Unknown` only ever *prevents* a removal.
///
/// [`Unknown`]: RootContentClass::Unknown
pub fn classify_root_content(
    source_root: &Path,
    entries: &[RootEntry],
    title_folders: &[PathBuf],
    tracked_media_paths: &[PathBuf],
) -> RootContentInventory {
    let tracked: BTreeSet<&Path> = tracked_media_paths.iter().map(PathBuf::as_path).collect();
    let mut inventory = RootContentInventory::default();
    let mut unknown_paths: Vec<PathBuf> = Vec::new();
    let mut loose_directories: Vec<PathBuf> = Vec::new();

    for entry in entries {
        let inside_title_folder = title_folders
            .iter()
            .any(|folder| entry.path.starts_with(folder));

        if entry.is_directory() {
            // A directory inside a title folder travels with the title; only
            // directories the title move leaves behind are cleanup's business.
            if !inside_title_folder && !title_folders.iter().any(|folder| folder == &entry.path) {
                loose_directories.push(entry.path.clone());
            }
            continue;
        }

        let class = if tracked.contains(entry.path.as_path()) {
            RootContentClass::Managed
        } else if inside_title_folder && entry.path.starts_with(source_root) {
            RootContentClass::Companion
        } else {
            RootContentClass::Unknown
        };

        let classified = ClassifiedRootEntry {
            path: path_to_stored_string(&entry.path),
            size_bytes: entry.size_bytes(),
            class,
            canonical_sidecar: entry
                .path
                .file_name()
                .map(|name| is_canonical_sidecar_name(&name.to_string_lossy()))
                .unwrap_or(false),
        };

        match class {
            RootContentClass::Managed => inventory.managed.push(classified),
            RootContentClass::Companion => inventory.companions.push(classified),
            RootContentClass::Unknown => {
                unknown_paths.push(entry.path.clone());
                inventory.unknown.push(classified);
            }
        }
    }

    // Deepest first, so cleanup can walk the list and find each directory empty
    // by the time it reaches it — the same ordering `RootMoveTitleExecution`'s
    // `prune_directories` promises.
    loose_directories.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| left.cmp(right))
    });

    for directory in loose_directories {
        let holds_unknown = unknown_paths
            .iter()
            .any(|unknown| unknown.starts_with(&directory));
        let stored = path_to_stored_string(&directory);
        if holds_unknown {
            inventory.retained_directories.push(stored);
        } else {
            inventory.prunable_directories.push(stored);
        }
    }

    inventory
}

// ── Every-title accounting (T061, FR-021–FR-023) ─────────────────────────────

/// What the root change does with one assigned title.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RootChangeTitleOutcome {
    /// The title owns a folder under the source root; the folder relocates.
    Relocates,
    /// The title owns no folder, so nothing moves and only its stored root path
    /// changes (FR-076).
    CatalogOnly,
    /// The title cannot enter the operation yet, and cannot be dropped from it
    /// either (FR-023, FR-086).
    Blocked,
}

impl RootChangeTitleOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Relocates => "relocates",
            Self::CatalogOnly => "catalog_only",
            Self::Blocked => "blocked",
        }
    }

    /// The class this outcome is recorded as in the shared classification, so
    /// the counts the preview shows are the ones every other workflow shows.
    pub fn class(&self) -> TitleLocationClass {
        match self {
            Self::Relocates => TitleLocationClass::RootMove,
            Self::CatalogOnly => TitleLocationClass::CatalogOnly,
            Self::Blocked => TitleLocationClass::NeedsResolution,
        }
    }
}

/// Everything the planner needs about one title assigned to the changing root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootChangeTitleDraft {
    pub title_id: String,
    pub title_name: String,
    /// The folder the title owns today. `None` for a title with no folder match
    /// — it is still accounted for (FR-023), it simply has nothing to move.
    pub source_folder_path: Option<PathBuf>,
    /// Files beneath (or tracked by) the title, as the shared planner currency.
    ///
    /// [`SourceFile::relative_path`] is deliberately unread here: a root change
    /// preserves the layout relative to the **root**, not to the title folder
    /// (FR-026), so every destination is derived from the source path itself.
    /// [`SourceFile::full_blake3`] is unread too — dedup needs a destination
    /// that already holds content, which a change to an unconfigured path never
    /// has (that is US5's consolidation planner).
    pub files: Vec<SourceFile>,
    /// Directories beneath the title's folder, which cleanup may remove once
    /// empty (FR-028). Deepest first.
    pub source_directories: Vec<PathBuf>,
    pub hardlinks: Vec<HardlinkFact>,
    /// Why the title is blocked: an active download or import (FR-086), an
    /// unresolved repair, or another operation already owning it (FR-084).
    /// `Some` is what makes the title [`RootChangeTitleOutcome::Blocked`].
    pub blocked_reason: Option<String>,
    pub blocked_reason_code: Option<String>,
}

impl RootChangeTitleDraft {
    /// FR-023: every assigned title lands in exactly one outcome, and none of
    /// them is "excluded".
    pub fn outcome(&self) -> RootChangeTitleOutcome {
        if self.blocked_reason.is_some() {
            RootChangeTitleOutcome::Blocked
        } else if self.source_folder_path.is_none() {
            RootChangeTitleOutcome::CatalogOnly
        } else {
            RootChangeTitleOutcome::Relocates
        }
    }

    /// Bytes this title contributes to the operation, for the caller's
    /// free-space estimate (FR-080).
    pub fn bytes_total(&self) -> u64 {
        self.files
            .iter()
            .fold(0_u64, |total, file| total.saturating_add(file.size_bytes))
    }
}

/// One title the user has to repair before the root change can run (FR-023).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockedTitle {
    pub title_id: String,
    pub title_name: String,
    pub reason: String,
    pub reason_code: Option<String>,
}

/// The every-title ledger FR-023 demands: assigned titles in, the same number
/// out, none excluded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TitleAccounting {
    pub assigned_total: i64,
    pub relocating: i64,
    pub catalog_only: i64,
    pub blocked: i64,
    pub blocked_titles: Vec<BlockedTitle>,
}

impl TitleAccounting {
    /// The FR-023 invariant. False can only mean this module lost a title, which
    /// is why it is asserted rather than assumed.
    pub fn accounts_for_every_title(&self) -> bool {
        self.assigned_total == self.relocating + self.catalog_only + self.blocked
    }

    /// FR-023 + FR-086: a blocked title can neither move nor be excluded, so the
    /// operation cannot start while one exists.
    pub fn blocks_start(&self) -> bool {
        self.blocked > 0
    }

    fn counts(&self) -> ClassificationCounts {
        ClassificationCounts {
            root_move: self.relocating,
            catalog_only: self.catalog_only,
            needs_resolution: self.blocked,
            ..ClassificationCounts::default()
        }
    }
}

// ── Identity retention (T061, FR-021/FR-078) ─────────────────────────────────

/// What the root is, before the change.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RootRetentionFacts {
    /// The root is its library's default (FR-021, US4 scenario 2).
    pub is_library_default: bool,
    /// The root's role, when the library assigns one.
    pub role: Option<String>,
}

/// The facts the executor asserts after the path flip (FR-021, FR-078).
///
/// Stated positively and stored on the plan rather than left implicit: before
/// synthetic root ids (D1/FR-078) a path change *was* an identity change, and
/// the whole point of T010/T013 landing first is that it no longer is. These are
/// the post-conditions a root-change execution has to be able to prove.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootIdentityRetention {
    pub root_id: String,
    /// Always true: the synthetic id is path-independent (FR-078).
    pub keeps_root_id: bool,
    pub was_library_default: bool,
    /// Equal to `was_library_default`: a path change never moves the default
    /// (FR-021). Consolidation is the workflow that *can* move it (FR-022), and
    /// it is a different planner.
    pub remains_library_default: bool,
    pub retained_role: Option<String>,
    /// Every assigned title keeps pointing at this root id — including the
    /// blocked ones, which is why FR-023 can forbid exclusions at all.
    pub retained_title_assignments: i64,
}

impl RootIdentityRetention {
    fn statement(&self, source_path: &str, destination_path: &str) -> String {
        let mut statement = format!(
            "the root keeps its identity and its {} title assignment(s); only its path changes from {source_path} to {destination_path}",
            self.retained_title_assignments
        );
        if self.remains_library_default {
            statement.push_str("; it remains the library default");
        }
        if let Some(role) = self.retained_role.as_deref() {
            statement.push_str(&format!("; it keeps its \"{role}\" role"));
        }
        statement
    }
}

// ── Retirement ordering (T063, FR-087/FR-028) ────────────────────────────────

/// One reason the source location cannot be retired yet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootRetirementBlocker {
    pub code: String,
    pub detail: String,
}

/// The ordering and permission contract the executor and the recycle bin read
/// (FR-087, FR-028, FR-031).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootRetirementContract {
    pub source_root_path: String,
    pub destination_root_path: String,
    /// FR-087: always true. The configured path is flipped only after every
    /// recycle this operation performs has completed, because the recycle
    /// allowlist is derived from the configured root path and an early flip
    /// would make the bin reject the remaining source files.
    pub retire_configuration_after_recycling: bool,
    /// FR-087: the paths recycling must keep accepting for this operation's
    /// whole life, resume included. A resumed run reads these off the persisted
    /// plan, so an in-retirement root stays allowlisted even though the live
    /// configuration no longer names it.
    pub recycle_allowlist_paths: Vec<String>,
    /// FR-028/FR-031: nothing at the source is removed before its destination
    /// copy verified.
    pub requires_verification_before_source_removal: bool,
    /// FR-028: automatic cleanup removes empty directories and nothing else.
    pub empty_directories_only: bool,
    /// Directories automatic cleanup may remove once empty, deepest first.
    pub removable_directories: Vec<String>,
    /// Directories left standing because they hold unexplained content.
    pub retained_directories: Vec<String>,
    /// Empty when the source location may be removed (FR-023, FR-028).
    pub blockers: Vec<RootRetirementBlocker>,
}

impl RootRetirementContract {
    /// Whether cleanup may take the source location away (FR-028).
    pub fn permits_source_removal(&self) -> bool {
        self.blockers.is_empty()
    }

    pub fn blocker(&self, code: &str) -> Option<&RootRetirementBlocker> {
        self.blockers.iter().find(|blocker| blocker.code == code)
    }
}

// ── Planner ──────────────────────────────────────────────────────────────────

/// The whole planning request. Every fact is supplied; nothing is read here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootChangePlanRequest {
    pub library_id: String,
    /// The root's synthetic id, unchanged by this operation (FR-078).
    pub root_id: String,
    pub source_root_path: PathBuf,
    /// The new, unconfigured path (FR-020).
    pub destination_root_path: PathBuf,
    /// **Move with Scryer** or **Files are already there** (US4). A request with
    /// nothing to move is downgraded to
    /// [`LocationExecutionMode::CatalogOnly`] (FR-076).
    pub mode: LocationExecutionMode,
    pub retention: RootRetentionFacts,
    /// Every title assigned to the root. Not a selection: FR-023 forbids one.
    pub titles: Vec<RootChangeTitleDraft>,
    /// The caller's scan of the source root.
    pub entries: Vec<RootEntry>,
    pub verification_depth: VerificationDepth,
    pub free_space: FreeSpaceEstimate,
    /// `Some(true)` when the two paths share a volume (rename fast path,
    /// FR-032), `None` when the relationship could not be probed.
    pub same_volume: Option<bool>,
}

/// The planner result: the plan the user confirms, the instructions the runner
/// executes, and the root-scoped facts neither of those two can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRootChange {
    pub plan: LocationPlan,
    pub execution: RootMoveExecutionPlan,
    pub accounting: TitleAccounting,
    pub retention: RootIdentityRetention,
    pub content: RootContentInventory,
    pub retirement: RootRetirementContract,
    pub warnings: Vec<String>,
}

impl PlannedRootChange {
    /// The runner's view of this plan, through the shared work-plan seam.
    pub fn work_plan(&self) -> crate::location::executor::OperationWorkPlan {
        self.execution.to_work_plan()
    }
}

/// Build the root-change preview and execution plan (T060–T063).
pub fn build_root_change_plan(request: &RootChangePlanRequest) -> PlannedRootChange {
    let accounting = build_accounting(&request.titles);
    let title_folders: Vec<PathBuf> = request
        .titles
        .iter()
        .filter_map(|title| title.source_folder_path.clone())
        .collect();
    let tracked_media_paths: Vec<PathBuf> = request
        .titles
        .iter()
        .flat_map(|title| title.files.iter())
        .filter(|file| file.media_file_id.is_some())
        .map(|file| file.path.clone())
        .collect();
    let content = classify_root_content(
        &request.source_root_path,
        &request.entries,
        &title_folders,
        &tracked_media_paths,
    );

    let retention = RootIdentityRetention {
        root_id: request.root_id.clone(),
        keeps_root_id: true,
        was_library_default: request.retention.is_library_default,
        remains_library_default: request.retention.is_library_default,
        retained_role: request.retention.role.clone(),
        retained_title_assignments: accounting.assigned_total,
    };

    let source_root_display = path_to_stored_string(&request.source_root_path);
    let destination_root_display = path_to_stored_string(&request.destination_root_path);
    let retirement = build_retirement_contract(
        &source_root_display,
        &destination_root_display,
        &accounting,
        &content,
    );

    let header = LocationPlanHeader::new(
        LocationOperationType::RootChange,
        execution_mode_for(request, &accounting),
    )
    .with_source(
        Some(request.library_id.clone()),
        Some(request.root_id.clone()),
    )
    // FR-021/FR-078: one root id on both sides. The path moves; the identity
    // does not.
    .with_destination(
        Some(request.library_id.clone()),
        Some(request.root_id.clone()),
    )
    // FR-023: the "selection" is every assigned title, blocked ones included, so
    // a title arriving on or leaving the root between preview and start voids
    // the confirmation (FR-081).
    .with_selection(
        request
            .titles
            .iter()
            .map(|title| title.title_id.clone())
            .collect::<Vec<_>>(),
    );

    let mut builder = LocationPlanBuilder::new(header);
    builder.classification(accounting.counts());
    builder.free_space(request.free_space.clone());
    builder.verification_depth(request.verification_depth);

    let mut warnings: Vec<String> = Vec::new();
    let mut execution = RootMoveExecutionPlan {
        no_op_titles: 0,
        unresolved_titles: accounting.blocked,
        ..RootMoveExecutionPlan::default()
    };

    // The root-level statement first: what the operation does to the root
    // itself, before anything it does to a title (FR-021).
    builder.push(
        PlanItem::new(PlanItemKind::CatalogChange)
            .with_paths(
                Some(source_root_display.clone()),
                Some(destination_root_display.clone()),
            )
            .with_reason_code(plan_reasons::ROOT_IDENTITY_RETAINED)
            .with_detail(retention.statement(&source_root_display, &destination_root_display)),
    );

    for (index, draft) in request.titles.iter().enumerate() {
        let (title_execution, items, title_warnings) = plan_title(request, draft, index as i64);
        builder.extend(items);
        warnings.extend(title_warnings);
        if let Some(title_execution) = title_execution {
            execution.titles.push(title_execution);
        }
    }

    // FR-027: unexplained content is listed, item by item, so it is neither
    // silently deleted nor silently abandoned — and so that new junk appearing
    // at the source between preview and start changes the fingerprint.
    for entry in &content.unknown {
        builder.push(
            PlanItem::new(PlanItemKind::UnmanagedContent)
                .with_paths(Some(entry.path.clone()), Option::<String>::None)
                .with_size(entry.size_bytes)
                .with_reason_code(plan_reasons::UNKNOWN_ROOT_CONTENT)
                .with_detail(format!(
                    "{} is not tracked by any title on this root; it stays where it is",
                    entry.path
                )),
        );
    }

    // FR-028/FR-023: say why the source location survives the operation.
    for blocker in &retirement.blockers {
        builder.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_paths(Some(source_root_display.clone()), Option::<String>::None)
                .with_reason_code(plan_reasons::SOURCE_RETIREMENT_BLOCKED)
                .with_detail(blocker.detail.clone()),
        );
        warnings.push(blocker.detail.clone());
    }

    PlannedRootChange {
        plan: builder.build(),
        execution,
        accounting,
        retention,
        content,
        retirement,
        warnings,
    }
}

fn build_accounting(titles: &[RootChangeTitleDraft]) -> TitleAccounting {
    let mut accounting = TitleAccounting {
        assigned_total: titles.len() as i64,
        ..TitleAccounting::default()
    };
    for draft in titles {
        match draft.outcome() {
            RootChangeTitleOutcome::Relocates => accounting.relocating += 1,
            RootChangeTitleOutcome::CatalogOnly => accounting.catalog_only += 1,
            RootChangeTitleOutcome::Blocked => {
                accounting.blocked += 1;
                accounting.blocked_titles.push(BlockedTitle {
                    title_id: draft.title_id.clone(),
                    title_name: draft.title_name.clone(),
                    reason: draft.blocked_reason.clone().unwrap_or_else(|| {
                        format!("\"{}\" needs a repair before it can move", draft.title_name)
                    }),
                    reason_code: draft.blocked_reason_code.clone(),
                });
            }
        }
    }
    accounting
}

fn build_retirement_contract(
    source_root_display: &str,
    destination_root_display: &str,
    accounting: &TitleAccounting,
    content: &RootContentInventory,
) -> RootRetirementContract {
    let mut blockers = Vec::new();
    if accounting.blocks_start() {
        blockers.push(RootRetirementBlocker {
            code: retirement_blockers::BLOCKED_TITLES.to_string(),
            detail: format!(
                "{} title(s) on this root must be repaired before the source root can be retired; they cannot be excluded from a root change",
                accounting.blocked
            ),
        });
    }
    if content.blocks_source_removal() {
        blockers.push(RootRetirementBlocker {
            code: retirement_blockers::UNEXPLAINED_SOURCE_CONTENT.to_string(),
            detail: format!(
                "{} item(s) at {source_root_display} are not explained by the catalog; the source location is kept until they are resolved",
                content.unknown.len()
            ),
        });
    }

    RootRetirementContract {
        source_root_path: source_root_display.to_string(),
        destination_root_path: destination_root_display.to_string(),
        retire_configuration_after_recycling: true,
        recycle_allowlist_paths: vec![source_root_display.to_string()],
        requires_verification_before_source_removal: true,
        empty_directories_only: true,
        removable_directories: content.prunable_directories.clone(),
        retained_directories: content.retained_directories.clone(),
        blockers,
    }
}

/// A root change with nothing to move needs no move mode; FR-076 asks the UI to
/// skip the chooser in exactly that case.
fn execution_mode_for(
    request: &RootChangePlanRequest,
    accounting: &TitleAccounting,
) -> LocationExecutionMode {
    if accounting.relocating > 0 {
        request.mode
    } else {
        LocationExecutionMode::CatalogOnly
    }
}

fn plan_title(
    request: &RootChangePlanRequest,
    draft: &RootChangeTitleDraft,
    sequence: i64,
) -> (Option<RootMoveTitleExecution>, Vec<PlanItem>, Vec<String>) {
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    let outcome = draft.outcome();

    if outcome == RootChangeTitleOutcome::Blocked {
        // FR-023: represented, never dropped. The user repairs it; there is no
        // "exclude" affordance to offer.
        items.push(
            PlanItem::new(PlanItemKind::Blocked)
                .with_title(draft.title_id.clone())
                .with_paths(
                    draft.source_folder_path.as_deref().map(path_to_stored_string),
                    Option::<String>::None,
                )
                .with_reason_code(
                    draft
                        .blocked_reason_code
                        .clone()
                        .unwrap_or_else(|| plan_reasons::TITLE_BLOCKED_FOR_ROOT_CHANGE.to_string()),
                )
                .with_detail(draft.blocked_reason.clone().unwrap_or_else(|| {
                    format!("\"{}\" needs a repair before it can move", draft.title_name)
                })),
        );
        return (None, items, warnings);
    }

    let source_root_display = path_to_stored_string(&request.source_root_path);
    let destination_root_display = path_to_stored_string(&request.destination_root_path);

    if outcome == RootChangeTitleOutcome::CatalogOnly {
        items.push(
            PlanItem::new(PlanItemKind::CatalogChange)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::CATALOG_ONLY_ROOT_CHANGE)
                .with_detail(format!(
                    "\"{}\" owns no folder, so only its stored root path changes",
                    draft.title_name
                )),
        );
        return (
            Some(RootMoveTitleExecution {
                title_id: draft.title_id.clone(),
                title_name: draft.title_name.clone(),
                sequence,
                class: outcome.class(),
                source_library_id: request.library_id.clone(),
                source_root_id: request.root_id.clone(),
                source_folder_path: None,
                destination_library_id: request.library_id.clone(),
                // FR-021: same id, new path.
                destination_root_id: request.root_id.clone(),
                destination_folder_path: None,
                destination_root_path: Some(destination_root_display),
                source_root_path: Some(source_root_display),
                same_volume: request.same_volume,
                files: Vec::new(),
                deduplicated_sources: Vec::new(),
                deduplicated_media_file_ids: Vec::new(),
                renamed_destinations: Vec::new(),
                prune_directories: Vec::new(),
                warnings: Vec::new(),
                converted_facet: None,
                dropped_tag_prefixes: Vec::new(),
                merge_target_title_id: None,
            }),
            items,
            warnings,
        );
    }

    let source_folder = draft
        .source_folder_path
        .clone()
        .expect("a relocating title owns a folder");
    let destination_folder = rebase(
        &source_folder,
        &request.source_root_path,
        &request.destination_root_path,
    )
    .unwrap_or_else(|| {
        // A folder outside the root cannot be rebased. The classifier already
        // treats such content as unexplained; here the folder still has to land
        // somewhere, and the root's own basename is the only defensible place.
        request.destination_root_path.join(
            source_folder
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&draft.title_id)),
        )
    });
    let destination_folder_display = path_to_stored_string(&destination_folder);

    let mut files = Vec::new();
    for file in &draft.files {
        // FR-026: layout is preserved relative to the *root*, so a file keeps
        // its position however deeply it is nested — and a title folder need not
        // be a direct child of the root.
        let destination_path = match rebase(
            &file.path,
            &request.source_root_path,
            &request.destination_root_path,
        ) {
            Some(path) => path,
            None => {
                warnings.push(format!(
                    "\"{}\" tracks {} outside the root being changed; it moves into the title's destination folder",
                    draft.title_name,
                    file.path.display()
                ));
                destination_folder.join(
                    file.path
                        .file_name()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from(&draft.title_id)),
                )
            }
        };

        let source_display = path_to_stored_string(&file.path);
        let destination_display = path_to_stored_string(&destination_path);

        if !file.path.starts_with(&source_folder) {
            items.push(
                PlanItem::new(PlanItemKind::Warning)
                    .with_title(draft.title_id.clone())
                    .with_paths(Some(source_display.clone()), Some(destination_display.clone()))
                    .with_reason_code(plan_reasons::FILE_OUTSIDE_TITLE_FOLDER)
                    .with_detail(format!(
                        "\"{}\" tracks {source_display} outside its own folder",
                        draft.title_name
                    )),
            );
        }

        let mut item = PlanItem::new(PlanItemKind::Move)
            .with_title(draft.title_id.clone())
            .with_paths(Some(source_display.clone()), Some(destination_display.clone()))
            .with_size(file.size_bytes);
        item.media_file_id = file.media_file_id.clone();
        if let Some(same_volume) = request.same_volume {
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

    // FR-085. A same-volume root change renames rather than copies, so nothing
    // is recycled and a hardlink survives; anything else recycles the source.
    let recycles_source = request.same_volume != Some(true);
    for warning in hardlink_warnings(&draft.hardlinks, request.same_volume, recycles_source) {
        items.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::HARDLINKED_SOURCE)
                .with_detail(warning.message()),
        );
        warnings.push(warning.message());
    }

    let mut prune_directories: Vec<String> = draft
        .source_directories
        .iter()
        .map(path_to_stored_string)
        .collect();
    // The title's own folder is the last thing cleanup may remove, and only when
    // it is empty (FR-028).
    let source_folder_display = path_to_stored_string(&source_folder);
    if !prune_directories.contains(&source_folder_display) {
        prune_directories.push(source_folder_display.clone());
    }

    let execution = RootMoveTitleExecution {
        title_id: draft.title_id.clone(),
        title_name: draft.title_name.clone(),
        sequence,
        class: outcome.class(),
        source_library_id: request.library_id.clone(),
        source_root_id: request.root_id.clone(),
        source_folder_path: Some(source_folder_display),
        destination_library_id: request.library_id.clone(),
        // FR-021/FR-078 in the shared plan currency: the destination root *is*
        // the source root. Only the two paths differ.
        destination_root_id: request.root_id.clone(),
        destination_folder_path: Some(destination_folder_display),
        destination_root_path: Some(destination_root_display),
        source_root_path: Some(source_root_display),
        same_volume: request.same_volume,
        files,
        // A change to an unconfigured path has no destination content, so there
        // is nothing to dedup against and nothing to rename around (FR-072–075
        // belong to US5's consolidation planner).
        deduplicated_sources: Vec::new(),
        deduplicated_media_file_ids: Vec::new(),
        renamed_destinations: Vec::new(),
        prune_directories,
        warnings: warnings.clone(),
        converted_facet: None,
        dropped_tag_prefixes: Vec::new(),
        merge_target_title_id: None,
    };

    (Some(execution), items, warnings)
}

/// Re-anchor `path` from `source_root` onto `destination_root`, preserving its
/// relative position (FR-026). `None` when `path` is not under `source_root`.
fn rebase(path: &Path, source_root: &Path, destination_root: &Path) -> Option<PathBuf> {
    path.strip_prefix(source_root)
        .ok()
        .map(|relative| destination_root.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::location::collisions::FullHash;
    use crate::location::hardlinks::LinkCount;
    use crate::location::preview::{
        LOCATION_TYPED_CONFIRMATION_PHRASE, PlanConfirmationError, PlanConfirmationRequest,
    };

    const SOURCE_ROOT: &str = "/media/old";
    const DESTINATION_ROOT: &str = "/media/new";

    fn tracked(path: &str, size_bytes: u64) -> SourceFile {
        SourceFile {
            media_file_id: Some(format!("file-{path}")),
            full_blake3: FullHash::Absent,
            path: PathBuf::from(path),
            relative_path: None,
            size_bytes,
        }
    }

    fn companion(path: &str, size_bytes: u64) -> SourceFile {
        SourceFile {
            media_file_id: None,
            full_blake3: FullHash::Absent,
            path: PathBuf::from(path),
            relative_path: None,
            size_bytes,
        }
    }

    fn title(id: &str, folder: &str, files: Vec<SourceFile>) -> RootChangeTitleDraft {
        RootChangeTitleDraft {
            title_id: id.to_string(),
            title_name: id.to_string(),
            source_folder_path: Some(PathBuf::from(folder)),
            files,
            source_directories: Vec::new(),
            hardlinks: Vec::new(),
            blocked_reason: None,
            blocked_reason_code: None,
        }
    }

    fn fileless(id: &str) -> RootChangeTitleDraft {
        RootChangeTitleDraft {
            title_id: id.to_string(),
            title_name: id.to_string(),
            source_folder_path: None,
            files: Vec::new(),
            source_directories: Vec::new(),
            hardlinks: Vec::new(),
            blocked_reason: None,
            blocked_reason_code: None,
        }
    }

    fn blocked(id: &str, folder: &str, reason: &str) -> RootChangeTitleDraft {
        RootChangeTitleDraft {
            blocked_reason: Some(reason.to_string()),
            blocked_reason_code: Some("active_download_or_import".to_string()),
            ..title(id, folder, Vec::new())
        }
    }

    fn request(titles: Vec<RootChangeTitleDraft>) -> RootChangePlanRequest {
        RootChangePlanRequest {
            library_id: "library-1".to_string(),
            root_id: "root-1".to_string(),
            source_root_path: PathBuf::from(SOURCE_ROOT),
            destination_root_path: PathBuf::from(DESTINATION_ROOT),
            mode: LocationExecutionMode::MoveWithScryer,
            retention: RootRetentionFacts::default(),
            titles,
            entries: Vec::new(),
            verification_depth: VerificationDepth::default(),
            free_space: FreeSpaceEstimate::unknown(),
            same_volume: Some(false),
        }
    }

    fn confirm(planned: &PlannedRootChange, phrase: Option<&str>) -> Result<(), PlanConfirmationError> {
        planned.plan.confirm(&PlanConfirmationRequest {
            fingerprint: planned.plan.fingerprint.clone(),
            typed_confirmation: phrase.map(str::to_string),
        })
    }

    // ── US4 scenario 1: every title accounted for, none excluded ─────────────

    #[test]
    fn every_assigned_title_is_accounted_for_with_no_exclusions() {
        let planned = build_root_change_plan(&request(vec![
            title(
                "movie-a",
                "/media/old/Movie A",
                vec![tracked("/media/old/Movie A/a.mkv", 10)],
            ),
            title(
                "movie-b",
                "/media/old/Movie B",
                vec![tracked("/media/old/Movie B/b.mkv", 20)],
            ),
            fileless("movie-c"),
            blocked("movie-d", "/media/old/Movie D", "an import is in progress"),
        ]));

        assert_eq!(planned.accounting.assigned_total, 4);
        assert_eq!(planned.accounting.relocating, 2);
        assert_eq!(planned.accounting.catalog_only, 1);
        assert_eq!(planned.accounting.blocked, 1);
        assert!(planned.accounting.accounts_for_every_title());
        // The plan's own classification counts agree, so the preview cannot show
        // a smaller population than the root holds (SC-005).
        assert_eq!(planned.plan.classification.total(), 4);
        // Every title is in the fingerprinted selection, blocked one included.
        assert_eq!(planned.plan.header.selection.len(), 4);
    }

    #[test]
    fn a_blocked_title_blocks_the_start_and_the_source_retirement() {
        let planned = build_root_change_plan(&request(vec![
            title(
                "movie-a",
                "/media/old/Movie A",
                vec![tracked("/media/old/Movie A/a.mkv", 10)],
            ),
            blocked("movie-d", "/media/old/Movie D", "an import is in progress"),
        ]));

        // FR-023 + FR-086: it cannot move and it cannot be excluded, so nothing
        // starts until the user repairs it.
        assert!(planned.plan.blocks_start());
        assert_eq!(
            confirm(&planned, Some(LOCATION_TYPED_CONFIRMATION_PHRASE)),
            Err(PlanConfirmationError::Blocked)
        );
        // FR-023: "Blocked titles MUST be repaired before the source root is
        // retired."
        assert!(!planned.retirement.permits_source_removal());
        assert!(
            planned
                .retirement
                .blocker(retirement_blockers::BLOCKED_TITLES)
                .is_some()
        );
        // And the blocked title is named, not merely counted.
        assert_eq!(planned.accounting.blocked_titles.len(), 1);
        assert_eq!(
            planned.accounting.blocked_titles[0].reason,
            "an import is in progress"
        );
        // It produces no instructions, but the runner still knows it existed.
        assert_eq!(planned.execution.unresolved_titles, 1);
        assert_eq!(planned.execution.titles.len(), 1);
    }

    // ── US4 scenario 2: identity, role, and default retention ────────────────

    #[test]
    fn the_root_keeps_its_id_default_status_role_and_assignments() {
        let mut plan_request = request(vec![
            title(
                "movie-a",
                "/media/old/Movie A",
                vec![tracked("/media/old/Movie A/a.mkv", 10)],
            ),
            fileless("movie-c"),
        ]);
        plan_request.retention = RootRetentionFacts {
            is_library_default: true,
            role: Some("primary".to_string()),
        };

        let planned = build_root_change_plan(&plan_request);

        assert!(planned.retention.keeps_root_id);
        assert_eq!(planned.retention.root_id, "root-1");
        assert!(planned.retention.remains_library_default);
        assert_eq!(planned.retention.retained_role.as_deref(), Some("primary"));
        assert_eq!(planned.retention.retained_title_assignments, 2);
    }

    #[test]
    fn the_execution_plan_carries_one_root_id_on_both_sides() {
        let planned = build_root_change_plan(&request(vec![
            title(
                "movie-a",
                "/media/old/Movie A",
                vec![tracked("/media/old/Movie A/a.mkv", 10)],
            ),
            fileless("movie-c"),
        ]));

        // FR-078: identity is path-independent, so the lowering onto the shared
        // plan currency is "one root, two paths".
        for title in &planned.execution.titles {
            assert_eq!(title.source_root_id, title.destination_root_id);
            assert_eq!(title.source_library_id, title.destination_library_id);
            assert!(!title.crosses_libraries());
            assert_eq!(title.source_root_path.as_deref(), Some(SOURCE_ROOT));
            assert_eq!(title.destination_root_path.as_deref(), Some(DESTINATION_ROOT));
        }
        assert_eq!(
            planned.plan.header.source_root_id,
            planned.plan.header.destination_root_id
        );
    }

    // ── US4 scenario 3: unmanaged content ────────────────────────────────────

    #[test]
    fn unmanaged_classification_separates_managed_companion_and_unknown() {
        let inventory = classify_root_content(
            Path::new(SOURCE_ROOT),
            &[
                RootEntry::file("/media/old/Movie A/a.mkv", 10),
                RootEntry::file("/media/old/Movie A/a.nfo", 1),
                RootEntry::file("/media/old/Movie A/fanart.jpg", 2),
                RootEntry::file("/media/old/notes.txt", 3),
                RootEntry::file("/media/old/junk/archive.rar", 4),
                RootEntry::directory("/media/old/Movie A"),
                RootEntry::directory("/media/old/junk"),
            ],
            &[PathBuf::from("/media/old/Movie A")],
            &[PathBuf::from("/media/old/Movie A/a.mkv")],
        );

        assert_eq!(inventory.managed.len(), 1);
        assert_eq!(inventory.companions.len(), 2);
        assert_eq!(inventory.unknown.len(), 2);
        assert_eq!(inventory.unknown_bytes(), 7);
        assert!(inventory.blocks_source_removal());
        // Every scanned file landed in exactly one bucket (FR-027).
        assert_eq!(inventory.entry_count(), 5);
    }

    #[test]
    fn a_canonical_sidecar_is_a_companion_and_is_flagged_as_one() {
        let inventory = classify_root_content(
            Path::new(SOURCE_ROOT),
            &[RootEntry::file("/media/old/Movie A/movie.nfo", 1)],
            &[PathBuf::from("/media/old/Movie A")],
            &[],
        );

        assert_eq!(inventory.companions.len(), 1);
        assert_eq!(inventory.companions[0].class, RootContentClass::Companion);
        assert!(inventory.companions[0].canonical_sidecar);
    }

    #[test]
    fn an_entry_outside_the_root_fails_closed_as_unknown() {
        let inventory = classify_root_content(
            Path::new(SOURCE_ROOT),
            &[RootEntry::file("/elsewhere/stray.mkv", 5)],
            &[],
            &[],
        );

        assert_eq!(inventory.unknown.len(), 1);
        assert!(inventory.blocks_source_removal());
    }

    #[test]
    fn unknown_content_is_listed_separately_and_blocks_source_removal() {
        let mut plan_request = request(vec![title(
            "movie-a",
            "/media/old/Movie A",
            vec![tracked("/media/old/Movie A/a.mkv", 10)],
        )]);
        plan_request.entries = vec![
            RootEntry::file("/media/old/Movie A/a.mkv", 10),
            RootEntry::file("/media/old/tax-return.pdf", 3),
            RootEntry::directory("/media/old/Movie A"),
        ];

        let planned = build_root_change_plan(&plan_request);

        // FR-027: listed separately, on its own plan section.
        let section = planned
            .plan
            .section(PlanItemKind::UnmanagedContent)
            .expect("unmanaged section");
        assert_eq!(section.items.total, 1);
        assert_eq!(
            section.items.items[0].source_path.as_deref(),
            Some("/media/old/tax-return.pdf")
        );
        // FR-028: it stops the source location from being removed…
        assert!(!planned.retirement.permits_source_removal());
        assert!(
            planned
                .retirement
                .blocker(retirement_blockers::UNEXPLAINED_SOURCE_CONTENT)
                .is_some()
        );
        // …and the preview says so out loud (C3).
        assert!(
            planned
                .warnings
                .iter()
                .any(|warning| warning.contains("not explained by the catalog"))
        );
        // …but it does not stop the titles from moving (US4 scenario 3 blocks
        // removal, not the operation).
        assert!(!planned.plan.blocks_start());
        assert_eq!(planned.execution.titles.len(), 1);
    }

    #[test]
    fn new_unknown_content_at_the_source_voids_the_confirmation() {
        let mut plan_request = request(vec![title(
            "movie-a",
            "/media/old/Movie A",
            vec![tracked("/media/old/Movie A/a.mkv", 10)],
        )]);
        plan_request.entries = vec![RootEntry::file("/media/old/Movie A/a.mkv", 10)];
        let before = build_root_change_plan(&plan_request);

        plan_request
            .entries
            .push(RootEntry::file("/media/old/appeared.iso", 9));
        let after = build_root_change_plan(&plan_request);

        assert_ne!(before.plan.fingerprint, after.plan.fingerprint);
    }

    // ── US4 scenario 4: empty-directory-only cleanup ─────────────────────────

    #[test]
    fn cleanup_facts_are_empty_directories_only_and_gated_on_verification() {
        let mut plan_request = request(vec![RootChangeTitleDraft {
            source_directories: vec![PathBuf::from("/media/old/Movie A/Extras")],
            ..title(
                "movie-a",
                "/media/old/Movie A",
                vec![tracked("/media/old/Movie A/Extras/a.mkv", 10)],
            )
        }]);
        plan_request.entries = vec![
            RootEntry::file("/media/old/Movie A/Extras/a.mkv", 10),
            RootEntry::directory("/media/old/Movie A"),
            RootEntry::directory("/media/old/Movie A/Extras"),
            RootEntry::directory("/media/old/empty-shelf"),
        ];

        let planned = build_root_change_plan(&plan_request);

        assert!(planned.retirement.empty_directories_only);
        assert!(planned.retirement.requires_verification_before_source_removal);
        // A directory outside every title folder that holds nothing unexplained
        // is prunable; nothing else is proposed for removal.
        assert_eq!(
            planned.retirement.removable_directories,
            vec!["/media/old/empty-shelf".to_string()]
        );
        assert!(planned.retirement.retained_directories.is_empty());
        // The title's own directories are cleanup's business through the shared
        // per-title instruction set, deepest first.
        let title = &planned.execution.titles[0];
        assert_eq!(
            title.prune_directories,
            vec![
                "/media/old/Movie A/Extras".to_string(),
                "/media/old/Movie A".to_string(),
            ]
        );
        // Nothing at the source is removed while unexplained content remains —
        // and here there is none, so removal is permitted.
        assert!(planned.retirement.permits_source_removal());
    }

    #[test]
    fn a_directory_holding_unknown_content_is_retained_not_pruned() {
        let inventory = classify_root_content(
            Path::new(SOURCE_ROOT),
            &[
                RootEntry::directory("/media/old/junk"),
                RootEntry::directory("/media/old/spare"),
                RootEntry::file("/media/old/junk/archive.rar", 4),
            ],
            &[],
            &[],
        );

        assert_eq!(
            inventory.retained_directories,
            vec!["/media/old/junk".to_string()]
        );
        assert_eq!(
            inventory.prunable_directories,
            vec!["/media/old/spare".to_string()]
        );
    }

    // ── US4 scenario 5: typed confirmation ───────────────────────────────────

    #[test]
    fn a_root_change_requires_the_shared_typed_confirmation() {
        let planned = build_root_change_plan(&request(vec![title(
            "movie-a",
            "/media/old/Movie A",
            vec![tracked("/media/old/Movie A/a.mkv", 10)],
        )]));

        assert_eq!(
            planned.plan.header.operation_type,
            LocationOperationType::RootChange
        );
        assert!(planned.plan.confirmation.requires_typed_confirmation());
        assert_eq!(
            planned.plan.confirmation.typed_phrase.as_deref(),
            Some(LOCATION_TYPED_CONFIRMATION_PHRASE)
        );
        assert_eq!(
            confirm(&planned, None),
            Err(PlanConfirmationError::TypedConfirmationRequired)
        );
        assert_eq!(
            confirm(&planned, Some("relocate")),
            Err(PlanConfirmationError::TypedConfirmationMismatch)
        );
        assert_eq!(confirm(&planned, Some(LOCATION_TYPED_CONFIRMATION_PHRASE)), Ok(()));
    }

    // ── Retirement ordering (FR-087) ─────────────────────────────────────────

    #[test]
    fn the_configuration_is_retired_only_after_recycling_completes() {
        let planned = build_root_change_plan(&request(vec![title(
            "movie-a",
            "/media/old/Movie A",
            vec![tracked("/media/old/Movie A/a.mkv", 10)],
        )]));

        assert!(planned.retirement.retire_configuration_after_recycling);
        assert_eq!(planned.retirement.source_root_path, SOURCE_ROOT);
        assert_eq!(planned.retirement.destination_root_path, DESTINATION_ROOT);
    }

    #[test]
    fn the_source_root_stays_recycle_allowlisted_for_resume() {
        let planned = build_root_change_plan(&request(vec![title(
            "movie-a",
            "/media/old/Movie A",
            vec![tracked("/media/old/Movie A/a.mkv", 10)],
        )]));

        // FR-087: a resumed run reads the allowlist off the persisted plan, so
        // an in-retirement root is still accepted by the bin.
        assert_eq!(
            planned.retirement.recycle_allowlist_paths,
            vec![SOURCE_ROOT.to_string()]
        );
        // The per-title instruction set carries the same path, which is the key
        // `RecycleBinSourceRecycler` resolves its per-root config by.
        assert_eq!(
            planned.execution.titles[0].source_root_path.as_deref(),
            Some(SOURCE_ROOT)
        );
    }

    // ── Layout preservation (FR-026) ─────────────────────────────────────────

    #[test]
    fn nested_folders_and_files_keep_their_root_relative_layout() {
        let planned = build_root_change_plan(&request(vec![title(
            "series-a",
            "/media/old/Shows/Series A",
            vec![
                tracked("/media/old/Shows/Series A/Season 01/s01e01.mkv", 10),
                companion("/media/old/Shows/Series A/Season 01/s01e01.srt", 1),
            ],
        )]));

        let title = &planned.execution.titles[0];
        assert_eq!(
            title.destination_folder_path.as_deref(),
            Some("/media/new/Shows/Series A")
        );
        assert_eq!(
            title.files[0].destination_path,
            "/media/new/Shows/Series A/Season 01/s01e01.mkv"
        );
        // The companion travels at the same relative position; nothing is
        // renamed, because a root change does not recalculate names (FR-026).
        assert_eq!(
            title.files[1].destination_path,
            "/media/new/Shows/Series A/Season 01/s01e01.srt"
        );
        assert!(planned.plan.section(PlanItemKind::Rename).is_none());
    }

    #[test]
    fn a_tracked_file_outside_its_title_folder_keeps_its_root_relative_position() {
        let planned = build_root_change_plan(&request(vec![title(
            "movie-a",
            "/media/old/Movie A",
            vec![tracked("/media/old/loose/a-extra.mkv", 5)],
        )]));

        let title = &planned.execution.titles[0];
        assert_eq!(title.files[0].destination_path, "/media/new/loose/a-extra.mkv");
        let warning = planned
            .plan
            .section(PlanItemKind::Warning)
            .expect("warning section");
        assert!(
            warning
                .items
                .items
                .iter()
                .any(|item| item.reason_code.as_deref()
                    == Some(plan_reasons::FILE_OUTSIDE_TITLE_FOLDER))
        );
    }

    // ── Mode, catalog-only titles, and warnings ──────────────────────────────

    #[test]
    fn a_root_change_with_nothing_to_move_needs_no_move_mode() {
        let planned = build_root_change_plan(&request(vec![fileless("movie-c")]));

        assert_eq!(planned.plan.header.mode, LocationExecutionMode::CatalogOnly);
        let title = &planned.execution.titles[0];
        assert_eq!(title.class, TitleLocationClass::CatalogOnly);
        assert!(title.files.is_empty());
        assert!(title.destination_folder_path.is_none());
        assert_eq!(title.destination_root_id, "root-1");
    }

    #[test]
    fn the_plan_states_the_root_identity_retention_before_anything_else() {
        let mut plan_request = request(vec![title(
            "movie-a",
            "/media/old/Movie A",
            vec![tracked("/media/old/Movie A/a.mkv", 10)],
        )]);
        plan_request.retention = RootRetentionFacts {
            is_library_default: true,
            role: None,
        };

        let planned = build_root_change_plan(&plan_request);

        let catalog = planned
            .plan
            .section(PlanItemKind::CatalogChange)
            .expect("catalog section");
        let statement = catalog
            .items
            .items
            .iter()
            .find(|item| item.reason_code.as_deref() == Some(plan_reasons::ROOT_IDENTITY_RETAINED))
            .expect("identity statement");
        let detail = statement.detail.clone().unwrap_or_default();
        assert!(detail.contains("keeps its identity"));
        assert!(detail.contains("remains the library default"));
        assert_eq!(statement.source_path.as_deref(), Some(SOURCE_ROOT));
        assert_eq!(statement.destination_path.as_deref(), Some(DESTINATION_ROOT));
    }

    #[test]
    fn a_cross_volume_root_change_warns_about_hardlinked_sources() {
        let planned = build_root_change_plan(&request(vec![RootChangeTitleDraft {
            hardlinks: vec![HardlinkFact {
                path: "/media/old/Movie A/a.mkv".to_string(),
                link_count: LinkCount::Known(2),
                size_bytes: 10,
            }],
            ..title(
                "movie-a",
                "/media/old/Movie A",
                vec![tracked("/media/old/Movie A/a.mkv", 10)],
            )
        }]));

        assert!(
            planned
                .warnings
                .iter()
                .any(|warning| warning.to_lowercase().contains("hardlink")
                    || warning.to_lowercase().contains("link"))
        );
    }
}
