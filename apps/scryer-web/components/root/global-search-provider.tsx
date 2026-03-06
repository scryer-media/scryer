import { type ReactNode, useEffect } from "react";
import { useGlobalSearch } from "@/lib/hooks/use-global-search";
import type { Facet } from "@/lib/types";
import type { LocaleCode } from "@/lib/i18n";
import { SearchContext } from "@/lib/context/search-context";

type GlobalSearchProviderProps = {
  activeFacet: Facet;
  queueFacet: Facet;
  uiLanguage: LocaleCode;
  onCatalogChanged: () => void;
  children: ReactNode;
};

export function GlobalSearchProvider({
  activeFacet,
  queueFacet,
  uiLanguage,
  onCatalogChanged,
  children,
}: GlobalSearchProviderProps) {
  const searchState = useGlobalSearch({
    queueFacet,
    uiLanguage,
    onCatalogChanged,
  });

  useEffect(() => {
    searchState.setQueueFacet(activeFacet);
    searchState.setTvdbCandidates([]);
    searchState.setSearchResults([]);
    searchState.setSelectedTvdbId(null);
  }, [activeFacet, searchState.setQueueFacet, searchState.setTvdbCandidates, searchState.setSearchResults, searchState.setSelectedTvdbId]);

  return (
    <SearchContext.Provider value={searchState}>
      {children}
    </SearchContext.Provider>
  );
}
