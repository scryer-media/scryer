import {
  DEFAULT_QUALITY_PROFILE_QUALITY_TIERS,
} from "@/lib/constants/quality-profiles";
import { QUALITY_PROFILE_INHERIT_VALUE } from "@/lib/constants/settings";
import type {
  FacetScoringPersonaSelectionRecord,
  ParsedQualityProfile,
  ParsedQualityProfileEntry,
  QualityProfileDraft,
  QualityProfileGraphPayload,
  QualityProfileSettingsPayload,
  ProfileListChoice,
  ProfileRawRecord,
  ScoringPersonaId,
  ScoringOverridesPayload,
  ViewCategoryId,
} from "@/lib/types";

type ProfileCatalogParseResult = {
  text: string;
  entries: ParsedQualityProfileEntry[];
  profiles: ParsedQualityProfile[];
  isRawValid: boolean;
};

export function buildDefaultCategoryPersonaSelections(
  globalPersona: ScoringPersonaId = "Balanced",
): Record<ViewCategoryId, FacetScoringPersonaSelectionRecord> {
  return {
    movie: {
      scope: "movie",
      overridePersona: null,
      effectivePersona: globalPersona,
      inheritsGlobal: true,
    },
    series: {
      scope: "series",
      overridePersona: null,
      effectivePersona: globalPersona,
      inheritsGlobal: true,
    },
    anime: {
      scope: "anime",
      overridePersona: null,
      effectivePersona: globalPersona,
      inheritsGlobal: true,
    },
  };
}

export function normalizeQualityProfileEntry(rawEntry: unknown): ParsedQualityProfileEntry | null {
  if (!rawEntry || typeof rawEntry !== "object") {
    return null;
  }

  const candidate = rawEntry as ProfileRawRecord;
  const rawId = typeof candidate.id === "string" ? candidate.id.trim() : "";
  if (!rawId) {
    return null;
  }

  const rawName = typeof candidate.name === "string" ? candidate.name.trim() : "";
  const name = rawName || rawId;

  const rawCriteria = candidate.criteria;
  if (!rawCriteria || typeof rawCriteria !== "object") {
    return null;
  }

  const criteria = rawCriteria as ProfileRawRecord;

  const qualityTiers = parseStringArrayValue(criteria.quality_tiers).map((value) =>
    value.toUpperCase(),
  );
  const resolvedQualityTiers = qualityTiers.length > 0 ? qualityTiers : [];

  const rawArchivalQuality = typeof criteria.archival_quality === "string" ? criteria.archival_quality.trim() : "";

  const readBoolean = (rawValue: unknown, fallback: boolean) =>
    typeof rawValue === "boolean" ? rawValue : fallback;

  const readList = (rawValue: unknown): string[] => parseStringArrayValue(rawValue);

  return {
    id: rawId,
    name,
    criteria: {
      quality_tiers: dedupeOrdered(resolvedQualityTiers.map((value) => value.toUpperCase())),
      archival_quality: rawArchivalQuality ? rawArchivalQuality.toUpperCase() : null,
      allow_unknown_quality: readBoolean(criteria.allow_unknown_quality, false),
      source_allowlist: readList(criteria.source_allowlist),
      source_blocklist: readList(criteria.source_blocklist),
      video_codec_allowlist: readList(criteria.video_codec_allowlist),
      video_codec_blocklist: readList(criteria.video_codec_blocklist),
      audio_codec_allowlist: readList(criteria.audio_codec_allowlist),
      audio_codec_blocklist: readList(criteria.audio_codec_blocklist),
      dolby_vision_allowed: readBoolean(criteria.dolby_vision_allowed, false),
      detected_hdr_allowed: readBoolean(criteria.detected_hdr_allowed, false),
      prefer_remux: readBoolean(criteria.prefer_remux, false),
      allow_bd_disk: readBoolean(criteria.allow_bd_disk, false),
      allow_upgrades: readBoolean(criteria.allow_upgrades, false),
      scoring_overrides: readScoringOverrides(criteria.scoring_overrides),
      cutoff_tier: typeof criteria.cutoff_tier === "string" && criteria.cutoff_tier.trim()
        ? criteria.cutoff_tier.trim().toUpperCase()
        : null,
      min_score_to_grab: typeof criteria.min_score_to_grab === "number"
        ? criteria.min_score_to_grab
        : null,
    },
  };
}

