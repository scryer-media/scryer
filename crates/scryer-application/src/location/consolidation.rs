//! Root-consolidation planner: folding one root into **another existing root of
//! the same library** (US5, T070, FR-020, FR-022, FR-024–FR-029, FR-072–FR-075,
//! FR-087).
//!
//! This is FR-020's second branch — the one the root-change planner refuses by
//! name (`root_change::refusal_codes::DESTINATION_IS_CONFIGURED_ROOT`):
//!
//! > **FR-020**: Each configured root in library settings MUST offer **Change
//! > root** (to a new unconfigured path, or to another existing root in the same
//! > library — the latter being consolidation).
//!
//! Everything a root change does, a consolidation does too: it accounts for
//! every assigned title with no way to exclude one (FR-023), it separates
//! managed content from companions from unexplained content (FR-027), and it
//! retires the source location only after recycling completes and only when
//! nothing unexplained is left standing (FR-028, FR-087). Those rules are
//! *reused verbatim* from [`crate::location::root_change`] rather than restated
//! here, so the two branches of one settings action can never disagree.
//!
//! # What consolidation adds
//!
//! The destination already holds content. That single difference is what brings
//! in FR-024's classification, FR-025's uniquing, and the merge engine:
//!
//! > **FR-024**: The consolidation preview MUST classify: titles moving into
//! > unused destination folders; titles merging with existing destination
//! > titles; folder-name collisions between unrelated titles; media collisions;
//! > dedup-eligible identical files; sidecar/non-media collisions requiring
//! > rename; and untracked/unsupported content that prevents safe source-root
//! > retirement.
//!
//! Those seven live on [`ConsolidationClassification`], counted from the same
//! decisions that produce the plan items, so the preview's summary and its item
//! list cannot drift (SC-004).
//!
//! # Layout, or naming? Both, in that order (FR-026)
//!
//! > **FR-026**: Root replacement SHOULD preserve the source root's relative
//! > folder layout where practical; consolidation MAY apply destination naming
//! > rules to avoid collisions, with every changed folder name previewed.
//!
//! So the default is a root change's rule — re-anchor the absolute source path
//! onto the destination root and keep the whole relative position, however
//! deeply nested. Destination naming is applied only where preserving the layout
//! is *not* practical, which is exactly the two cases the destination's existing
//! content creates:
//!
//! 1. the re-anchored folder is already occupied by something that is not this
//!    title's merge target → the incoming folder is uniqued (FR-025), and
//! 2. the title merges into an existing destination title → the destination
//!    title's own folder wins, because the destination title keeps everything
//!    including its folder (FR-063).
//!
//! Every folder whose name changed is emitted as a [`PlanItemKind::Rename`]
//! item, which is US5 scenario 4's "every changed folder name was shown before
//! confirmation".
//!
//! # Unrelated titles never merge over a name (FR-025)
//!
//! > **FR-025**: Unrelated titles MUST never merge because they calculate the
//! > same destination folder; the incoming folder gets a unique previewed name
//! > or the operation remains blocked.
//!
//! Two branches are offered and this planner takes the first: it uniques.
//! Blocking would be the safe-but-useless answer for the ordinary case of two
//! libraries that both hold a `Blade Runner (1982)` folder, and the spec puts
//! uniquing first. The name comes from the collision engine's own
//! [`collision_rename_base`] plus its numeric disambiguation, so an incoming
//! *folder* is renamed by the same rule as an incoming *file* (FR-074), and the
//! two can never diverge into two naming schemes.
//!
//! A merge is decided by canonical metadata identity and nothing else
//! ([`crate::location::identity::detect_destination_title`], FR-055). A folder
//! name is never evidence of identity — which is precisely what FR-025 is about.
//!
//! # Purity
//!
//! No IO, no clock, no catalog access, in the
//! [`crate::location::root_change`] / [`crate::location::transfer_effects`]
//! idiom: the caller assembles the drafts, the destination listings, and the
//! identity outcomes, and every rule below is testable from literals. The IO
//! half is [`crate::location::consolidation_execution`].
//!
//! # Execution modes (spec gap, recorded)
//!
//! US5 never states which execution modes a consolidation offers. What the spec
//! *does* say settles it:
//!
//! - FR-020 files consolidation under **Change root**, and US4 scenario 1 —
//!   the other branch of the same action — names **Move with Scryer**.
//! - The heading over FR-030–FR-032 is "Managed move execution — *Move with
//!   Scryer* (US2, **US4**, **US5**, US6)", so US5 executes on that path.
//! - **Files are already there** is US3/FR-050: content the user moved *by hand*
//!   into a destination folder, accounted against stored catalog proof. A
//!   consolidation destination is an existing root whose content already belongs
//!   to other titles; there is no "the user already did this" shape of the
//!   request, and adopting a whole root has no spec basis.
//! - FR-076's catalog-only downgrade still applies: a source root whose titles
//!   have no tracked files needs no mode choice at all.
//!
//! So **Move with Scryer** is the only requestable mode, **CatalogOnly** is
//! derived, and **Files are already there** is refused by name
//! ([`refusal_codes::MODE_NOT_SUPPORTED`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::location::classify::TitleLocationClass;
use crate::location::collisions::{
    CollisionDisposition, CollisionNaming, CollisionPlan, CollisionPlanRequest, ContentFacts,
    DestinationItem, IncomingItem, PathCaseRule, RecycleAvailability, collision_rename_base,
    plan_collisions,
};
use crate::location::hardlinks::{HardlinkFact, hardlink_warnings};
use crate::location::identity::DestinationIdentityOutcome;
use crate::location::merge::summary::MergePreviewSummary;
use crate::location::model::{LocationExecutionMode, LocationOperationType, VerificationDepth};
use crate::location::preview::{
    FreeSpaceEstimate, LocationPlan, LocationPlanBuilder, LocationPlanHeader, PlanItem,
    PlanItemKind,
};
use crate::location::root_change::{
    BlockedTitle, RootContentInventory, RootEntry, RootRetirementBlocker, RootRetirementContract,
    TitleAccounting, classify_root_content, retirement_blockers,
};
use crate::location::root_move::{
    RootMoveExecutionPlan, RootMoveFileExecution, RootMoveTitleExecution, SourceFile,
    blocked_merge_summary_items, merge_summary_items,
};
use crate::stored_paths::path_to_stored_string;

/// Reason codes on the plan items this planner emits, so the UI groups and
/// translates rather than parsing prose (C3).
pub mod plan_reasons {
    /// The opening statement: what the consolidation does to the two roots
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
    /// The title has no folder to move, so only its stored root reference
    /// changes (FR-076).
    pub const CATALOG_ONLY_CONSOLIDATION: &str = "catalog_only_consolidation";
    /// The title cannot enter the consolidation until the user repairs it, and
    /// it cannot be excluded either (FR-023, FR-086).
    pub const TITLE_BLOCKED_FOR_CONSOLIDATION: &str = "title_blocked_for_consolidation";
    /// FR-024 (7): content at the source the catalog does not explain (FR-027).
    pub const UNKNOWN_ROOT_CONTENT: &str = "unknown_root_content";
    /// Why the source root cannot be retired once the titles have moved
    /// (FR-023, FR-028).
    pub const SOURCE_RETIREMENT_BLOCKED: &str = "source_retirement_blocked";
    /// The destination folder this title lands in already holds content, so the
    /// destination keeps every name it already has (FR-072).
    pub const DESTINATION_FOLDER_EXISTS: &str = "destination_folder_exists";
    /// A tracked file lives outside its title's folder but inside the root; it
    /// keeps its root-relative position (FR-026).
    pub const FILE_OUTSIDE_TITLE_FOLDER: &str = "file_outside_title_folder";
    /// Source files share their inode with another directory entry (FR-085).
    pub const HARDLINKED_SOURCE: &str = "hardlinked_source";
}

/// Machine-readable codes for the reasons a **consolidate root** request is
/// refused before anything is planned.
pub mod refusal_codes {
    /// A root path has to be absolute.
    pub const PATH_NOT_ABSOLUTE: &str = "root_consolidation_path_not_absolute";
    /// Source and destination are the same root.
    pub const SAME_ROOT: &str = "root_consolidation_same_root";
    /// Source and destination paths overlap, so moving out of one would move
    /// into the other.
    pub const PATHS_OVERLAP: &str = "root_consolidation_paths_overlap";
    /// The destination root id is not a configured root of this library. This
    /// is the mirror image of the root-change refusal: a destination that is
    /// *not* already a root is a **root change**, not a consolidation (FR-020).
    pub const DESTINATION_NOT_A_CONFIGURED_ROOT: &str =
        "root_consolidation_destination_not_a_configured_root";
    /// The source root is a symlink; moving out of one and retiring it would act
    /// on the link rather than on the content the user means.
    pub const SOURCE_ROOT_IS_SYMLINK: &str = "root_consolidation_source_root_is_symlink";
    /// The source root is not a readable directory right now.
    pub const SOURCE_ROOT_UNAVAILABLE: &str = "root_consolidation_source_root_unavailable";
    /// The destination root is not a readable directory right now, so what it
    /// already holds cannot be planned against.
    pub const DESTINATION_ROOT_UNAVAILABLE: &str =
        "root_consolidation_destination_root_unavailable";
    /// **Files are already there** is not a consolidation mode; see the module
    /// docs.
    pub const MODE_NOT_SUPPORTED: &str = "root_consolidation_mode_not_supported";
}

