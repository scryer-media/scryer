import type { ScoringPersonaId } from "@/lib/types/quality-profiles";

export const QUALITY_TIER_CHOICES = [
  { value: "4320P", label: "8K (4320P)" },
  { value: "2160P", label: "4k (2160P)" },
  { value: "1440P", label: "1440P" },
  { value: "1080P", label: "1080P" },
  { value: "1080I", label: "1080i" },
  { value: "720P", label: "720P" },
  { value: "480P", label: "480P" },
  { value: "360P", label: "360P" },
] as const;

export const DEFAULT_QUALITY_PROFILE_QUALITY_TIERS = ["2160P", "1080P", "720P"] as const;

export const QUALITY_SOURCE_CHOICES = [
  { value: "WEB-DL", label: "WEB-DL" },
  { value: "BluRay", label: "BluRay" },
  { value: "HDTV", label: "HDTV" },
  { value: "DVD", label: "DVD" },
] as const;

export const VIDEO_CODEC_CHOICES = [
  { value: "H.264", label: "H.264" },
  { value: "H.265", label: "H.265" },
  { value: "AV1", label: "AV1" },
  { value: "VP9", label: "VP9" },
  { value: "VP8", label: "VP8" },
  { value: "XVID", label: "XVID" },
  { value: "x264", label: "x264 (encoding)" },
  { value: "x265", label: "x265 (encoding)" },
] as const;

export const AUDIO_CODEC_CHOICES = [
  { value: "AAC", label: "AAC" },
  { value: "AC3", label: "AC3" },
  { value: "DDP", label: "DDP" },
  { value: "DTS", label: "DTS" },
  { value: "EAC3", label: "EAC3" },
  { value: "FLAC", label: "FLAC" },
  { value: "OPUS", label: "OPUS" },
  { value: "TRUEHD", label: "TrueHD" },
] as const;

export const SCORING_PERSONA_CHOICES = [
  { value: "Balanced", labelKey: "qualityProfile.personaBalanced" },
  { value: "Audiophile", labelKey: "qualityProfile.personaAudiophile" },
  { value: "Efficient", labelKey: "qualityProfile.personaEfficient" },
  { value: "Compatible", labelKey: "qualityProfile.personaCompatible" },
] as const;

export const SCORING_OVERRIDE_KEYS = [
  "allow_x265_non4k",
  "block_dv_without_fallback",
  "prefer_compact_encodes",
  "prefer_lossless_audio",
  "block_upscaled",
] as const;

export const PERSONA_OVERRIDE_DEFAULTS: Record<ScoringPersonaId, Record<string, boolean>> = {
  Balanced: { allow_x265_non4k: false, block_dv_without_fallback: false, prefer_compact_encodes: false, prefer_lossless_audio: false, block_upscaled: true },
  Audiophile: { allow_x265_non4k: false, block_dv_without_fallback: false, prefer_compact_encodes: false, prefer_lossless_audio: true, block_upscaled: true },
  Efficient: { allow_x265_non4k: true, block_dv_without_fallback: false, prefer_compact_encodes: true, prefer_lossless_audio: false, block_upscaled: true },
  Compatible: { allow_x265_non4k: false, block_dv_without_fallback: false, prefer_compact_encodes: false, prefer_lossless_audio: false, block_upscaled: true },
};

export const PERSONA_DESCRIPTION_KEYS: Record<ScoringPersonaId, string> = {
  Balanced: "setup.personaBalancedDesc",
  Audiophile: "setup.personaAudiophileDesc",
  Efficient: "setup.personaEfficientDesc",
  Compatible: "setup.personaCompatibleDesc",
};

/** Key scoring traits per persona — derived from the Rust scoring_weights.rs presets. */
export const PERSONA_SCORING_TRAITS: Record<ScoringPersonaId, string[]> = {
  Balanced: [
    "persona.trait.balanced.source",
    "persona.trait.balanced.audio",
    "persona.trait.balanced.x265",
    "persona.trait.balanced.size",
    "persona.trait.balanced.remux",
    "persona.trait.balanced.hdr",
  ],
  Audiophile: [
    "persona.trait.audiophile.source",
    "persona.trait.audiophile.audio",
    "persona.trait.audiophile.x265",
    "persona.trait.audiophile.size",
    "persona.trait.audiophile.remux",
    "persona.trait.audiophile.hdr",
  ],
  Efficient: [
    "persona.trait.efficient.source",
    "persona.trait.efficient.audio",
    "persona.trait.efficient.x265",
    "persona.trait.efficient.size",
    "persona.trait.efficient.remux",
    "persona.trait.efficient.hdr",
  ],
  Compatible: [
    "persona.trait.compatible.source",
    "persona.trait.compatible.audio",
    "persona.trait.compatible.x265",
    "persona.trait.compatible.size",
    "persona.trait.compatible.remux",
    "persona.trait.compatible.hdr",
  ],
};
