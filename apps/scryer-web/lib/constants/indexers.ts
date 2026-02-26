import type { IndexerCategoryRoutingSettings } from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";

export const INDEXER_ROUTING_SETTINGS_KEY = "indexer.routing";
export const INDEXER_ROUTING_EMPTY: IndexerCategoryRoutingSettings = {
  categories: [],
  enabled: true,
  priority: 0,
};

/**
 * Standard Newznab categories per media type.
 * 2000 = Movies, 5000 = TV, 5070 = Anime (TV).
 */
const DEFAULT_CATEGORIES_BY_SCOPE: Record<ViewCategoryId, string[]> = {
  movie: ["2000"],
  series: ["5000"],
  anime: ["5070"],
};

export function getDefaultIndexerRouting(
  scope: ViewCategoryId,
): IndexerCategoryRoutingSettings {
  return {
    categories: DEFAULT_CATEGORIES_BY_SCOPE[scope],
    enabled: true,
    priority: 0,
  };
}
