import {
  DEFAULT_QUALITY_PROFILE_QUALITY_TIERS,
} from "@/lib/constants/quality-profiles";
import { QUALITY_PROFILE_INHERIT_VALUE } from "@/lib/constants/settings";
import type {
  ParsedQualityProfile,
  ParsedQualityProfileEntry,
  QualityProfileDraft,
  ProfileListChoice,
  ProfileRawRecord,
} from "@/lib/types";

type ProfileCatalogParseResult = {
  text: string;
  entries: ParsedQualityProfileEntry[];
  profiles: ParsedQualityProfile[];
  isRawValid: boolean;
};

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
      atmos_preferred: readBoolean(criteria.atmos_preferred, false),
      dolby_vision_allowed: readBoolean(criteria.dolby_vision_allowed, false),
      detected_hdr_allowed: readBoolean(criteria.detected_hdr_allowed, false),
      prefer_remux: readBoolean(criteria.prefer_remux, false),
      prefer_dual_audio: readBoolean(criteria.prefer_dual_audio, false),
      allow_bd_disk: readBoolean(criteria.allow_bd_disk, false),
      allow_upgrades: readBoolean(criteria.allow_upgrades, false),
    },
  };
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
    atmos_preferred: true,
    dolby_vision_allowed: true,
    detected_hdr_allowed: true,
    prefer_remux: true,
    prefer_dual_audio: false,
    allow_bd_disk: true,
    allow_upgrades: true,
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
    atmos_preferred:
      typeof entry.criteria.atmos_preferred === "boolean" ? entry.criteria.atmos_preferred : true,
    dolby_vision_allowed:
      typeof entry.criteria.dolby_vision_allowed === "boolean"
        ? entry.criteria.dolby_vision_allowed
        : true,
    detected_hdr_allowed:
      typeof entry.criteria.detected_hdr_allowed === "boolean"
        ? entry.criteria.detected_hdr_allowed
        : true,
    prefer_remux: typeof entry.criteria.prefer_remux === "boolean" ? entry.criteria.prefer_remux : true,
    prefer_dual_audio: typeof entry.criteria.prefer_dual_audio === "boolean" ? entry.criteria.prefer_dual_audio : false,
    allow_bd_disk: typeof entry.criteria.allow_bd_disk === "boolean" ? entry.criteria.allow_bd_disk : true,
    allow_upgrades:
      typeof entry.criteria.allow_upgrades === "boolean" ? entry.criteria.allow_upgrades : true,
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
      atmos_preferred: draft.atmos_preferred,
      dolby_vision_allowed: draft.dolby_vision_allowed,
      detected_hdr_allowed: draft.detected_hdr_allowed,
      prefer_remux: draft.prefer_remux,
      prefer_dual_audio: draft.prefer_dual_audio,
      allow_bd_disk: draft.allow_bd_disk,
      allow_upgrades: draft.allow_upgrades,
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

  try {
    const parsed = JSON.parse(trimmed);
    if (typeof parsed === "string") {
      return parsed.trim();
    }
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to parse quality profile ID as JSON", { value: trimmed, error });
    }
    // keep raw value if it is not JSON
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
