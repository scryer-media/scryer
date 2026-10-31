import { userFacingGraphQlErrorMessage } from "../graphql/error-message.ts";
import type {
  PostImportTracking,
  SeasonPackSeedMode,
  SeedGoalMetAction,
  SeedingProfileDraft,
  SeedingProfileRecord,
} from "@/lib/types/seeding-profiles";

/** Settings key backing the global default seeding profile. */
export const DEFAULT_SEEDING_PROFILE_SETTING_KEY =
  "download_client.default_seeding_profile";

/**
 * Sentinel select value for "no profile". Radix selects cannot carry an empty
 * string value, so assignment dropdowns use this the way the download-client
 * mapping dropdown uses `AUTOMATIC_DOWNLOAD_CLIENT_ID`.
 */
export const SEEDING_PROFILE_INHERIT_VALUE = "__inherit__";

export const SEASON_PACK_SEED_MODES: readonly SeasonPackSeedMode[] = [
  "INHERIT",
  "OVERRIDE",
];

export const SEED_GOAL_MET_ACTIONS: readonly SeedGoalMetAction[] = [
  "REMOVE_ENTRY",
  "STOP_SEEDING",
  "KEEP",
];

export const POST_IMPORT_TRACKING_MODES: readonly PostImportTracking[] = [
  "PARK",
  "HAND_OFF",
];

export function buildSeedingProfileTemplate(): SeedingProfileDraft {
  return {
    id: "",
    name: "",
    ratio: "",
    seedTimeMinutes: "",
    seasonPackMode: "INHERIT",
    seasonPackRatio: "",
    seasonPackSeedTimeMinutes: "",
    honorTrackerMinimums: true,
    goalMetAction: "REMOVE_ENTRY",
    neverRemove: false,
    // Empty inherits the system floor, which is what a new profile should do
    // until an operator deliberately overrides it for this tracker.
    minimumSeeders: "",
    // Parking keeps Scryer managing the torrent, which is what every install
    // did before handoff existed.
    postImportTracking: "PARK",
  };
}

function numberToDraftValue(value: number | null | undefined): string {
  return value === null || value === undefined ? "" : String(value);
}

export function seedingProfileToDraft(
  profile: SeedingProfileRecord,
): SeedingProfileDraft {
  return {
    id: profile.id,
    name: profile.name,
    ratio: numberToDraftValue(profile.ratio),
    seedTimeMinutes: formatSeedDuration(profile.seedTimeMinutes),
    seasonPackMode: profile.seasonPackMode,
    seasonPackRatio: numberToDraftValue(profile.seasonPackRatio),
    seasonPackSeedTimeMinutes: formatSeedDuration(
      profile.seasonPackSeedTimeMinutes,
    ),
    honorTrackerMinimums: profile.honorTrackerMinimums,
    goalMetAction: profile.goalMetAction,
    neverRemove: profile.neverRemove,
    minimumSeeders: numberToDraftValue(profile.minimumSeeders),
    postImportTracking: profile.postImportTracking,
  };
}

type ParsedGoal = { ok: true; value: number | null } | { ok: false };

/**
 * Unlike the goal fields, zero is meaningful here: it disables the check for
 * this profile rather than reading as "unset". Empty still means inherit.
 */
export function parseMinimumSeeders(raw: string): ParsedGoal {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: true, value: null };
  }
  if (!/^\d+$/.test(trimmed)) {
    return { ok: false };
  }
  const value = Number(trimmed);
  return Number.isSafeInteger(value) ? { ok: true, value } : { ok: false };
}

function parseOptionalRatio(raw: string): ParsedGoal {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: true, value: null };
  }
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return { ok: false };
  }
  return { ok: true, value: parsed };
}

const MINUTES_PER_UNIT: Record<string, number> = {
  m: 1,
  h: 60,
  d: 60 * 24,
  w: 60 * 24 * 7,
};

/// Largest unit first, so a duration only ever splits into the units it needs.
const SEED_DURATION_UNITS: [string, number][] = [
  ["w", MINUTES_PER_UNIT.w!],
  ["d", MINUTES_PER_UNIT.d!],
  ["h", MINUTES_PER_UNIT.h!],
  ["m", 1],
];

const SEED_DURATION_TOKEN = /(\d+)\s*([mhdw])/giu;

