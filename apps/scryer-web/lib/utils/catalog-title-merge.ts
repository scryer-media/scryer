import type { TitleRecord } from "@/lib/types";

/**
 * Fold a refreshed title record into the one the UI is already showing.
 *
 * Catalog list projections are much narrower than the side-panel selection,
 * and they narrow further still with the visible table columns and view mode.
 * GraphQL answers an unselected field with `undefined` and a cleared one with
 * `null`, so an absent field always means "not requested" and never "deleted":
 * carrying it over from `current` is what keeps a reactive list refresh from
 * blanking the selected title's panel -- most visibly the Watch-in control,
 * which unmounts the moment `playbackLinks` goes undefined.
 *
 * Artwork needs the stronger rule (a null poster is just as unusable as a
 * missing one), and `metadataFetchedAt` keeps the newest stamp either side has.
 */
export function mergePreferLoadedImageFields(
  current: TitleRecord,
  incoming: TitleRecord,
): TitleRecord {
  const incomingHasPoster = Boolean(
    incoming.posterUrl || incoming.posterSourceUrl,
  );
  const incomingHasBackground = Boolean(
    incoming.backgroundUrl || incoming.backgroundSourceUrl,
  );

  const merged = { ...current } as Record<string, unknown>;
  for (const [field, value] of Object.entries(incoming)) {
    if (value !== undefined) {
      merged[field] = value;
    }
  }

  return {
    ...(merged as TitleRecord),
    posterUrl: incomingHasPoster
      ? incoming.posterUrl
      : (current.posterUrl ?? null),
    posterSourceUrl: incomingHasPoster
      ? incoming.posterSourceUrl
      : (current.posterSourceUrl ?? null),
    backgroundUrl: incomingHasBackground
      ? incoming.backgroundUrl
      : (current.backgroundUrl ?? null),
    backgroundSourceUrl: incomingHasBackground
      ? incoming.backgroundSourceUrl
      : (current.backgroundSourceUrl ?? null),
    metadataFetchedAt: incoming.metadataFetchedAt ?? current.metadataFetchedAt,
  };
}

export type ActiveCatalogListFilters = {
  facet: TitleRecord["facet"];
  query: string;
  libraryIds: readonly string[];
};

export function buildActiveCatalogListFilters(
  facet: TitleRecord["facet"],
  query: string,
  libraryIds: readonly string[],
): ActiveCatalogListFilters {
  return {
    facet,
    query: query.trim().toLocaleLowerCase(),
    libraryIds: [...libraryIds],
  };
}

export function catalogTitleMatchesActiveListFilters(
  title: TitleRecord,
  filters: ActiveCatalogListFilters,
): boolean {
  if (title.facet !== filters.facet) {
    return false;
  }

  if (
    filters.libraryIds.length > 0 &&
    !filters.libraryIds.includes(title.libraryId)
  ) {
    return false;
  }

  return (
    filters.query.length === 0 ||
    title.name.toLocaleLowerCase().includes(filters.query)
  );
}

export function upsertCatalogTitleRecord(
  titles: TitleRecord[],
  title: TitleRecord,
  filters?: ActiveCatalogListFilters,
): TitleRecord[] {
  const existingIndex = titles.findIndex((item) => item.id === title.id);
  if (filters && !catalogTitleMatchesActiveListFilters(title, filters)) {
    if (existingIndex === -1) {
      return titles;
    }
    const next = [...titles];
    next.splice(existingIndex, 1);
    return next;
  }

  const next = [...titles];
  if (existingIndex === -1) {
    next.push(title);
  } else {
    next[existingIndex] = mergePreferLoadedImageFields(
      next[existingIndex],
      title,
    );
  }
  return next;
}

/**
 * Fold a reactively refreshed title into the loaded catalog list.
 *
 * A title that is not loaded is inserted only when it is new
 * (`insertIfMissing`): an update to a title on a page that has not been
 * loaded yet must not pull it into the list out of order.
 */
export function applyRefreshedCatalogTitle(
  titles: TitleRecord[],
  title: TitleRecord,
  filters: ActiveCatalogListFilters,
  insertIfMissing: boolean,
): TitleRecord[] {
  if (!insertIfMissing && !titles.some((item) => item.id === title.id)) {
    return titles;
  }
  return upsertCatalogTitleRecord(titles, title, filters);
}
