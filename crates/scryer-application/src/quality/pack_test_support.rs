//! Behavioral tests use the shipped pack; expected numbers belong in fixtures.

use crate::ParsedReleaseMetadata;
use crate::quality_profile::{
    CoverageSizeBasis, QualityProfile, QualityProfileDecision, ScoringConfig,
};
use crate::rules::builtin_trash::baseline_engine;
use crate::rules::user_rule_input::{ReleaseRuntimeInfo, RuleContextInfo, build_rule_input};
use crate::scoring_weights::{ScoringOverrides, ScoringPersona};

pub(super) fn balanced_scoring_config() -> ScoringConfig {
    ScoringConfig::default()
}

pub(super) fn test_scoring_config(
    persona: &ScoringPersona,
    overrides: &ScoringOverrides,
) -> ScoringConfig {
    ScoringConfig {
        scoring_persona: persona.clone(),
        scoring_overrides: overrides.clone(),
        ..Default::default()
    }
}

pub(super) fn test_scoring_config_for_category(
    persona: &ScoringPersona,
    overrides: &ScoringOverrides,
    _category: Option<&str>,
) -> ScoringConfig {
    test_scoring_config(persona, overrides)
}

fn configured_profile(profile: &QualityProfile, config: &ScoringConfig) -> QualityProfile {
    let mut profile = profile.clone();
    profile.criteria.scoring_persona = config.scoring_persona.clone();
    profile.criteria.scoring_overrides = config.scoring_overrides.clone();
    profile
}