/**
 * Seed goals are set in weeks and days far more often than in minutes, so the
 * field takes durations (`90m`, `36h`, `2w`, `1d 12h`) and stores the minutes
 * the API wants. A bare number is still minutes, which is what the field
 * accepted before and what a tracker's own rules are usually quoted in.
 */
export function parseSeedDuration(raw: string): ParsedGoal {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: true, value: null };
  }
  if (/^\d+$/.test(trimmed)) {
    const parsed = Number(trimmed);
    return Number.isSafeInteger(parsed) && parsed > 0
      ? { ok: true, value: parsed }
      : { ok: false };
  }

  // Every character has to belong to a token, so "2w rubbish" is rejected
  // rather than silently read as "2w".
  const consumed = trimmed.replace(SEED_DURATION_TOKEN, "").trim();
  if (consumed) {
    return { ok: false };
  }

  let total = 0;
  const seen = new Set<string>();
  for (const match of trimmed.matchAll(SEED_DURATION_TOKEN)) {
    const unit = match[2]!.toLowerCase();
    // `1d 1d` is a typo, not two days.
    if (seen.has(unit)) {
      return { ok: false };
    }
    seen.add(unit);
    total += Number(match[1]) * MINUTES_PER_UNIT[unit]!;
  }
  return Number.isSafeInteger(total) && total > 0
    ? { ok: true, value: total }
    : { ok: false };
}

/**
 * Minutes rendered back into the duration syntax the field accepts, so a saved
 * profile reopens showing what was typed rather than its minute count.
 */
export function formatSeedDuration(minutes: number | null | undefined): string {
  if (minutes === null || minutes === undefined || minutes <= 0) {
    return "";
  }
  let remaining = Math.floor(minutes);
  const parts: string[] = [];
  for (const [unit, size] of SEED_DURATION_UNITS) {
    const count = Math.floor(remaining / size);
    if (count > 0) {
      parts.push(`${count}${unit}`);
      remaining -= count * size;
    }
  }
  return parts.join(" ");
}

/** Draft fields that can carry a validation message of their own. */
export type SeedingProfileField =
  | "name"
  | "ratio"
  | "seedTimeMinutes"
  | "seasonPackRatio"
  | "seasonPackSeedTimeMinutes"
  | "minimumSeeders";

export type SeedingProfileFieldErrors = Partial<
  Record<SeedingProfileField, string>
>;

/// Field order for the first-error summary; matches the form's reading order.
const SEEDING_PROFILE_FIELD_ORDER: SeedingProfileField[] = [
  "name",
  "ratio",
  "seedTimeMinutes",
  "seasonPackRatio",
  "seasonPackSeedTimeMinutes",
  "minimumSeeders",
];

/**
 * Mirrors the backend's `SeedingProfile::validate` so bad input is rejected
 * before the mutation round-trip, keyed by field so the editor can show each
 * message under the input that caused it.
 */
export function validateSeedingProfileFields(
  draft: SeedingProfileDraft,
): SeedingProfileFieldErrors {
  const errors: SeedingProfileFieldErrors = {};
  if (!draft.name.trim()) {
    errors.name = "Enter a name for this profile.";
  }
  if (!parseOptionalRatio(draft.ratio).ok) {
    errors.ratio = "Enter a number greater than zero, or leave empty.";
  }
  if (!parseSeedDuration(draft.seedTimeMinutes).ok) {
    errors.seedTimeMinutes =
      "Enter a duration like 90m, 36h, 1d 12h or 2w, or leave empty.";
  }
  // Season-pack goals only exist in override mode, so inherit mode cannot be
  // held back by whatever is sitting in those inputs.
  if (draft.seasonPackMode === "OVERRIDE") {
    if (!parseOptionalRatio(draft.seasonPackRatio).ok) {
      errors.seasonPackRatio =
        "Enter a number greater than zero, or leave empty.";
    }
    if (!parseSeedDuration(draft.seasonPackSeedTimeMinutes).ok) {
      errors.seasonPackSeedTimeMinutes =
        "Enter a duration like 90m, 36h, 1d 12h or 2w, or leave empty.";
    }
  }
  if (!parseMinimumSeeders(draft.minimumSeeders).ok) {
    errors.minimumSeeders =
      "Enter a whole number of 0 or more, or leave empty.";
  }
  return errors;
}

