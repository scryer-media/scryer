use super::*;
use crate::release_parser::parse_release_metadata;
use crate::scoring_weights::balanced_weights;

// ── normalize_quality_tier ────────────────────────────────────────────────

#[test]
fn normalize_quality_1080p() {
    assert_eq!(
        normalize_quality_tier(Some("1080p")),
        Some("1080P".to_string())
    );
}

#[test]
fn normalize_quality_2160p() {
    assert_eq!(
        normalize_quality_tier(Some("2160p")),
        Some("2160P".to_string())
    );
}

#[test]
fn normalize_quality_4320p() {
    assert_eq!(
        normalize_quality_tier(Some("4320p")),
        Some("4320P".to_string())
    );
}

#[test]
fn normalize_quality_720p() {
    assert_eq!(
        normalize_quality_tier(Some("720p")),
        Some("720P".to_string())
    );
}

#[test]
fn normalize_quality_none() {
    assert_eq!(normalize_quality_tier(None), None);
}

#[test]
fn normalize_quality_already_uppercase() {
    assert_eq!(
        normalize_quality_tier(Some("1080P")),
        Some("1080P".to_string())
    );
}

// ── normalize_source ──────────────────────────────────────────────────────

#[test]
fn normalize_source_webdl_variants() {
    assert_eq!(normalize_source(Some("WEB-DL")), Some("WEB-DL".to_string()));
    assert_eq!(normalize_source(Some("webdl")), Some("WEB-DL".to_string()));
    assert_eq!(normalize_source(Some("WEB")), Some("WEB-DL".to_string()));
}

#[test]
fn normalize_source_bluray_variants() {
    assert_eq!(normalize_source(Some("BluRay")), Some("BluRay".to_string()));
    assert_eq!(normalize_source(Some("BD")), Some("BluRay".to_string()));
    assert_eq!(normalize_source(Some("UHD")), Some("BluRay".to_string()));
}

#[test]
fn normalize_source_webrip() {
    assert_eq!(normalize_source(Some("WEBRip")), Some("WEBRip".to_string()));
}

#[test]
fn normalize_source_brdisk_maps_to_bluray() {
    assert_eq!(normalize_source(Some("BRDISK")), Some("BRDISK".to_string()));
}

#[test]
fn normalize_source_rawhd_maps_to_hdtv_family() {
    assert_eq!(normalize_source(Some("RAWHD")), Some("HDTV".to_string()));
}

#[test]
fn normalize_source_none() {
    assert_eq!(normalize_source(None), None);
}

// ── normalize_codec ───────────────────────────────────────────────────────

#[test]
fn normalize_codec_h264() {
    assert_eq!(normalize_codec(Some("H264")), Some("H.264".to_string()));
    assert_eq!(normalize_codec(Some("h264")), Some("H.264".to_string()));
    assert_eq!(normalize_codec(Some("AVC")), Some("H.264".to_string()));
    assert_eq!(normalize_codec(Some("AVC1")), Some("H.264".to_string()));
    assert_eq!(normalize_codec(Some("x264")), Some("H.264".to_string()));
}

#[test]
fn normalize_codec_h265() {
    assert_eq!(normalize_codec(Some("H265")), Some("H.265".to_string()));
    assert_eq!(normalize_codec(Some("h265")), Some("H.265".to_string()));
    assert_eq!(normalize_codec(Some("HEVC")), Some("H.265".to_string()));
    assert_eq!(normalize_codec(Some("HEV1")), Some("H.265".to_string()));
    assert_eq!(normalize_codec(Some("HVC1")), Some("H.265".to_string()));
    assert_eq!(normalize_codec(Some("x265")), Some("H.265".to_string()));
}

#[test]
fn normalize_codec_passthrough() {
    assert_eq!(normalize_codec(Some("AV1")), Some("AV1".to_string()));
    assert_eq!(normalize_codec(Some("AV01")), Some("AV1".to_string()));
    assert_eq!(normalize_codec(Some("VP9")), Some("VP9".to_string()));
    assert_eq!(
        crate::release_parser::VideoCodec::parse("AV01").expect("parse codec"),
        crate::release_parser::VideoCodec::Av1
    );
}

#[test]
fn parse_release_metadata_canonicalizes_video_codec_aliases() {
    assert_eq!(
        parse_release_metadata("Movie.2024.2160p.BluRay.Remux.HEVC.DTS-HD")
            .video_codec
            .as_ref(),
        Some(&crate::release_parser::VideoCodec::H265)
    );
    assert_eq!(
        parse_release_metadata("Movie.2024.1080p.BluRay.AVC.AAC")
            .video_codec
            .as_ref(),
        Some(&crate::release_parser::VideoCodec::H264)
    );
    let av1_8k = parse_release_metadata("Movie.2026.4320p.WEB-DL.AV1.AAC");
    assert_eq!(
        av1_8k.video_codec.as_ref(),
        Some(&crate::release_parser::VideoCodec::Av1)
    );
    assert_eq!(av1_8k.quality.as_deref(), Some("4320p"));
}

// ── normalize_list ────────────────────────────────────────────────────────

#[test]
fn normalize_list_uppercases() {
    let result = normalize_list(vec!["web-dl".into(), "bluray".into()]);
    assert_eq!(result, vec!["WEB-DL", "BLURAY"]);
}

#[test]
fn normalize_list_trims() {
    let result = normalize_list(vec!["  DDP  ".into()]);
    assert_eq!(result, vec!["DDP"]);
}

#[test]
fn normalize_list_filters_empty() {
    let result = normalize_list(vec!["DDP".into(), "".into(), "  ".into()]);
    assert_eq!(result, vec!["DDP"]);
}

#[test]
fn parse_profile_normalizes_video_codec_lists() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"video_codec_allowlist":["h264","hevc","av01"],"video_codec_blocklist":["avc","x265","av1"]}}"#,
    )
    .expect("should parse");

    assert_eq!(
        profile.criteria.video_codec_allowlist,
        vec![
            crate::release_parser::VideoCodec::H264,
            crate::release_parser::VideoCodec::H265,
            crate::release_parser::VideoCodec::Av1
        ]
    );
    assert_eq!(
        profile.criteria.video_codec_blocklist,
        vec![
            crate::release_parser::VideoCodec::H264,
            crate::release_parser::VideoCodec::H265,
            crate::release_parser::VideoCodec::Av1
        ]
    );
}

// ── resolve_archival_quality ──────────────────────────────────────────────

#[test]
fn resolve_archival_quality_explicit() {
    let result = resolve_archival_quality(Some("1080p".to_string()), &["2160P".to_string()]);
    assert_eq!(result, Some("1080P".to_string()));
}

#[test]
fn resolve_archival_quality_falls_back_to_first_tier() {
    let result = resolve_archival_quality(None, &["2160P".to_string(), "1080P".to_string()]);
    assert_eq!(result, Some("2160P".to_string()));
}

#[test]
fn resolve_archival_quality_falls_back_to_1080p_when_empty() {
    let result = resolve_archival_quality(None, &[]);
    assert_eq!(result, Some("1080P".to_string()));
}

// ── QualityProfile parsing ────────────────────────────────────────────────

#[test]
fn parse_minimal_profile() {
    let profile = QualityProfile::parse(r#"{"id":"test","name":"Test","criteria":{}}"#)
        .expect("should parse");
    assert_eq!(profile.id, "test");
    assert!(profile.criteria.quality_tiers.is_empty());
    assert!(!profile.criteria.allow_unknown_quality);
    // detected_hdr_allowed defaults to true
    assert!(profile.criteria.detected_hdr_allowed);
}

#[test]
fn parse_profile_normalizes_tiers() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160p","1080p"]}}"#,
    )
    .expect("should parse");
    assert_eq!(
        profile.criteria.quality_tiers,
        vec!["2160P".to_string(), "1080P".to_string()]
    );
}

#[test]
fn parse_profile_catalog() {
    let profiles = parse_profile_catalog_from_json(
        r#"[{"id":"a","name":"A","criteria":{}},{"id":"b","name":"B","criteria":{}}]"#,
    )
    .expect("should parse");
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0].id, "a");
    assert_eq!(profiles[1].id, "b");
}

#[test]
fn parse_profile_invalid_json() {
    assert!(QualityProfile::parse("{invalid").is_err());
}

// ── evaluate_against_profile: quality tier scoring ────────────────────────

/// Tier membership is a gate, not a score.
///
/// A listed quality contributes no points at all: ordering by tier happens
/// before any score is consulted, in the admission gate and in search ranking.
/// It used to add 3200/900/300 by position, which let a size penalty or a
/// custom-format bonus argue across a whole resolution step.
#[test]
fn a_listed_quality_scores_no_tier_points() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();

    for title in [
        "Movie.2024.2160p.WEB-DL.H.265",
        "Movie.2024.1080p.WEB-DL.H.265",
    ] {
        let release = parse_release_metadata(title);
        let d = evaluate_against_profile(&profile, &release, false, &w);
        assert!(
            !d.scoring_log
                .iter()
                .any(|e| e.code.starts_with("quality_tier_")),
            "{title} still scored a tier bonus: {:?}",
            d.scoring_log
        );
        assert!(d.allowed, "{title} is in the profile and must be allowed");
    }
}