function scoringOverridesFromGraph(raw: unknown): ScoringOverridesPayload {
  if (!raw || typeof raw !== "object") return {};
  const obj = raw as Record<string, unknown>;
  const result: ScoringOverridesPayload = {};
  const mapping: [string, keyof ScoringOverridesPayload][] = [
    ["allowX265Non4K", "allow_x265_non4k"],
    ["blockDvWithoutFallback", "block_dv_without_fallback"],
    ["preferCompactEncodes", "prefer_compact_encodes"],
    ["preferLosslessAudio", "prefer_lossless_audio"],
    ["blockUpscaled", "block_upscaled"],
  ];
  for (const [camel, snake] of mapping) {
    if (typeof obj[camel] === "boolean") {
      result[snake] = obj[camel] as boolean;
    }
  }
  return result;
}

function scoringOverridesToGraphInput(overrides: ScoringOverridesPayload | undefined | null): Record<string, boolean | null> {
  if (!overrides) return {};
  const result: Record<string, boolean | null> = {};
  const mapping: [keyof ScoringOverridesPayload, string][] = [
    ["allow_x265_non4k", "allowX265Non4K"],
    ["block_dv_without_fallback", "blockDvWithoutFallback"],
    ["prefer_compact_encodes", "preferCompactEncodes"],
    ["prefer_lossless_audio", "preferLosslessAudio"],
    ["block_upscaled", "blockUpscaled"],
  ];
  for (const [snake, camel] of mapping) {
    if (typeof overrides[snake] === "boolean") {
      result[camel] = overrides[snake] as boolean;
    }
  }
  return result;
}

export function qualityProfileEntryFromGraph(
  profile: QualityProfileGraphPayload,
): ParsedQualityProfileEntry {
  return {
    id: profile.id.trim(),
    name: profile.name.trim() || profile.id.trim(),
    criteria: {
      quality_tiers: dedupeOrdered(
        profile.criteria.qualityTiers.map((value) => value.trim().toUpperCase()),
      ),
      archival_quality: profile.criteria.archivalQuality?.trim().toUpperCase() || null,
      allow_unknown_quality: profile.criteria.allowUnknownQuality,
      source_allowlist: dedupeOrdered(profile.criteria.sourceAllowlist.map((value) => value.trim())),
      source_blocklist: dedupeOrdered(profile.criteria.sourceBlocklist.map((value) => value.trim())),
      video_codec_allowlist: dedupeOrdered(
        profile.criteria.videoCodecAllowlist.map((value) => value.trim()),
      ),
      video_codec_blocklist: dedupeOrdered(
        profile.criteria.videoCodecBlocklist.map((value) => value.trim()),
      ),
      audio_codec_allowlist: dedupeOrdered(
        profile.criteria.audioCodecAllowlist.map((value) => value.trim()),
      ),
      audio_codec_blocklist: dedupeOrdered(
        profile.criteria.audioCodecBlocklist.map((value) => value.trim()),
      ),
      dolby_vision_allowed: profile.criteria.dolbyVisionAllowed,
      detected_hdr_allowed: profile.criteria.detectedHdrAllowed,
      prefer_remux: profile.criteria.preferRemux,
      allow_bd_disk: profile.criteria.allowBdDisk,
      allow_upgrades: profile.criteria.allowUpgrades,
      scoring_overrides: scoringOverridesFromGraph(profile.criteria.scoringOverrides),
      cutoff_tier: profile.criteria.cutoffTier?.trim().toUpperCase() || null,
      min_score_to_grab: profile.criteria.minScoreToGrab,
    },
  };
}

export function qualityProfileSettingsToEntries(
  payload: QualityProfileSettingsPayload | null | undefined,
): ParsedQualityProfileEntry[] {
  return (payload?.profiles ?? []).map(qualityProfileEntryFromGraph);
}