/// A refusal, with the code the client routes on and the sentence it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationRefusal {
    pub code: &'static str,
    pub detail: String,
}

impl ConsolidationRefusal {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for ConsolidationRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.detail)
    }
}

/// The filesystem and configuration facts a consolidation request is checked
/// against.
///
/// Separated from the check itself for the same reason the planner is pure: the
/// rules are then testable from literals, and the `stat`s happen once, in the
/// use case, where they can be reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationPathFacts {
    pub source_root_id: String,
    pub destination_root_id: String,
    /// Canonicalized where possible; both roots are expected to exist.
    pub source_root: PathBuf,
    pub destination_root: PathBuf,
    pub source_root_is_symlink: bool,
    pub source_root_is_directory: bool,
    pub destination_root_is_directory: bool,
    /// Every configured root of **this library**, paired with its id. The
    /// destination must be one of them; that is what makes the request a
    /// consolidation rather than a root change (FR-020).
    pub library_root_ids: Vec<String>,
    pub mode: LocationExecutionMode,
}

/// FR-020's admissibility rules for the consolidation branch.
pub fn check_consolidation_paths(
    facts: &ConsolidationPathFacts,
) -> Result<(), ConsolidationRefusal> {
    if facts.mode == LocationExecutionMode::FilesAlreadyThere {
        return Err(ConsolidationRefusal::new(
            refusal_codes::MODE_NOT_SUPPORTED,
            "a consolidation moves managed content between two configured roots; \"files are already there\" adopts content at a destination folder and is not offered here",
        ));
    }

    if facts.source_root_id == facts.destination_root_id {
        return Err(ConsolidationRefusal::new(
            refusal_codes::SAME_ROOT,
            format!(
                "root {} cannot be consolidated into itself",
                facts.source_root_id
            ),
        ));
    }

    // The mirror image of `root_change::refusal_codes::DESTINATION_IS_CONFIGURED_ROOT`:
    // a destination that is *not* already a root of this library is a root
    // change, and routing the user there by name beats a generic error.
    if !facts
        .library_root_ids
        .iter()
        .any(|root_id| root_id == &facts.destination_root_id)
    {
        return Err(ConsolidationRefusal::new(
            refusal_codes::DESTINATION_NOT_A_CONFIGURED_ROOT,
            format!(
                "root {} is not a configured root of this library; moving a root to a path that is not already a root is a root change, not a consolidation",
                facts.destination_root_id
            ),
        ));
    }

    for path in [&facts.source_root, &facts.destination_root] {
        if !path.is_absolute() {
            return Err(ConsolidationRefusal::new(
                refusal_codes::PATH_NOT_ABSOLUTE,
                format!("{} is not an absolute path", path.display()),
            ));
        }
    }

    if facts.source_root_is_symlink {
        return Err(ConsolidationRefusal::new(
            refusal_codes::SOURCE_ROOT_IS_SYMLINK,
            format!(
                "{} is a symlink; resolve it to the real directory before consolidating it",
                facts.source_root.display()
            ),
        ));
    }
    if !facts.source_root_is_directory {
        return Err(ConsolidationRefusal::new(
            refusal_codes::SOURCE_ROOT_UNAVAILABLE,
            format!(
                "{} is not a readable directory right now, so its contents cannot be planned",
                facts.source_root.display()
            ),
        ));
    }
    if !facts.destination_root_is_directory {
        return Err(ConsolidationRefusal::new(
            refusal_codes::DESTINATION_ROOT_UNAVAILABLE,
            format!(
                "{} is not a readable directory right now, so what it already holds cannot be planned against",
                facts.destination_root.display()
            ),
        ));
    }

    // Two configured roots should never nest, but a mis-configured pair would
    // make "move everything out of A into B" mean "move A into itself".
    if facts.destination_root == facts.source_root
        || facts.destination_root.starts_with(&facts.source_root)
        || facts.source_root.starts_with(&facts.destination_root)
    {
        return Err(ConsolidationRefusal::new(
            refusal_codes::PATHS_OVERLAP,
            format!(
                "{} and {} overlap; consolidating one into the other would move content into itself",
                facts.source_root.display(),
                facts.destination_root.display()
            ),
        ));
    }

    Ok(())
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
            Some("this root is not the library default, so the library's default is unchanged".to_string())
        }
    }
}

// ── FR-024: the seven-way preview classification ─────────────────────────────

/// Where one title's content lands at the destination root (FR-024's first three
/// classifications, plus the two a root change already had).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ConsolidationPlacement {
    /// FR-024 (1): the re-anchored folder is free, so the source root's relative
    /// layout is preserved exactly (FR-026).
    UnusedFolder,
    /// FR-024 (2): a destination title shares this title's canonical identity.
    /// The destination title keeps its folder (FR-063) and this title's content
    /// moves into it.
    MergesWithDestinationTitle {
        destination_title_id: String,
        destination_title_name: Option<String>,
    },
    /// FR-024 (3) + FR-025: the re-anchored folder name is taken by something
    /// unrelated, so the incoming folder gets a unique previewed name.
    FolderNameCollision {
        /// The name the layout would have preserved.
        collided_name: String,
        /// The destination title that already owns that name, when the occupier
        /// is a title rather than an untracked directory.
        occupied_by_title_id: Option<String>,
    },
    /// FR-076: no folder to place.
    NoFolder,
}

impl ConsolidationPlacement {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnusedFolder => "unused_folder",
            Self::MergesWithDestinationTitle { .. } => "merges_with_destination_title",
            Self::FolderNameCollision { .. } => "folder_name_collision",
            Self::NoFolder => "no_folder",
        }
    }

    /// The destination title this title folds into (US7, FR-055, FR-063).
    pub fn merge_target(&self) -> Option<&str> {
        match self {
            Self::MergesWithDestinationTitle {
                destination_title_id,
                ..
            } => Some(destination_title_id.as_str()),
            _ => None,
        }
    }
}

/// What one assigned title's outcome is, in the FR-023 ledger's vocabulary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationTitleOutcome {
    /// The title owns a folder under the source root; its content moves.
    Relocates,
    /// The title owns no folder, so nothing moves and only its stored root
    /// reference changes (FR-076).
    CatalogOnly,
    /// The title cannot enter the operation yet, and cannot be dropped from it
    /// either (FR-023, FR-086).
    Blocked,
}

impl ConsolidationTitleOutcome {
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

/// FR-024's seven classifications, counted off the same decisions that built the
/// plan items.
///
/// Three are title-scoped (1–3), three are file-scoped (4–6), and the seventh is
/// the source root's unexplained content — the one that decides whether the
/// source root can be retired at all.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ConsolidationClassification {
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

impl ConsolidationClassification {
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

    /// FR-024's total, for a preview that wants to say "7 classifications, N
    /// items".
    pub fn classified_items(&self) -> i64 {
        self.moving_into_unused_folders
            + self.merging_with_destination_titles
            + self.folder_name_collisions
            + self.media_collisions
            + self.dedup_eligible_files
            + self.companion_collisions
            + self.untracked_source_entries
    }
}

// ── Folder resolution (FR-025, FR-026) ───────────────────────────────────────

/// What the caller found at one candidate destination folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestinationFolderState {
    /// Nothing exists at the path.
    Free,
    /// An existing but empty directory: nothing to collide with, so the layout
    /// is preserved and the directory is reused.
    Empty,
    /// An existing directory owned by a destination title.
    OwnedByTitle { title_id: String },
    /// An existing non-empty directory that no destination title owns.
    Occupied,
}

impl DestinationFolderState {
    fn is_free_for(&self, merge_target: Option<&str>) -> bool {
        match self {
            Self::Free | Self::Empty => true,
            Self::OwnedByTitle { title_id } => Some(title_id.as_str()) == merge_target,
            Self::Occupied => false,
        }
    }
}

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
    /// What the caller found at each candidate destination path, keyed by the
    /// stored form of that path. A path missing from the map is [`Free`].
    ///
    /// [`Free`]: DestinationFolderState::Free
    pub destination_states: BTreeMap<String, DestinationFolderState>,
}

