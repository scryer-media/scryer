//! Destination-wins collision handling and BLAKE3-proven deduplication
//! (FR-072–075, FR-090, D4).
//!
//! Destination content always keeps the pathname; incoming content is
//! deduplicated or renamed, never overwritten (FR-072). Deduplication is decided
//! only by a full-file BLAKE3 match — size and the sampled proof are candidate
//! pre-filters, never the deciding comparison (FR-073, D4). When the recycle bin
//! is disabled, unavailable, or rejects a file, the incoming copy is preserved
//! and renamed with a visible warning; permanent deletion is never a fallback
//! (C3).
//!
//! This module is a *planning* engine: it takes facts about the incoming files
//! and the destination directory and returns typed decisions. It performs no
//! filesystem IO, so the preview and the executor can run the exact same
//! decision function over the same facts and be guaranteed to agree (FR-084's
//! "executes exactly what was previewed" property).
//!
//! Role rules (primary/additional) are decided elsewhere and are deliberately
//! independent of the filename decisions made here (FR-074).

use std::borrow::Cow;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// What happens to an incoming file that collides with destination content.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CollisionDisposition {
    /// Nothing at the destination claims the pathname: the incoming item lands
    /// under its proposed name unchanged.
    PlaceAsIs,
    /// The quick check (size plus sampled head/tail proof) could not tell the
    /// incoming file apart from the destination's copy, so the transfer's full
    /// comparison decides: proven identical, the source copy is dropped and the
    /// destination's survives; different, the incoming file is preserved under
    /// the FR-074 name (FR-073). A preview never claims more than "may merge".
    MayMerge,
    /// Different content: keep the destination filename, rename the incoming
    /// file with a source-library suffix plus numeric disambiguation (FR-074).
    RenameIncoming,
    /// A companion asset renamed purely to keep following its media file after
    /// that media file was renamed — no collision of its own (FR-075).
    FollowRenamedMedia,
    /// The "colliding" path is the moving title's own source folder under a
    /// different case — a rename, not a collision (spec Edge Cases).
    CaseOnlyRename,
}

impl CollisionDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PlaceAsIs => "place_as_is",
            Self::MayMerge => "may_merge",
            Self::RenameIncoming => "rename_incoming",
            Self::FollowRenamedMedia => "follow_renamed_media",
            Self::CaseOnlyRename => "case_only_rename",
        }
    }

    /// Parse a persisted disposition value. Unknown values are rejected rather
    /// than defaulted, so a checkpoint written by a newer build cannot be
    /// silently reinterpreted.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "place_as_is" => Some(Self::PlaceAsIs),
            "may_merge" => Some(Self::MayMerge),
            // Plans written before the transfer became the sole judge of
            // identity recorded a preview-time verdict; at transfer time both
            // were exactly a may-merge candidate.
            "dedup_recycle_source" | "dedup_preserve_with_warning" => Some(Self::MayMerge),
            "rename_incoming" => Some(Self::RenameIncoming),
            "follow_renamed_media" => Some(Self::FollowRenamedMedia),
            "case_only_rename" => Some(Self::CaseOnlyRename),
            _ => None,
        }
    }

    /// Whether the incoming item lands under a name other than the one it
    /// proposed. Previews count these as collision renames (FR-080).
    pub fn is_rename(&self) -> bool {
        matches!(self, Self::RenameIncoming | Self::FollowRenamedMedia)
    }

    /// Whether the transfer's full comparison decides this item's fate.
    pub fn may_merge(&self) -> bool {
        matches!(self, Self::MayMerge)
    }
}

/// Whether a colliding item is tracked media or a companion asset. The final
/// summary lists renamed and deduplicated assets separately from media files
/// (FR-075).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CollisionItemKind {
    /// Tracked media file.
    Media,
    /// Recognized companion asset: NFO, subtitles, artwork, trickplay,
    /// thumbnails, related directories.
    CompanionAsset,
    /// A canonical sidecar (`movie.nfo`, `tvshow.nfo`) — the destination's copy
    /// stays authoritative and the incoming one is preserved under a new name.
    CanonicalSidecar,
}

impl CollisionItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::CompanionAsset => "companion_asset",
            Self::CanonicalSidecar => "canonical_sidecar",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "media" => Some(Self::Media),
            "companion_asset" => Some(Self::CompanionAsset),
            "canonical_sidecar" => Some(Self::CanonicalSidecar),
            _ => None,
        }
    }

    /// Media files and companion assets are summarized separately (FR-075).
    pub fn is_media(&self) -> bool {
        matches!(self, Self::Media)
    }
}

/// Filenames that are canonical, folder-level sidecars: the destination's copy
/// stays authoritative and an incoming one is preserved under a new name rather
/// than replacing it (FR-075, spec Edge Cases).
pub const CANONICAL_SIDECAR_NAMES: &[&str] = &["movie.nfo", "tvshow.nfo", "season.nfo"];

/// Classify a filename as a canonical sidecar. Always matched case-insensitively
/// regardless of the destination's case rule: `Movie.nfo` is the same canonical
/// artifact as `movie.nfo` to every media server that reads it.
pub fn is_canonical_sidecar_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    CANONICAL_SIDECAR_NAMES
        .iter()
        .any(|known| *known == lowered)
}

/// Case-sensitivity rule of the destination filesystem, so previews match what
/// the platform will actually do (FR-090, C7).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PathCaseRule {
    /// Distinct names differing only by case can coexist (typical Linux).
    CaseSensitive,
    /// Names differing only by case collide (typical macOS/Windows).
    CaseInsensitive,
}

