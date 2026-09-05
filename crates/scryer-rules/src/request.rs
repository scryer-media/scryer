//! Request policy family: decide a media request at the moment it is submitted.
//!
//! A request rule answers one question — should Scryer approve this request by
//! itself, deny it, or put it in front of a human — and it answers it while the
//! requester is waiting. That single fact shapes everything here. The evaluation
//! budget is the tightest of any family (100 ms, [`RuntimeLimits::request_defaults`]).
//! Nothing a rule can do is allowed to fail the submission: a rule that errors,
//! times out, or returns a malformed decision produces no vote at all, and the
//! application's arbitration reads the absence as manual review.
//!
//! The three votes are not symmetric. `deny` and `approve` are decisions;
//! `manual` is the safe answer, and it is what everything uncertain collapses
//! to. So `manual` wins inside a rule (a rule that says both "approve" and
//! "manual" is a rule that is not sure), and the host's own hold — a rule that
//! reads a fact Scryer could not observe — is expressed as that same `manual`,
//! with the observation's reason codes attached so the approver sees *which*
//! fact was missing rather than a bare "needs review".
//!
//! Facts arrive in the shared [`Observation`] envelopes and are subject to the
//! same host-derived hold as maintenance facts: a request rule reading a
//! certification Scryer never resolved is held before it is consulted, because
//! on the bare `input.facts` surface an unknown certification is just a missing
//! key, which a rule would read as a decisive "not rated above PG-13".
//!
//! Rules may also stamp `tags` on the title the request creates. Tags are
//! collected on every path where the rule actually ran — including an abstain,
//! because "this is a kids' film" is true whether or not the rule had an opinion
//! about approving it — and never on a held rule, which by construction did not
//! run.

use crate::RulesError;
use crate::policy::decode::{decode_reasons, decode_tags};
use crate::policy::engine::{EvalOutcome, EvalRecord, PolicyEngine, PolicyEvaluator, RuleHandle};
use crate::policy::observation::serialize_fact_namespaces;
use crate::policy::wrapper::{WrapperField, object_wrapper_source};
use crate::policy::{PolicyFamily, PolicyRecord};
use crate::runtime::RuntimeLimits;
use crate::validation;
use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use regorus::Value;
use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};
use std::collections::{BTreeMap, BTreeSet};

/// Three-valued availability envelope wrapping every request fact. Owned by the
/// shared policy core; re-exported here so a caller building a request input
/// reaches for it through the family whose facts it is describing.
pub use crate::policy::observation::Observation;

// ── Contract constants ──────────────────────────────────────────────────────

/// Version of the request input document. Rules are authored against a specific
/// version; bumping it is a breaking change to every stored request rule, and
/// it is recorded on every decision trace so an old decision stays explainable
/// against the surface it was made on.
pub const REQUEST_INPUT_SCHEMA_VERSION: u32 = 1;

/// Package prefix for user-authored request rules.
pub(crate) const USER_PACKAGE_PREFIX: &str = "scryer.request.user";

/// Package prefix for the generated evaluation wrapper. Separate from the user
/// package because the wrapper reads that package's whole document.
const WRAPPER_PACKAGE_PREFIX: &str = "scryer.request.wrapper";

/// The head an author writes to send a request to a human themselves.
/// Maintenance says `unknown if`; requests say `manual if`. The name is a
/// per-family parameter of the policy core, and the wrapper below reads exactly
/// what this constant says, so the two cannot drift.
const HOLD_RULE_NAME: &str = "manual";

/// Root of the input surface that names an identifiable person.
///
/// Authoring or previewing a rule that reads anything under here is a way of
/// asking the instance about a specific user, so the host requires the author to
/// already hold the instance's permission-management authority — the same gate
/// [`crate::maintenance::PERSON_TARGETED_MAINTENANCE_FACTS`] guards for
/// maintenance. Unlike maintenance, where person-targeting is a property of
/// individual facts, *every* field of the requester document is about one named
/// person, so the whole subtree is the unit.
///
/// [`validation::request_person_targeted_paths`] is what turns a rule source
/// into the concrete list of paths under this root that it reads.
pub const PERSON_TARGETED_REQUEST_ROOT: &str = "input.requester";

// ── Input document ──────────────────────────────────────────────────────────

/// Input document set once per request evaluation.
///
/// `evaluation_time` is supplied by the host so a pre-flight preview and the
/// submit that follows it compare against the same instant, and so a re-run of
/// the same draft decides the same way. Policies must never reach for a clock of
/// their own.
///
/// Everything under `requester`, `library`, `request`, and `now` is always
/// known: it is the draft the requester is looking at, and the account they are
/// signed in as. Everything under `facts` is an [`Observation`] and may be
/// unknown — which holds the rule — or absent, which is a real answer. The fact
/// snapshot serializes into two namespaces exactly as maintenance's does:
/// `observations` is the envelope map verbatim, `facts` is derived from it by
/// unwrapping the known values.
#[derive(Debug, Clone)]
pub struct RequestInput {
    pub schema_version: u32,
    pub evaluation_time: DateTime<Utc>,
    pub now: RequestClockDoc,
    pub requester: RequestRequesterDoc,
    pub library: RequestLibraryDoc,
    pub request: RequestDoc,
    pub facts: RequestFactsDoc,
}

impl Serialize for RequestInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let namespaces =
            serialize_fact_namespaces(&self.facts).map_err(serde::ser::Error::custom)?;
        let mut state = serializer.serialize_struct("RequestInput", 8)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("evaluation_time", &self.evaluation_time)?;
        state.serialize_field("now", &self.now)?;
        state.serialize_field("requester", &self.requester)?;
        state.serialize_field("library", &self.library)?;
        state.serialize_field("request", &self.request)?;
        state.serialize_field("facts", &namespaces.facts)?;
        state.serialize_field("observations", &namespaces.observations)?;
        state.end()
    }
}

/// Calendar convenience derived from `evaluation_time`.
///
/// The `time.*` builtins can compute both of these from `input.evaluation_time`,
/// but "not on weekends" is a thing operators want to write in one line, and a
/// rule that parses timestamps to get there is a rule with more ways to be
/// wrong. Both fields are UTC, deliberately: an instance-local weekday would
/// make the same rule decide differently after a timezone change.
#[derive(Debug, Clone, Serialize)]
pub struct RequestClockDoc {
    /// Lowercase English weekday, `"monday"` … `"sunday"`, in UTC.
    pub weekday: String,
    /// Hour of the day in UTC, 0–23.
    pub hour_utc: u32,
}

impl RequestClockDoc {
    /// The clock document for an evaluation instant. The one place the weekday
    /// vocabulary is decided, so a fact builder and this contract cannot
    /// disagree about whether Sunday is `"sunday"` or `"Sun"`.
    pub fn at(evaluation_time: DateTime<Utc>) -> Self {
        Self {
            weekday: match evaluation_time.weekday() {
                Weekday::Mon => "monday",
                Weekday::Tue => "tuesday",
                Weekday::Wed => "wednesday",
                Weekday::Thu => "thursday",
                Weekday::Fri => "friday",
                Weekday::Sat => "saturday",
                Weekday::Sun => "sunday",
            }
            .to_string(),
            hour_utc: evaluation_time.hour(),
        }
    }
}

