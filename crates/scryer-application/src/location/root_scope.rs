//! The root-scoped planner: every title on one root goes to one path, and then
//! that root is retired (US4 + US5, FR-020–FR-029, FR-072–FR-075, FR-087).
//!
//! FR-020 offers one settings action, **Change root**, with two destinations:
//!
//! > Each configured root in library settings MUST offer **Change root** (to a
//! > new unconfigured path, or to another existing root in the same library —
//! > the latter being consolidation).
//!
//! Both branches are the same operation. Every title assigned to the root is
//! accounted for with no way to exclude one (FR-023); the source root's content
//! is separated into managed, companion, and unexplained buckets (FR-027); the
//! source location is retired only after recycling completes and only when
//! nothing unexplained is left standing (FR-028, FR-087). Only the **last step**
//! differs, and [`RootScopeVariant`] is that difference:
//!
//! | Variant | Destination | Last step |
//! |---|---|---|
//! | [`RootScopeVariant::ChangePath`] | a new, unconfigured path | the root is repointed; it keeps its synthetic id, its role, and its default status (FR-021, FR-078) |
//! | [`RootScopeVariant::FoldInto`] | another configured root of the same library | the source root's configuration is removed, and the library default may transfer to the destination (FR-022) |
//!
//! # What the destination's content changes
//!
//! A change to an unconfigured path lands on an empty destination, so the
//! source root's relative layout is preserved exactly and nothing can collide.
//! Folding into a configured root lands on content that already belongs to other
//! titles, and that single difference is what brings in FR-024's seven-way
//! classification, FR-025's folder uniquing, FR-072–FR-075's collision and
//! dedup rules, and the merge engine:
//!
//! > **FR-024**: The consolidation preview MUST classify: titles moving into
//! > unused destination folders; titles merging with existing destination
//! > titles; folder-name collisions between unrelated titles; media collisions;
//! > dedup-eligible identical files; sidecar/non-media collisions requiring
//! > rename; and untracked/unsupported content that prevents safe source-root
//! > retirement.
//!
//! Those seven live on [`RootScopeClassification`], counted from the same
//! decisions that produce the plan items, so the preview's summary and its item
//! list cannot drift (SC-004).
//!
//! # Layout, or naming? Both, in that order (FR-026)
//!
//! > **FR-026**: Root replacement SHOULD preserve the source root's relative
//! > folder layout where practical; consolidation MAY apply destination naming
//! > rules to avoid collisions, with every changed folder name previewed.
//!
//! The default is always to re-anchor the absolute source path onto the
//! destination root and keep the whole relative position, however deeply nested
//! — which also means a title folder need not be a direct child of the root.
//! Destination naming is applied only where preserving the layout is *not*
//! practical, which is exactly the two cases a non-empty destination creates:
//! the re-anchored folder is occupied by something that is not this title's
//! merge target (FR-025 uniques it), or the title merges into a destination
//! title that already owns a folder (FR-063 gives it that folder).
//!
//! # Unrelated titles never merge over a name (FR-025)
//!
//! Two branches are offered and this planner takes the first: it uniques. The
//! name comes from the collision engine's own [`collision_rename_base`] plus its
//! numeric disambiguation, so an incoming *folder* is renamed by the same rule
//! as an incoming *file* (FR-074). A merge is decided by canonical metadata
//! identity and nothing else ([`crate::location::identity`], FR-055); a folder
//! name is never evidence of identity.
//!
//! # Two gates, not one
//!
//! The spec blocks two different things for two different reasons:
//!
//! - **Start** is blocked by a blocked title. FR-086 keeps a title with an active
//!   download or import out of a move; FR-023 forbids excluding it from a
//!   root-scoped operation. One holding a blocked title therefore cannot start.
//! - **Source removal** is blocked by unexplained content. US4 scenario 3:
//!   unknown files "are never silently deleted or abandoned, and root removal is
//!   blocked until the user resolves them". That does not stop the titles from
//!   moving; it stops cleanup from taking the source location away underneath
//!   content Scryer cannot explain (FR-028).
//!
//! # Retirement ordering (FR-087)
//!
//! The recycle bin's allowlist is derived from a configured media root, so
//! retiring the source root's configuration before the last source file is
//! recycled would make the bin reject every remaining file — and FR-073 forbids
//! falling back to permanent deletion. [`RootRetirementContract`] carries the
//! source path the allowlist must keep accepting for the operation's whole life,
//! resume included, plus the blockers that must be empty before the
//! configuration step may happen at all.
//!
//! # One title planner (D1)
//!
//! A root-scope title *is* a root move: one library on both sides, the same
//! collision, dedup, rename and hardlink rules, the same instruction set. The
//! one thing this module decides for itself is the destination folder —
//! re-anchored from the source root for a path change (FR-026), or the folder
//! [`resolve_root_scope_folders`] settled on for a fold (FR-025, FR-063) — so
//! the per-title work is [`crate::location::root_move::plan_title`]'s, and what
//! is left here is the root-scoped statements it cannot make and FR-024's seven
//! counts, folded off the plan it produced.
//!
//! # Purity
//!
//! No IO, no clock, no catalog access, in the [`crate::location::root_move`] /
//! [`crate::location::transfer_effects`] idiom: the caller assembles the drafts,
//! the destination listings, and the identity outcomes, and every rule below is
//! testable from literals. The IO half is
//! [`crate::location::root_scope_execution`].
//!
//! # Execution modes (spec gap, recorded)
//!
//! US5 never states which execution modes a fold offers. What the spec *does*
//! say settles it: FR-020 files consolidation under **Change root**, whose other
//! branch names **Move with Scryer**; the heading over FR-030–FR-032 is "Managed
//! move execution"; **Files are already there** is US3/FR-050, about content the
//! user moved by hand into a destination *folder*, which a whole root is not.
//! So **Move with Scryer** is the only requestable fold mode, **CatalogOnly** is
//! derived (FR-076), and **Files are already there** is refused by name.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::location::classify::{ClassificationCounts, TitleLocationClass};
use crate::location::collisions::{
    CollisionNaming, DestinationItem, PathCaseRule, RecycleAvailability, collision_rename_base,
    is_canonical_sidecar_name,
};
use crate::location::hardlinks::HardlinkFact;
use crate::location::identity::DestinationIdentityOutcome;
use crate::location::merge::summary::MergePreviewSummary;
use crate::location::model::{LocationExecutionMode, LocationOperationType, VerificationDepth};
use crate::location::preview::{
    FreeSpaceEstimate, LocationPlan, LocationPlanBuilder, LocationPlanHeader, PlanItem,
    PlanItemKind,
};
use crate::location::root_move::{
    RootMoveExecutionPlan, RootMovePlanRequest, RootMoveTitleDraft, SourceFile, file_name_or,
    merge_summary_items, rebase, same_named_destination_warning,
};
use crate::location::transfer_effects::TitleAssociationFacts;
use crate::stored_paths::path_to_stored_string;

/// Reason codes on the plan items this planner emits, so the UI groups and
/// translates rather than parsing prose (C3).
pub mod plan_reasons {
    /// The root keeps its synthetic id, its role, its default status, and every
    /// title assignment; only its path changes (FR-021, FR-078).
    pub const ROOT_IDENTITY_RETAINED: &str = "root_identity_retained";
    /// The opening statement of a fold: what it does to the two roots
    /// themselves (FR-020, FR-022).
    pub const ROOTS_CONSOLIDATED: &str = "roots_consolidated";
    /// FR-022: the source root was the library default, so the destination
    /// becomes it.
    pub const DEFAULT_ROOT_TRANSFERRED: &str = "default_root_transferred";
    /// FR-024 (1): the re-anchored destination folder is free, so the layout is
    /// preserved exactly (FR-026).
    pub const MOVES_INTO_UNUSED_FOLDER: &str = "moves_into_unused_folder";
    /// FR-024 (2): a destination title shares this title's canonical identity,
    /// so the two merge (FR-055, FR-063).
    pub const MERGES_WITH_DESTINATION_TITLE: &str = "merges_with_destination_title";
    /// FR-024 (3) + FR-025: the re-anchored folder name is taken by something
    /// unrelated, so the incoming folder is uniqued rather than merged over.
    pub const FOLDER_NAME_UNIQUED: &str = "folder_name_uniqued";
    /// The title has no folder to move, so only its stored root path changes
    /// (FR-076).
    pub const CATALOG_ONLY_ROOT_CHANGE: &str = "catalog_only_root_change";
    /// The fold's wording for the same thing.
    pub const CATALOG_ONLY_CONSOLIDATION: &str = "catalog_only_consolidation";
    /// The title cannot enter the root change until the user repairs it; it
    /// cannot be excluded either (FR-023, FR-086).
    pub const TITLE_BLOCKED_FOR_ROOT_CHANGE: &str = "title_blocked_for_root_change";
    /// The fold's wording for the same thing.
    pub const TITLE_BLOCKED_FOR_CONSOLIDATION: &str = "title_blocked_for_consolidation";
    /// Content at the source root the catalog does not explain (FR-027).
    pub const UNKNOWN_ROOT_CONTENT: &str = "unknown_root_content";
    /// Why the source location cannot be removed once the titles have moved
    /// (FR-028, FR-023).
    pub const SOURCE_RETIREMENT_BLOCKED: &str = "source_retirement_blocked";
}

// The destination-folder, outside-the-folder and hardlink reasons are the
// shared planner's and are stated in its vocabulary
// ([`crate::location::root_move::plan_reasons`]): one planner emits them for
// every workflow, so this module does not keep a second copy of the strings.

/// Machine-readable codes for the reasons a source root cannot be retired.
pub mod retirement_blockers {
    /// At least one assigned title is blocked and must be repaired first
    /// (FR-023).
    pub const BLOCKED_TITLES: &str = "blocked_titles";
    /// Content Scryer cannot explain still sits at the source (FR-027, FR-028).
    pub const UNEXPLAINED_SOURCE_CONTENT: &str = "unexplained_source_content";
}

/// A refusal, with the code the client routes on and the sentence it shows.
///
/// One type for both variants: the codes differ (they are what route a user
/// between the two branches of **Change root**), the shape does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootScopeRefusal {
    pub code: &'static str,
    pub detail: String,
}

impl RootScopeRefusal {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for RootScopeRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.detail)
    }
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
pub enum RootScopeTitleOutcome {
    /// The title owns a folder under the source root; the folder relocates.
    Relocates,
    /// The title owns no folder, so nothing moves and only its stored root path
    /// changes (FR-076).
    CatalogOnly,
    /// The title cannot enter the operation yet, and cannot be dropped from it
    /// either (FR-023, FR-086).
    Blocked,
}

impl RootScopeTitleOutcome {
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

    /// The shared classification counts this ledger lowers onto.
    ///
    /// Public because the consolidation planner reuses this whole ledger: US5
    /// accounts for every assigned title by the same FR-023 rule, so it must
    /// also report the same counts.
    pub fn counts(&self) -> ClassificationCounts {
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

// ── FR-022: the default-root transfer ────────────────────────────────────────

/// What consolidating does to the library's default root (FR-022, US5.3).
///
/// > **FR-022**: Consolidating a default source root makes the destination the
/// > default; consolidating a non-default root leaves the default unchanged.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DefaultRootTransfer {
    pub source_was_default: bool,
    pub destination_was_default: bool,
}

impl DefaultRootTransfer {
    /// Whether the destination root is the library default once the source root
    /// is gone. `true` when it already was, and `true` when the source was —
    /// which is FR-022's whole rule.
    pub fn destination_becomes_default(&self) -> bool {
        self.source_was_default || self.destination_was_default
    }

    /// Whether the default actually moved, which is the only case worth a
    /// sentence in the preview.
    pub fn transfers_the_default(&self) -> bool {
        self.source_was_default && !self.destination_was_default
    }

    fn statement(&self, destination_root_path: &str) -> Option<String> {
        if self.transfers_the_default() {
            Some(format!(
                "the source root is this library's default, so {destination_root_path} becomes the default once the consolidation completes"
            ))
        } else if self.source_was_default {
            None
        } else {
            Some(
                "this root is not the library default, so the library's default is unchanged"
                    .to_string(),
            )
        }
    }
}

// ── FR-024: the seven-way preview classification ─────────────────────────────

/// FR-024's seven classifications, counted off the same decisions that built the
/// plan items.
///
/// Three are title-scoped (1–3), three are file-scoped (4–6), and the seventh is
/// the source root's unexplained content — the one that decides whether the
/// source root can be retired at all.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RootScopeClassification {
    /// (1) Titles moving into unused destination folders.
    pub moving_into_unused_folders: i64,
    /// (2) Titles merging with an existing destination title.
    pub merging_with_destination_titles: i64,
    /// (3) Folder-name collisions between unrelated titles, uniqued (FR-025).
    pub folder_name_collisions: i64,
    /// (4) Media files whose name collides with destination media.
    pub media_collisions: i64,
    /// (5) Files proven identical to destination content and therefore
    /// dedup-eligible (FR-073).
    pub dedup_eligible_files: i64,
    /// (6) Sidecars and companion assets whose name collides and which are
    /// therefore renamed (FR-075).
    pub companion_collisions: i64,
    /// (7) Entries at the source root the catalog cannot explain, which prevent
    /// safe source-root retirement (FR-027, FR-028).
    pub untracked_source_entries: i64,
    /// Titles with nothing on disk (FR-076). Not one of FR-024's seven, but the
    /// ledger has to close.
    pub catalog_only: i64,
    /// Titles the user must repair first (FR-023, FR-086).
    pub blocked: i64,
}