/// The profile's ordering is what says one quality beats another, and it is the
/// same lookup both the admission gate and search ranking use.
#[test]
fn the_profile_orders_its_tiers() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#,
    ).unwrap();

    assert_eq!(
        crate::quality_profile::quality_tier_index(&profile.criteria, Some("2160p")),
        Some(0)
    );
    assert_eq!(
        crate::quality_profile::quality_tier_index(&profile.criteria, Some("1080p")),
        Some(1)
    );
    assert_eq!(
        crate::quality_profile::quality_tier_index(&profile.criteria, Some("720p")),
        None,
        "a quality the profile does not list has no position"
    );
}

#[test]
fn quality_not_in_tiers_is_blocked() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.480p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"quality_not_in_profile_tiers".to_string())
    );
}

// ── evaluate_against_profile: source scoring ──────────────────────────────

#[test]
fn bluray_source_gets_150() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.2160p.BluRay.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "source_bluray" && e.delta == 150)
    );
}

#[test]
fn webdl_source_gets_120() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "source_webdl" && e.delta == 120)
    );
}

#[test]
fn source_blocklist_blocks() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"source_blocklist":["HDTV"],"allow_upgrades":true,"allow_unknown_quality":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.HDTV.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"source_in_profile_blocklist".to_string())
    );
}

#[test]
fn video_codec_allowlist_accepts_h264_family_aliases() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"video_codec_allowlist":["H264"],"allow_upgrades":true,"allow_unknown_quality":true}}"#,
    )
    .unwrap();
    let w = balanced_weights();

    for raw in [
        "Movie.2024.1080p.WEB-DL.H264",
        "Movie.2024.1080p.WEB-DL.x264",
        "Movie.2024.1080p.WEB-DL.AVC",
    ] {
        let release = parse_release_metadata(raw);
        let d = evaluate_against_profile(&profile, &release, false, &w);
        assert!(d.allowed, "{raw}");
        assert!(
            d.scoring_log
                .iter()
                .any(|e| e.code == "video_codec_preferred_0"),
            "{raw}"
        );
    }
}

#[test]
fn video_codec_blocklist_blocks_h264_family_aliases() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"video_codec_blocklist":["H264"],"allow_upgrades":true,"allow_unknown_quality":true}}"#,
    )
    .unwrap();
    let w = balanced_weights();

    for raw in [
        "Movie.2024.1080p.WEB-DL.H264",
        "Movie.2024.1080p.WEB-DL.x264",
        "Movie.2024.1080p.WEB-DL.AVC",
    ] {
        let release = parse_release_metadata(raw);
        let d = evaluate_against_profile(&profile, &release, false, &w);
        assert!(!d.allowed, "{raw}");
        assert!(
            d.block_codes
                .contains(&"video_codec_in_profile_blocklist".to_string()),
            "{raw}"
        );
    }
}

#[test]
fn low_quality_theatrical_sources_block_by_default() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.Name.2024.HQCAM.x264");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"source_low_quality_theatrical".to_string())
    );
}

// ── evaluate_against_profile: DV/HDR ─────────────────────────────────────

#[test]
fn dolby_vision_bonus_when_allowed() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"dolby_vision_allowed":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let mut release = parse_release_metadata("Movie.2024.2160p.WEB-DL.DV.H.265");
    release.has_hdr_fallback = false;
    release.is_hdr10plus = false;
    release.is_hlg = false;
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "dolby_vision_bonus" && e.delta == 50)
    );
}

#[test]
fn dolby_vision_blocks_when_not_allowed() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"dolby_vision_allowed":false,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let mut release = parse_release_metadata("Movie.2024.2160p.WEB-DL.DV.H.265");
    release.has_hdr_fallback = false;
    release.is_hdr10plus = false;
    release.is_hlg = false;
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"dolby_vision_not_allowed".to_string())
    );
}

#[test]
fn hdr_blocks_when_not_allowed() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"detected_hdr_allowed":false,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.HDR.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(d.block_codes.contains(&"hdr_not_allowed".to_string()));
}

// ── evaluate_against_profile: remux / atmos / dual audio ──────────────────

#[test]
fn balanced_profile_scores_explicit_remux_preference() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"prefer_remux":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.BluRay.REMUX.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "prefer_remux_match" && e.delta == 250)
    );
}

#[test]
fn audiophile_profile_scores_remux_preference() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"prefer_remux":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = crate::scoring_weights::build_weights(
        &crate::scoring_weights::ScoringPersona::Audiophile,
        &crate::scoring_weights::ScoringOverrides::default(),
    );
    let release = parse_release_metadata("Movie.2024.1080p.BluRay.REMUX.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "prefer_remux_match" && e.delta == 400)
    );
}

#[test]
fn audiophile_profile_penalizes_missing_remux() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"prefer_remux":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = crate::scoring_weights::build_weights(
        &crate::scoring_weights::ScoringPersona::Audiophile,
        &crate::scoring_weights::ScoringOverrides::default(),
    );
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "prefer_remux_missing" && e.delta == -80)
    );
}

#[test]
fn audiophile_persona_applies_atmos_bonus() {
    let profile = QualityProfile::default();
    let w = crate::scoring_weights::build_weights(
        &crate::scoring_weights::ScoringPersona::Audiophile,
        &crate::scoring_weights::ScoringOverrides::default(),
    );
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.DDP.Atmos.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "atmos_preferred_match" && e.delta == 150)
    );
}

#[test]
fn dual_audio_no_baseline_bonus() {
    // prefer_dual_audio no longer triggers built-in scoring — that's now
    // handled by managed convenience rules. The quality profile scorer
    // should NOT emit a dual_audio scoring entry.
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"prefer_dual_audio":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.DUAL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        !d.scoring_log.iter().any(|e| e.code == "dual_audio"),
        "dual_audio scoring should not appear — handled by convenience rules"
    );
}

// ── evaluate_against_profile: required audio languages ────────────────────

#[test]
fn required_audio_language_match() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"required_audio_languages":["ENG"],"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.English.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(d.allowed);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "required_audio_languages_match")
    );
}

#[test]
fn required_audio_language_missing_blocks() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"required_audio_languages":["JPN"],"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.English.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"required_audio_language_missing".to_string())
    );
}

#[test]
fn required_audio_language_match_accepts_canonical_lowercase_codes() {
    let mut profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    )
    .unwrap();
    profile.criteria.required_audio_languages = vec!["eng".to_string()];
    let w = balanced_weights();
    let mut release = parse_release_metadata("Movie.2024.1080p.WEB-DL.English.H.265");
    let title_context = crate::title_audio_language_context(None, None, Some("movie"), &[]);
    release.languages_audio =
        crate::release_audio_language_hints_for_title(&release, None, Some(&title_context), true);
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(d.allowed);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "required_audio_languages_match")
    );
}

#[test]
fn dual_audio_release_satisfies_required_english() {
    let mut profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    )
    .unwrap();
    profile.criteria.required_audio_languages = vec!["eng".to_string()];
    let w = balanced_weights();
    let mut release = parse_release_metadata("Anime.Show.S01E01.1080p.WEB-DL.DUAL.H.265");
    let title_context = crate::title_audio_language_context(None, None, Some("anime"), &[]);
    release.languages_audio =
        crate::release_audio_language_hints_for_title(&release, None, Some(&title_context), true);
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(d.allowed);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "required_audio_languages_match")
    );
}

#[test]
fn french_origin_unlabeled_release_blocks_required_english() {
    let mut profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    )
    .unwrap();
    profile.criteria.required_audio_languages = vec!["eng".to_string()];
    let w = balanced_weights();
    let mut release = parse_release_metadata("Film.2024.1080p.WEB-DL.H.265");
    let title_context =
        crate::title_audio_language_context(None, Some("France"), Some("movie"), &[]);
    release.languages_audio =
        crate::release_audio_language_hints_for_title(&release, None, Some(&title_context), true);
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"required_audio_language_missing".to_string())
    );
}

#[test]
fn unknown_non_anime_unlabeled_release_satisfies_required_english() {
    let mut profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    )
    .unwrap();
    profile.criteria.required_audio_languages = vec!["eng".to_string()];
    let w = balanced_weights();
    let mut release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let title_context = crate::title_audio_language_context(None, None, Some("movie"), &[]);
    release.languages_audio =
        crate::release_audio_language_hints_for_title(&release, None, Some(&title_context), true);
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(d.allowed);
}

#[test]
fn non_anime_dual_audio_satisfies_required_english() {
    let mut profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    )
    .unwrap();
    profile.criteria.required_audio_languages = vec!["eng".to_string()];
    let w = balanced_weights();
    let mut release = parse_release_metadata("Movie.2024.1080p.WEB-DL.DUAL.H.265");
    let title_context =
        crate::title_audio_language_context(None, Some("France"), Some("movie"), &[]);
    release.languages_audio =
        crate::release_audio_language_hints_for_title(&release, None, Some(&title_context), true);
    // DUAL audio means English plus the title's original language (French here),
    // so a required-English profile is satisfied rather than falsely blocked.
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(d.allowed);
    assert!(
        !d.block_codes
            .contains(&"required_audio_language_missing".to_string())
    );
}

