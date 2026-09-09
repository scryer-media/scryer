//! Exact byte-boundary parity for the bundled TRaSH size template.
//!
//! The legacy size scorer remains the oracle while this migration is being
//! verified.  The template deliberately does not own the host's implausible
//! size veto, so this test compares only the emitted size contributions.

use std::collections::{BTreeMap, HashSet};

use crate::quality_profile::{
    CoverageSizeBasis, QualityProfileDecision,
    apply_size_scoring_for_category_with_remux_preference,
};
use crate::rules::builtin_trash::verified_pack;
use crate::rules::user_rule_input::{ReleaseRuntimeInfo, RuleContextInfo, build_rule_input};
use crate::scoring_weights::{ScoringOverrides, ScoringPersona, build_weights_for_category};
use crate::{QualityProfile, parse_release_metadata};

const GIB: f64 = 1_073_741_824.0;

#[derive(Clone, Copy)]
struct Case {
    category: &'static str,
    basis: CoverageSizeBasis,
}

fn empty_decision() -> QualityProfileDecision {
    QualityProfileDecision {
        release_score: 0,
        scoring_log: vec![],
        allowed: true,
        block_codes: vec![],
        preference_score: 0,
        tier_index: None,
    }
}

fn size_engine() -> scryer_rules::UserRulesEngine {
    let pack = verified_pack().expect("bundled TRaSH pack parses");
    let template = pack
        .templates
        .iter()
        .find(|template| template.id == "trash-guides-size")
        .expect("bundled TRaSH size template exists");
    let id = "trash_size_parity".to_string();
    let policy = scryer_rules::UserPolicy {
        rego_source: scryer_rules::rewrite_package_declaration(&template.rego_source, &id),
        id: id.clone(),
        name: template.title.clone(),
        origin: scryer_rules::PolicyOrigin::User,
        applied_facets: vec![],
    };
    scryer_rules::UserRulesEngine::build_with_baseline_rules(&[policy], &HashSet::from([id]))
        .expect("bundled TRaSH size template builds")
}

fn profile(persona: ScoringPersona) -> QualityProfile {
    let mut profile = QualityProfile::parse(
        r#"{"id":"size-parity","name":"Size parity","criteria":{"quality_tiers":["1080P"],"allow_unknown_quality":true}}"#,
    )
    .expect("test profile parses");
    profile.criteria.scoring_persona = persona;
    profile.criteria.scoring_overrides = ScoringOverrides::default();
    profile
}

fn release_title(category: &str, codec: &str) -> String {
    let episode = if category == "movie" {
        "2024"
    } else {
        "S01E01"
    };
    format!("Size.Curve.{episode}.1080p.WEB-DL.{codec}-GRP")
}

fn expected_gib(category: &str, codec: &str, runtime_minutes: Option<i32>) -> f64 {
    let bitrate = match category {
        "movie" => 9.1,
        "series" | "anime" => 8.5,
        _ => unreachable!("test categories are fixed"),
    };
    let codec_factor = match codec {
        "H.264" => 1.1,
        "H.265" => 0.75,
        _ => unreachable!("test codecs are fixed"),
    };
    let runtime = runtime_minutes.unwrap_or(match category {
        "movie" => 120,
        "series" => 45,
        "anime" => 24,
        _ => unreachable!("test categories are fixed"),
    });
    // The corpus uses WEB-DL sources, whose shared source factor is 0.8.
    (bitrate * codec_factor * 0.8 * f64::from(runtime) * 60.0 / 8.0 / 1024.0).max(0.5)
}

fn boundary_ratios(category: &str) -> &'static [f64] {
    match category {
        "movie" => &[4.0, 2.4, 1.8, 1.35, 1.0, 0.75, 0.55, 0.35, 0.1],
        "series" => &[4.0, 2.4, 1.8, 1.35, 1.0, 0.75, 0.55, 0.35, 0.04],
        "anime" => &[2.5, 2.1, 1.6, 1.2, 0.85, 0.65, 0.5, 0.3, 0.04],
        _ => unreachable!("test categories are fixed"),
    }
}

fn boundary_bytes(category: &str, codec: &str, runtime: Option<i32>, ratio: f64) -> [i64; 3] {
    let boundary = (ratio * expected_gib(category, codec, runtime) * GIB).round() as i64;
    [boundary - 1, boundary, boundary + 1]
}

