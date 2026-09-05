//! Maintenance policy family: match existing library media against user rules.
//!
//! The matcher answers one question — does this subject match this rule right
//! now — and returns match / no-match / unknown. It never selects, names, or
//! performs an action: the action comes from the validated rule revision that
//! owns the matcher, so a malformed or timed-out evaluation can never authorize
//! one.
//!
//! Media-server play history is in the schema now that a provider-neutral
//! signal store stands behind it, and it is fact-shaped like everything else:
//! `watched_by_user_ids`, `last_watched_at`, and the two requester rollups are
//! plain values that go missing when the host could not observe them. Watchlist
//! and favorite signals are still absent — nothing collects them yet, and a
//! fact rules cannot be told the truth about has no business in the contract.
//!
//! Two things make a rule safe to write. The rule reads facts as plain values,
//! so it says what it means (`input.facts.monitored`, not a status dance), and
//! the *host* — not the author — decides that a rule reading a fact Scryer
//! could not observe for this subject must be held. The set of facts a rule
//! reads is extracted from its source at build time, which is why fact names
//! have to be literal and why `input` cannot be imported.

use crate::RulesError;
use crate::policy::decode::decode_reasons;
use crate::policy::engine::{EvalOutcome, EvalRecord, PolicyEngine, PolicyEvaluator, RuleHandle};
use crate::policy::observation::serialize_fact_namespaces;
use crate::policy::wrapper::{WrapperField, object_wrapper_source};
use crate::policy::{PolicyFamily, PolicyRecord};
use crate::runtime::RuntimeLimits;
use crate::validation;
use chrono::{DateTime, Utc};
use regorus::Value;
use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};
use std::collections::BTreeSet;

#[cfg(test)]
use crate::policy::decode::{MAX_REASON_CODE_LEN, MAX_REASON_CODES};
#[cfg(test)]
use crate::runtime;

/// Three-valued availability envelope wrapping every maintenance fact. Owned by
/// the shared policy core; re-exported here because maintenance facts are what
/// it was written for and what most callers reach for it through.
pub use crate::policy::observation::Observation;

// ── Contract constants ──────────────────────────────────────────────────────

/// Version of the maintenance input document. Rules are authored against a
/// specific version; bumping it is a breaking change to every stored matcher.
///
/// v2 split the fact snapshot in two: `input.facts.<name>` is the bare value
/// and is simply missing when the fact is absent or unknown, while
/// `input.observations.<name>` keeps the full three-valued envelope for rules
/// that need to tell those two apart.
pub const MAINTENANCE_INPUT_SCHEMA_VERSION: u32 = 2;

/// Package prefix for user-authored maintenance matchers.
pub(crate) const USER_PACKAGE_PREFIX: &str = "scryer.maintenance.user";

/// Package prefix for the generated evaluation wrapper. The wrapper lives in a
/// separate package because it reads the whole user package document; putting
/// it inside that package would make the module self-referential.
const WRAPPER_PACKAGE_PREFIX: &str = "scryer.maintenance.wrapper";

/// The head an author writes to hold a subject themselves. Maintenance says
/// `unknown if { … }`; the name is a per-family parameter of the policy core,
/// and the request family says `manual if { … }` instead.
const HOLD_RULE_NAME: &str = "unknown";

// ── Input document ──────────────────────────────────────────────────────────

/// Input document set once per subject for maintenance rule evaluation.
///
/// `evaluation_time` is supplied by the host and captured once per run so every
/// subject in that run compares against the same instant. Policies must never
/// reach for a clock of their own — `time.now_ns()` exists, but it would make
/// the same subject decide differently on a retry.
///
/// The fact snapshot is held once, as envelopes, and serializes into two
/// namespaces: `observations` is the envelope map verbatim, and `facts` is
/// derived from it by unwrapping the known values and dropping everything else.
/// Deriving rather than storing both is the point — the simple surface and the
/// advanced one cannot drift apart.
#[derive(Debug, Clone)]
pub struct MaintenanceInput {
    pub schema_version: u32,
    pub evaluation_time: DateTime<Utc>,
    pub subject: MaintenanceSubjectDoc,
    pub library: MaintenanceLibraryDoc,
    pub facts: MaintenanceFactsDoc,
}

