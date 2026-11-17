//! Resolution-weight model and codec-aware size policy regression fixtures.
use super::*;
use crate::acquisition::scoring::RankHead;

const GIB: f64 = 1_073_741_824.0;

fn score(
    raw: &str,
    gib: f64,
    runtime: Option<i32>,
    persona: ScoringPersona,
) -> QualityProfileDecision {
    let mut profile = builtin_4k_profile();
    profile.criteria.scoring_persona = persona.clone();
    let config = test_scoring_config(&persona, &ScoringOverrides::default());
    let release = crate::parse_release_metadata(raw);
    let mut decision =
        score_with_pack_for_category(&profile, &release, false, &config, Some("series"));
    score_size_with_pack_basis(
        &mut decision,
        &release,
        Some((gib * GIB) as i64),
        Some("series"),
        CoverageSizeBasis::single(runtime),
        false,
        &config,
    );
    apply_min_score_gate(&profile, &mut decision);
    decision
}

fn rank(decision: &QualityProfileDecision) -> RankHead {
    RankHead {
        blocked: !decision.allowed,
        side: crate::decision_pipeline::SideFacts::of_decision(Some(decision)),
        negated_revision: 0,
        negated_score: -decision.preference_score,
        size_fit_penalty: decision.size_fit_penalty,
        pipeline: decision.decision_pipeline,
    }
}

#[test]
fn matching_mixed_profile_releases_have_a_100_point_resolution_step() {
    for persona in [
        ScoringPersona::Balanced,
        ScoringPersona::Efficient,
        ScoringPersona::Audiophile,
        ScoringPersona::Compatible,
    ] {
        let low = score(
            "Quiet.Meridian.S06E02.720p.NF.WEB-DL.DDP5.1.x264-NTb",
            1.2,
            Some(60),
            persona.clone(),
        );
        let high = score(
            "Quiet.Meridian.S06E02.1080p.NF.WEB-DL.DDP5.1.x264-NTb",
            2.2,
            Some(60),
            persona.clone(),
        );
        assert!(low.allowed && high.allowed, "{persona:?}");
        assert_eq!(high.release_score - low.release_score, 100, "{persona:?}");
        assert!(
            high.scoring_log
                .iter()
                .filter(|e| e.code.starts_with("size_"))
                .all(|e| e.delta == 0)
        );
        assert!(rank(&high).side.tier_sort_key < rank(&low).side.tier_sort_key);
    }
}

#[test]
fn resolution_weight_model_covers_normal_preferences_without_replacing_tier_order() {
    // Real bundled contributions, varied independently. Subtract the actual
    // 100-point step to model alternatives without maintaining a second scorer.
    let cases = [
        (
            "identical",
            "NF.WEB-DL.DDP5.1.x264-NTb",
            "NF.WEB-DL.DDP5.1.x264-NTb",
        ),
        (
            "codec",
            "NF.WEB-DL.DDP5.1.AV1-NTb",
            "NF.WEB-DL.DDP5.1.x264-NTb",
        ),
        (
            "audio",
            "NF.WEB-DL.TrueHD.Atmos.7.1.x264-NTb",
            "NF.WEB-DL.DDP5.1.x264-NTb",
        ),
        (
            "source",
            "WEB-DL.DDP5.1.x264-PortmereWorks",
            "WEBRip.DDP5.1.x264-PortmereWorks",
        ),
        (
            "group",
            "NF.WEB-DL.DDP5.1.x264-NTb",
            "NF.WEB-DL.DDP5.1.x264-PortmereWorks",
        ),
    ];
    println!("case,lower_preference_advantage,margin_25,margin_100,margin_200,margin_300");
    for (name, low_suffix, high_suffix) in cases {
        let low = score(
            &format!("Quiet.Meridian.S01E01.720p.{low_suffix}"),
            1.2,
            Some(60),
            ScoringPersona::Balanced,
        );
        let high = score(
            &format!("Quiet.Meridian.S01E01.1080p.{high_suffix}"),
            2.2,
            Some(60),
            ScoringPersona::Balanced,
        );
        assert!(low.allowed && high.allowed);
        let advantage = low.release_score - (high.release_score - 100);
        println!(
            "{name},{advantage},{},{},{},{}",
            25 - advantage,
            100 - advantage,
            200 - advantage,
            300 - advantage
        );
        if name != "group" {
            assert!(high.release_score > low.release_score, "{name}");
        }
        // Group reputation can exceed any modest numeric bump. Quality remains
        // the primary ordering dimension even in that deliberately adverse case.
        assert!(rank(&high).side.tier_sort_key < rank(&low).side.tier_sort_key);
    }
}

