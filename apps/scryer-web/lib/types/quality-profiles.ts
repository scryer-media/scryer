export type ParsedQualityProfile = {
  id: string;
  name: string;
};

export type ScoringPersonaId = "Balanced" | "Audiophile" | "Efficient" | "Compatible";

export type QualityTargetId = "4k" | "1080p";

export type FacetQualityPrefs = {
  quality: QualityTargetId;
  persona: ScoringPersonaId;
};

export type ScoringOverridesPayload = {
  allow_x265_non4k?: boolean | null;
  block_dv_without_fallback?: boolean | null;
  prefer_compact_encodes?: boolean | null;
  prefer_lossless_audio?: boolean | null;
  block_upscaled?: boolean | null;
};

export type QualityProfileCriteriaPayload = {
  quality_tiers: string[];
  archival_quality: string | null;
  allow_unknown_quality: boolean;
  source_allowlist: string[];
  source_blocklist: string[];
  video_codec_allowlist: string[];
  video_codec_blocklist: string[];
  audio_codec_allowlist: string[];
  audio_codec_blocklist: string[];
  atmos_preferred: boolean;
  dolby_vision_allowed: boolean;
  detected_hdr_allowed: boolean;
  prefer_remux: boolean;
  prefer_dual_audio: boolean;
  allow_bd_disk: boolean;
  allow_upgrades: boolean;
  scoring_persona: ScoringPersonaId;
  scoring_overrides: ScoringOverridesPayload;
  cutoff_tier: string | null;
  min_score_to_grab: number | null;
  facet_persona_overrides: Record<string, ScoringPersonaId>;
};

export type ParsedQualityProfileEntry = ParsedQualityProfile & {
  criteria: QualityProfileCriteriaPayload;
};

export type CommittedQualityProfileDraft = {
  catalogText: string;
  draftEntry: ParsedQualityProfileEntry;
};

export type QualityProfileDraft = {
  id: string;
  name: string;
  quality_tiers: string[];
  archival_quality: string;
  allow_unknown_quality: boolean;
  source_allowlist: string[];
  source_blocklist: string[];
  video_codec_allowlist: string[];
  video_codec_blocklist: string[];
  audio_codec_allowlist: string[];
  audio_codec_blocklist: string[];
  atmos_preferred: boolean;
  dolby_vision_allowed: boolean;
  detected_hdr_allowed: boolean;
  prefer_remux: boolean;
  prefer_dual_audio: boolean;
  allow_bd_disk: boolean;
  allow_upgrades: boolean;
  scoring_persona: ScoringPersonaId;
  scoring_overrides: ScoringOverridesPayload;
  cutoff_tier: string;
  min_score_to_grab: number | null;
  facet_persona_overrides: Record<string, ScoringPersonaId>;
};

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

export type QualityProfileListField =
  | "source_allowlist"
  | "source_blocklist"
  | "video_codec_allowlist"
  | "video_codec_blocklist"
  | "audio_codec_allowlist"
  | "audio_codec_blocklist";

export type ProfileListChoice = {
  value: string;
  label: string;
};

export type ProfileRawRecord = Record<string, JsonValue>;

export type ViewCategoryId = "movie" | "series" | "anime";
