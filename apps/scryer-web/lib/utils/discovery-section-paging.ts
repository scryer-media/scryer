// Paging state for a discovery section grid.
//
// The server pages a section by offset over its full, stable order. The grid
// appends each page it receives, keeps the first copy of any title a later
// page repeats, and starts over from offset zero whenever the query that
// produced the pages (section, sort, filters, owned and requested toggles)
// changes. A page that answers an older query or an offset already passed is
// ignored, so a slow response cannot splice stale titles into a new grid.
// The server also names the discovery generations each page was composed
// from; when a later page names different ones, the section was rebuilt
// between pages and the grid starts over from offset zero.

import type {
  DiscoveryHomeFilters,
  DiscoverySectionSort,
} from "@/lib/types/discovery";

export const DISCOVERY_SECTION_PAGE_SIZE = 40;

export function discoveryBrowseSectionId(
  routeSectionId: string | null,
  filters: DiscoveryHomeFilters,
): string | null {
  return (
    routeSectionId ??
    (filters.genreTagKeys?.length || filters.themeTagKeys?.length
      ? "browse_all"
      : null)
  );
}

export type DiscoverySectionPageItem = {
  targetKind: string;
  targetKey: string;
};

export type DiscoverySectionQuery = {
  sectionId: string;
  sort: DiscoverySectionSort;
  filters: DiscoveryHomeFilters;
  hideOwned: boolean;
  hideRequested: boolean;
};

export type DiscoverySectionPaging<T extends DiscoverySectionPageItem> = {
  /** Identity of the query these pages answer. */
  queryKey: string;
  items: T[];
  /** Server offset of the next page. */
  offset: number;
  /** Section total under the current query; null before the first page. */
  totalCount: number | null;
  /** Generations the pages so far were composed from; null before the first. */
  generationIds: readonly string[] | null;
  exhausted: boolean;
};

function sortedList<T>(values: readonly T[] | null | undefined): T[] {
  return [...(values ?? [])].sort();
}

/** A stable key for everything that decides a section's membership and order. */
export function discoverySectionQueryKey(query: DiscoverySectionQuery): string {
  const filters = query.filters;
  return JSON.stringify([
    query.sectionId,
    query.sort,
    query.hideOwned,
    query.hideRequested,
    sortedList(filters.contentTypes),
    sortedList(filters.genreTagKeys),
    sortedList(filters.themeTagKeys),
    sortedList(filters.studioSlugs),
    filters.minimumYear ?? null,
    filters.maximumYear ?? null,
    filters.minimumRating ?? null,
  ]);
}

/** The dedupe key of a card, or null for a card without a target key. */
export function discoverySectionItemKey(
  item: DiscoverySectionPageItem,
): string | null {
  return item.targetKey ? `${item.targetKind}:${item.targetKey}` : null;
}

function sameGenerations(
  left: readonly string[],
  right: readonly string[],
): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

export function startDiscoverySectionPaging<T extends DiscoverySectionPageItem>(
  queryKey: string,
): DiscoverySectionPaging<T> {
  return {
    queryKey,
    items: [],
    offset: 0,
    totalCount: null,
    generationIds: null,
    exhausted: false,
  };
}

/**
 * The paging state for `queryKey`: the current state when it already answers
 * that query, and a fresh one at offset zero when the query changed.
 */
export function discoverySectionPagingFor<T extends DiscoverySectionPageItem>(
  state: DiscoverySectionPaging<T>,
  queryKey: string,
): DiscoverySectionPaging<T> {
  return state.queryKey === queryKey
    ? state
    : startDiscoverySectionPaging<T>(queryKey);
}

/** Offset and size of the next page to ask for, or null when none is left. */
export function nextDiscoverySectionPage(
  state: DiscoverySectionPaging<DiscoverySectionPageItem>,
  pageSize = DISCOVERY_SECTION_PAGE_SIZE,
): { offset: number; limit: number } | null {
  return state.exhausted ? null : { offset: state.offset, limit: pageSize };
}

/**
 * Appends one server page. `page.offset` is the offset the page was asked
 * for; a page for another query or another offset leaves the state as it is.
 * A page composed from other generations than the pages before it returns a
 * fresh state at offset zero for the same query, which the caller refetches.
 */
export function appendDiscoverySectionPage<T extends DiscoverySectionPageItem>(
  state: DiscoverySectionPaging<T>,
  page: {
    queryKey: string;
    offset: number;
    items: readonly T[];
    totalCount: number;
    generationIds: readonly string[];
  },
): DiscoverySectionPaging<T> {
  if (
    page.queryKey !== state.queryKey ||
    page.offset !== state.offset ||
    state.exhausted
  ) {
    return state;
  }
  if (
    state.generationIds !== null &&
    !sameGenerations(state.generationIds, page.generationIds)
  ) {
    return startDiscoverySectionPaging<T>(state.queryKey);
  }
  const seen = new Set<string>();
  for (const item of state.items) {
    const key = discoverySectionItemKey(item);
    if (key !== null) {
      seen.add(key);
    }
  }
  const added: T[] = [];
  for (const item of page.items) {
    const key = discoverySectionItemKey(item);
    if (key === null) {
      added.push(item);
    } else if (!seen.has(key)) {
      seen.add(key);
      added.push(item);
    }
  }
  const offset = state.offset + page.items.length;
  const totalCount = Math.max(0, page.totalCount);
  return {
    queryKey: state.queryKey,
    items: added.length > 0 ? [...state.items, ...added] : state.items,
    offset,
    totalCount,
    generationIds: [...page.generationIds],
    exhausted: page.items.length === 0 || offset >= totalCount,
  };
}
