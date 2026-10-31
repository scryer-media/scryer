import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type SetStateAction,
} from "react";
import type { Facet } from "../types/index.ts";
import {
  CATALOG_FILTER_FACETS,
  emptyCatalogFilters,
  readCatalogFilters,
  writeCatalogFilters,
  type CatalogFilterState,
} from "../utils/catalog-filter-persistence.ts";

function readFiltersByFacet(): Record<Facet, CatalogFilterState> {
  return {
    MOVIE: readCatalogFilters("MOVIE"),
    SERIES: readCatalogFilters("SERIES"),
    ANIME: readCatalogFilters("ANIME"),
  };
}

export function useCatalogFilterState(activeFacet: Facet) {
  const [byFacet, setByFacet] = useState(readFiltersByFacet);
  const [debouncedByFacet, setDebouncedByFacet] = useState(byFacet);
  const saved = useRef(byFacet);
  const latest = useRef(byFacet);
  const persist = useCallback(() => {
    for (const facet of CATALOG_FILTER_FACETS) {
      if (saved.current[facet] !== latest.current[facet]) {
        writeCatalogFilters(facet, latest.current[facet]);
      }
    }
    saved.current = latest.current;
  }, []);
  useEffect(() => {
    latest.current = byFacet;
    const timer = window.setTimeout(persist, 250);
    return () => window.clearTimeout(timer);
  }, [byFacet, persist]);
  useEffect(() => {
    window.addEventListener("pagehide", persist);
    return () => {
      window.removeEventListener("pagehide", persist);
      persist();
    };
  }, [persist]);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedByFacet(byFacet), 250);
    return () => window.clearTimeout(timer);
  }, [byFacet]);

  const update = useCallback(
    <K extends keyof CatalogFilterState>(
      key: K,
      action: SetStateAction<CatalogFilterState[K]>,
    ) => {
      setByFacet((current) => {
        const previous = current[activeFacet][key];
        const next = typeof action === "function" ? action(previous) : action;
        return Object.is(previous, next)
          ? current
          : {
              ...current,
              [activeFacet]: { ...current[activeFacet], [key]: next },
            };
      });
    },
    [activeFacet],
  );
  const setTitleFilter = useCallback(
    (next: SetStateAction<string>) => update("query", next),
    [update],
  );
  const setSelectedLibraryIds = useCallback(
    (next: SetStateAction<string[]>) => update("libraryIds", next),
    [update],
  );
  const setTitleQuickFilters = useCallback(
    (next: SetStateAction<CatalogFilterState["quick"]>) =>
      update("quick", next),
    [update],
  );
  const setAdvancedTitleFilters = useCallback(
    (next: SetStateAction<CatalogFilterState["advanced"]>) =>
      update("advanced", next),
    [update],
  );
  const clearFilters = useCallback(() => {
    const empty = emptyCatalogFilters();
    setByFacet((current) => ({ ...current, [activeFacet]: empty }));
    setDebouncedByFacet((current) => ({ ...current, [activeFacet]: empty }));
  }, [activeFacet]);
  const current = byFacet[activeFacet];
  const debounced = debouncedByFacet[activeFacet];
  const effectiveAdvancedTitleFilters = useMemo(
    () => ({
      ...current.advanced,
      minimumYear: debounced.advanced.minimumYear,
      maximumYear: debounced.advanced.maximumYear,
      minimumRating: debounced.advanced.minimumRating,
    }),
    [current.advanced, debounced.advanced],
  );
  return {
    titleFilter: current.query,
    setTitleFilter,
    debouncedTitleFilter: debounced.query.trim(),
    selectedLibraryIds: current.libraryIds,
    setSelectedLibraryIds,
    titleQuickFilters: current.quick,
    setTitleQuickFilters,
    advancedTitleFilters: current.advanced,
    setAdvancedTitleFilters,
    effectiveAdvancedTitleFilters,
    clearFilters,
  };
}