#[test]
fn non_anime_dual_audio_does_not_satisfy_unrelated_required_language() {
    let mut profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    )
    .unwrap();
    profile.criteria.required_audio_languages = vec!["jpn".to_string()];
    let w = balanced_weights();
    let mut release = parse_release_metadata("Movie.2024.1080p.WEB-DL.DUAL.H.265");
    let title_context =
        crate::title_audio_language_context(None, Some("France"), Some("movie"), &[]);
    release.languages_audio =
        crate::release_audio_language_hints_for_title(&release, None, Some(&title_context), true);
    // DUAL infers eng+fra for a French title; a required Japanese track is still
    // correctly reported missing, so the gate keeps blocking genuine mismatches.
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"required_audio_language_missing".to_string())
    );
}

#[test]
fn japanese_only_release_still_blocks_required_english() {
    let mut profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    )
    .unwrap();
    profile.criteria.required_audio_languages = vec!["eng".to_string()];
    let w = balanced_weights();
    let mut release = parse_release_metadata("Anime.Show.S01E01.1080p.WEB-DL.JAPANESE.H.265");
    let title_context = crate::title_audio_language_context(None, None, Some("anime"), &[]);
    release.languages_audio =
        crate::release_audio_language_hints_for_title(&release, None, Some(&title_context), true);
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"required_audio_language_missing".to_string())
    );
}

// ── evaluate_against_profile: upgrade guard ───────────────────────────────

#[test]
fn upgrade_blocked_when_has_existing_file_and_upgrades_disabled() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_upgrades":false,"allow_unknown_quality":true}}"#,
    )
    .unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, true, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"upgrade_blocked_by_profile".to_string())
    );
}

#[test]
fn upgrade_allowed_when_no_existing_file() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_upgrades":false,"allow_unknown_quality":true}}"#,
    )
    .unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(d.allowed);
}

// ── evaluate_against_profile: proper upload / low confidence ──────────────

#[test]
fn proper_upload_bonus() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    )
    .unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.PROPER.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "proper_upload" && e.delta == 30)
    );
}

// ── resolve_profile_id_for_title ──────────────────────────────────────────

#[test]
fn resolve_profile_id_title_wins() {
    let result = resolve_profile_id_for_title(
        Some("title"),
        Some("library"),
        Some("category"),
        Some("global"),
    );
    assert_eq!(result, Some("title".to_string()));
}

#[test]
fn resolve_profile_id_library_fallback() {
    let result =
        resolve_profile_id_for_title(None, Some("library"), Some("category"), Some("global"));
    assert_eq!(result, Some("library".to_string()));
}

#[test]
fn resolve_profile_id_category_fallback() {
    let result = resolve_profile_id_for_title(None, None, Some("category"), Some("global"));
    assert_eq!(result, Some("category".to_string()));
}

#[test]
fn resolve_profile_id_global_fallback() {
    let result = resolve_profile_id_for_title(None, None, None, Some("global"));
    assert_eq!(result, Some("global".to_string()));
}

#[test]
fn resolve_profile_id_none_fallback() {
    let result = resolve_profile_id_for_title(None, None, None, None);
    assert_eq!(result, None);
}

// ── default profiles ──────────────────────────────────────────────────────

#[test]
fn default_4k_profile_has_three_tiers() {
    let profile = builtin_4k_profile();
    assert_eq!(profile.criteria.quality_tiers.len(), 3);
    assert_eq!(profile.criteria.quality_tiers[0], "2160P");
    assert!(!profile.criteria.prefer_remux);
}

#[test]
fn default_8k_profile_has_4320p_archival_tier() {
    let profile = builtin_8k_profile();
    assert_eq!(profile.id, "8k");
    assert_eq!(profile.criteria.archival_quality.as_deref(), Some("4320P"));
    assert_eq!(profile.criteria.quality_tiers[0], "4320P");
    assert!(!profile.criteria.prefer_remux);
}

#[test]
fn default_1080p_profile_has_two_tiers() {
    let profile = builtin_1080p_profile();
    assert_eq!(profile.criteria.quality_tiers.len(), 2);
    assert_eq!(profile.criteria.quality_tiers[0], "1080P");
    assert!(!profile.criteria.prefer_remux);
}

// ── apply_size_scoring_for_category ───────────────────────────────────────

#[test]
fn size_scoring_no_size_is_noop() {
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let w = balanced_weights();
    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, None, None, None, &w);
    assert!(d.scoring_log.is_empty());
}

#[test]
fn size_scoring_zero_bytes_is_noop() {
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let w = balanced_weights();
    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, Some(0), None, None, &w);
    assert!(d.scoring_log.is_empty());
}

#[test]
fn size_scoring_anime_expects_smaller() {
    let release = parse_release_metadata("Anime.2024.1080p.WEB-DL.H.265");
    let w = balanced_weights();
    let size_1gb = 1024 * 1024 * 1024_i64;

    let mut d_anime = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut d_anime,
        &release,
        Some(size_1gb),
        Some("anime"),
        None,
        &w,
    );

    let mut d_movie = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d_movie, &release, Some(size_1gb), None, None, &w);

    // 1GB for anime 1080p is near expected; for a movie it is much too small.
    assert!(d_anime.release_score > d_movie.release_score);
}

#[test]
fn size_scoring_scales_with_runtime() {
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let w = balanced_weights();
    let size_12gb = 12 * 1024 * 1024 * 1024_i64;

    // 12 GB for a standard 2-hour movie (baseline 120 min) → ~1.5× expected (8 GiB × 0.8 WEB)
    let mut d_standard = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut d_standard,
        &release,
        Some(size_12gb),
        None,
        Some(120),
        &w,
    );

    // 12 GB for a 3-hour movie → expected is scaled up by 180/120 = 1.5×
    let mut d_long = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d_long, &release, Some(size_12gb), None, Some(180), &w);

    // The long movie should score higher because 12 GB is more "expected" for 3 hours
    assert!(d_long.release_score > d_standard.release_score);
}

#[test]
fn size_scoring_anime_ova_runtime_scales_expectation() {
    let release = parse_release_metadata("Anime.2024.1080p.WEB-DL.H.265");
    let w = balanced_weights();
    let size_3gb = 3 * 1024 * 1024 * 1024_i64;

    // 3 GB for a standard 24-min anime episode → quite large relative to expected
    let mut d_standard = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut d_standard,
        &release,
        Some(size_3gb),
        Some("anime"),
        Some(24),
        &w,
    );

    // 3 GB for a 50-min OVA → runtime scales the expected size up, so ratio is lower
    let mut d_ova = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut d_ova,
        &release,
        Some(size_3gb),
        Some("anime"),
        Some(50),
        &w,
    );

    // OVA should score better (higher) because 3 GB is less of an outlier for 50 min
    assert!(d_ova.release_score >= d_standard.release_score);
}

/// Releases spanning the facets, qualities, codecs and sources the size model
/// distinguishes, each sized the way a real release of that shape is sized.
///
/// Used by the drift property below: the point of a continuous size term is that
/// *realistic* releases stop jumping a whole bucket when the landed byte count
/// comes in a few percent under the announced one. The test asserts each entry
/// lands in a healthy band, so a miscalibrated fixture fails loudly instead of
/// quietly testing the flat ends of the curve.
const SIZE_DRIFT_CORPUS: &[(&str, Option<&str>, i32, f64)] = &[
    // (release, category hint, runtime minutes, announced size in GiB)
    (
        "Portmere.2024.2160p.WEB-DL.DDP5.1.H.265-GRP",
        None,
        118,
        32.0,
    ),
    ("Portmere.2024.2160p.BluRay.HDR.H.265-GRP", None, 118, 35.0),
    ("Portmere.2024.1080p.WEB-DL.H.264-GRP", None, 118, 8.4),
    ("Portmere.2024.1080p.BluRay.x265-GRP", None, 118, 12.0),
    ("Portmere.2024.720p.WEB-DL.H.264-GRP", None, 96, 2.5),
    ("Portmere.2024.1080p.WEB-DL.AV1-GRP", None, 141, 6.5),
    (
        "Glass.Harbor.S02E04.1080p.WEB-DL.DDP5.1.H.264-GRP",
        Some("series"),
        44,
        2.8,
    ),
    (
        "Glass.Harbor.S02E04.2160p.WEB-DL.H.265-GRP",
        Some("series"),
        44,
        5.0,
    ),
    (
        "Glass.Harbor.S01E01.720p.HDTV.x264-GRP",
        Some("series"),
        58,
        1.7,
    ),
    (
        "Glass.Harbor.S03E09.1080p.BluRay.REMUX-GRP",
        Some("series"),
        52,
        7.5,
    ),
    (
        "Umibe.Signal.S01E11.1080p.WEB-DL.H.265-GRP",
        Some("anime"),
        24,
        1.0,
    ),
    (
        "Umibe.Signal.S01E11.720p.BluRay.x264-GRP",
        Some("anime"),
        24,
        1.0,
    ),
    (
        "Umibe.Signal.OVA.1080p.BluRay.H.265-GRP",
        Some("anime"),
        52,
        3.6,
    ),
];

/// Every band the curve can report, weakest first.
const SIZE_BANDS: [&str; 9] = [
    "size_tiny_for_quality",
    "size_very_small_for_quality",
    "size_small_for_quality",
    "size_slightly_small_for_quality",
    "size_expected_for_quality",
    "size_large_for_quality",
    "size_very_large_for_quality",
    "size_massive_for_quality",
    "size_excessive_for_quality",
];