#[test]
fn plausible_sizes_tie_and_only_outliers_pay_a_ranking_cost() {
    let raw = "Quiet.Meridian.S01E01.1080p.NF.WEB-DL.DDP5.1.x264-NTb";
    let decisions: Vec<_> = [2.2, 3.2, 5.5, 12.0]
        .into_iter()
        .map(|gib| score(raw, gib, Some(60), ScoringPersona::Balanced))
        .collect();
    assert!(decisions.iter().all(|d| d.allowed));
    // Sizes within the bundled rule's Balanced band score alike; above it the
    // rule's oversize penalty applies on top of the ranking cost.
    assert_eq!(decisions[1].release_score, decisions[0].release_score);
    for (d, gib) in decisions.iter().zip([2.2, 3.2, 5.5, 12.0]).skip(2) {
        assert_eq!(
            d.release_score - decisions[0].release_score,
            formula_for(raw, "series", gib, 60, &ScoringPersona::Balanced),
            "{gib}"
        );
    }
    assert!(decisions[..3].iter().all(|d| d.size_fit_penalty == 0));
    assert!(decisions[3].size_fit_penalty > 0);
}

#[test]
fn equivalent_h264_hevc_and_av1_bitrates_have_equal_size_fit() {
    for (codec, factor) in [("H.264", 1.1), ("H.265", 0.75), ("AV1", 0.5)] {
        let raw = format!("Quiet.Meridian.S01E01.1080p.NF.WEB-DL.DDP5.1.{codec}-NTb");
        for relative in [0.75, 1.0, 1.5] {
            let gib = 8.5 * factor * 0.8 * 60.0 * 60.0 / 8.0 / 1024.0 * relative;
            let d = score(&raw, gib, Some(60), ScoringPersona::Balanced);
            assert!(d.allowed, "{codec} {gib}");
            assert_eq!(d.size_fit_penalty, 0, "{codec} {gib}");
        }
    }
}

#[test]
fn extreme_size_limits_use_runtime_codec_and_source() {
    let profile = builtin_4k_profile();
    for (raw, bytes, category, runtime, code) in [
        (
            "Quiet.Meridian.2024.1080p.WEB-DL.H.264",
            100 * 1024 * 1024,
            "movie",
            90,
            "size_implausibly_small_for_quality",
        ),
        (
            "Quiet.Meridian.S01E01.1080p.WEB-DL.AV1",
            100 * 1024 * 1024 * 1024,
            "series",
            20,
            "size_implausible_for_quality",
        ),
    ] {
        let release = crate::parse_release_metadata(raw);
        let mut d = evaluate_profile_requirements(&profile, &release, false, Some(category));
        let before = d.release_score;
        apply_size_requirement(
            &mut d,
            &profile,
            &release,
            Some(bytes),
            Some(category),
            CoverageSizeBasis::single(Some(runtime)),
        );
        assert!(!d.allowed);
        assert!(
            d.block_codes.iter().any(|c| c == code),
            "{:?}",
            d.block_codes
        );
        assert_eq!(before, d.release_score);
    }
}