fn expected_entries(
    parsed: &crate::ParsedReleaseMetadata,
    profile: &QualityProfile,
    case: Case,
    size_bytes: Option<i64>,
) -> BTreeMap<String, i32> {
    let weights = build_weights_for_category(
        &profile.criteria.scoring_persona,
        &profile.criteria.scoring_overrides,
        Some(case.category),
    );
    let mut decision = empty_decision();
    apply_size_scoring_for_category_with_remux_preference(
        &mut decision,
        parsed,
        size_bytes,
        Some(case.category),
        case.basis,
        profile.criteria.prefer_remux,
        &weights,
    );
    decision
        .scoring_log
        .into_iter()
        .filter(|entry| entry.kind == crate::quality_profile::ScoringEntryKind::ScoreContribution)
        .map(|entry| (entry.code, entry.delta))
        .collect()
}

fn actual_entries(
    engine: &scryer_rules::UserRulesEngine,
    parsed: &crate::ParsedReleaseMetadata,
    profile: &QualityProfile,
    case: Case,
    size_bytes: Option<i64>,
) -> (BTreeMap<String, i32>, bool) {
    let input = build_rule_input(
        parsed,
        profile,
        &empty_decision(),
        ReleaseRuntimeInfo {
            size_bytes,
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
            category: Some(case.category),
            original_language: Some(if case.category == "anime" { "ja" } else { "en" }),
            original_country: None,
            title_tags: &[],
            has_existing_file: false,
            existing_score: None,
            search_mode: "canonical",
            runtime_minutes: case.basis.total_runtime_minutes,
            coverage_total_runtime_minutes: case.basis.total_runtime_minutes,
            coverage_member_runtime_minutes: case.basis.member_runtime_minutes,
            coverage_member_count: Some(case.basis.member_count),
            is_filler: false,
        },
        None,
    );
    let result = engine
        .evaluator()
        .evaluate(&input, case.category)
        .expect("size template evaluates");
    (
        result
            .entries
            .iter()
            .map(|entry| (entry.code.clone(), entry.delta))
            .collect(),
        result.errors.is_empty(),
    )
}

#[test]
#[ignore = "explicit bundled size-template parity; 2,016 deterministic boundary fixtures"]
fn bundled_size_curve_matches_legacy_oracle_at_exact_byte_boundaries() {
    let engine = size_engine();
    let personas = [
        ScoringPersona::Balanced,
        ScoringPersona::Audiophile,
        ScoringPersona::Efficient,
        ScoringPersona::Compatible,
    ];
    let bases = [
        (CoverageSizeBasis::default(), None),
        (CoverageSizeBasis::single(Some(45)), Some(45)),
        (
            CoverageSizeBasis::aggregate(Some(288), Some(24), 12),
            Some(24),
        ),
    ];
    let mut fixtures = 0;
    let mut mismatches = Vec::new();

    for persona in personas {
        let profile = profile(persona.clone());
        for category in ["movie", "series", "anime"] {
            for codec in ["H.264", "H.265"] {
                let parsed = parse_release_metadata(&release_title(category, codec));
                for (basis, member_runtime) in bases {
                    let case = Case { category, basis };
                    // Missing sizes must remain absent from both implementations.
                    fixtures += 1;
                    let expected = expected_entries(&parsed, &profile, case, None);
                    let (actual, no_errors) =
                        actual_entries(&engine, &parsed, &profile, case, None);
                    if actual != expected || !no_errors {
                        mismatches.push(format!(
                            "{persona:?}/{category}/{codec}/basis={basis:?}/size=None: expected={expected:?}, actual={actual:?}, no_errors={no_errors}"
                        ));
                    }

                    // Aggregate coverage uses the member reading when its total reading
                    // is below the curve.  The other bases exercise the total reading.
                    let runtime = if basis.covers_multiple_members() {
                        member_runtime
                    } else {
                        basis.total_runtime_minutes
                    };
                    for &ratio in boundary_ratios(category) {
                        for size_bytes in boundary_bytes(category, codec, runtime, ratio) {
                            fixtures += 1;
                            let expected =
                                expected_entries(&parsed, &profile, case, Some(size_bytes));
                            let (actual, errors) =
                                actual_entries(&engine, &parsed, &profile, case, Some(size_bytes));
                            if actual != expected || !errors {
                                mismatches.push(format!(
                                    "{persona:?}/{category}/{codec}/basis={basis:?}/ratio={ratio}/size={size_bytes}: expected={expected:?}, actual={actual:?}, no_errors={errors}"
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    assert_eq!(fixtures, 2_016, "fixture matrix changed unexpectedly");
    assert!(
        mismatches.is_empty(),
        "{}/{} size-curve parity mismatches:\n{}",
        mismatches.len(),
        fixtures,
        mismatches
            .into_iter()
            .take(20)
            .collect::<Vec<_>>()
            .join("\n")
    );
}