impl PathCaseRule {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CaseSensitive => "case_sensitive",
            Self::CaseInsensitive => "case_insensitive",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "case_sensitive" => Some(Self::CaseSensitive),
            "case_insensitive" => Some(Self::CaseInsensitive),
            _ => None,
        }
    }

    /// The comparison key for a name or path under this rule.
    pub fn fold<'a>(&self, value: &'a str) -> Cow<'a, str> {
        match self {
            Self::CaseSensitive => Cow::Borrowed(value),
            Self::CaseInsensitive => Cow::Owned(value.to_lowercase()),
        }
    }

    /// Whether two names refer to the same filesystem entry under this rule.
    pub fn names_equal(&self, left: &str, right: &str) -> bool {
        self.fold(left) == self.fold(right)
    }

    /// The rule the host platform uses by default. Callers that know the actual
    /// destination filesystem (a case-sensitive volume on macOS, a
    /// case-insensitive share on Linux) MUST pass their own rule instead: this
    /// is only the default when nothing better is known.
    pub fn platform_default() -> Self {
        if cfg!(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "windows"
        )) {
            Self::CaseInsensitive
        } else {
            Self::CaseSensitive
        }
    }
}

/// State of the full-file BLAKE3 for one file. Only [`FullHash::Known`] can
/// prove a duplicate; absent or stale hashes mean "not identical", never a
/// guess (FR-073, D4, FR-047 invalidation).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FullHash {
    /// The stored full-file BLAKE3 (hex) is current for the file's bytes.
    Known(String),
    /// A full hash was stored but a changed quick hash invalidated it; the file
    /// is queued for backfill (FR-047).
    Stale,
    /// No full hash has been computed for this file yet.
    #[default]
    Absent,
}

impl FullHash {
    pub fn known(hex: impl Into<String>) -> Self {
        Self::Known(hex.into())
    }

    pub fn as_known(&self) -> Option<&str> {
        match self {
            Self::Known(hex) => Some(hex.as_str()),
            _ => None,
        }
    }

    /// Read the 0205 columns off a media file row.
    ///
    /// Invalidation clears the whole group (FR-046), so a stored hash is
    /// current by construction — *provided* the row can say when it was
    /// computed. A hash with no `hash_computed_at` is a vintage nothing can
    /// attest, so it reads back [`FullHash::Stale`]: the backfill job will
    /// recompute it, and until then it proves nothing rather than proving
    /// something unfounded (D4: never a guess).
    pub fn from_persisted(hashes: Option<&crate::location::model::PersistedContentHashes>) -> Self {
        match hashes {
            None => Self::Absent,
            Some(hashes) if hashes.full_blake3.trim().is_empty() => Self::Absent,
            Some(hashes) if hashes.hash_computed_at.is_none() => Self::Stale,
            Some(hashes) => Self::known(hashes.full_blake3.clone()),
        }
    }
}

/// The content facts the preview's quick check reads: the size and the sampled
/// head/tail proof. They can rule a look-alike *out*; they never prove identity.
/// Full-file BLAKE3 is the transfer's job, while both copies still exist (C4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContentFacts {
    pub size_bytes: u64,
    /// The sampled content proof digest (see [`crate::fs_integrity`]). `None`
    /// when the preview did not read it — the file was unreadable, or the
    /// sizes already differed and there was nothing to sample for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampled_proof: Option<String>,
}

impl ContentFacts {
    pub fn new(size_bytes: u64) -> Self {
        Self {
            size_bytes,
            sampled_proof: None,
        }
    }

    pub fn with_sampled_proof(mut self, proof: impl Into<String>) -> Self {
        self.sampled_proof = Some(proof.into());
        self
    }
}

/// What the quick check concluded about a name collision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupVerdict {
    /// Same size and the same sampled proof on both sides: the files may be
    /// identical, and only the transfer's full comparison can say (FR-073).
    MayMerge,
    /// Size or the sampled proof differ: different content, no further read
    /// needed and nothing to warn about.
    DifferentContent,
    /// Same size, but a sampled proof is missing on at least one side, so the
    /// quick check could not run. The transfer compares the pair anyway; the
    /// preview says so rather than guessing either way.
    Unproven {
        incoming_missing: bool,
        destination_missing: bool,
    },
}

impl DedupVerdict {
    pub fn is_different(&self) -> bool {
        matches!(self, Self::DifferentContent)
    }
}

/// The preview's quick check: size first, then the sampled proof. Either can
/// rule a candidate out; agreement only makes it a may-merge candidate, because
/// the full-file comparison belongs to the transfer (FR-073, C4).
pub fn dedup_verdict(incoming: &ContentFacts, destination: &ContentFacts) -> DedupVerdict {
    if incoming.size_bytes != destination.size_bytes {
        return DedupVerdict::DifferentContent;
    }
    match (
        incoming.sampled_proof.as_deref(),
        destination.sampled_proof.as_deref(),
    ) {
        (Some(left), Some(right)) if left == right => DedupVerdict::MayMerge,
        (Some(_), Some(_)) => DedupVerdict::DifferentContent,
        (left, right) => DedupVerdict::Unproven {
            incoming_missing: left.is_none(),
            destination_missing: right.is_none(),
        },
    }
}

/// Naming context for collision renames (FR-074).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionNaming {
    /// Readable source-library label used as the rename suffix.
    pub source_library_label: String,
}

impl CollisionNaming {
    pub fn from_source_library(label: impl AsRef<str>) -> Self {
        Self {
            source_library_label: sanitize_suffix_label(label.as_ref()),
        }
    }
}

/// Characters that are unsafe in a filename on at least one supported platform.
const UNSAFE_NAME_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|', '(', ')'];

/// Reduce an arbitrary library name to something safe and readable inside a
/// filename suffix. Path separators, reserved characters, and the parentheses
/// that delimit the suffix itself are removed; runs of whitespace collapse.
pub fn sanitize_suffix_label(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|ch| {
            if ch.is_control() || UNSAFE_NAME_CHARS.contains(&ch) {
                ' '
            } else {
                ch
            }
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches('.').trim().to_string();
    if trimmed.is_empty() {
        "source".to_string()
    } else {
        trimmed
    }
}

/// Split a filename into (stem, extension-with-dot). A leading dot belongs to
/// the stem (`.plexmatch` has no extension).
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    }
}

