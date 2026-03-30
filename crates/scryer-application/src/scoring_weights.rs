use serde::{Deserialize, Serialize};

use crate::quality_profile::BLOCK_SCORE;

/// Scoring persona — a named preset that sets all default scoring weights.
/// Users pick one persona and optionally flip a few overrides.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum ScoringPersona {
    #[default]
    Balanced,
    Audiophile,
    Efficient,
    Compatible,
}

/// Five toggles that patch specific weights regardless of persona.
/// `None` means "use the persona's default". `Some(true/false)` overrides it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScoringOverrides {
    /// Allow x265/HEVC at non-4K resolutions without penalty.
    #[serde(default)]
    pub allow_x265_non4k: Option<bool>,

    /// Block Dolby Vision releases that lack an HDR10 fallback layer.
    #[serde(default)]
    pub block_dv_without_fallback: Option<bool>,

    /// Invert the size curve to reward smaller, more efficient encodes.
    #[serde(default)]
    pub prefer_compact_encodes: Option<bool>,

    /// Extra bonus for lossless audio codecs (TrueHD, FLAC, DTS-HD MA, PCM).
    #[serde(default)]
    pub prefer_lossless_audio: Option<bool>,

    /// Block releases with AI upscale indicators.
    #[serde(default)]
    pub block_upscaled: Option<bool>,
}

/// Every numeric constant used by the scoring functions. Built from a persona
/// preset, then patched by any active overrides.
#[derive(Debug, Clone)]
pub struct ScoringWeights {
    // ── Source bonuses ──────────────────────────────────────
    pub source_bluray: i32,
    pub source_webdl: i32,
    pub source_webrip: i32,
    pub source_hdtv: i32,

    // ── Video codec (when no allowlist configured) ─────────
    pub video_codec_high: i32,
    pub video_codec_mid: i32,
    pub x265_non4k_penalty: i32,

    // ── Audio codec hierarchy (when no allowlist configured) ─
    pub audio_truehd_atmos: i32,
    pub audio_dtsx: i32,
    pub audio_truehd: i32,
    pub audio_dtsma: i32,
    pub audio_flac: i32,
    pub audio_ddp_atmos: i32,
    pub audio_ddp: i32,
    pub audio_dtshd: i32,
    pub audio_dts: i32,
    pub audio_ac3: i32,
    pub audio_aac: i32,
    pub audio_mp3: i32,
    pub audio_opus: i32,

    // ── Audio channels ─────────────────────────────────────
    pub channels_71: i32,
    pub channels_51: i32,
    pub channels_20: i32,
    pub channels_10: i32,

    // ── HDR / visual features ──────────────────────────────
    pub dolby_vision: i32,
    pub hdr10: i32,
    pub hdr10plus: i32,
    pub sdr_at_4k_penalty: i32,

    // ── Feature preferences ────────────────────────────────
    pub remux_bonus: i32,
    pub remux_missing_penalty: i32,
    pub atmos_bonus: i32,
    pub atmos_missing_penalty: i32,
    pub block_dv_without_fallback: bool,
    pub dual_audio_bonus: i32,
    pub dual_audio_missing_penalty: i32,
    pub dual_audio_present: i32,
    pub proper_bonus: i32,
    pub repack_bonus: i32,

    // ── Release group reputation ───────────────────────────
    pub group_gold: i32,
    pub group_silver: i32,
    pub group_bronze: i32,
    pub group_banned: i32,
    pub group_unknown_penalty: i32,

    // ── Streaming service ──────────────────────────────────
    pub streaming_tier1: i32,
    pub streaming_tier2: i32,
    pub streaming_tier3: i32,
    pub streaming_anime: i32,

    // ── Size curve ─────────────────────────────────────────
    pub size_excessive: i32,
    pub size_massive: i32,
    pub size_very_large: i32,
    pub size_large: i32,
    pub size_expected: i32,
    pub size_slightly_small: i32,
    pub size_small: i32,
    pub size_very_small: i32,
    pub size_tiny: i32,

    // ── Unwanted content ───────────────────────────────────
    pub upscaled_penalty: i32,
    pub hardcoded_subs_penalty: i32,
    pub reencode_penalty: i32,

