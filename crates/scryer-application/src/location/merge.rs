//! Merging a source title into an existing destination title (US7,
//! FR-063–FR-067, D8).
//!
//! The destination wins everything except the source's **media file records**
//! and its **history**; every other row recorded against the source title
//! retires with it through the ordinary title-delete path. The full
//! source→destination identity map (title, episodes, specials, series-movie
//! links) is built before anything is written, and a slot that carries a file or
//! a history row and cannot be mapped blocks the plan rather than being guessed
//! at (FR-066, C3).
//!
//! # Module map
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`map`] | The source→destination identity map and the FR-066 block decision. |
//! | [`roles`] | Post-merge media-role resolution per logical slot (FR-068–FR-070). |
//! | [`summary`] | FR-071: the serializable preview summary. |
//! | [`engine`] | `plan_merge` / `execute_merge` and the repository seam. |

pub mod engine;
pub mod map;
pub mod roles;
pub mod summary;

use serde::{Deserialize, Serialize};

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
/// `Ord` is derived so a `file_episode_map` row can sort deterministically in
/// [`roles::MergedRolePlan`]: a preview and its execution must agree, and a
/// resumed operation must repeat itself.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
