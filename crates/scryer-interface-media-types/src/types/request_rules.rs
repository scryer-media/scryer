//! Wire types for request rules, pre-flight, decision traces, and lifecycle
//! claims (spec 0003 §7).
//!
//! Two audiences read this surface and they are not allowed to see the same
//! things. An author or an approver gets the whole trace: which rule voted what,
//! on which revision, and the document it saw. A **requester** gets the verdict,
//! the reasons behind it, and the tags an approval would stamp — never a vote
//! table, never a line of Rego, never the input document (FR-020). The split is
//! not a convention here: [`RequestPreflightPayload`] has no field that could
//! carry a vote, and a [`RequestRuleDecision`] read by its own requester comes
//! back with `votes` emptied.

use async_graphql::{Enum, ID, InputObject, Json, SimpleObject};
use chrono::{DateTime, Utc};

use super::{DiscoveryContentRatingPayload, ExternalIdInput, MonitorTypeValue};

// ── Enums ──────────────────────────────────────────────────────────────────

/// How a request rule set is evaluated.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RequestRuleEvaluationModeValue {
    /// Stored but never evaluated, and the mode every new rule set is created
    /// in.
    Disabled,
    /// Evaluated and recorded; the effective decision stays whatever the
    /// library permission would have produced on its own.
    Shadow,
    /// Evaluated, recorded, and acted on.
    Enforce,
}

/// What a request evaluation concluded.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RequestDecisionOutcomeValue {
    /// Approve without waiting for a person.
    AutoApprove,
    /// Leave the request for a human. Every uncertainty lands here: a held
    /// rule, a rule that errored, an unobservable fact, or no rule matching at
    /// all.
    ManualReview,
    /// Refuse the request.
    Deny,
}

/// One rule's vote on one request.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RequestVoteValue {
    /// The rule would approve.
    Approve,
    /// The rule refuses. A deny outranks every other vote.
    Deny,
    /// The rule wants a person to look. Outranks an approve.
    Manual,
    /// The rule ran and had no opinion.
    Abstain,
}

/// What produced a lifecycle claim.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleClaimProducerValue {
    /// A finite lease approved on a media request.
    RequestLease,
    /// A request approved as "forever": a permanent keep.
    RequestPermanent,
    /// An administrator pinned the title by hand.
    OperatorKeep,
}

/// What a lifecycle claim holds.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleClaimKindValue {
    /// Holds until `expiresAt`, whose clock starts at the title's first import.
    RetainUntil,
    /// Holds indefinitely.
    Keep,
}

/// Lifecycle state of one claim. Claims are released, never deleted, so the
/// terminal states are history and only `DORMANT` and `ACTIVE` hold anything.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleClaimStateValue {
    /// Created but not yet started: the title has never imported.
    Dormant,
    /// Running. A retention claim has an `expiresAt`.
    Active,
    /// The retention window elapsed.
    Expired,
    /// Withdrawn before it ran out.
    Released,
    /// Superseded by a permanent keep.
    Converted,
}

// ── Read models ────────────────────────────────────────────────────────────