    // ── Edition bonuses (movies) ───────────────────────────
    pub edition_imax: i32,
    pub edition_extended: i32,
    pub edition_hybrid: i32,
    pub edition_criterion: i32,
    pub edition_remaster: i32,

    // ── Anime-specific ─────────────────────────────────────
    pub anime_10bit_bonus: i32,
    pub anime_v2_bonus: i32,
    pub anime_uncensored_bonus: i32,
    pub anime_dubs_only_penalty: i32,
}

/// Build a complete set of scoring weights from a persona and optional overrides.
pub fn build_weights(persona: &ScoringPersona, overrides: &ScoringOverrides) -> ScoringWeights {
    let mut weights = match persona {
        ScoringPersona::Balanced => balanced_weights(),
        ScoringPersona::Audiophile => audiophile_weights(),
        ScoringPersona::Efficient => efficient_weights(),
        ScoringPersona::Compatible => compatible_weights(),
    };
    apply_overrides(&mut weights, persona, overrides);
    weights
}

/// Build weights with category-aware adjustments.  Anime content never
/// ships with Atmos audio, so the atmos missing penalty is zeroed out
/// to avoid penalizing every anime release.
pub fn build_weights_for_category(
    persona: &ScoringPersona,
    overrides: &ScoringOverrides,
    category: Option<&str>,
) -> ScoringWeights {
    let mut weights = build_weights(persona, overrides);
    if matches!(
        category.map(|c| c.trim().to_ascii_lowercase()).as_deref(),
        Some("anime")
    ) {
        weights.atmos_missing_penalty = 0;
    }
    weights
}

struct OverrideDefaults {
    allow_x265_non4k: bool,
    block_dv_without_fallback: bool,
    prefer_compact_encodes: bool,
    prefer_lossless_audio: bool,
    block_upscaled: bool,
}

fn override_defaults(persona: &ScoringPersona) -> OverrideDefaults {
    match persona {
        ScoringPersona::Balanced => OverrideDefaults {
            allow_x265_non4k: false,
            block_dv_without_fallback: false,
            prefer_compact_encodes: false,
            prefer_lossless_audio: false,
            block_upscaled: true,
        },
        ScoringPersona::Audiophile => OverrideDefaults {
            allow_x265_non4k: false,
            block_dv_without_fallback: false,
            prefer_compact_encodes: false,
            prefer_lossless_audio: true,
            block_upscaled: true,
        },
        ScoringPersona::Efficient => OverrideDefaults {
            allow_x265_non4k: true,
            block_dv_without_fallback: false,
            prefer_compact_encodes: true,
            prefer_lossless_audio: false,
            block_upscaled: true,
        },
        ScoringPersona::Compatible => OverrideDefaults {
            allow_x265_non4k: false,
            block_dv_without_fallback: false,
            prefer_compact_encodes: false,
            prefer_lossless_audio: false,
            block_upscaled: true,
        },
    }
}

