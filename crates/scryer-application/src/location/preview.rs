//! Shared preview core: every location workflow builds the same fingerprinted
//! plan (D6, FR-080–082).
//!
//! Large previews return complete counts with a sampled item list, following the
//! established `LibraryRenamePlan` pattern; the fingerprint always covers the
//! full plan, not the sample (FR-081).
//!
//! This module is deliberately workflow-agnostic. It owns:
//!
//! - the typed plan-item vocabulary every workflow expresses its plan in,
//! - the builder that turns a full item list into complete counts plus samples,
//! - the full-plan fingerprint and the confirmation check that validates it,
//! - free-space estimation, including the extra copy cost when the recycle bin
//!   lives on another volume (FR-080),
//! - the verification-depth statement (FR-042/043),
//! - the typed-confirmation hook for root-wide operations (FR-029/082).
//!
//! The per-workflow planners (root move, root change, consolidation, transfer,
//! adoption) build [`PlanItem`]s and hand them to [`LocationPlanBuilder`]; they
//! never invent their own fingerprint or confirmation rules.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::helpers::{HashDomain, blake3_identity_hex};
use crate::location::classify::ClassificationCounts;
use crate::location::merge::summary::MergePreviewSummary;
use crate::location::model::{
    LocationExecutionMode, LocationOperationType, VerificationDepth,
};

/// How many items of each section a preview returns alongside the complete
/// count. Mirrors the rename-plan/delete-preview sampling contract (FR-081).
pub const PLAN_SECTION_SAMPLE_LIMIT: usize = 25;

/// The word a user types to confirm a root-wide operation (FR-029, FR-082),
/// following the `user_delete.rs` typed-confirmation contract.
pub const LOCATION_TYPED_CONFIRMATION_PHRASE: &str = "MOVE";

/// The prompt shown beside the typed-confirmation field.
pub const LOCATION_TYPED_CONFIRMATION_PROMPT: &str =
    "Type MOVE to confirm this root-wide operation.";

/// Fingerprint over the complete plan. A changed filesystem, catalog, selection,
/// or destination produces a different fingerprint and voids the confirmation
/// (FR-081, C2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PlanFingerprint(pub String);

impl PlanFingerprint {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A bounded window over a plan section: the complete count plus the items the
/// UI actually renders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SampledPlanItems<T> {
    /// Complete count for this section across the whole plan.
    pub total: i64,
    /// The sampled subset returned to the caller.
    pub items: Vec<T>,
}

impl<T> SampledPlanItems<T> {
    /// True when `items` holds every item in the section.
    pub fn is_complete(&self) -> bool {
        self.items.len() as i64 == self.total
    }
}

/// How much consent an operation demands, scaling with blast radius (C2, FR-082).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationRequirement {
    /// A simple confirm suffices.
    Simple,
    /// Root-wide operations require typed confirmation (FR-029), reusing the
    /// established `requires_typed_confirmation` pattern.
    Typed,
}

/// Every kind of change a location plan can contain (FR-080). One vocabulary for
/// all six workflows, so Activity and the preview UI are written once.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PlanItemKind {
    /// Content moves from a source path to a destination path.
    Move,
    /// Content keeps its directory but changes name (folder-name repair,
    /// collision disambiguation, sidecar follow-renames).
    Rename,
    /// A source title folds into an existing destination title (US7).
    Merge,
    /// A proven-duplicate file is recycled rather than moved (FR-073).
    Dedup,
    /// Catalog-only change: ownership flip, folder-match correction, root
    /// reference update — no bytes move (FR-014, FR-076).
    CatalogChange,
    /// A media file's role for its logical slot changes (FR-068–070).
    RoleChange,
    /// The title already satisfies the request; nothing happens.
    NoOp,
    /// The title cannot enter the operation until the user resolves something
    /// (FR-016, FR-086).
    Blocked,
    /// Content at the source that Scryer does not manage, surfaced rather than
    /// abandoned (FR-027).
    UnmanagedContent,
    /// Something the user must see before confirming: hardlinked sources
    /// (FR-085), preserve-instead-of-recycle (FR-073), broken seeding copies.
    Warning,
}

impl PlanItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Rename => "rename",
            Self::Merge => "merge",
            Self::Dedup => "dedup",
            Self::CatalogChange => "catalog_change",
            Self::RoleChange => "role_change",
            Self::NoOp => "no_op",
            Self::Blocked => "blocked",
            Self::UnmanagedContent => "unmanaged_content",
            Self::Warning => "warning",
        }
    }

    /// Kinds whose presence stops the operation from starting until the user
    /// resolves them (FR-016).
    pub fn blocks_start(&self) -> bool {
        matches!(self, Self::Blocked)
    }
}

/// One previewed change. Item bodies are what the fingerprint is taken over, so
/// every field here is part of "the plan changed" (FR-081).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanItem {
    pub kind: PlanItemKind,
    /// Title this item belongs to, when it belongs to one.
    pub title_id: Option<String>,
    /// Media file this item acts on, when it acts on tracked media.
    pub media_file_id: Option<String>,
    pub source_path: Option<String>,
    pub destination_path: Option<String>,
    /// Bytes this item accounts for; zero for catalog-only items.
    pub size_bytes: u64,
    /// Whether source and destination share a volume, where the planner knows
    /// (FR-080). `None` means "not determined".
    pub same_volume: Option<bool>,
    /// Machine-readable reason, for grouping and i18n.
    pub reason_code: Option<String>,
    /// Human-readable explanation, required for blocking and warning kinds (C3).
    pub detail: Option<String>,
}

impl PlanItem {
    /// A plain item of `kind` with everything else empty.
    pub fn new(kind: PlanItemKind) -> Self {
        Self {
            kind,
            title_id: None,
            media_file_id: None,
            source_path: None,
            destination_path: None,
            size_bytes: 0,
            same_volume: None,
            reason_code: None,
            detail: None,
        }
    }

    pub fn with_title(mut self, title_id: impl Into<String>) -> Self {
        self.title_id = Some(title_id.into());
        self
    }

    pub fn with_paths(
        mut self,
        source: Option<impl Into<String>>,
        destination: Option<impl Into<String>>,
    ) -> Self {
        self.source_path = source.map(Into::into);
        self.destination_path = destination.map(Into::into);
        self
    }