impl RootScopeClassification {
    /// Every assigned title lands in exactly one of the five title-scoped
    /// buckets. False can only mean this module lost a title, which is why it is
    /// asserted rather than assumed (FR-023).
    pub fn accounts_for(&self, assigned_total: i64) -> bool {
        assigned_total
            == self.moving_into_unused_folders
                + self.merging_with_destination_titles
                + self.folder_name_collisions
                + self.catalog_only
                + self.blocked
    }
}

// ── Folder resolution (FR-025, FR-026) ───────────────────────────────────────

/// One title, as folder resolution sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderResolutionTitle {
    pub title_id: String,
    pub title_name: String,
    /// The folder the title owns today, under the source root. `None` for a
    /// fileless title (FR-076).
    pub source_folder_path: Option<PathBuf>,
    /// The destination title this title merges into, and the folder that title
    /// owns. `None` when there is no merge.
    pub merge_target_title_id: Option<String>,
    pub merge_target_title_name: Option<String>,
    /// The merge target's own folder. `None` when the destination title owns no
    /// folder, in which case the re-anchored path is used and the destination
    /// title inherits it.
    pub merge_target_folder_path: Option<PathBuf>,
}

/// Everything folder resolution needs. Pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderResolutionRequest {
    pub source_root: PathBuf,
    pub destination_root: PathBuf,
    /// FR-090: previews must match the destination filesystem's own case rules.
    pub case_rule: PathCaseRule,
    /// FR-074's suffix label, reused for folders (FR-025/FR-026).
    pub naming: CollisionNaming,
    pub titles: Vec<FolderResolutionTitle>,
    /// Every destination path that is **not** free, keyed by its stored form,
    /// with the destination title that owns it when a title does.
    ///
    /// A path missing from the map is free: either nothing is there, or an
    /// empty unowned directory is, which is nothing to collide with and is
    /// reused as it stands.
    pub destination_occupants: BTreeMap<String, Option<String>>,
}

/// Where one title's folder lands, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFolder {
    pub title_id: String,
    /// `None` only for a title that owns no folder.
    pub destination_folder: Option<PathBuf>,
    /// FR-024 (3) + FR-025: the name the layout would have preserved, when
    /// something unrelated already held it and the incoming folder was uniqued
    /// instead. `None` when the folder landed on the name it asked for — which
    /// includes a merge, where the destination title's own folder is the
    /// answer (FR-063).
    pub collided_name: Option<String>,
    /// The destination title that already owned `collided_name`, when the
    /// occupier is a title rather than untracked content.
    pub occupied_by_title_id: Option<String>,
    /// The folder name the operation ends up writing, when it differs from the
    /// source folder's own name. US5.4: every changed folder name is shown
    /// before confirmation.
    pub renamed_to: Option<String>,
}

/// Resolve every title's destination folder (FR-025, FR-026).
///
/// Ordering matters and is deliberate: titles are resolved in the order given,
/// and each one claims the folder it lands in, so a name uniqued for title A is
/// not handed to title B a moment later. The caller passes titles in a stable
/// order (by id), so a re-plan produces the same names — which is what makes the
/// fingerprint meaningful across a preview/start pair (FR-081).
pub fn resolve_root_scope_folders(request: &FolderResolutionRequest) -> Vec<ResolvedFolder> {
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    let mut resolved = Vec::with_capacity(request.titles.len());

    for title in &request.titles {
        let Some(source_folder) = title.source_folder_path.as_ref() else {
            resolved.push(ResolvedFolder {
                title_id: title.title_id.clone(),
                destination_folder: None,
                collided_name: None,
                occupied_by_title_id: None,
                renamed_to: None,
            });
            continue;
        };

        // FR-063: a merge target keeps the folder it already owns; its content
        // is already there and stays there.
        if title.merge_target_title_id.is_some() {
            let destination_folder = title
                .merge_target_folder_path
                .clone()
                .or_else(|| {
                    rebase(
                        source_folder,
                        &request.source_root,
                        &request.destination_root,
                    )
                })
                .unwrap_or_else(|| {
                    request
                        .destination_root
                        .join(file_name_or(source_folder, &title.title_id))
                });
            claimed.insert(fold(&request.case_rule, &destination_folder));
            let renamed_to = renamed_name(source_folder, &destination_folder);
            resolved.push(ResolvedFolder {
                title_id: title.title_id.clone(),
                destination_folder: Some(destination_folder),
                collided_name: None,
                occupied_by_title_id: None,
                renamed_to,
            });
            continue;
        }

        // FR-026: preserve the source root's relative folder layout by default.
        let preserved = rebase(
            source_folder,
            &request.source_root,
            &request.destination_root,
        )
        .unwrap_or_else(|| {
            // A folder outside the root cannot be re-anchored; its own
            // basename directly under the destination root is the only
            // defensible place. `classify_root_content` already treats such
            // content as unexplained, so this is the belt to that braces.
            request
                .destination_root
                .join(file_name_or(source_folder, &title.title_id))
        });

        let occupant = request
            .destination_occupants
            .get(&path_to_stored_string(&preserved));
        let already_claimed = claimed.contains(&fold(&request.case_rule, &preserved));

        if !already_claimed && occupant.is_none() {
            claimed.insert(fold(&request.case_rule, &preserved));
            resolved.push(ResolvedFolder {
                title_id: title.title_id.clone(),
                destination_folder: Some(preserved),
                collided_name: None,
                occupied_by_title_id: None,
                renamed_to: None,
            });
            continue;
        }

        // FR-025: unrelated titles never merge over a name.
        let collided_name = file_name_or(&preserved, &title.title_id);
        let unique = unique_folder_path(
            &preserved,
            &collided_name,
            &request.naming.source_library_label,
            &request.case_rule,
            &claimed,
            &request.destination_occupants,
        );
        claimed.insert(fold(&request.case_rule, &unique));
        let occupied_by_title_id = occupant.cloned().flatten();
        let renamed_to = renamed_name(source_folder, &unique);
        resolved.push(ResolvedFolder {
            title_id: title.title_id.clone(),
            destination_folder: Some(unique),
            collided_name: Some(collided_name),
            occupied_by_title_id,
            renamed_to,
        });
    }

    resolved
}

/// `"<name> (from <Label>)"`, then `"(2)"`, `"(3)"`… until nothing holds it.
///
/// The base and the numbering are the collision engine's own
/// ([`collision_rename_base`] and the same `" (n)"` shape it appends), so a
/// folder is renamed by the rule that renames files (FR-074/FR-075) rather than
/// by a second scheme the user has to learn.
fn unique_folder_path(
    preserved: &Path,
    collided_name: &str,
    label: &str,
    case_rule: &PathCaseRule,
    claimed: &BTreeSet<String>,
    destination_occupants: &BTreeMap<String, Option<String>>,
) -> PathBuf {
    let parent = preserved
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let base = collision_rename_base(collided_name, label);

    for attempt in 0..1_000_u32 {
        let name = if attempt == 0 {
            base.clone()
        } else {
            format!("{base} ({})", attempt + 1)
        };
        let candidate = parent.join(&name);
        let folded = fold(case_rule, &candidate);
        if claimed.contains(&folded) {
            continue;
        }
        if !destination_occupants.contains_key(&path_to_stored_string(&candidate)) {
            return candidate;
        }
    }
    // Unreachable in practice; a thousand same-named folders is a filesystem
    // nobody has. Falling back to the preserved path is still safe because the
    // collision engine refuses to overwrite anything inside it (FR-072).
    preserved.to_path_buf()
}

fn fold(case_rule: &PathCaseRule, path: &Path) -> String {
    case_rule.fold(&path_to_stored_string(path)).into_owned()
}

/// The folder name the operation ends up writing, when it differs from the
/// source folder's own name. US5.4: every changed folder name is shown before
/// confirmation.
fn renamed_name(source_folder: &Path, destination_folder: &Path) -> Option<String> {
    let source = source_folder.file_name()?;
    let destination = destination_folder.file_name()?;
    (source != destination).then(|| destination.to_string_lossy().to_string())
}

// ── Destination admissibility (FR-020) ───────────────────────────────────────

/// Machine-readable codes for the reasons a **Change root** request is refused
/// before anything is planned, plus the two or three words each refusal needs to
/// name the branch it is refusing on.
///
/// Named rather than prose so the client can route the user — notably "this is
/// consolidation, not a root change", and its mirror image. The rules are the
/// same on both branches and the *codes* are what tell them apart, so this is
/// one rule table with two columns rather than two tables that have to be kept
/// in step by hand.
pub mod refusal_codes {
    /// One branch's half of the shared refusal vocabulary.
    pub struct RootScopeRefusalVocabulary {
        /// A root path has to be absolute; a relative one means different
        /// things to Scryer and to the user's shell.
        pub path_not_absolute: &'static str,
        /// Source and destination are the same location, or one contains the
        /// other.
        pub paths_overlap: &'static str,
        /// The source root is a symlink. Moving out of one and retiring it
        /// would act on the link, not on the content the user means.
        pub source_root_is_symlink: &'static str,
        /// The source root is not a directory Scryer can read right now.
        pub source_root_unavailable: &'static str,
        /// The request named an execution mode neither branch offers: both are
        /// managed moves, and "files are already there" adopts content the user
        /// placed by hand, which is a different workflow (US3).
        pub mode_not_supported: &'static str,
        /// What this branch is, for the refusals that have to name it.
        pub subject: &'static str,
        /// What the user does about a symlinked source root.
        pub resolve_symlink_action: &'static str,
        /// What overlapping paths would mean on this branch.
        pub overlap_consequence: &'static str,
    }

    pub const CHANGE: RootScopeRefusalVocabulary = RootScopeRefusalVocabulary {
        path_not_absolute: "root_change_path_not_absolute",
        paths_overlap: "root_change_paths_overlap",
        source_root_is_symlink: "root_change_source_root_is_symlink",
        source_root_unavailable: "root_change_source_root_unavailable",
        mode_not_supported: "root_change_mode_not_supported",
        subject: "a root change",
        resolve_symlink_action: "changing the root",
        overlap_consequence: "a root change needs a destination outside the root it replaces",
    };

    pub const FOLD: RootScopeRefusalVocabulary = RootScopeRefusalVocabulary {
        path_not_absolute: "root_consolidation_path_not_absolute",
        paths_overlap: "root_consolidation_paths_overlap",
        source_root_is_symlink: "root_consolidation_source_root_is_symlink",
        source_root_unavailable: "root_consolidation_source_root_unavailable",
        mode_not_supported: "root_consolidation_mode_not_supported",
        subject: "a consolidation",
        resolve_symlink_action: "consolidating it",
        overlap_consequence: "consolidating one into the other would move content into itself",
    };

    /// The destination exists and is not an empty directory.
    pub const CHANGE_DESTINATION_NOT_EMPTY: &str = "root_change_destination_not_empty";
    /// The destination does not exist and neither does its parent, so nothing
    /// would create it.
    pub const CHANGE_DESTINATION_PARENT_MISSING: &str = "root_change_destination_parent_missing";
    /// The destination is a configured root of a *different* library. A
    /// destination that is a root of **this** library is not refused at all —
    /// it is the same request said the other way, and FR-020 plans it as a fold
    /// (see `AppUseCase::resolve_root_scope_destination`).
    pub const CHANGE_DESTINATION_IS_CONFIGURED_ROOT: &str =
        "root_change_destination_is_configured_root";
    /// Source and destination are the same root.
    pub const FOLD_SAME_ROOT: &str = "root_consolidation_same_root";
    /// The destination root is not a readable directory right now, so what it
    /// already holds cannot be planned against.
    pub const FOLD_DESTINATION_ROOT_UNAVAILABLE: &str =
        "root_consolidation_destination_root_unavailable";
}

/// What the destination path is, as the caller's `stat` found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationPathState {
    /// Nothing exists at the path.
    Missing { parent_exists: bool },
    /// An existing directory, and whether it holds anything.
    Directory { empty: bool },
    /// Something that is not a directory: a file, a symlink, a device node.
    NotADirectory,
}

/// Which branch's destination the request named, and the configuration each one
/// is checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootScopePathVariant {
    /// US4: a new path that is not a root of this library.
    ChangePath {
        /// Every configured root path **outside this library**, canonicalized
        /// the same way, paired with the id of the root that holds it. A root
        /// of this library is never in here: the caller resolved that case to
        /// [`RootScopePathVariant::FoldInto`] before asking.
        configured_roots: Vec<(String, PathBuf)>,
    },
    /// US5: another configured root of the same library.
    FoldInto {
        source_root_id: String,
        destination_root_id: String,
    },
}