fn apply_overrides(
    weights: &mut ScoringWeights,
    persona: &ScoringPersona,
    overrides: &ScoringOverrides,
) {
    let defaults = override_defaults(persona);

    // allow_x265_non4k: when enabled, remove penalty
    let allow_x265 = overrides
        .allow_x265_non4k
        .unwrap_or(defaults.allow_x265_non4k);
    if allow_x265 {
        weights.x265_non4k_penalty = weights.x265_non4k_penalty.max(0);
    }

    weights.block_dv_without_fallback = overrides
        .block_dv_without_fallback
        .unwrap_or(defaults.block_dv_without_fallback);

    // prefer_compact_encodes: swap size curve
    let compact = overrides
        .prefer_compact_encodes
        .unwrap_or(defaults.prefer_compact_encodes);
    if compact != defaults.prefer_compact_encodes {
        if compact {
            let eff = efficient_weights();
            weights.size_excessive = eff.size_excessive;
            weights.size_massive = eff.size_massive;
            weights.size_very_large = eff.size_very_large;
            weights.size_large = eff.size_large;
            weights.size_expected = eff.size_expected;
            weights.size_slightly_small = eff.size_slightly_small;
            weights.size_small = eff.size_small;
            weights.size_very_small = eff.size_very_small;
            weights.size_tiny = eff.size_tiny;
        } else {
            let bal = balanced_weights();
            weights.size_excessive = bal.size_excessive;
            weights.size_massive = bal.size_massive;
            weights.size_very_large = bal.size_very_large;
            weights.size_large = bal.size_large;
            weights.size_expected = bal.size_expected;
            weights.size_slightly_small = bal.size_slightly_small;
            weights.size_small = bal.size_small;
            weights.size_very_small = bal.size_very_small;
            weights.size_tiny = bal.size_tiny;
        }
    }

    // prefer_lossless_audio: swap audio weights to audiophile-style
    let lossless = overrides
        .prefer_lossless_audio
        .unwrap_or(defaults.prefer_lossless_audio);
    if lossless != defaults.prefer_lossless_audio {
        if lossless {
            let aud = audiophile_weights();
            weights.audio_truehd_atmos = aud.audio_truehd_atmos;
            weights.audio_dtsx = aud.audio_dtsx;
            weights.audio_truehd = aud.audio_truehd;
            weights.audio_dtsma = aud.audio_dtsma;
            weights.audio_flac = aud.audio_flac;
        } else {
            let bal = balanced_weights();
            weights.audio_truehd_atmos = bal.audio_truehd_atmos;
            weights.audio_dtsx = bal.audio_dtsx;
            weights.audio_truehd = bal.audio_truehd;
            weights.audio_dtsma = bal.audio_dtsma;
            weights.audio_flac = bal.audio_flac;
        }
    }

    // block_upscaled: toggle penalty
    let block_up = overrides.block_upscaled.unwrap_or(defaults.block_upscaled);
    if !block_up {
        weights.upscaled_penalty = 0;
    }
}

// ─── Persona presets ────────────────────────────────────────────────────────

/// Balanced — mainstream quality preference without a strong remux bias.
pub(crate) fn balanced_weights() -> ScoringWeights {
    ScoringWeights {
        // Source
        source_bluray: 150,
        source_webdl: 120,
        source_webrip: 80,
        source_hdtv: 40,

        // Video codec
        video_codec_high: 60,
        video_codec_mid: 40,
        x265_non4k_penalty: -100,

        // Audio codec — matches legacy 3-tier: lossless=60, high=40, standard=20
        audio_truehd_atmos: 60,
        audio_dtsx: 60,
        audio_truehd: 60,
        audio_dtsma: 40,
        audio_flac: 60,
        audio_ddp_atmos: 40,
        audio_ddp: 40,
        audio_dtshd: 40,
        audio_dts: 40,
        audio_ac3: 20,
        audio_aac: 20,
        audio_mp3: 0,
        audio_opus: 0,

        // Audio channels
        channels_71: 30,
        channels_51: 15,
        channels_20: 0,
        channels_10: -15,

        // HDR
        dolby_vision: 50,
        hdr10: 30,
        hdr10plus: 30,
        sdr_at_4k_penalty: -150,

        // Features
        remux_bonus: 0,
        remux_missing_penalty: 0,
        atmos_bonus: 0,
        atmos_missing_penalty: 0,
        block_dv_without_fallback: false,
        dual_audio_bonus: 150,
        dual_audio_missing_penalty: -30,
        dual_audio_present: 40,
        proper_bonus: 30,
        repack_bonus: 30,

        // Release groups — not applied until Phase C
        group_gold: 300,
        group_silver: 150,
        group_bronze: 50,
        group_banned: BLOCK_SCORE,
        group_unknown_penalty: -30,

        // Streaming — not applied until Phase E
        streaming_tier1: 30,
        streaming_tier2: 20,
        streaming_tier3: 10,
        streaming_anime: 20,

        // Size curve
        size_excessive: -300,
        size_massive: 550,
        size_very_large: 380,
        size_large: 240,
        size_expected: 120,
        size_slightly_small: 0,
        size_small: -700,
        size_very_small: -1300,
        size_tiny: -2500,

        // Unwanted — not applied until Phase E
        upscaled_penalty: BLOCK_SCORE,
        hardcoded_subs_penalty: -300,
        reencode_penalty: -400,

        // Editions — not applied until Phase E
        edition_imax: 80,
        edition_extended: 40,
        edition_hybrid: 30,
        edition_criterion: 20,
        edition_remaster: 20,

        // Anime — not applied until Phase E
        anime_10bit_bonus: 40,
        anime_v2_bonus: 20,
        anime_uncensored_bonus: 30,
        anime_dubs_only_penalty: -100,
    }
}