/// The account the request is being submitted by.
///
/// Every field here names or describes one identifiable person; see
/// [`PERSON_TARGETED_REQUEST_ROOT`].
#[derive(Debug, Clone, Serialize)]
pub struct RequestRequesterDoc {
    pub user_id: String,
    pub username: String,
    /// `"local"` or `"external_auto_provisioned"`.
    pub account_kind: String,
    /// Instance-wide permissions held by the account.
    pub app_permissions: Vec<String>,
    /// Permissions the account holds on the *target* library.
    pub library_permissions: Vec<String>,
    /// External accounts verified as linked to this one.
    pub linked_providers: Vec<String>,
    /// RFC3339. Optional because accounts predating the column have none.
    pub created_at: Option<String>,
}

/// The library the request is targeting.
#[derive(Debug, Clone, Serialize)]
pub struct RequestLibraryDoc {
    pub id: String,
    pub name: String,
    pub facet: String,
    pub is_default: bool,
}

/// The draft request itself — what the requester chose in the dialog.
#[derive(Debug, Clone, Serialize)]
pub struct RequestDoc {
    /// Fixed to `"manual"` today. The field exists so a watchlist-sourced
    /// request can slot in later without a schema bump.
    pub origin: String,
    pub title: String,
    pub year: Option<i32>,
    pub external_ids: BTreeMap<String, String>,
    pub quality_profile_id: Option<String>,
    pub quality_profile_name: Option<String>,
    pub monitor_type: Option<String>,
    pub monitor_selection_season_count: Option<i64>,
    /// True when the requester asked to keep the title indefinitely. `lease_days`
    /// is then meaningless and absent, which is why a rule about finite leases
    /// has to say `not input.request.lease_forever` first.
    pub lease_forever: bool,
    pub lease_days: Option<i64>,
}

/// One content-rating certification as the metadata source reported it.
#[derive(Debug, Clone, Serialize)]
pub struct RequestCertificationDoc {
    pub country: String,
    pub value: String,
    pub source: String,
}

/// The fact snapshot, and the single source of truth for both input namespaces.
///
/// Every field is an observation envelope. A fact Scryer failed to resolve stays
/// unknown rather than defaulting to a value the rule would read as decisive —
/// which for a request family is the difference between "this film is not adult"
/// and "nobody could tell me whether this film is adult".
#[derive(Debug, Clone, Serialize)]
pub struct RequestFactsDoc {
    // ── content rating ──
    /// Minimum age the content is rated for.
    pub age_rating: Observation<i64>,
    /// Every certification the sources reported, flattened.
    pub certifications: Observation<Vec<RequestCertificationDoc>>,
    /// The US certification value, when there is one (`G`…`NC-17`, `TV-Y`…`TV-MA`).
    pub certification_label: Observation<String>,
    /// The label placed on the host ladder, 0–4. Unknown without a US label —
    /// see [`certification_rank_for_label`].
    pub certification_rank: Observation<i64>,
    pub commonsense_recommended: Observation<bool>,

    // ── title metadata ──
    pub genres: Observation<Vec<String>>,
    pub canonical_tag_keys: Observation<Vec<String>>,
    pub themes: Observation<Vec<String>>,
    pub is_adult: Observation<bool>,
    pub rating: Observation<f64>,
    /// Normalized ratings keyed by source. The key set comes from the metadata
    /// gateway and is not catalogued, so a rule reads one source with
    /// `object.get(input.facts.ratings_by_source, "imdb", 0)` rather than by
    /// dotted path.
    pub ratings_by_source: Observation<BTreeMap<String, f64>>,
    pub tmdb_vote_average: Observation<f64>,
    pub tmdb_vote_count: Observation<i64>,
    pub popularity: Observation<f64>,
    pub runtime_minutes: Observation<i64>,
    pub original_language: Observation<String>,
    pub country: Observation<String>,
    pub network: Observation<String>,
    pub studio: Observation<String>,
    pub content_status: Observation<String>,
    /// RFC3339 or `YYYY-MM-DD`, as the source reported it.
    pub release_date: Observation<String>,
    pub first_aired: Observation<String>,
    /// Days between the release date and `evaluation_time`.
    pub release_age_days: Observation<i64>,
    pub award_count: Observation<i64>,

    // ── quality ──
    pub quality_profile_tiers: Observation<Vec<String>>,
    /// Highest vertical resolution the profile's tiers name — see
    /// [`max_resolution_for_quality_tiers`].
    pub quality_profile_max_resolution: Observation<i64>,
    pub quality_profile_allows_upgrades: Observation<bool>,

    // ── catalog ──
    /// Libraries of the same facet that already hold this identity.
    pub exists_in_library_ids: Observation<Vec<String>>,
    /// History for this identity fingerprint, across every requester.
    pub previous_request_count: Observation<i64>,
    pub previously_denied: Observation<bool>,
    pub previously_approved: Observation<bool>,

    // ── requester history ──
    pub pending_request_count: Observation<i64>,
    pub approved_last_30d: Observation<i64>,
    pub denied_last_30d: Observation<i64>,
    pub total_approved: Observation<i64>,
    pub active_lease_count: Observation<i64>,
    pub days_since_last_request: Observation<i64>,

    // ── library ──
    pub library_title_count: Observation<i64>,
}

// ── Contract helpers ────────────────────────────────────────────────────────

/// Place a certification label on the host's 0–4 ladder.
///
/// The ladder exists because certification *strings* are not comparable and
/// their vocabularies do not line up: a rule saying "nothing above PG-13" has to
/// mean the same thing for a film rated `PG-13` and a series rated `TV-14`. The
/// mapping is deliberately coarse and deliberately US-only — the rank is derived
/// from `certification_label`, which is the US value, and a title with no US
/// certification has no rank at all rather than a guessed one.
///
/// Lives here, beside the fact it produces, so the ladder and the fact that
/// carries it are read and changed together.
///
/// | rank | labels |
/// |---|---|
/// | 0 | `G`, `TV-Y`, `TV-Y7`, `TV-G` |
/// | 1 | `PG`, `TV-PG` |
/// | 2 | `PG-13`, `TV-14` |
/// | 3 | `R` |
/// | 4 | `NC-17`, `TV-MA` |
pub fn certification_rank_for_label(label: &str) -> Option<i64> {
    match label.trim().to_ascii_uppercase().as_str() {
        "G" | "TV-Y" | "TV-Y7" | "TV-G" => Some(0),
        "PG" | "TV-PG" => Some(1),
        "PG-13" | "TV-14" => Some(2),
        "R" => Some(3),
        "NC-17" | "TV-MA" => Some(4),
        _ => None,
    }
}

