//! Shared preview core: every location workflow builds the same fingerprinted
//! plan (D6, FR-080–082).
//!
//! Large previews return complete counts with a sampled item list, following the
//! established `LibraryRenamePlan` pattern; the fingerprint always covers the
//! full plan, not the sample (FR-081). Plan building, free-space estimation, and
//! the typed-confirmation hook land in T017.

use serde::{Deserialize, Serialize};

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