/// The base collision-renamed name (before numeric disambiguation):
/// `"<stem> (from <Label>)<.ext>"` (FR-074).
pub fn collision_rename_base(name: &str, label: &str) -> String {
    let (stem, ext) = split_name(name);
    format!("{stem} (from {label}){ext}")
}

/// Apply numeric disambiguation to a base collision name:
/// `"<stem> (from <Label>) (2)<.ext>"`, `(3)`, and so on (FR-074).
fn numbered_variant(name: &str, index: u32) -> String {
    let (stem, ext) = split_name(name);
    format!("{stem} ({index}){ext}")
}

/// One incoming file (or directory) being placed at the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingItem {
    /// Caller-chosen stable id, echoed back on the decision so the executor can
    /// correlate a decision with its media-file/asset record.
    pub id: String,
    /// The name the item would land under if nothing were in the way.
    pub proposed_name: String,
    pub kind: CollisionItemKind,
    pub content: ContentFacts,
    /// For companion assets: the [`IncomingItem::id`] of the media file this
    /// asset belongs to, so its name follows a renamed media file (FR-075).
    pub companion_of: Option<String>,
    /// Stored path of the source file. Only used to recognize a "collision"
    /// that is really this item's own path under a different case.
    pub source_path: Option<String>,
}

impl IncomingItem {
    pub fn media(id: impl Into<String>, proposed_name: impl Into<String>, size_bytes: u64) -> Self {
        Self {
            id: id.into(),
            proposed_name: proposed_name.into(),
            kind: CollisionItemKind::Media,
            content: ContentFacts::new(size_bytes),
            companion_of: None,
            source_path: None,
        }
    }

    /// A companion asset. Canonical sidecars are detected from the name so
    /// callers cannot accidentally classify `movie.nfo` as an ordinary asset.
    pub fn companion(
        id: impl Into<String>,
        proposed_name: impl Into<String>,
        size_bytes: u64,
    ) -> Self {
        let proposed_name = proposed_name.into();
        let kind = if is_canonical_sidecar_name(&proposed_name) {
            CollisionItemKind::CanonicalSidecar
        } else {
            CollisionItemKind::CompanionAsset
        };
        Self {
            id: id.into(),
            proposed_name,
            kind,
            content: ContentFacts::new(size_bytes),
            companion_of: None,
            source_path: None,
        }
    }

    pub fn with_content(mut self, content: ContentFacts) -> Self {
        self.content = content;
        self
    }

    pub fn with_companion_of(mut self, media_id: impl Into<String>) -> Self {
        self.companion_of = Some(media_id.into());
        self
    }

    pub fn with_source_path(mut self, path: impl Into<String>) -> Self {
        self.source_path = Some(path.into());
        self
    }
}

/// One item that already exists at the destination. The destination always wins
/// the pathname (FR-072).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationItem {
    pub name: String,
    pub kind: CollisionItemKind,
    pub content: ContentFacts,
    /// Stored path of the destination entry, when known. Used to recognize the
    /// moving title's own source path under a different case.
    pub path: Option<String>,
}

impl DestinationItem {
    pub fn media(name: impl Into<String>, size_bytes: u64) -> Self {
        Self {
            name: name.into(),
            kind: CollisionItemKind::Media,
            content: ContentFacts::new(size_bytes),
            path: None,
        }
    }

    pub fn companion(name: impl Into<String>, size_bytes: u64) -> Self {
        let name = name.into();
        let kind = if is_canonical_sidecar_name(&name) {
            CollisionItemKind::CanonicalSidecar
        } else {
            CollisionItemKind::CompanionAsset
        };
        Self {
            name,
            kind,
            content: ContentFacts::new(size_bytes),
            path: None,
        }
    }

    pub fn with_content(mut self, content: ContentFacts) -> Self {
        self.content = content;
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// A user-visible warning produced by a collision decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollisionWarning {
    /// A canonical sidecar was preserved as a renamed incoming artifact while
    /// the destination's canonical file stays authoritative (FR-075).
    CanonicalSidecarPreserved {
        canonical_name: String,
        preserved_as: String,
    },
    /// The sizes match but the preview could not read a sampled proof on at
    /// least one side, so it cannot say whether the pair may merge; the
    /// transfer compares them in full either way.
    QuickCheckUnavailable { name: String, detail: String },
}

impl CollisionWarning {
    pub fn code(&self) -> &'static str {
        match self {
            Self::CanonicalSidecarPreserved { .. } => "canonical_sidecar_preserved",
            Self::QuickCheckUnavailable { .. } => "quick_check_unavailable",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::CanonicalSidecarPreserved {
                canonical_name,
                preserved_as,
            } => format!(
                "The destination's \"{canonical_name}\" stays authoritative; the incoming one was preserved as \"{preserved_as}\"."
            ),
            Self::QuickCheckUnavailable { name, detail } => format!(
                "\"{name}\" could not be quick-checked against the destination copy ({detail}); the transfer's full comparison decides whether it merges or is preserved."
            ),
        }
    }
}

/// The decision for one incoming item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionDecision {
    /// Echo of [`IncomingItem::id`].
    pub item_id: String,
    pub kind: CollisionItemKind,
    pub disposition: CollisionDisposition,
    /// The name the incoming item proposed.
    pub proposed_name: String,
    /// The name the incoming item lands under if it is written. For
    /// [`CollisionDisposition::MayMerge`] this is the collided-with name: the
    /// transfer either merges into it or preserves the file under the FR-074
    /// name beside it.
    pub final_name: String,
    /// The destination name that was collided with, when there was one.
    pub collided_with: Option<String>,
    pub warnings: Vec<CollisionWarning>,
}

impl CollisionDecision {
    pub fn renamed(&self) -> bool {
        self.final_name != self.proposed_name
    }
}