    pub fn with_size(mut self, size_bytes: u64) -> Self {
        self.size_bytes = size_bytes;
        self
    }

    pub fn with_same_volume(mut self, same_volume: bool) -> Self {
        self.same_volume = Some(same_volume);
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_reason_code(mut self, reason_code: impl Into<String>) -> Self {
        self.reason_code = Some(reason_code.into());
        self
    }
}

/// One section of a plan: every item of one kind, as a complete count with a
/// sample (FR-081).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanSection {
    pub kind: PlanItemKind,
    pub items: SampledPlanItems<PlanItem>,
    /// Complete byte total for the section, not just the sampled items.
    pub bytes_total: i64,
}

/// The scope-defining header of a plan: what is being done, from where, to
/// where. Part of the fingerprint, so changing the destination invalidates the
/// confirmation (FR-081).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocationPlanHeader {
    pub operation_type: LocationOperationType,
    pub mode: LocationExecutionMode,
    pub source_library_id: Option<String>,
    pub destination_library_id: Option<String>,
    pub source_root_id: Option<String>,
    pub destination_root_id: Option<String>,
    /// The selection this plan was built for, in a stable order. A changed
    /// selection must change the fingerprint even when the resulting items
    /// happen to coincide (FR-081).
    pub selection: Vec<String>,
}

impl LocationPlanHeader {
    pub fn new(operation_type: LocationOperationType, mode: LocationExecutionMode) -> Self {
        Self {
            operation_type,
            mode,
            source_library_id: None,
            destination_library_id: None,
            source_root_id: None,
            destination_root_id: None,
            selection: Vec::new(),
        }
    }

    pub fn with_source(
        mut self,
        library_id: Option<String>,
        root_id: Option<String>,
    ) -> Self {
        self.source_library_id = library_id;
        self.source_root_id = root_id;
        self
    }

    pub fn with_destination(
        mut self,
        library_id: Option<String>,
        root_id: Option<String>,
    ) -> Self {
        self.destination_library_id = library_id;
        self.destination_root_id = root_id;
        self
    }

    /// Records the selection, sorted and de-duplicated so the fingerprint does
    /// not depend on the order the caller happened to collect ids in.
    pub fn with_selection(mut self, selection: impl IntoIterator<Item = String>) -> Self {
        let mut selection: Vec<String> = selection.into_iter().collect();
        selection.sort();
        selection.dedup();
        self.selection = selection;
        self
    }
}

/// Complete counts across the whole plan (FR-080/081).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PlanCounts {
    pub items_total: i64,
    pub titles_total: i64,
    pub files_total: i64,
    pub bytes_total: i64,
    /// Per-kind complete counts, including kinds with no sampled items.
    pub by_kind: BTreeMap<String, i64>,
}

impl PlanCounts {
    pub fn for_kind(&self, kind: PlanItemKind) -> i64 {
        self.by_kind.get(kind.as_str()).copied().unwrap_or(0)
    }
}

/// The depth statement the preview makes before anything moves (FR-042/043).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationStatement {
    /// Depth resolved from the user preference at preview time.
    pub depth: VerificationDepth,
    /// Files this depth will be applied to.
    pub files: i64,
    /// Bytes this depth will be applied to.
    pub bytes: i64,
}

impl VerificationStatement {
    /// A statement for a plan that copies nothing: same-volume renames and
    /// catalog-only work need no verification pass (FR-032).
    pub fn none(depth: VerificationDepth) -> Self {
        Self {
            depth,
            files: 0,
            bytes: 0,
        }
    }

    /// Whether any file in this plan will actually be verified.
    pub fn applies(&self) -> bool {
        self.files > 0
    }
}

/// The confirmation this plan demands (FR-029, FR-082).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanConfirmation {
    pub requirement: ConfirmationRequirement,
    /// The phrase the user must type, when typed confirmation applies.
    pub typed_phrase: Option<String>,
    /// The prompt shown beside the field, when typed confirmation applies.
    pub typed_prompt: Option<String>,
}

impl PlanConfirmation {
    /// Derives the requirement from the operation type, reusing
    /// [`LocationOperationType::requires_typed_confirmation`] so root-wide
    /// operations can never lose their stronger gate (FR-029).
    pub fn for_operation(operation_type: LocationOperationType) -> Self {
        if operation_type.requires_typed_confirmation() {
            Self {
                requirement: ConfirmationRequirement::Typed,
                typed_phrase: Some(LOCATION_TYPED_CONFIRMATION_PHRASE.to_string()),
                typed_prompt: Some(LOCATION_TYPED_CONFIRMATION_PROMPT.to_string()),
            }
        } else {
            Self {
                requirement: ConfirmationRequirement::Simple,
                typed_phrase: None,
                typed_prompt: None,
            }
        }
    }

    pub fn requires_typed_confirmation(&self) -> bool {
        matches!(self.requirement, ConfirmationRequirement::Typed)
    }
}

/// The complete preview handed to the user and echoed back on confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocationPlan {
    pub header: LocationPlanHeader,
    /// Fingerprint over the **full** plan — header, selection, and every item —
    /// never over the sampled subset (FR-081).
    pub fingerprint: PlanFingerprint,
    pub counts: PlanCounts,
    /// Sections in a stable kind order, each a complete count plus a sample.
    pub sections: Vec<PlanSection>,
    /// Per-class title counts for the selection (FR-015).
    pub classification: ClassificationCounts,
    pub free_space: FreeSpaceEstimate,
    pub verification: VerificationStatement,
    pub confirmation: PlanConfirmation,
    /// The FR-071 merge summary for every title in this plan that merges into
    /// an existing destination title (US7), in plan order. Empty for every plan
    /// with no merge in it, which is every plan outside a cross-library
    /// transfer.
    ///
    /// This is part of the fingerprinted material (see
    /// [`build_plan_fingerprint`]) because the merge decision is derived from
    /// catalog state — the destination title's episodes, collections, links,
    /// tags, and episode-scoped rows — that no plan *item* mentions. Leaving it
    /// out would let the destination gain an episode between preview and start
    /// and still confirm a fingerprint that no longer describes the merge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merges: Vec<MergePreviewSummary>,
}