/// The vertical resolution one quality tier names, if it names one.
///
/// Tier labels are normalized to a numeric prefix plus `P` (`2160P`, `1080P`,
/// `720P`, `480P`); `4K` is accepted as the one common alias for 2160. Anything
/// else — `SDTV`, `RAW-HD`, a tier name with no number in front — names no
/// resolution and reports `None` rather than a zero a rule would compare
/// against.
fn resolution_for_quality_tier(tier: &str) -> Option<i64> {
    let tier = tier.trim().to_ascii_uppercase();
    if tier.starts_with("4K") {
        return Some(2160);
    }
    let digits: String = tier.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = &tier[digits.len()..];
    if !(rest.is_empty() || rest.starts_with('P')) {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// The highest vertical resolution a profile's tier list names.
///
/// This is what `input.facts.quality_profile_max_resolution` is: the one number
/// that makes "only 720p and below for this user" expressible without teaching
/// rules the profile's tier vocabulary. `None` when no tier names a resolution,
/// which leaves the fact unknown rather than claiming a profile allows nothing.
pub fn max_resolution_for_quality_tiers(tiers: &[String]) -> Option<i64> {
    tiers
        .iter()
        .filter_map(|tier| resolution_for_quality_tier(tier))
        .max()
}

// ── Output contract ─────────────────────────────────────────────────────────

/// What one rule said about the request.
///
/// `Abstain` is not a vote: a rule that abstains ran and had no opinion, which
/// is different in the trace from a rule that was never consulted and different
/// again from one that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestVote {
    Approve,
    Deny,
    Manual,
    Abstain,
}

/// The closed output of one request rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRuleDecision {
    pub vote: RequestVote,
    pub reason_codes: Vec<String>,
    /// Tags to stamp on the title the request creates. Collected on every path
    /// where the rule ran, including an abstain; always empty on a held rule,
    /// which did not run.
    pub tags: Vec<String>,
    /// True when the host held this rule because a fact it reads was
    /// unobservable, rather than the rule declaring `manual` itself. Both
    /// arbitrate the same way; only the trace tells them apart.
    pub held: bool,
}

/// One rule's decision, attributed to the exact policy revision that produced it
/// via `policy_content_hash`.
pub type RequestEvalRecord = EvalRecord<RequestRuleDecision>;

/// A per-rule failure. A rule that errors produces no record at all, so it can
/// never approve or deny; the application reads its absence as manual review.
#[derive(Debug, Clone)]
pub struct RequestEvalError {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub message: String,
}

pub type RequestEvalResult = EvalOutcome<RequestRuleDecision, RequestEvalError>;

// ── Package rewriting and wrapper generation ────────────────────────────────

/// Rewrite (or insert) the package declaration so stored request source always
/// carries the request prefix plus the system-assigned rule ID.
pub fn rewrite_package_declaration(rego_source: &str, rule_id: &str) -> String {
    crate::runtime::rewrite_package_declaration_with_prefix(
        rego_source,
        USER_PACKAGE_PREFIX,
        rule_id,
    )
}

/// Source for the generated evaluation entry point.
///
/// Five heads, each defaulted with `object.get`, which is what makes every one
/// of them optional: a rule that only denies leaves `approve`, `manual`, and
/// `tags` off its package document entirely and the defaults fill in. A head
/// that *is* defined but has the wrong type survives to the host, which rejects
/// it rather than coercing it — a `deny := "yes"` must not deny.
pub(crate) fn decision_wrapper_source(rule_id: &str) -> String {
    object_wrapper_source(
        WRAPPER_PACKAGE_PREFIX,
        USER_PACKAGE_PREFIX,
        rule_id,
        "decision",
        &[
            WrapperField::new("approve", "approve", "false"),
            WrapperField::new("deny", "deny", "false"),
            WrapperField::new("manual", HOLD_RULE_NAME, "false"),
            WrapperField::new("reasons", "reasons", "[]"),
            WrapperField::new("tags", "tags", "[]"),
        ],
    )
}

pub(crate) fn decision_wrapper_rule_path(rule_id: &str) -> String {
    format!("data.{WRAPPER_PACKAGE_PREFIX}.{rule_id}.decision")
}

pub(crate) fn decision_wrapper_policy_path(rule_id: &str) -> String {
    format!("internal/{rule_id}_request_wrapper.rego")
}

pub(crate) fn user_policy_path(rule_id: &str) -> String {
    format!("request/{rule_id}.rego")
}

// ── Engine ──────────────────────────────────────────────────────────────────

/// A request rule loaded from the database.
#[derive(Debug, Clone)]
pub struct RequestPolicy {
    pub id: String,
    pub name: String,
    pub rego_source: String,
}

/// The request policy family: what the shared core needs to know to run request
/// rules, and nothing else.
pub struct RequestFamily;

impl PolicyRecord for RequestPolicy {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn rego_source(&self) -> &str {
        &self.rego_source
    }
}

impl PolicyFamily for RequestFamily {
    const NAME: &'static str = "request";
    const USER_PACKAGE_PREFIX: &'static str = USER_PACKAGE_PREFIX;
    const WRAPPER_PACKAGE_PREFIX: &'static str = WRAPPER_PACKAGE_PREFIX;
    /// Request facts come from metadata Scryer may simply not have. A rule
    /// reading one it could not observe must reach a human, not read the missing
    /// key as a decisive answer.
    const TRACKS_REFERENCED_FACTS: bool = true;

    type Policy = RequestPolicy;
    type Input = RequestInput;
    type Decision = RequestRuleDecision;
    type RuleExtra = ();
    type EvalContext = ();
    type EvalError = RequestEvalError;

    fn limits() -> RuntimeLimits {
        RuntimeLimits::request_defaults()
    }

    fn user_policy_path(rule_id: &str) -> String {
        user_policy_path(rule_id)
    }

    fn wrapper_policy_path(rule_id: &str) -> String {
        decision_wrapper_policy_path(rule_id)
    }

    fn wrapper_source(rule_id: &str) -> String {
        decision_wrapper_source(rule_id)
    }

    fn wrapper_rule_path(rule_id: &str) -> String {
        decision_wrapper_rule_path(rule_id)
    }

    fn rule_extra(_policy: &Self::Policy) -> Self::RuleExtra {}

    /// A rule whose fact dependencies cannot be read off its source must not
    /// load at all: the host could not then tell whether it is deciding on
    /// evidence Scryer actually has.
    fn referenced_facts(
        policy: &Self::Policy,
        policy_path: &str,
    ) -> Result<BTreeSet<String>, String> {
        validation::request_fact_references(&policy.rego_source, policy_path)
    }

    fn hold_rule_name() -> Option<&'static str> {
        Some(HOLD_RULE_NAME)
    }

    /// A held rule contributes the safe vote and the observation's own reason
    /// codes — the approver sees which fact was missing. No tags: the rule never
    /// ran, so it never claimed one.
    fn held_decision(reason_codes: Vec<String>) -> Self::Decision {
        RequestRuleDecision {
            vote: RequestVote::Manual,
            reason_codes,
            tags: Vec::new(),
            held: true,
        }
    }

    fn decode(
        value: &Value,
        _rule_id: &str,
        _rule_name: &str,
        _extra: &Self::RuleExtra,
    ) -> Result<Self::Decision, String> {
        decode_decision(value)
    }

    fn eval_error(rule: &RuleHandle<Self::RuleExtra>, message: String) -> Self::EvalError {
        RequestEvalError {
            rule_set_id: rule.id.clone(),
            rule_set_name: rule.name.clone(),
            message,
        }
    }
}

/// Pre-compiled engine holding every enabled request rule.
///
/// Built once per rule-set revision and shared; evaluators are cheap clones
/// created per evaluation.
pub type RequestRulesEngine = PolicyEngine<RequestFamily>;

/// Evaluates every loaded request rule against one draft request.
pub type RequestRulesEvaluator = PolicyEvaluator<RequestFamily>;