#[test]
fn uncertain_runtime_size_and_pack_reports_do_not_create_false_lower_rejections() {
    let profile = builtin_4k_profile();
    let release = crate::parse_release_metadata("Quiet.Meridian.S01.1080p.WEB-DL.H.264");
    for basis in [
        CoverageSizeBasis::single(None),
        CoverageSizeBasis::aggregate(Some(540), Some(45), 12),
    ] {
        let mut d = evaluate_profile_requirements(&profile, &release, false, Some("series"));
        apply_size_requirement(
            &mut d,
            &profile,
            &release,
            Some(100 * 1024 * 1024),
            Some("series"),
            basis,
        );
        assert!(d.allowed);
    }
    let unknown = score(
        "Quiet.Meridian.S01E01.1080p.WEB-DL.H.264",
        0.1,
        None,
        ScoringPersona::Balanced,
    );
    assert!(unknown.allowed);
    assert_eq!(unknown.size_fit_penalty, 0);
}

#[test]
fn adding_a_lower_floor_rederives_the_same_tier_bonus_for_both_releases() {
    let release =
        crate::parse_release_metadata("Quiet.Meridian.S01E01.1080p.NF.WEB-DL.DDP5.1.x264-NTb");
    let mut profile = builtin_1080p_profile();
    let a = score_with_pack(&profile, &release, false, &balanced_scoring_config());
    profile.criteria.quality_tiers = vec!["1080P".into()];
    let b = score_with_pack(&profile, &release, false, &balanced_scoring_config());
    assert_eq!(a.release_score - b.release_score, 100);
    let alternate =
        crate::parse_release_metadata("Quiet.Meridian.S01E01.1080p.NF.WEB-DL.DDP5.1.AV1-NTb");
    let alternate_b = score_with_pack(&profile, &alternate, false, &balanced_scoring_config());
    profile.criteria.quality_tiers.push("720P".into());
    let alternate_a = score_with_pack(&profile, &alternate, false, &balanced_scoring_config());
    assert_eq!(
        alternate_a.release_score - a.release_score,
        alternate_b.release_score - b.release_score,
    );
}

#[test]
fn resolution_bonus_counts_standard_steps_above_the_lowest_admitted_quality() {
    for (tiers, expected) in [
        (vec!["2160P", "1080P", "720P"], [0, 100, 200]),
        (vec!["2160P", "1080P"], [0, 0, 100]),
        (vec!["1080P"], [0, 0, 0]),
        (vec!["2160P", "720P"], [0, 0, 200]),
        (vec!["2160P", "1080P", "720P", "720P"], [0, 100, 200]),
        (vec![], [0, 0, 0]),
    ] {
        let mut profile = builtin_4k_profile();
        profile.criteria.quality_tiers = tiers.iter().map(|tier| (*tier).into()).collect();
        for (quality, bonus) in ["720p", "1080p", "2160p"].into_iter().zip(expected) {
            let release = crate::parse_release_metadata(&format!(
                "Quiet.Meridian.S01E01.{quality}.WEB-DL.H.264"
            ));
            let decision = evaluate_profile_requirements(&profile, &release, false, Some("series"));
            let actual: i32 = decision
                .scoring_log
                .iter()
                .filter(|entry| entry.code.starts_with("quality_resolution_"))
                .map(|entry| entry.delta)
                .sum();
            assert_eq!(actual, bonus, "{tiers:?} {quality}");
        }
        let unknown = crate::parse_release_metadata("Quiet.Meridian.S01E01.WEB-DL.H.264");
        let decision = evaluate_profile_requirements(&profile, &unknown, false, Some("series"));
        assert!(
            !decision
                .scoring_log
                .iter()
                .any(|entry| entry.code.starts_with("quality_resolution_"))
        );
    }
}

// ── Oversize penalty in the bundled size rule ───────────────────────────────

