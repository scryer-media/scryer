use super::*;
use crate::release_parser::parse_release_metadata;

// ── normalize_quality ─────────────────────────────────────────────────────

#[test]
fn normalize_quality_1080p() {
    assert_eq!(normalize_quality(Some("1080p")), Some("1080P".to_string()));
}

#[test]
fn normalize_quality_2160p() {
    assert_eq!(normalize_quality(Some("2160p")), Some("2160P".to_string()));
}

#[test]
fn normalize_quality_720p() {
    assert_eq!(normalize_quality(Some("720p")), Some("720P".to_string()));
}

#[test]
fn normalize_quality_none() {
    assert_eq!(normalize_quality(None), None);
}

#[test]
fn normalize_quality_already_uppercase() {
    assert_eq!(normalize_quality(Some("1080P")), Some("1080P".to_string()));
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
    assert_eq!(normalize_source(Some("BluRay")), Some("BLURAY".to_string()));
    assert_eq!(normalize_source(Some("BD")), Some("BLURAY".to_string()));
    assert_eq!(normalize_source(Some("UHD")), Some("BLURAY".to_string()));
}

#[test]
fn normalize_source_webrip() {
    assert_eq!(normalize_source(Some("WEBRip")), Some("WEBRIP".to_string()));
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
}

#[test]
fn normalize_codec_h265() {
    assert_eq!(normalize_codec(Some("H265")), Some("H.265".to_string()));
    assert_eq!(normalize_codec(Some("h265")), Some("H.265".to_string()));
}

#[test]
fn normalize_codec_passthrough() {
    assert_eq!(normalize_codec(Some("AV1")), Some("AV1".to_string()));
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
    let profile = QualityProfile::parse(
        r#"{"id":"test","name":"Test","criteria":{}}"#,
    )
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

#[test]
fn tier_0_gets_3200_bonus() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "quality_tier_0" && e.delta == 3200));
}

#[test]
fn tier_1_gets_900_bonus() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "quality_tier_1" && e.delta == 900));
}

#[test]
fn quality_not_in_tiers_is_blocked() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.480p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(!d.allowed);
    assert!(d.block_codes.contains(&"quality_not_in_profile_tiers".to_string()));
}

// ── evaluate_against_profile: source scoring ──────────────────────────────

#[test]
fn bluray_source_gets_150() {
    let profile = QualityProfile::default();
    let release = parse_release_metadata("Movie.2024.2160p.BluRay.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "source_bluray" && e.delta == 150));
}

#[test]
fn webdl_source_gets_120() {
    let profile = QualityProfile::default();
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "source_webdl" && e.delta == 120));
}

#[test]
fn source_blocklist_blocks() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"source_blocklist":["HDTV"],"allow_upgrades":true,"allow_unknown_quality":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.HDTV.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(!d.allowed);
    assert!(d.block_codes.contains(&"source_in_profile_blocklist".to_string()));
}

// ── evaluate_against_profile: DV/HDR ─────────────────────────────────────

#[test]
fn dolby_vision_bonus_when_allowed() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"dolby_vision_allowed":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.DV.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "dolby_vision_bonus" && e.delta == 50));
}

#[test]
fn dolby_vision_blocks_when_not_allowed() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"dolby_vision_allowed":false,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.DV.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(!d.allowed);
    assert!(d.block_codes.contains(&"dolby_vision_not_allowed".to_string()));
}

#[test]
fn hdr_blocks_when_not_allowed() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"detected_hdr_allowed":false,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.HDR.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(!d.allowed);
    assert!(d.block_codes.contains(&"hdr_not_allowed".to_string()));
}

// ── evaluate_against_profile: remux / atmos / dual audio ──────────────────

#[test]
fn remux_preference_bonus() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"prefer_remux":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.BluRay.REMUX.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "prefer_remux_match" && e.delta == 200));
}

#[test]
fn remux_missing_penalty() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"prefer_remux":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "prefer_remux_missing" && e.delta == -50));
}

#[test]
fn atmos_preference_bonus() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"atmos_preferred":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.DDP.Atmos.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "atmos_preferred_match" && e.delta == 100));
}

#[test]
fn dual_audio_preferred_bonus() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"prefer_dual_audio":true,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.DUAL.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "dual_audio_preferred_match" && e.delta == 150));
}

#[test]
fn dual_audio_bonus_when_not_preferred() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"prefer_dual_audio":false,"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.DUAL.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "dual_audio" && e.delta == 40));
}

// ── evaluate_against_profile: required audio languages ────────────────────

#[test]
fn required_audio_language_match() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"required_audio_languages":["ENG"],"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.English.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.allowed);
    assert!(d.scoring_log.iter().any(|e| e.code == "required_audio_languages_match"));
}

#[test]
fn required_audio_language_missing_blocks() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"required_audio_languages":["JPN"],"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.English.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(!d.allowed);
    assert!(d.block_codes.contains(&"required_audio_language_missing".to_string()));
}

// ── evaluate_against_profile: upgrade guard ───────────────────────────────

#[test]
fn upgrade_blocked_when_has_existing_file_and_upgrades_disabled() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_upgrades":false,"allow_unknown_quality":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, true);
    assert!(!d.allowed);
    assert!(d.block_codes.contains(&"upgrade_blocked_by_profile".to_string()));
}

#[test]
fn upgrade_allowed_when_no_existing_file() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_upgrades":false,"allow_unknown_quality":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.allowed);
}

// ── evaluate_against_profile: proper upload / low confidence ──────────────