/// A request rule set: identity, scope, and evaluation state. The matcher lives
/// on the revision, never here.
#[derive(SimpleObject, Clone)]
pub struct RequestRuleSet {
    /// Rule-set ID.
    pub id: ID,
    /// Rule-set name.
    pub name: String,
    /// Rule-set description; empty when the author supplied none.
    pub description: String,
    /// Whether the rule set is enabled. Derived from `evaluationMode` rather
    /// than set independently: false for `DISABLED`, true otherwise.
    pub enabled: bool,
    /// Evaluation mode currently stored for the rule set.
    pub evaluation_mode: RequestRuleEvaluationModeValue,
    /// Libraries the rule is confined to. Empty means every library.
    pub library_ids: Vec<String>,
    /// Revision number of the matcher currently in force.
    pub current_revision_number: i32,
    /// How many recorded decisions this rule took part in. Traces outlive the
    /// rule that produced them, so this can be non-zero for a rule that is now
    /// disabled.
    pub decision_count: i32,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

/// One immutable revision of a request rule set's matcher.
#[derive(SimpleObject, Clone)]
pub struct RequestRuleRevision {
    /// Revision ID.
    pub id: ID,
    /// ID of the rule set this revision belongs to.
    pub rule_set_id: ID,
    /// Revision number, starting at one and incremented on every matcher edit.
    pub revision_number: i32,
    /// Rego source as the editor should show it, with the package declaration
    /// and the rego.v1 import stripped.
    pub rego_source: String,
    /// Hash of the exact stored source, used to attribute a decision to the
    /// revision that produced it.
    pub matcher_content_hash: String,
    /// ID of the user who wrote the revision, or null when unattributed.
    pub created_by: Option<ID>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
}

/// A rule set together with the revision currently in force.
#[derive(SimpleObject, Clone)]
pub struct RequestRuleSetDetail {
    /// The rule set itself.
    pub rule_set: RequestRuleSet,
    /// The revision currently in force.
    pub revision: RequestRuleRevision,
}

/// Identifier returned after deleting a request rule set.
#[derive(SimpleObject, Clone)]
pub struct DeleteRequestRuleSetPayload {
    /// Deleted rule-set ID.
    pub id: ID,
}

/// Result of validating request rule source without saving it.
#[derive(SimpleObject, Clone)]
pub struct RequestRuleValidationPayload {
    /// Whether the source is valid.
    pub valid: bool,
    /// Validation errors; empty when valid.
    pub errors: Vec<String>,
}

/// One reason a decision came out the way it did, in the only form that is safe
/// to show a requester: a stable code and the name of the rule that emitted it.
/// Rule bodies never reach this surface.
#[derive(SimpleObject, Clone)]
pub struct RequestPreflightReason {
    /// Stable reason code the rule emitted.
    pub code: String,
    /// Name of the rule that emitted it.
    pub rule_name: String,
}

/// One rule's recorded vote inside a decision trace.
///
/// `vote` is null exactly when `error` is set: a rule that failed produced no
/// vote, and rendering that as an abstain would say the rule looked and had no
/// opinion.
#[derive(SimpleObject, Clone)]
pub struct RequestRuleVote {
    /// Rule set that voted.
    pub rule_set_id: ID,
    /// That rule set's name as it was when the vote was cast. Recorded on the
    /// trace, so it survives the rule being renamed or deleted.
    pub rule_set_name: String,
    /// Revision that voted.
    pub revision_number: i32,
    /// The vote, or null when the rule failed.
    pub vote: Option<RequestVoteValue>,
    /// True when Scryer held the rule because a fact it reads was unobservable,
    /// rather than the author writing `manual`. Both arbitrate identically.
    pub held: bool,
    /// Reason codes the rule emitted.
    pub reason_codes: Vec<String>,
    /// Tags the rule emitted.
    pub tags: Vec<String>,
    /// Why the rule failed, or null when it did not.
    pub error: Option<String>,
}

/// The durable trace of one request evaluation (FR-016).
///
/// `policyOutcome` is what the rules concluded and `effectiveOutcome` is what
/// the instance acted on. They disagree on purpose whenever the instance gate is
/// off or every deciding rule is in shadow.
#[derive(SimpleObject, Clone)]
pub struct RequestRuleDecision {
    /// Trace ID, or null for a preview, which persists nothing.
    pub id: Option<ID>,
    /// The media request this decision belongs to, or null for a preview.
    pub request_id: Option<ID>,
    /// UTC time the evaluation ran.
    pub evaluated_at: DateTime<Utc>,
    /// Strictest mode among the rules that were consulted.
    pub mode: RequestRuleEvaluationModeValue,
    /// What actually happened to the request.
    pub effective_outcome: RequestDecisionOutcomeValue,
    /// What the rules concluded, whether or not they were allowed to act.
    pub policy_outcome: RequestDecisionOutcomeValue,
    /// Why the decision fell back rather than following a rule vote
    /// (`no_rule_matched`, `held`, `error`, `rule_manual`), or null when a rule
    /// decided.
    pub fallback_reason: Option<String>,
    /// Per-rule votes. **Empty for a requester reading their own request's
    /// decision**, who sees `reasons` instead.
    pub votes: Vec<RequestRuleVote>,
    /// Reason codes with the rules that emitted them, flattened out of the
    /// votes. Safe for every audience, and the only explanation a requester
    /// gets.
    pub reasons: Vec<RequestPreflightReason>,
    /// Tags the rules emitted. Recorded on every path; applied to a title only
    /// on an approval.
    pub tags: Vec<String>,
    /// Schema version of the input document the rules were evaluated against.
    pub input_schema_version: i32,
}

/// Author-side preview of one matcher against one hypothetical request.
///
/// Nothing is persisted but the trace the evaluation writes, and the caller
/// already holds catalog-settings authority, so this is the one place the whole
/// input document is returned.
#[derive(SimpleObject, Clone)]
pub struct RequestRulePreviewPayload {
    /// Stored rule set the preview ran, or the throwaway ID an unsaved draft
    /// compiled under.
    pub rule_set_id: String,
    /// Hash of the exact source that produced this outcome.
    pub matcher_content_hash: String,
    /// The single rule's verdict, shaped as a decision so the authoring UI and
    /// the decision browser render the same component. `effectiveOutcome`
    /// equals `policyOutcome` here because a preview is never subject to the
    /// instance gate.
    pub decision: RequestRuleDecision,
    /// True when the sample's metadata could not be fully established, so some
    /// facts were unknown.
    pub metadata_partial: bool,
    /// Of the tags the rule emitted, the ones the title tag registry does not
    /// define. They are dropped at approval, so an author needs to define them
    /// in Settings before the rule can apply them. `decision.tags` still lists
    /// everything the rule emitted.
    pub undefined_tags: Vec<String>,
    /// The exact document the rule saw.
    pub input_document: Json<serde_json::Value>,
}

/// Requester-side pre-flight: what would happen if this draft were submitted
/// now (FR-020).
///
/// Deliberately has no vote table and no input document. A requester sees the
/// outcome, the reasons behind it, and the tags an approval would stamp.
#[derive(SimpleObject, Clone)]
pub struct RequestPreflightPayload {
    /// What would actually happen if the draft were submitted now.
    pub outcome: RequestDecisionOutcomeValue,
    /// Why, in codes with the rule names that produced them.
    pub reasons: Vec<RequestPreflightReason>,
    /// Tags an approval would stamp on the created title.
    pub tags: Vec<String>,
    /// True when the metadata could not be fully established, so the verdict
    /// rests on incomplete facts. An unevaluable policy never approves.
    pub metadata_partial: bool,
    /// Strictest mode among the rules that were consulted.
    pub evaluation_mode: RequestRuleEvaluationModeValue,
    /// Why the verdict fell back to needing approval, when it did. One of the
    /// arbitration vocabulary's codes (`rule_manual`, `held`, `error`,
    /// `no_rule_matched`), or null when a rule decided outright. It carries no
    /// rule internals: it names the shape of the fallback, never a vote.
    pub fallback_reason: Option<String>,
}

/// The instance-wide request-rule gate. One switch, defaulting off: a request
/// rule votes and executes nothing, so there is one blast radius.
#[derive(SimpleObject, Clone, Copy)]
pub struct RequestRuleInstanceGates {
    /// Whether stored request rules are allowed to decide anything. While this
    /// is off every evaluation still runs and is recorded, but the effective
    /// outcome stays whatever the library permission would have produced.
    pub evaluation_enabled: bool,
}

/// One hold on one title.
#[derive(SimpleObject, Clone)]
pub struct TitleClaim {
    /// Claim ID.
    pub id: ID,
    /// Title the claim holds.
    pub title_id: ID,
    /// Library that title belongs to.
    pub library_id: ID,
    /// What produced the claim.
    pub producer: LifecycleClaimProducerValue,
    /// The producing media request's ID, or null for an operator pin.
    pub producer_ref: Option<String>,
    /// What the claim holds.
    pub kind: LifecycleClaimKindValue,
    /// Current lifecycle state.
    pub state: LifecycleClaimStateValue,
    /// Requested window in days for a retention claim; null for a keep.
    pub duration_days: Option<i32>,
    /// When the claim started running, which is the title's first import. Null
    /// while dormant.
    pub starts_at: Option<DateTime<Utc>>,
    /// When the window closes. Null while dormant and for a keep.
    pub expires_at: Option<DateTime<Utc>>,
    /// ID of the user the claim is attributed to, or null when unattributed.
    pub created_by: Option<ID>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
    /// Why the claim was released or converted; kept as history, never cleared.
    pub released_reason: Option<String>,
}

/// The lease one media request holds, derived from the live claim it produced.
/// Null on the request payload until an approval creates that claim.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestLease {
    /// Days the requester asked for; null means forever.
    pub requested_days: Option<i32>,
    /// Days the approver granted; null means forever.
    pub approved_days: Option<i32>,
    /// State of the claim holding the title.
    pub state: LifecycleClaimStateValue,
    /// When the lease started running, which is the title's first import. Null
    /// while the claim is dormant.
    pub starts_at: Option<DateTime<Utc>>,
    /// When the lease runs out. Null while dormant and for a forever request.
    pub expires_at: Option<DateTime<Utc>>,
}

