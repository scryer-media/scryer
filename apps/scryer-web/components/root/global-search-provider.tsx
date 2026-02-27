import { type ReactNode } from "react";
import { useGlobalSearch } from "@/lib/hooks/use-global-search";
import type { UseGlobalSearchResult } from "@/lib/hooks/use-global-search";
import type { Facet } from "@/lib/types";
import type { LocaleCode } from "@/lib/i18n";

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

type GlobalSearchProviderProps = {
  t: Translate;
  setGlobalStatus: (status: string) => void;
  queueFacet: Facet;
  uiLanguage: LocaleCode;
  onCatalogChanged: () => void;
  children: (searchState: UseGlobalSearchResult) => ReactNode;
};

export function GlobalSearchProvider({
  t,
  setGlobalStatus,
  queueFacet,
  uiLanguage,
  onCatalogChanged,
  children,
}: GlobalSearchProviderProps) {
  const searchState = useGlobalSearch({
    t,
    setGlobalStatus,
    queueFacet,
    uiLanguage,
    onCatalogChanged,
  });

  return <>{children(searchState)}</>;
}