fn size_band_index(code: &str) -> usize {
    SIZE_BANDS
        .iter()
        .position(|candidate| *candidate == code)
        .unwrap_or_else(|| panic!("unexpected size band {code}"))
}

fn size_delta(release: &str, category: Option<&str>, runtime: i32, size_gib: f64) -> (String, i32) {
    let parsed = parse_release_metadata(release);
    let weights = balanced_weights();
    let mut decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut decision,
        &parsed,
        Some((size_gib * 1024.0 * 1024.0 * 1024.0) as i64),
        category,
        Some(runtime),
        &weights,
    );
    let entry = decision
        .scoring_log
        .first()
        .expect("size scoring always logs exactly one entry for a positive size");
    (entry.code.clone(), entry.delta)
}

/// **The D3 property.** A landed file is routinely a few percent smaller than
/// the size the NZB advertised (par2 and RAR overhead are counted in the
/// announcement but not in the video file). Under the old step function that
/// drift could cross a bucket boundary and move the score by a whole bucket
/// weight — up to 700 points on the Balanced curve, against a +200 grab
/// threshold — so a release admitted at grab was refused at import as a
/// downgrade. The term is now continuous, so the same drift moves the number by
/// tens of points.
#[test]
fn realistic_landed_drift_moves_the_size_term_only_slightly() {
    // A deterministic sweep rather than a random draw: a property test that
    // fails one run in fifty is worse than no test.
    let factors = [0.88_f64, 0.90, 0.93, 0.95, 0.97, 0.99, 1.0];
    let healthy = size_band_index("size_slightly_small_for_quality")
        ..=size_band_index("size_very_large_for_quality");

    let mut worst = 0;
    for (release, category, runtime, announced_gib) in SIZE_DRIFT_CORPUS {
        let (announced_code, announced_delta) =
            size_delta(release, *category, *runtime, *announced_gib);
        assert!(
            healthy.contains(&size_band_index(&announced_code)),
            "corpus entry `{release}` at {announced_gib} GiB is not a plausible \
             size for what it claims: it lands in {announced_code}"
        );
        for factor in factors {
            let (landed_code, landed_delta) =
                size_delta(release, *category, *runtime, announced_gib * factor);
            let moved = (landed_delta - announced_delta).abs();
            worst = worst.max(moved);
            assert!(
                moved <= 125,
                "`{release}` at ×{factor} moved the size term by {moved} \
                 ({announced_code} {announced_delta} → {landed_code} {landed_delta})"
            );
            let bands = size_band_index(&landed_code).abs_diff(size_band_index(&announced_code));
            assert!(
                bands <= 1,
                "`{release}` at ×{factor} skipped a band: {announced_code} → {landed_code}"
            );
        }
    }
    assert!(worst > 0, "the corpus must actually exercise the curve");
}

/// The same drift, swept across the **whole** curve rather than the bands a
/// healthy release occupies.
///
/// The bound here is 300 rather than 100, and that is a property of the Balanced
/// weight table, not of the interpolation: `size_small` is −700 where
/// `size_slightly_small` is 0, and one bucket is only 1.35× wide, so the gentlest
/// curve that still honours both weights moves ~300 points across a 12 % drift
/// there. It was 700 before, discontinuously, which is the regression this pins.
/// Closing the remaining gap means re-baselining those weights, which is a
/// separate product decision.
#[test]
fn no_drift_anywhere_on_the_curve_cliffs_the_way_a_bucket_step_did() {
    let release = "Boundary.2024.1080p.WEB-DL.H.264-GRP";
    let mut bands_seen = std::collections::HashSet::new();
    let mut worst = 0;

    // 0.9 GiB to 60 GiB against a ~7 GiB expectation covers tiny through
    // excessive. The sweep starts above the minimum-size veto (0.10 × expected
    // ≈ 0.70 GiB) and stops below the maximum one: those are steps by design.
    let mut gib = 0.9_f64;
    while gib < 55.0 {
        let (announced_code, announced_delta) = size_delta(release, None, 120, gib);
        let (landed_code, landed_delta) = size_delta(release, None, 120, gib * 0.88);
        bands_seen.insert(announced_code.clone());
        let moved = (landed_delta - announced_delta).abs();
        worst = worst.max(moved);
        assert!(
            moved <= 300,
            "a 12% drift at {gib:.2} GiB moved the size term by {moved} \
             ({announced_code} {announced_delta} → {landed_code} {landed_delta})"
        );
        let bands = size_band_index(&landed_code).abs_diff(size_band_index(&announced_code));
        assert!(
            bands <= 1,
            "a 12% drift at {gib:.2} GiB skipped a band: {announced_code} → {landed_code}"
        );
        gib *= 1.05;
    }

    assert!(
        bands_seen.len() >= 7,
        "the sweep must cross most of the curve to be worth anything: {bands_seen:?}"
    );
    assert!(worst > 0);
}

/// **D21, as it now reads.** A release far below anything its quality and
/// runtime could produce is *penalised*, not refused.
///
/// The veto is gone. Its false positives were not fakes but honest aggregates
/// whose indexer reported one member's size, and the profile's minimum score
/// still refuses a genuinely tiny release on the numbers.
#[test]
fn size_implausibly_small_penalises_a_release_a_tenth_of_its_size() {
    // 1080p WEB-DL, 120 min → expected ≈ 7.5 GiB. 400 MiB is ~5% of that.
    let release = parse_release_metadata("Portmere.2024.1080p.WEB-DL.H.264-GRP");
    let weights = balanced_weights();

    let mut decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut decision,
        &release,
        Some(400 * 1024 * 1024),
        None,
        Some(120),
        &weights,
    );

    assert!(
        decision.allowed,
        "a tiny movie is a penalty now, not a block: {:?}",
        decision.block_codes
    );
    assert_eq!(decision.scoring_log[0].code, "size_tiny_for_quality");
    assert_eq!(decision.scoring_log[0].delta, weights.size_tiny);
    assert!(
        !decision
            .scoring_log
            .iter()
            .any(|entry| entry.delta == BLOCK_SCORE),
        "{:?}",
        decision.scoring_log
    );
}

/// **BL2.** The honest number is always in the log.
///
/// `total` — the bar every later comparison uses — is the pass's score with the
/// `BLOCK_SCORE` entries stripped out. When the bottom veto existed, replacing
/// the band entry with it would have left a refused file carrying **no** size
/// term and a bar 2500 points above the same file a byte the other side of the
/// threshold. Nothing replaces the band today either.
#[test]
fn a_tiny_release_carries_the_size_penalty_it_earned() {
    let release = parse_release_metadata("Portmere.2024.1080p.WEB-DL.H.264-GRP");
    let weights = balanced_weights();

    let mut decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut decision,
        &release,
        Some(400 * 1024 * 1024),
        None,
        Some(120),
        &weights,
    );

    let non_block: i32 = decision
        .scoring_log
        .iter()
        .filter(|entry| entry.delta != BLOCK_SCORE)
        .map(|entry| entry.delta)
        .sum();
    assert_eq!(
        non_block, weights.size_tiny,
        "the band must be in the log: {:?}",
        decision.scoring_log
    );
}

/// The same property stated as monotonicity: getting smaller must never improve
/// the bar. It used to improve it by 2500 as the veto replaced the band.
#[test]
fn the_bar_is_monotone_across_the_bottom_of_the_size_curve() {
    let release = parse_release_metadata("Portmere.2024.1080p.WEB-DL.H.264-GRP");
    let weights = balanced_weights();

    // Expected ≈ 7.04 GiB for a 120-minute 1080p WEB-DL H.264 movie, so the
    // 0.10 curve anchor sits at ≈ 721 MiB. One sample either side of it.
    let bar_at = |size_mib: i64| {
        let mut decision = QualityProfileDecision::new();
        apply_size_scoring_for_category(
            &mut decision,
            &release,
            Some(size_mib * 1024 * 1024),
            None,
            Some(120),
            &weights,
        );
        let total: i32 = decision
            .scoring_log
            .iter()
            .filter(|entry| entry.delta != BLOCK_SCORE)
            .map(|entry| entry.delta)
            .sum();
        (total, decision.allowed)
    };

    let (just_above, allowed_above) = bar_at(760);
    let (just_below, allowed_below) = bar_at(680);
    assert!(allowed_above, "neither sample is refused on size");
    assert!(allowed_below, "the anchor is not a veto any more");
    assert!(
        just_below <= just_above,
        "the smaller file scored better: {just_below} > {just_above}"
    );
}

/// **MA5.** The episodic curve anchor is calibrated against Sonarr's shipped
/// `QualityDefinition.MinSize` (4 MB/min at 1080p), so an ordinary 4.5 MB/min
/// WEB-DL episode sits above it and takes the tiny penalty at less than full
/// strength.
#[test]
fn an_ordinary_episode_at_sonarrs_minimum_bitrate_is_not_vetoed() {
    let release = parse_release_metadata("Portmere.S01E04.1080p.WEB-DL.H.264-GRP");
    let weights = balanced_weights();

    // 45 minutes at 4.5 MB/min = 202.5 MB.
    let mut decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut decision,
        &release,
        Some(202_500_000),
        Some("series"),
        Some(45),
        &weights,
    );

    assert!(
        decision.allowed,
        "an episode above Sonarr's own MinSize was vetoed: {:?}",
        decision.block_codes
    );
    assert_eq!(decision.scoring_log[0].code, "size_tiny_for_quality");
}