/// Where one title's folder lands, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFolder {
    pub title_id: String,
    /// `None` only for a title that owns no folder.
    pub destination_folder: Option<PathBuf>,
    pub placement: ConsolidationPlacement,
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
pub fn resolve_consolidation_folders(request: &FolderResolutionRequest) -> Vec<ResolvedFolder> {
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    let mut resolved = Vec::with_capacity(request.titles.len());

    for title in &request.titles {
        let Some(source_folder) = title.source_folder_path.as_ref() else {
            resolved.push(ResolvedFolder {
                title_id: title.title_id.clone(),
                destination_folder: None,
                placement: ConsolidationPlacement::NoFolder,
                renamed_to: None,
            });
            continue;
        };

        // FR-063: a merge target keeps the folder it already owns; its content
        // is already there and stays there.
        if let Some(destination_title_id) = title.merge_target_title_id.as_deref() {
            let destination_folder = title
                .merge_target_folder_path
                .clone()
                .or_else(|| rebase(source_folder, &request.source_root, &request.destination_root))
                .unwrap_or_else(|| {
                    request
                        .destination_root
                        .join(file_name_of(source_folder, &title.title_id))
                });
            claimed.insert(fold(&request.case_rule, &destination_folder));
            let renamed_to = renamed_name(source_folder, &destination_folder);
            resolved.push(ResolvedFolder {
                title_id: title.title_id.clone(),
                destination_folder: Some(destination_folder),
                placement: ConsolidationPlacement::MergesWithDestinationTitle {
                    destination_title_id: destination_title_id.to_string(),
                    destination_title_name: title.merge_target_title_name.clone(),
                },
                renamed_to,
            });
            continue;
        }

        // FR-026: preserve the source root's relative folder layout by default.
        let preserved = rebase(source_folder, &request.source_root, &request.destination_root)
            .unwrap_or_else(|| {
                // A folder outside the root cannot be re-anchored; its own
                // basename directly under the destination root is the only
                // defensible place. `classify_root_content` already treats such
                // content as unexplained, so this is the belt to that braces.
                request
                    .destination_root
                    .join(file_name_of(source_folder, &title.title_id))
            });

        let state = request
            .destination_states
            .get(&path_to_stored_string(&preserved))
            .cloned()
            .unwrap_or(DestinationFolderState::Free);
        let already_claimed = claimed.contains(&fold(&request.case_rule, &preserved));

        if !already_claimed && state.is_free_for(None) {
            claimed.insert(fold(&request.case_rule, &preserved));
            resolved.push(ResolvedFolder {
                title_id: title.title_id.clone(),
                destination_folder: Some(preserved),
                placement: ConsolidationPlacement::UnusedFolder,
                renamed_to: None,
            });
            continue;
        }

        // FR-025: unrelated titles never merge over a name.
        let collided_name = file_name_of(&preserved, &title.title_id);
        let unique = unique_folder_path(
            &preserved,
            &collided_name,
            &request.naming.source_library_label,
            &request.case_rule,
            &claimed,
            &request.destination_states,
        );
        claimed.insert(fold(&request.case_rule, &unique));
        let occupied_by_title_id = match &state {
            DestinationFolderState::OwnedByTitle { title_id } => Some(title_id.clone()),
            _ => None,
        };
        let renamed_to = renamed_name(source_folder, &unique);
        resolved.push(ResolvedFolder {
            title_id: title.title_id.clone(),
            destination_folder: Some(unique),
            placement: ConsolidationPlacement::FolderNameCollision {
                collided_name,
                occupied_by_title_id,
            },
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
    destination_states: &BTreeMap<String, DestinationFolderState>,
) -> PathBuf {
    let parent = preserved.parent().map(Path::to_path_buf).unwrap_or_default();
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
        let state = destination_states
            .get(&path_to_stored_string(&candidate))
            .cloned()
            .unwrap_or(DestinationFolderState::Free);
        if state.is_free_for(None) {
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

fn file_name_of(path: &Path, fallback: &str) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| fallback.to_string())
}

fn renamed_name(source_folder: &Path, destination_folder: &Path) -> Option<String> {
    let source = source_folder.file_name()?;
    let destination = destination_folder.file_name()?;
    (source != destination).then(|| destination.to_string_lossy().to_string())
}

/// Re-anchor `path` from `source_root` onto `destination_root`, preserving its
/// relative position (FR-026). `None` when `path` is not under `source_root`.
fn rebase(path: &Path, source_root: &Path, destination_root: &Path) -> Option<PathBuf> {
    path.strip_prefix(source_root)
        .ok()
        .map(|relative| destination_root.join(relative))
}

// ── Planner input ────────────────────────────────────────────────────────────

/// Everything the planner needs about one title assigned to the source root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationTitleDraft {
    pub title_id: String,
    pub title_name: String,
    /// The folder the title owns today. `None` for a fileless title — it is
    /// still accounted for (FR-023), it simply has nothing to move.
    pub source_folder_path: Option<PathBuf>,
    /// Files beneath (or tracked by) the title.
    ///
    /// Unlike a root change, [`SourceFile::relative_path`] **is** read here:
    /// a consolidation may rename the title's folder (FR-025/FR-026), so a
    /// file's position is preserved relative to its *folder* rather than to the
    /// root. For a folder whose name is preserved the two are the same thing.
    /// [`SourceFile::full_blake3`] is read too — it is what proves a duplicate
    /// without reading a byte (FR-073, D4).
    pub files: Vec<SourceFile>,
    /// Directories beneath the title's folder, deepest first, that cleanup may
    /// remove once empty (FR-028).
    pub source_directories: Vec<PathBuf>,
    pub hardlinks: Vec<HardlinkFact>,
    /// Where this title's folder lands, from [`resolve_consolidation_folders`].
    pub resolved: ResolvedFolder,
    /// Entries already present at the resolved destination folder (FR-072).
    pub destination_entries: Vec<DestinationItem>,
    /// Whether recycling is usable for this title's source root (FR-073).
    pub recycle: RecycleAvailability,
    /// What destination-title detection concluded (FR-055), for the same-name
    /// warning that FR-025 exists to make unnecessary.
    pub destination_identity: Option<DestinationIdentityOutcome>,
    /// The merge the engine planned for this title at preview time (FR-066,
    /// FR-071), or `None` when the title is not a merge candidate.
    pub merge_summary: Option<MergePreviewSummary>,
    /// Why the title is blocked: an active download or import (FR-086), or
    /// another operation already owning it (FR-084).
    pub blocked_reason: Option<String>,
    pub blocked_reason_code: Option<String>,
}

impl ConsolidationTitleDraft {
    /// FR-023: every assigned title lands in exactly one outcome, and none of
    /// them is "excluded".
    pub fn outcome(&self) -> ConsolidationTitleOutcome {
        if self.blocked_reason.is_some() {
            ConsolidationTitleOutcome::Blocked
        } else if self.source_folder_path.is_none() {
            ConsolidationTitleOutcome::CatalogOnly
        } else {
            ConsolidationTitleOutcome::Relocates
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

/// The whole planning request. Every fact is supplied; nothing is read here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootConsolidationPlanRequest {
    pub library_id: String,
    /// The root being folded away. Its configuration is retired at the end
    /// (FR-087); its synthetic id does not survive the operation.
    pub source_root_id: String,
    pub source_root_path: PathBuf,
    /// The root that absorbs the content. It keeps its id, and may gain the
    /// library default (FR-022, FR-078).
    pub destination_root_id: String,
    pub destination_root_path: PathBuf,
    pub default_transfer: DefaultRootTransfer,
    /// Every title assigned to the source root. Not a selection: FR-023 forbids
    /// one.
    pub titles: Vec<ConsolidationTitleDraft>,
    /// The caller's scan of the source root.
    pub entries: Vec<RootEntry>,
    pub verification_depth: VerificationDepth,
    pub free_space: FreeSpaceEstimate,
    /// `Some(true)` when the two roots share a volume (rename fast path,
    /// FR-032), `None` when the relationship could not be probed.
    pub same_volume: Option<bool>,
    pub case_rule: PathCaseRule,
    pub naming: CollisionNaming,
}

/// The planner result: the plan the user confirms, the instructions the runner
/// executes, and the root-scoped facts neither of those two can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRootConsolidation {
    pub plan: LocationPlan,
    pub execution: RootMoveExecutionPlan,
    pub accounting: TitleAccounting,
    pub classification: ConsolidationClassification,
    pub default_transfer: DefaultRootTransfer,
    pub content: RootContentInventory,
    pub retirement: RootRetirementContract,
    pub warnings: Vec<String>,
}

impl PlannedRootConsolidation {
    /// The runner's view of this plan, through the shared work-plan seam.
    pub fn work_plan(&self) -> crate::location::executor::OperationWorkPlan {
        self.execution.to_work_plan()
    }
}

/// The consolidation-specific half of the root-scoped tail (T071).
///
/// It rides *inside* [`crate::location::root_change::RootChangeTail`] rather
/// than beside it, because the two branches of FR-020's **Change root** share
/// one epilogue: the same recycle-bin relocation, the same empty-directory
/// prune, the same "only after all recycling completes" ordering (FR-087). Only
/// the last step differs — a root change repoints its root at a new path, and a
/// consolidation removes the root's configuration entirely — and that is exactly
/// what this struct's presence selects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationTail {
    /// The root that absorbs the content. It keeps its id (FR-078).
    pub destination_root_id: String,
    pub default_transfer: DefaultRootTransfer,
}

// ── Planner ──────────────────────────────────────────────────────────────────

/// Build the consolidation preview and execution plan (T070).
pub fn build_root_consolidation_plan(
    request: &RootConsolidationPlanRequest,
) -> PlannedRootConsolidation {
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
    // FR-027, reused verbatim from the root-change planner: the three buckets
    // and the prunable/retained directory split are the same question, and the
    // two branches of one settings action must answer it the same way.
    let content = classify_root_content(
        &request.source_root_path,
        &request.entries,
        &title_folders,
        &tracked_media_paths,
    );

    let source_root_display = path_to_stored_string(&request.source_root_path);
    let destination_root_display = path_to_stored_string(&request.destination_root_path);
    let retirement = build_retirement_contract(
        &source_root_display,
        &destination_root_display,
        &accounting,
        &content,
    );

    let header = LocationPlanHeader::new(
        LocationOperationType::RootConsolidation,
        execution_mode_for(&accounting),
    )
    .with_source(
        Some(request.library_id.clone()),
        Some(request.source_root_id.clone()),
    )
    // One library, two roots: the destination root is a real, different root
    // (FR-020), unlike a root change where both sides carry one id.
    .with_destination(
        Some(request.library_id.clone()),
        Some(request.destination_root_id.clone()),
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
    let mut classification = ConsolidationClassification {
        catalog_only: accounting.catalog_only,
        blocked: accounting.blocked,
        untracked_source_entries: content.unknown.len() as i64,
        ..ConsolidationClassification::default()
    };
    let mut execution = RootMoveExecutionPlan {
        no_op_titles: 0,
        unresolved_titles: accounting.blocked,
        ..RootMoveExecutionPlan::default()
    };

    // The root-level statement first: what the operation does to the two roots
    // themselves, before anything it does to a title (FR-020, FR-022).
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
    if let Some(statement) = request.default_transfer.statement(&destination_root_display) {
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

    for (index, draft) in request.titles.iter().enumerate() {
        let (title_execution, items, title_warnings) =
            plan_title(request, draft, index as i64, &mut classification);
        builder.extend(items);
        warnings.extend(title_warnings);
        // FR-071 + FR-081: the merge summary is both what the preview shows and
        // part of what the fingerprint covers, so it is recorded for every merge
        // candidate — including a blocked one, whose records the user has to see
        // before deciding anything.
        if let Some(summary) = draft.merge_summary.clone() {
            builder.merge(summary);
        }
        if let Some(title_execution) = title_execution {
            execution.titles.push(title_execution);
        }
    }

    // FR-024 (7) / FR-027: unexplained content is listed, item by item, so it is
    // neither silently deleted nor silently abandoned — and so that new junk
    // appearing at the source between preview and start changes the fingerprint.
    for entry in &content.unknown {
        builder.push(
            PlanItem::new(PlanItemKind::UnmanagedContent)
                .with_paths(Some(entry.path.clone()), Option::<String>::None)
                .with_size(entry.size_bytes)
                .with_reason_code(plan_reasons::UNKNOWN_ROOT_CONTENT)
                .with_detail(format!(
                    "{} is not tracked by any title on this root; it stays where it is, and the source root stays configured until it is resolved",
                    entry.path
                )),
        );
    }

    // FR-028/FR-023: say why the source root survives the operation.
    for blocker in &retirement.blockers {
        builder.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_paths(Some(source_root_display.clone()), Option::<String>::None)
                .with_reason_code(plan_reasons::SOURCE_RETIREMENT_BLOCKED)
                .with_detail(blocker.detail.clone()),
        );
        warnings.push(blocker.detail.clone());
    }

    PlannedRootConsolidation {
        plan: builder.build(),
        execution,
        accounting,
        classification,
        default_transfer: request.default_transfer,
        content,
        retirement,
        warnings,
    }
}

fn build_accounting(titles: &[ConsolidationTitleDraft]) -> TitleAccounting {
    let mut accounting = TitleAccounting {
        assigned_total: titles.len() as i64,
        ..TitleAccounting::default()
    };
    for draft in titles {
        match draft.outcome() {
            ConsolidationTitleOutcome::Relocates => accounting.relocating += 1,
            ConsolidationTitleOutcome::CatalogOnly => accounting.catalog_only += 1,
            ConsolidationTitleOutcome::Blocked => {
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

/// FR-028/FR-023, restated for the branch that removes a root rather than
/// repointing it.
///
/// The blockers are the root-change ones, and they mean *more* here: a root
/// change keeps the source root configured (pointing somewhere else), so
/// unexplained content only stops the old directory from being deleted. A
/// consolidation removes the root's configuration, and US4.3's "root removal is
/// blocked until the user resolves them" is then literally about this operation's
/// last step.
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
                "{} title(s) on this root must be repaired before the source root can be retired; they cannot be excluded from a consolidation",
                accounting.blocked
            ),
        });
    }
    if content.blocks_source_removal() {
        blockers.push(RootRetirementBlocker {
            code: retirement_blockers::UNEXPLAINED_SOURCE_CONTENT.to_string(),
            detail: format!(
                "{} item(s) at {source_root_display} are not explained by the catalog, so {source_root_display} stays a configured root until they are resolved; its titles still move onto {destination_root_display}",
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

/// A consolidation with nothing to move needs no move mode; FR-076 asks the UI
/// to skip the chooser in exactly that case.
///
/// The requested mode is not consulted: **files are already there** was refused
/// at admission ([`check_consolidation_paths`]), so the only mode a consolidation
/// can be running in is **Move with Scryer** — see the module docs on the
/// execution-mode gap in US5.
fn execution_mode_for(accounting: &TitleAccounting) -> LocationExecutionMode {
    if accounting.relocating > 0 {
        LocationExecutionMode::MoveWithScryer
    } else {
        LocationExecutionMode::CatalogOnly
    }
}

fn plan_title(
    request: &RootConsolidationPlanRequest,
    draft: &ConsolidationTitleDraft,
    sequence: i64,
    classification: &mut ConsolidationClassification,
) -> (Option<RootMoveTitleExecution>, Vec<PlanItem>, Vec<String>) {
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    let outcome = draft.outcome();

    let source_root_display = path_to_stored_string(&request.source_root_path);
    let destination_root_display = path_to_stored_string(&request.destination_root_path);

    if outcome == ConsolidationTitleOutcome::Blocked {
        // FR-023: represented, never dropped. The user repairs it; there is no
        // "exclude" affordance to offer.
        items.push(
            PlanItem::new(PlanItemKind::Blocked)
                .with_title(draft.title_id.clone())
                .with_paths(
                    draft.source_folder_path.as_deref().map(path_to_stored_string),
                    Option::<String>::None,
                )
                .with_reason_code(draft.blocked_reason_code.clone().unwrap_or_else(|| {
                    plan_reasons::TITLE_BLOCKED_FOR_CONSOLIDATION.to_string()
                }))
                .with_detail(draft.blocked_reason.clone().unwrap_or_else(|| {
                    format!("\"{}\" needs a repair before it can move", draft.title_name)
                })),
        );
        // FR-066: an unmappable merge is refused per record, and the records are
        // what the user acts on.
        items.extend(blocked_merge_summary_items(
            &draft.title_id,
            &draft.title_name,
            draft.merge_summary.as_ref(),
        ));
        return (None, items, warnings);
    }

    if outcome == ConsolidationTitleOutcome::CatalogOnly {
        items.push(
            PlanItem::new(PlanItemKind::CatalogChange)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::CATALOG_ONLY_CONSOLIDATION)
                .with_detail(format!(
                    "\"{}\" owns no folder, so only its stored root reference changes",
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
                source_root_id: request.source_root_id.clone(),
                source_folder_path: None,
                destination_library_id: request.library_id.clone(),
                destination_root_id: request.destination_root_id.clone(),
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
                // A fileless title merges too when it shares an identity: the
                // catalog fold is the whole operation for it.
                merge_target_title_id: draft
                    .resolved
                    .placement
                    .merge_target()
                    .map(str::to_string),
            }),
            items,
            warnings,
        );
    }

    let source_folder = draft
        .source_folder_path
        .clone()
        .expect("a relocating title owns a folder");
    let destination_folder = draft
        .resolved
        .destination_folder
        .clone()
        .unwrap_or_else(|| {
            request
                .destination_root_path
                .join(file_name_of(&source_folder, &draft.title_id))
        });
    let source_folder_display = path_to_stored_string(&source_folder);
    let destination_folder_display = path_to_stored_string(&destination_folder);

    // FR-024's first three classifications, stated on the plan the user reads.
    match &draft.resolved.placement {
        ConsolidationPlacement::UnusedFolder => {
            classification.moving_into_unused_folders += 1;
            items.push(
                PlanItem::new(PlanItemKind::CatalogChange)
                    .with_title(draft.title_id.clone())
                    .with_paths(
                        Some(source_folder_display.clone()),
                        Some(destination_folder_display.clone()),
                    )
                    .with_reason_code(plan_reasons::MOVES_INTO_UNUSED_FOLDER)
                    .with_detail(format!(
                        "\"{}\" moves into an unused folder on the destination root; its folder layout is preserved",
                        draft.title_name
                    )),
            );
        }
        ConsolidationPlacement::MergesWithDestinationTitle {
            destination_title_id,
            destination_title_name,
        } => {
            classification.merging_with_destination_titles += 1;
            let named = destination_title_name
                .as_deref()
                .map(|name| format!("\"{name}\" ({destination_title_id})"))
                .unwrap_or_else(|| destination_title_id.clone());
            items.push(
                PlanItem::new(PlanItemKind::Merge)
                    .with_title(draft.title_id.clone())
                    .with_paths(
                        Some(source_folder_display.clone()),
                        Some(destination_folder_display.clone()),
                    )
                    .with_reason_code(plan_reasons::MERGES_WITH_DESTINATION_TITLE)
                    .with_detail(format!(
                        "\"{}\" shares a metadata identity with {named} on the destination root, so the two merge; the destination title keeps its id, settings, monitoring, naming, and its folder, and this title's additive data is unioned onto it",
                        draft.title_name
                    )),
            );
            items.extend(merge_summary_items(
                &draft.title_id,
                &draft.title_name,
                draft.merge_summary.as_ref(),
            ));
        }
        ConsolidationPlacement::FolderNameCollision {
            collided_name,
            occupied_by_title_id,
        } => {
            classification.folder_name_collisions += 1;
            let occupier = occupied_by_title_id
                .as_deref()
                .map(|title_id| format!("an unrelated title ({title_id})"))
                .unwrap_or_else(|| "content that no title on this library owns".to_string());
            items.push(
                PlanItem::new(PlanItemKind::Rename)
                    .for_folder()
                    .with_title(draft.title_id.clone())
                    .with_paths(
                        Some(source_folder_display.clone()),
                        Some(destination_folder_display.clone()),
                    )
                    .with_reason_code(plan_reasons::FOLDER_NAME_UNIQUED)
                    .with_detail(format!(
                        "\"{collided_name}\" at the destination root already belongs to {occupier}, and an identical folder name is not evidence of an identical title; \"{}\" is previewed into \"{}\" instead",
                        draft.title_name,
                        file_name_of(&destination_folder, &draft.title_id)
                    )),
            );
        }
        ConsolidationPlacement::NoFolder => {}
    }

    // FR-055's same-name statement still earns its place: it is the sentence
    // that explains why two same-named titles are about to sit side by side.
    if let Some(warning) = same_named_destination_warning(draft) {
        items.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_reason_code(plan_reasons::FOLDER_NAME_UNIQUED)
                .with_detail(warning.clone()),
        );
        warnings.push(warning);
    }

    // FR-072: the destination folder keeps every name it already has.
    let collisions = plan_title_collisions(request, draft);
    if !draft.destination_entries.is_empty() {
        items.push(
            PlanItem::new(PlanItemKind::Warning)
                .with_title(draft.title_id.clone())
                .with_paths(
                    Option::<String>::None,
                    Some(destination_folder_display.clone()),
                )
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
            .unwrap_or_else(|| file_name_of(&file.path, &draft.title_id));

        // FR-026: the layout is preserved relative to the title's folder, which
        // is the same thing as relative to the root whenever the folder name
        // survived — and the only correct thing when it did not.
        let destination_path = match file.relative_path.as_ref() {
            Some(relative) => match relative.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => {
                    destination_folder.join(parent).join(&final_name)
                }
                _ => destination_folder.join(&final_name),
            },
            None => {
                let placed = destination_folder.join(&final_name);
                warnings.push(format!(
                    "\"{}\" tracks {} outside its folder; it moves into the destination folder",
                    draft.title_name,
                    file.path.display()
                ));
                items.push(
                    PlanItem::new(PlanItemKind::Warning)
                        .with_title(draft.title_id.clone())
                        .with_paths(
                            Some(path_to_stored_string(&file.path)),
                            Some(path_to_stored_string(&placed)),
                        )
                        .with_reason_code(plan_reasons::FILE_OUTSIDE_TITLE_FOLDER)
                        .with_detail(
                            "this tracked file is not inside the title's folder; it is placed in the destination folder"
                                .to_string(),
                        ),
                );
                placed
            }
        };

        let source_display = path_to_stored_string(&file.path);
        let destination_display = path_to_stored_string(&destination_path);

        match decision.as_ref().map(|decision| decision.disposition) {
            // FR-024 (5) + FR-073: proven duplicate. No bytes are written, the
            // source copy is recycled, and the preview says so before anything
            // is confirmed.
            Some(CollisionDisposition::DedupRecycleSource) => {
                classification.dedup_eligible_files += 1;
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
                for warning in decision
                    .as_ref()
                    .map(|decision| decision.warnings.clone())
                    .unwrap_or_default()
                {
                    warnings.push(warning.message());
                }
                continue;
            }
            // FR-024 (4) and (6): a media collision and a companion collision
            // are counted separately, which is what FR-075's "lists renamed and
            // deduplicated assets separately from media files" asks for.
            Some(disposition) if disposition.is_rename() => {
                if file.media_file_id.is_some() {
                    classification.media_collisions += 1;
                } else {
                    classification.companion_collisions += 1;
                }
                renamed_destinations.push(destination_display.clone());
                items.push(
                    PlanItem::new(PlanItemKind::Rename)
                        .with_title(draft.title_id.clone())
                        .with_paths(
                            Some(source_display.clone()),
                            Some(destination_display.clone()),
                        )
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
            .with_paths(
                Some(source_display.clone()),
                Some(destination_display.clone()),
            )
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

    // FR-085. A same-volume consolidation renames rather than copies, so nothing
    // is recycled and a hardlink survives — unless a dedup recycles a source
    // copy, which happens on either volume relationship.
    let recycles_source = request.same_volume != Some(true) || !deduplicated_sources.is_empty();
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
    if !prune_directories.contains(&source_folder_display) {
        prune_directories.push(source_folder_display.clone());
    }

    let execution = RootMoveTitleExecution {
        title_id: draft.title_id.clone(),
        title_name: draft.title_name.clone(),
        sequence,
        class: outcome.class(),
        source_library_id: request.library_id.clone(),
        source_root_id: request.source_root_id.clone(),
        source_folder_path: Some(source_folder_display),
        destination_library_id: request.library_id.clone(),
        destination_root_id: request.destination_root_id.clone(),
        destination_folder_path: Some(destination_folder_display),
        destination_root_path: Some(destination_root_display),
        source_root_path: Some(source_root_display),
        same_volume: request.same_volume,
        files,
        deduplicated_sources,
        deduplicated_media_file_ids,
        renamed_destinations,
        prune_directories,
        warnings: warnings.clone(),
        // One library on both sides: no facet conversion, no tag stripping.
        converted_facet: None,
        dropped_tag_prefixes: Vec::new(),
        // The one field that turns this title's catalog step into the merge
        // engine's transaction (US7, FR-063).
        merge_target_title_id: draft
            .resolved
            .placement
            .merge_target()
            .map(str::to_string),
    };

    (Some(execution), items, warnings)
}

fn plan_title_collisions(
    request: &RootConsolidationPlanRequest,
    draft: &ConsolidationTitleDraft,
) -> Option<CollisionPlan> {
    if draft.destination_entries.is_empty() {
        return None;
    }

    let mut incoming = Vec::with_capacity(draft.files.len());
    for file in &draft.files {
        let name = file_name_of(&file.path, &draft.title_id);
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

    Some(plan_collisions(
        &CollisionPlanRequest::new(request.case_rule, request.naming.clone())
            .with_recycle(draft.recycle.clone())
            .with_destination(draft.destination_entries.clone())
            .with_incoming(incoming),
    ))
}

/// Stable, collision-free id for one source file inside one title's collision
/// plan: the media file id when there is one, else the stored source path. The
/// same shape the root-move planner uses, so the two produce comparable plans.
fn collision_item_id(file: &SourceFile) -> String {
    match file.media_file_id.as_deref() {
        Some(id) => format!("media:{id}"),
        None => format!("asset:{}", path_to_stored_string(&file.path)),
    }
}

/// FR-055/FR-025's same-name statement: two titles that share a folder name but
/// no identity end up side by side, and the preview says so rather than letting
/// the user discover it.
fn same_named_destination_warning(draft: &ConsolidationTitleDraft) -> Option<String> {
    let outcome = draft.destination_identity.as_ref()?;
    let title_id = outcome.same_name_title_id.as_deref()?;
    let name = outcome
        .same_name_title_name
        .as_deref()
        .unwrap_or(&draft.title_name);
    Some(format!(
        "the destination root already holds a title called \"{name}\" ({title_id}); it shares no metadata identity with \"{}\", so the two are not merged and both will exist there",
        draft.title_name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::location::collisions::FullHash;
    use crate::location::hardlinks::LinkCount;
    use crate::location::identity::{DestinationIdentityOutcome, IdentityCandidate};
    use crate::location::merge::DestinationIdentityMatch;
    use crate::location::preview::{
        LOCATION_TYPED_CONFIRMATION_PHRASE, PlanConfirmationError, PlanConfirmationRequest,
    };

    const SOURCE_ROOT: &str = "/media/old";
    const DESTINATION_ROOT: &str = "/media/keep";

    fn tracked(path: &str, size_bytes: u64, folder: &str) -> SourceFile {
        SourceFile {
            media_file_id: Some(format!("file-{path}")),
            full_blake3: FullHash::Absent,
            path: PathBuf::from(path),
            relative_path: PathBuf::from(path)
                .strip_prefix(folder)
                .ok()
                .map(Path::to_path_buf),
            size_bytes,
        }
    }

    fn companion_file(path: &str, size_bytes: u64, folder: &str) -> SourceFile {
        SourceFile {
            media_file_id: None,
            full_blake3: FullHash::Absent,
            path: PathBuf::from(path),
            relative_path: PathBuf::from(path)
                .strip_prefix(folder)
                .ok()
                .map(Path::to_path_buf),
            size_bytes,
        }
    }

    fn resolution_title(id: &str, folder: Option<&str>) -> FolderResolutionTitle {
        FolderResolutionTitle {
            title_id: id.to_string(),
            title_name: id.to_string(),
            source_folder_path: folder.map(PathBuf::from),
            merge_target_title_id: None,
            merge_target_title_name: None,
            merge_target_folder_path: None,
        }
    }

    fn resolution(
        titles: Vec<FolderResolutionTitle>,
        states: Vec<(&str, DestinationFolderState)>,
    ) -> FolderResolutionRequest {
        FolderResolutionRequest {
            source_root: PathBuf::from(SOURCE_ROOT),
            destination_root: PathBuf::from(DESTINATION_ROOT),
            case_rule: PathCaseRule::CaseSensitive,
            naming: CollisionNaming::from_source_library("Old Disk"),
            titles,
            destination_states: states
                .into_iter()
                .map(|(path, state)| (path.to_string(), state))
                .collect(),
        }
    }

    fn draft(id: &str, folder: &str, files: Vec<SourceFile>, resolved: ResolvedFolder) -> ConsolidationTitleDraft {
        ConsolidationTitleDraft {
            title_id: id.to_string(),
            title_name: id.to_string(),
            source_folder_path: Some(PathBuf::from(folder)),
            files,
            source_directories: Vec::new(),
            hardlinks: Vec::new(),
            resolved,
            destination_entries: Vec::new(),
            recycle: RecycleAvailability::Available,
            destination_identity: None,
            merge_summary: None,
            blocked_reason: None,
            blocked_reason_code: None,
        }
    }

    fn resolved_unused(id: &str, folder: &str) -> ResolvedFolder {
        ResolvedFolder {
            title_id: id.to_string(),
            destination_folder: Some(PathBuf::from(folder)),
            placement: ConsolidationPlacement::UnusedFolder,
            renamed_to: None,
        }
    }

    fn request(titles: Vec<ConsolidationTitleDraft>) -> RootConsolidationPlanRequest {
        RootConsolidationPlanRequest {
            library_id: "library-1".to_string(),
            source_root_id: "root-old".to_string(),
            source_root_path: PathBuf::from(SOURCE_ROOT),
            destination_root_id: "root-keep".to_string(),
            destination_root_path: PathBuf::from(DESTINATION_ROOT),
            default_transfer: DefaultRootTransfer::default(),
            titles,
            entries: Vec::new(),
            verification_depth: VerificationDepth::Full,
            free_space: FreeSpaceEstimate::unknown(),
            same_volume: Some(false),
            case_rule: PathCaseRule::CaseSensitive,
            naming: CollisionNaming::from_source_library("Old Disk"),
        }
    }

    fn facts() -> ConsolidationPathFacts {
        ConsolidationPathFacts {
            source_root_id: "root-old".to_string(),
            destination_root_id: "root-keep".to_string(),
            source_root: PathBuf::from(SOURCE_ROOT),
            destination_root: PathBuf::from(DESTINATION_ROOT),
            source_root_is_symlink: false,
            source_root_is_directory: true,
            destination_root_is_directory: true,
            library_root_ids: vec!["root-old".to_string(), "root-keep".to_string()],
            mode: LocationExecutionMode::MoveWithScryer,
        }
    }

    // ── Admissibility (FR-020) ───────────────────────────────────────────────

    #[test]
    fn a_destination_that_is_not_a_configured_root_is_refused_as_a_root_change() {
        let mut facts = facts();
        facts.destination_root_id = "root-elsewhere".to_string();
        let refusal = check_consolidation_paths(&facts).expect_err("not a root of this library");
        assert_eq!(
            refusal.code,
            refusal_codes::DESTINATION_NOT_A_CONFIGURED_ROOT
        );
        assert!(
            refusal.detail.contains("root change"),
            "the user is routed to the other branch: {}",
            refusal.detail
        );
    }

    #[test]
    fn a_root_cannot_be_consolidated_into_itself() {
        let mut facts = facts();
        facts.destination_root_id = facts.source_root_id.clone();
        assert_eq!(
            check_consolidation_paths(&facts).expect_err("same root").code,
            refusal_codes::SAME_ROOT
        );
    }

    #[test]
    fn files_already_there_is_not_a_consolidation_mode() {
        let mut facts = facts();
        facts.mode = LocationExecutionMode::FilesAlreadyThere;
        let refusal = check_consolidation_paths(&facts).expect_err("mode refused");
        assert_eq!(refusal.code, refusal_codes::MODE_NOT_SUPPORTED);
    }

    #[test]
    fn overlapping_roots_and_unreadable_roots_are_refused() {
        let mut nested = facts();
        nested.destination_root = PathBuf::from("/media/old/inner");
        assert_eq!(
            check_consolidation_paths(&nested).expect_err("overlap").code,
            refusal_codes::PATHS_OVERLAP
        );

        let mut symlinked = facts();
        symlinked.source_root_is_symlink = true;
        assert_eq!(
            check_consolidation_paths(&symlinked)
                .expect_err("symlink")
                .code,
            refusal_codes::SOURCE_ROOT_IS_SYMLINK
        );

        let mut unreadable = facts();
        unreadable.destination_root_is_directory = false;
        assert_eq!(
            check_consolidation_paths(&unreadable)
                .expect_err("unreadable destination")
                .code,
            refusal_codes::DESTINATION_ROOT_UNAVAILABLE
        );

        assert!(check_consolidation_paths(&facts()).is_ok());
    }

    // ── FR-025/FR-026 folder resolution ──────────────────────────────────────

    #[test]
    fn a_free_destination_folder_preserves_the_source_root_relative_layout() {
        let resolved = resolve_consolidation_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Shows/Series A"))],
            Vec::new(),
        ));
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Shows/Series A")),
            "FR-026 preserves the whole relative position, however deeply nested"
        );
        assert_eq!(resolved[0].placement, ConsolidationPlacement::UnusedFolder);
        assert!(resolved[0].renamed_to.is_none());
    }

    #[test]
    fn an_empty_destination_directory_is_not_a_collision() {
        let resolved = resolve_consolidation_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Movie (2020)"))],
            vec![("/media/keep/Movie (2020)", DestinationFolderState::Empty)],
        ));
        assert_eq!(resolved[0].placement, ConsolidationPlacement::UnusedFolder);
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Movie (2020)"))
        );
    }

    /// FR-025: two unrelated titles calculating the same destination folder never
    /// merge over the name — the incoming folder gets a unique previewed one.
    #[test]
    fn an_unrelated_title_owning_the_name_gets_the_incoming_folder_uniqued() {
        let resolved = resolve_consolidation_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Movie (2020)"))],
            vec![(
                "/media/keep/Movie (2020)",
                DestinationFolderState::OwnedByTitle {
                    title_id: "other".to_string(),
                },
            )],
        ));
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Movie (2020) (from Old Disk)"))
        );
        assert_eq!(
            resolved[0].placement,
            ConsolidationPlacement::FolderNameCollision {
                collided_name: "Movie (2020)".to_string(),
                occupied_by_title_id: Some("other".to_string()),
            }
        );
        assert_eq!(
            resolved[0].renamed_to.as_deref(),
            Some("Movie (2020) (from Old Disk)"),
            "US5.4: the changed folder name is previewed"
        );
    }

    #[test]
    fn untracked_destination_content_also_forces_a_unique_name() {
        let resolved = resolve_consolidation_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Movie (2020)"))],
            vec![("/media/keep/Movie (2020)", DestinationFolderState::Occupied)],
        ));
        assert!(matches!(
            resolved[0].placement,
            ConsolidationPlacement::FolderNameCollision {
                occupied_by_title_id: None,
                ..
            }
        ));
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Movie (2020) (from Old Disk)"))
        );
    }

    #[test]
    fn a_uniqued_name_that_is_also_taken_is_numbered() {
        let resolved = resolve_consolidation_folders(&resolution(
            vec![resolution_title("t1", Some("/media/old/Movie (2020)"))],
            vec![
                ("/media/keep/Movie (2020)", DestinationFolderState::Occupied),
                (
                    "/media/keep/Movie (2020) (from Old Disk)",
                    DestinationFolderState::Occupied,
                ),
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

        let resolved = resolve_consolidation_folders(&resolution(
            vec![title],
            vec![(
                "/media/keep/Movie 2020",
                DestinationFolderState::OwnedByTitle {
                    title_id: "dest".to_string(),
                },
            )],
        ));
        assert_eq!(
            resolved[0].destination_folder.as_deref(),
            Some(Path::new("/media/keep/Movie 2020")),
            "FR-063: the destination title keeps the folder it already has"
        );
        assert_eq!(resolved[0].placement.merge_target(), Some("dest"));
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
        let resolved = resolve_consolidation_folders(&request);
        assert_eq!(resolved[0].placement, ConsolidationPlacement::UnusedFolder);
        assert_eq!(resolved[1].placement, ConsolidationPlacement::UnusedFolder);

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
        let resolved = resolve_consolidation_folders(&request);
        assert_eq!(resolved[0].placement, ConsolidationPlacement::UnusedFolder);
        assert!(
            matches!(
                resolved[1].placement,
                ConsolidationPlacement::FolderNameCollision { .. }
            ),
            "the second title cannot take a name the first already claimed"
        );
    }

    #[test]
    fn a_fileless_title_resolves_to_no_folder() {
        let resolved = resolve_consolidation_folders(&resolution(
            vec![resolution_title("t1", None)],
            Vec::new(),
        ));
        assert_eq!(resolved[0].placement, ConsolidationPlacement::NoFolder);
        assert!(resolved[0].destination_folder.is_none());
    }

    // ── FR-023/FR-024 accounting and classification ──────────────────────────

    #[test]
    fn every_assigned_title_is_accounted_for_and_the_seven_classifications_close() {
        let moving = draft(
            "t1",
            "/media/old/Alpha (2020)",
            vec![tracked(
                "/media/old/Alpha (2020)/Alpha.mkv",
                100,
                "/media/old/Alpha (2020)",
            )],
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        );
        let mut merging = draft(
            "t2",
            "/media/old/Beta (2021)",
            vec![tracked(
                "/media/old/Beta (2021)/Beta.mkv",
                200,
                "/media/old/Beta (2021)",
            )],
            ResolvedFolder {
                title_id: "t2".to_string(),
                destination_folder: Some(PathBuf::from("/media/keep/Beta")),
                placement: ConsolidationPlacement::MergesWithDestinationTitle {
                    destination_title_id: "dest-2".to_string(),
                    destination_title_name: Some("Beta".to_string()),
                },
                renamed_to: Some("Beta".to_string()),
            },
        );
        merging.destination_entries = vec![DestinationItem::media("Beta.mkv", 999)];
        let colliding = draft(
            "t3",
            "/media/old/Gamma (2022)",
            vec![tracked(
                "/media/old/Gamma (2022)/Gamma.mkv",
                300,
                "/media/old/Gamma (2022)",
            )],
            ResolvedFolder {
                title_id: "t3".to_string(),
                destination_folder: Some(PathBuf::from("/media/keep/Gamma (2022) (from Old Disk)")),
                placement: ConsolidationPlacement::FolderNameCollision {
                    collided_name: "Gamma (2022)".to_string(),
                    occupied_by_title_id: Some("dest-3".to_string()),
                },
                renamed_to: Some("Gamma (2022) (from Old Disk)".to_string()),
            },
        );
        let fileless = ConsolidationTitleDraft {
            source_folder_path: None,
            files: Vec::new(),
            resolved: ResolvedFolder {
                title_id: "t4".to_string(),
                destination_folder: None,
                placement: ConsolidationPlacement::NoFolder,
                renamed_to: None,
            },
            ..draft("t4", "/unused", Vec::new(), resolved_unused("t4", "/unused"))
        };
        let blocked = ConsolidationTitleDraft {
            blocked_reason: Some("an import is running".to_string()),
            blocked_reason_code: Some("active_download_or_import".to_string()),
            ..draft(
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
        let planned = build_root_consolidation_plan(&plan_request);

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

        // The merging title's file collides with an identically named
        // destination file and is renamed rather than overwritten (FR-072/074).
        assert_eq!(planned.classification.media_collisions, 1);
        let merged = planned
            .execution
            .title("t2")
            .expect("the merging title has instructions");
        assert_eq!(merged.merge_target_title_id.as_deref(), Some("dest-2"));
        assert_eq!(merged.renamed_destinations.len(), 1);
        assert!(
            merged.files[0]
                .destination_path
                .starts_with("/media/keep/Beta/"),
            "a merging title's content lands in the destination title's folder: {}",
            merged.files[0].destination_path
        );
    }

    /// FR-024 (5) + FR-073: a file proven identical by full BLAKE3 is
    /// dedup-eligible, and the preview says so before anything is confirmed.
    #[test]
    fn an_identical_file_is_classified_dedup_eligible_and_recycled_not_copied() {
        let mut file = tracked(
            "/media/old/Alpha (2020)/Alpha.mkv",
            100,
            "/media/old/Alpha (2020)",
        );
        file.full_blake3 = FullHash::known("abc123");
        let mut title = draft(
            "t1",
            "/media/old/Alpha (2020)",
            vec![file],
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        );
        title.destination_entries = vec![
            DestinationItem::media("Alpha.mkv", 100)
                .with_content(ContentFacts::new(100).with_full_blake3("abc123")),
        ];

        let planned = build_root_consolidation_plan(&request(vec![title]));
        assert_eq!(planned.classification.dedup_eligible_files, 1);
        assert_eq!(planned.classification.media_collisions, 0);
        let execution = planned.execution.title("t1").expect("instructions");
        assert_eq!(execution.deduplicated_sources.len(), 1);
        assert!(
            execution.files.is_empty(),
            "a proven duplicate writes no bytes"
        );
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Dedup), 1);
    }

    /// SC-003 + FR-073: with the recycle bin unavailable the incoming copy is
    /// preserved under a new name — never permanently deleted.
    #[test]
    fn an_identical_file_is_preserved_and_renamed_when_the_bin_cannot_take_it() {
        let mut file = tracked(
            "/media/old/Alpha (2020)/Alpha.mkv",
            100,
            "/media/old/Alpha (2020)",
        );
        file.full_blake3 = FullHash::known("abc123");
        let mut title = draft(
            "t1",
            "/media/old/Alpha (2020)",
            vec![file],
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        );
        title.destination_entries = vec![
            DestinationItem::media("Alpha.mkv", 100)
                .with_content(ContentFacts::new(100).with_full_blake3("abc123")),
        ];
        title.recycle = RecycleAvailability::Disabled;

        let planned = build_root_consolidation_plan(&request(vec![title]));
        let execution = planned.execution.title("t1").expect("instructions");
        assert!(
            execution.deduplicated_sources.is_empty(),
            "nothing is recycled when the bin cannot take it"
        );
        assert_eq!(execution.renamed_destinations.len(), 1);
        assert_eq!(execution.files.len(), 1, "the incoming copy is preserved");
        assert!(
            planned
                .warnings
                .iter()
                .any(|warning| warning.contains("preserved")),
            "the user is told: {:?}",
            planned.warnings
        );
    }

    /// FR-075: a colliding sidecar is renamed and counted separately from media.
    #[test]
    fn a_colliding_sidecar_is_renamed_and_counted_apart_from_media() {
        let mut title = draft(
            "t1",
            "/media/old/Alpha (2020)",
            vec![
                tracked(
                    "/media/old/Alpha (2020)/Alpha.mkv",
                    100,
                    "/media/old/Alpha (2020)",
                ),
                companion_file(
                    "/media/old/Alpha (2020)/movie.nfo",
                    10,
                    "/media/old/Alpha (2020)",
                ),
            ],
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        );
        title.destination_entries = vec![DestinationItem::companion("movie.nfo", 12)];

        let planned = build_root_consolidation_plan(&request(vec![title]));
        assert_eq!(planned.classification.companion_collisions, 1);
        assert_eq!(planned.classification.media_collisions, 0);
        let execution = planned.execution.title("t1").expect("instructions");
        assert!(
            execution
                .renamed_destinations
                .iter()
                .any(|path| path.contains("movie (from Old Disk)")),
            "renamed to: {:?}",
            execution.renamed_destinations
        );
    }

    /// FR-026: a nested file keeps its position inside the title's folder even
    /// when that folder had to be renamed (FR-025).
    #[test]
    fn a_uniqued_folder_still_preserves_the_layout_inside_it() {
        let title = draft(
            "t1",
            "/media/old/Series A",
            vec![tracked(
                "/media/old/Series A/Season 01/S01E01.mkv",
                50,
                "/media/old/Series A",
            )],
            ResolvedFolder {
                title_id: "t1".to_string(),
                destination_folder: Some(PathBuf::from("/media/keep/Series A (from Old Disk)")),
                placement: ConsolidationPlacement::FolderNameCollision {
                    collided_name: "Series A".to_string(),
                    occupied_by_title_id: Some("other".to_string()),
                },
                renamed_to: Some("Series A (from Old Disk)".to_string()),
            },
        );
        let planned = build_root_consolidation_plan(&request(vec![title]));
        let execution = planned.execution.title("t1").expect("instructions");
        assert_eq!(
            execution.files[0].destination_path,
            "/media/keep/Series A (from Old Disk)/Season 01/S01E01.mkv"
        );
        assert_eq!(planned.plan.counts.for_kind(PlanItemKind::Rename), 1);
        // The rename is the *folder*, and a folder is not a file. The count the
        // user types a confirmation phrase against has to be the number of
        // files that actually move.
        assert_eq!(
            planned.plan.counts.files_total, 1,
            "one file moves; the folder rename beside it is not a second file"
        );
    }

    /// The count the typed confirmation confirms is the number of files, and a
    /// consolidation emits both kinds of `Rename`: one folder uniquing and one
    /// media collision. Only the second is a file.
    #[test]
    fn the_file_count_counts_files_and_not_the_folder_rename_beside_them() {
        let uniqued = draft(
            "t1",
            "/media/old/Alpha (2020)",
            vec![tracked(
                "/media/old/Alpha (2020)/Alpha.mkv",
                50,
                "/media/old/Alpha (2020)",
            )],
            ResolvedFolder {
                title_id: "t1".to_string(),
                destination_folder: Some(PathBuf::from("/media/keep/Alpha (2020) (from Old Disk)")),
                placement: ConsolidationPlacement::FolderNameCollision {
                    collided_name: "Alpha (2020)".to_string(),
                    occupied_by_title_id: Some("other".to_string()),
                },
                renamed_to: Some("Alpha (2020) (from Old Disk)".to_string()),
            },
        );
        let mut colliding = draft(
            "t2",
            "/media/old/Beta (2021)",
            vec![tracked(
                "/media/old/Beta (2021)/Beta.mkv",
                60,
                "/media/old/Beta (2021)",
            )],
            resolved_unused("t2", "/media/keep/Beta (2021)"),
        );
        // The destination folder already holds a file of the same name whose
        // content differs, so FR-074 renames the incoming one.
        colliding.destination_entries = vec![DestinationItem::media("Beta.mkv", 61)];

        let planned = build_root_consolidation_plan(&request(vec![uniqued, colliding]));
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
            planned.plan.counts.files_total,
            2,
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

        let mut plan_request = request(vec![draft(
            "t1",
            "/media/old/Alpha (2020)",
            Vec::new(),
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        )]);
        plan_request.default_transfer = transfer;
        let planned = build_root_consolidation_plan(&plan_request);
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
                    && item.detail.as_deref().is_some_and(|detail| {
                        detail.contains("becomes the default")
                    })
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

    // ── FR-023/FR-028/FR-029 ─────────────────────────────────────────────────

    #[test]
    fn a_blocked_title_blocks_the_start_and_the_source_retirement() {
        let blocked = ConsolidationTitleDraft {
            blocked_reason: Some("an import is running for \"Held\"".to_string()),
            blocked_reason_code: Some("active_download_or_import".to_string()),
            ..draft(
                "t1",
                "/media/old/Held (2019)",
                Vec::new(),
                resolved_unused("t1", "/media/keep/Held (2019)"),
            )
        };
        let planned = build_root_consolidation_plan(&request(vec![blocked]));

        assert!(planned.plan.blocks_start());
        assert!(matches!(
            planned.plan.confirm(&PlanConfirmationRequest {
                fingerprint: planned.plan.fingerprint.clone(),
                typed_confirmation: Some(LOCATION_TYPED_CONFIRMATION_PHRASE.to_string()),
            }),
            Err(PlanConfirmationError::Blocked)
        ));
        assert!(
            planned
                .retirement
                .blocker(retirement_blockers::BLOCKED_TITLES)
                .is_some()
        );
        assert!(!planned.retirement.permits_source_removal());
        assert_eq!(planned.execution.unresolved_titles, 1);
    }

    #[test]
    fn unexplained_source_content_keeps_the_source_root_configured_without_stopping_the_move() {
        let mut plan_request = request(vec![draft(
            "t1",
            "/media/old/Alpha (2020)",
            vec![tracked(
                "/media/old/Alpha (2020)/Alpha.mkv",
                100,
                "/media/old/Alpha (2020)",
            )],
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        )]);
        plan_request.entries = vec![
            RootEntry::file("/media/old/Alpha (2020)/Alpha.mkv", 100),
            RootEntry::file("/media/old/someone-elses.txt", 9),
        ];
        let planned = build_root_consolidation_plan(&plan_request);

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
    fn a_consolidation_requires_the_shared_typed_confirmation() {
        let planned = build_root_consolidation_plan(&request(vec![draft(
            "t1",
            "/media/old/Alpha (2020)",
            vec![tracked(
                "/media/old/Alpha (2020)/Alpha.mkv",
                100,
                "/media/old/Alpha (2020)",
            )],
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        )]));

        assert_eq!(
            planned.plan.header.operation_type,
            LocationOperationType::RootConsolidation
        );
        assert_eq!(
            planned.plan.confirmation.typed_phrase.as_deref(),
            Some(LOCATION_TYPED_CONFIRMATION_PHRASE)
        );
        assert!(matches!(
            planned.plan.confirm(&PlanConfirmationRequest {
                fingerprint: planned.plan.fingerprint.clone(),
                typed_confirmation: None,
            }),
            Err(PlanConfirmationError::TypedConfirmationRequired)
        ));
        assert!(matches!(
            planned.plan.confirm(&PlanConfirmationRequest {
                fingerprint: planned.plan.fingerprint.clone(),
                typed_confirmation: Some("move".to_string()),
            }),
            Err(PlanConfirmationError::TypedConfirmationMismatch)
        ));
        assert!(
            planned
                .plan
                .confirm(&PlanConfirmationRequest {
                    fingerprint: planned.plan.fingerprint.clone(),
                    typed_confirmation: Some(LOCATION_TYPED_CONFIRMATION_PHRASE.to_string()),
                })
                .is_ok()
        );
    }

    #[test]
    fn the_plan_carries_two_different_root_ids_in_one_library() {
        let planned = build_root_consolidation_plan(&request(vec![draft(
            "t1",
            "/media/old/Alpha (2020)",
            Vec::new(),
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        )]));
        assert_eq!(planned.plan.header.source_root_id.as_deref(), Some("root-old"));
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
    fn a_consolidation_with_nothing_on_disk_needs_no_move_mode() {
        let fileless = ConsolidationTitleDraft {
            source_folder_path: None,
            resolved: ResolvedFolder {
                title_id: "t1".to_string(),
                destination_folder: None,
                placement: ConsolidationPlacement::NoFolder,
                renamed_to: None,
            },
            ..draft("t1", "/unused", Vec::new(), resolved_unused("t1", "/unused"))
        };
        let planned = build_root_consolidation_plan(&request(vec![fileless]));
        assert_eq!(planned.plan.header.mode, LocationExecutionMode::CatalogOnly);
    }

    #[test]
    fn a_same_named_destination_title_is_named_and_never_merged_into() {
        let mut title = draft(
            "t1",
            "/media/old/Alpha (2020)",
            Vec::new(),
            ResolvedFolder {
                title_id: "t1".to_string(),
                destination_folder: Some(PathBuf::from("/media/keep/Alpha (2020) (from Old Disk)")),
                placement: ConsolidationPlacement::FolderNameCollision {
                    collided_name: "Alpha (2020)".to_string(),
                    occupied_by_title_id: Some("dest".to_string()),
                },
                renamed_to: Some("Alpha (2020) (from Old Disk)".to_string()),
            },
        );
        title.destination_identity = Some(DestinationIdentityOutcome {
            match_kind: DestinationIdentityMatch::SameNameNoIdentity,
            matched_title_id: None,
            candidates: Vec::<IdentityCandidate>::new(),
            same_name_title_id: Some("dest".to_string()),
            same_name_title_name: Some("Alpha".to_string()),
        });

        let planned = build_root_consolidation_plan(&request(vec![title]));
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

    #[test]
    fn a_cross_volume_consolidation_warns_about_hardlinked_sources() {
        let mut title = draft(
            "t1",
            "/media/old/Alpha (2020)",
            vec![tracked(
                "/media/old/Alpha (2020)/Alpha.mkv",
                100,
                "/media/old/Alpha (2020)",
            )],
            resolved_unused("t1", "/media/keep/Alpha (2020)"),
        );
        title.hardlinks = vec![HardlinkFact {
            path: "/media/old/Alpha (2020)/Alpha.mkv".to_string(),
            link_count: LinkCount::Known(2),
            size_bytes: 100,
        }];
        let planned = build_root_consolidation_plan(&request(vec![title]));
        assert!(
            planned
                .warnings
                .iter()
                .any(|warning| warning.to_lowercase().contains("link")),
            "{:?}",
            planned.warnings
        );
    }

    #[test]
    fn new_unknown_content_at_the_source_voids_the_confirmation() {
        let title = || {
            draft(
                "t1",
                "/media/old/Alpha (2020)",
                vec![tracked(
                    "/media/old/Alpha (2020)/Alpha.mkv",
                    100,
                    "/media/old/Alpha (2020)",
                )],
                resolved_unused("t1", "/media/keep/Alpha (2020)"),
            )
        };
        let mut before = request(vec![title()]);
        before.entries = vec![RootEntry::file("/media/old/Alpha (2020)/Alpha.mkv", 100)];
        let mut after = request(vec![title()]);
        after.entries = vec![
            RootEntry::file("/media/old/Alpha (2020)/Alpha.mkv", 100),
            RootEntry::file("/media/old/appeared.txt", 4),
        ];

        let before = build_root_consolidation_plan(&before);
        let after = build_root_consolidation_plan(&after);
        assert_ne!(
            before.plan.fingerprint, after.plan.fingerprint,
            "FR-081: unexplained content is fingerprinted material"
        );
    }
}
