import type { Facet } from "../types/index.ts";
import {
  EMPTY_TITLE_ADVANCED_FILTERS,
  EMPTY_TITLE_QUICK_FILTERS,
  type TitleCatalogAdvancedFilters,
  type TitleCatalogQuickFilters,
} from "./title-catalog-query.ts";

export type CatalogFilterState = {
  query: string;
  libraryIds: string[];
  quick: TitleCatalogQuickFilters;
  advanced: TitleCatalogAdvancedFilters;
};

export const CATALOG_FILTER_FACETS: Facet[] = ["MOVIE", "SERIES", "ANIME"];
const STORAGE_PREFIX = "scryer:catalog-filters:v1:";

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function strings(value: unknown): string[] {
  return Array.isArray(value)
    ? [
        ...new Set(
          value.filter(
            (entry): entry is string =>
              typeof entry === "string" && entry.length > 0,
          ),
        ),
      ]
    : [];
}

function numberInRange(
  value: unknown,
  maximum: number,
  integer = false,
): number | null {
  return typeof value === "number" &&
    Number.isFinite(value) &&
    value >= 0 &&
    value <= maximum &&
    (!integer || Number.isInteger(value))
    ? value
    : null;
}

export function normalizeCatalogFilters(value: unknown): CatalogFilterState {
  const state = record(value);
  const quick = record(state.quick);
  const advanced = record(state.advanced);
  return {
    query: typeof state.query === "string" ? state.query : "",
    libraryIds: strings(state.libraryIds),
    quick: {
      monitored: quick.monitored === true,
      unmonitored: quick.unmonitored === true,
      continuing: quick.continuing === true,
      ended: quick.ended === true,
    },
    advanced: {
      rootFolderIds: strings(advanced.rootFolderIds),
      genreTagKeys: strings(advanced.genreTagKeys),
      themeTagKeys: strings(advanced.themeTagKeys),
      userTagLabels: strings(advanced.userTagLabels),
      minimumYear: numberInRange(advanced.minimumYear, 9999, true),
      maximumYear: numberInRange(advanced.maximumYear, 9999, true),
      minimumRating: numberInRange(advanced.minimumRating, 10),
      presences: strings(advanced.presences).filter(
        (value): value is TitleCatalogAdvancedFilters["presences"][number] =>
          value === "MISSING" || value === "PARTIAL" || value === "COMPLETE",
      ),
      needsAttention: advanced.needsAttention === true,
    },
  };
}

export function emptyCatalogFilters(): CatalogFilterState {
  return normalizeCatalogFilters({
    quick: EMPTY_TITLE_QUICK_FILTERS,
    advanced: EMPTY_TITLE_ADVANCED_FILTERS,
  });
}

export function readCatalogFilters(facet: Facet): CatalogFilterState {
  try {
    if (typeof window !== "undefined") {
      return normalizeCatalogFilters(
        JSON.parse(
          window.localStorage.getItem(STORAGE_PREFIX + facet) ?? "null",
        ),
      );
    }
  } catch {
    // Unavailable or stale browser storage must not prevent catalog access.
  }
  return emptyCatalogFilters();
}

export function writeCatalogFilters(
  facet: Facet,
  state: CatalogFilterState,
): void {
  if (typeof window === "undefined") return;
  try {
    const serialized = JSON.stringify(normalizeCatalogFilters(state));
    if (serialized === JSON.stringify(emptyCatalogFilters())) {
      window.localStorage.removeItem(STORAGE_PREFIX + facet);
    } else {
      window.localStorage.setItem(STORAGE_PREFIX + facet, serialized);
    }
  } catch {
    // Keep filters usable in memory when browser storage is unavailable.
  }
}
