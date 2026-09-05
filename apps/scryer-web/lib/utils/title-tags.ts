import type {
  BulkTitleTagsDraft,
  TitleTagDefinition,
  TitleTagRewriteCounts,
  TitleTagsDelta,
} from "@/lib/types/title-tags";

/// Entries carrying this prefix are structured settings the app writes for
/// itself (quality profile, monitor type, and friends). They live in the same
/// bag as user tags and must never be shown in, or written by, a tag picker.
export const RESERVED_TITLE_TAG_PREFIX = "scryer:";

/// Mirrors `MAX_USER_TITLE_TAG_LEN` in `scryer-application/src/helpers.rs`.
export const MAX_TITLE_TAG_LENGTH = 64;

export const EMPTY_TITLE_TAGS_DELTA: TitleTagsDelta = { add: [], remove: [] };

export function isReservedTitleTag(label: string): boolean {
  return label.trim().toLowerCase().startsWith(RESERVED_TITLE_TAG_PREFIX);
}

/// The same normal form the backend stores: trimmed, lowercased, with internal
/// whitespace collapsed to one space. Applied on the client so a label typed
/// with stray spacing lines up with what the registry already holds instead of
/// being refused after a round trip.
export function normalizeTitleTagLabel(label: string): string {
  return label.trim().toLowerCase().replace(/\s+/g, " ");
}

/// User-visible tags on a title: reserved entries stripped, normalized,
/// deduplicated, and sorted so a picker renders the same order every time.
export function userTitleTags(tags: readonly string[] | null | undefined): string[] {
  if (!tags || tags.length === 0) {
    return [];
  }
  const seen = new Set<string>();
  for (const raw of tags) {
    if (typeof raw !== "string" || isReservedTitleTag(raw)) {
      continue;
    }
    const normalized = normalizeTitleTagLabel(raw);
    if (normalized) {
      seen.add(normalized);
    }
  }
  return Array.from(seen).sort();
}

/// The patch that turns `before` into `after`. Both sides are normalized
/// first, so a picker never sends a no-op that differs only in spelling.
export function titleTagsDelta(
  before: readonly string[] | null | undefined,
  after: readonly string[] | null | undefined,
): TitleTagsDelta {
  const beforeTags = userTitleTags(before);
  const afterTags = userTitleTags(after);
  const beforeSet = new Set(beforeTags);
  const afterSet = new Set(afterTags);
  return {
    add: afterTags.filter((label) => !beforeSet.has(label)),
    remove: beforeTags.filter((label) => !afterSet.has(label)),
  };
}

export function isEmptyTitleTagsDelta(delta: TitleTagsDelta): boolean {
  return delta.add.length === 0 && delta.remove.length === 0;
}

/// Registry labels a title does not already carry — the contents of the
/// picker's "add" select. Free text is never an option, so an empty result
/// means the registry itself is exhausted or empty.
export function availableTitleTagLabels(
  definitions: readonly TitleTagDefinition[],
  applied: readonly string[] | null | undefined,
): string[] {
  const appliedSet = new Set(userTitleTags(applied));
  const labels = new Set<string>();
  for (const definition of definitions) {
    const label = normalizeTitleTagLabel(definition.label);
    if (label && !appliedSet.has(label)) {
      labels.add(label);
    }
  }
  return Array.from(labels).sort();
}

/// One `updateTitleTags` patch from the bulk dialog's two pickers. A label
/// chosen in both is contradictory; removal wins, because the operator's
/// intent to strip a label is the destructive half and silently adding it
/// back would be the surprising outcome. The dialog also hides a label from
/// one picker once the other holds it, so this is a defensive floor.
export function buildBulkTitleTagsDelta(draft: BulkTitleTagsDraft): TitleTagsDelta {
  const remove = userTitleTags(draft.remove);
  const removeSet = new Set(remove);
  const add = userTitleTags(draft.add).filter((label) => !removeSet.has(label));
  return { add, remove };
}

export function hasBulkTitleTagsChanges(draft: BulkTitleTagsDraft): boolean {
  return !isEmptyTitleTagsDelta(buildBulkTitleTagsDelta(draft));
}

