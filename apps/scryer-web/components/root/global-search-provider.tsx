import { type ReactNode, useEffect } from "react";
import { GrabbedReleaseToastListener } from "@/components/root/grabbed-release-toast-listener";
import { useGlobalSearch } from "@/lib/hooks/use-global-search";
import type { Facet } from "@/lib/types";
import type { LocaleCode } from "@/lib/i18n";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { SearchContext } from "@/lib/context/search-context";
import type { AuthUser } from "@/lib/hooks/use-auth";

type GlobalSearchProviderProps = {
  activeFacet: Facet;
  queueFacet: Facet;
  uiLanguage: LocaleCode;
  authenticatedUser: AuthUser;
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
  children: ReactNode;
};

export function GlobalSearchProvider({
  activeFacet,
  queueFacet,
  uiLanguage,
  authenticatedUser,
  onOpenOverview,
  children,
}: GlobalSearchProviderProps) {
  const searchState = useGlobalSearch({
    authenticatedUser,
    queueFacet,
    uiLanguage,
  });

  const { setQueueFacet, setTvdbCandidates } = searchState;
  useEffect(() => {
    setQueueFacet(activeFacet);
    setTvdbCandidates([]);
  }, [activeFacet, setQueueFacet, setTvdbCandidates]);

  return (
    <SearchContext.Provider value={searchState}>
      <GrabbedReleaseToastListener onOpenOverview={onOpenOverview} />
      {children}
    </SearchContext.Provider>
  );
}