impl RequestRulesEvaluator {
    /// Evaluate every loaded rule against one draft request.
    ///
    /// Per-rule failures — rules that error at runtime, exceed the 100 ms
    /// budget, or return a malformed decision — are collected and never abort
    /// the batch, and never fail the submission. A failing rule contributes no
    /// record, so it can neither approve nor deny; the caller arbitrates its
    /// absence as manual review.
    ///
    /// A rule that reads a fact Scryer could not observe for this request is
    /// held before it is consulted at all, and contributes
    /// [`RequestVote::Manual`] with the observation's reason codes. Rules that
    /// opt out by reading `input.observations.*` are consulted normally and may
    /// still declare their own `manual`, which composes with this one: either is
    /// enough to send the request to a human.
    pub fn evaluate(&mut self, input: &RequestInput) -> Result<RequestEvalResult, RulesError> {
        self.evaluate_policies(input, &())
    }
}

// ── Output validation ───────────────────────────────────────────────────────

/// Convert the wrapper's decision object into the closed output contract.
///
/// Fails closed: anything that is not exactly the declared shape becomes an
/// error for that rule instead of a coerced decision. That includes a malformed
/// *tag* — a rule that emits `kids/teens` produces no vote at all rather than a
/// vote with the tag quietly dropped. Losing a tag silently would change what
/// the title ends up looking like with nothing in the trace to say so, and the
/// fail-closed direction here is manual review, which is the safe one.
pub(crate) fn decode_decision(value: &Value) -> Result<RequestRuleDecision, String> {
    if matches!(value, Value::Undefined) {
        return Err("decision rule produced no value".to_string());
    }
    if value.as_object().is_err() {
        return Err("decision must be an object".to_string());
    }

    let approve = *value["approve"]
        .as_bool()
        .map_err(|_| "'approve' must be a boolean".to_string())?;
    let deny = *value["deny"]
        .as_bool()
        .map_err(|_| "'deny' must be a boolean".to_string())?;
    let manual = *value["manual"]
        .as_bool()
        .map_err(|_| "'manual' must be a boolean".to_string())?;
    let reason_codes = decode_reasons(&value["reasons"])?;
    let tags = decode_tags(&value["tags"])?;

    // Most restrictive wins inside one rule, and `manual` is the least
    // committed answer rather than the least permissive one: a rule saying both
    // "approve" and "manual" is a rule that is not sure, and an unsure rule must
    // not auto-approve. Denying on its behalf would be just as wrong, so manual
    // outranks deny too. Across rules the application arbitrates the other way
    // round (deny > manual > approve); the two orders answer different
    // questions.
    let vote = if manual {
        RequestVote::Manual
    } else if deny {
        RequestVote::Deny
    } else if approve {
        RequestVote::Approve
    } else {
        RequestVote::Abstain
    };

    Ok(RequestRuleDecision {
        vote,
        reason_codes,
        tags,
        held: false,
    })
}

// ── Worked examples ─────────────────────────────────────────────────────────

/// The family-rated approval from the plan's worked examples: named requesters,
/// PG-13 or gentler, tagged `family` when it is gentler still.
pub const EXAMPLE_NAMED_REQUESTERS_FAMILY_RATED: &str = "package rules\nimport rego.v1\n\nrequesters := {\"alice\", \"bob\", \"carol\"}\n\napprove if {\n\tinput.requester.username in requesters\n\tinput.facts.certification_rank <= 2\n}\n\ntags contains \"family\" if {\n\tinput.facts.certification_rank <= 1\n}\n";

/// Short leases only, for one requester. `lease_forever` has to be ruled out
/// first: a forever lease carries no `lease_days` at all, and a rule that only
/// compared the days would match it by accident.
pub const EXAMPLE_SHORT_LEASE: &str = "package rules\nimport rego.v1\n\napprove if {\n\tinput.requester.username == \"bob\"\n\tnot input.request.lease_forever\n\tinput.request.lease_days <= 14\n}\n";

/// One requester, 720p profiles or lower.
pub const EXAMPLE_LOW_RESOLUTION: &str = "package rules\nimport rego.v1\n\napprove if {\n\tinput.requester.username == \"alice\"\n\tinput.facts.quality_profile_max_resolution <= 720\n}\n";

/// Deny adult content, with a reason code the requester is shown.
pub const EXAMPLE_DENY_ADULT_CONTENT: &str = "package rules\nimport rego.v1\n\ndeny if {\n\tinput.facts.is_adult\n}\n\nreasons contains \"adult_content\" if {\n\tinput.facts.is_adult\n}\n";

/// A monthly quota: after five approvals a human looks at the sixth.
pub const EXAMPLE_MONTHLY_QUOTA: &str =
    "package rules\nimport rego.v1\n\nmanual if {\n\tinput.facts.approved_last_30d >= 5\n}\n";

/// The shipped worked examples, keyed by the template ID the web gallery offers
/// them under.
///
/// Pinned here rather than only in the gallery so the validator and the gallery
/// have to change together: a template that needs editing before it saves is not
/// a template, and the test that proves it validates reads this array.
pub const REQUEST_RULE_EXAMPLES: [(&str, &str); 5] = [
    (
        "named-requesters-family-rated",
        EXAMPLE_NAMED_REQUESTERS_FAMILY_RATED,
    ),
    ("short-lease-approval", EXAMPLE_SHORT_LEASE),
    ("low-resolution-approval", EXAMPLE_LOW_RESOLUTION),
    ("deny-adult-content", EXAMPLE_DENY_ADULT_CONTENT),
    ("monthly-approval-quota", EXAMPLE_MONTHLY_QUOTA),
];

// ── Synthetic input ─────────────────────────────────────────────────────────

