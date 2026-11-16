//! Wire types for request rules, pre-flight, decision traces, and lifecycle
//! claims.
//!
//! Two audiences read this surface and they are not allowed to see the same
//! things. An author or an approver gets the whole trace: which rule voted what,
//! on which revision, and the document it saw. A **requester** gets the verdict,
//! the reasons behind it, and the tags an approval would stamp — never a vote
//! table, never a line of Rego, never the input document. The split is
//! not a convention here: [`RequestPreflightPayload`] has no field that could
//! carry a vote, and a [`RequestRuleDecision`] read by its own requester comes
//! back with `votes` emptied.

use async_graphql::{Enum, ID, InputObject, Json, SimpleObject};
use chrono::{DateTime, Utc};

use super::{DiscoveryContentRatingPayload, ExternalIdInput, MonitorTypeValue};

// ── Enums ──────────────────────────────────────────────────────────────────

/// Whether an evaluation consulted an enabled request rule.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RequestRuleEvaluationModeValue {
    /// No enabled rule took part.
    Disabled,
    /// An enabled rule was evaluated, recorded, and acted on.
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
}

/// A head a request rule's source can define. Votes decide the outcome; the
/// other heads shape what an approval does or say something to the people who
/// are notified about the request.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(name = "RequestActionHead", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RequestActionHeadValue {
    /// Vote to approve.
    Approve,
    /// Vote to refuse. Outranks every other vote.
    Deny,
    /// Vote to leave the request for a person.
    Manual,
    /// Approve into the named quality profile.
    SetQualityProfile,
    /// Approve with this lease, in days; `0` keeps the title forever.
    SetLeaseDays,
    /// Approve with this monitoring policy.
    SetMonitor,
    /// Approve with these required audio languages.
    SetRequiredAudioLanguages,
    /// Approve with these preferred audio languages, in order.
    SetPreferredAudioLanguages,
    /// Stamp these tags on the title an approval creates.
    AddTags,
    /// Add this text to the notification sent when the request is submitted.
    Notify,
}

/// What a lifecycle claim is.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleClaimKindValue {
    /// A time to delete: `expiresAt`, whose clock starts at the title's first
    /// import.
    RetainUntil,
}

/// Lifecycle state of one claim. The terminal states are history, and only
/// `DORMANT` and `ACTIVE` are live.
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
    /// Ended by an administrator keeping the title permanently, which leaves
    /// it with no expiry.
    Converted,
}

// ── Read models ────────────────────────────────────────────────────────────

/// A request rule set: identity, scope, and evaluation state. The source lives
/// on the revision, never here.
#[derive(SimpleObject, Clone)]
pub struct RequestRuleSet {
    /// Rule-set ID.
    pub id: ID,
    /// Rule-set name.
    pub name: String,
    /// Rule-set description; empty when the author supplied none.
    pub description: String,
    /// Whether the rule set takes part in deciding requests.
    pub enabled: bool,
    /// Libraries the rule is confined to. Empty means every library.
    pub library_ids: Vec<String>,
    /// Revision number of the source currently in force.
    pub current_revision_number: i32,
    /// How many recorded decisions this rule took part in. Traces outlive the
    /// rule that produced them, so this can be non-zero for a rule that is now
    /// disabled.
    pub decision_count: i32,
    /// What the revision in force can do, read from its source. Null when the
    /// source cannot be read.
    pub declared_actions: Option<RequestDeclaredActions>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

/// Every head a request rule's source defines, and every literal value each
/// valued head can produce.
#[derive(SimpleObject, Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestDeclaredActions {
    /// The heads the source defines.
    pub heads: Vec<RequestActionHeadValue>,
    /// Quality profile names `set_quality_profile` can set.
    pub quality_profiles: Vec<String>,
    /// Lease lengths `set_lease_days` can set; `0` is forever.
    pub lease_days: Vec<i32>,
    /// Monitoring policies `set_monitor` can set.
    pub monitors: Vec<String>,
    /// Audio language codes `set_required_audio_languages` can set.
    pub required_audio_languages: Vec<String>,
    /// Audio language codes `set_preferred_audio_languages` can set.
    pub preferred_audio_languages: Vec<String>,
    /// Tags `add_tags` can stamp.
    pub add_tags: Vec<String>,
    /// Texts `notify` can send.
    pub notifications: Vec<String>,
}

/// One text value and the rule set that set it.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct RequestShapedText {
    /// The value.
    pub value: String,
    /// Name of the rule set that set it, as it was when the request was
    /// evaluated.
    pub rule_set_name: String,
}

/// One number and the rule set that set it.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct RequestShapedNumber {
    /// The value.
    pub value: i32,
    /// Name of the rule set that set it, as it was when the request was
    /// evaluated.
    pub rule_set_name: String,
}