/// …and below the calibrated anchor it is the full tiny penalty, still not a
/// block.
#[test]
fn an_episode_far_under_the_calibrated_floor_is_penalised_at_full_strength() {
    let release = parse_release_metadata("Portmere.S01E04.1080p.WEB-DL.H.264-GRP");
    let weights = balanced_weights();

    // 45 minutes at ~1.5 MB/min.
    let mut decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut decision,
        &release,
        Some(67_500_000),
        Some("series"),
        Some(45),
        &weights,
    );

    assert!(decision.allowed, "{:?}", decision.block_codes);
    assert_eq!(decision.scoring_log[0].code, "size_tiny_for_quality");
    assert_eq!(decision.scoring_log[0].delta, weights.size_tiny);
}

/// A special used to be the one shape exempt from the minimum-size veto
/// (Sonarr's `AcceptableSizeSpecification.cs:29-33`), because a seven-minute S00
/// short has no recorded runtime and reads as a fraction of the series average.
/// With no veto left there is nothing to exempt it from: a special takes exactly
/// the penalty an ordinary episode of the same size takes, and the exemption
/// helper is gone with the veto.
#[test]
fn a_special_takes_the_same_size_penalty_as_any_other_episode() {
    let weights = balanced_weights();
    let special = parse_release_metadata("Portmere.S00E03.1080p.WEB-DL.H.264-GRP");
    assert_eq!(
        special.episode.as_ref().and_then(|episode| episode.season),
        Some(0)
    );

    let mut decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut decision,
        &special,
        Some(60 * 1024 * 1024),
        Some("series"),
        Some(45),
        &weights,
    );

    assert!(decision.allowed, "{:?}", decision.block_codes);
    assert_eq!(decision.scoring_log[0].code, "size_tiny_for_quality");
    assert_eq!(decision.scoring_log[0].delta, weights.size_tiny);

    // The same bytes under an ordinary episode number score identically: the
    // size term no longer asks what kind of episode this is.
    let ordinary = parse_release_metadata("Portmere.S01E03.1080p.WEB-DL.H.264-GRP");
    let mut ordinary_decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(
        &mut ordinary_decision,
        &ordinary,
        Some(60 * 1024 * 1024),
        Some("series"),
        Some(45),
        &weights,
    );
    assert!(ordinary_decision.allowed);
    assert_eq!(
        ordinary_decision.preference_score,
        decision.preference_score
    );
}

/// The size curve says nothing about files the import pipeline's sample filter
/// owns.
///
/// A `.strm` stream pointer holds a URL, not media, and its byte count says
/// nothing about the release. Refusing it here would make stream-pointer imports
/// fail on the length of their own filename.
#[test]
fn a_file_too_small_to_be_media_at_all_is_penalised_but_not_vetoed() {
    let release = parse_release_metadata("Portmere.2024.1080p.WEB-DL.H.264-GRP");
    let weights = balanced_weights();

    for size_bytes in [96_i64, 4 * 1024 * 1024, 49 * 1024 * 1024] {
        let mut decision = QualityProfileDecision::new();
        apply_size_scoring_for_category(
            &mut decision,
            &release,
            Some(size_bytes),
            None,
            Some(120),
            &weights,
        );
        assert!(
            decision.allowed,
            "{size_bytes} bytes was vetoed: {:?}",
            decision.block_codes
        );
        assert_eq!(
            decision.scoring_log[0].code, "size_tiny_for_quality",
            "{size_bytes} bytes should still take the full tiny penalty"
        );
        assert_eq!(decision.scoring_log[0].delta, weights.size_tiny);
    }
}

// ── coverage-aware pack sizes ─────────────────────────────────────────────

const GIB_F: f64 = 1024.0 * 1024.0 * 1024.0;

fn gib(value: f64) -> i64 {
    (value * GIB_F) as i64
}

/// One 45-minute 1080p WEB-DL H.264 episode, at the top of the expected band:
/// 8.5 Mbps × 1.10 codec × 0.80 source × 45 min ≈ 2.47 GiB.
const EPISODE_SIZE_GIB: f64 = 2.5;
const EPISODE_RUNTIME_MINUTES: i32 = 45;

fn score_pack_size(
    release_title: &str,
    size_bytes: i64,
    basis: CoverageSizeBasis,
    weights: &ScoringWeights,
) -> QualityProfileDecision {
    let release = parse_release_metadata(release_title);
    let mut decision = QualityProfileDecision::new();
    apply_size_scoring_for_category_with_remux_preference(
        &mut decision,
        &release,
        Some(size_bytes),
        Some("series"),
        basis,
        false,
        weights,
    );
    decision
}

/// The basis a season pack of `episodes` equal-length episodes has.
fn season_basis(episodes: i32) -> CoverageSizeBasis {
    CoverageSizeBasis::aggregate(
        Some(EPISODE_RUNTIME_MINUTES * episodes),
        Some(EPISODE_RUNTIME_MINUTES),
        episodes,
    )
}

/// A pack whose reported size really is the whole payload reads as the whole
/// payload. The reinterpretation is unreachable for it, and no diagnostic
/// claims otherwise.
#[test]
fn an_honestly_sized_pack_is_scored_on_its_total_runtime() {
    let weights = balanced_weights();
    let decision = score_pack_size(
        "Quiet.Meridian.S01.1080p.WEB-DL.H.264-GroupTag",
        gib(EPISODE_SIZE_GIB * 12.0),
        season_basis(12),
        &weights,
    );

    assert!(decision.allowed);
    assert_eq!(decision.scoring_log.len(), 1, "{:?}", decision.scoring_log);
    assert_eq!(decision.scoring_log[0].code, "size_expected_for_quality");
    assert!(
        !decision
            .scoring_log
            .iter()
            .any(|entry| entry.code == SIZE_PACK_MEMBER_BASIS_CODE),
        "an unambiguous size must not be reinterpreted"
    );
}

/// **The bridge shape.** An indexer lists a season pack under one episode's byte
/// count. Against the pack's total runtime that is a twentieth of what it should
/// be — the reading that used to refuse the release outright. Read against one
/// member it is an ordinary episode, so the release survives, and the
/// interpretation can never pay: the size term is capped at zero.
#[test]
fn a_pack_carrying_one_members_size_is_read_as_that_member() {
    let weights = balanced_weights();

    for episodes in [12, 24, 26] {
        let decision = score_pack_size(
            "Quiet.Meridian.S01.1080p.WEB-DL.H.264-GroupTag",
            gib(EPISODE_SIZE_GIB),
            season_basis(episodes),
            &weights,
        );

        assert!(
            decision.allowed,
            "{episodes}-episode pack was refused: {:?}",
            decision.block_codes
        );
        assert_eq!(
            decision.scoring_log.last().map(|entry| entry.code.as_str()),
            Some(SIZE_PACK_MEMBER_BASIS_CODE),
            "{episodes}-episode pack: {:?}",
            decision.scoring_log
        );
        assert_eq!(
            decision.scoring_log.last().map(|entry| entry.delta),
            Some(0),
            "the diagnostic must carry no weight of its own"
        );
        assert!(
            decision.preference_score <= 0,
            "{episodes}-episode pack earned {} from an inferred size",
            decision.preference_score
        );
        assert_ne!(
            decision.scoring_log[0].code, "size_tiny_for_quality",
            "{episodes}-episode pack still took the tiny penalty: {:?}",
            decision.scoring_log
        );
    }
}

/// A single episode has nothing to reinterpret: one member, one reading, and the
/// tiny penalty stands.
#[test]
fn a_single_episode_is_never_reinterpreted_as_a_member() {
    let weights = balanced_weights();
    let decision = score_pack_size(
        "Quiet.Meridian.S01E04.1080p.WEB-DL.H.264-GroupTag",
        gib(0.1),
        CoverageSizeBasis::single(Some(EPISODE_RUNTIME_MINUTES)),
        &weights,
    );

    assert!(decision.allowed);
    assert_eq!(decision.scoring_log.len(), 1, "{:?}", decision.scoring_log);
    assert_eq!(decision.scoring_log[0].code, "size_tiny_for_quality");
}

/// When neither reading is plausible the release is simply small, and it takes
/// the penalty in full. 300 MiB is a twentieth of one episode, never mind
/// twelve.
#[test]
fn a_genuinely_tiny_pack_keeps_the_full_tiny_penalty() {
    let weights = balanced_weights();
    let decision = score_pack_size(
        "Quiet.Meridian.S01.1080p.WEB-DL.H.264-GroupTag",
        300 * 1024 * 1024,
        season_basis(12),
        &weights,
    );

    assert!(decision.allowed, "{:?}", decision.block_codes);
    assert_eq!(decision.scoring_log.len(), 1, "{:?}", decision.scoring_log);
    assert_eq!(decision.scoring_log[0].code, "size_tiny_for_quality");
    assert_eq!(decision.scoring_log[0].delta, weights.size_tiny);
}

