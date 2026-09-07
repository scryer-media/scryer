import type { IndexerCapsCategory } from "@/lib/types/indexers";

export type IndexerCapsScope = "MOVIE" | "SERIES" | "ANIME";

export type IndexerCapsCategoryOverlay = {
  /** Indexer labels for codes the standard newznab tree already lists. */
  labelsByCode: Map<string, string>;
  /**
   * Categories the indexer advertises for this scope that the standard tree
   * does not list (a "Movies-DE" subcat, a custom id above 100000).
   */
  extraCategories: IndexerCapsCategory[];
};

const MOVIE_RANGE: readonly [number, number] = [2000, 3000];
const TV_RANGE: readonly [number, number] = [5000, 6000];
/** Newznab reserves 1000..8999 for its standard top-level trees. */
const STANDARD_RANGE: readonly [number, number] = [1000, 9000];

function inRange(value: number, [start, end]: readonly [number, number]): boolean {
  return value >= start && value < end;
}

/**
 * Which routing scopes a caps category belongs to. Mirrors the Prowlarr
 * import classifier: the 2000s are movies, the 5000s are TV, and a name
 * carrying "anime", "movie", "tv", or "series" wins over the number. Ids in
 * another standard newznab tree (audio, books, console, xxx) belong to no
 * media scope, and anything outside the standard trees (custom ids, string
 * codes) is offered everywhere because nothing says what it holds.
 */
export function capsCategoryScopes(category: IndexerCapsCategory): IndexerCapsScope[] {
  const scopes = new Set<IndexerCapsScope>();
  const name = (category.label ?? "").trim().toLowerCase();
  const numeric = /^\d+$/.test(category.code.trim())
    ? Number.parseInt(category.code.trim(), 10)
    : null;

  if (name.includes("anime")) {
    scopes.add("ANIME");
  }
  if (name.includes("movie") || name.includes("film")) {
    scopes.add("MOVIE");
  }
  if (/^tv(?![a-z])/.test(name) || name.includes("series")) {
    scopes.add("SERIES");
    scopes.add("ANIME");
  }
  if (numeric !== null && inRange(numeric, MOVIE_RANGE)) {
    scopes.add("MOVIE");
  }
  if (numeric !== null && inRange(numeric, TV_RANGE)) {
    scopes.add("SERIES");
    scopes.add("ANIME");
  }
  if (scopes.size === 0 && (numeric === null || !inRange(numeric, STANDARD_RANGE))) {
    return ["MOVIE", "SERIES", "ANIME"];
  }
  return Array.from(scopes);
}

/**
 * Split an indexer's caps categories for one routing scope into labels for
 * codes the picker already draws and extra categories it should add. Codes
 * are deduplicated on first occurrence and returned in caps order.
 */
export function overlayCapsCategories(
  scope: IndexerCapsScope,
  categories: readonly IndexerCapsCategory[],
  knownCodes: ReadonlySet<string>,
): IndexerCapsCategoryOverlay {
  const labelsByCode = new Map<string, string>();
  const extraCategories: IndexerCapsCategory[] = [];
  const seen = new Set<string>();
  for (const category of categories) {
    const code = category.code.trim();
    if (!code || seen.has(code)) {
      continue;
    }
    seen.add(code);
    const label = category.label?.trim() || null;
    if (knownCodes.has(code)) {
      if (label) {
        labelsByCode.set(code, label);
      }
      continue;
    }
    if (capsCategoryScopes({ code, label }).includes(scope)) {
      extraCategories.push({ code, label });
    }
  }
  return { labelsByCode, extraCategories };
}