impl RootScopePathVariant {
    fn vocabulary(&self) -> &'static refusal_codes::RootScopeRefusalVocabulary {
        match self {
            Self::ChangePath { .. } => &refusal_codes::CHANGE,
            Self::FoldInto { .. } => &refusal_codes::FOLD,
        }
    }

    fn folds(&self) -> bool {
        matches!(self, Self::FoldInto { .. })
    }
}

/// The filesystem and configuration facts a root-scoped request is checked
/// against.
///
/// Separated from the check itself for the same reason the planner is pure: the
/// rules are then testable from literals, and the `stat`s happen once, in the
/// use case, where they can be reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootScopePathFacts {
    pub variant: RootScopePathVariant,
    /// Canonicalized where possible; the source root is expected to exist.
    pub source_root: PathBuf,
    /// The destination with its existing ancestors canonicalized.
    pub destination_root: PathBuf,
    pub source_root_is_symlink: bool,
    pub source_root_is_directory: bool,
    pub destination: DestinationPathState,
    /// The execution mode the request named. Checked here rather than in the
    /// interface so the refusal is application vocabulary the client can
    /// translate, exactly as the plan items above do.
    pub mode: LocationExecutionMode,
}

/// FR-020's admissibility rules, for either of its two branches.
///
/// Deliberately *not* in [`build_root_scope_plan`]: the planner is pure, and
/// these are questions only the filesystem can answer. The use case asks them
/// twice — once when the preview is built, and once when the operation is
/// admitted — because the destination is a path anything could have written to
/// between the two.
pub fn check_root_scope_paths(facts: &RootScopePathFacts) -> Result<(), RootScopeRefusal> {
    let codes = facts.variant.vocabulary();

    // Neither branch adopts. Without this the mode would pass straight into the
    // plan header and label the operation `FILES_ALREADY_THERE` while the
    // executor performed a managed move.
    if facts.mode == LocationExecutionMode::FilesAlreadyThere {
        return Err(RootScopeRefusal::new(
            codes.mode_not_supported,
            format!(
                "{} moves the files itself; \"files are already there\" adopts content at a destination folder and is not offered here",
                codes.subject
            ),
        ));
    }

    // A fold's destination is named by id, so "is it this root?" is answerable
    // before anything is `stat`ed. "Is it a root at all?" is not asked here:
    // the caller resolved the destination against the library's roots and a
    // name that matched none of them never reaches the rules.
    if let RootScopePathVariant::FoldInto {
        source_root_id,
        destination_root_id,
    } = &facts.variant
        && source_root_id == destination_root_id
    {
        return Err(RootScopeRefusal::new(
            refusal_codes::FOLD_SAME_ROOT,
            format!("root {source_root_id} cannot be consolidated into itself"),
        ));
    }

    for path in [&facts.source_root, &facts.destination_root] {
        if !path.is_absolute() {
            return Err(RootScopeRefusal::new(
                codes.path_not_absolute,
                format!("{} is not an absolute path", path.display()),
            ));
        }
    }

    if facts.source_root_is_symlink {
        return Err(RootScopeRefusal::new(
            codes.source_root_is_symlink,
            format!(
                "{} is a symlink; resolve it to the real directory before {}",
                facts.source_root.display(),
                codes.resolve_symlink_action
            ),
        ));
    }
    if !facts.source_root_is_directory {
        return Err(RootScopeRefusal::new(
            codes.source_root_unavailable,
            format!(
                "{} is not a readable directory right now, so its contents cannot be planned",
                facts.source_root.display()
            ),
        ));
    }
    // Only a fold plans *against* its destination's contents, so only a fold
    // needs the destination readable before anything else is decided.
    if facts.variant.folds() && !matches!(facts.destination, DestinationPathState::Directory { .. })
    {
        return Err(RootScopeRefusal::new(
            refusal_codes::FOLD_DESTINATION_ROOT_UNAVAILABLE,
            format!(
                "{} is not a readable directory right now, so what it already holds cannot be planned against",
                facts.destination_root.display()
            ),
        ));
    }

    // Two configured roots should never nest, and a root change needs somewhere
    // outside itself to move to; either way, overlapping paths would mean
    // moving content into itself.
    if facts.destination_root == facts.source_root
        || facts.destination_root.starts_with(&facts.source_root)
        || facts.source_root.starts_with(&facts.destination_root)
    {
        return Err(RootScopeRefusal::new(
            codes.paths_overlap,
            format!(
                "{} and {} overlap; {}",
                facts.source_root.display(),
                facts.destination_root.display(),
                codes.overlap_consequence
            ),
        ));
    }

    let RootScopePathVariant::ChangePath { configured_roots } = &facts.variant else {
        return Ok(());
    };

    // A root of *this* library is the fold branch and never gets here. A root of
    // some other library is a genuine refusal: two libraries sharing one root
    // directory is a configuration Scryer cannot reconcile, and accepting it
    // here would move content into the other library's tree.
    if let Some((configured_root_id, path)) = configured_roots
        .iter()
        .find(|(_, path)| path == &facts.destination_root)
    {
        return Err(RootScopeRefusal::new(
            refusal_codes::CHANGE_DESTINATION_IS_CONFIGURED_ROOT,
            format!(
                "{} is already configured as root {configured_root_id} of another library",
                path.display()
            ),
        ));
    }

    match facts.destination {
        DestinationPathState::Missing {
            parent_exists: true,
        }
        | DestinationPathState::Directory { empty: true } => Ok(()),
        DestinationPathState::Missing {
            parent_exists: false,
        } => Err(RootScopeRefusal::new(
            refusal_codes::CHANGE_DESTINATION_PARENT_MISSING,
            format!(
                "{} does not exist and neither does the directory that would contain it",
                facts.destination_root.display()
            ),
        )),
        DestinationPathState::Directory { empty: false } => Err(RootScopeRefusal::new(
            refusal_codes::CHANGE_DESTINATION_NOT_EMPTY,
            format!(
                "{} already holds content; a root change needs an empty or not-yet-created destination",
                facts.destination_root.display()
            ),
        )),
        DestinationPathState::NotADirectory => Err(RootScopeRefusal::new(
            refusal_codes::CHANGE_DESTINATION_NOT_EMPTY,
            format!(
                "{} exists and is not a directory",
                facts.destination_root.display()
            ),
        )),
    }
}

// ── Planner input ────────────────────────────────────────────────────────────

/// Which of FR-020's two destinations this operation has, and therefore which
/// last step it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootScopeVariant {
    /// US4: a new, unconfigured path. The root is repointed and keeps its
    /// synthetic id, its role, and its default status (FR-021, FR-078).
    ChangePath { retention: RootRetentionFacts },
    /// US5: another configured root of the same library. The source root's
    /// configuration is retired, and the library default may transfer (FR-022).
    FoldInto {
        destination_root_id: String,
        default_transfer: DefaultRootTransfer,
    },
}

impl RootScopeVariant {
    /// Whether the destination is a root that already holds content, which is
    /// what brings in FR-024/FR-025 and the collision engine.
    pub fn folds_into_existing_root(&self) -> bool {
        matches!(self, Self::FoldInto { .. })
    }

    /// The operation type the shared plan header carries, which is what derives
    /// FR-029's typed confirmation.
    fn operation_type(&self) -> LocationOperationType {
        match self {
            Self::ChangePath { .. } => LocationOperationType::RootChange,
            Self::FoldInto { .. } => LocationOperationType::RootConsolidation,
        }
    }
}

/// Everything the planner needs about one title assigned to the root.
///
/// The fold-only fields default to "nothing at the destination", which is
/// exactly what a change to an unconfigured path finds there — so one draft
/// serves both variants without either having to pretend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootScopeTitleDraft {
    pub title_id: String,
    pub title_name: String,
    /// The folder the title owns today. `None` for a title with no folder match
    /// — it is still accounted for (FR-023), it simply has nothing to move.
    pub source_folder_path: Option<PathBuf>,
    /// Files beneath (or tracked by) the title, as the shared planner currency.
    ///
    /// [`SourceFile::relative_path`] is read only when the title's folder may be
    /// renamed, which only a fold can do: a change to an unconfigured path
    /// preserves the layout relative to the **root** (FR-026), so every
    /// destination is derived from the source path itself.
    /// [`SourceFile::full_blake3`] is likewise read only by a fold — dedup needs
    /// a destination that already holds content (FR-073, D4).
    pub files: Vec<SourceFile>,
    /// Directories beneath the title's folder, which cleanup may remove once
    /// empty (FR-028). Deepest first.
    pub source_directories: Vec<PathBuf>,
    pub hardlinks: Vec<HardlinkFact>,
    /// Why the title is blocked: an active download or import (FR-086), an
    /// unresolved repair, or another operation already owning it (FR-084).
    /// `Some` is what makes the title [`RootScopeTitleOutcome::Blocked`].
    pub blocked_reason: Option<String>,
    pub blocked_reason_code: Option<String>,

    /// Where this title's folder lands, from [`resolve_root_scope_folders`].
    /// `None` for a change to an unconfigured path, which needs no resolution:
    /// the re-anchored path is always free.
    pub resolved: Option<ResolvedFolder>,
    /// Entries already present at the resolved destination folder (FR-072).
    /// Always empty for a change to an unconfigured path.
    pub destination_entries: Vec<DestinationItem>,
    /// Whether recycling is usable for this title's source root (FR-073).
    pub recycle: RecycleAvailability,
    /// What destination-title detection concluded (FR-055), for the same-name
    /// warning that FR-025 exists to make unnecessary.
    pub destination_identity: Option<DestinationIdentityOutcome>,
    /// The merge the engine planned for this title at preview time (FR-066,
    /// FR-071), or `None` when the title is not a merge candidate.
    pub merge_summary: Option<MergePreviewSummary>,
}

impl RootScopeTitleDraft {
    /// A draft for a title with nothing at the destination — the shape a change
    /// to an unconfigured path always has.
    pub fn new(title_id: impl Into<String>, title_name: impl Into<String>) -> Self {
        Self {
            title_id: title_id.into(),
            title_name: title_name.into(),
            source_folder_path: None,
            files: Vec::new(),
            source_directories: Vec::new(),
            hardlinks: Vec::new(),
            blocked_reason: None,
            blocked_reason_code: None,
            resolved: None,
            destination_entries: Vec::new(),
            recycle: RecycleAvailability::Available,
            destination_identity: None,
            merge_summary: None,
        }
    }

    /// FR-023: every assigned title lands in exactly one outcome, and none of
    /// them is "excluded".
    pub fn outcome(&self) -> RootScopeTitleOutcome {
        if self.blocked_reason.is_some() {
            RootScopeTitleOutcome::Blocked
        } else if self.source_folder_path.is_none() {
            RootScopeTitleOutcome::CatalogOnly
        } else {
            RootScopeTitleOutcome::Relocates
        }
    }

    /// Bytes this title contributes to the operation, for the caller's
    /// free-space estimate (FR-080).
    pub fn bytes_total(&self) -> u64 {
        self.files
            .iter()
            .fold(0_u64, |total, file| total.saturating_add(file.size_bytes))
    }

    /// The destination title this title folds into (US7, FR-055, FR-063).
    ///
    /// Read off the detection outcome rather than stored beside it, so the
    /// planner and folder resolution can never disagree about which titles
    /// merge.
    pub fn merge_target(&self) -> Option<&str> {
        self.destination_identity
            .as_ref()
            .and_then(DestinationIdentityOutcome::merge_target)
    }

    /// FR-024 (3): the destination name this title's folder could not have,
    /// when FR-025 uniqued it instead.
    fn collided_name(&self) -> Option<&str> {
        self.resolved
            .as_ref()
            .and_then(|resolved| resolved.collided_name.as_deref())
    }
}

/// The whole planning request. Every fact is supplied; nothing is read here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootScopePlanRequest {
    pub library_id: String,
    /// The root the operation acts on. For a path change it is repointed and
    /// keeps this id (FR-078); for a fold its configuration is retired.
    pub root_id: String,
    pub source_root_path: PathBuf,
    /// The path every title lands under: the new unconfigured path, or the
    /// destination root's configured path.
    pub destination_root_path: PathBuf,
    pub variant: RootScopeVariant,
    /// **Move with Scryer** or **Files are already there** (US4). A request with
    /// nothing to move is downgraded to [`LocationExecutionMode::CatalogOnly`]
    /// (FR-076); a fold only ever runs **Move with Scryer** (see the module
    /// docs).
    pub mode: LocationExecutionMode,
    /// Every title assigned to the root. Not a selection: FR-023 forbids one.
    pub titles: Vec<RootScopeTitleDraft>,
    /// The caller's scan of the source root.
    pub entries: Vec<RootEntry>,
    pub verification_depth: VerificationDepth,
    pub free_space: FreeSpaceEstimate,
    /// `Some(true)` when the two paths share a volume (rename fast path,
    /// FR-032), `None` when the relationship could not be probed.
    pub same_volume: Option<bool>,
    /// FR-090: previews must match the destination filesystem's own case rules.
    /// Unread by a change to an unconfigured path, which cannot collide.
    pub case_rule: PathCaseRule,
    /// FR-074's suffix label, reused for folders (FR-025/FR-026).
    pub naming: CollisionNaming,
}

