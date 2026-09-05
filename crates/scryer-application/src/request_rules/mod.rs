//! Request rules (spec 0003, RFC 137 §4.3 "Future request rules").
//!
//! Everything between "a rule exists in the database" and "a request was
//! approved, denied, or left for a human, with a trace, tags, and a lease
//! claim".
//!
//! [`service`] is authoring: rule sets and immutable matcher revisions can be
//! created, edited, previewed against a real requester and title, armed, and
//! deleted. [`gates`] holds the one instance-wide switch, off by default.
//!
//! [`engine`] caches the compiled engine and the per-rule library scope, and is
//! rebuilt by every mutating authoring call. [`facts`] builds the input document
//! — a pure half that turns a read context into observations, and an async half
//! that performs each read once. [`arbitration`] is the pure across-rule order
//! (deny > manual > approve), and [`evaluation`] is what wires all of it
//! together and writes the trace. [`preflight`] is the requester-facing preview
//! that runs the identical path without persisting anything.
//!
//! # What holds the whole thing up
//!
//! Three properties, each enforced in one place rather than remembered at every
//! call site:
//!
//! 1. **Uncertainty never approves.** A held rule, an errored rule, an
//!    unobservable fact, an unreachable metadata gateway — all of them arbitrate
//!    to manual review, above approve ([`arbitration`], FR-012).
//! 2. **Nothing fails the requester.** Every failure inside this module degrades
//!    to the behaviour Scryer had before rules existed ([`evaluation`]).
//! 3. **Every evaluation is explainable.** Preview, shadow, enforce, and the
//!    error fallback all record what they concluded and why (FR-016).

pub mod arbitration;
pub mod engine;
pub mod evaluation;
pub mod facts;
pub mod gates;
pub mod preflight;
pub mod read_model;
pub mod service;

pub use arbitration::{
    Arbitration, ArbitrationReason, FALLBACK_ERROR, FALLBACK_HELD, FALLBACK_NO_RULE_MATCHED,
    FALLBACK_RULE_MANUAL, LIBRARY_PERMISSION_DECIDER, ScopedError, ScopedVote, arbitrate,
    legacy_outcome,
};
pub use engine::{RequestRuleScope, RequestRulesEngineCache, RequestRulesEngineHandle};
pub use evaluation::{
    PREFLIGHT_REQUEST_ID_PREFIX, RecordedVote, RequestDecisionReason, RequestEvaluation,
    RequestEvaluationPurpose,
};
pub use facts::{
    RequestCatalogContext, RequestDraft, RequestInputContext, RequestQualityContext,
    RequestRequesterHistoryContext, build_request_input, snapshot_age_rating,
    us_certification_label,
};
pub use gates::{RequestRuleGates, RequestRuleGatesUpdate};
pub use preflight::RequestPreflight;
pub use read_model::{MediaRequestPolicyFacts, RequestRuleDecisionView};
pub use service::{
    REQUEST_MAX_LEASE_DAYS, RequestRuleDraft, RequestRulePreviewMatcher, RequestRulePreviewRequest,
    RequestRulePreviewResult, RequestRuleSample, RequestRuleSetDetail, validate_lease_days,
    validate_tag_list,
};
