//! The simple builder's shared fixtures, evaluated by the engine that runs
//! saved rules. Each case states whether its rule holds for its input: the
//! case's `rule` when it names one (a row's own helper, such as
//! `rule_applies_2`), otherwise `rule_applies`;
//! the web generates the same sources from the same models, so a case that
//! disagrees here means the builder's meaning and the engine's have drifted.

use crate::{lifecycle, rewrite_package_declaration, runtime, validation};
use regorus::Value as RegoValue;
use serde_json::Value;

fn fixtures(source: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(source)
        .expect("builder fixtures are JSON")
        .as_array()
        .expect("builder fixtures are a list")
        .clone()
}

fn lifecycle_fixtures() -> Vec<Value> {
    fixtures(include_str!(
        "../../../apps/scryer-web/lib/contracts/lifecycle-builder-fixtures.json"
    ))
}

fn scoring_fixtures() -> Vec<Value> {
    fixtures(include_str!(
        "../../../apps/scryer-web/lib/contracts/scoring-builder-fixtures.json"
    ))
}

/// Evaluate each case's rule for every case of one fixture. An undefined
/// result is `false`, exactly as an unmet condition is for a saved rule.
fn assert_cases(fixture: &Value, mut engine: regorus::Engine, package: &str) {
    let id = fixture["id"].as_str().expect("fixture id");
    let cases = fixture["cases"].as_array().expect("fixture cases");
    assert!(
        !cases.is_empty(),
        "{id}: a fixture states at least one case"
    );
    for (index, case) in cases.iter().enumerate() {
        engine.set_input(RegoValue::from(case["input"].clone()));
        let rule = case["rule"].as_str().unwrap_or("rule_applies");
        let applies = engine
            .eval_rule(format!("data.{package}.{rule}"))
            .unwrap_or_else(|error| panic!("{id} case {index}: {error}"));
        let expected = case["expected"].as_bool().expect("case expectation");
        assert_eq!(
            applies == RegoValue::from(true),
            expected,
            "{id} case {index}: {rule} for {}",
            case["input"]
        );
    }
}

#[test]
fn lifecycle_builder_fixtures_validate_and_their_cases_hold_in_the_engine() {
    let fixtures = lifecycle_fixtures();
    assert!(!fixtures.is_empty());
    for fixture in &fixtures {
        assert_eq!(fixture["model"]["family"], "lifecycle");
        let id = "builder_fixture";
        let source = lifecycle::rewrite_package_declaration(
            fixture["source"].as_str().expect("fixture source"),
            id,
        );
        // Every source the builder writes saves: its action heads are ones
        // the server knows, with values the save accepts.
        let validation =
            validation::validate_lifecycle_rule(&source, id).expect("validator completes");
        assert!(
            validation.valid,
            "{}: {:?}",
            fixture["id"], validation.errors
        );
        let mut engine = runtime::configured_engine(&runtime::RuntimeLimits::lifecycle_defaults());
        engine
            .add_policy(format!("{id}.rego"), source)
            .unwrap_or_else(|error| panic!("{}: {error}", fixture["id"]));
        engine
            .add_policy(
                crate::lifecycle_actions::helper_policy_path(id),
                crate::lifecycle_actions::helper_module_source(id),
            )
            .expect("helper module loads");
        assert_cases(
            fixture,
            engine,
            &format!("{}.{id}", lifecycle::USER_PACKAGE_PREFIX),
        );
    }
}

#[test]
fn scoring_builder_fixtures_validate_and_their_cases_hold_in_the_engine() {
    let fixtures = scoring_fixtures();
    assert!(!fixtures.is_empty());
    for fixture in &fixtures {
        assert_eq!(fixture["model"]["family"], "scoring");
        let id = "builder_fixture";
        let source =
            rewrite_package_declaration(fixture["source"].as_str().expect("fixture source"), id);
        let validation = validation::validate_user_rule(&source, id).expect("validator completes");
        assert!(
            validation.valid,
            "{}: {:?}",
            fixture["id"], validation.errors
        );
        let mut engine = runtime::configured_engine(&runtime::RuntimeLimits::release_defaults());
        engine
            .add_policy(format!("{id}.rego"), source)
            .unwrap_or_else(|error| panic!("{}: {error}", fixture["id"]));
        assert_cases(fixture, engine, &format!("scryer.rules.user.{id}"));
    }
}