export function qualityProfileSettingsToCatalogText(
  payload: QualityProfileSettingsPayload | null | undefined,
): string {
  return normalizeQualityProfilesForUi(JSON.stringify(qualityProfileSettingsToEntries(payload)));
}

export function qualityProfileSettingsToCategoryOverrides(
  payload: QualityProfileSettingsPayload | null | undefined,
): Record<ViewCategoryId, string> {
  const result: Record<ViewCategoryId, string> = {
    movie: QUALITY_PROFILE_INHERIT_VALUE,
    series: QUALITY_PROFILE_INHERIT_VALUE,
    anime: QUALITY_PROFILE_INHERIT_VALUE,
  };

  for (const selection of payload?.categorySelections ?? []) {
    result[selection.scope] =
      selection.overrideProfileId?.trim() || QUALITY_PROFILE_INHERIT_VALUE;
  }

  return result;
}

export function qualityProfileSettingsToCategoryPersonaSelections(
  payload: QualityProfileSettingsPayload | null | undefined,
): Record<ViewCategoryId, FacetScoringPersonaSelectionRecord> {
  const globalPersona = payload?.globalScoringPersona ?? "Balanced";
  const result = buildDefaultCategoryPersonaSelections(globalPersona);

  for (const selection of payload?.categoryPersonaSelections ?? []) {
    result[selection.scope] = selection;
  }

  return result;
}

function readScoringOverrides(raw: unknown): ScoringOverridesPayload {
  if (!raw || typeof raw !== "object") return {};
  const obj = raw as Record<string, unknown>;
  const result: ScoringOverridesPayload = {};
  const keys = [
    "allow_x265_non4k",
    "block_dv_without_fallback",
    "prefer_compact_encodes",
    "prefer_lossless_audio",
    "block_upscaled",
  ] as const;
  for (const key of keys) {
    if (typeof obj[key] === "boolean") {
      result[key] = obj[key] as boolean;
    }
  }
  return result;
}

export function resolveQualityProfileCatalogState(rawValue: string): ProfileCatalogParseResult {
  const normalizedText = normalizeQualityProfilesForUi(rawValue);
  const parsedEntries = parseQualityProfileCatalogEntries(normalizedText);
  const parsedProfiles = parseQualityProfileCatalog(normalizedText);

  if (parsedEntries.length > 0) {
    return {
      text: normalizedText,
      entries: parsedEntries,
      profiles: parsedProfiles,
      isRawValid: true,
    };
  }

  return {
    text: normalizedText,
    entries: [],
    profiles: [],
    isRawValid: false,
  };
}

export function extractPrimaryNumber(value: string): number | null {
  const numericMatches = Array.from(value.matchAll(/\d+/g), (match) => Number(match[0])).filter(
    (parsed) => Number.isFinite(parsed),
  );
  if (numericMatches.length > 0) {
    return numericMatches[numericMatches.length - 1];
  }

  const lower = value.toLowerCase();
  if (lower.includes("8k")) {
    return 4320;
  }
  if (lower.includes("4k")) {
    return 2160;
  }

  return null;
}

export function extractQualitySortValue(value: string): number | null {
  const upper = value.toUpperCase();
  if (upper.includes("8K")) {
    return 4320;
  }
  if (upper.includes("4K")) {
    return 2160;
  }

  return extractPrimaryNumber(upper);
}

export function sortStringByNumericDesc(left: string, right: string): number {
  const leftNumber = extractQualitySortValue(left) ?? Number.NEGATIVE_INFINITY;
  const rightNumber = extractQualitySortValue(right) ?? Number.NEGATIVE_INFINITY;
  if (leftNumber === rightNumber) {
    return left.localeCompare(right);
  }
  return rightNumber - leftNumber;
}

export function sortProfileListChoiceByNumericDesc(
  left: ProfileListChoice,
  right: ProfileListChoice,
): number {
  return (
    sortStringByNumericDesc(left.label, right.label) ||
    left.value.localeCompare(right.value)
  );
}

export function normalizeProfileIdFromName(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)+/g, "")
    .replace(/-+/g, "-");
  return slug.length > 0 ? slug : "quality-profile";
}