/// The metadata a media request was decided against, read back out of the
/// snapshot captured when it was submitted.
///
/// `partial` is the load-bearing field: it separates "the source says this title
/// has no content rating" from "we could not ask the source", and a fact derived
/// from a missing group reads as unknown rather than absent.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestMetadataPayload {
    /// True when enrichment did not fully succeed.
    pub partial: bool,
    /// Groups that could not be established (`content_ratings`, `genres`,
    /// `mdblist`, `ratings`, `awards`, or `all`).
    pub missing: Vec<String>,
    /// Genres captured at submit time.
    pub genres: Vec<String>,
    /// Content ratings captured at submit time, one entry per country.
    pub content_ratings: Vec<DiscoveryContentRatingPayload>,
    /// Minimum age the captured ratings imply, or null when none of them said.
    pub age_rating: Option<i32>,
    /// US certification label, or null when the title has no US certification.
    pub certification_label: Option<String>,
    /// That label's rank on Scryer's US ladder, or null when the label is not
    /// on it. A label off the ladder is an absence of a rank, not an unknown.
    pub certification_rank: Option<i32>,
    /// True when any captured canonical tag is flagged adult.
    pub is_adult: bool,
    /// TMDB vote average, published for movies only.
    pub tmdb_vote_average: Option<f64>,
    /// TMDB vote count, published for movies only.
    pub tmdb_vote_count: Option<i32>,
    /// TMDB popularity, published for movies only.
    pub popularity: Option<f64>,
    /// How many awards the snapshot captured.
    pub award_count: i32,
}