/// Audiophile — maximum fidelity. Bigger is better. Lossless preferred.
fn audiophile_weights() -> ScoringWeights {
    ScoringWeights {
        source_bluray: 250,
        source_webdl: 100,
        source_webrip: 50,
        source_hdtv: 10,

        video_codec_high: 60,
        video_codec_mid: 40,
        x265_non4k_penalty: -250,

        audio_truehd_atmos: 400,
        audio_dtsx: 360,
        audio_truehd: 300,
        audio_dtsma: 260,
        audio_flac: 240,
        audio_ddp_atmos: 150,
        audio_ddp: 80,
        audio_dtshd: 70,
        audio_dts: 50,
        audio_ac3: 15,
        audio_aac: 10,
        audio_mp3: -50,
        audio_opus: 5,

        channels_71: 60,
        channels_51: 25,
        channels_20: -10,
        channels_10: -40,

        dolby_vision: 150,
        hdr10: 50,
        hdr10plus: 50,
        sdr_at_4k_penalty: -300,

        remux_bonus: 400,
        remux_missing_penalty: -80,
        atmos_bonus: 150,
        atmos_missing_penalty: -30,
        block_dv_without_fallback: false,
        dual_audio_bonus: 200,
        dual_audio_missing_penalty: -40,
        dual_audio_present: 50,
        proper_bonus: 50,
        repack_bonus: 50,

        group_gold: 500,
        group_silver: 250,
        group_bronze: 80,
        group_banned: BLOCK_SCORE,
        group_unknown_penalty: -60,

        streaming_tier1: 20,
        streaming_tier2: 15,
        streaming_tier3: 5,
        streaming_anime: 15,

        size_excessive: -150,
        size_massive: 700,
        size_very_large: 500,
        size_large: 350,
        size_expected: 200,
        size_slightly_small: 0,
        size_small: -400,
        size_very_small: -900,
        size_tiny: -2000,

        upscaled_penalty: BLOCK_SCORE,
        hardcoded_subs_penalty: -400,
        reencode_penalty: -800,

        edition_imax: 120,
        edition_extended: 60,
        edition_hybrid: 50,
        edition_criterion: 40,
        edition_remaster: 30,

        anime_10bit_bonus: 50,
        anime_v2_bonus: 25,
        anime_uncensored_bonus: 40,
        anime_dubs_only_penalty: -150,
    }
}

/// Efficient — best quality per gigabyte. x265 is good. Compact is good.
fn efficient_weights() -> ScoringWeights {
    ScoringWeights {
        source_bluray: 80,
        source_webdl: 150,
        source_webrip: 120,
        source_hdtv: 40,

        video_codec_high: 150,
        video_codec_mid: 30,
        x265_non4k_penalty: 100, // bonus, not penalty

        audio_truehd_atmos: 100,
        audio_dtsx: 90,
        audio_truehd: 80,
        audio_dtsma: 70,
        audio_flac: 60,
        audio_ddp_atmos: 110,
        audio_ddp: 100,
        audio_dtshd: 50,
        audio_dts: 50,
        audio_ac3: 30,
        audio_aac: 40,
        audio_mp3: -10,
        audio_opus: 35,

        channels_71: -20,
        channels_51: 20,
        channels_20: 10,
        channels_10: -10,

        dolby_vision: 50,
        hdr10: 20,
        hdr10plus: 20,
        sdr_at_4k_penalty: -80,

        remux_bonus: 0,
        remux_missing_penalty: 0,
        atmos_bonus: 0,
        atmos_missing_penalty: 0,
        block_dv_without_fallback: false,
        dual_audio_bonus: 80,
        dual_audio_missing_penalty: -15,
        dual_audio_present: 30,
        proper_bonus: 30,
        repack_bonus: 30,

        group_gold: 150,
        group_silver: 80,
        group_bronze: 30,
        group_banned: BLOCK_SCORE,
        group_unknown_penalty: -15,

        streaming_tier1: 40,
        streaming_tier2: 30,
        streaming_tier3: 20,
        streaming_anime: 25,

        // Inverted curve — sweet spot at or slightly below expected
        size_excessive: -250,
        size_massive: -200,
        size_very_large: -100,
        size_large: 0,
        size_expected: 200,
        size_slightly_small: 300,
        size_small: 100,
        size_very_small: -200,
        size_tiny: -800,

        upscaled_penalty: BLOCK_SCORE,
        hardcoded_subs_penalty: -200,
        reencode_penalty: -200,

        edition_imax: 40,
        edition_extended: 20,
        edition_hybrid: 20,
        edition_criterion: 10,
        edition_remaster: 10,

        anime_10bit_bonus: 60,
        anime_v2_bonus: 20,
        anime_uncensored_bonus: 20,
        anime_dubs_only_penalty: -60,
    }
}