/// The member reading uses the **codec-adjusted** thresholds, the same ones the
/// total reading uses. AV1 carries a 1.5× upper multiplier, so a member ratio of
/// ~2.8 is inside AV1's plausible range and outside H.265's — and the two
/// releases, identical but for the codec, come out on opposite sides.
#[test]
fn the_member_reading_respects_codec_adjusted_thresholds() {
    let weights = balanced_weights();

    // 8.5 Mbps × 0.50 (AV1) × 0.80 (WEB-DL) × 45 min ≈ 1.12 GiB per episode;
    // 3.15 GiB is ~2.8× that.
    let av1 = score_pack_size(
        "Salt.and.Signal.S01.1080p.WEB-DL.AV1-GroupTag",
        gib(3.15),
        season_basis(12),
        &weights,
    );
    assert_eq!(
        av1.scoring_log.last().map(|entry| entry.code.as_str()),
        Some(SIZE_PACK_MEMBER_BASIS_CODE),
        "AV1's headroom must reach the member reading too: {:?}",
        av1.scoring_log
    );
    assert!(av1.preference_score <= 0);

    // 8.5 × 0.75 (H.265) × 0.80 × 45 ≈ 1.68 GiB per episode; 4.71 GiB is the
    // same ~2.8×, which for H.265 is past `massive` and therefore not evidence
    // of anything.
    let h265 = score_pack_size(
        "Salt.and.Signal.S01.1080p.WEB-DL.H.265-GroupTag",
        gib(4.71),
        season_basis(12),
        &weights,
    );
    assert!(
        !h265
            .scoring_log
            .iter()
            .any(|entry| entry.code == SIZE_PACK_MEMBER_BASIS_CODE),
        "a member reading past `massive` is division talking: {:?}",
        h265.scoring_log
    );
    assert_eq!(h265.scoring_log[0].code, "size_tiny_for_quality");
}

/// The reinterpretation spares a penalty; it never grants a bonus. A pack whose
/// member reading lands in the expected band would earn `size_expected` if the
/// size were not in doubt — capped to zero because it is.
#[test]
fn an_inferred_member_size_can_never_earn_a_bonus() {
    let weights = balanced_weights();
    assert!(
        weights.size_expected > 0,
        "fixture precondition: the expected band pays"
    );

    let inferred = score_pack_size(
        "Quiet.Meridian.S01.1080p.WEB-DL.H.264-GroupTag",
        gib(EPISODE_SIZE_GIB),
        season_basis(12),
        &weights,
    );
    let unambiguous = score_pack_size(
        "Quiet.Meridian.S01E04.1080p.WEB-DL.H.264-GroupTag",
        gib(EPISODE_SIZE_GIB),
        CoverageSizeBasis::single(Some(EPISODE_RUNTIME_MINUTES)),
        &weights,
    );

    assert_eq!(inferred.preference_score, 0);
    assert!(
        unambiguous.preference_score > inferred.preference_score,
        "an inferred size outscored a measured one: {} vs {}",
        inferred.preference_score,
        unambiguous.preference_score
    );
}

/// The upper veto is untouched, and it still fires before anything else can
/// speak. A pack cannot reinterpret its way out of being far too large, because
/// the reinterpretation is only reachable from the bottom of the curve.
#[test]
fn the_upper_veto_still_blocks_an_impossible_pack() {
    let weights = balanced_weights();
    let decision = score_pack_size(
        "Quiet.Meridian.S01.1080p.WEB-DL.H.264-GroupTag",
        gib(600.0),
        season_basis(12),
        &weights,
    );

    assert!(!decision.allowed);
    assert!(
        decision
            .block_codes
            .contains(&"size_implausible_for_quality".to_string()),
        "{:?}",
        decision.block_codes
    );
    // The band is still logged first: a veto is a verdict on top of the honest
    // number, never instead of it.
    assert_eq!(decision.scoring_log[0].code, "size_excessive_for_quality");
}

#[test]
fn size_implausible_blocks_wildly_oversized() {
    // 300 GB claiming to be a 720p anime episode — ratio ~400×, clearly mislabeled
    let release = parse_release_metadata("Anime.2024.720p.WEB-DL.H.265");
    let w = balanced_weights();
    let size_300gb = 300 * 1024 * 1024 * 1024_i64;

    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, Some(size_300gb), Some("anime"), None, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"size_implausible_for_quality".to_string())
    );
}

#[test]
fn size_excessive_penalizes_oversized_anime() {
    // 3 GB for a 720p anime Blu-ray episode is far outside the anime envelope.
    // (720p anime baseline = 0.6 GiB × 1.35 BLURAY = 0.81 GiB; 3/0.81 = 3.7 → excessive)
    let release = parse_release_metadata("Anime.2024.720p.BluRay.H.265");
    let w = balanced_weights();
    let size_3gb = 3 * 1024 * 1024 * 1024_i64;

    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, Some(size_3gb), Some("anime"), None, &w);
    assert!(d.allowed);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "size_excessive_for_quality" && e.delta == w.size_excessive)
    );
}

#[test]
fn large_balanced_anime_remux_gets_size_penalty_with_explicit_remux_preference() {
    let profile = QualityProfile::parse(
        r#"{"id":"anime","name":"Anime","criteria":{"quality_tiers":["1080P","720P"],"prefer_remux":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let w = balanced_weights();
    let release = parse_release_metadata("Anime.S03E10.1080p.FLAC.2.0.AVC.REMUX-FraMeSToR");
    let size_7gb = 7 * 1024 * 1024 * 1024_i64;

    let mut d = evaluate_against_profile(&profile, &release, false, &w);
    apply_size_scoring_for_category(
        &mut d,
        &release,
        Some(size_7gb),
        Some("anime"),
        Some(24),
        &w,
    );

    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "prefer_remux_match" && e.delta == 250)
    );
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "size_excessive_for_quality" && e.delta == w.size_excessive)
    );
}

#[test]
fn size_large_bluray_remux_remains_eligible() {
    let release = parse_release_metadata("Movie.2024.2160p.BluRay.Remux.H.265.DTS-HD");
    let w = balanced_weights();
    let size_65gb = 65 * 1024 * 1024 * 1024_i64;

    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, Some(size_65gb), None, None, &w);
    assert!(d.allowed);
}

#[test]
fn size_scoring_accepts_plausible_8k_av1_webdl() {
    let release = parse_release_metadata("Movie.2026.4320p.WEB-DL.AV1.AAC");
    let w = balanced_weights();
    let size_65gb = 65 * 1024 * 1024 * 1024_i64;

    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, Some(size_65gb), None, Some(120), &w);

    assert!(d.allowed);
    assert_eq!(
        release.video_codec.as_ref(),
        Some(&crate::release_parser::VideoCodec::Av1)
    );
    assert!(
        !d.block_codes
            .contains(&"size_implausible_for_quality".to_string())
    );
}

#[test]
fn size_scoring_treats_hevc_and_h265_identically() {
    let hevc = parse_release_metadata("Movie.2024.2160p.BluRay.Remux.HEVC.DTS-HD");
    let h265 = parse_release_metadata("Movie.2024.2160p.BluRay.Remux.H.265.DTS-HD");
    let w = balanced_weights();
    let size_56gb = 56 * 1024 * 1024 * 1024_i64;

    let mut hevc_decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut hevc_decision, &hevc, Some(size_56gb), None, None, &w);

    let mut h265_decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut h265_decision, &h265, Some(size_56gb), None, None, &w);

    let hevc_size_code = hevc_decision
        .scoring_log
        .iter()
        .find(|entry| entry.code.starts_with("size_"))
        .map(|entry| entry.code.as_str());
    let h265_size_code = h265_decision
        .scoring_log
        .iter()
        .find(|entry| entry.code.starts_with("size_"))
        .map(|entry| entry.code.as_str());

    assert_eq!(
        hevc.video_codec.as_ref(),
        Some(&crate::release_parser::VideoCodec::H265)
    );
    assert_eq!(
        h265.video_codec.as_ref(),
        Some(&crate::release_parser::VideoCodec::H265)
    );
    assert_eq!(hevc_size_code, h265_size_code);
    assert_eq!(hevc_decision.release_score, h265_decision.release_score);
}

#[test]
fn size_scoring_treats_avc_and_h264_identically() {
    let avc = parse_release_metadata("Movie.2024.1080p.BluRay.AVC.DTS-HD");
    let h264 = parse_release_metadata("Movie.2024.1080p.BluRay.H.264.DTS-HD");
    let w = balanced_weights();
    let size_12gb = 12 * 1024 * 1024 * 1024_i64;

    let mut avc_decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut avc_decision, &avc, Some(size_12gb), None, None, &w);

    let mut h264_decision = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut h264_decision, &h264, Some(size_12gb), None, None, &w);

    let avc_size_code = avc_decision
        .scoring_log
        .iter()
        .find(|entry| entry.code.starts_with("size_"))
        .map(|entry| entry.code.as_str());
    let h264_size_code = h264_decision
        .scoring_log
        .iter()
        .find(|entry| entry.code.starts_with("size_"))
        .map(|entry| entry.code.as_str());

    assert_eq!(
        avc.video_codec.as_ref(),
        Some(&crate::release_parser::VideoCodec::H264)
    );
    assert_eq!(
        h264.video_codec.as_ref(),
        Some(&crate::release_parser::VideoCodec::H264)
    );
    assert_eq!(avc_size_code, h264_size_code);
    assert_eq!(avc_decision.release_score, h264_decision.release_score);
}

// ── QualityProfileDecision::log ───────────────────────────────────────────

