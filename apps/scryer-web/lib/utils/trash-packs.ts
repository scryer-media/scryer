import type { RuleSetRecord } from "@/lib/types/rule-sets";
import {
  getSubtitleLanguage,
  SUBTITLE_LANGUAGES,
} from "../constants/subtitle-languages.ts";

export const TRASH_LOCALE_PACK_KEY_PREFIX = "trash-guides:locale:";
const FRENCH_PACK_KEY_PREFIX = "trash-guides:locale:french-";

// Stable display order: the three mutually exclusive French variants first,
// then the single-pack locales.
const PACK_DISPLAY_ORDER = [
  "trash-guides:locale:french-vf",
  "trash-guides:locale:french-vo",
  "trash-guides:locale:french-vostfr",
  "trash-guides:locale:german",
  "trash-guides:locale:asian",
];

export function isTrashLocalePack(record: RuleSetRecord): boolean {
  return (
    record.isManaged &&
    (record.managedKey?.startsWith(TRASH_LOCALE_PACK_KEY_PREFIX) ?? false)
  );
}

export function isFrenchLocalePack(record: RuleSetRecord): boolean {
  return record.managedKey?.startsWith(FRENCH_PACK_KEY_PREFIX) ?? false;
}

export function trashLocalePacks(records: RuleSetRecord[]): RuleSetRecord[] {
  const rank = (record: RuleSetRecord): number => {
    const index = PACK_DISPLAY_ORDER.indexOf(record.managedKey ?? "");
    return index === -1 ? PACK_DISPLAY_ORDER.length : index;
  };
  return records
    .filter(isTrashLocalePack)
    .sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}

/**
 * The already-enabled French pack that blocks enabling `target`, if any.
 *
 * The French packs read contradictory score sets, so the backend refuses to
 * enable a second one; catching it here lets the UI show a translated message
 * instead of the raw mutation error.
 */
export function conflictingFrenchPack(
  records: RuleSetRecord[],
  target: RuleSetRecord,
): RuleSetRecord | null {
  if (!isFrenchLocalePack(target)) return null;
  return (
    records.find(
      (record) =>
        record.id !== target.id &&
        record.enabled &&
        isFrenchLocalePack(record),
    ) ?? null
  );
}

/**
 * Comma-separated user input to a normalized tag list: trimmed, lowercased,
 * deduplicated, empties dropped. An empty result means "no filter" — the
 * backend collapses it to an open pack.
 */
export function parseTagFilterInput(raw: string): string[] {
  const seen = new Set<string>();
  const tags: string[] = [];
  for (const part of raw.split(",")) {
    const tag = part.trim().toLowerCase();
    if (tag && !seen.has(tag)) {
      seen.add(tag);
      tags.push(tag);
    }
  }
  return tags;
}

function languageCodeForLocaleTag(tag: string): string | null {
  const normalized = tag.trim().toLowerCase();
  if (!normalized.startsWith("locale:")) return null;

  const locale = normalized.slice("locale:".length);
  const language =
    getSubtitleLanguage(locale) ??
    SUBTITLE_LANGUAGES.find((candidate) => candidate.name.toLowerCase() === locale);
  return language?.code ?? null;
}

/** Converts persisted locale tags to the shared picker’s language codes. */
export function localeFilterLanguageCodes(
  tags: readonly string[] | null | undefined,
): string[] {
  const seen = new Set<string>();
  return (tags ?? []).flatMap((tag) => {
    const code = languageCodeForLocaleTag(tag);
    if (!code || seen.has(code)) return [];
    seen.add(code);
    return [code];
  });
}

/** Preserves filters the language picker cannot represent when the selection changes. */
export function localeFilterUnmappedTags(
  tags: readonly string[] | null | undefined,
): string[] {
  return (tags ?? [])
    .map((tag) => tag.trim().toLowerCase())
    .filter((tag) => tag && !languageCodeForLocaleTag(tag));
}

/** Converts shared picker values into canonical locale tags for the managed-rule API. */
export function localeTagsForLanguageCodes(codes: readonly string[]): string[] {
  return codes.map((code) => `locale:${code}`);
}

export function formatTagFilter(
  tags: readonly string[] | null | undefined,
): string {
  return (tags ?? []).join(", ");
}