impl LocationPlan {
    /// Section for one kind, if the plan has any items of it.
    pub fn section(&self, kind: PlanItemKind) -> Option<&PlanSection> {
        self.sections.iter().find(|section| section.kind == kind)
    }

    /// Blocking items or blocking classifications stop the operation from
    /// starting until the user resolves or deselects them (FR-016).
    pub fn blocks_start(&self) -> bool {
        self.counts.for_kind(PlanItemKind::Blocked) > 0 || self.classification.blocks_start()
    }

    /// Validates a confirmation against this plan (FR-081, FR-082, FR-080).
    ///
    /// Order matters: a stale plan is reported before a missing typed phrase, so
    /// a user is never asked to retype a confirmation for a plan that is about
    /// to be regenerated anyway — and the same reasoning puts the space check
    /// ahead of the typed phrase.
    pub fn confirm(&self, request: &PlanConfirmationRequest) -> Result<(), PlanConfirmationError> {
        if request.fingerprint != self.fingerprint {
            return Err(PlanConfirmationError::Stale);
        }
        if self.blocks_start() {
            return Err(PlanConfirmationError::Blocked);
        }
        // FR-080: a measured shortfall is a refusal, because starting would fill
        // the destination volume and then fail partway through a title. An
        // *unmeasured* volume is not — `sufficient()` answers `None` when it was
        // never probed or could not be read, and refusing on "unknown" would
        // block every move onto a volume Scryer cannot stat.
        if self.free_space.sufficient() == Some(false) {
            return Err(PlanConfirmationError::InsufficientSpace);
        }
        if self.confirmation.requires_typed_confirmation() {
            let phrase = request
                .typed_confirmation
                .as_deref()
                .map(str::trim)
                .unwrap_or_default();
            if phrase.is_empty() {
                return Err(PlanConfirmationError::TypedConfirmationRequired);
            }
            if phrase != LOCATION_TYPED_CONFIRMATION_PHRASE {
                return Err(PlanConfirmationError::TypedConfirmationMismatch);
            }
        }
        Ok(())
    }
}

/// What a caller sends back when confirming a previewed plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanConfirmationRequest {
    pub fingerprint: PlanFingerprint,
    /// The phrase the user typed, for root-wide operations (FR-029).
    pub typed_confirmation: Option<String>,
}

/// Why a confirmation was refused.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PlanConfirmationError {
    /// The plan no longer describes reality; the user must re-preview (FR-081).
    Stale,
    /// Items still need a user decision (FR-016).
    Blocked,
    /// The destination — or the recycle bin's volume — was measured and does
    /// not have room for what the plan would write (FR-080). Unlike a stale
    /// plan, re-previewing does not fix this: the user has to free space or
    /// move less.
    InsufficientSpace,
    /// A root-wide operation was confirmed without the typed phrase.
    TypedConfirmationRequired,
    /// The typed phrase did not match.
    TypedConfirmationMismatch,
}

impl PlanConfirmationError {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stale => "stale_plan",
            Self::Blocked => "blocked_items",
            Self::InsufficientSpace => "insufficient_space",
            Self::TypedConfirmationRequired => "typed_confirmation_required",
            Self::TypedConfirmationMismatch => "typed_confirmation_mismatch",
        }
    }
}

/// A change observed underneath a running or confirmed plan.
///
/// This is the vocabulary behind FR-089's carve-out: the staleness check covers
/// **catalog inputs and items the operation has not processed yet**; expected
/// partial destination state left by an interrupted copy is the operation's own
/// work in progress and is resumable, not stale.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PlanInputChange {
    /// A catalog input the plan was built from changed (title moved, files
    /// added or removed, folder ownership reassigned).
    CatalogInput,
    /// A source item the operation has not reached yet changed on disk.
    UnprocessedSourceItem,
    /// The selection changed.
    Selection,
    /// The destination library, root, or path configuration changed.
    Destination,
    /// Destination content this operation itself wrote and verified.
    VerifiedDestinationFile,
    /// Destination content this operation itself started writing and did not
    /// finish — the interrupted-copy partial FR-089 explicitly allows.
    ExpectedDestinationPartial,
    /// Source content this operation already processed and settled.
    SettledSourceItem,
}

impl PlanInputChange {
    /// Whether this change invalidates the plan.
    ///
    /// The two destination-side variants and settled source items are the
    /// operation's own footprint: treating them as staleness would make every
    /// interrupted operation unresumable, which is exactly what FR-089 forbids.
    pub fn is_stale(&self) -> bool {
        match self {
            Self::CatalogInput
            | Self::UnprocessedSourceItem
            | Self::Selection
            | Self::Destination => true,
            Self::VerifiedDestinationFile
            | Self::ExpectedDestinationPartial
            | Self::SettledSourceItem => false,
        }
    }
}

/// Builds a [`LocationPlan`] from the full item list.
///
/// The builder never samples before fingerprinting: the fingerprint is taken
/// over the complete list, and only then are sections sampled (FR-081).
pub struct LocationPlanBuilder {
    header: LocationPlanHeader,
    items: Vec<PlanItem>,
    classification: ClassificationCounts,
    free_space: FreeSpaceEstimate,
    verification: VerificationStatement,
    merges: Vec<MergePreviewSummary>,
    sample_limit: usize,
}

impl LocationPlanBuilder {
    pub fn new(header: LocationPlanHeader) -> Self {
        let depth = VerificationDepth::default();
        Self {
            header,
            items: Vec::new(),
            classification: ClassificationCounts::default(),
            free_space: FreeSpaceEstimate::unknown(),
            verification: VerificationStatement::none(depth),
            merges: Vec::new(),
            sample_limit: PLAN_SECTION_SAMPLE_LIMIT,
        }
    }

    /// Record one title's FR-071 merge summary. Order is the caller's, and it
    /// is the order the fingerprint is taken over.
    pub fn merge(&mut self, summary: MergePreviewSummary) -> &mut Self {
        self.merges.push(summary);
        self
    }

    pub fn push(&mut self, item: PlanItem) -> &mut Self {
        self.items.push(item);
        self
    }

    pub fn extend(&mut self, items: impl IntoIterator<Item = PlanItem>) -> &mut Self {
        self.items.extend(items);
        self
    }

