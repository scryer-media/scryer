//! Core mechanics proved against a family that exists only here.
//!
//! The maintenance and release suites prove the two shipped families still
//! behave as they did. These prove the *core* keeps its promises for a family
//! nobody has written yet — which is the whole reason it exists.

use std::collections::{BTreeMap, BTreeSet};

use regorus::Value;
use serde::Serialize;

use super::decode::{decode_reasons, decode_tags};
use super::engine::{PolicyEngine, RuleHandle};
use super::observation::{Observation, serialize_fact_namespaces};
use super::wrapper::{WrapperField, object_wrapper_source};
use super::{PolicyFamily, PolicyRecord};
use crate::RulesError;
use crate::runtime::RuntimeLimits;

const USER_PREFIX: &str = "scryer.probe.user";
const WRAPPER_PREFIX: &str = "scryer.probe.wrapper";
/// The request family will say `manual`; the probe family proves the head name
/// really is a parameter by using neither `unknown` nor `manual`.
const HOLD_RULE: &str = "hold";

#[derive(Debug, Clone)]
struct ProbePolicy {
    id: String,
    name: String,
    rego_source: String,
    tier: &'static str,
}

impl PolicyRecord for ProbePolicy {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbeDecision {
    verdict: &'static str,
    reasons: Vec<String>,
    tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbeError {
    rule_set_id: String,
    tier: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProbeFacts {
    age_rating: Observation<i64>,
    genres: Observation<Vec<String>>,
}

#[derive(Debug, Clone)]
struct ProbeInput {
    facts: ProbeFacts,
}

impl Serialize for ProbeInput {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let namespaces =
            serialize_fact_namespaces(&self.facts).map_err(serde::ser::Error::custom)?;
        let mut state = serializer.serialize_struct("ProbeInput", 2)?;
        state.serialize_field("facts", &namespaces.facts)?;
        state.serialize_field("observations", &namespaces.observations)?;
        state.end()
    }
}

/// The probe family, parameterized on nothing but whether it bounds its input,
/// so the two settings of `BOUND_INPUT` are the same family in every other
/// respect.
struct Probe<const BOUND_INPUT: bool>;

/// The probe family as every other test uses it: input bounded, like any family
/// written against this core.
type ProbeFamily = Probe<true>;
/// The same family opted out of the bound, the way release is.
type UnboundedProbeFamily = Probe<false>;

impl<const BOUND: bool> PolicyFamily for Probe<BOUND> {
    const NAME: &'static str = "probe";
    const USER_PACKAGE_PREFIX: &'static str = USER_PREFIX;
    const WRAPPER_PACKAGE_PREFIX: &'static str = WRAPPER_PREFIX;
    const TRACKS_REFERENCED_FACTS: bool = true;
    const BOUND_INPUT: bool = BOUND;

    type Policy = ProbePolicy;
    type Input = ProbeInput;
    type Decision = ProbeDecision;
    type RuleExtra = &'static str;
    type EvalContext = str;
    type EvalError = ProbeError;

    fn limits() -> RuntimeLimits {
        RuntimeLimits::maintenance_defaults()
    }

    fn user_policy_path(rule_id: &str) -> String {
        format!("probe/{rule_id}.rego")
    }

    fn wrapper_policy_path(rule_id: &str) -> String {
        format!("internal/{rule_id}_probe_wrapper.rego")
    }

    fn wrapper_source(rule_id: &str) -> String {
        object_wrapper_source(
            WRAPPER_PREFIX,
            USER_PREFIX,
            rule_id,
            "decision",
            &[
                WrapperField::new("allow", "allow", "false"),
                WrapperField::new("hold", HOLD_RULE, "false"),
                WrapperField::new("reasons", "reasons", "[]"),
                WrapperField::new("tags", "tags", "[]"),
            ],
        )
    }

    fn wrapper_rule_path(rule_id: &str) -> String {
        format!("data.{WRAPPER_PREFIX}.{rule_id}.decision")
    }

    fn rule_extra(policy: &Self::Policy) -> Self::RuleExtra {
        policy.tier
    }

    fn referenced_facts(
        policy: &Self::Policy,
        _policy_path: &str,
    ) -> Result<BTreeSet<String>, String> {
        // The probe family names its dependencies in the source rather than
        // parsing them: the core only cares that a set comes back.
        Ok(["age_rating", "genres"]
            .into_iter()
            .filter(|fact| policy.rego_source.contains(&format!("input.facts.{fact}")))
            .map(str::to_string)
            .collect())
    }

    fn hold_rule_name() -> Option<&'static str> {
        Some(HOLD_RULE)
    }

    fn held_decision(reason_codes: Vec<String>) -> Self::Decision {
        ProbeDecision {
            verdict: "held",
            reasons: reason_codes,
            tags: Vec::new(),
        }
    }

    fn applies(extra: &Self::RuleExtra, ctx: &Self::EvalContext) -> bool {
        *extra == ctx
    }

