//! Frozen native expectations survive removal of the migration oracle.

use super::pack_test_support::pack_entries;
use crate::quality_profile::{
    CoverageSizeBasis, QualityProfile, apply_size_requirement, evaluate_profile_requirements,
};
use crate::scoring_weights::ScoringPersona;
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
struct Golden {
    profiles: Vec<serde_json::Value>,
    cases: Vec<Case>,
}

#[derive(serde::Deserialize)]
struct Case {
    profile: usize,
    title: String,
    category: String,
    total_runtime: Option<i32>,
    member_runtime: Option<i32>,
    member_count: i32,
    size_bytes: Option<i64>,
    #[serde(default)]
    entries: BTreeMap<String, i32>,
    #[serde(default)]
    block_codes: Vec<String>,
}

impl Case {
    fn basis(&self) -> CoverageSizeBasis {
        CoverageSizeBasis {
            total_runtime_minutes: self.total_runtime,
            member_runtime_minutes: self.member_runtime,
            member_count: self.member_count,
        }
    }
}

#[test]
#[ignore = "explicit 2,016-case frozen size-boundary corpus"]
fn bundled_size_curve_matches_frozen_byte_boundaries() {
    let golden: Golden =
        serde_json::from_str(include_str!("fixtures/trash-size-boundaries.json")).unwrap();
    assert_eq!(golden.cases.len(), 2016);
    for case in &golden.cases {
        let profile = QualityProfile::parse(&golden.profiles[case.profile].to_string()).unwrap();
        let release = crate::parse_release_metadata(&case.title);
        let decision =
            evaluate_profile_requirements(&profile, &release, false, Some(&case.category));
        let entries = pack_entries(
            &profile,
            &release,
            Some(&case.category),
            case.size_bytes,
            case.basis(),
            &decision,
        );
        let size_entries = entries
            .iter()
            .filter(|entry| entry.code.starts_with("size_"))
            .collect::<Vec<_>>();
        let actual = size_entries
            .iter()
            .map(|entry| (entry.code.clone(), entry.delta))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            actual.len(),
            size_entries.len(),
            "duplicate size contributions"
        );
        let context = format!(
            "profile={} title={} size={:?} basis={:?} actual={actual:?}",
            case.profile,
            case.title,
            case.size_bytes,
            case.basis()
        );
        // The band codes are frozen. Only an oversize penalty may move a delta
        // off zero: Balanced or Efficient curve, known runtime, whole-release
        // basis.
        assert!(
            actual.keys().eq(case.entries.keys()),
            "{context} expected={:?}",
            case.entries
        );
        let penalized_persona = match profile.criteria.scoring_persona {
            ScoringPersona::Balanced | ScoringPersona::Compatible | ScoringPersona::Efficient => {
                true
            }
            ScoringPersona::Audiophile => false,
        };
        let may_penalize = penalized_persona
            && case.total_runtime.is_some_and(|minutes| minutes > 0)
            && !actual.contains_key("size_pack_member_basis");
        for delta in actual.values() {
            assert!(
                *delta == 0 || (may_penalize && (-1200..0).contains(delta)),
                "{context}"
            );
        }
    }
}

#[test]
fn broad_size_guard_relaxes_historical_upper_rejections_without_numeric_entries() {
    let golden: Golden =
        serde_json::from_str(include_str!("fixtures/trash-size-guards.json")).unwrap();
    assert_eq!(golden.cases.len(), 1728);
    let mut relaxed = 0;
    for case in &golden.cases {
        let profile = QualityProfile::parse(&golden.profiles[case.profile].to_string()).unwrap();
        let release = crate::parse_release_metadata(&case.title);
        let mut decision =
            evaluate_profile_requirements(&profile, &release, false, Some(&case.category));
        apply_size_requirement(
            &mut decision,
            &profile,
            &release,
            case.size_bytes,
            Some(&case.category),
            case.basis(),
        );
        let upper_blocked = decision
            .block_codes
            .iter()
            .any(|code| code == "size_implausible_for_quality");
        let old_upper_blocked = case
            .block_codes
            .iter()
            .any(|code| code == "size_implausible_for_quality");
        relaxed += usize::from(old_upper_blocked && !upper_blocked);
        assert!(
            !upper_blocked || old_upper_blocked,
            "profile={} title={} size={:?} basis={:?}",
            case.profile,
            case.title,
            case.size_bytes,
            case.basis()
        );
        assert!(
            decision
                .scoring_log
                .iter()
                .filter(|entry| entry.code.starts_with("size_"))
                .all(|entry| entry.delta == 0)
        );
    }
    assert!(
        relaxed > 0,
        "the corpus must exercise the overly strict old upper gate"
    );
}
