//! Request rules: rules that vote on media requests and shape what an
//! approval writes.
//!
//! Everything between "a rule exists in the database" and "a request was
//! approved, denied, or left for a human, with a trace, tags, and a lease
//! claim".
//!
//! [`service`] is authoring: rule sets and immutable source revisions can be
//! created, edited, previewed against a real requester and title, enabled, and
//! deleted. A rule set is either disabled or enabled; there is no
//! instance-wide switch above it.
//!
//! [`engine`] caches the compiled engine and the per-rule library scope, and is
//! rebuilt by every mutating authoring call. [`facts`] builds the input document
//! — a pure half that turns a read context into observations, and an async half
//! that performs each read once. [`arbitration`] is the pure across-rule order
//! (deny > manual > approve), and [`evaluation`] is what wires all of it
//! together and writes the trace. [`preflight`] is the requester-facing preview
//! that runs the identical path without persisting anything. [`report`] runs
//! the report rules against a bad-file report, through the same engine and
//! trace, with its own arbitration and no permission fallback.
//!
//! # What holds the whole thing up
//!
//! Three properties, each enforced in one place rather than remembered at every
//! call site:
//!
//! 1. **Uncertainty never approves.** A held rule, an errored rule, an
//!    unobservable fact, an unreachable metadata gateway — all of them arbitrate
//!    to manual review, above approve ([`arbitration`]). When enabled rules
//!    cover a library and cannot be consulted at all (the facts cannot be
//!    assembled, the engine fails, or its generation could not be rebuilt),
//!    the request is held for a human too, whatever the Auto-Approve
//!    permission says ([`evaluation`]).
//! 2. **Nothing fails the requester.** A failure inside this module never fails
//!    the request: it lands as manual review, and with no enabled rule
//!    covering the library the Auto-Approve permission decides as it did
//!    before rules existed ([`evaluation`]).
//! 3. **Every evaluation is explainable.** Preview, enabled rules, and the
//!    error fallback all record what they concluded and why.

pub mod arbitration;
pub mod engine;
pub mod evaluation;
pub mod facts;
pub mod preflight;
pub mod read_model;
pub mod report;
pub mod service;

pub use arbitration::{
    ALL_FALLBACKS, ArbitratedShaping, Arbitration, ArbitrationReason, FALLBACK_ERROR,
    FALLBACK_HELD, FALLBACK_LIBRARY_PERMISSION, FALLBACK_NO_RULE_MATCHED, FALLBACK_REPORT_CONFLICT,
    FALLBACK_RULE_MANUAL, FALLBACK_SHAPING_CONFLICT, FALLBACK_SHAPING_INVALID,
    LIBRARY_PERMISSION_DECIDER, ReportArbitration, ReportVerdict, ScopedError, ScopedVote,
    ShapedValue, ShapingRefusal, arbitrate, arbitrate_report,
};
pub use engine::{
    RequestRuleKind, RequestRuleScope, RequestRulesEngineCache, RequestRulesEngineHandle,
};
pub use evaluation::{
    PREFLIGHT_REQUEST_ID_PREFIX, RecordedVote, RequestDecisionReason, RequestEvaluation,
    RequestEvaluationPurpose, ResolvedShaping,
};
pub use facts::{
    RequestCatalogContext, RequestDraft, RequestInputContext, RequestQualityContext,
    RequestRequesterHistoryContext, award_docs, build_request_input, certification_docs,
    content_ratings_age_rating, content_ratings_us_certification_label, snapshot_age_rating,
    us_certification_label,
};
pub use preflight::{RequestPreflight, StagedRequestMetadata};
pub use read_model::{
    MediaRequestPolicyFacts, MediaRequestPolicySelection, RequestRuleDecisionView,
};
pub use report::{
    MEDIA_ISSUE_DECISION_REQUEST_ID_PREFIX, MediaIssueReportSubject, ReportRuleEvaluation,
    ReportRuleOutcome,
};
pub use service::{
    REQUEST_MAX_LEASE_DAYS, RequestRuleDraft, RequestRulePreviewRequest, RequestRulePreviewResult,
    RequestRulePreviewSource, RequestRuleReportSample, RequestRuleSample, RequestRuleSetDetail,
    RequestRuleValidation, validate_lease_days, validate_tag_list,
};
