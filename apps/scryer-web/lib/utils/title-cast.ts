import type { TitleCreditRecord } from "@/lib/types/titles";

/**
 * Cards shown per rail. The overview fetches up to the server's 50-credit clamp
 * so the original/dub split has enough rows on both sides, then each rail caps
 * its own display here.
 */
export const TITLE_CAST_RAIL_DISPLAY_LIMIT = 15;

/** A rendered cast card may reserve more than one original-cast slot. */
export type TitleCastDisplayCredit = TitleCreditRecord & {
  slotSpan?: number;
};

/**
 * Drop credits with nothing to render. Placeholder slots on the dub rail are
 * the one exception — they carry a character but no person on purpose — so
 * they are built after this filter, never through it.
 */
export function titleCastCredits(
  credits: TitleCreditRecord[] | null | undefined,
): TitleCreditRecord[] {
  return (credits ?? []).filter(
    (credit) => (credit?.personName ?? "").trim().length > 0,
  );
}

/**
 * Top-billed first. Every rail uses this: billing rank is what "top billed
 * cast" means, and re-ordering by anything else (character name, say) buries
 * the leads.
 *
 * The later keys only break ties, and are chosen to be stable rather than
 * meaningful — two credits sharing a billing rank must not swap between
 * renders.
 */
export function sortTitleCastByBilling(
  credits: TitleCreditRecord[],
): TitleCreditRecord[] {
  return [...credits].sort((left, right) => {
    const leftBilling = left.billingOrder ?? Number.MAX_SAFE_INTEGER;
    const rightBilling = right.billingOrder ?? Number.MAX_SAFE_INTEGER;
    if (leftBilling !== rightBilling) {
      return leftBilling - rightBilling;
    }
    const byCharacter = (left.character ?? "").localeCompare(
      right.character ?? "",
    );
    if (byCharacter !== 0) {
      return byCharacter;
    }
    return (left.personName ?? "").localeCompare(right.personName ?? "");
  });
}

/**
 * Main-rail cast: on-screen performers plus the original Japanese voice cast,
 * top-billed first. TMDB actor rows carry no language; anime titles only have
 * voice_actor rows, so the `ja` filter is what keeps the main rail single-cast.
 */
export function titleCastOriginalCredits(
  credits: TitleCreditRecord[] | null | undefined,
): TitleCreditRecord[] {
  const cast = titleCastCredits(credits).filter(
    (credit) =>
      credit.kind !== "voice_actor" || (credit.language ?? "") === "ja",
  );
  return sortTitleCastByBilling(cast).slice(0, TITLE_CAST_RAIL_DISPLAY_LIMIT);
}

/**
 * Dub-rail cast: voice actors in one non-Japanese dub language, top-billed
 * first. Empty for movies and live-action series, which renders no dub rail.
 *
 * `language` is a dub language code (`en`, `de`, ...). Omitting it returns
 * every dub language at once, which is only useful for "is there a dub rail"
 * checks — the rail itself always picks one language.
 */
export function titleCastDubCredits(
  credits: TitleCreditRecord[] | null | undefined,
  language?: string | null,
): TitleCreditRecord[] {
  const cast = titleCastCredits(credits).filter(
    (credit) =>
      credit.kind === "voice_actor" &&
      (credit.language ?? "").length > 0 &&
      credit.language !== "ja" &&
      (!language || credit.language === language),
  );
  return sortTitleCastByBilling(cast).slice(0, TITLE_CAST_RAIL_DISPLAY_LIMIT);
}

/** A dub slot the provider has no actor for; rendered as an empty card. */
export function isTitleCastPlaceholder(credit: TitleCreditRecord): boolean {
  return (credit.personName ?? "").trim().length === 0;
}

/**
 * The dub rail, laid out to match `original` column for column.
 *
 * Sorting both rails by billing rank is nearly enough — AniList assigns the
 * rank per CHARACTER edge, so a character's Japanese and dubbed rows already
 * share one — but it drifts the moment a character has no actor in the chosen
 * dub: every column after it shifts, and each dub portrait sits under the wrong
 * face. So the original rail defines the slots and this fills them, emitting a
 * placeholder where the dub has nobody. Pairing is by billing rank (unique per
 * character within a title) and falls back to the character name for providers
 * that do not rank consistently.
 *
 * When more than one original credit resolves to one dub actor, that actor
 * spans the contiguous source slots rather than appearing as duplicate cards.
 * Returns an empty list when no dub actor exists at all, so the rail stays
 * hidden rather than rendering a row of placeholders.
 */
