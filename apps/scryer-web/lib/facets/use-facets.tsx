import * as React from "react";

import { useUiSettings } from "@/lib/context/ui-settings-context";
import type { FacetDefinition } from "@/lib/facets/registry";
import {
  filterVisibleFacets,
  hiddenFacetsFromUiSettings,
  isFacetVisible,
  visibleFacetOr,
  visibleFacets,
  visibleFacetValues,
  type HiddenFacets,
} from "@/lib/facets/visibility";
import type { Facet } from "@/lib/types/titles";
import type { CatalogDiscoveryItem } from "@/lib/types";
import { discoveryItemFacet } from "@/lib/utils/discovery-actions";

export type FacetVisibility = {
  /** Visible facets in registry order. Enumerate these, never the registry. */
  facets: FacetDefinition[];
  /** Domain ids of the visible facets (`MOVIE`, `SERIES`, `ANIME`). */
  ids: Facet[];
  /** View ids of the visible facets (`movies`, `series`, `anime`). */
  viewIds: string[];
  hidden: HiddenFacets;
  /** Whether a facet in any spelling is visible; unknown values are. */
  isVisible: (value: string | null | undefined) => boolean;
  /** Keeps items whose facet is visible; items with no facet are kept. */
  filter: <T>(
    items: readonly T[],
    facetOf: (item: T) => string | null | undefined,
  ) => T[];
  /** Filters fixed facet values, keeping any listed in `keep`. */
  values: <T extends string>(
    values: readonly T[],
    keep?: readonly string[],
  ) => T[];
  /** `facet` when visible, else the first visible facet. */
  visibleOr: (facet: Facet) => Facet;
};

export function facetVisibility(hidden: HiddenFacets): FacetVisibility {
  const facets = visibleFacets(hidden);
  return {
    facets,
    ids: facets.map((facet) => facet.id),
    viewIds: facets.map((facet) => facet.viewId),
    hidden,
    isVisible: (value) => isFacetVisible(value, hidden),
    filter: (items, facetOf) => filterVisibleFacets(items, facetOf, hidden),
    values: (values, keep) => visibleFacetValues(values, hidden, keep),
    visibleOr: (facet) => visibleFacetOr(facet, hidden),
  };
}

/**
 * The one way UI code learns which facets to offer. Any list, tab, chip,
 * picker, nav entry or search section that enumerates facets reads it here so
 * the user's hidden facets apply everywhere at once.
 */
export function useFacets(): FacetVisibility {
  const { hiddenFacets } = useUiSettings().uiSettings;
  const key = hiddenFacets.join(",");
  return React.useMemo(
    () =>
      facetVisibility(hiddenFacetsFromUiSettings(key ? key.split(",") : [])),
    [key],
  );
}

/** Renders `children` once per visible facet, in registry order. */
export function ForEachFacet({
  children,
}: {
  children: (facet: FacetDefinition, index: number) => React.ReactNode;
}) {
  const { facets } = useFacets();
  return <>{facets.map((facet, index) => children(facet, index))}</>;
}

/** Renders `children` only while `facet` (any spelling) is visible. */
export function FacetGate({
  facet,
  children,
  fallback = null,
}: {
  facet: string | null | undefined;
  children: React.ReactNode;
  fallback?: React.ReactNode;
}) {
  const { isVisible } = useFacets();
  return <>{isVisible(facet) ? children : fallback}</>;
}

/**
 * `discoveryItemFacet` bound to the user's hidden facets, so an anime film is
 * offered as a Movie while Anime is hidden. Use it for every discovery or
 * recommendation item instead of reading `contentType` directly.
 */
export function useDiscoveryItemFacet(): (
  item: Pick<CatalogDiscoveryItem, "contentType" | "targetKind">,
) => Facet | null {
  const { hidden } = useFacets();
  return React.useCallback(
    (item) => discoveryItemFacet(item, hidden),
    [hidden],
  );
}
