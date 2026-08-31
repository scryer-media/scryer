//! Destination-wins collision handling and BLAKE3-proven deduplication
//! (FR-072–075, FR-090, D4). Engine lands in T018.
//!
//! Destination content always keeps the pathname; incoming content is
//! deduplicated or renamed, never overwritten (FR-072). Deduplication is decided
//! only by a full-file BLAKE3 match — size and the sampled proof are candidate
//! pre-filters, never the deciding comparison (FR-073, D4). When the recycle bin
//! is disabled, unavailable, or rejects a file, the incoming copy is preserved
//! and renamed with a visible warning; permanent deletion is never a fallback
//! (C3).

use serde::{Deserialize, Serialize};

/// What happens to an incoming file that collides with destination content.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CollisionDisposition {
    /// Full BLAKE3 match: keep the destination copy, recycle the redundant
    /// incoming/source copy, merge catalog associations onto the survivor.
    DedupRecycleSource,
    /// Proven duplicate, but recycling is unavailable or refused: preserve the
    /// incoming copy under a disambiguated name and warn (FR-073).
    DedupPreserveWithWarning,
    /// Different content: keep the destination filename, rename the incoming
    /// file with a source-library suffix plus numeric disambiguation (FR-074).
    RenameIncoming,
    /// The "colliding" path is the moving title's own source folder under a
    /// different case — a rename, not a collision (spec Edge Cases).
    CaseOnlyRename,
}

impl CollisionDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DedupRecycleSource => "dedup_recycle_source",
            Self::DedupPreserveWithWarning => "dedup_preserve_with_warning",
            Self::RenameIncoming => "rename_incoming",
            Self::CaseOnlyRename => "case_only_rename",
        }
    }

    /// Dispositions that must surface a warning to the user on completion (C3).
    pub fn warns(&self) -> bool {
        matches!(self, Self::DedupPreserveWithWarning)
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
}