/// Everything the planner needs. All facts are supplied by the caller; the
/// planner performs no IO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionPlanRequest {
    /// Case rule of the *destination* filesystem (FR-090).
    pub case_rule: PathCaseRule,
    pub naming: CollisionNaming,
    /// Items already present at the destination.
    pub destination: Vec<DestinationItem>,
    /// Items being placed, in the order the caller wants them considered.
    pub incoming: Vec<IncomingItem>,
}

impl CollisionPlanRequest {
    pub fn new(case_rule: PathCaseRule, naming: CollisionNaming) -> Self {
        Self {
            case_rule,
            naming,
            destination: Vec::new(),
            incoming: Vec::new(),
        }
    }

    pub fn with_destination(mut self, destination: Vec<DestinationItem>) -> Self {
        self.destination = destination;
        self
    }

    pub fn with_incoming(mut self, incoming: Vec<IncomingItem>) -> Self {
        self.incoming = incoming;
        self
    }
}

/// Counts for the operation summary. Renamed and may-merge assets are
/// reported separately from media files (FR-075, FR-080).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollisionSummary {
    pub media_placed: usize,
    pub media_renamed: usize,
    pub media_may_merge: usize,
    pub assets_placed: usize,
    pub assets_renamed: usize,
    pub assets_may_merge: usize,
    /// "Collisions" that were the title's own path under a different case.
    pub case_only_renames: usize,
}

/// The planner's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionPlan {
    pub decisions: Vec<CollisionDecision>,
}

impl CollisionPlan {
    pub fn decision(&self, item_id: &str) -> Option<&CollisionDecision> {
        self.decisions.iter().find(|d| d.item_id == item_id)
    }

    pub fn media_decisions(&self) -> impl Iterator<Item = &CollisionDecision> {
        self.decisions.iter().filter(|d| d.kind.is_media())
    }

    pub fn asset_decisions(&self) -> impl Iterator<Item = &CollisionDecision> {
        self.decisions.iter().filter(|d| !d.kind.is_media())
    }

    /// Every warning across the plan, in decision order.
    pub fn warnings(&self) -> Vec<CollisionWarning> {
        self.decisions
            .iter()
            .flat_map(|d| d.warnings.iter().cloned())
            .collect()
    }

    pub fn summary(&self) -> CollisionSummary {
        let mut summary = CollisionSummary::default();
        for decision in &self.decisions {
            let media = decision.kind.is_media();
            match decision.disposition {
                CollisionDisposition::PlaceAsIs | CollisionDisposition::CaseOnlyRename => {
                    if media {
                        summary.media_placed += 1;
                    } else {
                        summary.assets_placed += 1;
                    }
                }
                CollisionDisposition::RenameIncoming | CollisionDisposition::FollowRenamedMedia => {
                    if media {
                        summary.media_renamed += 1;
                    } else {
                        summary.assets_renamed += 1;
                    }
                }
                CollisionDisposition::MayMerge => {
                    if media {
                        summary.media_may_merge += 1;
                    } else {
                        summary.assets_may_merge += 1;
                    }
                }
            }
            if decision.disposition == CollisionDisposition::CaseOnlyRename {
                summary.case_only_renames += 1;
            }
        }
        summary
    }
}

/// Whether a "collision" between a source path and a destination path is really
/// the same filesystem entry — the moving title's own folder or file under a
/// different case (spec Edge Cases, FR-090).
pub fn is_self_collision(
    source_path: &str,
    destination_path: &str,
    case_rule: PathCaseRule,
) -> bool {
    let normalize = |value: &str| {
        let trimmed = value.trim_end_matches(['/', '\\']);
        case_rule.fold(trimmed).into_owned()
    };
    normalize(source_path) == normalize(destination_path)
}

/// Index of names already claimed at the destination, plus names reserved by
/// earlier decisions in this same plan.
struct NameIndex<'a> {
    case_rule: PathCaseRule,
    destination: HashMap<String, &'a DestinationItem>,
    reserved: HashMap<String, String>,
}

impl<'a> NameIndex<'a> {
    fn new(case_rule: PathCaseRule, destination: &'a [DestinationItem]) -> Self {
        let mut map: HashMap<String, &'a DestinationItem> = HashMap::new();
        for item in destination {
            // On a case-insensitive destination two entries cannot really fold to
            // the same key; if the caller supplies them anyway, the first wins.
            map.entry(case_rule.fold(&item.name).into_owned())
                .or_insert(item);
        }
        Self {
            case_rule,
            destination: map,
            reserved: HashMap::new(),
        }
    }

    fn destination_item(&self, name: &str) -> Option<&'a DestinationItem> {
        // The `&str` is explicit because `Cow<str>` also implements `AsRef` for
        // path types some workspace dependencies bring in, which leaves the
        // lookup's key type ambiguous in a feature-unified build.
        let key: &str = &self.case_rule.fold(name);
        self.destination.get(key).copied()
    }

    fn is_taken(&self, name: &str) -> bool {
        let folded = self.case_rule.fold(name);
        let key: &str = &folded;
        self.destination.contains_key(key) || self.reserved.contains_key(key)
    }

    fn reserve(&mut self, name: &str) {
        self.reserved
            .insert(self.case_rule.fold(name).into_owned(), name.to_string());
    }

    /// First free name in the `base`, `base (2)`, `base (3)` … sequence.
    fn allocate(&self, base: &str) -> String {
        if !self.is_taken(base) {
            return base.to_string();
        }
        for index in 2u32.. {
            let candidate = numbered_variant(base, index);
            if !self.is_taken(&candidate) {
                return candidate;
            }
        }
        unreachable!("the taken-name set is finite")
    }
}