    pub fn classification(&mut self, counts: ClassificationCounts) -> &mut Self {
        self.classification = counts;
        self
    }

    pub fn free_space(&mut self, estimate: FreeSpaceEstimate) -> &mut Self {
        self.free_space = estimate;
        self
    }

    /// Sets the depth statement (FR-042/043).
    ///
    /// The depth itself comes from the user preference, which only the use-case
    /// layer can read: a workflow planner resolves it with
    /// `AppUseCase::resolve_verification_depth` and passes it here, so this
    /// module stays free of settings access. The file and byte totals are
    /// derived from the plan's copied content, which is the only content a
    /// verification pass applies to.
    pub fn verification_depth(&mut self, depth: VerificationDepth) -> &mut Self {
        self.verification = VerificationStatement::none(depth);
        self
    }

    pub fn sample_limit(&mut self, limit: usize) -> &mut Self {
        self.sample_limit = limit;
        self
    }

    pub fn build(&self) -> LocationPlan {
        let fingerprint = build_plan_fingerprint(&self.header, &self.items, &self.merges);
        let counts = build_counts(&self.items);
        let sections = build_sections(&self.items, self.sample_limit);
        let verification = VerificationStatement {
            depth: self.verification.depth,
            files: verified_file_count(&self.items),
            bytes: verified_byte_count(&self.items),
        };

        LocationPlan {
            confirmation: PlanConfirmation::for_operation(self.header.operation_type),
            header: self.header.clone(),
            fingerprint,
            counts,
            sections,
            classification: self.classification,
            free_space: self.free_space.clone(),
            verification,
            merges: self.merges.clone(),
        }
    }
}

/// Fingerprint over the complete plan: header, selection, every item in the
/// order the planner produced them, and every merge summary (FR-081).
///
/// Domain-separated through [`HashDomain::LocationPlan`], the same way the
/// rename plan and delete preview are, so a plan fingerprint can never be
/// substituted for another kind of identity.
///
/// # Why the merge summaries are hashed and not only the items
///
/// Everything else a plan decides is visible in an item: a path, a size, a
/// reason code. A merge is decided from catalog state on *both* sides — the
/// destination's episodes, collections, links, tags, and every episode-scoped
/// row referencing the source — and the plan items can only ever carry a
/// readable digest of that. Hashing [`MergePreviewSummary`] itself is what
/// makes FR-081's guarantee hold for US7: if a destination episode appears
/// between preview and start, the identity map changes, the summary changes,
/// and the confirmation the user is holding is refused as stale rather than
/// executing a merge nobody previewed.
pub fn build_plan_fingerprint(
    header: &LocationPlanHeader,
    items: &[PlanItem],
    merges: &[MergePreviewSummary],
) -> PlanFingerprint {
    let payload = serde_json::to_string(&(header, items, merges)).unwrap_or_default();
    PlanFingerprint(blake3_identity_hex(HashDomain::LocationPlan, payload))
}

fn build_counts(items: &[PlanItem]) -> PlanCounts {
    let mut by_kind: BTreeMap<String, i64> = BTreeMap::new();
    let mut titles: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut files_total = 0_i64;
    let mut bytes_total = 0_i64;

    for item in items {
        *by_kind.entry(item.kind.as_str().to_string()).or_insert(0) += 1;
        if let Some(title_id) = item.title_id.as_deref() {
            titles.insert(title_id);
        }
        if item.media_file_id.is_some() || item.source_path.is_some() {
            files_total += 1;
        }
        bytes_total = bytes_total.saturating_add(item.size_bytes as i64);
    }

    PlanCounts {
        items_total: items.len() as i64,
        titles_total: titles.len() as i64,
        files_total,
        bytes_total,
        by_kind,
    }
}

fn build_sections(items: &[PlanItem], sample_limit: usize) -> Vec<PlanSection> {
    let mut grouped: BTreeMap<PlanItemKind, (i64, i64, Vec<PlanItem>)> = BTreeMap::new();
    for item in items {
        let entry = grouped
            .entry(item.kind)
            .or_insert_with(|| (0, 0, Vec::new()));
        entry.0 += 1;
        entry.1 = entry.1.saturating_add(item.size_bytes as i64);
        if entry.2.len() < sample_limit {
            entry.2.push(item.clone());
        }
    }

    grouped
        .into_iter()
        .map(|(kind, (total, bytes_total, sample))| PlanSection {
            kind,
            items: SampledPlanItems {
                total,
                items: sample,
            },
            bytes_total,
        })
        .collect()
}

/// Files a verification pass will actually run over: content that is copied to
/// a different volume. Same-volume renames need no pass (FR-032).
fn verified_file_count(items: &[PlanItem]) -> i64 {
    items.iter().filter(|item| item_is_copied(item)).count() as i64
}

fn verified_byte_count(items: &[PlanItem]) -> i64 {
    items
        .iter()
        .filter(|item| item_is_copied(item))
        .fold(0_i64, |total, item| {
            total.saturating_add(item.size_bytes as i64)
        })
}

fn item_is_copied(item: &PlanItem) -> bool {
    matches!(item.kind, PlanItemKind::Move) && item.same_volume != Some(true)
}

/// Identity of the filesystem backing a path, for same-volume decisions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VolumeId(pub String);

/// Reads volume identity and free space. A seam so previews are testable without
/// real mounts, and so a platform that cannot answer degrades to "unknown"
/// rather than to a wrong answer.
pub trait VolumeProbe: Send + Sync {
    /// Volume backing `path`, or the nearest existing ancestor when the path is
    /// not created yet. `None` when the platform cannot say.
    fn volume_id(&self, path: &Path) -> Option<VolumeId>;

    /// Bytes an unprivileged writer can still use on that volume.
    fn available_bytes(&self, path: &Path) -> Option<u64>;
}

/// The real probe: `dev` identity on unix, path prefix on Windows, and the
/// established `filesystem_space_raw` helper for free space.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemVolumeProbe;

impl SystemVolumeProbe {
    /// Destination directories usually do not exist yet, so both queries walk up
    /// to the nearest ancestor that does.
    fn existing_ancestor(path: &Path) -> Option<PathBuf> {
        let mut current = Some(path);
        while let Some(candidate) = current {
            if candidate.exists() {
                return Some(candidate.to_path_buf());
            }
            current = candidate.parent();
        }
        None
    }
}