/** First field error in form order, for callers that want a single message. */
export function validateSeedingProfileDraft(
  draft: SeedingProfileDraft,
): string | null {
  const errors = validateSeedingProfileFields(draft);
  for (const field of SEEDING_PROFILE_FIELD_ORDER) {
    const message = errors[field];
    if (message) {
      return message;
    }
  }
  return null;
}

type SeedingProfileGoalInput = {
  name: string;
  ratio: number | null;
  seedTimeMinutes: number | null;
  seasonPackMode: SeasonPackSeedMode;
  seasonPackRatio: number | null;
  seasonPackSeedTimeMinutes: number | null;
  honorTrackerMinimums: boolean;
  goalMetAction: SeedGoalMetAction;
  neverRemove: boolean;
  minimumSeeders: number | null;
  postImportTracking: PostImportTracking;
};

/**
 * Normalizes a validated draft into mutation-input shape. Season-pack goals are
 * dropped in inherit mode, mirroring `SeedingProfile::normalized()` server-side
 * so the optimistic row matches what comes back.
 */
export function seedingProfileDraftToInput(
  draft: SeedingProfileDraft,
): SeedingProfileGoalInput {
  const ratio = parseOptionalRatio(draft.ratio);
  const seedTime = parseSeedDuration(draft.seedTimeMinutes);
  const seasonPackRatio = parseOptionalRatio(draft.seasonPackRatio);
  const seasonPackSeedTime = parseSeedDuration(draft.seasonPackSeedTimeMinutes);
  const minimumSeeders = parseMinimumSeeders(draft.minimumSeeders);
  const isOverride = draft.seasonPackMode === "OVERRIDE";

  return {
    name: draft.name.trim(),
    ratio: ratio.ok ? ratio.value : null,
    seedTimeMinutes: seedTime.ok ? seedTime.value : null,
    seasonPackMode: draft.seasonPackMode,
    seasonPackRatio:
      isOverride && seasonPackRatio.ok ? seasonPackRatio.value : null,
    seasonPackSeedTimeMinutes:
      isOverride && seasonPackSeedTime.ok ? seasonPackSeedTime.value : null,
    honorTrackerMinimums: draft.honorTrackerMinimums,
    goalMetAction: draft.goalMetAction,
    neverRemove: draft.neverRemove,
    minimumSeeders: minimumSeeders.ok ? minimumSeeders.value : null,
    postImportTracking: draft.postImportTracking,
  };
}

export function toCreateSeedingProfileInput(draft: SeedingProfileDraft) {
  return seedingProfileDraftToInput(draft);
}

/**
 * Every optional goal is sent explicitly so clearing a field in the editor
 * clears it server-side — `UpdateSeedingProfileInput` treats an explicit null
 * as "clear" and an omitted key as "preserve".
 */
export function toUpdateSeedingProfileInput(draft: SeedingProfileDraft) {
  return { id: draft.id, ...seedingProfileDraftToInput(draft) };
}

/** Reads the raw server message off a urql error without rewording it. */
export function extractSeedingProfileErrorMessage(
  error: unknown,
  translate?: Parameters<typeof userFacingGraphQlErrorMessage>[2],
): string | null {
  return userFacingGraphQlErrorMessage(error, "", translate) || null;
}

const EM_DASH = "—";

export function formatSeedingProfileRatio(ratio: number | null): string {
  return ratio === null ? EM_DASH : String(ratio);
}

export function formatSeedingProfileSeedTime(minutes: number | null): string {
  return minutes === null ? EM_DASH : formatSeedDuration(minutes);
}

/**
 * One-cell summary of the season-pack column: the override goals when the
 * profile overrides, otherwise the inherit marker.
 */
export function formatSeasonPackSummary(
  profile: Pick<
    SeedingProfileRecord,
    "seasonPackMode" | "seasonPackRatio" | "seasonPackSeedTimeMinutes"
  >,
  inheritLabel: string,
): string {
  if (profile.seasonPackMode !== "OVERRIDE") {
    return inheritLabel;
  }
  const parts = [
    profile.seasonPackRatio === null ? null : String(profile.seasonPackRatio),
    profile.seasonPackSeedTimeMinutes === null
      ? null
      : formatSeedDuration(profile.seasonPackSeedTimeMinutes),
  ].filter((part): part is string => part !== null);
  return parts.length === 0 ? EM_DASH : parts.join(" / ");
}