    fn decode(
        value: &Value,
        _rule_id: &str,
        _rule_name: &str,
        _extra: &Self::RuleExtra,
    ) -> Result<Self::Decision, String> {
        if matches!(value, Value::Undefined) {
            return Err("decision rule produced no value".to_string());
        }
        let allow = *value["allow"]
            .as_bool()
            .map_err(|_| "'allow' must be a boolean".to_string())?;
        let hold = *value["hold"]
            .as_bool()
            .map_err(|_| "'hold' must be a boolean".to_string())?;
        Ok(ProbeDecision {
            verdict: if hold {
                "held"
            } else if allow {
                "allow"
            } else {
                "abstain"
            },
            reasons: decode_reasons(&value["reasons"])?,
            tags: decode_tags(&value["tags"])?,
        })
    }

    fn post_decode(decision: &Self::Decision, extra: &Self::RuleExtra) -> Result<(), String> {
        if *extra == "strict" && !decision.tags.is_empty() {
            return Err("strict rules may not emit tags".to_string());
        }
        Ok(())
    }

    fn eval_error(rule: &RuleHandle<Self::RuleExtra>, message: String) -> Self::EvalError {
        ProbeError {
            rule_set_id: rule.id.clone(),
            tier: rule.extra,
            message,
        }
    }
}

type ProbeEngine = PolicyEngine<ProbeFamily>;

fn policy(id: &str, tier: &'static str, body: &str) -> ProbePolicy {
    ProbePolicy {
        id: id.to_string(),
        name: format!("probe {id}"),
        rego_source: format!("package {USER_PREFIX}.{id}\nimport rego.v1\n\n{body}"),
        tier,
    }
}

fn known_input() -> ProbeInput {
    ProbeInput {
        facts: ProbeFacts {
            age_rating: Observation::known(7),
            genres: Observation::known(vec!["comedy".to_string()]),
        },
    }
}

fn evaluate(
    policies: &[ProbePolicy],
    input: &ProbeInput,
    tier: &str,
) -> (Vec<(String, ProbeDecision)>, Vec<ProbeError>) {
    let engine = ProbeEngine::build(policies).expect("probe policies should compile");
    let outcome = engine
        .evaluator()
        .evaluate_policies(input, tier)
        .expect("evaluation should succeed");
    (
        outcome
            .records
            .into_iter()
            .map(|record| (record.rule_set_id, record.decision))
            .collect(),
        outcome.errors,
    )
}

#[test]
fn a_new_family_gets_the_whole_loop_for_free() {
    let (records, errors) = evaluate(
        &[policy(
            "family_rated",
            "normal",
            "allow if {\n  input.facts.age_rating <= 13\n}\n\n\
             reasons contains \"age_ok\"\n\n\
             tags contains \"kids\"\n",
        )],
        &known_input(),
        "normal",
    );

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        records,
        vec![(
            "family_rated".to_string(),
            ProbeDecision {
                verdict: "allow",
                reasons: vec!["age_ok".to_string()],
                tags: vec!["kids".to_string()],
            }
        )]
    );
}

#[test]
fn the_hold_head_name_is_a_family_parameter() {
    // `hold if`, not `unknown if` and not `manual if` — the core never assumes
    // a spelling.
    let (records, errors) = evaluate(
        &[policy(
            "author_held",
            "normal",
            "allow := true\n\nhold if {\n  input.observations.age_rating.status == \"known\"\n}\n",
        )],
        &known_input(),
        "normal",
    );

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(records[0].1.verdict, "held");
}

#[test]
fn a_rule_reading_an_unobservable_fact_is_held_before_it_is_consulted() {
    let mut input = known_input();
    input.facts.age_rating = Observation::unknown("metadata_not_collected");

    let (records, errors) = evaluate(
        &[policy(
            "needs_rating",
            "normal",
            "allow if {\n  not input.facts.age_rating\n}\n\n\
             tags contains \"unrated\"\n",
        )],
        &input,
        "normal",
    );

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        records[0].1,
        ProbeDecision {
            verdict: "held",
            reasons: vec!["metadata_not_collected".to_string()],
            tags: Vec::new(),
        },
        "a held rule is never evaluated, so it contributes no tags of its own"
    );
}

#[test]
fn an_absent_fact_still_decides() {
    let mut input = known_input();
    input.facts.age_rating = Observation::absent_because("title_has_no_rating");

    let (records, errors) = evaluate(
        &[policy(
            "needs_rating",
            "normal",
            "allow if {\n  not input.facts.age_rating\n}\n",
        )],
        &input,
        "normal",
    );

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(records[0].1.verdict, "allow");
}

#[test]
fn rules_that_do_not_apply_contribute_neither_record_nor_error() {
    let (records, errors) = evaluate(
        &[
            policy("scoped_out", "strict", "allow := true\n"),
            policy("scoped_in", "normal", "allow := true\n"),
        ],
        &known_input(),
        "normal",
    );

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].0, "scoped_in");
}

