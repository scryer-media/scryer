import type { Facet } from "@/lib/types";

export const CATALOG_TITLES_REFRESH_EVENT = "scryer:catalogTitlesRefresh";

export type CatalogTitlesRefreshDetail = {
  /** Facet the title landed in, so lists scoped elsewhere can skip the reload. */
  facet?: Facet | null;
  titleId?: string | null;
};

/**
 * Announces that the catalog gained a title. Adding from global search leaves
 * the panel open over whatever view is mounted, so that view has to hear about
 * the new title rather than relying on a navigation to remount it.
 */
export function dispatchCatalogTitlesRefresh(
  detail?: CatalogTitlesRefreshDetail,
) {
  if (typeof window === "undefined") {
    return;
  }

  window.dispatchEvent(
    new CustomEvent(
      CATALOG_TITLES_REFRESH_EVENT,
      detail ? { detail } : undefined,
    ),
  );
}

export function catalogTitlesRefreshDetail(
  event: Event,
): CatalogTitlesRefreshDetail {
  return event instanceof CustomEvent && event.detail
    ? (event.detail as CatalogTitlesRefreshDetail)
    : {};
}
