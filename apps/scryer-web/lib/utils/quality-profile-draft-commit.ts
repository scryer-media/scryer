import type {
  CommittedQualityProfileDraft,
  ParsedQualityProfileEntry,
  QualityProfileDraft,
} from "../types/quality-profiles.ts";
import { qualityProfileEntryToMutationInput } from "./quality-profiles.ts";

export function qualityProfileEditorSaveInput(
  entry: ParsedQualityProfileEntry,
) {
  // Other entries and defaults in the editor's catalog may be stale.
  return {
    profiles: [qualityProfileEntryToMutationInput(entry)],
    globalProfileId: null,
    globalScoringPersona: null,
    categorySelections: [],
    categoryPersonaSelections: [],
    replaceExisting: false,
  };
}

function normalizeProfileIdFromName(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)+/g, "")
    .replace(/-+/g, "-");
  return slug.length > 0 ? slug : "quality-profile";
}

function createUniqueProfileId(base: string, existingIds: string[]): string {
  const taken = new Set(existingIds.map((value) => value.toLowerCase()));
  let candidate = base;
  let suffix = 1;
  while (taken.has(candidate.toLowerCase())) {
    candidate = `${base}-${suffix}`;
    suffix += 1;
  }
  return candidate;
}

function dedupeOrdered(values: string[]): string[] {
  return values.reduce<string[]>((accumulator, value) => {
    const normalized = value.trim();
    if (!normalized || accumulator.includes(normalized)) {
      return accumulator;
    }
    accumulator.push(normalized);
    return accumulator;
  }, []);
}

function normalizeExclusiveProfileLists(
  allowedValues: string[],
  deniedValues: string[],
): {
  allowed: string[];
  denied: string[];
} {
  const deniedSet = new Set(dedupeOrdered(deniedValues));
  return {
    allowed: dedupeOrdered(allowedValues).filter(
      (value) => !deniedSet.has(value),
    ),
    denied: Array.from(deniedSet),
  };
}

function qualityProfileCatalogEntryFromDraft(
  draft: QualityProfileDraft,
): ParsedQualityProfileEntry {
  const sourceLists = normalizeExclusiveProfileLists(
    draft.source_allowlist,
    draft.source_blocklist,
  );
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
      ignore_proper_repack: draft.ignore_proper_repack,
      scoring_overrides: draft.scoring_overrides,
      cutoff_tier: draft.cutoff_tier || null,
      min_score_to_grab: draft.min_score_to_grab,
    },
  };
}

export function hasDuplicateQualityProfileName(
  entries: ParsedQualityProfileEntry[],
  candidateName: string,
  excludeId?: string | null,
): boolean {
  const normalizedCandidate = candidateName.trim().toLocaleLowerCase();
  if (!normalizedCandidate) {
    return false;
  }

  return entries.some(
    (entry) =>
      entry.id !== (excludeId ?? null) &&
      entry.name.trim().toLocaleLowerCase() === normalizedCandidate,
  );
}

export function commitQualityProfileDraftToEntries(
  entries: ParsedQualityProfileEntry[],
  draft: QualityProfileDraft,
): CommittedQualityProfileDraft {
  const trimmedName = draft.name.trim();
  const existingEntry = entries.find((entry) => entry.id === draft.id) ?? null;
  const existingIds = entries
    .map((entry) => entry.id.trim())
    .filter((entryId) => entryId.length > 0);
  const nextId =
    existingEntry?.id ??
    (draft.id.trim() ||
      createUniqueProfileId(
        normalizeProfileIdFromName(trimmedName),
        existingIds,
      ));
  const nextDraft: QualityProfileDraft = {
    ...draft,
    id: nextId,
    name: trimmedName,
  };
  const draftEntry = qualityProfileCatalogEntryFromDraft(nextDraft);

  return {
    catalogEntries: existingEntry
      ? entries.map((entry) => (entry.id === nextId ? draftEntry : entry))
      : [...entries, draftEntry],
    draftEntry,
  };
}
