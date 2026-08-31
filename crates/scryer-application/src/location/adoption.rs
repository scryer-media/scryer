//! "Files are already there": verify and adopt content the user moved outside
//! Scryer (US3, FR-050–053). Matcher lands in T050.
//!
//! Adoption never rewrites stored path prefixes. It scans the destination and
//! matches tracked media using stored identity information, size, media
//! characteristics, and stored content signatures — the sampled proof always,
//! and the persisted full BLAKE3 where one exists (FR-050). Insufficient proof
//! produces an unresolved state, never a guess (FR-052).

use serde::{Deserialize, Serialize};

/// How one file at the destination is accounted for during adoption (FR-051).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionAccounting {
    /// Tracked media matched to exactly one destination file with sufficient
    /// proof.
    AccountedFor,
    /// Tracked media with no matching destination file; blocks confirmation.
    Missing,
    /// A destination file that no tracked media claims; surfaced, never ignored.
    Additional,
    /// More than one plausible match, or proof too weak to decide; blocks
    /// confirmation (FR-052).
    Ambiguous,
}

impl AdoptionAccounting {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccountedFor => "accounted_for",
            Self::Missing => "missing",
            Self::Additional => "additional",
            Self::Ambiguous => "ambiguous",
        }
    }

    /// Confirmation is blocked while required tracked media is missing or
    /// ambiguous (FR-052).
    pub fn blocks_confirmation(&self) -> bool {
        matches!(self, Self::Missing | Self::Ambiguous)
    }
}

/// Strength of the evidence that tied a tracked media file to a destination
/// file. Recorded so the guarantee given is auditable afterwards (C4).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionMatchStrength {
    /// Persisted full BLAKE3 matched: the strongest proof available.
    FullHash,
    /// Size plus the sampled head+tail proof matched.
    SampledProof,
    /// Only stored identity and media characteristics lined up; not sufficient
    /// on its own to recycle a source copy (FR-053).
    IdentityOnly,
}

impl AdoptionMatchStrength {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FullHash => "full_hash",
            Self::SampledProof => "sampled_proof",
            Self::IdentityOnly => "identity_only",
        }
    }

    /// Source cleanup is left to the user unless Scryer can prove the source
    /// copy is redundant (FR-053).
    pub fn permits_source_recycle(&self) -> bool {
        matches!(self, Self::FullHash)
    }
}
