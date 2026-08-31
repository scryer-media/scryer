import type { ScoringPersonaId } from "@/lib/types/quality-profiles";

export const QUALITY_TIER_CHOICES = [
  { value: "4320P", label: "8K (4320P)" },
  { value: "2160P", label: "4k (2160P)" },
  { value: "1440P", label: "1440P" },
  { value: "1080P", label: "1080P" },
  { value: "1080I", label: "1080i" },
  { value: "720P", label: "720P" },
  { value: "576P", label: "576P" },
  { value: "480P", label: "480P" },
  { value: "360P", label: "360P" },
] as const;

export const DEFAULT_QUALITY_PROFILE_QUALITY_TIERS = ["2160P", "1080P", "720P"] as const;

export const QUALITY_SOURCE_CHOICES = [
  { value: "WEB-DL", label: "WEB-DL" },
  { value: "WEBRip", label: "WEBRip" },
  { value: "BluRay", label: "BluRay" },
  { value: "BRDISK", label: "BRDISK" },
  { value: "DVD", label: "DVD" },
  { value: "HDTV", label: "HDTV" },
  { value: "CAM", label: "CAM" },
  { value: "TELESYNC", label: "TELESYNC" },
  { value: "TELECINE", label: "TELECINE" },
  { value: "DVDSCR", label: "DVDSCR" },
  { value: "WORKPRINT", label: "WORKPRINT" },
] as const;

export const VIDEO_CODEC_CHOICES = [
  { value: "H.264", label: "H.264" },
  { value: "H.265", label: "H.265" },
  { value: "AV1", label: "AV1" },
  { value: "VP9", label: "VP9" },
  { value: "VC1", label: "VC1" },
  { value: "MPEG2", label: "MPEG2" },
  { value: "MPEG4", label: "MPEG4" },
  { value: "XVID", label: "XVID" },
  { value: "DIVX", label: "DIVX" },
  { value: "VVC", label: "VVC" },
] as const;

export const AUDIO_CODEC_CHOICES = [
  { value: "DDP", label: "DDP" },
  { value: "EAC3", label: "EAC3" },
  { value: "AC3", label: "AC3" },
  { value: "AAC", label: "AAC" },
  { value: "TRUEHD", label: "TrueHD" },
  { value: "DTSMA", label: "DTS-HD MA" },
  { value: "DTSX", label: "DTS:X" },
  { value: "DTSHD", label: "DTS-HD" },
  { value: "DTS", label: "DTS" },
  { value: "FLAC", label: "FLAC" },
  { value: "OPUS", label: "OPUS" },
  { value: "VORBIS", label: "Vorbis" },
  { value: "MP3", label: "MP3" },
  { value: "PCM", label: "PCM" },
] as const;

export const SCORING_PERSONA_CHOICES = [
  { value: "BALANCED", labelKey: "qualityProfile.personaBalanced" },
  { value: "AUDIOPHILE", labelKey: "qualityProfile.personaAudiophile" },
  { value: "EFFICIENT", labelKey: "qualityProfile.personaEfficient" },
  { value: "COMPATIBLE", labelKey: "qualityProfile.personaCompatible" },
] as const;

export const SCORING_OVERRIDE_KEYS = [
  "allow_x265_non4k",
  "block_dv_without_fallback",
  "prefer_compact_encodes",
  "prefer_lossless_audio",
  "block_upscaled",
] as const;

export const PERSONA_OVERRIDE_DEFAULTS: Record<ScoringPersonaId, Record<string, boolean>> = {
  BALANCED: { allow_x265_non4k: false, block_dv_without_fallback: false, prefer_compact_encodes: false, prefer_lossless_audio: false, block_upscaled: true },
  AUDIOPHILE: { allow_x265_non4k: false, block_dv_without_fallback: false, prefer_compact_encodes: false, prefer_lossless_audio: true, block_upscaled: true },
  EFFICIENT: { allow_x265_non4k: true, block_dv_without_fallback: false, prefer_compact_encodes: true, prefer_lossless_audio: false, block_upscaled: true },
  COMPATIBLE: { allow_x265_non4k: false, block_dv_without_fallback: false, prefer_compact_encodes: false, prefer_lossless_audio: false, block_upscaled: true },
};

export const PERSONA_DESCRIPTION_KEYS: Record<ScoringPersonaId, string> = {
  BALANCED: "setup.personaBalancedDesc",
  AUDIOPHILE: "setup.personaAudiophileDesc",
  EFFICIENT: "setup.personaEfficientDesc",
  COMPATIBLE: "setup.personaCompatibleDesc",
};

/** Key scoring traits per persona — derived from the Rust scoring_weights.rs presets. */
export const PERSONA_SCORING_TRAITS: Record<ScoringPersonaId, string[]> = {
  BALANCED: [
    "persona.trait.balanced.source",
    "persona.trait.balanced.audio",
    "persona.trait.balanced.x265",
    "persona.trait.balanced.size",
    "persona.trait.balanced.remux",
    "persona.trait.balanced.hdr",
  ],
  AUDIOPHILE: [
    "persona.trait.audiophile.source",
    "persona.trait.audiophile.audio",
    "persona.trait.audiophile.x265",
    "persona.trait.audiophile.size",
    "persona.trait.audiophile.remux",
    "persona.trait.audiophile.hdr",
  ],
  EFFICIENT: [
    "persona.trait.efficient.source",
    "persona.trait.efficient.audio",
    "persona.trait.efficient.x265",
    "persona.trait.efficient.size",
    "persona.trait.efficient.remux",
    "persona.trait.efficient.hdr",
  ],
  COMPATIBLE: [
    "persona.trait.compatible.source",
    "persona.trait.compatible.audio",
    "persona.trait.compatible.x265",
    "persona.trait.compatible.size",
    "persona.trait.compatible.remux",
    "persona.trait.compatible.hdr",
  ],
};