impl RootScopePlanRequest {
    /// The root id the destination side of the shared plan currency carries.
    /// For a path change it is the *same* root: the path moves, the identity
    /// does not (FR-021, FR-078).
    pub fn destination_root_id(&self) -> &str {
        match &self.variant {
            RootScopeVariant::ChangePath { .. } => &self.root_id,
            RootScopeVariant::FoldInto {
                destination_root_id,
                ..
            } => destination_root_id,
        }
    }
}

/// The planner result: the plan the user confirms, the instructions the runner
/// executes, and the root-scoped facts neither of those two can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRootScope {
    pub plan: LocationPlan,
    pub execution: RootMoveExecutionPlan,
    pub accounting: TitleAccounting,
    /// The post-conditions the path flip is checked against. Meaningful for a
    /// path change; a fold retires the source root instead, and its executor
    /// builds the destination root's retention separately.
    pub retention: RootIdentityRetention,
    /// FR-024's seven counters. A change to an unconfigured path fills only the
    /// three that can be non-zero there.
    pub classification: RootScopeClassification,
    /// FR-022. Default for a path change, which never moves the default.
    pub default_transfer: DefaultRootTransfer,
    pub content: RootContentInventory,
    pub retirement: RootRetirementContract,
    pub warnings: Vec<String>,
}

impl PlannedRootScope {
    /// The runner's view of this plan, through the shared work-plan seam.
    pub fn work_plan(&self) -> crate::location::executor::OperationWorkPlan {
        self.execution.to_work_plan()
    }
}

/// The fold-only half of the root-scoped tail.
///
/// It rides *inside* [`RootScopeTail`] rather than beside it, because the two
/// branches of FR-020's **Change root** share one epilogue: the same recycle-bin
/// relocation, the same empty-directory prune, the same "only after all
/// recycling completes" ordering (FR-087). Only the last step differs, and that
/// is exactly what this struct's presence selects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationTail {
    /// The root that absorbs the content. It keeps its id (FR-078).
    pub destination_root_id: String,
    pub default_transfer: DefaultRootTransfer,
}

/// The root-scoped facts that ride on the persisted plan so a resumed run has
/// them (FR-033, FR-087).
///
/// Everything here is durable-by-necessity rather than by convenience: the
/// retirement contract carries the recycle allowlist an in-retirement root
/// depends on and the only list of directories cleanup is allowed to remove;
/// the retention block is the set of post-conditions the configuration step is
/// checked against; and `assigned_title_ids` is FR-084's ownership claim, which
/// covers titles the instruction set does not (a catalog-only title has no
/// files, and a blocked title has no instructions at all).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootScopeTail {
    pub library_id: String,
    /// The root the operation acts on. Identical on both sides for a path
    /// change (FR-021, FR-078); the retired root for a fold.
    pub root_id: String,
    pub source_root_path: String,
    pub destination_root_path: String,
    /// Every title assigned to the root at preview time (FR-023, FR-084).
    pub assigned_title_ids: Vec<String>,
    pub retention: RootIdentityRetention,
    pub content: RootContentInventory,
    pub retirement: RootRetirementContract,
    /// Set when this root-scoped operation is FR-020's **second** branch: a fold
    /// into an existing root of the same library (US5).
    ///
    /// When it is set, `root_id` is the **source** root (the one being retired),
    /// `destination_root_path` is the destination root's configured path, and
    /// `retention` describes the **destination** root — the one that keeps its
    /// synthetic id and may gain the library default (FR-022, FR-078).
    ///
    /// Defaulted so a root change persisted before the fold existed still
    /// resumes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consolidation: Option<ConsolidationTail>,
}

impl RootScopeTail {
    pub fn source_root(&self) -> PathBuf {
        crate::stored_paths::stored_path_to_path_buf(&self.source_root_path)
    }

    pub fn destination_root(&self) -> PathBuf {
        crate::stored_paths::stored_path_to_path_buf(&self.destination_root_path)
    }
}

impl PlannedRootScope {
    /// The persistable tail for this plan.
    pub fn tail(
        &self,
        library_id: &str,
        root_id: &str,
        assigned_title_ids: Vec<String>,
        consolidation: Option<ConsolidationTail>,
    ) -> RootScopeTail {
        RootScopeTail {
            library_id: library_id.to_string(),
            root_id: root_id.to_string(),
            source_root_path: self.retirement.source_root_path.clone(),
            destination_root_path: self.retirement.destination_root_path.clone(),
            assigned_title_ids,
            retention: self.retention.clone(),
            content: self.content.clone(),
            retirement: self.retirement.clone(),
            consolidation,
        }
    }
}

// ── Planner ──────────────────────────────────────────────────────────────────