// ── Inputs ─────────────────────────────────────────────────────────────────

/// Creates a request rule set together with its first matcher revision. The
/// rule set is always created disabled; arming it is a second, deliberate call.
#[derive(InputObject)]
pub struct CreateRequestRuleSetInput {
    /// Rule-set name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Matcher source as written in the editor. The package declaration is
    /// applied by the server.
    pub rego_source: String,
    /// Libraries to confine the rule to. Omitted or empty means every library.
    pub library_ids: Option<Vec<String>>,
}

/// Replaces the matcher of an existing rule set, appending revision N+1.
#[derive(InputObject)]
pub struct UpdateRequestRuleMatcherInput {
    /// Rule-set ID.
    pub rule_set_id: ID,
    /// Replacement matcher source as written in the editor.
    pub rego_source: String,
}

/// Renames and re-scopes a rule set without touching its matcher, so no
/// revision is created.
#[derive(InputObject)]
pub struct UpdateRequestRuleMetadataInput {
    /// Rule-set ID.
    pub rule_set_id: ID,
    /// Replacement name.
    pub name: String,
    /// Replacement description; omitted clears it.
    pub description: Option<String>,
    /// Replacement library scope. Omitted or empty means every library.
    pub library_ids: Option<Vec<String>>,
}

/// Moves a rule set between evaluation modes.
#[derive(InputObject)]
pub struct SetRequestRuleModeInput {
    /// Rule-set ID.
    pub rule_set_id: ID,
    /// Mode to store. Anything other than `DISABLED` also enables the rule.
    pub mode: RequestRuleEvaluationModeValue,
}

