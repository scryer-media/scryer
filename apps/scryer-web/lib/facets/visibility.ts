import { FACET_REGISTRY, type FacetDefinition } from "./registry.ts";
import type { Facet } from "@/lib/types/titles";
import type { UiSettingsFacet } from "@/lib/types/settings";

/**
 * Facets the user chose to hide (`UiSettings.hiddenFacets`). Hiding removes a
 * facet as a choice or destination: nav entries, tabs, filter chips, pickers,
 * search sections and discovery content. It never hides facts about records
 * the user already has, such as a title's facet badge or its detail page.
 */
export type HiddenFacets = ReadonlySet<Facet>;

export const NO_HIDDEN_FACETS: HiddenFacets = new Set<Facet>();

const FROM_UI_SETTINGS: Record<UiSettingsFacet, Facet> = {
  MOVIES: "MOVIE",
  SERIES: "SERIES",
  ANIME: "ANIME",
};

const TO_UI_SETTINGS: Record<Facet, UiSettingsFacet> = {
  MOVIE: "MOVIES",
  SERIES: "SERIES",
  ANIME: "ANIME",
};

/**
 * Reads the saved list. Unknown values are dropped, and a list that would hide
 * every facet hides none: the server refuses that save, so it can only come
 * from a newer client or a bad row.
 */
export function hiddenFacetsFromUiSettings(
  values: readonly string[] | null | undefined,
): HiddenFacets {
  const hidden = new Set<Facet>();
  for (const value of values ?? []) {
    const facet = FROM_UI_SETTINGS[value as UiSettingsFacet];
    if (facet) hidden.add(facet);
  }
  return hidden.size >= FACET_REGISTRY.length ? NO_HIDDEN_FACETS : hidden;
}

/**
 * The facets to hide for this viewer: the ones an administrator turned off for
 * the instance and the ones the user hid. A user may not hide every facet, so
 * when the two together would cover them all only the instance's stand.
 */
export function combineHiddenFacets(
  userHidden: HiddenFacets,
  instanceDisabled: readonly string[] | null | undefined,
): HiddenFacets {
  const disabled = new Set<Facet>();
  for (const value of instanceDisabled ?? []) {
    const facet = resolveFacetId(value);
    if (facet) disabled.add(facet);
  }
  if (disabled.size === 0) return userHidden;
  if (disabled.size >= FACET_REGISTRY.length) return userHidden;
  const combined = new Set<Facet>([...userHidden, ...disabled]);
  return combined.size >= FACET_REGISTRY.length ? disabled : combined;
}

/** The save form of a hidden set, in registry order. */
export function hiddenFacetsToUiSettings(
  hidden: HiddenFacets,
): UiSettingsFacet[] {
  return FACET_REGISTRY.filter((facet) => hidden.has(facet.id)).map(
    (facet) => TO_UI_SETTINGS[facet.id],
  );
}

/**
 * Resolves any facet spelling the app stores or routes on: the domain enum
 * (`ANIME`), the UI settings enum (`MOVIES`), view ids (`movies`) and rule
 * values (`movie`).
 */
export function resolveFacetId(value: string | null | undefined): Facet | null {
  switch (value?.trim().toUpperCase()) {
    case "MOVIE":
    case "MOVIES":
      return "MOVIE";
    case "SERIES":
      return "SERIES";
    case "ANIME":
      return "ANIME";
    default:
      return null;
  }
}

/** Visible facets in registry order (Movies, Series, Anime). */
export function visibleFacets(hidden: HiddenFacets): FacetDefinition[] {
  return FACET_REGISTRY.filter((facet) => !hidden.has(facet.id));
}

/**
 * Whether a facet in any spelling is visible. A value that names no facet is
 * visible: hiding only ever removes what it can identify.
 */
export function isFacetVisible(
  value: string | null | undefined,
  hidden: HiddenFacets,
): boolean {
  const facet = resolveFacetId(value);
  return facet === null || !hidden.has(facet);
}

/** Keeps the items whose facet is visible; items with no facet are kept. */
export function filterVisibleFacets<T>(
  items: readonly T[],
  facetOf: (item: T) => string | null | undefined,
  hidden: HiddenFacets,
): T[] {
  if (hidden.size === 0) return [...items];
  return items.filter((item) => isFacetVisible(facetOf(item), hidden));
}

/**
 * A fixed list of facet values filtered to the visible ones, keeping values the
 * caller must still show (for example a facet a saved rule already selects, so
 * it stays visible and removable).
 */
export function visibleFacetValues<T extends string>(
  values: readonly T[],
  hidden: HiddenFacets,
  keep: readonly string[] = [],
): T[] {
  if (hidden.size === 0) return [...values];
  const kept = new Set(
    keep.map((value) => resolveFacetId(value)).filter((facet) => facet),
  );
  return values.filter((value) => {
    const facet = resolveFacetId(value);
    return facet === null || !hidden.has(facet) || kept.has(facet);
  });
}

/**
 * The visible facet to land on in place of `facet`: itself when visible,
 * otherwise the first visible facet in registry order.
 */
export function visibleFacetOr(facet: Facet, hidden: HiddenFacets): Facet {
  if (!hidden.has(facet)) return facet;
  return visibleFacets(hidden)[0]?.id ?? facet;
}