impl VolumeProbe for SystemVolumeProbe {
    fn volume_id(&self, path: &Path) -> Option<VolumeId> {
        let existing = Self::existing_ancestor(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::metadata(&existing).ok()?;
            Some(VolumeId(format!("dev:{}", metadata.dev())))
        }
        #[cfg(not(unix))]
        {
            // Windows has no cheap device id through `std`; the path prefix
            // (drive letter or UNC share) is the volume for the purposes of
            // "will this move be a rename or a copy".
            let prefix = existing.components().next()?;
            Some(VolumeId(format!(
                "prefix:{}",
                prefix.as_os_str().to_string_lossy().to_uppercase()
            )))
        }
    }

    fn available_bytes(&self, path: &Path) -> Option<u64> {
        let existing = Self::existing_ancestor(path)?;
        crate::helpers::filesystem_space_raw(&existing)
            .ok()
            .map(|space| space.available_bytes)
    }
}

/// What a free-space estimate needs to know (FR-080).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeSpaceRequest {
    /// A path on the source volume.
    pub source_path: PathBuf,
    /// A path on the destination volume.
    pub destination_path: PathBuf,
    /// Bytes the operation will write at the destination.
    pub moved_bytes: u64,
    /// Bytes of source content the operation will recycle once its destination
    /// copy is verified.
    pub recycled_bytes: u64,
    /// The configured recycle-bin root (`recycle_bin.rs`'s `base_path`), or
    /// `None` when recycling is disabled or unavailable for this operation.
    pub recycle_base_path: Option<PathBuf>,
}

/// Free-space expectation stated in the preview (FR-080).
///
/// Recycling is the subtle part: recycling a file on the *same* volume is a
/// rename and costs nothing, but a recycle bin configured on another volume
/// makes every recycled file a second full copy. That cost is part of what the
/// user must be told before confirming.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FreeSpaceEstimate {
    /// Bytes that must be free on the destination volume.
    pub destination_required_bytes: u64,
    pub destination_available_bytes: Option<u64>,
    /// Bytes that must be free wherever the recycle bin lives. Zero when
    /// recycling is a same-volume rename or is unavailable.
    pub recycle_required_bytes: u64,
    pub recycle_available_bytes: Option<u64>,
    /// The move is a same-volume rename, so it needs no destination space
    /// (FR-032).
    pub same_volume_move: bool,
    /// The recycle bin is on a different volume from the source, so recycling
    /// copies bytes instead of renaming them (FR-080).
    pub recycle_on_other_volume: bool,
    /// The recycle bin's volume is the destination volume, so its cost adds to
    /// the destination requirement rather than standing on its own.
    pub recycle_shares_destination_volume: bool,
    /// Recycling is configured and available for this operation.
    pub recycling_available: bool,
    /// The volumes behind this estimate were actually probed. An unprobed
    /// estimate reports "unknown", never "enough space".
    pub probed: bool,
}

impl FreeSpaceEstimate {
    /// The estimate for a plan whose volumes have not been probed. Everything is
    /// unknown rather than optimistic: [`Self::sufficient`] answers `None`, and
    /// the caller must surface "could not determine" instead of "enough space".
    pub fn unknown() -> Self {
        Self {
            destination_required_bytes: 0,
            destination_available_bytes: None,
            recycle_required_bytes: 0,
            recycle_available_bytes: None,
            same_volume_move: false,
            recycle_on_other_volume: false,
            recycle_shares_destination_volume: false,
            recycling_available: false,
            probed: false,
        }
    }

    /// Total bytes that must be free on the destination volume, including the
    /// recycle-copy cost when the bin shares that volume.
    pub fn destination_total_required_bytes(&self) -> u64 {
        if self.recycle_shares_destination_volume {
            self.destination_required_bytes
                .saturating_add(self.recycle_required_bytes)
        } else {
            self.destination_required_bytes
        }
    }

    /// `None` when the estimate was never probed, or when any volume the plan
    /// needs could not be measured — never a guess in either direction.
    pub fn sufficient(&self) -> Option<bool> {
        if !self.probed {
            return None;
        }
        let destination_required = self.destination_total_required_bytes();
        let destination_ok = if destination_required == 0 {
            true
        } else {
            self.destination_available_bytes? >= destination_required
        };

        let recycle_ok = if self.recycle_required_bytes == 0 || self.recycle_shares_destination_volume
        {
            true
        } else {
            self.recycle_available_bytes? >= self.recycle_required_bytes
        };

        Some(destination_ok && recycle_ok)
    }
}