export function createUniqueProfileId(base: string, existingIds: string[]): string {
  const taken = new Set(existingIds.map((value) => value.toLowerCase()));
  let candidate = base;
  let suffix = 1;
  while (taken.has(candidate.toLowerCase())) {
    candidate = `${base}-${suffix}`;
    suffix += 1;
  }
  return candidate;
}

export function parseStringArrayValue(raw: unknown): string[] {
  if (!Array.isArray(raw)) {
    return [];
  }

  return raw
    .map((item) => (typeof item === "string" ? item.trim() : ""))
    .filter((item) => item.length > 0);
}

export function normalizeExclusiveProfileLists(
  allowedValues: string[],
  deniedValues: string[],
): {
  allowed: string[];
  denied: string[];
} {
  const deniedSet = new Set(dedupeOrdered(deniedValues));
  return {
    allowed: dedupeOrdered(allowedValues).filter((value) => !deniedSet.has(value)),
    denied: Array.from(deniedSet),
  };
}

export function dedupeOrdered(values: string[]): string[] {
  return values.reduce<string[]>((accumulator, value) => {
    const normalized = value.trim();
    if (!normalized || accumulator.includes(normalized)) {
      return accumulator;
    }
    accumulator.push(normalized);
    return accumulator;
  }, []);
}

export function normalizeQualityTierList(raw: unknown): string[] {
  return dedupeOrdered(
    parseStringArrayValue(raw).map((value) => value.toUpperCase()),
  );
}

export function buildQualityProfileTemplate(profileId: string, profileName: string): QualityProfileDraft {
  return {
    id: profileId,
    name: profileName,
    quality_tiers: [...DEFAULT_QUALITY_PROFILE_QUALITY_TIERS],
    archival_quality: "2160P",
    allow_unknown_quality: false,
    source_allowlist: [],
    source_blocklist: [],
    video_codec_allowlist: [],
    video_codec_blocklist: [],
    audio_codec_allowlist: [],
    audio_codec_blocklist: [],
    dolby_vision_allowed: true,
    detected_hdr_allowed: true,
    prefer_remux: false,
    allow_bd_disk: true,
    allow_upgrades: true,
    scoring_overrides: {},
    cutoff_tier: "",
    min_score_to_grab: null,
  };
}

export function toQualityProfileDraft(
  entry: ParsedQualityProfileEntry | null,
  fallbackId: string,
  fallbackName: string,
): QualityProfileDraft {
  if (!entry) {
    return buildQualityProfileTemplate(fallbackId, fallbackName);
  }

  const sourceLists = normalizeExclusiveProfileLists(
    parseStringArrayValue(entry.criteria.source_allowlist),
    parseStringArrayValue(entry.criteria.source_blocklist),
  );
  const videoCodecLists = normalizeExclusiveProfileLists(
    parseStringArrayValue(entry.criteria.video_codec_allowlist),
    parseStringArrayValue(entry.criteria.video_codec_blocklist),
  );
  const audioCodecLists = normalizeExclusiveProfileLists(
    parseStringArrayValue(entry.criteria.audio_codec_allowlist),
    parseStringArrayValue(entry.criteria.audio_codec_blocklist),
  );

  return {
    id: typeof entry.id === "string" && entry.id.trim() ? entry.id.trim() : fallbackId,
    name:
      typeof entry.name === "string" && entry.name.trim().length > 0
        ? entry.name.trim()
        : fallbackName,
    source_allowlist: sourceLists.allowed,
    source_blocklist: sourceLists.denied,
    video_codec_allowlist: videoCodecLists.allowed,
    video_codec_blocklist: videoCodecLists.denied,
    audio_codec_allowlist: audioCodecLists.allowed,
    audio_codec_blocklist: audioCodecLists.denied,
    quality_tiers:
      typeof entry.criteria.quality_tiers === "undefined" || entry.criteria.quality_tiers === null
        ? [...DEFAULT_QUALITY_PROFILE_QUALITY_TIERS]
        : normalizeQualityTierList(entry.criteria.quality_tiers),
    archival_quality:
      typeof entry.criteria.archival_quality === "string" && entry.criteria.archival_quality.trim().length > 0
        ? entry.criteria.archival_quality.trim().toUpperCase()
        : "2160P",
    allow_unknown_quality:
      typeof entry.criteria.allow_unknown_quality === "boolean"
        ? entry.criteria.allow_unknown_quality
        : false,
    dolby_vision_allowed:
      typeof entry.criteria.dolby_vision_allowed === "boolean"
        ? entry.criteria.dolby_vision_allowed
        : true,
    detected_hdr_allowed:
      typeof entry.criteria.detected_hdr_allowed === "boolean"
        ? entry.criteria.detected_hdr_allowed
        : true,
    prefer_remux:
      typeof entry.criteria.prefer_remux === "boolean" ? entry.criteria.prefer_remux : false,
    allow_bd_disk: typeof entry.criteria.allow_bd_disk === "boolean" ? entry.criteria.allow_bd_disk : true,
    allow_upgrades:
      typeof entry.criteria.allow_upgrades === "boolean" ? entry.criteria.allow_upgrades : true,
    scoring_overrides: readScoringOverrides(entry.criteria.scoring_overrides),
    cutoff_tier:
      typeof entry.criteria.cutoff_tier === "string" && entry.criteria.cutoff_tier.trim()
        ? entry.criteria.cutoff_tier.trim().toUpperCase()
        : "",
    min_score_to_grab:
      typeof entry.criteria.min_score_to_grab === "number"
        ? entry.criteria.min_score_to_grab
        : null,
  };
}