#[test]
fn decision_log_tracks_entries() {
    let mut d = QualityProfileDecision::new();
    d.log("test_bonus", 100);
    d.log("test_penalty", -50);
    assert_eq!(d.release_score, 50);
    assert_eq!(d.preference_score, 50);
    assert_eq!(d.scoring_log.len(), 2);
    assert!(d.allowed);
}

#[test]
fn decision_log_block_sets_not_allowed() {
    let mut d = QualityProfileDecision::new();
    d.log("test_bonus", 100);
    d.log("blocked_rule", BLOCK_SCORE);
    assert!(!d.allowed);
    assert_eq!(d.block_codes, vec!["blocked_rule"]);
    assert_eq!(d.release_score, 100 + BLOCK_SCORE);
}

// ── Phase B: channel scoring ─────────────────────────────────────────────

#[test]
fn channel_71_gets_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.2160p.BluRay.TrueHD.7.1.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "audio_channels" && e.delta == 30)
    );
}

#[test]
fn channel_51_gets_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.2160p.BluRay.TrueHD.5.1.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "audio_channels" && e.delta == 15)
    );
}

#[test]
fn channel_20_is_neutral() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.DDP2.0.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    // 2.0 channels = 0 delta, so no audio_channels entry in log
    assert!(!d.scoring_log.iter().any(|e| e.code == "audio_channels"));
}

// ── Phase B: Atmos-aware audio scoring ───────────────────────────────────

#[test]
fn audiophile_truehd_atmos_outscores_truehd() {
    let profile = QualityProfile::default();
    let aud = crate::scoring_weights::build_weights(
        &crate::scoring_weights::ScoringPersona::Audiophile,
        &crate::scoring_weights::ScoringOverrides::default(),
    );

    let with_atmos = parse_release_metadata("Movie.2024.2160p.BluRay.TrueHD.Atmos.7.1.H.265");
    let no_atmos = parse_release_metadata("Movie.2024.2160p.BluRay.TrueHD.7.1.H.265");

    let d_atmos = evaluate_against_profile(&profile, &with_atmos, false, &aud);
    let d_plain = evaluate_against_profile(&profile, &no_atmos, false, &aud);

    assert!(d_atmos.preference_score > d_plain.preference_score);
}

#[test]
fn balanced_truehd_atmos_same_as_truehd() {
    let profile = QualityProfile::default();
    let w = balanced_weights();

    let with_atmos = parse_release_metadata("Movie.2024.2160p.BluRay.TrueHD.Atmos.5.1.H.265");
    let no_atmos = parse_release_metadata("Movie.2024.2160p.BluRay.TrueHD.5.1.H.265");

    let d_atmos = evaluate_against_profile(&profile, &with_atmos, false, &w);
    let d_plain = evaluate_against_profile(&profile, &no_atmos, false, &w);

    // Balanced treats Atmos+TrueHD the same as TrueHD. Atmos is now persona-native
    // for Audiophile only, so Balanced should not add a separate Atmos bias.
    let atmos_codec_score: i32 = d_atmos
        .scoring_log
        .iter()
        .filter(|e| e.code.starts_with("audio_codec"))
        .map(|e| e.delta)
        .sum();
    let plain_codec_score: i32 = d_plain
        .scoring_log
        .iter()
        .filter(|e| e.code.starts_with("audio_codec"))
        .map(|e| e.delta)
        .sum();
    assert_eq!(atmos_codec_score, plain_codec_score);
    assert!(
        !d_atmos
            .scoring_log
            .iter()
            .any(|e| e.code.starts_with("atmos_preferred"))
    );
}

// ── Phase B: DTS-X scoring ──────────────────────────────────────────────

#[test]
fn dtsx_scores_as_lossless() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.2160p.BluRay.DTS-X.7.1.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "audio_codec_lossless" && e.delta == 60)
    );
}

// ── Phase E: repack bonus ────────────────────────────────────────────────

#[test]
fn repack_gets_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.REPACK.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "repack_upload" && e.delta == 30)
    );
}

#[test]
fn proper_without_repack_has_no_repack_entry() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.PROPER.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(d.scoring_log.iter().any(|e| e.code == "proper_upload"));
    assert!(!d.scoring_log.iter().any(|e| e.code == "repack_upload"));
}

// ── Phase E: hardcoded subs penalty ──────────────────────────────────────

#[test]
fn hardcoded_subs_penalty() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.HC.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "hardcoded_subs" && e.delta == -300)
    );
}

#[test]
fn no_hardcoded_subs_no_penalty() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.scoring_log.iter().any(|e| e.code == "hardcoded_subs"));
}

// ── Phase E: edition scoring ─────────────────────────────────────────────

#[test]
fn edition_imax_gets_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.IMAX.2160p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "edition_bonus" && e.delta == 80)
    );
}

#[test]
fn edition_extended_gets_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.EXTENDED.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "edition_bonus" && e.delta == 40)
    );
}

#[test]
fn edition_criterion_gets_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.Criterion.1080p.BluRay.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "edition_bonus" && e.delta == 20)
    );
}

#[test]
fn no_edition_no_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.scoring_log.iter().any(|e| e.code == "edition_bonus"));
}

// ── Phase E: streaming service tier scoring ──────────────────────────────

#[test]
fn streaming_tier1_gets_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.AMZN.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "streaming_service" && e.delta == 30)
    );
}

#[test]
fn streaming_tier2_gets_smaller_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.HMAX.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "streaming_service" && e.delta == 20)
    );
}

#[test]
fn streaming_anime_tier_for_crunchyroll() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Anime.S01E01.1080p.CR.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "streaming_service" && e.delta == 20)
    );
}

#[test]
fn no_streaming_service_no_entry() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.BluRay.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.scoring_log.iter().any(|e| e.code == "streaming_service"));
}

// ── Phase E: SDR at 4K penalty ───────────────────────────────────────────

#[test]
fn sdr_at_4k_penalty() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "sdr_at_4k" && e.delta == -150)
    );
}

#[test]
fn hdr_at_4k_no_penalty() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.HDR.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.scoring_log.iter().any(|e| e.code == "sdr_at_4k"));
}

#[test]
fn sdr_at_1080p_no_penalty() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.scoring_log.iter().any(|e| e.code == "sdr_at_4k"));
}

#[test]
fn dv_without_hdr_fallback_blocks_when_override_enabled() {
    let profile = QualityProfile::default();
    let w = crate::scoring_weights::build_weights(
        &crate::scoring_weights::ScoringPersona::Balanced,
        &crate::scoring_weights::ScoringOverrides {
            block_dv_without_fallback: Some(true),
            ..crate::scoring_weights::ScoringOverrides::default()
        },
    );
    let mut release = parse_release_metadata("Movie.2024.2160p.WEB-DL.DV.H.265");
    release.has_hdr_fallback = false;
    release.is_hdr10plus = false;
    release.is_hlg = false;
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(!d.allowed);
    assert!(
        d.block_codes
            .contains(&"dolby_vision_missing_hdr_fallback".to_string())
    );
}

#[test]
fn dv_with_hdr_fallback_is_allowed_when_override_enabled() {
    let profile = QualityProfile::default();
    let w = crate::scoring_weights::build_weights(
        &crate::scoring_weights::ScoringPersona::Balanced,
        &crate::scoring_weights::ScoringOverrides {
            block_dv_without_fallback: Some(true),
            ..crate::scoring_weights::ScoringOverrides::default()
        },
    );
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.DV.HDR.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(d.allowed);
    assert!(
        !d.block_codes
            .contains(&"dolby_vision_missing_hdr_fallback".to_string())
    );
}

// ── Phase E: anime version bonus ─────────────────────────────────────────

#[test]
fn anime_v2_gets_bonus() {
    let profile = QualityProfile::default();
    let w = crate::scoring_weights::build_weights_for_category(
        &crate::scoring_weights::ScoringPersona::Balanced,
        &crate::scoring_weights::ScoringOverrides::default(),
        Some("anime"),
    );
    let release = parse_release_metadata("[Group] Anime Title - 01v2 [1080p] [HEVC]");
    let d = evaluate_against_profile_for_category(&profile, &release, false, &w, Some("anime"));
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "anime_version_bonus" && e.delta == 20)
    );
}

#[test]
fn no_anime_version_no_entry() {
    let profile = QualityProfile::default();
    let w = crate::scoring_weights::build_weights_for_category(
        &crate::scoring_weights::ScoringPersona::Balanced,
        &crate::scoring_weights::ScoringOverrides::default(),
        Some("anime"),
    );
    let release = parse_release_metadata("[Group] Anime Title - 01 [1080p] [HEVC]");
    let d = evaluate_against_profile_for_category(&profile, &release, false, &w, Some("anime"));
    assert!(
        !d.scoring_log
            .iter()
            .any(|e| e.code == "anime_version_bonus")
    );
}

#[test]
fn anime_10bit_uncensored_and_dubs_only_are_scored() {
    let profile = QualityProfile::default();
    let w = crate::scoring_weights::build_weights_for_category(
        &crate::scoring_weights::ScoringPersona::Balanced,
        &crate::scoring_weights::ScoringOverrides::default(),
        Some("anime"),
    );
    let mut release = parse_release_metadata("[Group] Anime Title - 01v2 [1080p] [HEVC]");
    release.is_10bit = true;
    release.is_uncensored = true;
    release.is_dubs_only = true;

    let d = evaluate_against_profile_for_category(&profile, &release, false, &w, Some("anime"));
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "anime_10bit_bonus" && e.delta == 40)
    );
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "anime_uncensored_bonus" && e.delta == 30)
    );
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "anime_dubs_only" && e.delta == -100)
    );
}