/// Validates request rule source without saving it.
#[derive(InputObject)]
pub struct ValidateRequestRuleInput {
    /// Matcher source to validate, as written in the editor.
    pub rego_source: String,
}

/// The hypothetical request an author-side preview is evaluated against.
///
/// It names a real user and a real library on purpose: the question a preview
/// answers is "what would this rule do to *this person* asking for *this
/// title*", and a synthetic requester would exercise none of the permission,
/// history, or linked-provider facts that make request rules useful.
#[derive(InputObject, Clone)]
pub struct RequestRuleSampleInput {
    /// User to evaluate as.
    pub user_id: ID,
    /// Library to evaluate against.
    pub library_id: ID,
    /// Provider identifiers of the sample title. An empty list is a legitimate
    /// preview of requester facts alone, and leaves every metadata fact
    /// unknown.
    pub external_ids: Vec<ExternalIdInput>,
    /// Quality profile the sample requests.
    pub quality_profile_id: Option<ID>,
    /// Monitoring policy the sample requests.
    pub monitor_type: Option<MonitorTypeValue>,
    /// Lease the sample requests, in days.
    pub lease_days: Option<i32>,
    /// Set to true for a sample that asks to keep the title forever. Rejected
    /// together with `leaseDays`.
    pub lease_forever: Option<bool>,
}

/// Runs one matcher against one hypothetical request without saving anything.
///
/// Supply either `ruleSetId` to preview a stored rule set or `regoSource` to
/// preview an unsaved draft, never both: they answer different questions, and
/// accepting both would make it ambiguous which source produced the verdict.
#[derive(InputObject)]
pub struct PreviewRequestRuleInput {
    /// Stored rule set to preview at its current revision.
    pub rule_set_id: Option<ID>,
    /// Unsaved matcher source to preview, as written in the editor.
    pub rego_source: Option<String>,
    /// The request to evaluate the matcher against.
    pub sample: RequestRuleSampleInput,
}

/// Arms or disarms the instance-wide request-rule gate. An omitted field leaves
/// the gate exactly as stored.
#[derive(InputObject)]
pub struct SetRequestRuleInstanceGatesInput {
    /// Whether stored request rules may decide anything.
    pub evaluation_enabled: Option<bool>,
}

/// Pushes a live claim's window out.
#[derive(InputObject)]
pub struct ExtendTitleClaimInput {
    /// Claim to extend.
    pub claim_id: ID,
    /// New expiry. Refused for a claim that is no longer live: an expired lease
    /// is replaced, not extended.
    pub expires_at: DateTime<Utc>,
}

/// Replaces a live claim with a permanent operator keep. The original stays as
/// history in the `CONVERTED` state.
#[derive(InputObject)]
pub struct ConvertTitleClaimToPermanentInput {
    /// Claim to convert.
    pub claim_id: ID,
}

/// Withdraws a claim by hand.
#[derive(InputObject)]
pub struct ReleaseTitleClaimInput {
    /// Claim to release.
    pub claim_id: ID,
    /// Why it is being released. Required by the service: a hold withdrawn
    /// without a reason leaves nothing to explain the deletion it permits.
    pub reason: Option<String>,
}
