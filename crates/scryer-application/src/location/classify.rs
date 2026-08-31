//! Per-title classification of a requested destination.
//!
//! Every title in a selection classifies into exactly one class, and the preview
//! groups them with counts and omits none (FR-015, SC-005). The fileless
//! catalog-only fast path is its own class so it never presents a move-mode
//! choice (FR-076).

use serde::{Deserialize, Serialize};

/// The single class a selected title falls into for a requested destination.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TitleLocationClass {
    /// Destination is in another library and the transfer is supported.
    CrossLibraryTransfer,
    /// Destination is another root inside the title's current library.
    RootMove,
    /// The title already lives at the requested destination; nothing to do.
    NoOp,
    /// Monitored title with no tracked files on disk: catalog reassignment only,
    /// no filesystem work and no move-mode selection (FR-076).
    CatalogOnly,
    /// The destination can never accept this title (facet or media-kind rules in
    /// spec "Boundaries"); the reason names the incompatible source library or
    /// facet (FR-017).
    Incompatible,
    /// The title could go, but a user decision is outstanding: ambiguous
    /// destination-title identity (FR-055), an active download or import
    /// (FR-086), or unmapped episode-scoped merge records (FR-066).
    NeedsResolution,
}

impl TitleLocationClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CrossLibraryTransfer => "cross_library_transfer",
            Self::RootMove => "root_move",
            Self::NoOp => "no_op",
            Self::CatalogOnly => "catalog_only",
            Self::Incompatible => "incompatible",
            Self::NeedsResolution => "needs_resolution",
        }
    }

    /// Parse a persisted class value (checkpoint rows carry the class the title
    /// was previewed as). Unknown values are rejected rather than defaulted.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "cross_library_transfer" => Some(Self::CrossLibraryTransfer),
            "root_move" => Some(Self::RootMove),
            "no_op" => Some(Self::NoOp),
            "catalog_only" => Some(Self::CatalogOnly),
            "incompatible" => Some(Self::Incompatible),
            "needs_resolution" => Some(Self::NeedsResolution),
            _ => None,
        }
    }

    /// Whether a title in this class moves bytes. `NoOp`, `CatalogOnly`,
    /// `Incompatible`, and `NeedsResolution` never do.
    pub fn moves_files(&self) -> bool {
        matches!(self, Self::CrossLibraryTransfer | Self::RootMove)
    }

    /// A bulk operation must not start while any title is in a blocking class;
    /// the user resolves it or removes it from the selection (FR-016).
    pub fn blocks_start(&self) -> bool {
        matches!(self, Self::Incompatible | Self::NeedsResolution)
    }
}

/// One classified title in a preview, carrying the explanation the UI shows for
/// blocking classes (FR-017).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassifiedTitle {
    pub title_id: String,
    pub class: TitleLocationClass,
    /// Why this title landed in its class. Required for `Incompatible` and
    /// `NeedsResolution`; optional otherwise.
    pub reason: Option<String>,
}

/// Complete counts per class for a selection. Every selected title is counted
/// exactly once (SC-005).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ClassificationCounts {
    pub cross_library_transfer: i64,
    pub root_move: i64,
    pub no_op: i64,
    pub catalog_only: i64,
    pub incompatible: i64,
    pub needs_resolution: i64,
}

impl ClassificationCounts {
    pub fn total(&self) -> i64 {
        self.cross_library_transfer
            + self.root_move
            + self.no_op
            + self.catalog_only
            + self.incompatible
            + self.needs_resolution
    }

    /// Any non-zero blocking class prevents the operation from starting
    /// (FR-016).
    pub fn blocks_start(&self) -> bool {
        self.incompatible > 0 || self.needs_resolution > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_file_bearing_classes_move_bytes() {
        assert!(TitleLocationClass::CrossLibraryTransfer.moves_files());
        assert!(TitleLocationClass::RootMove.moves_files());
        for class in [
            TitleLocationClass::NoOp,
            TitleLocationClass::CatalogOnly,
            TitleLocationClass::Incompatible,
            TitleLocationClass::NeedsResolution,
        ] {
            assert!(
                !class.moves_files(),
                "{} must not move files",
                class.as_str()
            );
        }
    }

    #[test]
    fn unresolved_or_incompatible_titles_block_the_start() {
        assert!(TitleLocationClass::Incompatible.blocks_start());
        assert!(TitleLocationClass::NeedsResolution.blocks_start());
        assert!(!TitleLocationClass::RootMove.blocks_start());

        let clean = ClassificationCounts {
            root_move: 3,
            no_op: 1,
            ..ClassificationCounts::default()
        };
        assert_eq!(clean.total(), 4);
        assert!(!clean.blocks_start());

        let blocked = ClassificationCounts {
            needs_resolution: 1,
            ..clean
        };
        assert_eq!(blocked.total(), 5);
        assert!(blocked.blocks_start());
    }
}