#[test]
fn a_broken_rule_is_isolated_and_attributed_with_family_metadata() {
    let (records, errors) = evaluate(
        &[
            policy("broken", "normal", "allow if {\n  lower(1) == \"x\"\n}\n"),
            policy("healthy", "normal", "allow := true\n"),
        ],
        &known_input(),
        "normal",
    );

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].0, "healthy");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].rule_set_id, "broken");
    assert_eq!(
        errors[0].tier, "normal",
        "family metadata reaches the error"
    );
}

#[test]
fn a_malformed_decision_is_an_error_not_a_decision() {
    let (records, errors) = evaluate(
        &[policy("numeric", "normal", "allow := 1\n")],
        &known_input(),
        "normal",
    );

    assert!(records.is_empty(), "{records:?}");
    assert!(
        errors[0].message.contains("'allow' must be a boolean"),
        "{}",
        errors[0].message
    );
}

#[test]
fn post_decode_rejection_drops_the_record() {
    let (records, errors) = evaluate(
        &[policy(
            "tagging_strict_rule",
            "strict",
            "allow := true\n\ntags contains \"kids\"\n",
        )],
        &known_input(),
        "strict",
    );

    assert!(records.is_empty(), "{records:?}");
    assert!(
        errors[0].message.contains("may not emit tags"),
        "{}",
        errors[0].message
    );
}

#[test]
fn oversized_input_is_refused_before_any_rule_runs() {
    let mut limits = RuntimeLimits::maintenance_defaults();
    limits.max_input_bytes = 16;
    let engine =
        ProbeEngine::build_with_limits(&[policy("tiny", "normal", "allow := true\n")], limits)
            .expect("should compile");
    let err = engine
        .evaluator()
        .evaluate_policies(&known_input(), "normal")
        .expect_err("input should exceed the bound");
    assert!(matches!(err, RulesError::InputTooLarge { .. }), "{err:?}");
}

#[test]
fn a_family_that_opts_out_of_the_bound_evaluates_an_oversized_input() {
    // Release opts out this way, so that the bound the core added is not a
    // behaviour change for a family that shipped without one. Same policy, same
    // input, same limits as the test above — only `BOUND_INPUT` differs, and
    // the oversized document is scored instead of refused.
    let mut limits = RuntimeLimits::maintenance_defaults();
    limits.max_input_bytes = 16;
    let policies = [policy("tiny", "normal", "allow := true\n")];

    let unbounded = PolicyEngine::<UnboundedProbeFamily>::build_with_limits(&policies, limits)
        .expect("should compile");
    let outcome = unbounded
        .evaluator()
        .evaluate_policies(&known_input(), "normal")
        .expect("an unbounded family should not refuse an oversized input");
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert_eq!(outcome.records.len(), 1);
    assert_eq!(outcome.records[0].decision.verdict, "allow");

    let bounded =
        PolicyEngine::<ProbeFamily>::build_with_limits(&policies, limits).expect("should compile");
    let err = bounded
        .evaluator()
        .evaluate_policies(&known_input(), "normal")
        .expect_err("a bounding family should still refuse the same input");
    assert!(matches!(err, RulesError::InputTooLarge { .. }), "{err:?}");
}

#[test]
fn an_empty_engine_decides_nothing() {
    let engine = ProbeEngine::empty();
    assert!(engine.is_empty());
    assert_eq!(engine.rule_count(), 0);
    let outcome = engine
        .evaluator()
        .evaluate_policies(&known_input(), "normal")
        .expect("evaluation should succeed");
    assert!(outcome.records.is_empty());
    assert!(outcome.errors.is_empty());
}

#[test]
fn handles_carry_the_content_hash_and_the_static_fact_set() {
    let policies = [policy(
        "hashed",
        "normal",
        "allow if {\n  input.facts.genres[_] == \"comedy\"\n}\n",
    )];
    let engine = ProbeEngine::build(&policies).expect("should compile");
    let rule = &engine.rules()[0];

    assert_eq!(rule.content_hash.len(), 64);
    assert_eq!(
        rule.content_hash,
        crate::runtime::content_hash(&policies[0].rego_source)
    );
    assert_eq!(
        rule.referenced_facts,
        BTreeSet::from(["genres".to_string()])
    );
    assert_eq!(rule.extra, "normal");
}

#[test]
fn held_reason_codes_are_deduplicated_in_fact_name_order() {
    let referenced = BTreeSet::from([
        "genres".to_string(),
        "age_rating".to_string(),
        "runtime".to_string(),
    ]);
    let unobservable = BTreeMap::from([
        ("age_rating".to_string(), "not_collected".to_string()),
        ("genres".to_string(), "source_unreachable".to_string()),
        ("runtime".to_string(), "not_collected".to_string()),
    ]);

    assert_eq!(
        super::observation::held_reason_codes(&referenced, &unobservable),
        vec![
            "not_collected".to_string(),
            "source_unreachable".to_string()
        ]
    );
}
