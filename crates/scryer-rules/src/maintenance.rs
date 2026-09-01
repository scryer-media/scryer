//! Maintenance policy family: match existing library media against user rules.
//!
//! The matcher answers one question — does this subject match this rule right
//! now — and returns match / no-match / unknown. It never selects, names, or
//! performs an action: the action comes from the validated rule revision that
//! owns the matcher, so a malformed or timed-out evaluation can never authorize
//! one.
//!
//! Media-server signals (Plex watchlist, Jellyfin/Emby favorites, play history)
//! are deliberately absent from schema v1. They arrive on their own track once
//! a provider-neutral signal store exists; adding them here early would force
//! rules to be written against facts Scryer cannot yet observe.

use crate::RulesError;
use crate::runtime::{self, RuntimeLimits};
use chrono::{DateTime, Utc};
use regorus::{Engine, Value};
use serde::Serialize;
use std::sync::Arc;
use tracing::warn;

// ── Contract constants ──────────────────────────────────────────────────────

/// Version of the maintenance input document. Rules are authored against a
/// specific version; bumping it is a breaking change to every stored matcher.
pub const MAINTENANCE_INPUT_SCHEMA_VERSION: u32 = 1;

/// Package prefix for user-authored maintenance matchers.
pub(crate) const USER_PACKAGE_PREFIX: &str = "scryer.maintenance.user";

/// Package prefix for the generated evaluation wrapper. The wrapper lives in a
/// separate package because it reads the whole user package document; putting
/// it inside that package would make the module self-referential.
const WRAPPER_PACKAGE_PREFIX: &str = "scryer.maintenance.wrapper";

/// Bounds on the reason codes a rule may emit. Reason codes are persisted on
/// candidates and rendered in the UI, so an unbounded list is a storage and
/// display hazard rather than a useful signal.
const MAX_REASON_CODES: usize = 32;
const MAX_REASON_CODE_LEN: usize = 120;

// ── Observation envelope ────────────────────────────────────────────────────

/// Three-valued availability envelope wrapping every maintenance fact.
///
/// `Absent` means the source completed and confirmed there is no value.
/// `Unknown` means Scryer cannot know — the data is stale, unmapped,
/// unsupported, forbidden, or the lookup failed. An unknown fact is never
/// coerced to `false`, `0`, or `""`; rules that need certainty must test
/// `status` explicitly.
///
/// `Absent` may also carry a `reason`, using the same stable code vocabulary as
/// `Unknown`. It answers a different question — *why is there no value*, not
/// *why could Scryer not look* — and is optional, so an absence with nothing
/// useful to say still serializes without the field.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Observation<T: Serialize> {
    Known {
        value: T,
        #[serde(skip_serializing_if = "Option::is_none")]
        observed_at: Option<String>,
    },
    Absent {
        #[serde(skip_serializing_if = "Option::is_none")]
        observed_at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Unknown {
        reason: String,
    },
}

impl<T: Serialize> Observation<T> {
    /// A value Scryer observed, with no recorded observation time.
    pub fn known(value: T) -> Self {
        Self::Known {
            value,
            observed_at: None,
        }
    }

    /// A value Scryer observed at a specific RFC3339 timestamp.
    pub fn known_at(value: T, observed_at: impl Into<String>) -> Self {
        Self::Known {
            value,
            observed_at: Some(observed_at.into()),
        }
    }

    /// Confirmed absence: the source answered, and there is no value.
    pub fn absent() -> Self {
        Self::Absent {
            observed_at: None,
            reason: None,
        }
    }

    /// Confirmed absence, with the time the source answered.
    pub fn absent_at(observed_at: impl Into<String>) -> Self {
        Self::Absent {
            observed_at: Some(observed_at.into()),
            reason: None,
        }
    }

    /// Confirmed absence, carrying the stable machine code that explains why
    /// there is no value. Still an absence: the source answered.
    pub fn absent_because(reason: impl Into<String>) -> Self {
        Self::Absent {
            observed_at: None,
            reason: Some(reason.into()),
        }
    }

    /// Scryer cannot know the answer. `reason` is a stable machine code.
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self::Unknown {
            reason: reason.into(),
        }
    }
}

// ── Input document ──────────────────────────────────────────────────────────

/// Input document set once per subject for maintenance rule evaluation.
///
/// `evaluation_time` is supplied by the host and captured once per run so every
/// subject in that run compares against the same instant. Policies must never
/// reach for a clock of their own.
#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceInput {
    pub schema_version: u32,
    pub evaluation_time: DateTime<Utc>,
    pub subject: MaintenanceSubjectDoc,
    pub library: MaintenanceLibraryDoc,
    pub facts: MaintenanceFactsDoc,
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

