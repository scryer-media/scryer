//! Lifecycle rules: operator-authored Rego that decides, per title, season
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
//! [`action_catalog`] holds the subject, risk and effect vocabulary the step
//! catalog is written in, and the single-action names earlier builds recorded
//! on action runs, which history still labels.

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
pub(crate) mod title_actions;
pub(crate) mod watch_recheck;

pub use action_catalog::{
    LifecycleActionKind, LifecycleEffectClass, LifecycleRiskClass, LifecycleSubjectKind,
};
pub use action_execution::{
    LIFECYCLE_HIGH_RISK_FAILURE_BREAKER, LIFECYCLE_MAX_ACTION_ATTEMPTS,
    LifecycleActionHandlingReport, LifecycleActionRunView, arming_kept_across_edit,
    derived_effect_arming, execution_reason, lifecycle_rule_state,
};
pub use action_sequence::{
    ACTION_SEQUENCE_KIND, LIFECYCLE_ACTION_SEQUENCE_SCHEMA_VERSION,
    LIFECYCLE_MAX_ACTION_SEQUENCE_STEPS, LifecycleActionCompletionPolicy, LifecycleActionSequence,
    LifecycleActionSequenceError, LifecycleActionStep, LifecycleActionStepDescriptor,
    LifecycleActionStepKind, LifecycleActionStepParameters, LifecycleActionStepRequirement,
    LifecycleSearchCondition, action_sequence_catalog, action_sequence_descriptor_for,
};
pub use declared_actions::{
    LIFECYCLE_DECLARED_ACTIONS_SCHEMA_VERSION, LifecycleDeclaredActions,
    build_declared_action_sequence, decided_actions_from_json, decided_actions_to_json,
    decided_heads_in_execution_order, head_supported_for_scope,
};
pub use evaluation::{
    LIFECYCLE_EVALUATION_TITLE_CHUNK, LifecycleCandidateFilter, LifecycleCandidateView,
    LifecycleEvaluationReport, LifecycleEvaluationTrigger, LifecycleExclusionView, LifecycleGates,
    LifecycleGatesUpdate, candidate_reason,
};
pub use safety::{LifecycleActivityCheck, LifecyclePlaybackHold, fold_playback_hold};
pub use service::{
    EXPIRED_REQUEST_LEASES_REGO, LIFECYCLE_MAX_GRACE_DAYS, LIFECYCLE_PREVIEW_DEFAULT_TITLES,
    LIFECYCLE_PREVIEW_MAX_TITLES, LifecycleMatcherDraft, LifecyclePreviewMatcher,
    LifecyclePreviewRequest, LifecyclePreviewResult, LifecyclePreviewSelection,
    LifecyclePreviewTitleResult, LifecycleRuleDraft, LifecycleRuleSetDetail,
    LifecycleRuleSourceValidation,
};