/// Rewrite a companion asset's name so it keeps following its media file after
/// that media file was renamed (FR-075). Returns `None` when the companion's
/// name does not carry the media stem, in which case the caller falls back to
/// the ordinary suffix scheme.
pub fn follow_media_rename(
    companion_name: &str,
    original_media_name: &str,
    renamed_media_name: &str,
    case_rule: PathCaseRule,
) -> Option<String> {
    let (original_stem, _) = split_name(original_media_name);
    let (renamed_stem, _) = split_name(renamed_media_name);
    if original_stem.is_empty() || companion_name.len() < original_stem.len() {
        return None;
    }
    let (head, tail) = companion_name.split_at(original_stem.len());
    if !case_rule.names_equal(head, original_stem) {
        return None;
    }
    Some(format!("{renamed_stem}{tail}"))
}

/// Plan every incoming item against the destination (FR-072–075, FR-090).
///
/// Items without a `companion_of` link are planned first, in input order, so a
/// companion asset can follow the media file it belongs to. The result contains
/// exactly one decision per incoming item, in input order.
pub fn plan_collisions(request: &CollisionPlanRequest) -> CollisionPlan {
    let case_rule = request.case_rule;
    let mut index = NameIndex::new(case_rule, &request.destination);
    let mut decisions: HashMap<String, CollisionDecision> = HashMap::new();

    let leaders = request.incoming.iter().filter(|i| i.companion_of.is_none());
    let followers = request.incoming.iter().filter(|i| i.companion_of.is_some());

    for item in leaders {
        let decision = decide_item(item, item.proposed_name.clone(), request, &mut index, false);
        decisions.insert(item.id.clone(), decision);
    }

    for item in followers {
        // The follower's target name is the one that keeps it attached to its
        // media file, when that media file was renamed.
        let mut followed = false;
        let mut target = item.proposed_name.clone();
        if let Some(parent_id) = item.companion_of.as_deref()
            && let Some(parent) = decisions.get(parent_id)
            && parent.final_name != parent.proposed_name
            && let Some(next) = follow_media_rename(
                &item.proposed_name,
                &parent.proposed_name,
                &parent.final_name,
                case_rule,
            )
        {
            target = next;
            followed = true;
        }
        let decision = decide_item(item, target, request, &mut index, followed);
        decisions.insert(item.id.clone(), decision);
    }

    let ordered = request
        .incoming
        .iter()
        .filter_map(|item| decisions.remove(&item.id))
        .collect();
    CollisionPlan { decisions: ordered }
}