impl Serialize for MaintenanceInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let namespaces =
            serialize_fact_namespaces(&self.facts).map_err(serde::ser::Error::custom)?;
        let mut state = serializer.serialize_struct("MaintenanceInput", 6)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("evaluation_time", &self.evaluation_time)?;
        state.serialize_field("subject", &self.subject)?;
        state.serialize_field("library", &self.library)?;
        state.serialize_field("facts", &namespaces.facts)?;
        state.serialize_field("observations", &namespaces.observations)?;
        state.end()
    }
}

/// Granularity the rule set is scoped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceSubjectKind {
    Title,
    Season,
    Episode,
}

/// Identity of the subject under evaluation. `season_number` and `episode_id`
/// are populated only for the matching subject kind.
#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceSubjectDoc {
    pub kind: MaintenanceSubjectKind,
    pub title_id: String,
    pub season_number: Option<i32>,
    pub episode_id: Option<String>,
    pub facet: String,
    pub name: String,
    pub year: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceLibraryDoc {
    pub id: String,
    pub name: String,
}

/// The fact snapshot, and the single source of truth for both input
/// namespaces. Every field is an observation envelope: a fact Scryer failed to
/// resolve stays unknown rather than defaulting to a value the rule would read
/// as decisive.
#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceFactsDoc {
    pub monitored: Observation<bool>,
    pub tags: Observation<Vec<String>>,
    pub quality_profile_id: Observation<String>,
    /// RFC3339 timestamps.
    pub added_at: Observation<String>,
    /// Provenance: who put the title in the library, and whether a media
    /// request is what created it. These are facts, not a classification —
    /// there is deliberately no derived `origin` field, because "managed",
    /// "requested", and "scan-discovered" are conclusions a rule composes for
    /// itself out of these five observations.
    pub added_by_user_id: Observation<String>,
    pub added_by_username: Observation<String>,
    pub requested: Observation<bool>,
    pub requested_by_user_ids: Observation<Vec<String>>,
    pub requested_by_usernames: Observation<Vec<String>>,
    pub first_imported_at: Observation<String>,
    pub last_upgraded_at: Observation<String>,
    pub has_file: Observation<bool>,
    pub file_count: Observation<i64>,
    pub total_file_size_bytes: Observation<i64>,
    pub files: Observation<Vec<MaintenanceFileDoc>>,
    pub episode_count: Observation<i64>,
    pub episode_file_count: Observation<i64>,
    pub monitored_episode_count: Observation<i64>,
    pub active_downloads: Observation<bool>,
    /// Media-server watch signals (RFC 137 section 7.3). Every one of these is
    /// unknown unless a signal-sync provider is connected *and* every enabled
    /// connection of it swept cleanly and recently: a partial watch picture
    /// would make "nobody watched this" a lie, and these are the facts a rule
    /// deletes media on.
    pub watched_by_user_ids: Observation<Vec<String>>,
    pub last_watched_at: Observation<String>,
    pub watched_by_any_requester: Observation<bool>,
    pub watched_by_all_requesters: Observation<bool>,
    /// Lifecycle claims (spec 0003 FR-041..FR-044). A claim is what a request
    /// approval leaves behind on the title, and these four say whether one
    /// still holds it. Every one of them is unknown when the claim store could
    /// not be read for the pass: a rule that deletes on "the lease expired"
    /// must be held rather than told there is no lease.
    ///
    /// `request_lease_state` is one of `none`, `dormant`, `active`, `expired`,
    /// aggregated over the title's retention claims; expiry is derived against
    /// the evaluation clock, so a lease whose window elapsed reads as expired
    /// before the sweep that flips the row has run.
    pub keep_claim_active: Observation<bool>,
    pub request_lease_state: Observation<String>,
    pub request_lease_expires_at: Observation<String>,
    pub active_retention_claims: Observation<i64>,
}