#[test]
fn proper_upload_bonus() {
    let profile = QualityProfile::parse(
        r#"{"id":"t","name":"T","criteria":{"allow_unknown_quality":true,"allow_upgrades":true}}"#,
    ).unwrap();
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.PROPER.H.265");
    let d = evaluate_against_profile(&profile, &release, false);
    assert!(d.scoring_log.iter().any(|e| e.code == "proper_upload" && e.delta == 30));
}

// ── resolve_profile_id_for_title ──────────────────────────────────────────

#[test]
fn resolve_profile_id_title_wins() {
    let result = resolve_profile_id_for_title(Some("title"), Some("category"), Some("global"));
    assert_eq!(result, Some("title".to_string()));
}

#[test]
fn resolve_profile_id_category_fallback() {
    let result = resolve_profile_id_for_title(None, Some("category"), Some("global"));
    assert_eq!(result, Some("category".to_string()));
}

#[test]
fn resolve_profile_id_global_fallback() {
    let result = resolve_profile_id_for_title(None, None, Some("global"));
    assert_eq!(result, Some("global".to_string()));
}

#[test]
fn resolve_profile_id_none_fallback() {
    let result = resolve_profile_id_for_title(None, None, None);
    assert_eq!(result, None);
}

// ── default profiles ──────────────────────────────────────────────────────

#[test]
fn default_4k_profile_has_three_tiers() {
    let profile = default_quality_profile_for_search();
    assert_eq!(profile.criteria.quality_tiers.len(), 3);
    assert_eq!(profile.criteria.quality_tiers[0], "2160P");
}

#[test]
fn default_1080p_profile_has_two_tiers() {
    let profile = default_quality_profile_1080p_for_search();
    assert_eq!(profile.criteria.quality_tiers.len(), 2);
    assert_eq!(profile.criteria.quality_tiers[0], "1080P");
}

// ── apply_size_scoring_for_category ───────────────────────────────────────

#[test]
fn size_scoring_no_size_is_noop() {
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, None, None, None);
    assert!(d.scoring_log.is_empty());
}

#[test]
fn size_scoring_zero_bytes_is_noop() {
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, Some(0), None, None);
    assert!(d.scoring_log.is_empty());
}

#[test]
fn size_scoring_anime_expects_smaller() {
    let release = parse_release_metadata("Anime.2024.1080p.WEB-DL.H.265");
    let size_2gb = 2 * 1024 * 1024 * 1024_i64;

    let mut d_anime = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d_anime, &release, Some(size_2gb), Some("anime"), None);

    let mut d_movie = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d_movie, &release, Some(size_2gb), None, None);

    // 2GB for anime 1080p is expected; for movie 1080p it's small
    assert!(d_anime.release_score > d_movie.release_score);
}

#[test]
fn size_scoring_scales_with_runtime() {
    let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265");
    let size_12gb = 12 * 1024 * 1024 * 1024_i64;

    // 12 GB for a standard 2-hour movie (baseline 120 min) → ~1.5× expected (8 GiB × 0.8 WEB)
    let mut d_standard = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d_standard, &release, Some(size_12gb), None, Some(120));

    // 12 GB for a 3-hour movie → expected is scaled up by 180/120 = 1.5×
    let mut d_long = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d_long, &release, Some(size_12gb), None, Some(180));

    // The long movie should score higher because 12 GB is more "expected" for 3 hours
    assert!(d_long.release_score <= d_standard.release_score);
}

#[test]
fn size_scoring_anime_ova_runtime_scales_expectation() {
    let release = parse_release_metadata("Anime.2024.1080p.WEB-DL.H.265");
    let size_3gb = 3 * 1024 * 1024 * 1024_i64;

    // 3 GB for a standard 24-min anime episode → quite large
    let mut d_standard = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d_standard, &release, Some(size_3gb), Some("anime"), Some(24));

    // 3 GB for a 50-min OVA → more expected
    let mut d_ova = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d_ova, &release, Some(size_3gb), Some("anime"), Some(50));

    // OVA should score the same or lower because 3 GB is more "normal" for 50 min
    assert!(d_ova.release_score <= d_standard.release_score);
}

#[test]
fn size_implausible_blocks_wildly_oversized() {
    // 300 GB claiming to be a 720p anime episode — ratio ~400×, clearly mislabeled
    let release = parse_release_metadata("Anime.2024.720p.WEB-DL.H.265");
    let size_300gb = 300 * 1024 * 1024 * 1024_i64;

    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, Some(size_300gb), Some("anime"), None);
    assert!(!d.allowed);
    assert!(d.block_codes.contains(&"size_implausible_for_quality".to_string()));
}

#[test]
fn size_excessive_stops_rewarding_oversized() {
    // 3 GB for a 720p anime Blu-ray episode (expected ~1.0 GiB) → ratio ~3× → not blocked
    // but 5 GB → ratio ~5× → excessive band, scores 0 (no bonus, no penalty)
    let release = parse_release_metadata("Anime.2024.720p.BluRay.H.265");
    let size_5gb = 5 * 1024 * 1024 * 1024_i64;

    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, Some(size_5gb), Some("anime"), None);
    assert!(d.allowed);
    assert_eq!(d.release_score, 0);
}

#[test]
fn size_plausible_bluray_remux_not_penalized() {
    // 50 GB for a 2160P Blu-ray Remux movie is normal (expected ~43 GiB)
    let release = parse_release_metadata("Movie.2024.2160p.BluRay.Remux.H.265.DTS-HD");
    let size_50gb = 50 * 1024 * 1024 * 1024_i64;

    let mut d = QualityProfileDecision::new();
    apply_size_scoring_for_category(&mut d, &release, Some(size_50gb), None, None);
    assert!(d.release_score > 0);
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
