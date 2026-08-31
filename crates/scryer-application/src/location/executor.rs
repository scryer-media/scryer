//! Operation runner: drives an operation through its states, checkpointing per
//! title, stopping at safe cancel points, and resuming across process restarts
//! (FR-030–033, FR-089, FR-092). Implementation lands in T015.
//!
//! Execution order per operation (FR-031): validate → fingerprinted preview →
//! explicit confirmation → move one title at a time → verify each copy at the
//! configured depth → apply destination permissions → flip catalog ownership →
//! recycle or preserve redundant sources → remove only empty source directories
//! → finalize root or source-title removal.

use serde::{Deserialize, Serialize};

/// Where a resumed operation picks up. Titles at or before `last_settled_sequence`
/// are never reprocessed, so verified work is never repeated (FR-092).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResumeCursor {
    pub operation_id: String,
    /// Highest checkpoint sequence whose title reached a settled state.
    pub last_settled_sequence: i64,
}

/// Why a running operation stopped short of completion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The user cancelled; the runner stops at the next safe title checkpoint.
    UserCanceled,
    /// Inputs the plan had not yet processed changed underneath the operation,
    /// so the plan is stale and a new preview is required (FR-089). Expected
    /// partial destination state from an interrupted copy is *not* stale.
    StalePlan,
    /// A verification, filesystem, or catalog error stopped the run.
    Error,
}

impl StopReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UserCanceled => "user_canceled",
            Self::StalePlan => "stale_plan",
            Self::Error => "error",
        }
    }
}
