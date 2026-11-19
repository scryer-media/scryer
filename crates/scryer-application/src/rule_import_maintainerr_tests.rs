use super::*;
use crate::lifecycle_rules::maintainerr::{MaintainerrTranslateContext, codes};
use serde_json::Value;

const LIVE_RULES: &str = include_str!("lifecycle_rules/maintainerr/fixtures/live-shape-rules.json");
const LIVE_CONSTANTS: &str =
    include_str!("lifecycle_rules/maintainerr/fixtures/live-shape-constants.json");
const LIVE_RADARR_PROFILES: &str =
    include_str!("lifecycle_rules/maintainerr/fixtures/live-shape-radarr-profiles.json");
const LIVE_SONARR_PROFILES: &str =
    include_str!("lifecycle_rules/maintainerr/fixtures/live-shape-sonarr-profiles.json");

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).expect("fixture parses")
}

fn fetched() -> FetchedMaintainerrRules {
    FetchedMaintainerrRules {
        version: "2.19.0".into(),
        rule_groups: parsed(LIVE_RULES)
            .as_array()
            .expect("groups are an array")
            .clone(),
        constants: parsed(LIVE_CONSTANTS),
        usernames: None,
        radarr_profiles: vec![(1, parsed(LIVE_RADARR_PROFILES))],
        sonarr_profiles: vec![(1, parsed(LIVE_SONARR_PROFILES))],
    }
}

fn plan_context() -> RuleImportPlanContext {
    RuleImportPlanContext {
        quality_profile_names: ["HD-1080p".to_string(), "SD".to_string()].into(),
        usernames: ["sample-user".to_string()].into(),
    }
}

fn lifecycle(item: &PlannedRuleImportItem) -> &PlannedMaintainerrLifecycleRule {
    match &item.translation {
        PlannedRuleImportTranslation::MaintainerrLifecycleRule(rule) => rule,
        other => panic!("not a lifecycle rule: {other:?}"),
    }
}

#[test]
fn every_group_plans_one_lifecycle_rule_matching_the_translator() {
    let planned = plan_maintainerr_rule_import(&fetched(), &plan_context()).expect("plans");
    let mut ctx = MaintainerrTranslateContext {
        scryer_quality_profile_names: plan_context().quality_profile_names,
        scryer_usernames: plan_context().usernames,
        ..MaintainerrTranslateContext::default()
    };
    ctx.add_arr_profiles(ArrKind::Radarr, 1, LIVE_RADARR_PROFILES)
        .expect("radarr profiles");
    ctx.add_arr_profiles(ArrKind::Sonarr, 1, LIVE_SONARR_PROFILES)
        .expect("sonarr profiles");
    let expected = crate::lifecycle_rules::maintainerr::translate_maintainerr_rule_groups(
        LIVE_RULES,
        LIVE_CONSTANTS,
        &ctx,
    )
    .expect("translates");
    assert_eq!(planned.len(), expected.len());
    for (item, rule) in planned.iter().zip(&expected) {
        let plan = lifecycle(item);
        assert_eq!(item.key, format!("rulegroup:{}", rule.source_group_id));
        assert_eq!(plan.name, rule.name);
        assert_eq!(plan.rego_source, rule.rego_source);
        assert_eq!(plan.grace_days, rule.grace_days);
        assert_eq!(plan.max_titles_per_run, rule.max_titles_per_run);
        assert_eq!(plan.facts.len(), rule.diagnostics.len());
        let display = &item.source_name;
        assert_eq!(
            plan.taken_names,
            [
                format!("(Maintainerr) {display}"),
                format!("(Maintainerr) FAILED - {display}")
            ]
        );
        match rule.status {
            MaintainerrRuleStatus::Translated => {
                assert_eq!(plan.state, RuleImportItemState::Saved);
                assert_eq!(plan.reason, None);
            }
            MaintainerrRuleStatus::Failed => {
                assert_eq!(plan.state, RuleImportItemState::SavedFailedTranslation);
                assert_eq!(plan.reason, Some(reasons::GROUP_UNTRANSLATABLE));
            }
        }
    }
    assert_eq!(planned[0].source_name, "Sample Rule Alpha");
}

#[test]
fn diagnostics_become_facts_with_severity_and_rule_index() {
    let planned = plan_maintainerr_rule_import(&fetched(), &plan_context()).expect("plans");
    let failed = planned
        .iter()
        .map(lifecycle)
        .find(|rule| rule.state == RuleImportItemState::SavedFailedTranslation)
        .expect("the fixture has a failed group");
    let unmapped = failed
        .facts
        .iter()
        .find(|fact| fact.code == codes::PROPERTY_UNMAPPED)
        .expect("an unmapped property");
    let details = unmapped.details.as_ref().expect("details");
    assert_eq!(details["severity"], "ERROR");
    assert!(details["ruleIndex"].is_u64());

    let warned = lifecycle(&planned[0]);
    let approximate = warned
        .facts
        .iter()
        .find(|fact| fact.code == codes::FACT_APPROXIMATE)
        .expect("a warning");
    assert_eq!(
        approximate.details.as_ref().expect("details")["severity"],
        "WARNING"
    );
}

#[test]
fn unreadable_sources_are_refused_with_their_codes() {
    let mut bad_profiles = fetched();
    bad_profiles.radarr_profiles = vec![(1, serde_json::json!({"not": "a list"}))];
    assert_eq!(
        plan_maintainerr_rule_import(&bad_profiles, &plan_context())
            .expect_err("profiles are unreadable")
            .validation_reason(),
        Some(crate::error_reasons::maintainerr_import::PROFILES_JSON_INVALID)
    );

    let mut too_many = fetched();
    too_many.rule_groups = vec![serde_json::json!({"id": 1}); MAX_ITEMS + 1];
    assert_eq!(
        plan_maintainerr_rule_import(&too_many, &plan_context())
            .expect_err("too many groups")
            .validation_reason(),
        Some(refusal::TOO_MANY_ITEMS)
    );
}

#[test]
fn remote_text_reaching_a_name_or_description_is_cleaned() {
    let mut source = fetched();
    let group = source.rule_groups[0]
        .as_object_mut()
        .expect("group is an object");
    group.insert(
        "name".into(),
        Value::String("Sample\u{0}Rule\u{2028}Alpha\u{7}".into()),
    );
    group.insert(
        "description".into(),
        Value::String("Line one\r\nLine\u{0} two\u{2029}end".into()),
    );
    let planned = plan_maintainerr_rule_import(&source, &plan_context()).expect("plans");
    let rule = lifecycle(&planned[0]);
    assert_eq!(planned[0].source_name, "Sample Rule Alpha");
    assert_eq!(rule.name, "(Maintainerr) Sample Rule Alpha");
    assert!(
        rule.description.ends_with("\n\nLine one\nLine  two end"),
        "{:?}",
        rule.description
    );
    assert!(
        !rule
            .description
            .chars()
            .any(|c| (c.is_control() && c != '\n') || matches!(c, '\u{2028}' | '\u{2029}'))
    );
}

#[test]
fn reasons_are_stable_distinct_codes() {
    let mut seen = std::collections::HashSet::new();
    for code in reasons::ALL {
        assert!(code.starts_with("RULE_IMPORT_"));
        assert!(!refusal::ALL.contains(code));
        assert!(!crate::rule_import_arr::reasons::ALL.contains(code));
        assert!(seen.insert(code));
    }
}