pub(super) fn pack_entries(
    profile: &QualityProfile,
    release: &ParsedReleaseMetadata,
    category: Option<&str>,
    size: Option<i64>,
    basis: CoverageSizeBasis,
    decision: &QualityProfileDecision,
) -> Vec<scryer_rules::UserRuleEntry> {
    let input = build_rule_input(
        release,
        profile,
        decision,
        ReleaseRuntimeInfo {
            size_bytes: size,
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
            category,
            original_language: None,
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
    let result = baseline_engine()
        .evaluator()
        .evaluate(&input, category.unwrap_or("movie"))
        .expect("bundled pack evaluates");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result.entries
}

fn append(decision: &mut QualityProfileDecision, entry: scryer_rules::UserRuleEntry) {
    decision.log_with_source(
        &entry.code,
        entry.delta,
        crate::quality_profile::ScoringSource::SystemRule {
            id: entry.rule_set_id,
            name: entry.rule_set_name,
        },
    );
}

pub(super) fn score_with_pack(
    profile: &QualityProfile,
    release: &ParsedReleaseMetadata,
    has_existing_file: bool,
    config: &ScoringConfig,
) -> QualityProfileDecision {
    score_with_pack_for_category(profile, release, has_existing_file, config, None)
}

pub(super) fn score_with_pack_for_category(
    profile: &QualityProfile,
    release: &ParsedReleaseMetadata,
    has_existing_file: bool,
    config: &ScoringConfig,
    category: Option<&str>,
) -> QualityProfileDecision {
    let profile = configured_profile(profile, config);
    let mut decision = crate::quality_profile::evaluate_profile_requirements(
        &profile,
        release,
        has_existing_file,
        category,
    );
    for entry in pack_entries(
        &profile,
        release,
        category,
        None,
        CoverageSizeBasis::default(),
        &decision,
    ) {
        append(&mut decision, entry);
    }
    decision
}

pub(super) fn score_size_with_pack(
    decision: &mut QualityProfileDecision,
    release: &ParsedReleaseMetadata,
    size_bytes: Option<i64>,
    category: Option<&str>,
    runtime: Option<i32>,
    config: &ScoringConfig,
) {
    score_size_with_pack_basis(
        decision,
        release,
        size_bytes,
        category,
        CoverageSizeBasis::single(runtime),
        false,
        config,
    );
}

pub(super) fn score_size_with_pack_basis(
    decision: &mut QualityProfileDecision,
    release: &ParsedReleaseMetadata,
    size_bytes: Option<i64>,
    category: Option<&str>,
    basis: CoverageSizeBasis,
    prefer_remux: bool,
    config: &ScoringConfig,
) {
    let mut profile = configured_profile(&QualityProfile::default(), config);
    profile.criteria.prefer_remux = prefer_remux;
    let mut entries = pack_entries(&profile, release, category, size_bytes, basis, decision);
    // Keep the explanatory member-basis entry after the numeric entry in these
    // size-only assertions. Production entry order is not a scoring contract.
    entries.sort_by_key(|entry| entry.code == "size_pack_member_basis");
    for entry in entries
        .into_iter()
        .filter(|entry| entry.code.starts_with("size_"))
    {
        append(decision, entry);
    }
    crate::quality_profile::apply_size_requirement(
        decision, &profile, release, size_bytes, category, basis,
    );
}

#[test]
fn bundled_group_reputation_preserves_context_precedence_and_one_winner() {
    use scryer_release_parser::ReleaseSource::{BluRay, WebDl};

    let cases = [
        (
            Some("FLUX"),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_gold",
        ),
        (
            Some("flux"),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_gold",
        ),
        (
            Some("SMURF"),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_silver",
        ),
        (
            Some("BLOOM"),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_bronze",
        ),
        (
            Some("YIFY"),
            Some(BluRay),
            "1080p",
            false,
            "movie",
            "group_banned",
        ),
        (
            Some("YIFY"),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_banned",
        ),
        (Some("YIFY"), None, "1080p", false, "movie", "group_banned"),
        (Some("RARBG"), None, "1080p", false, "movie", "group_banned"),
        (
            Some("UnknownGroup"),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_unknown",
        ),
        (None, Some(WebDl), "1080p", false, "movie", "group_unknown"),
        (
            Some(""),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_unknown",
        ),
        (
            Some("CtrlHD"),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_unknown",
        ),
        (
            Some("CtrlHD"),
            Some(WebDl),
            "1080p",
            false,
            "series",
            "group_gold",
        ),
        (
            Some("CtrlHD"),
            Some(BluRay),
            "1080p",
            false,
            "movie",
            "group_gold",
        ),
        (
            Some("CtrlHD"),
            Some(BluRay),
            "2160p",
            false,
            "movie",
            "group_gold",
        ),
        (
            Some("FraMeSToR"),
            Some(BluRay),
            "2160p",
            true,
            "movie",
            "group_gold",
        ),
        (
            Some("FraMeSToR"),
            Some(BluRay),
            "2160p",
            false,
            "movie",
            "group_unknown",
        ),
        (
            Some("3L"),
            Some(BluRay),
            "2160p",
            true,
            "movie",
            "group_gold",
        ),
        (
            Some("3L"),
            Some(BluRay),
            "2160p",
            true,
            "series",
            "group_unknown",
        ),
        (
            Some("AnimeRG"),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_unknown",
        ),
        (
            Some("AnimeRG"),
            Some(WebDl),
            "1080p",
            false,
            "anime",
            "group_banned",
        ),
        (
            Some("NAN0"),
            Some(WebDl),
            "1080p",
            false,
            "anime",
            "group_gold",
        ),
        (
            Some("alfaHD"),
            Some(WebDl),
            "1080p",
            false,
            "movie",
            "group_banned",
        ),
    ];
    for (group, source, quality, remux, facet, expected) in cases {
        let mut release = crate::parse_release_metadata("Quiet.Meridian.2025.1080p.WEB-DL.H.264");
        release.release_group = group.map(str::to_owned);
        release.source = source;
        release.quality = Some(quality.to_owned());
        release.is_remux = remux;
        let decision = score_with_pack_for_category(
            &QualityProfile::default(),
            &release,
            false,
            &balanced_scoring_config(),
            Some(facet),
        );
        let entries = decision
            .scoring_log
            .iter()
            .filter(|entry| entry.code.starts_with("group_"))
            .collect::<Vec<_>>();
        assert_eq!(
            entries.len(),
            1,
            "{group:?}/{source:?}/{quality}/{remux}/{facet}: {entries:?}"
        );
        assert_eq!(
            entries[0].code, expected,
            "{group:?}/{source:?}/{quality}/{remux}/{facet}"
        );
    }
}

#[test]
fn bundled_group_reputation_uses_selected_persona() {
    let release = crate::parse_release_metadata("Quiet.Meridian.2025.1080p.WEB-DL.H.264-FLUX");
    let contribution = |persona| {
        let config = test_scoring_config(&persona, &ScoringOverrides::default());
        let decision = score_with_pack(&QualityProfile::default(), &release, false, &config);
        decision
            .scoring_log
            .iter()
            .find(|entry| entry.code == "group_gold")
            .unwrap()
            .delta
    };
    assert!(contribution(ScoringPersona::Audiophile) > contribution(ScoringPersona::Balanced));
}

#[test]
fn bundled_upscale_override_preserves_unset_true_false_and_no_signal() {
    for persona in [
        ScoringPersona::Balanced,
        ScoringPersona::Audiophile,
        ScoringPersona::Efficient,
        ScoringPersona::Compatible,
    ] {
        for override_value in [None, Some(true), Some(false)] {
            for enhanced in [true, false] {
                let mut release =
                    crate::parse_release_metadata("Quiet.Meridian.2025.1080p.WEB-DL.H.264");
                release.is_ai_enhanced = enhanced;
                let config = test_scoring_config(
                    &persona,
                    &ScoringOverrides {
                        block_upscaled: override_value,
                        ..Default::default()
                    },
                );
                let decision =
                    score_with_pack(&QualityProfile::default(), &release, false, &config);
                let deltas = decision
                    .scoring_log
                    .iter()
                    .filter(|entry| entry.code == "ai_enhanced_upscaled")
                    .map(|entry| entry.delta)
                    .collect::<Vec<_>>();
                let expected = if enhanced && override_value != Some(false) {
                    vec![-10_000]
                } else {
                    vec![]
                };
                assert_eq!(
                    deltas, expected,
                    "{persona:?}/{override_value:?}/{enhanced}"
                );
            }
        }
    }
}

#[test]
fn bundled_dv_override_requires_signal_and_absence_of_every_fallback() {
    for override_value in [None, Some(false), Some(true)] {
        for dv in [false, true] {
            for fallback_mask in 0..8 {
                let mut release =
                    crate::parse_release_metadata("Quiet.Meridian.2025.2160p.WEB-DL.H.265");
                release.is_dolby_vision = dv;
                release.has_hdr_fallback = fallback_mask & 1 != 0;
                release.is_hdr10plus = fallback_mask & 2 != 0;
                release.is_hlg = fallback_mask & 4 != 0;
                let config = test_scoring_config(
                    &ScoringPersona::Balanced,
                    &ScoringOverrides {
                        block_dv_without_fallback: override_value,
                        ..Default::default()
                    },
                );
                let decision =
                    score_with_pack(&QualityProfile::default(), &release, false, &config);
                let deltas = decision
                    .scoring_log
                    .iter()
                    .filter(|entry| entry.code == "dolby_vision_missing_hdr_fallback")
                    .map(|entry| entry.delta)
                    .collect::<Vec<_>>();
                let expected = if dv && override_value == Some(true) && fallback_mask == 0 {
                    vec![-10_000]
                } else {
                    vec![]
                };
                assert_eq!(deltas, expected, "{override_value:?}/{dv}/{fallback_mask}");
                assert!(decision.block_codes.is_empty());
            }
        }
    }
}

#[test]
fn bundled_language_bonus_requires_all_canonical_languages() {
    let mut release = crate::parse_release_metadata("Quiet.Meridian.2025.1080p.WEB-DL.H.264");
    release.languages_audio = vec!["eng".into()];
    for (required, expected) in [
        (vec![], vec![]),
        (vec!["en"], vec![80]),
        (vec!["English"], vec![80]),
        (vec!["ENG", "jpn"], vec![]),
    ] {
        let mut profile = QualityProfile::default();
        profile.criteria.required_audio_languages =
            required.iter().map(|value| (*value).to_owned()).collect();
        let decision = score_with_pack(&profile, &release, false, &balanced_scoring_config());
        let deltas = decision
            .scoring_log
            .iter()
            .filter(|entry| entry.code == "required_audio_languages_match")
            .map(|entry| entry.delta)
            .collect::<Vec<_>>();
        assert_eq!(deltas, expected, "{required:?}");
    }
}
