use super::*;

// ── Balanced preset matches legacy hardcoded values ───────────────────────

#[test]
fn balanced_source_matches_legacy() {
    let w = build_weights(&ScoringPersona::Balanced, &ScoringOverrides::default());
    assert_eq!(w.source_bluray, 150);
    assert_eq!(w.source_webdl, 120);
    assert_eq!(w.source_webrip, 80);
    assert_eq!(w.source_hdtv, 40);
}

#[test]
fn balanced_video_codec_matches_legacy() {
    let w = build_weights(&ScoringPersona::Balanced, &ScoringOverrides::default());
    assert_eq!(w.video_codec_high, 60);
    assert_eq!(w.video_codec_mid, 40);
}

#[test]
fn balanced_audio_codec_matches_legacy_tiers() {
    let w = build_weights(&ScoringPersona::Balanced, &ScoringOverrides::default());
    // Lossless tier = 60 (legacy: FLAC|TRUEHD => 60)
    assert_eq!(w.audio_flac, 60);
    assert_eq!(w.audio_truehd, 60);
    // High tier = 40 (legacy: DDP|DTS|DTSHD|DTSMA => 40)
    assert_eq!(w.audio_ddp, 40);
    assert_eq!(w.audio_dts, 40);
    assert_eq!(w.audio_dtshd, 40);
    assert_eq!(w.audio_dtsma, 40);
    // Standard tier = 20 (legacy: AC3|AAC => 20)
    assert_eq!(w.audio_ac3, 20);
    assert_eq!(w.audio_aac, 20);
}

#[test]
fn balanced_dv_hdr_matches_legacy() {
    let w = build_weights(&ScoringPersona::Balanced, &ScoringOverrides::default());
    assert_eq!(w.dolby_vision, 50);
    assert_eq!(w.hdr10, 30);
}

#[test]
fn balanced_features_match_legacy() {
    let w = build_weights(&ScoringPersona::Balanced, &ScoringOverrides::default());
    assert_eq!(w.remux_bonus, 200);
    assert_eq!(w.remux_missing_penalty, -50);
    assert_eq!(w.atmos_bonus, 100);
    assert_eq!(w.atmos_missing_penalty, -20);
    assert_eq!(w.dual_audio_bonus, 150);
    assert_eq!(w.dual_audio_missing_penalty, -30);
    assert_eq!(w.dual_audio_present, 40);
    assert_eq!(w.proper_bonus, 30);
}

#[test]
fn balanced_size_curve_matches_legacy() {
    let w = build_weights(&ScoringPersona::Balanced, &ScoringOverrides::default());
    assert_eq!(w.size_massive, 550);
    assert_eq!(w.size_very_large, 380);
    assert_eq!(w.size_large, 240);
    assert_eq!(w.size_expected, 120);
    assert_eq!(w.size_slightly_small, 0);
    assert_eq!(w.size_small, -700);
    assert_eq!(w.size_very_small, -1300);
    assert_eq!(w.size_tiny, -2500);
}

// ── Persona defaults are distinct ─────────────────────────────────────────

#[test]
fn audiophile_has_higher_bluray_bonus() {
    let bal = build_weights(&ScoringPersona::Balanced, &ScoringOverrides::default());
    let aud = build_weights(&ScoringPersona::Audiophile, &ScoringOverrides::default());
    assert!(aud.source_bluray > bal.source_bluray);
}

#[test]
fn efficient_prefers_webdl_over_bluray() {
    let w = build_weights(&ScoringPersona::Efficient, &ScoringOverrides::default());
    assert!(w.source_webdl > w.source_bluray);
}

#[test]
fn efficient_has_inverted_size_curve() {
    let w = build_weights(&ScoringPersona::Efficient, &ScoringOverrides::default());
    // Sweet spot is at slightly_small (compact encodes), not massive
    assert!(w.size_slightly_small > w.size_massive);
    assert!(w.size_expected > w.size_large);
}

#[test]
fn compatible_penalizes_dolby_vision() {
    let w = build_weights(&ScoringPersona::Compatible, &ScoringOverrides::default());
    assert!(w.dolby_vision < 0);
}

#[test]
fn compatible_prefers_h264_over_h265() {
    let w = build_weights(&ScoringPersona::Compatible, &ScoringOverrides::default());
    assert!(w.video_codec_mid > w.video_codec_high);
}

// ── Override application ──────────────────────────────────────────────────

