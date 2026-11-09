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
        tier_index: decision.tier_index.unwrap_or(usize::MAX),
        negated_revision: 0,
        negated_score: -decision.preference_score,
        size_fit_penalty: decision.size_fit_penalty,
        language: decision.language,
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
        assert!(rank(&high).tier_index < rank(&low).tier_index);
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
        assert!(rank(&high).tier_index < rank(&low).tier_index);
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
    assert!(
        decisions
            .iter()
            .all(|d| d.release_score == decisions[0].release_score)
    );
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
