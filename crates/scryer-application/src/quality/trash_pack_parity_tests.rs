//! Immutable pre-migration scores evaluated through the real parser and input builder.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::quality_profile::{CoverageSizeBasis, QualityProfileDecision};
use crate::rules::user_rule_input::{ReleaseRuntimeInfo, RuleContextInfo, build_rule_input};
use crate::{QualityProfile, parse_release_metadata};

#[derive(Deserialize)]
struct Golden {
    titles: Vec<String>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    persona: crate::scoring_weights::ScoringPersona,
    overrides: crate::scoring_weights::ScoringOverrides,
    category: String,
    title: usize,
    size_bytes: Option<i64>,
    entries: Vec<(String, i32)>,
}

#[test]
#[ignore = "explicit generated-pack parity; requires SCRYER_TRASH_PACK"]
fn trash_pack_matches_native_numeric_golden() {
    let path = std::env::var_os("SCRYER_TRASH_PACK").expect("pack path required");
    let pack: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let policies = pack["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rule| rule["evaluationPhase"] == "baseline")
        .enumerate()
        .map(|(index, rule)| {
            let id = format!("trash_parity_{index}");
            scryer_rules::UserPolicy {
                rego_source: scryer_rules::rewrite_package_declaration(
                    rule["regoSource"].as_str().unwrap(),
                    &id,
                ),
                id,
                name: rule["id"].as_str().unwrap().into(),
                origin: scryer_rules::PolicyOrigin::User,
                applied_facets: vec![],
            }
        })
        .collect::<Vec<_>>();
    assert!(!policies.is_empty());
    let ids = policies.iter().map(|policy| policy.id.clone()).collect();
    let engine = scryer_rules::UserRulesEngine::build_with_baseline_rules(&policies, &ids).unwrap();
    let mut evaluator = engine.evaluator();
    let golden: Golden =
        serde_json::from_str(include_str!("fixtures/trash-legacy-numeric.json")).unwrap();
    let empty_decision = QualityProfileDecision {
        release_score: 0,
        scoring_log: vec![],
        allowed: true,
        block_codes: vec![],
        preference_score: 0,
        tier_index: None,
    };
    let mut mismatches = 0;
    let mut samples = Vec::new();
    let mut differences = BTreeMap::<String, usize>::new();
    // Keep the native numeric oracle immutable. The reviewed contract overlay
    // records only the former mandatory DV rejection becoming recoverable.
    let contract: Value =
        serde_json::from_str(include_str!("fixtures/trash-recoverable-expectations.json")).unwrap();
    let mut additions: BTreeMap<usize, BTreeMap<String, i32>> =
        serde_json::from_value(contract["cases"].clone()).unwrap();
    assert_eq!(additions.len(), 48);
    for (case_index, case) in golden.cases.iter().enumerate() {
        let mut profile = QualityProfile::parse(r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#).unwrap();
        profile.criteria.scoring_persona = case.persona.clone();
        profile.criteria.scoring_overrides = case.overrides.clone();
        profile.criteria.allow_unknown_quality = true;
        profile.criteria.prefer_remux = case.title % 3 == 0;
        profile.criteria.atmos_preferred = case.title % 3 == 1;
        profile.criteria.prefer_dual_audio = case.title % 3 == 2;
        let basis = match case.title % 3 {
            0 => CoverageSizeBasis::default(),
            1 => CoverageSizeBasis::single(Some(90)),
            _ => CoverageSizeBasis::aggregate(Some(288), Some(24), 12),
        };
        let parsed = parse_release_metadata(&golden.titles[case.title]);
        let input = build_rule_input(
            &parsed,
            &profile,
            &empty_decision,
            ReleaseRuntimeInfo {
                size_bytes: case.size_bytes,
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some(&case.category),
                original_language: Some(if case.category == "anime" { "ja" } else { "en" }),
                original_country: None,
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "canonical",
                runtime_minutes: basis.total_runtime_minutes,
                coverage_total_runtime_minutes: basis.total_runtime_minutes,
                coverage_member_runtime_minutes: basis.member_runtime_minutes,
                coverage_member_count: Some(basis.member_count),
                is_filler: false,
            },
            None,
        );
        let result = evaluator.evaluate(&input, &case.category).unwrap();
        let actual = result
            .entries
            .iter()
            .map(|entry| (entry.code.clone(), entry.delta))
            .collect::<BTreeMap<_, _>>();
        let mut expected = case.entries.iter().cloned().collect::<BTreeMap<_, _>>();
        if let Some(entries) = additions.remove(&case_index) {
            for (code, delta) in entries {
                assert!(
                    expected.insert(code, delta).is_none(),
                    "contract additions must not replace native numeric expectations"
                );
            }
        }
        if actual != expected || !result.errors.is_empty() || actual.len() != result.entries.len() {
            mismatches += 1;
            for code in actual
                .keys()
                .chain(expected.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                if actual.get(code) != expected.get(code) {
                    *differences
                        .entry(format!(
                            "{code}: {:?} -> {:?}",
                            expected.get(code),
                            actual.get(code)
                        ))
                        .or_default() += 1;
                }
            }
            if samples.len() < 12 {
                samples.push(format!("{:?}/{}/title={}/size={:?}: errors={:?}; expected={expected:?}; actual={actual:?}",
                    case.persona, case.category, case.title, case.size_bytes, result.errors));
            }
        }
    }
    assert!(
        additions.is_empty(),
        "every reviewed case must be evaluated"
    );
    assert_eq!(
        mismatches,
        0,
        "{mismatches}/{} mismatches\nDifferences: {differences:#?}\n{}",
        golden.cases.len(),
        samples.join("\n")
    );
}
