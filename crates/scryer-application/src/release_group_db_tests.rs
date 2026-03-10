use super::*;
use crate::scoring_weights::{build_weights, ScoringOverrides, ScoringPersona};

fn balanced_weights() -> ScoringWeights {
    build_weights(&ScoringPersona::Balanced, &ScoringOverrides::default())
}

// ── lookup_group ─────────────────────────────────────────────────────────────

#[test]
fn gold_web_group_found() {
    let entry = lookup_group("FLUX", Some("WEB-DL"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Gold);
    assert_eq!(entry.source_context, SourceContext::Web);
}

#[test]
fn silver_web_group_found() {
    let entry = lookup_group("SMURF", Some("WEB-DL"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Silver);
}

#[test]
fn bronze_web_group_found() {
    let entry = lookup_group("BLOOM", Some("WEB-DL"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Bronze);
}

#[test]
fn banned_lq_group_found_any_source() {
    let entry = lookup_group("YIFY", Some("BLURAY"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Banned);
    assert_eq!(entry.source_context, SourceContext::Any);
}

#[test]
fn banned_group_found_without_source() {
    let entry = lookup_group("RARBG", None, false).unwrap();
    assert_eq!(entry.tier, GroupTier::Banned);
}

#[test]
fn unknown_group_returns_none() {
    assert!(lookup_group("SomeRandomGroup2025", Some("WEB-DL"), false).is_none());
}

// ── Source context matching ──────────────────────────────────────────────────

#[test]
fn ctrlhd_gold_for_web() {
    let entry = lookup_group("CtrlHD", Some("WEB-DL"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Gold);
    assert_eq!(entry.source_context, SourceContext::Web);
}

#[test]
fn ctrlhd_gold_for_bluray() {
    let entry = lookup_group("CtrlHD", Some("BLURAY"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Gold);
    assert_eq!(entry.source_context, SourceContext::BluRay);
}

#[test]
fn ctrlhd_gold_for_uhd_bluray_via_remux_false() {
    // UHD BluRay without remux flag — source is BLURAY, matches BluRay context
    let entry = lookup_group("CtrlHD", Some("BLURAY"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Gold);
}

#[test]
fn remux_group_found_with_remux_flag() {
    let entry = lookup_group("FraMeSToR", Some("BLURAY"), true).unwrap();
    assert_eq!(entry.tier, GroupTier::Gold);
    assert_eq!(entry.source_context, SourceContext::Remux);
}

#[test]
fn remux_group_not_found_without_remux_flag() {
    // FraMeSToR is only in Remux context, not BluRay
    assert!(lookup_group("FraMeSToR", Some("BLURAY"), false).is_none());
}

// ── Anime context ────────────────────────────────────────────────────────────

#[test]
fn anime_banned_group_not_found_for_non_anime_source() {
    // AnimeRG is banned in Anime context only
    // With a WEB-DL source, it tries Web context first, then Any — neither has AnimeRG
    assert!(lookup_group("AnimeRG", Some("WEB-DL"), false).is_none());
}

#[test]
fn global_banned_group_found_for_any_source() {
    // YIFY is banned in Any context — found regardless of source
    let entry = lookup_group("YIFY", Some("WEB-DL"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Banned);
}

// ── Case insensitivity ───────────────────────────────────────────────────────

#[test]
fn lookup_is_case_insensitive() {
    let entry = lookup_group("flux", Some("WEB-DL"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Gold);

    let entry = lookup_group("FLUX", Some("WEB-DL"), false).unwrap();
    assert_eq!(entry.tier, GroupTier::Gold);
}

// ── apply_release_group_scoring ──────────────────────────────────────────────

#[test]
fn gold_group_gets_gold_score() {
    let w = balanced_weights();
    let (code, delta) = apply_release_group_scoring(&w, Some("FLUX"), Some("WEB-DL"), false);
    assert_eq!(code, "group_gold");
    assert_eq!(delta, w.group_gold);
}

#[test]
fn silver_group_gets_silver_score() {
    let w = balanced_weights();
    let (code, delta) = apply_release_group_scoring(&w, Some("SMURF"), Some("WEB-DL"), false);
    assert_eq!(code, "group_silver");
    assert_eq!(delta, w.group_silver);
}

#[test]
fn bronze_group_gets_bronze_score() {
    let w = balanced_weights();
    let (code, delta) = apply_release_group_scoring(&w, Some("BLOOM"), Some("WEB-DL"), false);
    assert_eq!(code, "group_bronze");
    assert_eq!(delta, w.group_bronze);
}

#[test]
fn banned_group_gets_block_score() {
    let w = balanced_weights();
    let (code, delta) = apply_release_group_scoring(&w, Some("YIFY"), Some("BLURAY"), false);
    assert_eq!(code, "group_banned");
    assert_eq!(delta, w.group_banned);
}

#[test]
fn unknown_group_gets_penalty() {
    let w = balanced_weights();
    let (code, delta) = apply_release_group_scoring(&w, Some("UnknownGroup"), Some("WEB-DL"), false);
    assert_eq!(code, "group_unknown");
    assert_eq!(delta, w.group_unknown_penalty);
}

#[test]
fn no_group_gets_unknown_penalty() {
    let w = balanced_weights();
    let (code, delta) = apply_release_group_scoring(&w, None, Some("WEB-DL"), false);
    assert_eq!(code, "group_unknown");
    assert_eq!(delta, w.group_unknown_penalty);
}

#[test]
fn empty_group_gets_unknown_penalty() {
    let w = balanced_weights();
    let (code, delta) = apply_release_group_scoring(&w, Some(""), Some("WEB-DL"), false);
    assert_eq!(code, "group_unknown");
    assert_eq!(delta, w.group_unknown_penalty);
}

// ── Persona-specific weights ─────────────────────────────────────────────────

#[test]
fn audiophile_gold_higher_than_balanced() {
    let bal = balanced_weights();
    let aud = build_weights(&ScoringPersona::Audiophile, &ScoringOverrides::default());
    let (_, bal_delta) = apply_release_group_scoring(&bal, Some("FLUX"), Some("WEB-DL"), false);
    let (_, aud_delta) = apply_release_group_scoring(&aud, Some("FLUX"), Some("WEB-DL"), false);
    assert!(aud_delta > bal_delta);
}

#[test]
fn bad_dual_audio_group_is_banned() {
    let w = balanced_weights();
    let (code, _) = apply_release_group_scoring(&w, Some("alfaHD"), Some("WEB-DL"), false);
    assert_eq!(code, "group_banned");
}

#[test]
fn remux_tier_01_scores_gold() {
    let w = balanced_weights();
    let (code, delta) = apply_release_group_scoring(&w, Some("FraMeSToR"), Some("BLURAY"), true);
    assert_eq!(code, "group_gold");
    assert_eq!(delta, w.group_gold);
}