/// Facts that name, or are computed from, identifiable people.
///
/// Authoring a matcher that reads one of these is a way of asking the instance
/// who added, requested, or watched something, so the host requires the author
/// to already hold the instance's permission-management authority. The list
/// lives here, beside the fact document, because it is a property of the facts
/// themselves rather than of any one caller.
///
/// `requested` and `last_watched_at` are deliberately absent: they are
/// anonymous aggregates that say something happened, never who did it.
pub const PERSON_TARGETED_MAINTENANCE_FACTS: [&str; 7] = [
    "added_by_user_id",
    "added_by_username",
    "requested_by_user_ids",
    "requested_by_usernames",
    "watched_by_user_ids",
    "watched_by_any_requester",
    "watched_by_all_requesters",
];

#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceFileDoc {
    pub size_bytes: Option<i64>,
    pub quality: Option<String>,
    pub video_codec: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub audio_languages: Vec<String>,
    pub subtitle_languages: Vec<String>,
    pub added_at: Option<String>,
}

// ── Output contract ─────────────────────────────────────────────────────────

/// Closed output of one maintenance matcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceOutcome {
    Match,
    NoMatch,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceDecision {
    pub outcome: MaintenanceOutcome,
    pub reason_codes: Vec<String>,
}

/// One rule's decision, attributed to the exact policy revision that produced
/// it via `policy_content_hash`.
pub type MaintenanceEvalRecord = EvalRecord<MaintenanceDecision>;

/// A per-rule failure. A rule that errors produces no record at all, so it can
/// never advance a candidate.
#[derive(Debug, Clone)]
pub struct MaintenanceEvalError {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub message: String,
}

pub type MaintenanceEvalResult = EvalOutcome<MaintenanceDecision, MaintenanceEvalError>;

// ── Package rewriting and wrapper generation ────────────────────────────────

/// Rewrite (or insert) the package declaration so stored maintenance source
/// always carries the maintenance prefix plus the system-assigned rule ID.
pub fn rewrite_package_declaration(rego_source: &str, rule_id: &str) -> String {
    crate::runtime::rewrite_package_declaration_with_prefix(
        rego_source,
        USER_PACKAGE_PREFIX,
        rule_id,
    )
}

/// Source for the generated evaluation entry point.
///
/// `object.get` with a default is what makes `unknown` and `reasons` optional
/// and an undefined `match` mean "did not match": a rule whose body fails
/// leaves the name off the package document entirely, and the default fills in.
/// A `match` that is defined but not boolean survives to the host, which
/// rejects it rather than coercing it.
pub(crate) fn decision_wrapper_source(rule_id: &str) -> String {
    object_wrapper_source(
        WRAPPER_PACKAGE_PREFIX,
        USER_PACKAGE_PREFIX,
        rule_id,
        "decision",
        &[
            WrapperField::new("matched", "match", "false"),
            WrapperField::new("unknown", HOLD_RULE_NAME, "false"),
            WrapperField::new("reasons", "reasons", "[]"),
        ],
    )
}

pub(crate) fn decision_wrapper_rule_path(rule_id: &str) -> String {
    format!("data.{WRAPPER_PACKAGE_PREFIX}.{rule_id}.decision")
}

pub(crate) fn decision_wrapper_policy_path(rule_id: &str) -> String {
    format!("internal/{rule_id}_maintenance_wrapper.rego")
}

pub(crate) fn user_policy_path(rule_id: &str) -> String {
    format!("maintenance/{rule_id}.rego")
}

// ── Engine ──────────────────────────────────────────────────────────────────

/// A maintenance matcher loaded from the database.
#[derive(Debug, Clone)]
pub struct MaintenancePolicy {
    pub id: String,
    pub name: String,
    pub rego_source: String,
}

/// The maintenance policy family: what the shared core needs to know to run
/// maintenance matchers, and nothing else.
pub struct MaintenanceFamily;

impl PolicyRecord for MaintenancePolicy {
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

impl PolicyFamily for MaintenanceFamily {
    const NAME: &'static str = "maintenance";
    const USER_PACKAGE_PREFIX: &'static str = USER_PACKAGE_PREFIX;
    const WRAPPER_PACKAGE_PREFIX: &'static str = WRAPPER_PACKAGE_PREFIX;
    /// Host-derived unknownness is the whole point of the maintenance family:
    /// a matcher that reads a fact Scryer could not observe is held before it
    /// is consulted.
    const TRACKS_REFERENCED_FACTS: bool = true;

