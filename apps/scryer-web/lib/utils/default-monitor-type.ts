import type { MetadataCatalogMonitorType } from "@/lib/hooks/use-global-search";

/** Monitoring choices a movie library may save as its default. */
export const MOVIE_DEFAULT_MONITOR_TYPES: readonly MetadataCatalogMonitorType[] =
  ["MONITORED", "UNMONITORED"];

/**
 * Monitoring choices a series or anime library may save as its default.
 * ADVANCED is a per-title selection, never a saved default.
 */
export const EPISODIC_DEFAULT_MONITOR_TYPES: readonly MetadataCatalogMonitorType[] =
  ["FUTURE_EPISODES", "MISSING_AND_FUTURE_EPISODES", "ALL_EPISODES", "NONE"];

/** Translation key for each monitoring choice's label. */
export const MONITOR_TYPE_LABEL_KEYS: Record<MetadataCatalogMonitorType, string> =
  {
    MONITORED: "search.monitorType.monitored",
    UNMONITORED: "search.monitorType.unmonitored",
    FUTURE_EPISODES: "search.monitorType.futureEpisodes",
    MISSING_AND_FUTURE_EPISODES: "search.monitorType.missingAndFutureEpisodes",
    ALL_EPISODES: "search.monitorType.allEpisodes",
    NONE: "search.monitorType.none",
    ADVANCED: "search.monitorType.advanced",
  };

export function defaultMonitorTypesForFacet(
  facet: string,
): readonly MetadataCatalogMonitorType[] {
  return facet === "MOVIE"
    ? MOVIE_DEFAULT_MONITOR_TYPES
    : EPISODIC_DEFAULT_MONITOR_TYPES;
}

export function resolveDefaultMonitorType(
  facet: string,
  saved: string | null | undefined,
  fallback: MetadataCatalogMonitorType,
): MetadataCatalogMonitorType {
  return (
    defaultMonitorTypesForFacet(facet).find((value) => value === saved) ??
    fallback
  );
}