/// The size entries the bundled pack emits for one release.
fn size_entries(
    raw: &str,
    category: &str,
    gib: f64,
    basis: CoverageSizeBasis,
    persona: ScoringPersona,
    overrides: ScoringOverrides,
) -> Vec<(String, i32)> {
    let mut profile = QualityProfile::default();
    profile.criteria.scoring_persona = persona;
    profile.criteria.scoring_overrides = overrides;
    let release = crate::parse_release_metadata(raw);
    let decision = evaluate_profile_requirements(&profile, &release, false, Some(category));
    pack_entries(
        &profile,
        &release,
        Some(category),
        Some((gib * GIB) as i64),
        basis,
        &decision,
    )
    .into_iter()
    .filter(|entry| entry.code.starts_with("size_"))
    .map(|entry| (entry.code, entry.delta))
    .collect()
}

/// The summed size delta for a single-member release of known runtime.
fn size_delta(
    raw: &str,
    category: &str,
    gib: f64,
    runtime: i32,
    persona: ScoringPersona,
    overrides: ScoringOverrides,
) -> i32 {
    size_entries(
        raw,
        category,
        gib,
        CoverageSizeBasis::single(Some(runtime)),
        persona,
        overrides,
    )
    .iter()
    .map(|(_, delta)| delta)
    .sum()
}

/// The rule's expected size, computed independently from the parsed release:
/// 1080p bitrate, shared codec factor, source factor and runtime.
fn expected_gib(raw: &str, category: &str, runtime: i32) -> f64 {
    use crate::release_parser::{ReleaseSource, VideoCodec};
    let release = crate::parse_release_metadata(raw);
    assert_eq!(
        release
            .quality
            .as_deref()
            .map(str::to_ascii_uppercase)
            .as_deref(),
        Some("1080P"),
        "{raw}"
    );
    let bitrate = if category == "movie" { 9.1 } else { 8.5 };
    let codec = match release.video_codec {
        Some(VideoCodec::H264) => 1.1,
        Some(VideoCodec::H265) => 0.75,
        Some(VideoCodec::Av1) => 0.5,
        other => panic!("{raw}: unexpected codec {other:?}"),
    };
    let source = match release.source {
        Some(ReleaseSource::WebDl) => 0.8,
        Some(ReleaseSource::BluRay) => 1.35,
        other => panic!("{raw}: unexpected source {other:?}"),
    };
    assert!(!release.is_remux && !release.is_bd_disk, "{raw}");
    (bitrate * codec * source * f64::from(runtime) * 60.0 / 8.0 / 1024.0).max(0.5)
}

/// The penalty formula: nothing up to `high`, then a logarithmic penalty
/// floored at -1200.
fn formula(ratio: f64, persona: &ScoringPersona) -> i32 {
    let (high, weight) = match persona {
        ScoringPersona::Balanced => (1.25, 1200.0),
        ScoringPersona::Efficient => (1.1, 1400.0),
        _ => return 0,
    };
    if ratio <= high {
        0
    } else {
        (-(weight * (ratio / high).ln()).round() as i32).max(-1200)
    }
}

fn formula_for(raw: &str, category: &str, gib: f64, runtime: i32, persona: &ScoringPersona) -> i32 {
    let bytes = (gib * GIB) as i64;
    formula(
        bytes as f64 / GIB / expected_gib(raw, category, runtime),
        persona,
    )
}

const ANIME_WEB_H264: &str = "Quiet.Meridian.S01E01.1080p.WEB-DL.x264-FICTGRP";
const ANIME_WEB_H265: &str = "Quiet.Meridian.S01E01.1080p.WEB-DL.x265-FICTGRP";
const ANIME_WEB_AV1: &str = "Quiet.Meridian.S01E01.1080p.WEB-DL.AV1-FICTGRP";
const ANIME_BD_H265: &str = "Quiet.Meridian.S01E01.1080p.BluRay.x265-FICTGRP";
const ANIME_BD_H264: &str = "Quiet.Meridian.S01E01.1080p.BluRay.x264-FICTGRP";
const MOVIE_BD_H264: &str = "Quiet.Meridian.2024.1080p.BluRay.x264-FICTGRP";

