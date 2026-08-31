//! Merging a source title into an existing destination title (US7, FR-063–067,
//! D8). Engine lands in T085, over the table inventory produced by T081.
//!
//! The full source→destination identity map (title, episodes, specials,
//! series-movie links) is built before anything is written; records scoped to an
//! episode that cannot be mapped block the plan rather than being dropped
//! (FR-066, C3). Unions execute as transactional id-rewrites at the title
//! checkpoint.

use serde::{Deserialize, Serialize};

/// How one table's rows are treated when a source title merges into a
/// destination title. The per-table assignment is the T081 inventory
/// deliverable.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MergeDisposition {
    /// Source rows are re-pointed at the destination identity and kept alongside
    /// the destination's own rows.
    Union,
    /// Source rows are rewritten through the identity map (episode ids, links).
    Map,
    /// The destination's value stands; the source's is discarded.
    DestinationWins,
    /// Source rows are intentionally not carried over.
    Drop,
}

impl MergeDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Union => "union",
            Self::Map => "map",
            Self::DestinationWins => "destination_wins",
            Self::Drop => "drop",
        }
    }
}

/// Result of matching a source title against destination candidates by stable
/// metadata identity, including redirects (FR-055).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DestinationIdentityMatch {
    /// Exactly one destination title shares the canonical identity: merge.
    Unique,
    /// No destination title shares the identity: plain transfer.
    None,
    /// More than one candidate: the user must resolve it; never auto-merged.
    Ambiguous,
    /// A destination title has the same name but no shared identity: never
    /// auto-merged (FR-055).
    SameNameNoIdentity,
}

impl DestinationIdentityMatch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unique => "unique",
            Self::None => "none",
            Self::Ambiguous => "ambiguous",
            Self::SameNameNoIdentity => "same_name_no_identity",
        }
    }

    /// Whether this match requires a user decision before the plan can start
    /// (FR-016).
    pub fn needs_resolution(&self) -> bool {
        matches!(self, Self::Ambiguous)
    }
}

/// Role a media file holds for one logical slot after a merge. Every role change
/// is previewed and no primary is silently demoted (FR-068–070).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MergedMediaRole {
    Primary,
    Additional,
}

impl MergedMediaRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Additional => "additional",
        }
    }
}