/**
 * The minimum-seeders deviation the profile table should spend ink on, or
 * `null` when the profile does not deviate.
 *
 * `null` inherits the system floor, so it says nothing the floor field does not
 * already say. `0` is an explicit opt-out of the check and always shows: it
 * stays an opt-out when the floor is raised later. Any other value earns a chip
 * only when it differs from the floor it would otherwise inherit — including a
 * value *below* the floor, which loosens admission instead of tightening it.
 */
export type SeedingProfileMinimumSeedersChip =
  { kind: "disabled" } | { kind: "threshold"; value: number };

export function seedingProfileMinimumSeedersChip(
  minimumSeeders: number | null,
  floor: number,
): SeedingProfileMinimumSeedersChip | null {
  if (minimumSeeders === null) {
    return null;
  }
  // Mirrors the backend clamp: a stored value at or below zero means "no check".
  if (minimumSeeders <= 0) {
    return { kind: "disabled" };
  }
  return minimumSeeders === floor
    ? null
    : { kind: "threshold", value: minimumSeeders };
}

/**
 * Whether a profile stops Scryer managing its torrents after import. Handoff
 * makes the goal-met action and the never-remove flag moot, so the editor and
 * the table both dim them rather than implying they still apply.
 */
export function handsOffAfterImport(
  postImportTracking: PostImportTracking,
): boolean {
  return postImportTracking === "HAND_OFF";
}

/** Resolves a stored assignment id to a select value, tolerating stale ids. */
export function seedingProfileSelectValue(
  seedingProfileId: string | null | undefined,
): string {
  return seedingProfileId ?? SEEDING_PROFILE_INHERIT_VALUE;
}

export function seedingProfileSelectValueToId(value: string): string | null {
  return value === SEEDING_PROFILE_INHERIT_VALUE ? null : value;
}

/**
 * Copy key for the inherit option of an indexer's seeding-profile picker.
 *
 * Prowlarr pushes two independent things to a managed child: seed goals, which
 * the "Managed by Prowlarr" sentinel speaks for, and an `appMinimumSeeders`
 * admission threshold, which it deliberately does not. A live Prowlarr always
 * sends the threshold (its app profile defaults to 1), so without naming it
 * here the row reads "Inherit default" while Prowlarr's value is what actually
 * governs admission.
 *
 * Zero is Prowlarr's explicit "do not enforce", not "inherit", and gets its own
 * wording. Keys are complete sentences rather than assembled fragments so the
 * plural and the off case stay translatable.
 */
export function seedingProfileInheritOptionKey(
  prowlarrManaged: boolean,
  prowlarrMinimumSeeders: number | null | undefined,
): string {
  if (prowlarrMinimumSeeders === null || prowlarrMinimumSeeders === undefined) {
    return prowlarrManaged
      ? "settings.seedingProfileProwlarrManaged"
      : "settings.seedingProfileInherit";
  }
  if (prowlarrMinimumSeeders <= 0) {
    return prowlarrManaged
      ? "settings.seedingProfileProwlarrManagedMinimumOff"
      : "settings.seedingProfileInheritProwlarrMinimumOff";
  }
  if (prowlarrMinimumSeeders === 1) {
    return prowlarrManaged
      ? "settings.seedingProfileProwlarrManagedMinimumOne"
      : "settings.seedingProfileInheritProwlarrMinimumOne";
  }
  return prowlarrManaged
    ? "settings.seedingProfileProwlarrManagedMinimum"
    : "settings.seedingProfileInheritProwlarrMinimum";
}

/**
 * True when an indexer can carry a seeding profile at all. The backend rejects
 * assignment on anything that is not torrent-capable, so the control mirrors
 * the download-client mapping's not-applicable handling instead of failing the
 * mutation.
 */
export function supportsSeedingProfileAssignment(
  protocolFamilies: readonly string[] | undefined,
): boolean {
  return (protocolFamilies ?? []).some(
    (family) => family.trim().toLowerCase() === "torrent",
  );
}