export function qualityProfileCatalogEntryFromDraft(draft: QualityProfileDraft): ParsedQualityProfileEntry {
  const sourceLists = normalizeExclusiveProfileLists(draft.source_allowlist, draft.source_blocklist);
  const videoCodecLists = normalizeExclusiveProfileLists(
    draft.video_codec_allowlist,
    draft.video_codec_blocklist,
  );
  const audioCodecLists = normalizeExclusiveProfileLists(
    draft.audio_codec_allowlist,
    draft.audio_codec_blocklist,
  );

  return {
    id: draft.id,
    name: draft.name,
    criteria: {
      quality_tiers: draft.quality_tiers,
      archival_quality: draft.archival_quality || null,
      allow_unknown_quality: draft.allow_unknown_quality,
      source_allowlist: sourceLists.allowed,
      source_blocklist: sourceLists.denied,
      video_codec_allowlist: videoCodecLists.allowed,
      video_codec_blocklist: videoCodecLists.denied,
      audio_codec_allowlist: audioCodecLists.allowed,
      audio_codec_blocklist: audioCodecLists.denied,
      dolby_vision_allowed: draft.dolby_vision_allowed,
      detected_hdr_allowed: draft.detected_hdr_allowed,
      prefer_remux: draft.prefer_remux,
      allow_bd_disk: draft.allow_bd_disk,
      allow_upgrades: draft.allow_upgrades,
      scoring_overrides: draft.scoring_overrides,
      cutoff_tier: draft.cutoff_tier || null,
      min_score_to_grab: draft.min_score_to_grab,
    },
  };
}

export function qualityProfileEntryToMutationInput(entry: ParsedQualityProfileEntry) {
  return {
    id: entry.id,
    name: entry.name,
    criteria: {
      qualityTiers: entry.criteria.quality_tiers,
      archivalQuality: entry.criteria.archival_quality,
      allowUnknownQuality: entry.criteria.allow_unknown_quality,
      sourceAllowlist: entry.criteria.source_allowlist,
      sourceBlocklist: entry.criteria.source_blocklist,
      videoCodecAllowlist: entry.criteria.video_codec_allowlist,
      videoCodecBlocklist: entry.criteria.video_codec_blocklist,
      audioCodecAllowlist: entry.criteria.audio_codec_allowlist,
      audioCodecBlocklist: entry.criteria.audio_codec_blocklist,
      dolbyVisionAllowed: entry.criteria.dolby_vision_allowed,
      detectedHdrAllowed: entry.criteria.detected_hdr_allowed,
      preferRemux: entry.criteria.prefer_remux,
      allowBdDisk: entry.criteria.allow_bd_disk,
      allowUpgrades: entry.criteria.allow_upgrades,
      scoringOverrides: scoringOverridesToGraphInput(entry.criteria.scoring_overrides),
      cutoffTier: entry.criteria.cutoff_tier,
      minScoreToGrab: entry.criteria.min_score_to_grab,
    },
  };
}