const PENALTY_CASES: [(&str, &str, &str, i32); 7] = [
    ("anime WEB-DL H.264", ANIME_WEB_H264, "anime", 24),
    ("anime WEB-DL H.265", ANIME_WEB_H265, "anime", 24),
    ("anime WEB-DL AV1", ANIME_WEB_AV1, "anime", 24),
    ("anime BluRay H.265", ANIME_BD_H265, "anime", 24),
    ("anime BluRay H.264", ANIME_BD_H264, "anime", 24),
    ("series WEB-DL H.264 45m", ANIME_WEB_H264, "series", 45),
    ("movie BluRay H.264 120m", MOVIE_BD_H264, "movie", 120),
];

const ALL_PERSONAS: [ScoringPersona; 4] = [
    ScoringPersona::Balanced,
    ScoringPersona::Efficient,
    ScoringPersona::Audiophile,
    ScoringPersona::Compatible,
];

#[test]
fn oversize_penalty_follows_the_formula_for_every_persona_and_category() {
    println!("persona,case,1.0,1.5,2,3,4,6,8");
    for persona in ALL_PERSONAS {
        for (name, raw, category, runtime) in PENALTY_CASES {
            let mut row = Vec::new();
            for gib in [1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0] {
                let actual = size_delta(
                    raw,
                    category,
                    gib,
                    runtime,
                    persona.clone(),
                    ScoringOverrides::default(),
                );
                let expected = formula_for(raw, category, gib, runtime, &persona);
                assert_eq!(actual, expected, "{persona:?} {name} {gib} GiB");
                row.push(actual.to_string());
            }
            println!("{persona:?},{name},{}", row.join(","));
        }
    }
}

#[test]
fn an_in_band_release_scores_zero_size_points_on_every_persona() {
    for persona in ALL_PERSONAS {
        for (name, raw, category, runtime) in PENALTY_CASES {
            let gib = expected_gib(raw, category, runtime);
            let entries = size_entries(
                raw,
                category,
                gib,
                CoverageSizeBasis::single(Some(runtime)),
                persona.clone(),
                ScoringOverrides::default(),
            );
            assert_eq!(entries.len(), 1, "{persona:?} {name}: {entries:?}");
            assert_eq!(entries[0].1, 0, "{persona:?} {name}: {entries:?}");
        }
    }
}

#[test]
fn oversized_anime_episode_sanity_targets_on_balanced() {
    let balanced = |raw: &str, gib: f64| {
        size_delta(
            raw,
            "anime",
            gib,
            24,
            ScoringPersona::Balanced,
            ScoringOverrides::default(),
        )
    };
    assert_eq!(balanced(ANIME_WEB_H264, 1.4), 0);
    for (raw, gib) in [(ANIME_BD_H265, 4.0), (ANIME_BD_H264, 4.0)] {
        let expected = formula_for(raw, "anime", gib, 24, &ScoringPersona::Balanced);
        assert!(expected < 0, "{raw}");
        assert_eq!(balanced(raw, gib), expected, "{raw}");
    }
    assert_eq!(balanced(ANIME_BD_H265, 8.0), -1200);
}

#[test]
fn the_oversize_penalty_is_floored() {
    for persona in [ScoringPersona::Balanced, ScoringPersona::Efficient] {
        for gib in [8.0, 40.0, 400.0] {
            assert_eq!(
                size_delta(
                    ANIME_BD_H265,
                    "anime",
                    gib,
                    24,
                    persona.clone(),
                    ScoringOverrides::default()
                ),
                -1200,
                "{persona:?} {gib}"
            );
        }
    }
}