export function titleCastDubCreditsAlignedTo(
  credits: TitleCreditRecord[] | null | undefined,
  language: string | null | undefined,
  original: TitleCreditRecord[],
): TitleCastDisplayCredit[] {
  const dub = titleCastDubCredits(credits, language);
  if (dub.length === 0) {
    return [];
  }
  if (original.length === 0) {
    return dub;
  }

  const byBilling = new Map<number, TitleCreditRecord>();
  const byCharacter = new Map<string, TitleCreditRecord>();
  for (const credit of dub) {
    if (typeof credit.billingOrder === "number") {
      byBilling.set(credit.billingOrder, credit);
    }
    const character = (credit.character ?? "").trim();
    if (character.length > 0 && !byCharacter.has(character)) {
      byCharacter.set(character, credit);
    }
  }

  const matches = original.map((slot) => {
    const character = (slot.character ?? "").trim();
    return (
      (typeof slot.billingOrder === "number"
        ? byBilling.get(slot.billingOrder)
        : undefined) ??
      (character.length > 0 ? byCharacter.get(character) : undefined)
    );
  });

  const aligned: TitleCastDisplayCredit[] = [];
  for (let index = 0; index < original.length; ) {
    const slot = original[index];
    const matched = matches[index];
    if (!matched) {
      aligned.push({
        kind: slot.kind,
        personName: "",
        character: slot.character ?? "",
        language: language ?? "",
        billingOrder: slot.billingOrder ?? null,
        personImageUrl: null,
        episodeCount: null,
      });
      index += 1;
      continue;
    }

    let slotSpan = 1;
    while (matches[index + slotSpan] === matched) {
      slotSpan += 1;
    }
    aligned.push(slotSpan === 1 ? matched : { ...matched, slotSpan });
    index += slotSpan;
  }

  return aligned;
}

/**
 * Dub languages this title actually has credits for, sorted by code so the
 * picker order is stable. SMG only harvests `ja`/`en` today, but its VA config
 * already supports es/fr/de/it/pt/ko/zh — those appear here on their own once
 * the data flows, with no client change.
 */
export function titleCastDubLanguages(
  credits: TitleCreditRecord[] | null | undefined,
): string[] {
  const languages = new Set<string>();
  for (const credit of titleCastCredits(credits)) {
    const language = credit.language ?? "";
    if (credit.kind === "voice_actor" && language.length > 0 && language !== "ja") {
      languages.add(language);
    }
  }
  return [...languages].sort();
}

/**
 * Which dub language the rail opens on: the viewer's prior pick when that
 * language is still present, otherwise English, otherwise the first available.
 */
export function titleCastPreferredDubLanguage(
  languages: string[],
  preferred?: string | null,
): string | null {
  if (preferred && languages.includes(preferred)) {
    return preferred;
  }
  if (languages.includes("en")) {
    return "en";
  }
  return languages[0] ?? null;
}

/**
 * Localized display name for a dub language code, falling back to the raw code
 * for anything the runtime cannot name.
 */
export function titleCastDubLanguageLabel(
  language: string,
  locale?: string,
): string {
  try {
    const names = new Intl.DisplayNames([locale ?? "en"], { type: "language" });
    return names.of(language) ?? language;
  } catch {
    return language;
  }
}

/**
 * React key for a cast card. Person identity is deliberately not exposed by the
 * API, so billing rank plus the rendered name is the most stable key available;
 * the index keeps it unique when a provider bills two people identically.
 */
export function titleCastCreditKey(
  credit: TitleCreditRecord,
  index: number,
): string {
  return `${credit.billingOrder ?? index}-${credit.personName}-${index}`;
}

/**
 * Episode count to show under a cast card, or null when the provider does not
 * count episodes for this title (movies) or reported a meaningless value.
 */
export function titleCastCreditEpisodeCount(
  credit: TitleCreditRecord,
): number | null {
  const count = credit.episodeCount;
  if (typeof count !== "number" || !Number.isFinite(count) || count <= 0) {
    return null;
  }
  return Math.trunc(count);
}

/** Character subline, or null when the provider supplied none. */
export function titleCastCreditCharacter(
  credit: TitleCreditRecord,
): string | null {
  const character = (credit.character ?? "").trim();
  return character.length > 0 ? character : null;
}