/// Compatible — plays on everything. Avoid risky formats. Universal decode.
fn compatible_weights() -> ScoringWeights {
    ScoringWeights {
        source_bluray: 120,
        source_webdl: 150,
        source_webrip: 100,
        source_hdtv: 60,

        video_codec_high: 40,
        video_codec_mid: 60, // H.264 preferred over H.265
        x265_non4k_penalty: -80,

        audio_truehd_atmos: 80,
        audio_dtsx: 70,
        audio_truehd: 60,
        audio_dtsma: 50,
        audio_flac: 40,
        audio_ddp_atmos: 130,
        audio_ddp: 120,
        audio_dtshd: 40,
        audio_dts: 50,
        audio_ac3: 60,
        audio_aac: 80,
        audio_mp3: 10,
        audio_opus: 30,

        channels_71: 10,
        channels_51: 30,
        channels_20: 20,
        channels_10: 0,

        dolby_vision: -50, // compatibility risk
        hdr10: 40,
        hdr10plus: 10,
        sdr_at_4k_penalty: 0, // SDR at 4K is fine for compatible

        remux_bonus: 0,
        remux_missing_penalty: 0,
        atmos_bonus: 0,
        atmos_missing_penalty: 0,
        block_dv_without_fallback: false,
        dual_audio_bonus: 100,
        dual_audio_missing_penalty: -20,
        dual_audio_present: 30,
        proper_bonus: 30,
        repack_bonus: 30,

        group_gold: 200,
        group_silver: 100,
        group_bronze: 40,
        group_banned: BLOCK_SCORE,
        group_unknown_penalty: -20,

        streaming_tier1: 30,
        streaming_tier2: 20,
        streaming_tier3: 15,
        streaming_anime: 20,

        // Standard curve
        size_excessive: -250,
        size_massive: 550,
        size_very_large: 380,
        size_large: 240,
        size_expected: 120,
        size_slightly_small: 0,
        size_small: -700,
        size_very_small: -1300,
        size_tiny: -2500,

        upscaled_penalty: BLOCK_SCORE,
        hardcoded_subs_penalty: -300,
        reencode_penalty: -400,

        edition_imax: 60,
        edition_extended: 30,
        edition_hybrid: 25,
        edition_criterion: 15,
        edition_remaster: 15,

        anime_10bit_bonus: 20,
        anime_v2_bonus: 15,
        anime_uncensored_bonus: 20,
        anime_dubs_only_penalty: -80,
    }
}

/// Look up the audio weight for a normalized codec string.
///
/// When `is_atmos` is true, TrueHD and DDP resolve to their Atmos-specific
/// weights (which may differ from the non-Atmos variant depending on persona).
pub fn audio_weight_for_codec(weights: &ScoringWeights, codec: &str, is_atmos: bool) -> i32 {
    match codec {
        "TRUEHD" if is_atmos => weights.audio_truehd_atmos,
        "TRUEHD" => weights.audio_truehd,
        "DTSX" => weights.audio_dtsx,
        "FLAC" => weights.audio_flac,
        "DDP" | "EAC3" if is_atmos => weights.audio_ddp_atmos,
        "DDP" | "EAC3" => weights.audio_ddp,
        "DTSMA" => weights.audio_dtsma,
        "DTSHD" => weights.audio_dtshd,
        "DTS" => weights.audio_dts,
        "AC3" => weights.audio_ac3,
        "AAC" => weights.audio_aac,
        "MP3" => weights.audio_mp3,
        "OPUS" => weights.audio_opus,
        _ => 0,
    }
}

#[cfg(test)]
#[path = "scoring_weights_tests.rs"]
mod scoring_weights_tests;