/// Estimates the free space an operation needs, including the recycle-bin cost
/// when the bin is on another volume (FR-080).
pub fn estimate_free_space(request: &FreeSpaceRequest, probe: &dyn VolumeProbe) -> FreeSpaceEstimate {
    let source_volume = probe.volume_id(&request.source_path);
    let destination_volume = probe.volume_id(&request.destination_path);
    let same_volume_move = match (&source_volume, &destination_volume) {
        (Some(source), Some(destination)) => source == destination,
        _ => false,
    };

    let destination_required_bytes = if same_volume_move {
        0
    } else {
        request.moved_bytes
    };

    let recycle_volume = request
        .recycle_base_path
        .as_ref()
        .and_then(|path| probe.volume_id(path));
    let recycling_available = request.recycle_base_path.is_some();
    let recycle_on_other_volume = match (&source_volume, &recycle_volume) {
        (Some(source), Some(recycle)) => source != recycle,
        // An unprobeable recycle path is treated as a separate volume: the
        // pessimistic reading is the safe one for a space warning.
        (_, None) => recycling_available,
        (None, Some(_)) => recycling_available,
    };
    let recycle_required_bytes = if recycle_on_other_volume {
        request.recycled_bytes
    } else {
        0
    };
    let recycle_shares_destination_volume = match (&destination_volume, &recycle_volume) {
        (Some(destination), Some(recycle)) => recycle_on_other_volume && destination == recycle,
        _ => false,
    };

    FreeSpaceEstimate {
        destination_required_bytes,
        destination_available_bytes: if destination_required_bytes == 0 {
            None
        } else {
            probe.available_bytes(&request.destination_path)
        },
        recycle_required_bytes,
        recycle_available_bytes: if recycle_required_bytes == 0 {
            None
        } else {
            request
                .recycle_base_path
                .as_ref()
                .and_then(|path| probe.available_bytes(path))
        },
        same_volume_move,
        recycle_on_other_volume,
        recycle_shares_destination_volume,
        recycling_available,
        probed: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    fn header() -> LocationPlanHeader {
        LocationPlanHeader::new(
            LocationOperationType::RootMove,
            LocationExecutionMode::MoveWithScryer,
        )
        .with_source(Some("library-a".to_string()), Some("root-1".to_string()))
        .with_destination(Some("library-a".to_string()), Some("root-2".to_string()))
        .with_selection(["title-2".to_string(), "title-1".to_string()])
    }

    fn move_item(title: &str, source: &str, destination: &str, size: u64) -> PlanItem {
        PlanItem::new(PlanItemKind::Move)
            .with_title(title)
            .with_paths(Some(source), Some(destination))
            .with_size(size)
    }

    /// A probe whose answers are dictated by the test, so volume relationships
    /// are exercised without real mounts.
    struct FakeProbe {
        volumes: HashMap<PathBuf, &'static str>,
        available: HashMap<PathBuf, u64>,
    }

    impl VolumeProbe for FakeProbe {
        fn volume_id(&self, path: &Path) -> Option<VolumeId> {
            self.volumes
                .get(path)
                .map(|value| VolumeId((*value).to_string()))
        }

        fn available_bytes(&self, path: &Path) -> Option<u64> {
            self.available.get(path).copied()
        }
    }

    fn fake_probe(volumes: &[(&str, &'static str)], available: &[(&str, u64)]) -> FakeProbe {
        FakeProbe {
            volumes: volumes
                .iter()
                .map(|(path, volume)| (PathBuf::from(path), *volume))
                .collect(),
            available: available
                .iter()
                .map(|(path, bytes)| (PathBuf::from(path), *bytes))
                .collect(),
        }
    }

    #[test]
    fn the_same_plan_always_fingerprints_the_same_way() {
        let items = vec![
            move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10),
            move_item("title-2", "/src/b.mkv", "/dst/b.mkv", 20),
        ];
        let mut builder = LocationPlanBuilder::new(header());
        builder.extend(items.clone());
        let first = builder.build();

        let mut rebuilt = LocationPlanBuilder::new(header());
        rebuilt.extend(items);
        assert_eq!(first.fingerprint, rebuilt.build().fingerprint);
    }

    /// FR-081 for US7: the merge decision is catalog state no plan item spells
    /// out, so it is hashed in its own right. A destination that gains an
    /// episode between preview and start changes the identity map, changes the
    /// summary, and voids the confirmation.
    #[test]
    fn a_changed_merge_summary_invalidates_the_confirmation() {
        let items = vec![move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10)];
        let summary = |free_form: &str| MergePreviewSummary {
            source_title_id: "title-1".to_string(),
            destination_title_id: "destination".to_string(),
            free_form_tags_added: vec![free_form.to_string()],
            ..MergePreviewSummary::default()
        };

        let mut without = LocationPlanBuilder::new(header());
        without.extend(items.clone());
        let without = without.build();

        let mut with = LocationPlanBuilder::new(header());
        with.extend(items.clone());
        with.merge(summary("rewatch"));
        let with = with.build();

        let mut changed = LocationPlanBuilder::new(header());
        changed.extend(items);
        changed.merge(summary("4k"));
        let changed = changed.build();

        assert_ne!(without.fingerprint, with.fingerprint);
        assert_ne!(with.fingerprint, changed.fingerprint);
        assert_eq!(with.merges.len(), 1);
        assert!(without.merges.is_empty());
    }

    #[test]
    fn a_changed_destination_or_selection_invalidates_the_confirmation() {
        let items = vec![move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10)];
        let mut builder = LocationPlanBuilder::new(header());
        builder.extend(items.clone());
        let plan = builder.build();

        let mut moved_destination = LocationPlanBuilder::new(
            header().with_destination(Some("library-a".to_string()), Some("root-3".to_string())),
        );
        moved_destination.extend(items.clone());
        assert_ne!(plan.fingerprint, moved_destination.build().fingerprint);

        let mut changed_selection =
            LocationPlanBuilder::new(header().with_selection(["title-1".to_string()]));
        changed_selection.extend(items.clone());
        assert_ne!(plan.fingerprint, changed_selection.build().fingerprint);

        let mut changed_item = LocationPlanBuilder::new(header());
        changed_item.push(move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 11));
        assert_ne!(plan.fingerprint, changed_item.build().fingerprint);
    }

    #[test]
    fn selection_order_is_not_part_of_the_plan_identity() {
        let items = vec![move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10)];
        let mut forward = LocationPlanBuilder::new(
            header().with_selection(["title-1".to_string(), "title-2".to_string()]),
        );
        forward.extend(items.clone());
        let mut reversed = LocationPlanBuilder::new(
            header().with_selection(["title-2".to_string(), "title-1".to_string()]),
        );
        reversed.extend(items);
        assert_eq!(forward.build().fingerprint, reversed.build().fingerprint);
    }

    #[test]
    fn sampling_never_changes_the_fingerprint_or_the_counts() {
        let items: Vec<PlanItem> = (0..80)
            .map(|index| {
                move_item(
                    &format!("title-{index}"),
                    &format!("/src/{index}.mkv"),
                    &format!("/dst/{index}.mkv"),
                    100,
                )
            })
            .collect();

        let mut full = LocationPlanBuilder::new(header());
        full.extend(items.clone());
        let full_plan = full.build();

        let mut sampled = LocationPlanBuilder::new(header());
        sampled.extend(items).sample_limit(5);
        let sampled_plan = sampled.build();

        assert_eq!(full_plan.fingerprint, sampled_plan.fingerprint);
        assert_eq!(sampled_plan.counts.items_total, 80);
        assert_eq!(sampled_plan.counts.titles_total, 80);
        assert_eq!(sampled_plan.counts.bytes_total, 8_000);

        let section = sampled_plan
            .section(PlanItemKind::Move)
            .expect("the plan should have a move section");
        assert_eq!(section.items.total, 80);
        assert_eq!(section.items.items.len(), 5);
        assert!(!section.items.is_complete());
        assert_eq!(section.bytes_total, 8_000);

        let complete_section = full_plan
            .section(PlanItemKind::Move)
            .expect("the plan should have a move section");
        assert_eq!(complete_section.items.items.len(), 25);
    }

    #[test]
    fn every_kind_is_counted_even_when_it_is_not_sampled() {
        let mut builder = LocationPlanBuilder::new(header());
        builder.extend([
            move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10),
            PlanItem::new(PlanItemKind::Dedup).with_title("title-1"),
            PlanItem::new(PlanItemKind::Blocked)
                .with_title("title-2")
                .with_detail("an import is running"),
        ]);
        let plan = builder.build();

        assert_eq!(plan.counts.for_kind(PlanItemKind::Move), 1);
        assert_eq!(plan.counts.for_kind(PlanItemKind::Dedup), 1);
        assert_eq!(plan.counts.for_kind(PlanItemKind::Blocked), 1);
        assert_eq!(plan.counts.for_kind(PlanItemKind::Merge), 0);
        assert!(plan.blocks_start());
    }

    #[test]
    fn only_cross_volume_moves_are_stated_as_verified() {
        let mut builder = LocationPlanBuilder::new(header());
        builder
            .verification_depth(VerificationDepth::Quick)
            .extend([
                move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10).with_same_volume(false),
                move_item("title-2", "/src/b.mkv", "/dst/b.mkv", 20).with_same_volume(true),
                PlanItem::new(PlanItemKind::CatalogChange).with_title("title-3"),
            ]);
        let plan = builder.build();

        assert_eq!(plan.verification.depth, VerificationDepth::Quick);
        assert_eq!(plan.verification.files, 1);
        assert_eq!(plan.verification.bytes, 10);
        assert!(plan.verification.applies());
    }

    #[test]
    fn root_wide_operations_demand_the_typed_phrase() {
        let mut builder = LocationPlanBuilder::new(LocationPlanHeader::new(
            LocationOperationType::RootChange,
            LocationExecutionMode::MoveWithScryer,
        ));
        builder.push(move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10));
        let plan = builder.build();

        assert!(plan.confirmation.requires_typed_confirmation());
        assert_eq!(
            plan.confirmation.typed_phrase.as_deref(),
            Some(LOCATION_TYPED_CONFIRMATION_PHRASE)
        );

        let missing = PlanConfirmationRequest {
            fingerprint: plan.fingerprint.clone(),
            typed_confirmation: None,
        };
        assert_eq!(
            plan.confirm(&missing),
            Err(PlanConfirmationError::TypedConfirmationRequired)
        );

        let wrong = PlanConfirmationRequest {
            fingerprint: plan.fingerprint.clone(),
            typed_confirmation: Some("move".to_string()),
        };
        assert_eq!(
            plan.confirm(&wrong),
            Err(PlanConfirmationError::TypedConfirmationMismatch)
        );

        let right = PlanConfirmationRequest {
            fingerprint: plan.fingerprint.clone(),
            typed_confirmation: Some(format!("  {LOCATION_TYPED_CONFIRMATION_PHRASE}  ")),
        };
        assert_eq!(plan.confirm(&right), Ok(()));

        let stale = PlanConfirmationRequest {
            fingerprint: PlanFingerprint("something-else".to_string()),
            typed_confirmation: Some(LOCATION_TYPED_CONFIRMATION_PHRASE.to_string()),
        };
        assert_eq!(plan.confirm(&stale), Err(PlanConfirmationError::Stale));
    }

    #[test]
    fn a_plain_operation_confirms_without_a_phrase_but_not_while_blocked() {
        let mut builder = LocationPlanBuilder::new(header());
        builder.push(move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10));
        let plan = builder.build();
        let request = PlanConfirmationRequest {
            fingerprint: plan.fingerprint.clone(),
            typed_confirmation: None,
        };
        assert_eq!(plan.confirm(&request), Ok(()));

        let mut blocked_builder = LocationPlanBuilder::new(header());
        blocked_builder.extend([
            move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10),
            PlanItem::new(PlanItemKind::Blocked)
                .with_title("title-2")
                .with_detail("a download is active"),
        ]);
        let blocked = blocked_builder.build();
        let blocked_request = PlanConfirmationRequest {
            fingerprint: blocked.fingerprint.clone(),
            typed_confirmation: None,
        };
        assert_eq!(
            blocked.confirm(&blocked_request),
            Err(PlanConfirmationError::Blocked)
        );
    }

    /// FR-080: a measured shortfall refuses the confirmation. Starting anyway
    /// would fill the destination volume and strand a title halfway through it,
    /// which is exactly the state the whole subsystem exists to avoid.
    #[test]
    fn a_measured_shortfall_refuses_the_confirmation_but_an_unknown_one_does_not() {
        let probe = fake_probe(&[("/src", "vol-a"), ("/dst", "vol-b")], &[("/dst", 9)]);
        let short = estimate_free_space(
            &FreeSpaceRequest {
                source_path: PathBuf::from("/src"),
                destination_path: PathBuf::from("/dst"),
                moved_bytes: 10,
                recycled_bytes: 0,
                recycle_base_path: None,
            },
            &probe,
        );
        assert_eq!(short.sufficient(), Some(false));

        let mut builder = LocationPlanBuilder::new(header());
        builder.push(move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10));
        builder.free_space(short);
        let plan = builder.build();
        assert_eq!(
            plan.confirm(&PlanConfirmationRequest {
                fingerprint: plan.fingerprint.clone(),
                typed_confirmation: None,
            }),
            Err(PlanConfirmationError::InsufficientSpace)
        );
        assert_eq!(
            PlanConfirmationError::InsufficientSpace.as_str(),
            "insufficient_space"
        );

        // An unprobed volume answers "unknown", and unknown is startable: a
        // destination Scryer cannot stat is not a destination it may refuse.
        let mut unknown_builder = LocationPlanBuilder::new(header());
        unknown_builder.push(move_item("title-1", "/src/a.mkv", "/dst/a.mkv", 10));
        unknown_builder.free_space(FreeSpaceEstimate::unknown());
        let unknown = unknown_builder.build();
        assert_eq!(unknown.free_space.sufficient(), None);
        assert_eq!(
            unknown.confirm(&PlanConfirmationRequest {
                fingerprint: unknown.fingerprint.clone(),
                typed_confirmation: None,
            }),
            Ok(())
        );
    }

    #[test]
    fn a_same_volume_move_needs_no_destination_space() {
        let probe = fake_probe(
            &[("/src", "vol-a"), ("/dst", "vol-a")],
            &[("/dst", 1_000)],
        );
        let estimate = estimate_free_space(
            &FreeSpaceRequest {
                source_path: PathBuf::from("/src"),
                destination_path: PathBuf::from("/dst"),
                moved_bytes: 5_000,
                recycled_bytes: 0,
                recycle_base_path: None,
            },
            &probe,
        );

        assert!(estimate.same_volume_move);
        assert_eq!(estimate.destination_required_bytes, 0);
        assert_eq!(estimate.sufficient(), Some(true));
    }

    #[test]
    fn a_cross_volume_move_requires_the_moved_bytes_at_the_destination() {
        let probe = fake_probe(
            &[("/src", "vol-a"), ("/dst", "vol-b")],
            &[("/dst", 4_999)],
        );
        let request = FreeSpaceRequest {
            source_path: PathBuf::from("/src"),
            destination_path: PathBuf::from("/dst"),
            moved_bytes: 5_000,
            recycled_bytes: 0,
            recycle_base_path: None,
        };
        let estimate = estimate_free_space(&request, &probe);

        assert!(!estimate.same_volume_move);
        assert_eq!(estimate.destination_required_bytes, 5_000);
        assert_eq!(estimate.sufficient(), Some(false));

        let roomy = fake_probe(
            &[("/src", "vol-a"), ("/dst", "vol-b")],
            &[("/dst", 5_000)],
        );
        assert_eq!(
            estimate_free_space(&request, &roomy).sufficient(),
            Some(true)
        );
    }

    #[test]
    fn a_recycle_bin_on_another_volume_adds_a_second_copy_cost() {
        let probe = fake_probe(
            &[
                ("/src", "vol-a"),
                ("/dst", "vol-b"),
                ("/recycle", "vol-c"),
            ],
            &[("/dst", 10_000), ("/recycle", 100)],
        );
        let estimate = estimate_free_space(
            &FreeSpaceRequest {
                source_path: PathBuf::from("/src"),
                destination_path: PathBuf::from("/dst"),
                moved_bytes: 5_000,
                recycled_bytes: 2_000,
                recycle_base_path: Some(PathBuf::from("/recycle")),
            },
            &probe,
        );

        assert!(estimate.recycle_on_other_volume);
        assert!(!estimate.recycle_shares_destination_volume);
        assert_eq!(estimate.recycle_required_bytes, 2_000);
        assert_eq!(estimate.recycle_available_bytes, Some(100));
        // The destination has room; the recycle volume does not.
        assert_eq!(estimate.sufficient(), Some(false));
    }

    #[test]
    fn a_same_volume_recycle_bin_costs_nothing_extra() {
        let probe = fake_probe(
            &[
                ("/src", "vol-a"),
                ("/dst", "vol-b"),
                ("/src/.recycle", "vol-a"),
            ],
            &[("/dst", 10_000)],
        );
        let estimate = estimate_free_space(
            &FreeSpaceRequest {
                source_path: PathBuf::from("/src"),
                destination_path: PathBuf::from("/dst"),
                moved_bytes: 5_000,
                recycled_bytes: 2_000,
                recycle_base_path: Some(PathBuf::from("/src/.recycle")),
            },
            &probe,
        );

        assert!(!estimate.recycle_on_other_volume);
        assert_eq!(estimate.recycle_required_bytes, 0);
        assert_eq!(estimate.sufficient(), Some(true));
    }

    #[test]
    fn a_recycle_bin_on_the_destination_volume_adds_to_the_destination_requirement() {
        let probe = fake_probe(
            &[
                ("/src", "vol-a"),
                ("/dst", "vol-b"),
                ("/dst/.recycle", "vol-b"),
            ],
            &[("/dst", 6_000), ("/dst/.recycle", 6_000)],
        );
        let estimate = estimate_free_space(
            &FreeSpaceRequest {
                source_path: PathBuf::from("/src"),
                destination_path: PathBuf::from("/dst"),
                moved_bytes: 5_000,
                recycled_bytes: 2_000,
                recycle_base_path: Some(PathBuf::from("/dst/.recycle")),
            },
            &probe,
        );

        assert!(estimate.recycle_on_other_volume);
        assert!(estimate.recycle_shares_destination_volume);
        assert_eq!(estimate.destination_total_required_bytes(), 7_000);
        assert_eq!(estimate.sufficient(), Some(false));
    }

    #[test]
    fn an_unmeasurable_volume_answers_unknown_rather_than_enough() {
        let probe = fake_probe(&[("/src", "vol-a"), ("/dst", "vol-b")], &[]);
        let estimate = estimate_free_space(
            &FreeSpaceRequest {
                source_path: PathBuf::from("/src"),
                destination_path: PathBuf::from("/dst"),
                moved_bytes: 5_000,
                recycled_bytes: 0,
                recycle_base_path: None,
            },
            &probe,
        );
        assert_eq!(estimate.sufficient(), None);
        assert_eq!(FreeSpaceEstimate::unknown().sufficient(), None);
    }

    #[test]
    fn expected_partials_are_resumable_while_foreign_changes_are_stale() {
        for change in [
            PlanInputChange::CatalogInput,
            PlanInputChange::UnprocessedSourceItem,
            PlanInputChange::Selection,
            PlanInputChange::Destination,
        ] {
            assert!(change.is_stale(), "{change:?} should invalidate the plan");
        }
        for change in [
            PlanInputChange::VerifiedDestinationFile,
            PlanInputChange::ExpectedDestinationPartial,
            PlanInputChange::SettledSourceItem,
        ] {
            assert!(!change.is_stale(), "{change:?} should stay resumable");
        }
    }
}
