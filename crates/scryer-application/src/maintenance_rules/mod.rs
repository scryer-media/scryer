//! Maintenance rules (RFC 137, "Policy Automation: Maintenance Rules,
//! Maintainerr Migration, and Future Request Rules").
//!
//! # Authoring only — RFC 137 Tracks A2/A3/A4
//!
//! [`action_catalog`] holds the static maintenance action catalog: the closed
//! action kinds, their descriptors (subjects, effect classes, risk class,
//! timing mode, allowed repeat modes), and the schema-versioned, host-validated
//! [`MaintenanceActionSpec`] a rule revision stores.
//!
//! [`service`] adds authoring and on-demand preview: rule sets and immutable
//! matcher revisions can be created, edited, previewed, and deleted.
//! [`facts`] builds the fact snapshot preview evaluates against.
//!
//! [`evaluation`] adds the scheduled dark evaluator (tracks C1/C2): behind five
//! instance gates that all default off, it evaluates rules an operator has
//! explicitly moved to `shadow` or `observe`, and reconciles durable lifecycle
//! candidates and their grace clocks.
//!
//! **No reachable action handler is registered.** Nothing executes an action:
//! rule sets are created disabled, the evaluator only ever writes the
//! `Observing`, `Canceled`, and `Excluded` candidate states, and preview writes
//! nothing at all.
//! The contract tests in [`action_catalog`] prove that raw Rego output cannot
//! select an action — policy evaluation yields only `match`, `no_match`, or
//! `unknown` (RFC 9.1), and there is no API anywhere that turns a free-form
//! action name into a [`MaintenanceActionSpec`].
//!
//! Later waves add the remaining RFC 6.2 files (`commands.rs`, `queries.rs`,
//! `evaluation.rs`, `candidates.rs`, `action_execution.rs`, `claims.rs`,
//! `conflicts.rs`, `exclusions.rs`, `import.rs`) alongside these.

pub mod action_catalog;
pub mod action_execution;
pub mod evaluation;
pub mod facts;
pub mod safety;
pub mod service;

pub use action_catalog::{
    MAINTENANCE_ACTION_SCHEMA_VERSION, MaintenanceActionDescriptor, MaintenanceActionKind,
    MaintenanceActionParameters, MaintenanceActionSpec, MaintenanceActionSpecError,
    MaintenanceEffectClass, MaintenanceRepeatMode, MaintenanceRiskClass, MaintenanceSubjectKind,
    MaintenanceTimingMode, action_catalog, descriptor_for,
};
// ── Scheduled dark evaluation (RFC 137 tracks C1/C2, WP-F) ──────────────────
pub use evaluation::{
    MAINTENANCE_EVALUATION_TITLE_CHUNK, MaintenanceCandidateFilter, MaintenanceCandidateView,
    MaintenanceEvaluationReport, MaintenanceEvaluationTrigger, MaintenanceExclusionView,
    MaintenanceGates, MaintenanceGatesUpdate, candidate_reason,
};
pub use service::{
    MAINTENANCE_PREVIEW_DEFAULT_TITLES, MAINTENANCE_PREVIEW_MAX_TITLES, MaintenanceMatcherDraft,
    MaintenancePreviewMatcher, MaintenancePreviewRequest, MaintenancePreviewResult,
    MaintenancePreviewSelection, MaintenancePreviewTitleResult, MaintenanceRuleDraft,
    MaintenanceRuleSetDetail,
};
// ── Safety preconditions (RFC 137 §9.10, WP-G) ──────────────────────────────
pub use safety::{MaintenanceActivityCheck, MaintenancePlaybackHold, fold_playback_hold};
// ── Action execution (RFC 137 tracks D2/D3, WP-H) ───────────────────────────
pub use action_execution::{
    MAINTENANCE_HIGH_RISK_FAILURE_BREAKER, MAINTENANCE_MAX_ACTION_ATTEMPTS,
    MAINTENANCE_MAX_ACTIONS_PER_RULE_PER_RUN, MAINTENANCE_MAX_HIGH_RISK_ACTIONS_PER_RUN,
    MaintenanceActionHandlingReport, MaintenanceActionRunView, execution_reason,
};