#[test]
fn anime_audiophile_has_no_missing_atmos_penalty() {
    let profile = QualityProfile::default();
    let w = crate::scoring_weights::build_weights_for_category(
        &crate::scoring_weights::ScoringPersona::Audiophile,
        &crate::scoring_weights::ScoringOverrides::default(),
        Some("anime"),
    );
    let release = parse_release_metadata("[Group] Anime Title - 01 [1080p] [HEVC]");
    let d = evaluate_against_profile_for_category(&profile, &release, false, &w, Some("anime"));
    assert!(
        !d.scoring_log
            .iter()
            .any(|e| e.code == "atmos_preferred_missing")
    );
}

// ── Phase E: AI enhanced penalty ─────────────────────────────────────────

#[test]
fn ai_enhanced_gets_block_score() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.AI.Enhanced.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "ai_enhanced_upscaled" && e.delta == BLOCK_SCORE)
    );
    assert!(!d.allowed);
}

#[test]
fn trash_guides_blocked_title_gets_block_score() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let mut release = parse_release_metadata("Series.Name.2160p.BiTOR.WEB-DL");
    release.guide_facts.push(scryer_release_parser::GuideFact {
        code: "trash.blocked.lq_release_title".to_string(),
    });
    let d = evaluate_against_profile_for_category(&profile, &release, false, &w, Some("series"));
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "trash_guides_lq_release_title" && e.delta == BLOCK_SCORE)
    );
    assert!(!d.allowed);
}

// ── Phase E: release group reputation ────────────────────────────────────

#[test]
fn known_gold_web_group_gets_bonus() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    // NTb is a Gold-tier WEB group in the release group database
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265-NTb");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "group_gold" && e.delta == 300)
    );
}

#[test]
fn unknown_group_gets_minor_penalty() {
    let profile = QualityProfile::default();
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265-XYZNOGROUP");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "group_unknown" && e.delta == -30)
    );
}

// ── Phase E: persona affects scoring ─────────────────────────────────────

#[test]
fn audiophile_persona_boosts_truehd_atmos_heavily() {
    let profile = QualityProfile::default();
    let aud = crate::scoring_weights::build_weights(
        &crate::scoring_weights::ScoringPersona::Audiophile,
        &crate::scoring_weights::ScoringOverrides::default(),
    );
    let release = parse_release_metadata("Movie.2024.2160p.BluRay.TrueHD.Atmos.7.1.H.265");
    let d = evaluate_against_profile(&profile, &release, false, &aud);
    // Audiophile TrueHD Atmos = 400
    assert!(
        d.scoring_log
            .iter()
            .any(|e| e.code == "audio_codec_lossless" && e.delta == 400)
    );
}

#[test]
fn efficient_persona_prefers_webdl_over_bluray() {
    let profile = QualityProfile::default();
    let eff = crate::scoring_weights::build_weights(
        &crate::scoring_weights::ScoringPersona::Efficient,
        &crate::scoring_weights::ScoringOverrides::default(),
    );
    let webdl = parse_release_metadata("Movie.2024.2160p.WEB-DL.H.265");
    let bluray = parse_release_metadata("Movie.2024.2160p.BluRay.H.265");

    let d_web = evaluate_against_profile(&profile, &webdl, false, &eff);
    let d_br = evaluate_against_profile(&profile, &bluray, false, &eff);

    let web_source: i32 = d_web
        .scoring_log
        .iter()
        .filter(|e| e.code.starts_with("source_"))
        .map(|e| e.delta)
        .sum();
    let br_source: i32 = d_br
        .scoring_log
        .iter()
        .filter(|e| e.code.starts_with("source_"))
        .map(|e| e.delta)
        .sum();
    assert!(web_source > br_source);
}

// ── Phase F: min score to grab ──────────────────────────────────────────

#[test]
fn min_score_blocks_low_scoring_release() {
    let mut profile = QualityProfile::default();
    profile.criteria.min_score_to_grab = Some(5000);
    let w = balanced_weights();
    // A basic 1080p WEB-DL will score well below 5000
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let mut d = evaluate_against_profile(&profile, &release, false, &w);
    apply_min_score_gate(&profile, &mut d);
    assert!(!d.allowed);
    assert!(d.block_codes.contains(&"score_below_minimum".to_string()));
}

#[test]
fn min_score_allows_high_scoring_release() {
    let mut profile = QualityProfile::default();
    profile.criteria.min_score_to_grab = Some(100);
    profile.criteria.prefer_remux = true;
    let w = balanced_weights();
    // Top-tier 2160p should easily exceed 100
    let release = parse_release_metadata("Movie.2024.2160p.BluRay.Remux.TrueHD.Atmos.7.1.H.265");
    let mut d = evaluate_against_profile(&profile, &release, false, &w);
    apply_min_score_gate(&profile, &mut d);
    assert!(d.allowed);
    assert!(!d.block_codes.contains(&"score_below_minimum".to_string()));
}

#[test]
fn min_score_none_does_not_block() {
    let mut profile = QualityProfile::default();
    profile.criteria.min_score_to_grab = None;
    let w = balanced_weights();
    let release = parse_release_metadata("Movie.2024.720p.HDTV.H.264");
    let d = evaluate_against_profile(&profile, &release, false, &w);
    // Even a low quality release is allowed when no min_score is set
    assert!(d.allowed);
}

// ── Phase F: cutoff tier ────────────────────────────────────────────────
//
// The `has_reached_cutoff` pair is gone (MA2). A scope's cutoff is decided by
// what is on **disk**, through `quality_meets_or_exceeds_cutoff` over the
// weakest member's quality; parsing the anchor row's `grabbed_release` said a
// twelve-episode season had reached cutoff on the strength of one member's
// unlanded grab. "Something is already in flight" is D18's question now.

#[test]
fn cutoff_reached_when_current_quality_at_cutoff() {
    assert!(quality_meets_or_exceeds_cutoff(
        "1080p",
        "1080P",
        &["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
    ));
}

#[test]
fn cutoff_reached_when_current_quality_above_cutoff() {
    assert!(quality_meets_or_exceeds_cutoff(
        "2160p",
        "1080P",
        &["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
    ));
}

#[test]
fn cutoff_not_reached_when_below() {
    assert!(!quality_meets_or_exceeds_cutoff(
        "720p",
        "1080P",
        &["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
    ));
}

#[test]
fn quality_meets_or_exceeds_cutoff_normalizes_input_tiers() {
    let reached = quality_meets_or_exceeds_cutoff(
        "1080p",
        "720p",
        &["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
    );
    assert!(reached);
}

#[test]
fn quality_meets_or_exceeds_cutoff_rejects_unrecognized_current_tier() {
    let reached = quality_meets_or_exceeds_cutoff(
        "dvd",
        "720P",
        &["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
    );
    assert!(!reached);
}

/// Issue #170: a bare `Sxx` trailed by unmistakable release metadata is a
/// title boundary in the neutral (context-free) parse. Without this, the
/// neutral title read "Quiet Meridian S01 iTALiAN" and the candidate failed
/// title matching before its real title was ever compared.
#[test]
fn bare_season_before_release_metadata_bounds_the_neutral_title() {
    let parsed = parse_release_metadata(
        "Quiet.Meridian.S01.iTALiAN.MULTi.1080p.DSNP.WEB-DL.DDP5.1.H.264-GRP",
    );
    assert_eq!(parsed.normalized_title, "QUIET MERIDIAN");

    // An isolated `Sxx` with no metadata trailer keeps the strict rule: a
    // title token that merely looks like a season must not truncate the title.
    let isolated = parse_release_metadata("Quiet.Meridian.S01");
    assert!(
        isolated.normalized_title.starts_with("QUIET MERIDIAN"),
        "unexpected title: {}",
        isolated.normalized_title
    );
}

/// A spelled-out season plus an arc name defeated the same title anchor: the
/// neutral title read "Quiet Meridian Season 1 The Arc Name" and the candidate
/// failed title matching before its real title was ever compared.
#[test]
fn a_spelled_season_number_bounds_the_neutral_title() {
    let release = "[GroupTag] Quiet Meridian - Season 1 - The Arc Name [BD 1080p][HEVC x265 10bit]";
    let parsed = parse_release_metadata(release);
    assert_eq!(parsed.normalized_title, "QUIET MERIDIAN", "{release}");
}

/// The numeric successor is what makes the word a marker; without one the title
/// keeps its own text. "Part 2" never bounds at all — in a sequel or multi-part
/// title it is the part of the name that tells releases apart.
#[test]
fn a_spelled_season_without_a_number_keeps_the_title_text() {
    for release in [
        "[GroupTag] Quiet Meridian Season of the Lantern [BD 1080p]",
        "Quiet.Meridian.Part.Two.1080p.BluRay.x264-GroupTag",
        "Quiet.Meridian.Part.2.1080p.BluRay.x264-GroupTag",
    ] {
        let parsed = parse_release_metadata(release);
        assert!(
            parsed.normalized_title.len() > "QUIET MERIDIAN".len(),
            "{release} truncated to {}",
            parsed.normalized_title
        );
    }
}