/// One list of texts and the rule set that set it.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct RequestShapedTextList {
    /// The value.
    pub value: Vec<String>,
    /// Name of the rule set that set it, as it was when the request was
    /// evaluated.
    pub rule_set_name: String,
}

/// What an approval of a request is changed to by its rules.
///
/// Applied on an automatic approval, offered to the approver on manual review,
/// and kept in the trace only on a denial. A head two rules set to different
/// values is listed in `conflicts`, and a head whose value cannot apply to the
/// request in `invalid`; neither carries a value.
#[derive(SimpleObject, Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestShaping {
    /// Quality profile name.
    pub quality_profile: Option<RequestShapedText>,
    /// Lease in days; `0` keeps the title forever.
    pub lease_days: Option<RequestShapedNumber>,
    /// Monitoring policy.
    pub monitor: Option<RequestShapedText>,
    /// Required audio language codes.
    pub required_audio_languages: Option<RequestShapedTextList>,
    /// Preferred audio language codes, in order.
    pub preferred_audio_languages: Option<RequestShapedTextList>,
    /// Texts the rules added to the submission notification, in evaluation
    /// order. Always empty on the requester's own surfaces.
    pub notifications: Vec<RequestShapedText>,
    /// Heads two rules set to different values.
    pub conflicts: Vec<RequestActionHeadValue>,
    /// Heads whose value cannot apply to this request.
    pub invalid: Vec<RequestActionHeadValue>,
}