#[test]
fn the_compact_curve_is_never_gentler_than_balanced() {
    let compact = |persona: ScoringPersona, prefer: Option<bool>, gib: f64| {
        size_delta(
            ANIME_WEB_H264,
            "anime",
            gib,
            24,
            persona,
            ScoringOverrides {
                prefer_compact_encodes: prefer,
                ..ScoringOverrides::default()
            },
        )
    };
    let expected = expected_gib(ANIME_WEB_H264, "anime", 24);
    let mut penalized = 0;
    for step in 0..=80 {
        let gib = expected * (1.0 + f64::from(step) * 0.05);
        let balanced = compact(ScoringPersona::Balanced, None, gib);
        let efficient = compact(ScoringPersona::Efficient, None, gib);
        assert!(efficient <= balanced, "{gib} GiB: {efficient} > {balanced}");
        if efficient < 0 {
            penalized += 1;
        }
        // The override picks the curve: compact on Balanced, balanced on
        // Efficient, and nothing on the personas without a penalty.
        assert_eq!(
            compact(ScoringPersona::Balanced, Some(true), gib),
            efficient
        );
        assert_eq!(
            compact(ScoringPersona::Efficient, Some(false), gib),
            balanced
        );
        assert_eq!(
            compact(ScoringPersona::Efficient, Some(true), gib),
            efficient
        );
        assert_eq!(compact(ScoringPersona::Audiophile, Some(true), gib), 0);
        assert_eq!(compact(ScoringPersona::Compatible, Some(true), gib), 0);
    }
    assert!(penalized > 70);
}

#[test]
fn for_equal_bytes_a_more_efficient_codec_is_penalized_at_least_as_hard() {
    for persona in [ScoringPersona::Balanced, ScoringPersona::Efficient] {
        for (h264, h265, av1) in [
            (ANIME_WEB_H264, ANIME_WEB_H265, ANIME_WEB_AV1),
            (
                ANIME_BD_H264,
                ANIME_BD_H265,
                "Quiet.Meridian.S01E01.1080p.BluRay.AV1-FICTGRP",
            ),
        ] {
            let mut any = false;
            for tenth in 5..=120 {
                let gib = f64::from(tenth) / 10.0;
                let delta = |raw| {
                    size_delta(
                        raw,
                        "anime",
                        gib,
                        24,
                        persona.clone(),
                        ScoringOverrides::default(),
                    )
                };
                let (a, b, c) = (delta(h264), delta(h265), delta(av1));
                assert!(b <= a, "{persona:?} {gib} GiB: H.265 {b} > H.264 {a}");
                assert!(c <= b, "{persona:?} {gib} GiB: AV1 {c} > H.265 {b}");
                any |= c < b || b < a;
            }
            assert!(any, "{persona:?} {h264}");
        }
    }
}

#[test]
fn unknown_runtime_and_member_basis_carry_no_oversize_penalty() {
    for persona in ALL_PERSONAS {
        let unknown = size_entries(
            ANIME_BD_H265,
            "anime",
            12.0,
            CoverageSizeBasis::single(None),
            persona.clone(),
            ScoringOverrides::default(),
        );
        assert!(!unknown.is_empty(), "{persona:?}");
        assert!(
            unknown.iter().all(|(_, d)| *d == 0),
            "{persona:?} {unknown:?}"
        );

        // Twelve 45-minute episodes listed at about twice one episode's
        // expected size: read as one member, which carries no points.
        let member = size_entries(
            ANIME_WEB_H264,
            "series",
            5.0,
            CoverageSizeBasis::aggregate(Some(540), Some(45), 12),
            persona.clone(),
            ScoringOverrides::default(),
        );
        assert!(
            member
                .iter()
                .any(|(code, _)| code == "size_pack_member_basis"),
            "{persona:?} {member:?}"
        );
        assert!(
            member.iter().all(|(_, d)| *d == 0),
            "{persona:?} {member:?}"
        );
    }
}
