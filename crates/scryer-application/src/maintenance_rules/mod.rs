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
//! **No reachable action handler is registered.** Nothing schedules evaluation,
//! records candidates, or executes an action: rule sets persist as disabled,
//! the service rejects any other evaluation mode, and preview writes nothing.
//! The contract tests in [`action_catalog`] prove that raw Rego output cannot
//! select an action — policy evaluation yields only `match`, `no_match`, or
//! `unknown` (RFC 9.1), and there is no API anywhere that turns a free-form
//! action name into a [`MaintenanceActionSpec`].
//!
//! Later waves add the remaining RFC 6.2 files (`commands.rs`, `queries.rs`,
//! `evaluation.rs`, `candidates.rs`, `action_execution.rs`, `claims.rs`,
//! `conflicts.rs`, `exclusions.rs`, `import.rs`) alongside these.

pub mod action_catalog;
pub mod facts;
pub mod service;

pub use action_catalog::{
    MAINTENANCE_ACTION_SCHEMA_VERSION, MaintenanceActionDescriptor, MaintenanceActionKind,
    MaintenanceActionParameters, MaintenanceActionSpec, MaintenanceActionSpecError,
    MaintenanceEffectClass, MaintenanceRepeatMode, MaintenanceRiskClass, MaintenanceSubjectKind,
    MaintenanceTimingMode, action_catalog, descriptor_for,
};
pub use service::{
    MAINTENANCE_PREVIEW_DEFAULT_TITLES, MAINTENANCE_PREVIEW_MAX_TITLES, MaintenanceMatcherDraft,
    MaintenancePreviewMatcher, MaintenancePreviewRequest, MaintenancePreviewResult,
    MaintenancePreviewSelection, MaintenancePreviewTitleResult, MaintenanceRuleDraft,
    MaintenanceRuleSetDetail,
};