#[test]
fn override_compact_encodes_swaps_size_curve() {
    let w = build_weights(
        &ScoringPersona::Balanced,
        &ScoringOverrides {
            prefer_compact_encodes: Some(true),
            ..ScoringOverrides::default()
        },
    );
    // Should now reward compact, matching Efficient curve
    assert!(w.size_slightly_small > w.size_massive);
}

#[test]
fn override_compact_off_on_efficient_uses_normal_curve() {
    let w = build_weights(
        &ScoringPersona::Efficient,
        &ScoringOverrides {
            prefer_compact_encodes: Some(false),
            ..ScoringOverrides::default()
        },
    );
    // Should use normal curve now
    assert_eq!(w.size_massive, 550);
    assert_eq!(w.size_slightly_small, 0);
}

#[test]
fn override_lossless_audio_boosts_lossless_codecs() {
    let base = build_weights(&ScoringPersona::Balanced, &ScoringOverrides::default());
    let boosted = build_weights(
        &ScoringPersona::Balanced,
        &ScoringOverrides {
            prefer_lossless_audio: Some(true),
            ..ScoringOverrides::default()
        },
    );
    assert!(boosted.audio_truehd > base.audio_truehd);
    assert!(boosted.audio_flac > base.audio_flac);
}

#[test]
fn override_allow_x265_non4k_removes_penalty() {
    let w = build_weights(
        &ScoringPersona::Audiophile,
        &ScoringOverrides {
            allow_x265_non4k: Some(true),
            ..ScoringOverrides::default()
        },
    );
    assert!(w.x265_non4k_penalty >= 0);
}

#[test]
fn override_block_upscaled_off_removes_penalty() {
    let w = build_weights(
        &ScoringPersona::Balanced,
        &ScoringOverrides {
            block_upscaled: Some(false),
            ..ScoringOverrides::default()
        },
    );
    assert_eq!(w.upscaled_penalty, 0);
}

// ── Default persona is Balanced ───────────────────────────────────────────

#[test]
fn default_persona_is_balanced() {
    assert_eq!(ScoringPersona::default(), ScoringPersona::Balanced);
}

#[test]
fn default_overrides_are_all_none() {
    let o = ScoringOverrides::default();
    assert!(o.allow_x265_non4k.is_none());
    assert!(o.block_dv_without_fallback.is_none());
    assert!(o.prefer_compact_encodes.is_none());
    assert!(o.prefer_lossless_audio.is_none());
    assert!(o.block_upscaled.is_none());
}

// ── audio_weight_for_codec ────────────────────────────────────────────────

#[test]
fn audio_weight_lookup_known_codecs() {
    let w = balanced_weights();
    assert_eq!(audio_weight_for_codec(&w, "TRUEHD", false), 60);
    assert_eq!(audio_weight_for_codec(&w, "FLAC", false), 60);
    assert_eq!(audio_weight_for_codec(&w, "DDP", false), 40);
    assert_eq!(audio_weight_for_codec(&w, "AC3", false), 20);
    assert_eq!(audio_weight_for_codec(&w, "DTSX", false), 60);
    assert_eq!(audio_weight_for_codec(&w, "DTSMA", false), 40);
    assert_eq!(audio_weight_for_codec(&w, "EAC3", false), 40);
}

#[test]
fn audio_weight_lookup_atmos_variants() {
    let w = balanced_weights();
    // Balanced: Atmos variants equal non-Atmos (no regression)
    assert_eq!(audio_weight_for_codec(&w, "TRUEHD", true), 60);
    assert_eq!(audio_weight_for_codec(&w, "DDP", true), 40);
    assert_eq!(audio_weight_for_codec(&w, "EAC3", true), 40);

    // Audiophile: Atmos variants are significantly higher
    let aud = build_weights(&ScoringPersona::Audiophile, &ScoringOverrides::default());
    assert!(
        audio_weight_for_codec(&aud, "TRUEHD", true)
            > audio_weight_for_codec(&aud, "TRUEHD", false)
    );
    assert!(audio_weight_for_codec(&aud, "DDP", true) > audio_weight_for_codec(&aud, "DDP", false));
}

#[test]
fn audio_weight_lookup_unknown_codec_returns_zero() {
    let w = balanced_weights();
    assert_eq!(audio_weight_for_codec(&w, "UNKNOWN", false), 0);
    assert_eq!(audio_weight_for_codec(&w, "", false), 0);
}