/// Build the root-scoped preview and execution plan for either variant.
pub fn build_root_scope_plan(request: &RootScopePlanRequest) -> PlannedRootScope {
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
    // FR-027: the three buckets and the prunable/retained directory split are
    // the same question for both variants, and the two branches of one settings
    // action must answer it the same way.
    let content = classify_root_content(
        &request.source_root_path,
        &request.entries,
        &title_folders,
        &tracked_media_paths,
    );

    let retention_facts = match &request.variant {
        RootScopeVariant::ChangePath { retention } => retention.clone(),
        RootScopeVariant::FoldInto { .. } => RootRetentionFacts::default(),
    };
    let retention = RootIdentityRetention {
        root_id: request.root_id.clone(),
        keeps_root_id: true,
        was_library_default: retention_facts.is_library_default,
        remains_library_default: retention_facts.is_library_default,
        retained_role: retention_facts.role.clone(),
        retained_title_assignments: accounting.assigned_total,
    };
    let default_transfer = match &request.variant {
        RootScopeVariant::ChangePath { .. } => DefaultRootTransfer::default(),
        RootScopeVariant::FoldInto {
            default_transfer, ..
        } => *default_transfer,
    };

    let source_root_display = path_to_stored_string(&request.source_root_path);
    let destination_root_display = path_to_stored_string(&request.destination_root_path);
    let retirement = build_retirement_contract(
        &source_root_display,
        &destination_root_display,
        &accounting,
        &content,
        &request.variant,
    );

    let header = LocationPlanHeader::new(
        request.variant.operation_type(),
        execution_mode_for(request, &accounting),
    )
    .with_source(
        Some(request.library_id.clone()),
        Some(request.root_id.clone()),
    )
    // FR-021/FR-078: a path change carries one root id on both sides — the path
    // moves, the identity does not. A fold names a real, different root.
    .with_destination(
        Some(request.library_id.clone()),
        Some(request.destination_root_id().to_string()),
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

    // The root-level statement first: what the operation does to the root or the
    // two roots themselves, before anything it does to a title.
    match &request.variant {
        RootScopeVariant::ChangePath { .. } => {
            builder.push(
                PlanItem::new(PlanItemKind::CatalogChange)
                    .with_paths(
                        Some(source_root_display.clone()),
                        Some(destination_root_display.clone()),
                    )
                    .with_reason_code(plan_reasons::ROOT_IDENTITY_RETAINED)
                    .with_detail(
                        retention.statement(&source_root_display, &destination_root_display),
                    ),
            );
        }
        RootScopeVariant::FoldInto { .. } => {
            builder.push(
                PlanItem::new(PlanItemKind::CatalogChange)
                    .with_paths(
                        Some(source_root_display.clone()),
                        Some(destination_root_display.clone()),
                    )
                    .with_reason_code(plan_reasons::ROOTS_CONSOLIDATED)
                    .with_detail(format!(
                        "every title on {source_root_display} moves onto {destination_root_display}; the source root's configuration is retired once all {} title(s) have landed and everything this operation recycles has completed",
                        accounting.assigned_total
                    )),
            );
            if let Some(statement) = default_transfer.statement(&destination_root_display) {
                builder.push(
                    PlanItem::new(PlanItemKind::CatalogChange)
                        .with_paths(
                            Some(source_root_display.clone()),
                            Some(destination_root_display.clone()),
                        )
                        .with_reason_code(plan_reasons::DEFAULT_ROOT_TRANSFERRED)
                        .with_detail(statement),
                );
            }
        }
    }

    // D1: one per-title planner. The root-scope layer decides the destination
    // folder (FR-025, FR-026) and states what the operation does to the *root*;
    // everything below the title — collisions, dedup, renames, the hardlink
    // warnings, the instruction set — is the shared root-move planner's, so the
    // two workflows cannot drift apart in what they promise or what they do.
    let move_request = shared_plan_request(request);
    let mut title_items: Vec<PlanItem> = Vec::new();
    for (index, draft) in request.titles.iter().enumerate() {
        let (title_execution, items, title_warnings) = crate::location::root_move::plan_title(
            &move_request,
            &root_move_draft(request, draft),
            index as i64,
        );
        let (items, restated_warnings) = restate_for_root_scope(request, draft, items);
        title_items.extend(items);
        warnings.extend(title_warnings);
        warnings.extend(restated_warnings.iter().cloned());
        // FR-071 + FR-081: the merge summary is both what the preview shows and
        // part of what the fingerprint covers, so it is recorded for every merge
        // candidate — including a blocked one, whose records the user has to see
        // before deciding anything.
        if let Some(summary) = draft.merge_summary.clone() {
            builder.merge(summary);
        }
        if let Some(mut title_execution) = title_execution {
            // A root-scope statement is the title's warning too: the runner
            // reports what the preview promised, and FR-055's same-name line is
            // the one the operation has to end by saying.
            title_execution.warnings.extend(restated_warnings);
            execution.titles.push(title_execution);
        }
    }
    let classification = classification_from(request, &accounting, &content, &title_items);
    builder.extend(title_items);

    // FR-024 (7) / FR-027: unexplained content is listed, item by item, so it is
    // neither silently deleted nor silently abandoned — and so that new junk
    // appearing at the source between preview and start changes the fingerprint.
    let unknown_detail_suffix = if request.variant.folds_into_existing_root() {
        ", and the source root stays configured until it is resolved"
    } else {
        ""
    };
    for entry in &content.unknown {
        builder.push(
            PlanItem::new(PlanItemKind::UnmanagedContent)
                .with_paths(Some(entry.path.clone()), Option::<String>::None)
                .with_size(entry.size_bytes)
                .with_reason_code(plan_reasons::UNKNOWN_ROOT_CONTENT)
                .with_detail(format!(
                    "{} is not tracked by any title on this root; it stays where it is{unknown_detail_suffix}",
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

    PlannedRootScope {
        plan: builder.build(),
        execution,
        accounting,
        retention,
        classification,
        default_transfer,
        content,
        retirement,
        warnings,
    }
}

/// The FR-023 ledger: assigned titles in, the same number out, none excluded.
fn build_accounting(titles: &[RootScopeTitleDraft]) -> TitleAccounting {
    let mut accounting = TitleAccounting {
        assigned_total: titles.len() as i64,
        ..TitleAccounting::default()
    };
    for draft in titles {
        match draft.outcome() {
            RootScopeTitleOutcome::Relocates => accounting.relocating += 1,
            RootScopeTitleOutcome::CatalogOnly => accounting.catalog_only += 1,
            RootScopeTitleOutcome::Blocked => {
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

/// FR-028/FR-023.
///
/// The blockers are the same two either way, and they mean *more* for a fold: a
/// path change keeps the source root configured (pointing somewhere else), so
/// unexplained content only stops the old directory from being deleted. A fold
/// removes the root's configuration, and US4.3's "root removal is blocked until
/// the user resolves them" is then literally about this operation's last step.
fn build_retirement_contract(
    source_root_display: &str,
    destination_root_display: &str,
    accounting: &TitleAccounting,
    content: &RootContentInventory,
    variant: &RootScopeVariant,
) -> RootRetirementContract {
    let mut blockers = Vec::new();
    if accounting.blocks_start() {
        blockers.push(RootRetirementBlocker {
            code: retirement_blockers::BLOCKED_TITLES.to_string(),
            detail: format!(
                "{} title(s) on this root must be repaired before the source root can be retired; they cannot be excluded from a {}",
                accounting.blocked,
                if variant.folds_into_existing_root() {
                    "consolidation"
                } else {
                    "root change"
                }
            ),
        });
    }
    if content.blocks_source_removal() {
        let detail = if variant.folds_into_existing_root() {
            format!(
                "{} item(s) at {source_root_display} are not explained by the catalog, so {source_root_display} stays a configured root until they are resolved; its titles still move onto {destination_root_display}",
                content.unknown.len()
            )
        } else {
            format!(
                "{} item(s) at {source_root_display} are not explained by the catalog; the source location is kept until they are resolved",
                content.unknown.len()
            )
        };
        blockers.push(RootRetirementBlocker {
            code: retirement_blockers::UNEXPLAINED_SOURCE_CONTENT.to_string(),
            detail,
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

/// An operation with nothing to move needs no move mode; FR-076 asks the UI to
/// skip the chooser in exactly that case.
///
/// A fold never consults the requested mode: **files are already there** was
/// refused at admission ([`check_root_scope_paths`]), so the only mode a fold can be
/// running in is **Move with Scryer**.
fn execution_mode_for(
    request: &RootScopePlanRequest,
    accounting: &TitleAccounting,
) -> LocationExecutionMode {
    if accounting.relocating == 0 {
        return LocationExecutionMode::CatalogOnly;
    }
    if request.variant.folds_into_existing_root() {
        LocationExecutionMode::MoveWithScryer
    } else {
        request.mode
    }
}

/// The shared per-title planner's request, filled with the facts a root-scoped
/// operation has (D1).
///
/// The mode is deliberately **not** the caller's: FR-020's "files are already
/// there" applies to the root, not to a title, and the per-title planner reads
/// `mode` only to decide whether to take US3's adoption path — which a
/// root-scoped operation never does. The mode the *plan* carries is still
/// [`execution_mode_for`]'s.
fn shared_plan_request(request: &RootScopePlanRequest) -> RootMovePlanRequest {
    RootMovePlanRequest {
        source_library_id: Some(request.library_id.clone()),
        destination_library_id: Some(request.library_id.clone()),
        source_root_id: Some(request.root_id.clone()),
        destination_root_id: Some(request.destination_root_id().to_string()),
        selection: Vec::new(),
        titles: Vec::new(),
        mode: LocationExecutionMode::MoveWithScryer,
        classification: ClassificationCounts::default(),
        verification_depth: request.verification_depth,
        free_space: request.free_space.clone(),
        case_rule: request.case_rule,
        naming: request.naming.clone(),
    }
}

/// One root-scoped title as the shared planner sees it (D1).
///
/// A root-scope title *is* a root move: one library on both sides, so no facet
/// conversion, no association facts, no adoption, and no library-transfer
/// statement. The one thing the root-scope layer decides for itself is the
/// destination folder — re-anchored from the source root for a path change
/// (FR-026), or the folder [`resolve_root_scope_folders`] settled on for a fold
/// (FR-025, FR-063).
fn root_move_draft(
    request: &RootScopePlanRequest,
    draft: &RootScopeTitleDraft,
) -> RootMoveTitleDraft {
    RootMoveTitleDraft {
        title_id: draft.title_id.clone(),
        title_name: draft.title_name.clone(),
        class: draft.outcome().class(),
        source_library_id: request.library_id.clone(),
        source_root_id: request.root_id.clone(),
        source_root_path: Some(request.source_root_path.clone()),
        source_folder_path: draft.source_folder_path.clone(),
        destination_library_id: request.library_id.clone(),
        destination_root_id: request.destination_root_id().to_string(),
        destination_root_path: Some(request.destination_root_path.clone()),
        destination_folder_path: destination_folder_for(request, draft),
        files: draft.files.clone(),
        source_directories: draft.source_directories.clone(),
        same_volume: request.same_volume,
        hardlinks: draft.hardlinks.clone(),
        destination_entries: draft.destination_entries.clone(),
        recycle: draft.recycle.clone(),
        blocked_reason: draft.blocked_reason.clone(),
        destination_identity: draft.destination_identity.clone(),
        facet_conversion: None,
        associations: TitleAssociationFacts::default(),
        merge_summary: draft.merge_summary.clone(),
        adoption: None,
    }
}

/// Where this title's folder lands, by root-scope rules rather than by the
/// naming policy.
///
/// A fold has already resolved it against the destination root's occupancy
/// (FR-025, FR-063). A change to an unconfigured path has nothing to resolve
/// against: FR-026 preserves the source root's relative layout, so the answer
/// is the source folder re-anchored onto the new path. A folder outside the
/// root cannot be re-anchored; [`classify_root_content`] already treats such
/// content as unexplained, and the folder's own basename directly under the
/// destination root is the only defensible place left.
fn destination_folder_for(
    request: &RootScopePlanRequest,
    draft: &RootScopeTitleDraft,
) -> Option<PathBuf> {
    if let Some(resolved) = draft.resolved.as_ref() {
        return resolved.destination_folder.clone();
    }
    let source_folder = draft.source_folder_path.as_ref()?;
    Some(
        rebase(
            source_folder,
            &request.source_root_path,
            &request.destination_root_path,
        )
        .unwrap_or_else(|| {
            request
                .destination_root_path
                .join(file_name_or(source_folder, &draft.title_id))
        }),
    )
}

/// The root-scope statements the shared planner cannot make, and the wording it
/// states in the move workflow's voice rather than this one's.
///
/// Three things are genuinely root-scoped and have to be added here: FR-024's
/// placement statement (1–3), the merge statement — which the shared planner
/// only makes for a title that also changes *library* (FR-056) — and FR-055's
/// same-name line. Two are the same decision under a different name and are
/// restated in place: a blocked title's reason code names the branch it blocks
/// (FR-023), and a folder rename here is FR-025's uniquing rather than FR-013's
/// naming-policy repair.
fn restate_for_root_scope(
    request: &RootScopePlanRequest,
    draft: &RootScopeTitleDraft,
    items: Vec<PlanItem>,
) -> (Vec<PlanItem>, Vec<String>) {
    let folds = request.variant.folds_into_existing_root();
    let mut warnings: Vec<String> = Vec::new();
    let mut restated: Vec<PlanItem> = Vec::new();

    for mut item in items {
        match item.reason_code.as_deref() {
            Some(crate::location::root_move::plan_reasons::CATALOG_ONLY_REASSIGNMENT) => {
                item = item
                    .with_reason_code(if folds {
                        plan_reasons::CATALOG_ONLY_CONSOLIDATION
                    } else {
                        plan_reasons::CATALOG_ONLY_ROOT_CHANGE
                    })
                    .with_detail(format!(
                        "\"{}\" owns no folder, so only its stored root {} changes",
                        draft.title_name,
                        if folds { "reference" } else { "path" }
                    ));
            }
            Some(crate::location::root_move::plan_reasons::FOLDER_NAME_REPAIR) => {
                let Some(collided_name) = draft.collided_name() else {
                    // FR-063: a merge target keeps the folder it already owns.
                    // The merge statement says so, and a second line calling it
                    // a rename would contradict it.
                    continue;
                };
                let occupier = draft
                    .resolved
                    .as_ref()
                    .and_then(|resolved| resolved.occupied_by_title_id.as_deref())
                    .map(|title_id| format!("an unrelated title ({title_id})"))
                    .unwrap_or_else(|| "content that no title on this library owns".to_string());
                let landed = item
                    .destination_path
                    .as_deref()
                    .map(|path| file_name_or(Path::new(path), &draft.title_id))
                    .unwrap_or_else(|| draft.title_id.clone());
                item = item
                    .with_reason_code(plan_reasons::FOLDER_NAME_UNIQUED)
                    .with_detail(format!(
                        "\"{collided_name}\" at the destination root already belongs to {occupier}, and an identical folder name is not evidence of an identical title; \"{}\" is previewed into \"{landed}\" instead",
                        draft.title_name
                    ));
            }
            // FR-023: the shared planner leaves a blocked title's reason code
            // open, because only the caller knows which branch it blocks.
            None if item.kind == PlanItemKind::Blocked => {
                item =
                    item.with_reason_code(draft.blocked_reason_code.clone().unwrap_or_else(|| {
                        if folds {
                            plan_reasons::TITLE_BLOCKED_FOR_CONSOLIDATION.to_string()
                        } else {
                            plan_reasons::TITLE_BLOCKED_FOR_ROOT_CHANGE.to_string()
                        }
                    }));
                if draft.blocked_reason.is_none() {
                    item = item.with_detail(format!(
                        "\"{}\" needs a repair before it can move",
                        draft.title_name
                    ));
                }
            }
            _ => {}
        }
        restated.push(item);
    }

    if draft.outcome() != RootScopeTitleOutcome::Relocates {
        return (restated, warnings);
    }

    // FR-024's first three classifications, stated on the plan the user reads.
    // The collision case states itself: it is the restated folder rename above.
    let mut leading: Vec<PlanItem> = Vec::new();
    let source_folder_display = draft
        .source_folder_path
        .as_deref()
        .map(path_to_stored_string);
    let destination_folder_display = destination_folder_for(request, draft)
        .as_deref()
        .map(path_to_stored_string);
    match (draft.merge_target(), draft.collided_name()) {
        (Some(destination_title_id), _) => {
            let named = draft
                .destination_identity
                .as_ref()
                .and_then(DestinationIdentityOutcome::merge_target_title_name)
                .map(|name| format!("\"{name}\" ({destination_title_id})"))
                .unwrap_or_else(|| destination_title_id.to_string());
            leading.push(
                PlanItem::new(PlanItemKind::Merge)
                    .with_title(draft.title_id.clone())
                    .with_paths(
                        source_folder_display.clone(),
                        destination_folder_display.clone(),
                    )
                    .with_reason_code(plan_reasons::MERGES_WITH_DESTINATION_TITLE)
                    .with_detail(format!(
                        "\"{}\" shares a metadata identity with {named} on the destination root, so the two merge; the destination title keeps its id, settings, monitoring, naming, and its folder, and this title's media file records and history are carried onto it",
                        draft.title_name
                    )),
            );
            leading.extend(merge_summary_items(
                &draft.title_id,
                &draft.title_name,
                draft.merge_summary.as_ref(),
            ));
        }
        (None, Some(_)) => {}
        (None, None) if folds => {
            leading.push(
                PlanItem::new(PlanItemKind::CatalogChange)
                    .with_title(draft.title_id.clone())
                    .with_paths(source_folder_display, destination_folder_display)
                    .with_reason_code(plan_reasons::MOVES_INTO_UNUSED_FOLDER)
                    .with_detail(format!(
                        "\"{}\" moves into an unused folder on the destination root; its folder layout is preserved",
                        draft.title_name
                    )),
            );
        }
        (None, None) => {}
    }

    // FR-055's same-name statement still earns its place: it is the sentence
    // that explains why two same-named titles are about to sit side by side.
    if let Some(warning) = same_named_destination_warning(
        draft.destination_identity.as_ref(),
        &draft.title_name,
        "root",
    ) {
        leading.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::FOLDER_NAME_UNIQUED)
                .with_detail(warning.clone()),
        );
        warnings.push(warning);
    }

    leading.extend(restated);
    (leading, warnings)
}

/// FR-024's seven counts, folded off the plan the shared planner produced.
///
/// Counted from the items the user reads rather than tallied beside them, so
/// the seven numbers and the plan cannot disagree (SC-004). The three
/// title-scoped counts come from the same two facts the placement statements
/// do; the three file-scoped ones are the dedup and rename items themselves,
/// split by whether they act on tracked media (FR-075); the seventh is the
/// content inventory's own bucket (FR-027).
fn classification_from(
    request: &RootScopePlanRequest,
    accounting: &TitleAccounting,
    content: &RootContentInventory,
    title_items: &[PlanItem],
) -> RootScopeClassification {
    let mut classification = RootScopeClassification {
        catalog_only: accounting.catalog_only,
        blocked: accounting.blocked,
        untracked_source_entries: content.unknown.len() as i64,
        ..RootScopeClassification::default()
    };

    for draft in &request.titles {
        if draft.outcome() != RootScopeTitleOutcome::Relocates {
            continue;
        }
        match (draft.merge_target(), draft.collided_name()) {
            (Some(_), _) => classification.merging_with_destination_titles += 1,
            (None, Some(_)) => classification.folder_name_collisions += 1,
            (None, None) => classification.moving_into_unused_folders += 1,
        }
    }

    for item in title_items {
        match item.kind {
            PlanItemKind::Dedup => classification.dedup_eligible_files += 1,
            // A folder-scoped rename is FR-025's uniquing, which is already
            // counted as a title; only a file's rename is FR-024 (4) or (6).
            PlanItemKind::Rename if !item.folder_scoped => {
                if item.media_file_id.is_some() {
                    classification.media_collisions += 1;
                } else {
                    classification.companion_collisions += 1;
                }
            }
            _ => {}
        }
    }

    classification
}

/// The one helper set the three planner test modules share.
///
/// The two variants differ in where their titles land, not in what a title is,
/// so one `title`, one `tracked`, and one `blocked` serve both — and a fixture
/// that drifts drifts for both at once.
#[cfg(test)]
mod plan_test_support {
    use super::*;

    use crate::location::collisions::FullHash;
    use crate::location::identity::{DestinationIdentityOutcome, IdentityCandidate};
    use crate::location::merge::DestinationIdentityMatch;
    use crate::location::preview::{PlanConfirmationError, PlanConfirmationRequest};

    pub(super) const SOURCE_ROOT: &str = "/media/old";
    /// A path that is not a configured root: the ChangePath destination.
    pub(super) const NEW_ROOT: &str = "/media/new";
    /// Another configured root of the same library: the FoldInto destination.
    pub(super) const KEEP_ROOT: &str = "/media/keep";

    pub(super) fn tracked(path: &str, size_bytes: u64) -> SourceFile {
        SourceFile {
            media_file_id: Some(format!("file-{path}")),
            full_blake3: FullHash::Absent,
            path: PathBuf::from(path),
            relative_path: None,
            size_bytes,
        }
    }

    pub(super) fn companion(path: &str, size_bytes: u64) -> SourceFile {
        SourceFile {
            media_file_id: None,
            full_blake3: FullHash::Absent,
            path: PathBuf::from(path),
            relative_path: None,
            size_bytes,
        }
    }

    /// A relocating title, with each file's folder-relative position derived
    /// the way [`crate::location::operations::collect_source_files`] derives it
    /// — a file outside the folder keeps `None`, which is the case the planner
    /// warns about.
    pub(super) fn title(id: &str, folder: &str, files: Vec<SourceFile>) -> RootScopeTitleDraft {
        let folder = PathBuf::from(folder);
        let files = files
            .into_iter()
            .map(|mut file| {
                file.relative_path = file.path.strip_prefix(&folder).ok().map(Path::to_path_buf);
                file
            })
            .collect();
        RootScopeTitleDraft {
            source_folder_path: Some(folder),
            files,
            ..RootScopeTitleDraft::new(id, id)
        }
    }

    /// The same title, with the folder a fold resolved for it.
    pub(super) fn folded(
        id: &str,
        folder: &str,
        files: Vec<SourceFile>,
        resolved: ResolvedFolder,
    ) -> RootScopeTitleDraft {
        RootScopeTitleDraft {
            resolved: Some(resolved),
            ..title(id, folder, files)
        }
    }

    pub(super) fn fileless(id: &str) -> RootScopeTitleDraft {
        RootScopeTitleDraft::new(id, id)
    }

    pub(super) fn blocked(id: &str, folder: &str, reason: &str) -> RootScopeTitleDraft {
        RootScopeTitleDraft {
            blocked_reason: Some(reason.to_string()),
            blocked_reason_code: Some("active_download_or_import".to_string()),
            ..title(id, folder, Vec::new())
        }
    }

    pub(super) fn resolved_unused(id: &str, folder: &str) -> ResolvedFolder {
        ResolvedFolder {
            title_id: id.to_string(),
            destination_folder: Some(PathBuf::from(folder)),
            collided_name: None,
            occupied_by_title_id: None,
            renamed_to: None,
        }
    }

    pub(super) fn resolved_uniqued(
        id: &str,
        folder: &str,
        collided_name: &str,
        occupied_by_title_id: Option<&str>,
    ) -> ResolvedFolder {
        ResolvedFolder {
            title_id: id.to_string(),
            destination_folder: Some(PathBuf::from(folder)),
            collided_name: Some(collided_name.to_string()),
            occupied_by_title_id: occupied_by_title_id.map(str::to_string),
            renamed_to: PathBuf::from(folder)
                .file_name()
                .map(|name| name.to_string_lossy().to_string()),
        }
    }

    /// The detection outcome that makes a title a merge candidate (FR-055).
    pub(super) fn merge_identity(title_id: &str, title_name: &str) -> DestinationIdentityOutcome {
        DestinationIdentityOutcome {
            match_kind: DestinationIdentityMatch::Unique,
            matched_title_id: Some(title_id.to_string()),
            candidates: vec![IdentityCandidate {
                title_id: title_id.to_string(),
                title_name: title_name.to_string(),
                shared_identities: Vec::new(),
            }],
            same_name_title_id: None,
            same_name_title_name: None,
        }
    }

    /// US4: a change to an unconfigured path.
    pub(super) fn change_request(titles: Vec<RootScopeTitleDraft>) -> RootScopePlanRequest {
        RootScopePlanRequest {
            library_id: "library-1".to_string(),
            root_id: "root-1".to_string(),
            source_root_path: PathBuf::from(SOURCE_ROOT),
            destination_root_path: PathBuf::from(NEW_ROOT),
            variant: RootScopeVariant::ChangePath {
                retention: RootRetentionFacts::default(),
            },
            mode: LocationExecutionMode::MoveWithScryer,
            titles,
            entries: Vec::new(),
            verification_depth: VerificationDepth::default(),
            free_space: FreeSpaceEstimate::unknown(),
            same_volume: Some(false),
            case_rule: PathCaseRule::CaseSensitive,
            naming: CollisionNaming::from_source_library("Old Disk"),
        }
    }

    /// US5: a fold into another configured root of the same library.
    pub(super) fn fold_request(titles: Vec<RootScopeTitleDraft>) -> RootScopePlanRequest {
        RootScopePlanRequest {
            root_id: "root-old".to_string(),
            destination_root_path: PathBuf::from(KEEP_ROOT),
            variant: RootScopeVariant::FoldInto {
                destination_root_id: "root-keep".to_string(),
                default_transfer: DefaultRootTransfer::default(),
            },
            verification_depth: VerificationDepth::Full,
            ..change_request(titles)
        }
    }

    pub(super) fn resolution_title(id: &str, folder: Option<&str>) -> FolderResolutionTitle {
        FolderResolutionTitle {
            title_id: id.to_string(),
            title_name: id.to_string(),
            source_folder_path: folder.map(PathBuf::from),
            merge_target_title_id: None,
            merge_target_title_name: None,
            merge_target_folder_path: None,
        }
    }

    /// A folder-resolution request. `occupants` names the destination paths
    /// that are not free, with the destination title that owns each when one
    /// does; anything not named is free.
    pub(super) fn resolution(
        titles: Vec<FolderResolutionTitle>,
        occupants: Vec<(&str, Option<&str>)>,
    ) -> FolderResolutionRequest {
        FolderResolutionRequest {
            source_root: PathBuf::from(SOURCE_ROOT),
            destination_root: PathBuf::from(KEEP_ROOT),
            case_rule: PathCaseRule::CaseSensitive,
            naming: CollisionNaming::from_source_library("Old Disk"),
            titles,
            destination_occupants: occupants
                .into_iter()
                .map(|(path, owner)| (path.to_string(), owner.map(str::to_string)))
                .collect(),
        }
    }

    pub(super) fn confirm(
        planned: &PlannedRootScope,
        phrase: Option<&str>,
    ) -> Result<(), PlanConfirmationError> {
        planned.plan.confirm(&PlanConfirmationRequest {
            fingerprint: planned.plan.fingerprint.clone(),
            typed_confirmation: phrase.map(str::to_string),
        })
    }

    /// Every plan item of `kind`, whatever section it landed in.
    pub(super) fn items_of(planned: &PlannedRootScope, kind: PlanItemKind) -> Vec<&PlanItem> {
        planned
            .plan
            .section(kind)
            .map(|section| section.items.items.iter().collect())
            .unwrap_or_default()
    }

    /// The details of every plan item of `kind` carrying `reason_code`.
    pub(super) fn details_for<'a>(
        planned: &'a PlannedRootScope,
        kind: PlanItemKind,
        reason_code: &str,
    ) -> Vec<&'a str> {
        items_of(planned, kind)
            .into_iter()
            .filter(|item| item.reason_code.as_deref() == Some(reason_code))
            .filter_map(|item| item.detail.as_deref())
            .collect()
    }
}

#[cfg(test)]
mod change_path_tests {
    use super::plan_test_support::*;
    use super::*;

    use crate::location::hardlinks::LinkCount;
    use crate::location::preview::{LOCATION_TYPED_CONFIRMATION_PHRASE, PlanConfirmationError};

    const DESTINATION_ROOT: &str = NEW_ROOT;

    fn request(titles: Vec<RootScopeTitleDraft>) -> RootScopePlanRequest {
        change_request(titles)
    }

    // ── US4 scenario 1: every title accounted for, none excluded ─────────────

    #[test]
    fn every_assigned_title_is_accounted_for_with_no_exclusions() {
        let planned = build_root_scope_plan(&request(vec![
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
        let planned = build_root_scope_plan(&request(vec![
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
        plan_request.variant = RootScopeVariant::ChangePath {
            retention: RootRetentionFacts {
                is_library_default: true,
                role: Some("primary".to_string()),
            },
        };

        let planned = build_root_scope_plan(&plan_request);

        assert!(planned.retention.keeps_root_id);
        assert_eq!(planned.retention.root_id, "root-1");
        assert!(planned.retention.remains_library_default);
        assert_eq!(planned.retention.retained_role.as_deref(), Some("primary"));
        assert_eq!(planned.retention.retained_title_assignments, 2);
    }

    #[test]
    fn the_execution_plan_carries_one_root_id_on_both_sides() {
        let planned = build_root_scope_plan(&request(vec![
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
            assert_eq!(
                title.destination_root_path.as_deref(),
                Some(DESTINATION_ROOT)
            );
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

        let planned = build_root_scope_plan(&plan_request);

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
        let before = build_root_scope_plan(&plan_request);

        plan_request
            .entries
            .push(RootEntry::file("/media/old/appeared.iso", 9));
        let after = build_root_scope_plan(&plan_request);

        assert_ne!(before.plan.fingerprint, after.plan.fingerprint);
    }

    // ── US4 scenario 4: empty-directory-only cleanup ─────────────────────────

    #[test]
    fn cleanup_facts_are_empty_directories_only_and_gated_on_verification() {
        let mut plan_request = request(vec![RootScopeTitleDraft {
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

        let planned = build_root_scope_plan(&plan_request);

        assert!(planned.retirement.empty_directories_only);
        assert!(
            planned
                .retirement
                .requires_verification_before_source_removal
        );
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
        let planned = build_root_scope_plan(&request(vec![title(
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
        assert_eq!(
            confirm(&planned, Some(LOCATION_TYPED_CONFIRMATION_PHRASE)),
            Ok(())
        );
    }

    // ── Retirement ordering (FR-087) ─────────────────────────────────────────

    #[test]
    fn the_configuration_is_retired_only_after_recycling_completes() {
        let planned = build_root_scope_plan(&request(vec![title(
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
        let planned = build_root_scope_plan(&request(vec![title(
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

    // ── Layout preservation (FR-026), and D1's adapter onto it ──────────────

    /// D1 for a path change: the destination folder is the source folder
    /// re-anchored from the source root onto the new path, however deeply the
    /// folder is nested — and every file inside it keeps its position, because
    /// the shared planner lays a file down relative to the folder it was given.
    #[test]
    fn nested_folders_and_files_keep_their_root_relative_layout() {
        let planned = build_root_scope_plan(&request(vec![title(
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

    /// D2: a tracked file outside its title's folder follows the shared rule
    /// everywhere — it lands in the destination folder's root, and the preview
    /// says so. The root-relative special case a root change used to have is
    /// gone, so one planner answers this for every workflow.
    #[test]
    fn a_tracked_file_outside_its_title_folder_lands_in_the_destination_folder() {
        let planned = build_root_scope_plan(&request(vec![title(
            "movie-a",
            "/media/old/Movie A",
            vec![tracked("/media/old/loose/a-extra.mkv", 5)],
        )]));

        let title = &planned.execution.titles[0];
        assert_eq!(
            title.files[0].destination_path,
            "/media/new/Movie A/a-extra.mkv"
        );
        assert!(
            items_of(&planned, PlanItemKind::Warning)
                .iter()
                .any(|item| item.reason_code.as_deref()
                    == Some(crate::location::root_move::plan_reasons::FILE_OUTSIDE_TITLE_FOLDER))
        );
    }

    // ── Mode, catalog-only titles, and warnings ──────────────────────────────

    #[test]
    fn a_root_change_with_nothing_to_move_needs_no_move_mode() {
        let planned = build_root_scope_plan(&request(vec![fileless("movie-c")]));

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
        plan_request.variant = RootScopeVariant::ChangePath {
            retention: RootRetentionFacts {
                is_library_default: true,
                role: None,
            },
        };

        let planned = build_root_scope_plan(&plan_request);

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
        assert_eq!(
            statement.destination_path.as_deref(),
            Some(DESTINATION_ROOT)
        );
    }

    #[test]
    fn a_cross_volume_root_change_warns_about_hardlinked_sources() {
        let planned = build_root_scope_plan(&request(vec![RootScopeTitleDraft {
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

#[cfg(test)]
mod fold_tests {
    use super::plan_test_support::*;
    use super::*;

    use crate::location::collisions::{ContentFacts, FullHash};
    use crate::location::identity::{DestinationIdentityOutcome, IdentityCandidate};
    use crate::location::merge::DestinationIdentityMatch;

    const DESTINATION_ROOT: &str = KEEP_ROOT;

    fn request(titles: Vec<RootScopeTitleDraft>) -> RootScopePlanRequest {
        fold_request(titles)
    }

    fn facts() -> RootScopePathFacts {
        RootScopePathFacts {
            variant: RootScopePathVariant::FoldInto {
                source_root_id: "root-old".to_string(),
                destination_root_id: "root-keep".to_string(),
            },
            source_root: PathBuf::from(SOURCE_ROOT),
            destination_root: PathBuf::from(DESTINATION_ROOT),
            source_root_is_symlink: false,
            source_root_is_directory: true,
            destination: DestinationPathState::Directory { empty: false },
            mode: LocationExecutionMode::MoveWithScryer,
        }
    }

    /// The fold branch's ids, for a test that changes one of them.
    fn fold_ids(facts: &mut RootScopePathFacts) -> (&mut String, &mut String) {
        match &mut facts.variant {
            RootScopePathVariant::FoldInto {
                source_root_id,
                destination_root_id,
            } => (source_root_id, destination_root_id),
            RootScopePathVariant::ChangePath { .. } => unreachable!("a fold fixture"),
        }
    }

    // ── Admissibility (FR-020) ───────────────────────────────────────────────

    #[test]
    fn a_root_cannot_be_consolidated_into_itself() {
        let mut facts = facts();
        let (source, destination) = fold_ids(&mut facts);
        *destination = source.clone();
        assert_eq!(
            check_root_scope_paths(&facts).expect_err("same root").code,
            refusal_codes::FOLD_SAME_ROOT
        );
    }

    #[test]
    fn files_already_there_is_not_a_consolidation_mode() {
        let mut facts = facts();
        facts.mode = LocationExecutionMode::FilesAlreadyThere;
        let refusal = check_root_scope_paths(&facts).expect_err("mode refused");
        assert_eq!(refusal.code, refusal_codes::FOLD.mode_not_supported);
    }

    #[test]
    fn overlapping_roots_and_unreadable_roots_are_refused() {
        let mut nested = facts();
        nested.destination_root = PathBuf::from("/media/old/inner");
        assert_eq!(
            check_root_scope_paths(&nested).expect_err("overlap").code,
            refusal_codes::FOLD.paths_overlap
        );

        let mut symlinked = facts();
        symlinked.source_root_is_symlink = true;
        assert_eq!(
            check_root_scope_paths(&symlinked)
                .expect_err("symlink")
                .code,
            refusal_codes::FOLD.source_root_is_symlink
        );

        let mut unreadable = facts();
        unreadable.destination = DestinationPathState::NotADirectory;
        assert_eq!(
            check_root_scope_paths(&unreadable)
                .expect_err("unreadable destination")
                .code,
            refusal_codes::FOLD_DESTINATION_ROOT_UNAVAILABLE
        );

        assert!(check_root_scope_paths(&facts()).is_ok());
    }

    // ── FR-025/FR-026 folder resolution ──────────────────────────────────────

    #[test]
    fn a_free_destination_folder_preserves_the_source_root_relative_layout() {
        let resolved = resolve_root_scope_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Shows/Series A"))],
            Vec::new(),
        ));
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Shows/Series A")),
            "FR-026 preserves the whole relative position, however deeply nested"
        );
        assert!(resolved[0].collided_name.is_none());
        assert!(resolved[0].renamed_to.is_none());
    }

    #[test]
    fn an_empty_destination_directory_is_not_a_collision() {
        let resolved = resolve_root_scope_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Movie (2020)"))],
            // An empty directory nothing owns is free, so it is not named.
            Vec::new(),
        ));
        assert!(resolved[0].collided_name.is_none());
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Movie (2020)"))
        );
    }

    /// FR-025: two unrelated titles calculating the same destination folder never
    /// merge over the name — the incoming folder gets a unique previewed one.
    #[test]
    fn an_unrelated_title_owning_the_name_gets_the_incoming_folder_uniqued() {
        let resolved = resolve_root_scope_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Movie (2020)"))],
            vec![("/media/keep/Movie (2020)", Some("other"))],
        ));
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Movie (2020) (from Old Disk)"))
        );
        assert_eq!(resolved[0].collided_name.as_deref(), Some("Movie (2020)"));
        assert_eq!(resolved[0].occupied_by_title_id.as_deref(), Some("other"));
        assert_eq!(
            resolved[0].renamed_to.as_deref(),
            Some("Movie (2020) (from Old Disk)"),
            "US5.4: the changed folder name is previewed"
        );
    }

    #[test]
    fn untracked_destination_content_also_forces_a_unique_name() {
        let resolved = resolve_root_scope_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Movie (2020)"))],
            vec![("/media/keep/Movie (2020)", None)],
        ));
        assert_eq!(resolved[0].collided_name.as_deref(), Some("Movie (2020)"));
        assert!(resolved[0].occupied_by_title_id.is_none());
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Movie (2020) (from Old Disk)"))
        );
    }

    #[test]
    fn a_uniqued_name_that_is_also_taken_is_numbered() {
        let resolved = resolve_root_scope_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Movie (2020)"))],
            vec![
                ("/media/keep/Movie (2020)", None),
                ("/media/keep/Movie (2020) (from Old Disk)", None),
            ],
        ));
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Movie (2020) (from Old Disk) (2)"))
        );
    }

    #[test]
    fn a_merge_target_keeps_the_folder_it_already_owns() {
        let mut title = resolution_title("t1", Some("/media/old/Movie (2020)"));
        title.merge_target_title_id = Some("dest".to_string());
        title.merge_target_title_name = Some("Movie".to_string());
        title.merge_target_folder_path = Some(PathBuf::from("/media/keep/Movie 2020"));

        let resolved = resolve_root_scope_folders(&resolution(
            vec![title],
            vec![("/media/keep/Movie 2020", Some("dest"))],
        ));
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Movie 2020")),
            "FR-063: the destination title keeps the folder it already has"
        );
        assert!(
            resolved[0].collided_name.is_none(),
            "a merge target's own folder is not a collision"
        );
        assert_eq!(resolved[0].renamed_to.as_deref(), Some("Movie 2020"));
    }

    #[test]
    fn a_case_insensitive_destination_treats_two_spellings_as_one_name() {
        let mut request = resolution(
            vec![
                resolution_title("t1", Some("/media/old/Movie (2020)")),
                resolution_title("t2", Some("/media/old/nested/MOVIE (2020)")),
            ],
            Vec::new(),
        );
        request.case_rule = PathCaseRule::CaseInsensitive;
        // Both would land under different parents, so nothing collides: the
        // point of this case is that the folded claim set does not over-reach.
        let resolved = resolve_root_scope_folders(&request);
        assert!(resolved[0].collided_name.is_none());
        assert!(resolved[1].collided_name.is_none());

        // Two titles whose folders differ only by case *do* collide on a
        // case-insensitive destination (FR-090).
        let mut request = resolution(
            vec![
                resolution_title("t1", Some("/media/old/Movie (2020)")),
                resolution_title("t2", Some("/media/old/MOVIE (2020)")),
            ],
            Vec::new(),
        );
        request.case_rule = PathCaseRule::CaseInsensitive;
        let resolved = resolve_root_scope_folders(&request);
        assert!(resolved[0].collided_name.is_none());
        assert!(
            resolved[1].collided_name.is_some(),
            "the second title cannot take a name the first already claimed"
        );
    }

    #[test]
    fn a_fileless_title_resolves_to_no_folder() {
        let resolved =
            resolve_root_scope_folders(&resolution(vec![resolution_title("t1", None)], Vec::new()));
        assert!(resolved[0].destination_folder.is_none());
        assert!(resolved[0].collided_name.is_none());
    }

    // ── FR-023/FR-024 accounting and classification ──────────────────────────

    #[test]
    fn every_assigned_title_is_accounted_for_and_the_seven_classifications_close() {
        let moving = folded(
            "t1",
            "/media/old/Alpha (2020)",
            vec![tracked("/media/old/Alpha (2020)/Alpha.mkv", 100)],
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        );
        // FR-024 (4): a media file whose name is taken at the destination.
        // FR-024 (6): a sidecar in the same position, counted apart from it.
        let mut merging = folded(
            "t2",
            "/media/old/Beta (2021)",
            vec![
                tracked("/media/old/Beta (2021)/Beta.mkv", 200),
                companion("/media/old/Beta (2021)/Beta.srt", 2),
            ],
            ResolvedFolder {
                renamed_to: Some("Beta".to_string()),
                ..resolved_unused("t2", "/media/keep/Beta")
            },
        );
        merging.destination_identity = Some(merge_identity("dest-2", "Beta"));
        merging.destination_entries = vec![
            DestinationItem::media("Beta.mkv", 999),
            DestinationItem::companion("Beta.srt", 3),
        ];
        // FR-024 (5) + FR-073: a file proven identical by full BLAKE3 on both
        // sides is dedup-eligible — no bytes written, the source copy recycled.
        let mut duplicate = tracked("/media/old/Gamma (2022)/Gamma.mkv", 300);
        duplicate.full_blake3 = FullHash::known("abc123");
        let mut colliding = folded(
            "t3",
            "/media/old/Gamma (2022)",
            vec![duplicate],
            resolved_uniqued(
                "t3",
                "/media/keep/Gamma (2022) (from Old Disk)",
                "Gamma (2022)",
                Some("dest-3"),
            ),
        );
        colliding.destination_entries = vec![
            DestinationItem::media("Gamma.mkv", 300)
                .with_content(ContentFacts::new(300).with_full_blake3("abc123")),
        ];
        let fileless = super::plan_test_support::fileless("t4");
        let blocked = RootScopeTitleDraft {
            blocked_reason: Some("an import is running".to_string()),
            blocked_reason_code: Some("active_download_or_import".to_string()),
            ..folded(
                "t5",
                "/media/old/Delta (2023)",
                Vec::new(),
                resolved_unused("t5", "/media/keep/Delta (2023)"),
            )
        };

        let mut plan_request = request(vec![moving, merging, colliding, fileless, blocked]);
        plan_request.entries = vec![
            RootEntry::file("/media/old/Alpha (2020)/Alpha.mkv", 100),
            RootEntry::file("/media/old/stray.txt", 7),
        ];
        let planned = build_root_scope_plan(&plan_request);

        assert_eq!(planned.accounting.assigned_total, 5);
        assert!(
            planned.accounting.accounts_for_every_title(),
            "the FR-023 ledger has to close"
        );
        assert!(
            planned
                .classification
                .accounts_for(planned.accounting.assigned_total),
            "every title lands in exactly one FR-024 title-scoped bucket"
        );
        assert_eq!(planned.classification.moving_into_unused_folders, 1);
        assert_eq!(planned.classification.merging_with_destination_titles, 1);
        assert_eq!(planned.classification.folder_name_collisions, 1);
        assert_eq!(planned.classification.catalog_only, 1);
        assert_eq!(planned.classification.blocked, 1);
        assert_eq!(planned.classification.untracked_source_entries, 1);

        // FR-072/074/075: a renamed media file, a renamed sidecar beside it,
        // and a proven duplicate are three separate counts.
        assert_eq!(planned.classification.media_collisions, 1);
        assert_eq!(planned.classification.companion_collisions, 1);
        assert_eq!(planned.classification.dedup_eligible_files, 1);

        let merged = planned
            .execution
            .title("t2")
            .expect("the merging title has instructions");
        assert_eq!(merged.merge_target_title_id.as_deref(), Some("dest-2"));
        assert_eq!(merged.renamed_destinations.len(), 2);
        assert!(
            merged.files[0]
                .destination_path
                .starts_with("/media/keep/Beta/"),
            "a merging title's content lands in the destination title's folder: {}",
            merged.files[0].destination_path
        );

        let deduplicated = planned
            .execution
            .title("t3")
            .expect("the colliding title has instructions");
        assert_eq!(deduplicated.deduplicated_sources.len(), 1);
        assert!(
            deduplicated.files.is_empty(),
            "a proven duplicate writes no bytes"
        );
    }

    // ── D1: the adapter onto the shared per-title planner ────────────────────

    /// FR-025/FR-063: a fold plans against the folder resolution settled on,
    /// not against the source root's layout — and a merge target keeps the
    /// folder it already owns, with no rename line contradicting the merge.
    #[test]
    fn a_fold_plans_against_the_resolved_folder_and_a_merge_target_keeps_its_own() {
        let uniqued = folded(
            "t1",
            "/media/old/Alpha (2020)",
            vec![tracked("/media/old/Alpha (2020)/Alpha.mkv", 10)],
            resolved_uniqued(
                "t1",
                "/media/keep/Alpha (2020) (from Old Disk)",
                "Alpha (2020)",
                Some("other"),
            ),
        );
        let mut merging = folded(
            "t2",
            "/media/old/Beta (2021)",
            vec![tracked("/media/old/Beta (2021)/Sub/Beta.mkv", 20)],
            ResolvedFolder {
                renamed_to: Some("Beta".to_string()),
                ..resolved_unused("t2", "/media/keep/Beta")
            },
        );
        merging.destination_identity = Some(merge_identity("dest-2", "Beta"));

        let planned = build_root_scope_plan(&request(vec![uniqued, merging]));

        let first = planned.execution.title("t1").expect("instructions");
        assert_eq!(
            first.destination_folder_path.as_deref(),
            Some("/media/keep/Alpha (2020) (from Old Disk)")
        );
        assert_eq!(
            first.files[0].destination_path,
            "/media/keep/Alpha (2020) (from Old Disk)/Alpha.mkv"
        );
        // FR-025's uniquing, stated as itself rather than as FR-013's
        // naming-policy repair.
        let uniqued_details = details_for(
            &planned,
            PlanItemKind::Rename,
            plan_reasons::FOLDER_NAME_UNIQUED,
        );
        assert_eq!(uniqued_details.len(), 1);
        assert!(uniqued_details[0].contains("an unrelated title (other)"));

        let second = planned.execution.title("t2").expect("instructions");
        assert_eq!(second.merge_target_title_id.as_deref(), Some("dest-2"));
        assert_eq!(
            second.destination_folder_path.as_deref(),
            Some("/media/keep/Beta")
        );
        assert_eq!(
            second.files[0].destination_path, "/media/keep/Beta/Sub/Beta.mkv",
            "the layout inside the folder survives the folder keeping its own name"
        );
        assert_eq!(
            details_for(
                &planned,
                PlanItemKind::Merge,
                plan_reasons::MERGES_WITH_DESTINATION_TITLE
            )
            .len(),
            1
        );
        assert!(
            !items_of(&planned, PlanItemKind::Rename)
                .iter()
                .any(|item| item.folder_scoped && item.title_id.as_deref() == Some("t2")),
            "FR-063: a merge target's folder is not previewed as a rename"
        );
    }

    /// The count the typed confirmation confirms is the number of files, and a
    /// consolidation emits both kinds of `Rename`: one folder uniquing and one
    /// media collision. Only the second is a file.
    #[test]
    fn the_file_count_counts_files_and_not_the_folder_rename_beside_them() {
        let uniqued = folded(
            "t1",
            "/media/old/Alpha (2020)",
            vec![tracked("/media/old/Alpha (2020)/Alpha.mkv", 50)],
            resolved_uniqued(
                "t1",
                "/media/keep/Alpha (2020) (from Old Disk)",
                "Alpha (2020)",
                Some("other"),
            ),
        );
        let mut colliding = folded(
            "t2",
            "/media/old/Beta (2021)",
            vec![tracked("/media/old/Beta (2021)/Beta.mkv", 60)],
            resolved_unused("t2", "/media/keep/Beta (2021)"),
        );
        // The destination folder already holds a file of the same name whose
        // content differs, so FR-074 renames the incoming one.
        colliding.destination_entries = vec![DestinationItem::media("Beta.mkv", 61)];

        let planned = build_root_scope_plan(&request(vec![uniqued, colliding]));
        assert_eq!(
            planned.plan.counts.for_kind(PlanItemKind::Rename),
            2,
            "one folder uniquing and one media collision"
        );
        let file_renames = planned
            .plan
            .section(PlanItemKind::Rename)
            .expect("the renames are previewed")
            .items
            .items
            .iter()
            .filter(|item| !item.folder_scoped)
            .count();
        assert_eq!(file_renames, 1);
        assert_eq!(
            planned.plan.counts.files_total, 2,
            "two files move; the folder rename is not a third"
        );
        assert_eq!(
            planned.plan.counts.files_total,
            planned
                .execution
                .titles
                .iter()
                .map(|title| title.files.len() as i64)
                .sum::<i64>(),
            "the previewed file count is the number of files the runner will walk (SC-004)"
        );
        assert_eq!(
            planned.plan.counts.bytes_total,
            50 + 60,
            "the collided file's bytes count once, not once per item describing it"
        );
    }

    // ── FR-022 ───────────────────────────────────────────────────────────────

    #[test]
    fn consolidating_the_default_root_moves_the_default_to_the_destination() {
        let transfer = DefaultRootTransfer {
            source_was_default: true,
            destination_was_default: false,
        };
        assert!(transfer.destination_becomes_default());
        assert!(transfer.transfers_the_default());

        let mut plan_request = request(vec![folded(
            "t1",
            "/media/old/Alpha (2020)",
            Vec::new(),
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        )]);
        plan_request.variant = RootScopeVariant::FoldInto {
            destination_root_id: "root-keep".to_string(),
            default_transfer: transfer,
        };
        let planned = build_root_scope_plan(&plan_request);
        assert!(planned.default_transfer.destination_becomes_default());
        let stated = planned
            .plan
            .section(PlanItemKind::CatalogChange)
            .expect("the root-level statements are catalog changes")
            .items
            .items
            .iter()
            .any(|item| {
                item.reason_code.as_deref() == Some(plan_reasons::DEFAULT_ROOT_TRANSFERRED)
                    && item
                        .detail
                        .as_deref()
                        .is_some_and(|detail| detail.contains("becomes the default"))
            });
        assert!(stated, "US5.3 is stated before the user confirms");
    }

    #[test]
    fn consolidating_a_non_default_root_leaves_the_default_alone() {
        let transfer = DefaultRootTransfer {
            source_was_default: false,
            destination_was_default: false,
        };
        assert!(!transfer.destination_becomes_default());
        assert!(!transfer.transfers_the_default());

        let transfer = DefaultRootTransfer {
            source_was_default: false,
            destination_was_default: true,
        };
        assert!(
            transfer.destination_becomes_default(),
            "a destination that was already the default stays the default"
        );
        assert!(!transfer.transfers_the_default());
    }

    // ── FR-023/FR-028 ────────────────────────────────────────────────────────
    //
    // One planner answers a blocked title, the typed confirmation, the hardlink
    // warnings, the catalog-only downgrade and the fingerprint the same way on
    // both branches, and `change_path_tests` covers each of them once. What
    // survives here is what the *fold* answers differently: what unexplained
    // source content does to a root that is about to leave the configuration.

    #[test]
    fn unexplained_source_content_keeps_the_source_root_configured_without_stopping_the_move() {
        let mut plan_request = request(vec![folded(
            "t1",
            "/media/old/Alpha (2020)",
            vec![tracked("/media/old/Alpha (2020)/Alpha.mkv", 100)],
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        )]);
        plan_request.entries = vec![
            RootEntry::file("/media/old/Alpha (2020)/Alpha.mkv", 100),
            RootEntry::file("/media/old/someone-elses.txt", 9),
        ];
        let planned = build_root_scope_plan(&plan_request);

        assert_eq!(planned.content.unknown.len(), 1);
        assert_eq!(planned.classification.untracked_source_entries, 1);
        assert!(
            planned
                .retirement
                .blocker(retirement_blockers::UNEXPLAINED_SOURCE_CONTENT)
                .is_some()
        );
        assert!(
            !planned.plan.blocks_start(),
            "unexplained content blocks the root's removal, not the move"
        );
        assert_eq!(
            planned.plan.counts.for_kind(PlanItemKind::UnmanagedContent),
            1
        );
    }

    #[test]
    fn the_plan_carries_two_different_root_ids_in_one_library() {
        let planned = build_root_scope_plan(&request(vec![folded(
            "t1",
            "/media/old/Alpha (2020)",
            Vec::new(),
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        )]));
        assert_eq!(
            planned.plan.header.source_root_id.as_deref(),
            Some("root-old")
        );
        assert_eq!(
            planned.plan.header.destination_root_id.as_deref(),
            Some("root-keep")
        );
        assert_eq!(
            planned.plan.header.source_library_id,
            planned.plan.header.destination_library_id
        );
        let execution = planned.execution.title("t1").expect("instructions");
        assert_eq!(execution.source_root_id, "root-old");
        assert_eq!(execution.destination_root_id, "root-keep");
        assert!(!execution.crosses_libraries());
    }

    #[test]
    fn a_same_named_destination_title_is_named_and_never_merged_into() {
        let mut title = folded(
            "t1",
            "/media/old/Alpha (2020)",
            Vec::new(),
            resolved_uniqued(
                "t1",
                "/media/keep/Alpha (2020) (from Old Disk)",
                "Alpha (2020)",
                Some("dest"),
            ),
        );
        title.destination_identity = Some(DestinationIdentityOutcome {
            match_kind: DestinationIdentityMatch::SameNameNoIdentity,
            matched_title_id: None,
            candidates: Vec::<IdentityCandidate>::new(),
            same_name_title_id: Some("dest".to_string()),
            same_name_title_name: Some("Alpha".to_string()),
        });

        let planned = build_root_scope_plan(&request(vec![title]));
        assert!(
            planned
                .warnings
                .iter()
                .any(|warning| warning.contains("shares no metadata identity")),
            "{:?}",
            planned.warnings
        );
        let execution = planned.execution.title("t1").expect("instructions");
        assert!(
            execution.merge_target_title_id.is_none(),
            "FR-025: a shared name never produces a merge"
        );
    }
}

/// The one thing the two branches of **Change root** do not share: their last
/// step. Everything above is one planner; these are the facts that select
/// between "repoint this root" and "retire this root".
#[cfg(test)]
mod variant_tests {
    use super::plan_test_support::*;
    use super::*;

    fn request(variant: RootScopeVariant) -> RootScopePlanRequest {
        RootScopePlanRequest {
            root_id: "root-old".to_string(),
            variant,
            verification_depth: VerificationDepth::Full,
            ..change_request(vec![folded(
                "movie-a",
                "/media/old/Movie A",
                vec![tracked("/media/old/Movie A/a.mkv", 10)],
                resolved_unused("movie-a", "/media/new/Movie A"),
            )])
        }
    }

    /// FR-021/FR-078: the root is repointed. One id on both sides, the default
    /// stays where it was, and the plan opens by saying so.
    #[test]
    fn change_path_keeps_the_root_id_and_its_default_status() {
        let planned = build_root_scope_plan(&request(RootScopeVariant::ChangePath {
            retention: RootRetentionFacts {
                is_library_default: true,
                role: Some("primary".to_string()),
            },
        }));

        assert_eq!(
            planned.plan.header.operation_type,
            LocationOperationType::RootChange
        );
        assert_eq!(
            planned.plan.header.source_root_id.as_deref(),
            Some("root-old")
        );
        assert_eq!(
            planned.plan.header.destination_root_id.as_deref(),
            Some("root-old"),
            "a path change carries one root id on both sides"
        );
        assert!(planned.retention.keeps_root_id);
        assert!(planned.retention.was_library_default);
        assert!(planned.retention.remains_library_default);
        assert_eq!(planned.retention.retained_role.as_deref(), Some("primary"));
        // FR-022 belongs to the other branch: a path change never moves the
        // default, so there is nothing to transfer.
        assert!(!planned.default_transfer.transfers_the_default());
        assert!(!planned.default_transfer.destination_becomes_default());

        let opening = planned
            .plan
            .section(PlanItemKind::CatalogChange)
            .and_then(|section| section.items.items.first())
            .and_then(|item| item.detail.clone())
            .expect("the root statement leads the plan");
        assert!(opening.contains("keeps its identity"), "{opening}");
        assert!(opening.contains("remains the library default"), "{opening}");
    }

    /// FR-020/FR-022: the source root's configuration is retired, the
    /// destination root is a different root, and the library default follows the
    /// content.
    #[test]
    fn fold_into_retires_the_source_root_and_transfers_the_default() {
        let planned = build_root_scope_plan(&request(RootScopeVariant::FoldInto {
            destination_root_id: "root-keep".to_string(),
            default_transfer: DefaultRootTransfer {
                source_was_default: true,
                destination_was_default: false,
            },
        }));

        assert_eq!(
            planned.plan.header.operation_type,
            LocationOperationType::RootConsolidation
        );
        assert_eq!(
            planned.plan.header.source_root_id.as_deref(),
            Some("root-old")
        );
        assert_eq!(
            planned.plan.header.destination_root_id.as_deref(),
            Some("root-keep"),
            "a fold names a real, different root"
        );
        assert!(planned.default_transfer.transfers_the_default());
        assert!(planned.default_transfer.destination_becomes_default());

        let details: Vec<String> = planned
            .plan
            .section(PlanItemKind::CatalogChange)
            .map(|section| {
                section
                    .items
                    .items
                    .iter()
                    .filter_map(|item| item.detail.clone())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("configuration is retired")),
            "{details:?}"
        );
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("becomes the default")),
            "{details:?}"
        );
        // The retirement contract is the same shape either way; what changes is
        // what it means, which is why both branches share one tail.
        assert_eq!(planned.retirement.destination_root_path, "/media/new");
        assert!(planned.retirement.permits_source_removal());
    }

    /// The blocked-title blocker names the branch it is blocking, because the
    /// consequence differs: a path change keeps the root configured, a fold
    /// cannot remove it.
    #[test]
    fn the_retirement_blocker_names_the_branch_it_blocks() {
        let blocked = |variant: RootScopeVariant| {
            let mut plan_request = request(variant);
            plan_request.titles[0].blocked_reason = Some("an import is running".to_string());
            build_root_scope_plan(&plan_request)
                .retirement
                .blockers
                .first()
                .expect("a blocked title blocks the retirement")
                .detail
                .clone()
        };

        assert!(
            blocked(RootScopeVariant::ChangePath {
                retention: RootRetentionFacts::default(),
            })
            .contains("root change")
        );
        assert!(
            blocked(RootScopeVariant::FoldInto {
                destination_root_id: "root-keep".to_string(),
                default_transfer: DefaultRootTransfer::default(),
            })
            .contains("consolidation")
        );
    }
}