fn decide_item(
    item: &IncomingItem,
    target_name: String,
    request: &CollisionPlanRequest,
    index: &mut NameIndex<'_>,
    followed_media_rename: bool,
) -> CollisionDecision {
    let case_rule = request.case_rule;
    let existing = index.destination_item(&target_name);

    // A "collision" with the moving title's own path under a different case is
    // a rename, not a collision (spec Edge Cases, FR-090).
    if let (Some(existing), Some(source_path)) = (existing, item.source_path.as_deref())
        && let Some(destination_path) = existing.path.as_deref()
        && is_self_collision(source_path, destination_path, case_rule)
    {
        index.reserve(&target_name);
        return CollisionDecision {
            item_id: item.id.clone(),
            kind: item.kind,
            disposition: CollisionDisposition::CaseOnlyRename,
            proposed_name: item.proposed_name.clone(),
            final_name: target_name,
            collided_with: None,
            warnings: Vec::new(),
        };
    }

    let Some(existing) = existing else {
        if index.is_taken(&target_name) {
            // Another incoming item in this same batch already claimed the name.
            // Destination-wins does not apply between two incoming files, but
            // they still cannot share a pathname.
            let base = collision_rename_base(&target_name, &request.naming.source_library_label);
            let final_name = index.allocate(&base);
            index.reserve(&final_name);
            return CollisionDecision {
                item_id: item.id.clone(),
                kind: item.kind,
                disposition: CollisionDisposition::RenameIncoming,
                proposed_name: item.proposed_name.clone(),
                final_name,
                collided_with: Some(target_name),
                warnings: Vec::new(),
            };
        }
        index.reserve(&target_name);
        let disposition = if followed_media_rename {
            CollisionDisposition::FollowRenamedMedia
        } else {
            CollisionDisposition::PlaceAsIs
        };
        return CollisionDecision {
            item_id: item.id.clone(),
            kind: item.kind,
            disposition,
            proposed_name: item.proposed_name.clone(),
            final_name: target_name,
            collided_with: None,
            warnings: Vec::new(),
        };
    };

    let verdict = dedup_verdict(&item.content, &existing.content);
    let mut warnings = Vec::new();

    if !verdict.is_different() {
        // The quick check could not tell the pair apart. The transfer's full
        // comparison decides (FR-073): identical content merges into the
        // destination's copy and the source is dropped; different content is
        // preserved under the FR-074 name. The preview promises only that.
        if let DedupVerdict::Unproven {
            incoming_missing,
            destination_missing,
        } = verdict
        {
            let detail = match (incoming_missing, destination_missing) {
                (true, true) => "neither file could be sampled",
                (true, false) => "the incoming file could not be sampled",
                _ => "the destination file could not be sampled",
            };
            warnings.push(CollisionWarning::QuickCheckUnavailable {
                name: target_name.clone(),
                detail: detail.to_string(),
            });
        }
        // The name stays claimed by the destination copy either way; the
        // suffixed name a different file would take is allocated at transfer
        // time, against the folder as it is then.
        return CollisionDecision {
            item_id: item.id.clone(),
            kind: item.kind,
            disposition: CollisionDisposition::MayMerge,
            proposed_name: item.proposed_name.clone(),
            final_name: existing.name.clone(),
            collided_with: Some(existing.name.clone()),
            warnings,
        };
    }

    // FR-074/FR-075: destination keeps its filename; the incoming file is
    // renamed with the source-library suffix plus numeric disambiguation.
    let base = collision_rename_base(&target_name, &request.naming.source_library_label);
    let final_name = index.allocate(&base);
    index.reserve(&final_name);
    if item.kind == CollisionItemKind::CanonicalSidecar
        || existing.kind == CollisionItemKind::CanonicalSidecar
    {
        warnings.push(CollisionWarning::CanonicalSidecarPreserved {
            canonical_name: existing.name.clone(),
            preserved_as: final_name.clone(),
        });
    }

    CollisionDecision {
        item_id: item.id.clone(),
        kind: item.kind,
        disposition: CollisionDisposition::RenameIncoming,
        proposed_name: item.proposed_name.clone(),
        final_name,
        collided_with: Some(existing.name.clone()),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naming() -> CollisionNaming {
        CollisionNaming::from_source_library("Movies 4K")
    }

    fn request(case_rule: PathCaseRule) -> CollisionPlanRequest {
        CollisionPlanRequest::new(case_rule, naming())
    }

    fn facts(size: u64, sampled: &str) -> ContentFacts {
        ContentFacts::new(size).with_sampled_proof(sampled)
    }

    // --- FR-073: the quick check ------------------------------------------

    #[test]
    fn matching_size_and_sample_make_a_may_merge_candidate_not_a_proof() {
        let verdict = dedup_verdict(&facts(100, "sample"), &facts(100, "sample"));
        assert_eq!(verdict, DedupVerdict::MayMerge);
        assert!(!verdict.is_different());
    }

    #[test]
    fn different_sizes_short_circuit_before_sampling() {
        let verdict = dedup_verdict(&facts(100, "sample"), &ContentFacts::new(101));
        assert_eq!(verdict, DedupVerdict::DifferentContent);
    }

    #[test]
    fn different_sampled_proofs_rule_a_candidate_out() {
        let verdict = dedup_verdict(&facts(100, "sample-a"), &facts(100, "sample-b"));
        assert_eq!(verdict, DedupVerdict::DifferentContent);
    }

    #[test]
    fn a_missing_sample_leaves_the_pair_unproven_rather_than_different() {
        let verdict = dedup_verdict(&ContentFacts::new(100), &facts(100, "sample"));
        assert_eq!(
            verdict,
            DedupVerdict::Unproven {
                incoming_missing: true,
                destination_missing: false,
            }
        );
        assert!(!verdict.is_different());
    }

    /// FR-046 clears the whole 0205 column group, so a persisted hash is
    /// current by construction — but only when the row can say *when* it was
    /// computed.
    #[test]
    fn a_persisted_hash_with_a_vintage_reads_back_as_known() {
        let hashes = crate::location::model::PersistedContentHashes {
            full_blake3: "abc".to_string(),
            move_crc: Some(7),
            crc_algorithm: Some(crate::location::model::MoveCrcAlgorithm::Crc64Nvme),
            hash_computed_at: Some(chrono::Utc::now()),
        };

        assert_eq!(
            FullHash::from_persisted(Some(&hashes)),
            FullHash::known("abc")
        );
    }

    /// A hash nothing can date is a hash nothing can vouch for; it reads back
    /// stale so the backfill job recomputes it.
    #[test]
    fn a_persisted_hash_without_a_vintage_reads_back_as_stale() {
        let hashes = crate::location::model::PersistedContentHashes {
            full_blake3: "abc".to_string(),
            move_crc: None,
            crc_algorithm: None,
            hash_computed_at: None,
        };

        assert_eq!(FullHash::from_persisted(Some(&hashes)), FullHash::Stale);
    }

    #[test]
    fn an_unhashed_row_reads_back_as_absent() {
        assert_eq!(FullHash::from_persisted(None), FullHash::Absent);
    }

    #[test]
    fn a_may_merge_candidate_keeps_the_destination_name_for_the_transfer_to_judge() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![
                    DestinationItem::media("Film.mkv", 100).with_content(facts(100, "sample")),
                ])
                .with_incoming(vec![
                    IncomingItem::media("m1", "Film.mkv", 100).with_content(facts(100, "sample")),
                ]),
        );

        let decision = plan.decision("m1").expect("decision");
        assert_eq!(decision.disposition, CollisionDisposition::MayMerge);
        assert!(decision.disposition.may_merge());
        assert!(!decision.disposition.is_rename());
        assert_eq!(decision.final_name, "Film.mkv");
        assert_eq!(decision.collided_with.as_deref(), Some("Film.mkv"));
        assert!(decision.warnings.is_empty());
        assert_eq!(plan.summary().media_may_merge, 1);
        assert_eq!(plan.summary().media_renamed, 0);
    }

    #[test]
    fn a_lookalike_the_preview_could_not_sample_is_still_the_transfers_call() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![
                    DestinationItem::media("Film.mkv", 100).with_content(facts(100, "sample")),
                ])
                .with_incoming(vec![
                    IncomingItem::media("m1", "Film.mkv", 100).with_content(ContentFacts::new(100)),
                ]),
        );

        let decision = plan.decision("m1").expect("decision");
        assert_eq!(decision.disposition, CollisionDisposition::MayMerge);
        assert_eq!(decision.final_name, "Film.mkv");
        let warning = decision.warnings.first().expect("warning");
        assert_eq!(warning.code(), "quick_check_unavailable");
        assert!(
            warning
                .message()
                .contains("the incoming file could not be sampled"),
            "{}",
            warning.message()
        );
    }

    #[test]
    fn a_may_merge_media_file_still_pulls_its_companions_along_unrenamed() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![
                    DestinationItem::media("Film.mkv", 100).with_content(facts(100, "sample")),
                ])
                .with_incoming(vec![
                    IncomingItem::media("m1", "Film.mkv", 100).with_content(facts(100, "sample")),
                    IncomingItem::companion("s1", "Film.en.srt", 10).with_companion_of("m1"),
                ]),
        );
        // The media file's name did not change at preview time, so the
        // companion has nothing to follow; the transfer renames both together
        // if the media file turns out to be different.
        let companion = plan.decision("s1").expect("s1");
        assert_eq!(companion.disposition, CollisionDisposition::PlaceAsIs);
        assert_eq!(companion.final_name, "Film.en.srt");
    }

    // --- FR-072 / FR-074: destination wins, renames disambiguate ----------

    #[test]
    fn a_free_name_is_placed_unchanged() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![DestinationItem::media("Other.mkv", 5)])
                .with_incoming(vec![IncomingItem::media("m1", "Film.mkv", 100)]),
        );
        let decision = plan.decision("m1").expect("decision");
        assert_eq!(decision.disposition, CollisionDisposition::PlaceAsIs);
        assert_eq!(decision.final_name, "Film.mkv");
        assert!(!decision.renamed());
    }

    #[test]
    fn a_non_identical_media_collision_renames_the_incoming_file() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![
                    DestinationItem::media("Film.mkv", 100).with_content(facts(100, "sample-dest")),
                ])
                .with_incoming(vec![
                    IncomingItem::media("m1", "Film.mkv", 200)
                        .with_content(facts(200, "sample-src")),
                ]),
        );

        let decision = plan.decision("m1").expect("decision");
        assert_eq!(decision.disposition, CollisionDisposition::RenameIncoming);
        assert_eq!(decision.final_name, "Film (from Movies 4K).mkv");
        assert_eq!(decision.collided_with.as_deref(), Some("Film.mkv"));
        assert!(decision.warnings.is_empty(), "clearly different content");
        assert_eq!(plan.summary().media_renamed, 1);
    }

    #[test]
    fn numeric_disambiguation_kicks_in_when_the_suffixed_name_is_also_taken() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![
                    DestinationItem::media("Film.mkv", 1),
                    DestinationItem::media("Film (from Movies 4K).mkv", 2),
                    DestinationItem::media("Film (from Movies 4K) (2).mkv", 3),
                ])
                .with_incoming(vec![IncomingItem::media("m1", "Film.mkv", 100)]),
        );

        assert_eq!(
            plan.decision("m1").expect("decision").final_name,
            "Film (from Movies 4K) (3).mkv"
        );
    }

    #[test]
    fn two_incoming_files_cannot_claim_the_same_name() {
        let plan = plan_collisions(&request(PathCaseRule::CaseSensitive).with_incoming(vec![
            IncomingItem::media("m1", "Film.mkv", 100),
            IncomingItem::media("m2", "Film.mkv", 200),
        ]));

        assert_eq!(plan.decision("m1").expect("m1").final_name, "Film.mkv");
        let second = plan.decision("m2").expect("m2");
        assert_eq!(second.disposition, CollisionDisposition::RenameIncoming);
        assert_eq!(second.final_name, "Film (from Movies 4K).mkv");
    }

    #[test]
    fn decisions_come_back_in_input_order() {
        let plan = plan_collisions(&request(PathCaseRule::CaseSensitive).with_incoming(vec![
            IncomingItem::companion("s1", "Film.en.srt", 1).with_companion_of("m1"),
            IncomingItem::media("m1", "Film.mkv", 100),
        ]));
        let ids: Vec<&str> = plan.decisions.iter().map(|d| d.item_id.as_str()).collect();
        assert_eq!(ids, vec!["s1", "m1"]);
    }

    // --- FR-075: sidecars and companion assets ----------------------------

    #[test]
    fn a_companion_follows_its_renamed_media_file() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![
                    DestinationItem::media("Film.mkv", 100).with_content(facts(100, "dest")),
                ])
                .with_incoming(vec![
                    IncomingItem::media("m1", "Film.mkv", 200).with_content(facts(200, "src")),
                    IncomingItem::companion("s1", "Film.en.srt", 10).with_companion_of("m1"),
                    IncomingItem::companion("a1", "Film-thumb.jpg", 20).with_companion_of("m1"),
                ]),
        );

        assert_eq!(
            plan.decision("m1").expect("m1").final_name,
            "Film (from Movies 4K).mkv"
        );
        let subtitle = plan.decision("s1").expect("s1");
        assert_eq!(
            subtitle.disposition,
            CollisionDisposition::FollowRenamedMedia
        );
        assert_eq!(subtitle.final_name, "Film (from Movies 4K).en.srt");
        assert_eq!(
            plan.decision("a1").expect("a1").final_name,
            "Film (from Movies 4K)-thumb.jpg"
        );
        let summary = plan.summary();
        assert_eq!(summary.media_renamed, 1);
        assert_eq!(
            summary.assets_renamed, 2,
            "assets are summarized separately from media"
        );
    }

    #[test]
    fn a_companion_stays_put_when_its_media_file_is_not_renamed() {
        let plan = plan_collisions(&request(PathCaseRule::CaseSensitive).with_incoming(vec![
            IncomingItem::media("m1", "Film.mkv", 100),
            IncomingItem::companion("s1", "Film.en.srt", 10).with_companion_of("m1"),
        ]));
        assert_eq!(plan.decision("s1").expect("s1").final_name, "Film.en.srt");
        assert_eq!(
            plan.decision("s1").expect("s1").disposition,
            CollisionDisposition::PlaceAsIs
        );
    }

    #[test]
    fn a_lookalike_asset_is_a_may_merge_candidate_counted_apart_from_media() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![
                    DestinationItem::companion("poster.jpg", 10).with_content(facts(10, "sample")),
                ])
                .with_incoming(vec![
                    IncomingItem::companion("a1", "poster.jpg", 10)
                        .with_content(facts(10, "sample")),
                ]),
        );

        let decision = plan.decision("a1").expect("a1");
        assert_eq!(decision.disposition, CollisionDisposition::MayMerge);
        let summary = plan.summary();
        assert_eq!(summary.assets_may_merge, 1);
        assert_eq!(summary.media_may_merge, 0);
    }

    #[test]
    fn a_canonical_sidecar_is_preserved_while_the_destination_stays_authoritative() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![
                    DestinationItem::companion("movie.nfo", 4).with_content(facts(4, "dest")),
                ])
                .with_incoming(vec![
                    IncomingItem::companion("n1", "movie.nfo", 7).with_content(facts(7, "src")),
                ]),
        );

        let decision = plan.decision("n1").expect("n1");
        assert_eq!(decision.kind, CollisionItemKind::CanonicalSidecar);
        assert_eq!(decision.disposition, CollisionDisposition::RenameIncoming);
        assert_eq!(decision.final_name, "movie (from Movies 4K).nfo");
        let warning = decision
            .warnings
            .iter()
            .find(|w| w.code() == "canonical_sidecar_preserved")
            .expect("canonical sidecar warning");
        assert!(warning.message().contains("stays authoritative"));
    }

    #[test]
    fn tvshow_nfo_is_recognized_case_insensitively() {
        assert!(is_canonical_sidecar_name("tvshow.nfo"));
        assert!(is_canonical_sidecar_name("TVShow.NFO"));
        assert!(!is_canonical_sidecar_name("Film.nfo"));
    }

    // --- FR-090: per-platform case sensitivity ----------------------------

    #[test]
    fn a_case_insensitive_destination_collides_on_case_only_differences() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseInsensitive)
                .with_destination(vec![DestinationItem::media("FILM.MKV", 100)])
                .with_incoming(vec![IncomingItem::media("m1", "film.mkv", 200)]),
        );
        let decision = plan.decision("m1").expect("m1");
        assert_eq!(decision.disposition, CollisionDisposition::RenameIncoming);
        assert_eq!(decision.collided_with.as_deref(), Some("FILM.MKV"));
        assert_eq!(decision.final_name, "film (from Movies 4K).mkv");
    }

    #[test]
    fn a_case_sensitive_destination_treats_different_case_as_distinct() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseSensitive)
                .with_destination(vec![DestinationItem::media("FILM.MKV", 100)])
                .with_incoming(vec![IncomingItem::media("m1", "film.mkv", 200)]),
        );
        let decision = plan.decision("m1").expect("m1");
        assert_eq!(decision.disposition, CollisionDisposition::PlaceAsIs);
        assert_eq!(decision.final_name, "film.mkv");
    }

    #[test]
    fn the_titles_own_path_under_a_different_case_is_a_rename_not_a_collision() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseInsensitive)
                .with_destination(vec![
                    DestinationItem::media("FILM.MKV", 100).with_path("/media/Movies/FILM.MKV"),
                ])
                .with_incoming(vec![
                    IncomingItem::media("m1", "Film.mkv", 100)
                        .with_source_path("/media/movies/film.mkv"),
                ]),
        );

        let decision = plan.decision("m1").expect("m1");
        assert_eq!(decision.disposition, CollisionDisposition::CaseOnlyRename);
        assert_eq!(decision.final_name, "Film.mkv");
        assert!(decision.collided_with.is_none());
        assert_eq!(plan.summary().case_only_renames, 1);
    }

    #[test]
    fn a_different_titles_path_under_the_same_name_is_still_a_collision() {
        let plan = plan_collisions(
            &request(PathCaseRule::CaseInsensitive)
                .with_destination(vec![
                    DestinationItem::media("FILM.MKV", 100)
                        .with_path("/media/Movies/Other/FILM.MKV"),
                ])
                .with_incoming(vec![
                    IncomingItem::media("m1", "Film.mkv", 120)
                        .with_source_path("/media/movies/film.mkv"),
                ]),
        );
        assert_eq!(
            plan.decision("m1").expect("m1").disposition,
            CollisionDisposition::RenameIncoming
        );
    }

    #[test]
    fn self_collision_ignores_a_trailing_separator() {
        assert!(is_self_collision(
            "/media/Movies/Film",
            "/media/movies/film/",
            PathCaseRule::CaseInsensitive
        ));
        assert!(!is_self_collision(
            "/media/Movies/Film",
            "/media/movies/film/",
            PathCaseRule::CaseSensitive
        ));
    }

    // --- naming helpers ---------------------------------------------------

    #[test]
    fn suffix_labels_are_sanitized_but_stay_readable() {
        assert_eq!(sanitize_suffix_label("Movies 4K"), "Movies 4K");
        assert_eq!(sanitize_suffix_label("TV / Anime"), "TV Anime");
        assert_eq!(sanitize_suffix_label("  Kids (old)  "), "Kids old");
        assert_eq!(sanitize_suffix_label("   "), "source");
    }

    #[test]
    fn dotfiles_keep_their_whole_name_as_the_stem() {
        assert_eq!(
            collision_rename_base(".plexmatch", "Movies"),
            ".plexmatch (from Movies)"
        );
    }

    #[test]
    fn the_platform_default_case_rule_matches_the_host() {
        let expected = if cfg!(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "windows"
        )) {
            PathCaseRule::CaseInsensitive
        } else {
            PathCaseRule::CaseSensitive
        };
        assert_eq!(PathCaseRule::platform_default(), expected);
    }

    #[test]
    fn dispositions_round_trip_through_their_persisted_strings() {
        for disposition in [
            CollisionDisposition::PlaceAsIs,
            CollisionDisposition::MayMerge,
            CollisionDisposition::RenameIncoming,
            CollisionDisposition::FollowRenamedMedia,
            CollisionDisposition::CaseOnlyRename,
        ] {
            assert_eq!(
                CollisionDisposition::parse(disposition.as_str()),
                Some(disposition)
            );
        }
        assert_eq!(CollisionDisposition::parse("nonsense"), None);
        // Preview-decided verdicts from older plans read back as the candidate
        // the transfer treats them as.
        assert_eq!(
            CollisionDisposition::parse("dedup_recycle_source"),
            Some(CollisionDisposition::MayMerge)
        );
        assert_eq!(
            CollisionDisposition::parse("dedup_preserve_with_warning"),
            Some(CollisionDisposition::MayMerge)
        );
    }
}