    type Policy = MaintenancePolicy;
    type Input = MaintenanceInput;
    type Decision = MaintenanceDecision;
    type RuleExtra = ();
    type EvalContext = ();
    type EvalError = MaintenanceEvalError;

    fn limits() -> RuntimeLimits {
        RuntimeLimits::maintenance_defaults()
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

    /// A matcher whose fact dependencies cannot be read off its source must not
    /// load at all: the host could not then tell whether it is deciding on
    /// evidence Scryer actually has.
    fn referenced_facts(
        policy: &Self::Policy,
        policy_path: &str,
    ) -> Result<BTreeSet<String>, String> {
        validation::maintenance_fact_references(&policy.rego_source, policy_path)
    }

    fn hold_rule_name() -> Option<&'static str> {
        Some(HOLD_RULE_NAME)
    }

    fn held_decision(reason_codes: Vec<String>) -> Self::Decision {
        MaintenanceDecision {
            outcome: MaintenanceOutcome::Unknown,
            reason_codes,
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
        MaintenanceEvalError {
            rule_set_id: rule.id.clone(),
            rule_set_name: rule.name.clone(),
            message,
        }
    }
}

/// Pre-compiled engine holding every active maintenance matcher.
///
/// Built once per rule-set revision and shared; evaluators are cheap clones
/// created per evaluation run.
pub type MaintenanceRulesEngine = PolicyEngine<MaintenanceFamily>;

/// Evaluates every loaded matcher against one subject at a time.
pub type MaintenanceRulesEvaluator = PolicyEvaluator<MaintenanceFamily>;

impl MaintenanceRulesEvaluator {
    /// Evaluate every loaded matcher against one subject.
    ///
    /// Per-rule failures — compilation-clean rules that error at runtime,
    /// exceed the execution budget, or return a malformed decision — are
    /// collected and never abort the batch. A failing rule contributes no
    /// record, so it cannot advance or cancel a candidate.
    ///
    /// A rule that reads a fact Scryer could not observe for this subject is
    /// held before it is consulted at all. On the simple surface an unknown
    /// fact is simply a missing key, which a policy would otherwise read as a
    /// decisive "no" — so the host, not the author, is what makes an
    /// unobservable fact fail closed. Rules that opt out by reading
    /// `input.observations.*` are consulted normally and may still declare
    /// their own `unknown`, which composes with this one: either is enough to
    /// hold the subject.
    ///
    /// A held rule is not evaluated, so its reason codes are the observations'
    /// own — the operator sees *which fact* Scryer could not read, which is the
    /// actionable half. A policy's `reasons` still apply on every path where
    /// the policy is consulted at all, including its own `unknown`.
    pub fn evaluate(
        &mut self,
        input: &MaintenanceInput,
    ) -> Result<MaintenanceEvalResult, RulesError> {
        self.evaluate_policies(input, &())
    }
}

// ── Output validation ───────────────────────────────────────────────────────

/// Convert the wrapper's decision object into the closed output contract.
///
/// Fails closed: anything that is not exactly the declared shape becomes an
/// error for that rule instead of a coerced decision.
pub(crate) fn decode_decision(value: &Value) -> Result<MaintenanceDecision, String> {
    if matches!(value, Value::Undefined) {
        return Err("decision rule produced no value".to_string());
    }
    if value.as_object().is_err() {
        return Err("decision must be an object".to_string());
    }

    let matched = *value["matched"]
        .as_bool()
        .map_err(|_| "'match' must be a boolean".to_string())?;
    let unknown = *value["unknown"]
        .as_bool()
        .map_err(|_| "'unknown' must be a boolean".to_string())?;
    let reason_codes = decode_reasons(&value["reasons"])?;

    // Unknown wins over match: a rule that cannot see enough to decide must
    // hold the candidate rather than advance it.
    let outcome = if unknown {
        MaintenanceOutcome::Unknown
    } else if matched {
        MaintenanceOutcome::Match
    } else {
        MaintenanceOutcome::NoMatch
    };

    Ok(MaintenanceDecision {
        outcome,
        reason_codes,
    })
}

// ── Synthetic input ─────────────────────────────────────────────────────────

/// Build a representative input for validation dry-runs: a Title subject with
/// every fact known, so a rule reaching for any documented fact executes its
/// real path. Nothing is unknown here on purpose — validation is about whether
/// the rule is well-formed, and a rule held for unobservable facts would prove
/// nothing about that.
pub(crate) fn synthetic_maintenance_input() -> MaintenanceInput {
    MaintenanceInput {
        schema_version: MAINTENANCE_INPUT_SCHEMA_VERSION,
        evaluation_time: DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
            .expect("fixed synthetic timestamp is in range"),
        subject: MaintenanceSubjectDoc {
            kind: MaintenanceSubjectKind::Title,
            title_id: "title-1".to_string(),
            season_number: None,
            episode_id: None,
            facet: "movie".to_string(),
            name: "Test Movie".to_string(),
            year: Some(2024),
        },
        library: MaintenanceLibraryDoc {
            id: "library-1".to_string(),
            name: "Movies".to_string(),
        },
        facts: MaintenanceFactsDoc {
            monitored: Observation::known(true),
            tags: Observation::known(vec!["keep".to_string()]),
            quality_profile_id: Observation::known("profile-1".to_string()),
            added_at: Observation::known("2024-01-01T00:00:00Z".to_string()),
            added_by_user_id: Observation::known("user-1".to_string()),
            added_by_username: Observation::known("operator".to_string()),
            requested: Observation::known(true),
            requested_by_user_ids: Observation::known(vec!["user-1".to_string()]),
            requested_by_usernames: Observation::known(vec!["operator".to_string()]),
            first_imported_at: Observation::known("2024-01-02T00:00:00Z".to_string()),
            last_upgraded_at: Observation::known("2024-02-01T00:00:00Z".to_string()),
            has_file: Observation::known(true),
            file_count: Observation::known(1),
            total_file_size_bytes: Observation::known(8_000_000_000),
            files: Observation::known(vec![MaintenanceFileDoc {
                size_bytes: Some(8_000_000_000),
                quality: Some("2160P".to_string()),
                video_codec: Some("hevc".to_string()),
                video_width: Some(3840),
                video_height: Some(2160),
                audio_languages: vec!["eng".to_string()],
                subtitle_languages: vec!["eng".to_string()],
                added_at: Some("2024-01-02T00:00:00Z".to_string()),
            }]),
            episode_count: Observation::known(0),
            episode_file_count: Observation::known(0),
            monitored_episode_count: Observation::known(0),
            active_downloads: Observation::known(false),
            watched_by_user_ids: Observation::known(vec!["user-1".to_string()]),
            last_watched_at: Observation::known("2024-03-01T00:00:00Z".to_string()),
            watched_by_any_requester: Observation::known(true),
            watched_by_all_requesters: Observation::known(true),
            keep_claim_active: Observation::known(false),
            request_lease_state: Observation::known("none".to_string()),
            // The one deliberately absent fact in the synthetic document: a
            // title with no live lease has no expiry, and inventing one would
            // make a dry-run of "the lease lapsed before <date>" pass against a
            // date no instance ever produced.
            request_lease_expires_at: Observation::absent(),
            active_retention_claims: Observation::known(0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZeroU32;
    use core::time::Duration;

    fn policy(id: &str, body: &str) -> MaintenancePolicy {
        MaintenancePolicy {
            id: id.to_string(),
            name: format!("rule {id}"),
            rego_source: rewrite_package_declaration(body, id),
        }
    }

    fn evaluate(policies: &[MaintenancePolicy]) -> MaintenanceEvalResult {
        evaluate_against(policies, synthetic_maintenance_input())
    }

    fn evaluate_against(
        policies: &[MaintenancePolicy],
        input: MaintenanceInput,
    ) -> MaintenanceEvalResult {
        let engine = MaintenanceRulesEngine::build(policies).expect("policies should compile");
        engine
            .evaluator()
            .evaluate(&input)
            .expect("evaluation should succeed")
    }

    #[test]
    fn matching_rule_reports_match() {
        let result = evaluate(&[policy(
            "monitored_movie",
            "match if {\n  input.facts.monitored\n}\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.records.len(), 1);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Match
        );
        assert!(result.records[0].decision.reason_codes.is_empty());
    }

    #[test]
    fn undefined_match_reports_no_match() {
        let result = evaluate(&[policy(
            "never_matches",
            "match if {\n  input.subject.facet == \"series\"\n}\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::NoMatch
        );
    }

    /// The manual surface: `input.observations.*` never triggers host-derived
    /// unknownness, so a rule that opts into it still owns its own `unknown`.
    #[test]
    fn a_policy_declared_unknown_takes_precedence_over_match() {
        let result = evaluate(&[policy(
            "holds_on_unknown",
            "match := true\n\nunknown if {\n  input.observations.last_upgraded_at.status == \"known\"\n}\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Unknown
        );
    }

    /// The core of schema v2: no `unknown if` is written anywhere, and the rule
    /// is still held because a fact it reads is one Scryer could not observe.
    #[test]
    fn a_rule_reading_an_unobservable_fact_is_held_without_declaring_unknown() {
        let mut input = synthetic_maintenance_input();
        input.facts.active_downloads = Observation::unknown("not_yet_collected");

        let result = evaluate_against(
            &[policy(
                "downloads_in_flight",
                "match if {\n  not input.facts.active_downloads\n}\n",
            )],
            input,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Unknown,
            "a missing key would otherwise read as a decisive 'no downloads'"
        );
        assert_eq!(
            result.records[0].decision.reason_codes,
            vec!["not_yet_collected".to_string()]
        );
    }

    /// Absence is an answer. `added_by_user_id` is absent for a scan-created
    /// title, and a rule keying on that must decide rather than hold.
    #[test]
    fn a_rule_matching_an_absent_fact_decides_rather_than_holding() {
        let mut input = synthetic_maintenance_input();
        input.facts.added_by_user_id = Observation::absent_because("title_added_by_system");

        let result = evaluate_against(
            &[policy(
                "system_added",
                "match if {\n  not input.facts.added_by_user_id\n  input.facts.has_file\n}\n",
            )],
            input,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Match
        );
        assert!(result.records[0].decision.reason_codes.is_empty());
    }

    /// Every unobservable fact the rule reads contributes its reason, once.
    #[test]
    fn held_rules_report_the_union_of_the_reasons_that_held_them() {
        let mut input = synthetic_maintenance_input();
        input.facts.active_downloads = Observation::unknown("not_yet_collected");
        input.facts.last_upgraded_at = Observation::unknown("not_yet_collected");
        input.facts.added_by_username = Observation::unknown("user_not_found");

        let result = evaluate_against(
            &[policy(
                "many_unknowns",
                "match if {\n  input.facts.active_downloads\n  input.facts.last_upgraded_at\n  \
                 input.facts.added_by_username == \"operator\"\n}\n",
            )],
            input,
        );

        assert_eq!(
            result.records[0].decision.reason_codes,
            vec![
                "not_yet_collected".to_string(),
                "user_not_found".to_string()
            ],
            "one code per distinct reason, in fact-name order"
        );
    }

    /// The opt-out has to actually opt out, or the advanced surface is
    /// unreachable: a rule reading the envelope sees the unknown itself.
    #[test]
    fn observation_references_do_not_trigger_host_derived_unknownness() {
        let mut input = synthetic_maintenance_input();
        input.facts.active_downloads = Observation::unknown("not_yet_collected");

        let result = evaluate_against(
            &[policy(
                "inspects_the_envelope",
                "match if {\n  input.observations.active_downloads.status == \"unknown\"\n}\n\n\
                 reasons contains reason if {\n  \
                   reason := input.observations.active_downloads.reason\n\
                 }\n",
            )],
            input,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Match
        );
        assert_eq!(
            result.records[0].decision.reason_codes,
            vec!["not_yet_collected".to_string()]
        );
    }

    /// `evaluation_time` is the clock rules are meant to use, and date maths on
    /// it has to work — this is what the regorus `time` feature buys.
    #[test]
    fn evaluation_time_supports_date_arithmetic() {
        const AGE_MATCHER: &str = "day_ns := (24 * 60 * 60) * 1000000000\n\n\
             match if {\n  \
               age := time.parse_rfc3339_ns(input.evaluation_time) - \
             time.parse_rfc3339_ns(input.facts.added_at)\n  \
               age > 180 * day_ns\n\
             }\n";

        // Synthetic input is evaluated at 2023-11-14.
        let mut old = synthetic_maintenance_input();
        old.facts.added_at = Observation::known("2020-01-01T00:00:00Z".to_string());
        let matched = evaluate_against(&[policy("added_long_ago", AGE_MATCHER)], old);
        assert!(matched.errors.is_empty(), "{:?}", matched.errors);
        assert_eq!(
            matched.records[0].decision.outcome,
            MaintenanceOutcome::Match,
            "date maths must actually run, not evaluate to undefined"
        );

        let mut recent = synthetic_maintenance_input();
        recent.facts.added_at = Observation::known("2023-11-01T00:00:00Z".to_string());
        let unmatched = evaluate_against(&[policy("added_recently", AGE_MATCHER)], recent);
        assert!(unmatched.errors.is_empty(), "{:?}", unmatched.errors);
        assert_eq!(
            unmatched.records[0].decision.outcome,
            MaintenanceOutcome::NoMatch
        );
    }

    #[test]
    fn reason_codes_are_collected_from_a_set_rule() {
        let result = evaluate(&[policy(
            "set_reasons",
            "match := true\n\nreasons contains \"stale\"\n\nreasons contains \"unwatched\"\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.reason_codes,
            vec!["stale".to_string(), "unwatched".to_string()]
        );
    }

    #[test]
    fn reason_codes_are_collected_from_an_array_rule() {
        let result = evaluate(&[policy(
            "array_reasons",
            "match := true\n\nreasons := [\"first\", \"second\"]\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.reason_codes,
            vec!["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn non_boolean_match_is_an_error_not_a_decision() {
        let result = evaluate(&[policy("numeric_match", "match := 1\n")]);

        assert!(result.records.is_empty(), "{:?}", result.records);
        assert_eq!(result.errors.len(), 1);
        assert!(
            result.errors[0]
                .message
                .contains("'match' must be a boolean"),
            "{}",
            result.errors[0].message
        );
    }

    #[test]
    fn oversized_reason_list_is_rejected() {
        let entries = (0..MAX_REASON_CODES + 1)
            .map(|i| format!("\"reason_{i}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let result = evaluate(&[policy(
            "too_many_reasons",
            &format!("match := true\n\nreasons := [{entries}]\n"),
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
            &format!("match := true\n\nreasons := [\"{long}\"]\n"),
        )]);

        assert!(result.records.is_empty(), "{:?}", result.records);
        assert!(
            result.errors[0].message.contains("at most 120"),
            "{}",
            result.errors[0].message
        );
    }

    #[test]
    fn a_broken_rule_does_not_stop_the_batch() {
        let result = evaluate(&[
            policy(
                "broken",
                "match if {\n  lower(input.subject.year) == \"x\"\n}\n",
            ),
            policy("healthy", "match := true\n"),
        ]);

        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].rule_set_id, "broken");
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].rule_set_id, "healthy");
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Match
        );
    }

    #[test]
    fn execution_budget_turns_a_runaway_rule_into_an_error() {
        let mut limits = RuntimeLimits::maintenance_defaults();
        limits.max_execution_time = Duration::from_millis(1);
        limits.timer_check_interval = NonZeroU32::new(1).expect("non-zero");

        let policies = [policy(
            "runaway",
            "match if {\n  count([1 |\n    some i in numbers.range(1, 3000)\n    some j in numbers.range(1, 3000)\n    i == j\n  ]) > 0\n}\n",
        )];
        let engine =
            MaintenanceRulesEngine::build_with_limits(&policies, limits).expect("should compile");
        let result = engine
            .evaluator()
            .evaluate(&synthetic_maintenance_input())
            .expect("evaluation should return, not hang");

        assert!(result.records.is_empty(), "{:?}", result.records);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].rule_set_id, "runaway");
    }

    #[test]
    fn records_carry_a_stable_policy_content_hash() {
        let policies = [policy("hashed", "match := true\n")];
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
        let engine = MaintenanceRulesEngine::empty();
        assert!(engine.is_empty());
        assert_eq!(engine.rule_count(), 0);
        let result = engine
            .evaluator()
            .evaluate(&synthetic_maintenance_input())
            .expect("evaluation should succeed");
        assert!(result.records.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn oversized_input_is_rejected_before_evaluation() {
        let mut limits = RuntimeLimits::maintenance_defaults();
        limits.max_input_bytes = 16;
        let engine =
            MaintenanceRulesEngine::build_with_limits(&[policy("tiny", "match := true\n")], limits)
                .expect("should compile");
        let err = engine
            .evaluator()
            .evaluate(&synthetic_maintenance_input())
            .expect_err("input should exceed the bound");
        assert!(matches!(err, RulesError::InputTooLarge { .. }), "{err:?}");
    }

    #[test]
    fn the_input_document_carries_bare_facts_and_full_observations() {
        let mut input = synthetic_maintenance_input();
        input.facts.active_downloads = Observation::unknown("not_yet_collected");
        input.facts.added_by_user_id = Observation::absent_because("title_added_by_system");

        let doc = serde_json::to_value(&input).expect("input serializes");

        assert_eq!(doc["schema_version"], 2);
        // Known: bare value on facts, envelope on observations.
        assert_eq!(doc["facts"]["monitored"], true);
        assert_eq!(doc["observations"]["monitored"]["status"], "known");
        assert_eq!(doc["observations"]["monitored"]["value"], true);
        // Unknown and absent: missing from facts entirely, still on
        // observations with the reason that explains them.
        for fact in ["active_downloads", "added_by_user_id"] {
            assert!(doc["facts"].get(fact).is_none(), "{fact}");
            assert!(doc["observations"][fact]["reason"].is_string(), "{fact}");
        }
        assert_eq!(doc["observations"]["active_downloads"]["status"], "unknown");
        assert_eq!(doc["observations"]["added_by_user_id"]["status"], "absent");
        // The two namespaces describe the same facts, so neither can gain a
        // key the other never heard of.
        for fact in doc["facts"].as_object().expect("facts object").keys() {
            assert!(doc["observations"].get(fact).is_some(), "{fact}");
        }
    }

    #[test]
    fn unknown_observations_serialize_without_a_value() {
        let doc = serde_json::to_value(Observation::<bool>::unknown("source_unreachable"))
            .expect("serializable");
        assert_eq!(doc["status"], "unknown");
        assert_eq!(doc["reason"], "source_unreachable");
        assert!(doc.get("value").is_none());

        let absent = serde_json::to_value(Observation::<bool>::absent()).expect("serializable");
        assert_eq!(absent["status"], "absent");
        assert!(absent.get("observed_at").is_none());
    }

    /// The wrapper is what every stored matcher is evaluated through, so its
    /// text is a contract with the rules already in people's databases. It is
    /// generated by the shared core now; it must still come out character for
    /// character as it always did.
    #[test]
    fn the_generated_wrapper_is_unchanged_and_projects_the_family_hold_head() {
        assert_eq!(
            decision_wrapper_source("rule_1"),
            "package scryer.maintenance.wrapper.rule_1\n\
             import rego.v1\n\n\
             decision := {\n\
             \t\"matched\": object.get(data.scryer.maintenance.user.rule_1, \"match\", false),\n\
             \t\"unknown\": object.get(data.scryer.maintenance.user.rule_1, \"unknown\", false),\n\
             \t\"reasons\": object.get(data.scryer.maintenance.user.rule_1, \"reasons\", []),\n\
             }\n"
        );
        assert_eq!(
            decision_wrapper_rule_path("rule_1"),
            "data.scryer.maintenance.wrapper.rule_1.decision"
        );
        assert_eq!(user_policy_path("rule_1"), "maintenance/rule_1.rego");
        assert_eq!(
            decision_wrapper_policy_path("rule_1"),
            "internal/rule_1_maintenance_wrapper.rego"
        );

        // Maintenance says `unknown if`; the request family will say
        // `manual if`. The head the wrapper reads is exactly what the family
        // declares, so the two cannot drift apart.
        let hold = MaintenanceFamily::hold_rule_name().expect("maintenance holds");
        assert_eq!(hold, "unknown");
        assert!(
            decision_wrapper_source("rule_1").contains(&format!("\"{hold}\", false)")),
            "the wrapper must read the head the family declares"
        );
    }
}
