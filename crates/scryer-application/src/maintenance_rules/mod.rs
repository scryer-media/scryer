//! Maintenance rules: operator-authored Rego that decides, per title, season
//! or episode, what happens to it.
//!
//! A rule's source declares its own actions through action heads
//! (`unmonitor if { … }`, `delete_files if { … }`, `add_tags := [...] if
//! { … }`). [`declared_actions`] reads what a source can do and turns what one
//! evaluation decided into the step sequence [`action_sequence`] defines and
//! the sequence executor runs, in an order the host fixes.
//!
//! [`service`] holds authoring and on-demand preview. [`evaluation`] is the
//! scheduled evaluator that reconciles durable lifecycle candidates and their
//! grace clocks for rules an operator has moved to Dry run or Enabled.
//! [`action_execution`] runs due candidates behind the instance effect gates
//! and the rule's own arming.
//!
//! [`action_catalog`] and the v2 sequence form of [`action_sequence`] are how
//! rules stored a separate action definition before sources declared their
//! own. They stay readable so historical revisions, candidates and runs
//! render.

pub mod action_catalog;
pub mod action_execution;
pub mod action_sequence;
pub mod declared_actions;
pub mod evaluation;
pub mod facts;
pub(crate) mod list_facts;
pub mod safety;
pub(crate) mod scoped_deletion;
pub(crate) mod sequence_execution;
pub(crate) mod sequence_history;
pub mod service;
pub(crate) mod storage;
pub(crate) mod subjects;
pub(crate) mod watch_recheck;

pub use action_catalog::{
    MAINTENANCE_ACTION_SCHEMA_VERSION, MaintenanceActionDescriptor, MaintenanceActionKind,
    MaintenanceActionParameters, MaintenanceActionSpec, MaintenanceActionSpecError,
    MaintenanceEffectClass, MaintenanceRepeatMode, MaintenanceRiskClass, MaintenanceSubjectKind,
    MaintenanceTimingMode, action_catalog, descriptor_for,
};
pub use action_execution::{
    MAINTENANCE_HIGH_RISK_FAILURE_BREAKER, MAINTENANCE_MAX_ACTION_ATTEMPTS,
    MAINTENANCE_MAX_ACTIONS_PER_RULE_PER_RUN, MAINTENANCE_MAX_HIGH_RISK_ACTIONS_PER_RUN,
    MaintenanceActionHandlingReport, MaintenanceActionRunView, derived_effect_arming,
    execution_reason, maintenance_rule_state,
};
pub use action_sequence::{
    ACTION_SEQUENCE_KIND, MAINTENANCE_ACTION_SEQUENCE_SCHEMA_VERSION,
    MAINTENANCE_MAX_ACTION_SEQUENCE_STEPS, MaintenanceActionCompletionPolicy,
    MaintenanceActionDefinition, MaintenanceActionSequence, MaintenanceActionSequenceError,
    MaintenanceActionStep, MaintenanceActionStepDescriptor, MaintenanceActionStepKind,
    MaintenanceActionStepParameters, MaintenanceActionStepRequirement, MaintenanceSearchCondition,
    action_definition_from_persisted_json, action_sequence_catalog, action_sequence_descriptor_for,
};
pub use declared_actions::{
    MAINTENANCE_DECLARED_ACTIONS_SCHEMA_VERSION, MaintenanceDeclaredActions,
    build_declared_action_sequence, decided_actions_from_json, decided_actions_to_json,
    decided_heads_in_execution_order, head_supported_for_scope,
};
pub use evaluation::{
    MAINTENANCE_EVALUATION_TITLE_CHUNK, MaintenanceCandidateFilter, MaintenanceCandidateView,
    MaintenanceEvaluationReport, MaintenanceEvaluationTrigger, MaintenanceExclusionView,
    MaintenanceGates, MaintenanceGatesUpdate, candidate_reason,
};
pub use safety::{MaintenanceActivityCheck, MaintenancePlaybackHold, fold_playback_hold};
pub use service::{
    EXPIRED_REQUEST_LEASES_REGO, MAINTENANCE_MAX_GRACE_DAYS, MAINTENANCE_PREVIEW_DEFAULT_TITLES,
    MAINTENANCE_PREVIEW_MAX_TITLES, MaintenanceMatcherDraft, MaintenancePreviewMatcher,
    MaintenancePreviewRequest, MaintenancePreviewResult, MaintenancePreviewSelection,
    MaintenancePreviewTitleResult, MaintenanceRuleDraft, MaintenanceRuleSetDetail,
    MaintenanceRuleSourceValidation,
};