export function normalizeQualityProfilesForUi(rawValue: string): string {
  const trimmedValue = rawValue.trim();
  if (!trimmedValue) {
    return "";
  }

  try {
    return JSON.stringify(JSON.parse(rawValue), null, 2);
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to format quality profiles JSON for UI", { rawValue, error });
    }
    return rawValue;
  }
}

export function normalizeQualityProfilesForSave(rawValue: string): string {
  try {
    return JSON.stringify(JSON.parse(rawValue));
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to format quality profiles JSON for save", { rawValue, error });
    }
    return rawValue;
  }
}

export function parseQualityProfileCatalogEntries(rawValue: string): ParsedQualityProfileEntry[] {
  if (!rawValue.trim()) {
    return [];
  }

  try {
    const parsed = JSON.parse(rawValue);
    return Array.isArray(parsed)
      ? parsed
          .map((entry) => normalizeQualityProfileEntry(entry))
          .filter((entry): entry is ParsedQualityProfileEntry => entry !== null)
      : [];
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to parse quality profile catalog entries", { rawValue, error });
    }
    return [];
  }
}

export function parseQualityProfileCatalog(rawValue: string): ParsedQualityProfile[] {
  if (!rawValue.trim()) {
    return [];
  }

  try {
    const entries = parseQualityProfileCatalogEntries(rawValue);

    const normalized = entries
      .map((entry) => {
        if (!entry || typeof entry !== "object") {
          return null;
        }

        const cast = entry as { id?: unknown; name?: unknown };
        if (typeof cast.id !== "string" || !cast.id.trim()) {
          return null;
        }

        return {
          id: cast.id.trim(),
          name: typeof cast.name === "string" ? cast.name.trim() : cast.id.trim(),
        } satisfies ParsedQualityProfile;
      })
      .filter((entry): entry is ParsedQualityProfile => entry !== null);

    return normalized;
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to parse quality profile catalog", { rawValue, error });
    }
    return [];
  }
}

export function coerceProfileSetting(rawValue?: string | null): string {
  const normalized = normalizeProfileId(rawValue ?? "");
  return normalized || QUALITY_PROFILE_INHERIT_VALUE;
}

export function normalizeProfileId(value: string | null | undefined): string {
  if (!value) {
    return "";
  }

  const trimmed = value.trim();
  if (!trimmed) {
    return "";
  }

  if (trimmed === QUALITY_PROFILE_INHERIT_VALUE) {
    return QUALITY_PROFILE_INHERIT_VALUE;
  }

  if (trimmed.startsWith('"')) {
    try {
      const parsed = JSON.parse(trimmed);
      if (typeof parsed === "string") {
        return parsed.trim();
      }
    } catch {
      // keep raw value if it is not valid JSON
    }
  }

  return trimmed;
}

export function toProfileOptions(profiles: ParsedQualityProfile[]) {
  const deduped = profiles.reduce<Array<{ value: string; label: string }>>(
    (accumulator, profile) => {
      const id = profile.id.trim();
      if (
        !id ||
        id === QUALITY_PROFILE_INHERIT_VALUE ||
        accumulator.some((entry) => entry.value === id)
      ) {
        return accumulator;
      }

      accumulator.push({
        value: id,
        label: profile.name?.trim().length ? profile.name!.trim() : id,
      });

      return accumulator;
    },
    [],
  );

  return deduped;
}

export function isValidProfileSelection(profiles: ParsedQualityProfile[], profileId: string) {
  if (profileId === QUALITY_PROFILE_INHERIT_VALUE) {
    return true;
  }

  return profiles.some((profile) => profile.id === profileId);
}