/// The fact snapshot. Every field is an observation envelope: a fact Scryer
/// failed to resolve stays unknown rather than defaulting to a value the rule
/// would read as decisive.
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
}

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
#[derive(Debug, Clone)]
pub struct MaintenanceEvalRecord {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub policy_content_hash: String,
    pub decision: MaintenanceDecision,
}

/// A per-rule failure. A rule that errors produces no record at all, so it can
/// never advance a candidate.
#[derive(Debug, Clone)]
pub struct MaintenanceEvalError {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct MaintenanceEvalResult {
    pub records: Vec<MaintenanceEvalRecord>,
    pub errors: Vec<MaintenanceEvalError>,
}

// ── Package rewriting and wrapper generation ────────────────────────────────

/// Rewrite (or insert) the package declaration so stored maintenance source
/// always carries the maintenance prefix plus the system-assigned rule ID.
pub fn rewrite_package_declaration(rego_source: &str, rule_id: &str) -> String {
    runtime::rewrite_package_declaration_with_prefix(rego_source, USER_PACKAGE_PREFIX, rule_id)
}

/// Source for the generated evaluation entry point.
///
/// `object.get` with a default is what makes `unknown` and `reasons` optional
/// and an undefined `match` mean "did not match": a rule whose body fails
/// leaves the name off the package document entirely, and the default fills in.
/// A `match` that is defined but not boolean survives to the host, which
/// rejects it rather than coercing it.
pub(crate) fn decision_wrapper_source(rule_id: &str) -> String {
    format!(
        "package {WRAPPER_PACKAGE_PREFIX}.{rule_id}\n\
         import rego.v1\n\n\
         decision := {{\n\
         \t\"matched\": object.get(data.{USER_PACKAGE_PREFIX}.{rule_id}, \"match\", false),\n\
         \t\"unknown\": object.get(data.{USER_PACKAGE_PREFIX}.{rule_id}, \"unknown\", false),\n\
         \t\"reasons\": object.get(data.{USER_PACKAGE_PREFIX}.{rule_id}, \"reasons\", []),\n\
         }}\n"
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

#[derive(Debug, Clone)]
struct MaintenanceRuleHandle {
    id: String,
    name: String,
    content_hash: String,
}

/// Pre-compiled engine holding every active maintenance matcher.
///
/// Built once per rule-set revision and shared; evaluators are cheap clones
/// created per evaluation run.
#[derive(Clone)]
pub struct MaintenanceRulesEngine {
    template: Arc<Engine>,
    rules: Vec<MaintenanceRuleHandle>,
    limits: RuntimeLimits,
}

impl MaintenanceRulesEngine {
    /// Build an engine under the standard maintenance limits.
    pub fn build(policies: &[MaintenancePolicy]) -> Result<Self, RulesError> {
        Self::build_with_limits(policies, RuntimeLimits::maintenance_defaults())
    }

    /// Build an engine under caller-supplied limits. Exists so tests can prove
    /// the execution budget is enforced without waiting out the real one.
    pub fn build_with_limits(
        policies: &[MaintenancePolicy],
        limits: RuntimeLimits,
    ) -> Result<Self, RulesError> {
        let mut engine = runtime::configured_engine(&limits);
        let mut rules = Vec::with_capacity(policies.len());

        for policy in policies {
            engine
                .add_policy(user_policy_path(&policy.id), policy.rego_source.clone())
                .map_err(|e| RulesError::Compilation(format!("{}: {e}", policy.id)))?;
            engine
                .add_policy(
                    decision_wrapper_policy_path(&policy.id),
                    decision_wrapper_source(&policy.id),
                )
                .map_err(|e| RulesError::Compilation(format!("{}: {e}", policy.id)))?;
            rules.push(MaintenanceRuleHandle {
                id: policy.id.clone(),
                name: policy.name.clone(),
                content_hash: runtime::content_hash(&policy.rego_source),
            });
        }

        Ok(Self {
            template: Arc::new(engine),
            rules,
            limits,
        })
    }

    pub fn empty() -> Self {
        let limits = RuntimeLimits::maintenance_defaults();
        Self {
            template: Arc::new(runtime::configured_engine(&limits)),
            rules: Vec::new(),
            limits,
        }
    }

    /// True when no matchers are loaded. Callers should skip evaluation.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Create an evaluator for a single evaluation run.
    pub fn evaluator(&self) -> MaintenanceRulesEvaluator {
        MaintenanceRulesEvaluator {
            engine: (*self.template).clone(),
            rules: self.rules.clone(),
            limits: self.limits,
        }
    }
}

/// Evaluates every loaded matcher against one subject at a time.
pub struct MaintenanceRulesEvaluator {
    engine: Engine,
    rules: Vec<MaintenanceRuleHandle>,
    limits: RuntimeLimits,
}

impl MaintenanceRulesEvaluator {
    /// Evaluate every loaded matcher against one subject.
    ///
    /// Per-rule failures — compilation-clean rules that error at runtime,
    /// exceed the execution budget, or return a malformed decision — are
    /// collected and never abort the batch. A failing rule contributes no
    /// record, so it cannot advance or cancel a candidate.
    pub fn evaluate(
        &mut self,
        input: &MaintenanceInput,
    ) -> Result<MaintenanceEvalResult, RulesError> {
        let mut result = MaintenanceEvalResult {
            records: Vec::new(),
            errors: Vec::new(),
        };

        if self.rules.is_empty() {
            return Ok(result);
        }

        let input_value = runtime::bounded_input_value(input, &self.limits)?;
        self.engine.set_input(input_value);

        for rule in &self.rules {
            match self.engine.eval_rule(decision_wrapper_rule_path(&rule.id)) {
                Ok(value) => match decode_decision(&value) {
                    Ok(decision) => result.records.push(MaintenanceEvalRecord {
                        rule_set_id: rule.id.clone(),
                        rule_set_name: rule.name.clone(),
                        policy_content_hash: rule.content_hash.clone(),
                        decision,
                    }),
                    Err(message) => {
                        warn!(
                            rule_id = rule.id.as_str(),
                            %message,
                            "maintenance rule produced a malformed decision"
                        );
                        result.errors.push(MaintenanceEvalError {
                            rule_set_id: rule.id.clone(),
                            rule_set_name: rule.name.clone(),
                            message,
                        });
                    }
                },
                Err(e) => {
                    warn!(
                        rule_id = rule.id.as_str(),
                        error = %e,
                        "maintenance rule evaluation failed, skipping"
                    );
                    result.errors.push(MaintenanceEvalError {
                        rule_set_id: rule.id.clone(),
                        rule_set_name: rule.name.clone(),
                        message: e.to_string(),
                    });
                }
            }
        }

        Ok(result)
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

fn decode_reasons(value: &Value) -> Result<Vec<String>, String> {
    let items: Vec<&Value> = if let Ok(array) = value.as_array() {
        array.iter().collect()
    } else if let Ok(set) = value.as_set() {
        set.iter().collect()
    } else {
        return Err("'reasons' must be an array or set of strings".to_string());
    };

    if items.len() > MAX_REASON_CODES {
        return Err(format!(
            "'reasons' has {} entries, at most {MAX_REASON_CODES} are allowed",
            items.len()
        ));
    }

    let mut reasons = Vec::with_capacity(items.len());
    for item in items {
        let reason = item
            .as_string()
            .map_err(|_| "'reasons' must contain only strings".to_string())?;
        if reason.len() > MAX_REASON_CODE_LEN {
            return Err(format!(
                "reason code is {} characters, at most {MAX_REASON_CODE_LEN} are allowed",
                reason.len()
            ));
        }
        reasons.push(reason.to_string());
    }

    Ok(reasons)
}

// ── Synthetic input ─────────────────────────────────────────────────────────

/// Build a representative input for validation dry-runs: a Title subject with
/// every fact known, so a rule reaching for any documented fact executes its
/// real path instead of short-circuiting on an unknown envelope.
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
        let engine = MaintenanceRulesEngine::build(policies).expect("policies should compile");
        engine
            .evaluator()
            .evaluate(&synthetic_maintenance_input())
            .expect("evaluation should succeed")
    }

    #[test]
    fn matching_rule_reports_match() {
        let result = evaluate(&[policy(
            "monitored_movie",
            "match if {\n  input.facts.monitored.status == \"known\"\n  input.facts.monitored.value\n}\n",
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

    #[test]
    fn unknown_takes_precedence_over_match() {
        let result = evaluate(&[policy(
            "holds_on_unknown",
            "match := true\n\nunknown if {\n  input.facts.last_upgraded_at.status == \"known\"\n}\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Unknown
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
}
