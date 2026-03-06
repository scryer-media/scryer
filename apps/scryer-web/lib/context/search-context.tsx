import { createContext, useContext } from "react";
import type { UseGlobalSearchResult } from "@/lib/hooks/use-global-search";

export const SearchContext = createContext<UseGlobalSearchResult | null>(null);

export function useSearchContext(): UseGlobalSearchResult {
  const ctx = useContext(SearchContext);
  if (!ctx) throw new Error("useSearchContext must be used within SearchProvider");
  return ctx;
}