export function emptyBulkTitleTagsDraft(): BulkTitleTagsDraft {
  return { add: [], remove: [] };
}

/// The three places a rename cannot reach. Rego revisions are immutable and a
/// managed pack's tag filter belongs to the pack, so a rename leaves all three
/// naming the old label.
export const TITLE_TAG_REFERENCE_KINDS = [
  "maintenanceRuleSets",
  "releaseRuleSets",
  "managedTagFilters",
] as const;

export type TitleTagReferenceKind = (typeof TITLE_TAG_REFERENCE_KINDS)[number];

export type TitleTagReference = {
  kind: TitleTagReferenceKind;
  count: number;
};

export type TitleTagRenameWarning = {
  total: number;
  references: TitleTagReference[];
};

export const EMPTY_TITLE_TAG_REWRITE_COUNTS: TitleTagRewriteCounts = {
  titles: 0,
  seriesMovies: 0,
  delayProfiles: 0,
  maintenanceRuleSets: 0,
  releaseRuleSets: 0,
  managedTagFilters: 0,
};

/// Non-zero reference counts in a fixed order, or null when a rename left
/// nothing behind. Kept separate from the formatting so the section can decide
/// whether to render a warning at all before it builds a sentence.
export function titleTagRenameWarning(
  counts: TitleTagRewriteCounts,
): TitleTagRenameWarning | null {
  const references = TITLE_TAG_REFERENCE_KINDS.map((kind) => ({
    kind,
    count: Math.max(0, Math.trunc(counts[kind] ?? 0)),
  })).filter((reference) => reference.count > 0);
  if (references.length === 0) {
    return null;
  }
  return {
    total: references.reduce((sum, reference) => sum + reference.count, 0),
    references,
  };
}

const TITLE_TAG_REFERENCE_LABEL_KEYS: Record<TitleTagReferenceKind, string> = {
  maintenanceRuleSets: "settings.titleTagReferenceMaintenanceRuleSets",
  releaseRuleSets: "settings.titleTagReferenceReleaseRuleSets",
  managedTagFilters: "settings.titleTagReferenceManagedTagFilters",
};

export type TitleTagTranslate = (
  key: string,
  values?: Record<string, string | number>,
) => string;

/// The sentence the settings section shows after a rename: how many rule
/// sources still name the old label, and where they are. Null when a rename
/// rewrote everything that could be rewritten.
export function formatTitleTagRenameWarning(
  counts: TitleTagRewriteCounts,
  previousLabel: string,
  t: TitleTagTranslate,
): string | null {
  const warning = titleTagRenameWarning(counts);
  if (!warning) {
    return null;
  }
  const references = warning.references
    .map((reference) =>
      t(TITLE_TAG_REFERENCE_LABEL_KEYS[reference.kind], {
        count: reference.count,
      }),
    )
    .join(", ");
  return t("settings.titleTagRenameWarning", {
    label: previousLabel,
    count: warning.total,
    references,
  });
}

/// What a rename did rewrite, always shown so the operator can see the blast
/// radius even when nothing was left behind.
export function formatTitleTagRenameSummary(
  counts: TitleTagRewriteCounts,
  label: string,
  t: TitleTagTranslate,
): string {
  return t("settings.titleTagRenameSummary", {
    label,
    titles: Math.max(0, Math.trunc(counts.titles ?? 0)),
    seriesMovies: Math.max(0, Math.trunc(counts.seriesMovies ?? 0)),
    delayProfiles: Math.max(0, Math.trunc(counts.delayProfiles ?? 0)),
  });
}

/// Client-side guard mirroring the backend validator, so a bad label is named
/// before it costs a round trip. Returns a locale key, or null when valid.
export function titleTagLabelErrorKey(label: string): string | null {
  const normalized = normalizeTitleTagLabel(label);
  if (!normalized) {
    return "settings.titleTagLabelRequired";
  }
  if (isReservedTitleTag(normalized)) {
    return "settings.titleTagLabelReserved";
  }
  if (normalized.length > MAX_TITLE_TAG_LENGTH) {
    return "settings.titleTagLabelTooLong";
  }
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(normalized)) {
    return "settings.titleTagLabelInvalid";
  }
  return null;
}