/// Build a representative input for validation dry-runs: a PG-13 movie request
/// with every fact known, so a rule reaching for any documented fact executes
/// its real path. Nothing is unknown here on purpose — validation is about
/// whether the rule is well-formed, and a rule held for unobservable facts would
/// prove nothing about that.
pub(crate) fn synthetic_request_input() -> RequestInput {
    let evaluation_time = DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
        .expect("fixed synthetic timestamp is in range");

    RequestInput {
        schema_version: REQUEST_INPUT_SCHEMA_VERSION,
        evaluation_time,
        now: RequestClockDoc::at(evaluation_time),
        requester: RequestRequesterDoc {
            user_id: "user-1".to_string(),
            username: "operator".to_string(),
            account_kind: "local".to_string(),
            app_permissions: vec!["manage_users".to_string()],
            library_permissions: vec!["view".to_string(), "request".to_string()],
            linked_providers: vec!["jellyfin".to_string()],
            created_at: Some("2024-01-01T00:00:00Z".to_string()),
        },
        library: RequestLibraryDoc {
            id: "library-1".to_string(),
            name: "Movies".to_string(),
            facet: "movie".to_string(),
            is_default: true,
        },
        request: RequestDoc {
            origin: "manual".to_string(),
            title: "Test Movie".to_string(),
            year: Some(2024),
            external_ids: BTreeMap::from([
                ("tmdb".to_string(), "1".to_string()),
                ("imdb".to_string(), "tt0000001".to_string()),
            ]),
            quality_profile_id: Some("profile-1".to_string()),
            quality_profile_name: Some("HD".to_string()),
            monitor_type: Some("futureepisodes".to_string()),
            monitor_selection_season_count: Some(2),
            lease_forever: false,
            lease_days: Some(14),
        },
        facts: RequestFactsDoc {
            age_rating: Observation::known(13),
            certifications: Observation::known(vec![RequestCertificationDoc {
                country: "US".to_string(),
                value: "PG-13".to_string(),
                source: "tmdb".to_string(),
            }]),
            certification_label: Observation::known("PG-13".to_string()),
            certification_rank: Observation::known(2),
            commonsense_recommended: Observation::known(true),
            genres: Observation::known(vec!["Action".to_string(), "Comedy".to_string()]),
            canonical_tag_keys: Observation::known(vec![
                "canonical:genre:action".to_string(),
                "canonical:theme:heist".to_string(),
            ]),
            themes: Observation::known(vec!["heist".to_string()]),
            is_adult: Observation::known(false),
            rating: Observation::known(7.5),
            ratings_by_source: Observation::known(BTreeMap::from([
                ("imdb".to_string(), 7.4),
                ("tmdb".to_string(), 7.6),
            ])),
            tmdb_vote_average: Observation::known(7.6),
            tmdb_vote_count: Observation::known(1_200),
            popularity: Observation::known(42.0),
            runtime_minutes: Observation::known(120),
            original_language: Observation::known("eng".to_string()),
            country: Observation::known("US".to_string()),
            network: Observation::known("Test Network".to_string()),
            studio: Observation::known("Test Studio".to_string()),
            content_status: Observation::known("released".to_string()),
            release_date: Observation::known("2024-01-01T00:00:00Z".to_string()),
            first_aired: Observation::known("2024-01-01T00:00:00Z".to_string()),
            release_age_days: Observation::known(30),
            award_count: Observation::known(2),
            quality_profile_tiers: Observation::known(vec![
                "1080P".to_string(),
                "720P".to_string(),
            ]),
            quality_profile_max_resolution: Observation::known(1080),
            quality_profile_allows_upgrades: Observation::known(true),
            exists_in_library_ids: Observation::known(vec!["library-2".to_string()]),
            previous_request_count: Observation::known(1),
            previously_denied: Observation::known(false),
            previously_approved: Observation::known(true),
            pending_request_count: Observation::known(1),
            approved_last_30d: Observation::known(2),
            denied_last_30d: Observation::known(0),
            total_approved: Observation::known(9),
            active_lease_count: Observation::known(1),
            days_since_last_request: Observation::known(3),
            library_title_count: Observation::known(120),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::decode::{MAX_REASON_CODE_LEN, MAX_REASON_CODES, MAX_TAGS};
    use crate::runtime;
    use core::num::NonZeroU32;
    use core::time::Duration;

    fn policy(id: &str, body: &str) -> RequestPolicy {
        RequestPolicy {
            id: id.to_string(),
            name: format!("rule {id}"),
            rego_source: rewrite_package_declaration(body, id),
        }
    }

    fn evaluate(policies: &[RequestPolicy]) -> RequestEvalResult {
        evaluate_against(policies, synthetic_request_input())
    }

    fn evaluate_against(policies: &[RequestPolicy], input: RequestInput) -> RequestEvalResult {
        let engine = RequestRulesEngine::build(policies).expect("policies should compile");
        engine
            .evaluator()
            .evaluate(&input)
            .expect("evaluation should succeed")
    }

    fn only_decision(result: &RequestEvalResult) -> &RequestRuleDecision {
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.records.len(), 1, "expected exactly one record");
        &result.records[0].decision
    }

    // ── vote precedence ──

    #[test]
    fn an_approving_rule_votes_approve() {
        let result = evaluate(&[policy(
            "approves",
            "approve if {\n  input.facts.certification_rank <= 2\n}\n",
        )]);
        assert_eq!(only_decision(&result).vote, RequestVote::Approve);
    }

    #[test]
    fn a_denying_rule_votes_deny() {
        let result = evaluate(&[policy(
            "denies",
            "deny if {\n  input.facts.certification_rank >= 2\n}\n",
        )]);
        assert_eq!(only_decision(&result).vote, RequestVote::Deny);
    }

    /// Deny beats approve inside one rule: the rule reached both conclusions,
    /// and the one that does not hand over media is the one that stands.
    #[test]
    fn deny_beats_approve_inside_one_rule() {
        let result = evaluate(&[policy(
            "both",
            "approve := true\n\ndeny if {\n  input.facts.is_adult == false\n}\n",
        )]);
        assert_eq!(only_decision(&result).vote, RequestVote::Deny);
    }

    /// Manual beats both: a rule that says "manual" alongside anything else is a
    /// rule that is not sure, and an unsure rule must neither approve nor deny
    /// on its own.
    #[test]
    fn manual_beats_deny_and_approve_inside_one_rule() {
        let result = evaluate(&[policy(
            "all_three",
            "approve := true\n\ndeny := true\n\nmanual := true\n",
        )]);
        let decision = only_decision(&result);
        assert_eq!(decision.vote, RequestVote::Manual);
        assert!(
            !decision.held,
            "the author declared manual; the host did not hold the rule"
        );
    }

    #[test]
    fn a_rule_that_fires_nothing_abstains() {
        let result = evaluate(&[policy(
            "quiet",
            "approve if {\n  input.library.facet == \"series\"\n}\n",
        )]);
        assert_eq!(only_decision(&result).vote, RequestVote::Abstain);
    }

    // ── tags ──

    #[test]
    fn tags_are_collected_on_every_path_the_rule_ran() {
        for (id, body, expected) in [
            (
                "tagged_approve",
                "approve := true\n\ntags contains \"kids\"\n",
                RequestVote::Approve,
            ),
            (
                "tagged_deny",
                "deny := true\n\ntags contains \"kids\"\n",
                RequestVote::Deny,
            ),
            (
                "tagged_manual",
                "manual := true\n\ntags contains \"kids\"\n",
                RequestVote::Manual,
            ),
            (
                // Abstaining is still running: "this is a kids' film" is true
                // whether or not the rule had an opinion about approving it.
                "tagged_abstain",
                "tags contains \"kids\"\n",
                RequestVote::Abstain,
            ),
        ] {
            let result = evaluate(&[policy(id, body)]);
            let decision = only_decision(&result);
            assert_eq!(decision.vote, expected, "{id}");
            assert_eq!(decision.tags, vec!["kids".to_string()], "{id}");
        }
    }

    #[test]
    fn tags_are_collected_from_an_array_rule_in_order() {
        let result = evaluate(&[policy(
            "array_tags",
            "approve := true\n\ntags := [\"family\", \"kids\"]\n",
        )]);
        assert_eq!(
            only_decision(&result).tags,
            vec!["family".to_string(), "kids".to_string()]
        );
    }

    /// A malformed tag is a per-rule error, not a dropped tag. The rule then
    /// contributes no vote at all, which arbitrates to manual review — the safe
    /// direction. Dropping it silently would change what the created title looks
    /// like with nothing in the trace to say so.
    #[test]
    fn a_malformed_tag_turns_the_whole_rule_into_an_error() {
        let result = evaluate(&[policy(
            "bad_tag",
            "approve := true\n\ntags contains \"kids/teens\"\n",
        )]);
        assert!(result.records.is_empty(), "{:?}", result.records);
        assert_eq!(result.errors.len(), 1);
        assert!(
            result.errors[0].message.contains("unsupported character"),
            "{}",
            result.errors[0].message
        );
    }

    #[test]
    fn a_reserved_tag_prefix_turns_the_whole_rule_into_an_error() {
        let result = evaluate(&[policy(
            "reserved_tag",
            "approve := true\n\ntags contains \"scryer:managed\"\n",
        )]);
        assert!(result.records.is_empty(), "{:?}", result.records);
        assert!(
            result.errors[0].message.contains("reserved"),
            "{}",
            result.errors[0].message
        );
    }

    #[test]
    fn oversized_tag_list_is_rejected() {
        let entries = (0..=MAX_TAGS)
            .map(|i| format!("\"tag{i}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let result = evaluate(&[policy(
            "too_many_tags",
            &format!("approve := true\n\ntags := [{entries}]\n"),
        )]);
        assert!(result.records.is_empty(), "{:?}", result.records);
        assert!(
            result.errors[0].message.contains("at most 16"),
            "{}",
            result.errors[0].message
        );
    }

    // ── reasons ──

    #[test]
    fn reason_codes_are_collected_from_a_set_rule() {
        let result = evaluate(&[policy(
            "reasoned_deny",
            "deny := true\n\nreasons contains \"adult_content\"\n\nreasons contains \"quota\"\n",
        )]);
        assert_eq!(
            only_decision(&result).reason_codes,
            vec!["adult_content".to_string(), "quota".to_string()]
        );
    }

    #[test]
    fn oversized_reason_list_is_rejected() {
        let entries = (0..=MAX_REASON_CODES)
            .map(|i| format!("\"reason_{i}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let result = evaluate(&[policy(
            "too_many_reasons",
            &format!("deny := true\n\nreasons := [{entries}]\n"),
        )]);
        assert!(result.records.is_empty(), "{:?}", result.records);
        assert!(
            result.errors[0].message.contains("at most 32"),
            "{}",
            result.errors[0].message
        );
    }

    #[test]
    fn overlong_reason_code_is_rejected() {
        let long = "x".repeat(MAX_REASON_CODE_LEN + 1);
        let result = evaluate(&[policy(
            "long_reason",
            &format!("deny := true\n\nreasons := [\"{long}\"]\n"),
        )]);
        assert!(result.records.is_empty(), "{:?}", result.records);
        assert!(
            result.errors[0].message.contains("at most 120"),
            "{}",
            result.errors[0].message
        );
    }

    // ── host-derived holds ──

    /// The core of the family: no `manual if` is written anywhere, and the rule
    /// is still held to a human because a fact it reads is one Scryer could not
    /// observe. On the bare fact surface an unknown certification is a missing
    /// key, which `<= 2` would read as a decisive "not rated above PG-13".
    #[test]
    fn a_rule_reading_an_unobservable_fact_is_held_without_declaring_manual() {
        let mut input = synthetic_request_input();
        input.facts.certification_rank = Observation::unknown("no_us_certification");

        let result = evaluate_against(
            &[policy(
                "family_rated",
                "approve if {\n  input.facts.certification_rank <= 2\n}\n\ntags contains \"family\"\n",
            )],
            input,
        );

        let decision = only_decision(&result);
        assert_eq!(decision.vote, RequestVote::Manual);
        assert!(decision.held, "the host held this rule, not the author");
        assert_eq!(
            decision.reason_codes,
            vec!["no_us_certification".to_string()]
        );
        assert!(
            decision.tags.is_empty(),
            "a held rule never ran, so it never claimed a tag"
        );
    }

    #[test]
    fn held_rules_report_the_union_of_the_reasons_that_held_them() {
        let mut input = synthetic_request_input();
        input.facts.certification_rank = Observation::unknown("metadata_unavailable");
        input.facts.is_adult = Observation::unknown("metadata_unavailable");
        input.facts.approved_last_30d = Observation::unknown("history_unreadable");

        let result = evaluate_against(
            &[policy(
                "many_unknowns",
                "approve if {\n  input.facts.certification_rank <= 2\n  \
                 not input.facts.is_adult\n  input.facts.approved_last_30d < 5\n}\n",
            )],
            input,
        );

        assert_eq!(
            only_decision(&result).reason_codes,
            vec![
                "history_unreadable".to_string(),
                "metadata_unavailable".to_string()
            ],
            "one code per distinct reason, in fact-name order"
        );
    }

    /// Absence is an answer. A title with no US certification at all is a real
    /// answer to "what is its rank", and a rule keying on the missing key must
    /// decide rather than hold.
    #[test]
    fn a_rule_matching_an_absent_fact_decides_rather_than_holding() {
        let mut input = synthetic_request_input();
        input.facts.certification_label = Observation::absent_because("no_us_certification");

        let result = evaluate_against(
            &[policy(
                "unrated",
                "manual if {\n  not input.facts.certification_label\n}\n",
            )],
            input,
        );

        let decision = only_decision(&result);
        assert_eq!(decision.vote, RequestVote::Manual);
        assert!(!decision.held, "an absence is an answer, not a hold");
    }

    /// The opt-out has to actually opt out, or the advanced surface is
    /// unreachable: a rule reading the envelope sees the unknown itself and
    /// decides for itself what it means.
    #[test]
    fn observation_references_do_not_trigger_host_derived_unknownness() {
        let mut input = synthetic_request_input();
        input.facts.certification_rank = Observation::unknown("no_us_certification");

        let result = evaluate_against(
            &[policy(
                "inspects_the_envelope",
                "deny if {\n  input.observations.certification_rank.status == \"unknown\"\n}\n\n\
                 reasons contains reason if {\n  \
                   reason := input.observations.certification_rank.reason\n\
                 }\n",
            )],
            input,
        );

        let decision = only_decision(&result);
        assert_eq!(decision.vote, RequestVote::Deny);
        assert!(!decision.held);
        assert_eq!(
            decision.reason_codes,
            vec!["no_us_certification".to_string()]
        );
    }

    // ── malformed heads ──

    #[test]
    fn a_non_boolean_head_is_an_error_naming_that_head() {
        for (id, body, expected) in [
            (
                "numeric_approve",
                "approve := 1\n",
                "'approve' must be a boolean",
            ),
            (
                "string_deny",
                "deny := \"yes\"\n",
                "'deny' must be a boolean",
            ),
            (
                "numeric_manual",
                "manual := 0\n",
                "'manual' must be a boolean",
            ),
        ] {
            let result = evaluate(&[policy(id, body)]);
            assert!(result.records.is_empty(), "{id}: {:?}", result.records);
            assert_eq!(result.errors.len(), 1, "{id}");
            assert!(
                result.errors[0].message.contains(expected),
                "{id}: {}",
                result.errors[0].message
            );
        }
    }

    #[test]
    fn a_broken_rule_does_not_stop_the_batch() {
        let result = evaluate(&[
            policy(
                "broken",
                "approve if {\n  lower(input.request.year) == \"x\"\n}\n",
            ),
            policy("healthy", "approve := true\n"),
        ]);

        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].rule_set_id, "broken");
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].rule_set_id, "healthy");
        assert_eq!(result.records[0].decision.vote, RequestVote::Approve);
    }

    // ── budget, bounds, hashing ──

    #[test]
    fn execution_budget_turns_a_runaway_rule_into_an_error() {
        let mut limits = RuntimeLimits::request_defaults();
        limits.max_execution_time = Duration::from_millis(1);
        limits.timer_check_interval = NonZeroU32::new(1).expect("non-zero");

        let policies = [policy(
            "runaway",
            "approve if {\n  count([1 |\n    some i in numbers.range(1, 3000)\n    some j in numbers.range(1, 3000)\n    i == j\n  ]) > 0\n}\n",
        )];
        let engine =
            RequestRulesEngine::build_with_limits(&policies, limits).expect("should compile");
        let result = engine
            .evaluator()
            .evaluate(&synthetic_request_input())
            .expect("evaluation should return, not hang");

        assert!(result.records.is_empty(), "{:?}", result.records);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].rule_set_id, "runaway");
    }

    #[test]
    fn oversized_input_is_rejected_before_evaluation() {
        let mut limits = RuntimeLimits::request_defaults();
        limits.max_input_bytes = 16;
        let engine =
            RequestRulesEngine::build_with_limits(&[policy("tiny", "approve := true\n")], limits)
                .expect("should compile");
        let err = engine
            .evaluator()
            .evaluate(&synthetic_request_input())
            .expect_err("input should exceed the bound");
        assert!(matches!(err, RulesError::InputTooLarge { .. }), "{err:?}");
    }

    #[test]
    fn records_carry_a_stable_policy_content_hash() {
        let policies = [policy("hashed", "approve := true\n")];
        let first = evaluate(&policies);
        let second = evaluate(&policies);

        let hash = &first.records[0].policy_content_hash;
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, &second.records[0].policy_content_hash);
        assert_eq!(
            hash,
            &runtime::content_hash(&policies[0].rego_source),
            "record hash must be the hash of the stored source"
        );
    }

    #[test]
    fn empty_engine_evaluates_to_nothing() {
        let engine = RequestRulesEngine::empty();
        assert!(engine.is_empty());
        assert_eq!(engine.rule_count(), 0);
        let result = engine
            .evaluator()
            .evaluate(&synthetic_request_input())
            .expect("evaluation should succeed");
        assert!(result.records.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn the_request_limits_are_the_submit_path_budget() {
        let limits = RuntimeLimits::request_defaults();
        assert_eq!(limits.max_execution_time, Duration::from_millis(100));
        assert_eq!(limits.max_input_bytes, 1024 * 1024);
        assert_eq!(limits.max_policy_bytes.get(), 256 * 1024);
        assert_eq!(limits.max_policy_lines.get(), 5_000);
    }

    // ── input document ──

    #[test]
    fn the_input_document_carries_bare_facts_and_full_observations() {
        let mut input = synthetic_request_input();
        input.facts.certification_rank = Observation::unknown("no_us_certification");
        input.facts.network = Observation::absent_because("movies_have_no_network");

        let doc = serde_json::to_value(&input).expect("input serializes");

        assert_eq!(doc["schema_version"], 1);
        assert_eq!(doc["evaluation_time"], "2023-11-14T22:13:20Z");
        assert_eq!(doc["now"]["weekday"], "tuesday");
        assert_eq!(doc["now"]["hour_utc"], 22);
        assert_eq!(doc["requester"]["username"], "operator");
        assert_eq!(doc["library"]["facet"], "movie");
        assert_eq!(doc["request"]["lease_days"], 14);
        assert_eq!(doc["request"]["external_ids"]["tmdb"], "1");

        // Known: bare value on facts, envelope on observations.
        assert_eq!(doc["facts"]["is_adult"], false);
        assert_eq!(doc["observations"]["is_adult"]["status"], "known");
        assert_eq!(doc["facts"]["certifications"][0]["value"], "PG-13");

        // Unknown and absent: missing from facts entirely, still on
        // observations with the reason that explains them.
        for fact in ["certification_rank", "network"] {
            assert!(doc["facts"].get(fact).is_none(), "{fact}");
            assert!(doc["observations"][fact]["reason"].is_string(), "{fact}");
        }
        assert_eq!(
            doc["observations"]["certification_rank"]["status"],
            "unknown"
        );
        assert_eq!(doc["observations"]["network"]["status"], "absent");

        // The two namespaces describe the same facts, so neither can gain a key
        // the other never heard of.
        for fact in doc["facts"].as_object().expect("facts object").keys() {
            assert!(doc["observations"].get(fact).is_some(), "{fact}");
        }
    }

    /// `evaluation_time` is the clock rules are meant to use, and date maths on
    /// it has to work.
    #[test]
    fn evaluation_time_supports_date_arithmetic() {
        let result = evaluate(&[policy(
            "recent_release",
            "day_ns := (24 * 60 * 60) * 1000000000\n\n\
             approve if {\n  \
               age := time.parse_rfc3339_ns(input.evaluation_time) - \
             time.parse_rfc3339_ns(input.facts.release_date)\n  \
               age < 0\n\
             }\n",
        )]);
        // Synthetic input is evaluated at 2023-11-14 for a 2024 release, so the
        // age is negative and the rule fires — the point is that the maths ran
        // rather than evaluating to undefined.
        assert_eq!(only_decision(&result).vote, RequestVote::Approve);
    }

    #[test]
    fn the_clock_document_agrees_with_the_evaluation_time() {
        let input = synthetic_request_input();
        assert_eq!(input.now.weekday, "tuesday");
        assert_eq!(input.now.hour_utc, 22);

        // Every weekday maps to a distinct lowercase name.
        let names: BTreeSet<String> = (0..7)
            .map(|day| {
                RequestClockDoc::at(
                    DateTime::<Utc>::from_timestamp(1_700_000_000 + day * 86_400, 0)
                        .expect("in range"),
                )
                .weekday
            })
            .collect();
        assert_eq!(names.len(), 7, "{names:?}");
    }

    // ── wrapper text ──

    /// The wrapper is what every stored rule is evaluated through, so its text
    /// is a contract with the rules in people's databases from the moment this
    /// ships.
    #[test]
    fn the_generated_wrapper_projects_five_heads_and_the_family_hold_head() {
        assert_eq!(
            decision_wrapper_source("rule_1"),
            "package scryer.request.wrapper.rule_1\n\
             import rego.v1\n\n\
             decision := {\n\
             \t\"approve\": object.get(data.scryer.request.user.rule_1, \"approve\", false),\n\
             \t\"deny\": object.get(data.scryer.request.user.rule_1, \"deny\", false),\n\
             \t\"manual\": object.get(data.scryer.request.user.rule_1, \"manual\", false),\n\
             \t\"reasons\": object.get(data.scryer.request.user.rule_1, \"reasons\", []),\n\
             \t\"tags\": object.get(data.scryer.request.user.rule_1, \"tags\", []),\n\
             }\n"
        );
        assert_eq!(
            decision_wrapper_rule_path("rule_1"),
            "data.scryer.request.wrapper.rule_1.decision"
        );
        assert_eq!(user_policy_path("rule_1"), "request/rule_1.rego");
        assert_eq!(
            decision_wrapper_policy_path("rule_1"),
            "internal/rule_1_request_wrapper.rego"
        );

        // Maintenance says `unknown if`; requests say `manual if`. The head the
        // wrapper reads is exactly what the family declares.
        let hold = RequestFamily::hold_rule_name().expect("requests hold");
        assert_eq!(hold, "manual");
        assert!(
            decision_wrapper_source("rule_1").contains(&format!("\"{hold}\", false)")),
            "the wrapper must read the head the family declares"
        );
    }

    #[test]
    fn the_package_prefixes_are_the_documented_ones() {
        assert_eq!(RequestFamily::USER_PACKAGE_PREFIX, "scryer.request.user");
        assert_eq!(
            RequestFamily::WRAPPER_PACKAGE_PREFIX,
            "scryer.request.wrapper"
        );
        assert!(
            rewrite_package_declaration("approve := true\n", "abc")
                .starts_with("package scryer.request.user.abc\nimport rego.v1\n")
        );
    }

    // ── contract helpers ──

    #[test]
    fn the_certification_ladder_covers_every_documented_label() {
        for (label, rank) in [
            ("G", 0),
            ("TV-Y", 0),
            ("TV-Y7", 0),
            ("TV-G", 0),
            ("PG", 1),
            ("TV-PG", 1),
            ("PG-13", 2),
            ("TV-14", 2),
            ("R", 3),
            ("NC-17", 4),
            ("TV-MA", 4),
        ] {
            assert_eq!(certification_rank_for_label(label), Some(rank), "{label}");
            assert_eq!(
                certification_rank_for_label(&label.to_ascii_lowercase()),
                Some(rank),
                "{label} lowercased"
            );
            assert_eq!(
                certification_rank_for_label(&format!("  {label} ")),
                Some(rank),
                "{label} padded"
            );
        }
    }

    /// A label the ladder does not know produces no rank at all. That is what
    /// leaves `certification_rank` unknown for a title with only a foreign
    /// certification, which holds every rule that reads it.
    #[test]
    fn an_unknown_certification_label_has_no_rank() {
        for label in ["", "  ", "12A", "FSK 16", "UNRATED", "NR", "TV-Y77", "PG13"] {
            assert_eq!(certification_rank_for_label(label), None, "{label}");
        }
    }

    #[test]
    fn quality_tiers_report_their_highest_resolution() {
        assert_eq!(
            max_resolution_for_quality_tiers(&[
                "720P".to_string(),
                "1080P".to_string(),
                "480P".to_string()
            ]),
            Some(1080)
        );
        assert_eq!(
            max_resolution_for_quality_tiers(&["2160P".to_string()]),
            Some(2160)
        );
        assert_eq!(
            max_resolution_for_quality_tiers(&["4K".to_string()]),
            Some(2160)
        );
        assert_eq!(
            max_resolution_for_quality_tiers(&["4k".to_string(), "1080p".to_string()]),
            Some(2160),
            "case and mixed vocabularies still compare"
        );
        assert_eq!(
            max_resolution_for_quality_tiers(&[" 720P ".to_string()]),
            Some(720)
        );
    }

    /// A tier list that names no resolution leaves the fact unknown rather than
    /// claiming the profile allows nothing.
    #[test]
    fn quality_tiers_that_name_no_resolution_report_nothing() {
        assert_eq!(max_resolution_for_quality_tiers(&[]), None);
        assert_eq!(
            max_resolution_for_quality_tiers(&[
                "SDTV".to_string(),
                "RAW-HD".to_string(),
                "BLURAY".to_string(),
                "".to_string()
            ]),
            None
        );
        assert_eq!(
            max_resolution_for_quality_tiers(&["SDTV".to_string(), "1080P".to_string()]),
            Some(1080),
            "an unparseable tier is skipped, not fatal"
        );
    }

    // ── worked examples ──

    #[test]
    fn the_worked_examples_vote_the_way_the_plan_says() {
        // 1. `operator` is not one of the named requesters, so no approval; the
        //    tag rule is unconditional on rank and PG-13 is rank 2, so no tag.
        let alice = evaluate(&[policy("example_1", EXAMPLE_NAMED_REQUESTERS_FAMILY_RATED)]);
        assert_eq!(only_decision(&alice).vote, RequestVote::Abstain);

        let mut named = synthetic_request_input();
        named.requester.username = "alice".to_string();
        let approved = evaluate_against(
            &[policy("example_1", EXAMPLE_NAMED_REQUESTERS_FAMILY_RATED)],
            named.clone(),
        );
        let decision = only_decision(&approved);
        assert_eq!(decision.vote, RequestVote::Approve);
        assert!(
            decision.tags.is_empty(),
            "PG-13 is rank 2, not a family tag"
        );

        let mut gentle = named.clone();
        gentle.facts.certification_rank = Observation::known(1);
        let tagged = evaluate_against(
            &[policy("example_1", EXAMPLE_NAMED_REQUESTERS_FAMILY_RATED)],
            gentle,
        );
        assert_eq!(only_decision(&tagged).tags, vec!["family".to_string()]);

        // 2. Bob's short lease. A forever lease must not match, even though it
        //    carries no days to compare.
        let mut bob = synthetic_request_input();
        bob.requester.username = "bob".to_string();
        assert_eq!(
            only_decision(&evaluate_against(
                &[policy("example_2", EXAMPLE_SHORT_LEASE)],
                bob.clone()
            ))
            .vote,
            RequestVote::Approve
        );

        let mut forever = bob.clone();
        forever.request.lease_forever = true;
        forever.request.lease_days = None;
        assert_eq!(
            only_decision(&evaluate_against(
                &[policy("example_2", EXAMPLE_SHORT_LEASE)],
                forever
            ))
            .vote,
            RequestVote::Abstain,
            "a forever lease must not satisfy a finite-lease rule"
        );

        // 3. Alice at 720p or lower. The synthetic profile tops out at 1080.
        assert_eq!(
            only_decision(&evaluate_against(
                &[policy("example_3", EXAMPLE_LOW_RESOLUTION)],
                named.clone()
            ))
            .vote,
            RequestVote::Abstain
        );
        let mut sd = named;
        sd.facts.quality_profile_max_resolution = Observation::known(720);
        assert_eq!(
            only_decision(&evaluate_against(
                &[policy("example_3", EXAMPLE_LOW_RESOLUTION)],
                sd
            ))
            .vote,
            RequestVote::Approve
        );

        // 4. Adult content is denied with a reason the requester sees.
        let mut adult = synthetic_request_input();
        adult.facts.is_adult = Observation::known(true);
        let denied = evaluate_against(&[policy("example_4", EXAMPLE_DENY_ADULT_CONTENT)], adult);
        let decision = only_decision(&denied);
        assert_eq!(decision.vote, RequestVote::Deny);
        assert_eq!(decision.reason_codes, vec!["adult_content".to_string()]);

        // 5. The monthly quota sends the sixth approval to a human.
        let mut quota = synthetic_request_input();
        quota.facts.approved_last_30d = Observation::known(5);
        let held = evaluate_against(&[policy("example_5", EXAMPLE_MONTHLY_QUOTA)], quota);
        let decision = only_decision(&held);
        assert_eq!(decision.vote, RequestVote::Manual);
        assert!(!decision.held, "the author declared manual");
    }

    #[test]
    fn every_worked_example_loads_into_an_engine() {
        for (index, (template_id, source)) in REQUEST_RULE_EXAMPLES.into_iter().enumerate() {
            let rule_id = format!("request_template_{index}");
            RequestRulesEngine::build(&[policy(&rule_id, source)])
                .unwrap_or_else(|error| panic!("{template_id} should load: {error}"));
        }
    }
}