/// One immutable revision of a request rule set's source.
#[derive(SimpleObject, Clone)]
pub struct RequestRuleRevision {
    /// Revision ID.
    pub id: ID,
    /// ID of the rule set this revision belongs to.
    pub rule_set_id: ID,
    /// Revision number, starting at one and incremented on every source edit.
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
    /// The visual builder's stored document, or null when the rule was not
    /// written with the builder. Read only by the single rule set query.
    pub builder_model: Option<Json<serde_json::Value>>,
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
    /// Structured conditions and source locations.
    pub diagnostics: Vec<super::rules::RuleValidationDiagnosticPayload>,
    /// Whether the source is valid.
    pub valid: bool,
    /// Validation errors; empty when valid.
    pub errors: Vec<String>,
    /// Findings that never block saving.
    pub warnings: Vec<super::rules::RuleValidationDiagnosticPayload>,
    /// What the source can do, or null when it could not be read.
    pub declared_actions: Option<RequestDeclaredActions>,
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

/// The durable trace of one request evaluation. It outlives the rules that
/// made it.
///
/// `policyOutcome` is what the rules concluded and `effectiveOutcome` is what
/// the instance acted on. They disagree only when the rules approved and the
/// title tag registry could not be read: the request is held for approval,
/// with the fallback `error`. When the input facts could not be assembled or
/// the rules could not be evaluated, both are `MANUAL_REVIEW` with the
/// fallback `error`. Decisions recorded by 0.22 and earlier may also show
/// `policyOutcome` `MANUAL_REVIEW` with the Auto-Approve permission's
/// `effectiveOutcome` for that case.
#[derive(SimpleObject, Clone)]
pub struct RequestRuleDecision {
    /// Trace ID, or null for a preview, which persists nothing.
    pub id: Option<ID>,
    /// The media request this decision belongs to, or null for a preview.
    pub request_id: Option<ID>,
    /// UTC time the evaluation ran.
    pub evaluated_at: DateTime<Utc>,
    /// `ENFORCE` when an enabled rule was consulted, `DISABLED` when none was.
    pub mode: RequestRuleEvaluationModeValue,
    /// What actually happened to the request.
    pub effective_outcome: RequestDecisionOutcomeValue,
    /// What the rules concluded.
    pub policy_outcome: RequestDecisionOutcomeValue,
    /// Why the decision fell back rather than following a rule's own
    /// `approve` or `deny`: `rule_manual`, `held`, `error`, `no_rule_matched`,
    /// `library_permission` (the Auto-Approve permission approved),
    /// `shaping_conflict` or `shaping_invalid`. Null when a rule decided.
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
    /// What the rules changed an approval to, or null when they changed
    /// nothing.
    pub shaping: Option<RequestShaping>,
}

/// Author-side preview of one rule against one hypothetical request.
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
    /// equals `policyOutcome` here.
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
/// now.
///
/// Deliberately has no vote table and no input document. A requester sees the
/// outcome, the reasons behind it, and the tags an approval would stamp.
#[derive(SimpleObject, Clone)]
pub struct RequestPreflightPayload {
    /// What would actually happen if the draft were submitted now.
    pub outcome: RequestDecisionOutcomeValue,
    /// Why, in codes with the rule names that produced them.
    pub reasons: Vec<RequestPreflightReason>,
    /// Tags an approval would stamp on the created title. Empty when the
    /// draft would be denied.
    pub tags: Vec<String>,
    /// True when the metadata could not be fully established, so the verdict
    /// rests on incomplete facts. A rule that reads a fact that could not be
    /// established is held rather than run.
    pub metadata_partial: bool,
    /// `ENFORCE` when an enabled rule was consulted, `DISABLED` when none was.
    pub evaluation_mode: RequestRuleEvaluationModeValue,
    /// Why the verdict is a fallback rather than a rule's own `approve` or
    /// `deny`: `rule_manual`, `held`, `error`, `no_rule_matched`,
    /// `library_permission` (the Auto-Approve permission approved),
    /// `shaping_conflict` or `shaping_invalid`. Null when a rule decided
    /// outright. It carries no rule internals: it names the shape of the
    /// fallback, never a vote.
    pub fallback_reason: Option<String>,
    /// What an approval would be changed to, or null when nothing would be.
    /// Never carries a notification.
    pub shaping: Option<RequestShaping>,
}

/// One request lease on one title: a time to delete that rules read.
#[derive(SimpleObject, Clone)]
pub struct TitleClaim {
    /// Claim ID.
    pub id: ID,
    /// Title the claim is on.
    pub title_id: ID,
    /// Library that title belongs to.
    pub library_id: ID,
    /// What produced the claim.
    pub producer: LifecycleClaimProducerValue,
    /// The producing media request's ID.
    pub producer_ref: Option<String>,
    /// What the claim is.
    pub kind: LifecycleClaimKindValue,
    /// Current lifecycle state.
    pub state: LifecycleClaimStateValue,
    /// Requested window in days.
    pub duration_days: Option<i32>,
    /// When the claim started running, which is the title's first import. Null
    /// while dormant.
    pub starts_at: Option<DateTime<Utc>>,
    /// When the window closes. Null while dormant.
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

/// The lease one media request holds, derived from the claim it produced. Null
/// on the request payload until an approval creates that claim, and for a
/// request approved without a lease, which has none.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestLease {
    /// Days the requester asked for; null means forever.
    pub requested_days: Option<i32>,
    /// Days the approver granted; null means forever.
    pub approved_days: Option<i32>,
    /// State of the lease's claim.
    pub state: LifecycleClaimStateValue,
    /// When the lease started running, which is the title's first import. Null
    /// while the claim is dormant.
    pub starts_at: Option<DateTime<Utc>>,
    /// When the lease runs out. Null while dormant.
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

/// Creates a request rule set together with its first source revision. The
/// rule set is always created disabled; enabling it is a second, deliberate call.
#[derive(InputObject)]
pub struct CreateRequestRuleSetInput {
    /// Rule-set name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Rule source as written in the editor. The package declaration is
    /// applied by the server.
    pub rego_source: String,
    /// Libraries to confine the rule to. Omitted or empty means every library.
    pub library_ids: Option<Vec<String>>,
    /// The visual builder's document, stored as given: a JSON object of at
    /// most 64 KiB. Omitted or null stores none.
    pub builder_model: Option<Json<serde_json::Value>>,
}

/// Replaces the source of an existing rule set, appending revision N+1.
#[derive(InputObject)]
pub struct UpdateRequestRuleMatcherInput {
    /// Rule-set ID.
    pub rule_set_id: ID,
    /// Replacement rule source as written in the editor.
    pub rego_source: String,
    /// The visual builder's document: a JSON object of at most 64 KiB.
    /// Omitted keeps the stored one; null clears it.
    pub builder_model: async_graphql::MaybeUndefined<Json<serde_json::Value>>,
}

/// Renames and re-scopes a rule set without touching its source, so no
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

/// Validates request rule source without saving it.
#[derive(InputObject)]
pub struct ValidateRequestRuleInput {
    /// Matcher source to validate, as written in the editor.
    pub rego_source: String,
    /// Libraries the rule would be confined to, which decide the monitoring
    /// policies it may set. Omitted or empty means every library.
    pub library_ids: Option<Vec<String>>,
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
    /// Audio languages the sample requires. Omit or send null to leave it to
    /// the library; an empty list asks for no requirement.
    pub required_audio_languages: Option<Vec<String>>,
    /// Audio languages the sample prefers, in order. Omit or send null to
    /// leave it to the library.
    pub preferred_audio_languages: Option<Vec<String>>,
}

/// Runs one rule against one hypothetical request without saving anything.
///
/// Supply either `ruleSetId` to preview a stored rule set or `regoSource` to
/// preview an unsaved draft, never both: they answer different questions, and
/// accepting both would make it ambiguous which source produced the verdict.
#[derive(InputObject)]
pub struct PreviewRequestRuleInput {
    /// Stored rule set to preview at its current revision.
    pub rule_set_id: Option<ID>,
    /// Unsaved rule source to preview, as written in the editor.
    pub rego_source: Option<String>,
    /// The request to evaluate the rule against.
    pub sample: RequestRuleSampleInput,
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

/// Ends a live lease so the title carries no expiry. The lease stays as history
/// in the `CONVERTED` state.
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
