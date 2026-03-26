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
  required_audio_languages: string[];
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

export type QualityProfileSelectionRecord = {
  scope: ViewCategoryId;
  overrideProfileId: string | null;
  effectiveProfileId: string;
  inheritsGlobal: boolean;
};

export type QualityProfileGraphCriteriaPayload = {
  qualityTiers: string[];
  archivalQuality: string | null;
  allowUnknownQuality: boolean;
  sourceAllowlist: string[];
  sourceBlocklist: string[];
  videoCodecAllowlist: string[];
  videoCodecBlocklist: string[];
  audioCodecAllowlist: string[];
  audioCodecBlocklist: string[];
  atmosPreferred: boolean;
  dolbyVisionAllowed: boolean;
  detectedHdrAllowed: boolean;
  preferRemux: boolean;
  allowBdDisk: boolean;
  allowUpgrades: boolean;
  preferDualAudio: boolean;
  requiredAudioLanguages: string[];
  scoringPersona: ScoringPersonaId;
  scoringOverrides: ScoringOverridesPayload;
  cutoffTier: string | null;
  minScoreToGrab: number | null;
  facetPersonaOverrides: Array<{
    scope: ViewCategoryId;
    persona: ScoringPersonaId;
  }>;
};

export type QualityProfileGraphPayload = {
  id: string;
  name: string;
  criteria: QualityProfileGraphCriteriaPayload;
};

export type QualityProfileSettingsPayload = {
  globalProfileId: string;
  profiles: QualityProfileGraphPayload[];
  categorySelections: QualityProfileSelectionRecord[];
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
  required_audio_languages: string[];
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
