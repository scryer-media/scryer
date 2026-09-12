//! Frozen native expectations survive removal of the migration oracle.

use super::pack_test_support::pack_entries;
use crate::quality_profile::{
    CoverageSizeBasis, QualityProfile, apply_size_requirement, evaluate_profile_requirements,
};
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
        assert_eq!(
            actual,
            case.entries,
            "profile={} title={} size={:?} basis={:?}",
            case.profile,
            case.title,
            case.size_bytes,
            case.basis()
        );
    }
}

#[test]
fn mandatory_size_bound_matches_frozen_expectations_without_numeric_entries() {
    let golden: Golden =
        serde_json::from_str(include_str!("fixtures/trash-size-guards.json")).unwrap();
    assert_eq!(golden.cases.len(), 1728);
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
        assert_eq!(
            decision.block_codes,
            case.block_codes,
            "profile={} title={} size={:?} basis={:?}",
            case.profile,
            case.title,
            case.size_bytes,
            case.basis()
        );
        assert!(decision.scoring_log.iter().all(|entry| entry.delta == 0));
    }
}
