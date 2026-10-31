export const TITLE_OVERVIEW_BULK_REFRESH_DEBOUNCE_MS = 1500;
export const TITLE_OVERVIEW_BULK_REFRESH_MAX_WAIT_MS = 10000;

export const TITLE_OVERVIEW_HYDRATION_STARTED_KIND =
  "metadata_hydration_started";
export const TITLE_OVERVIEW_HYDRATION_COMPLETED_KIND =
  "metadata_hydration_completed";
export const TITLE_OVERVIEW_HYDRATION_FAILED_KIND = "metadata_hydration_failed";
export const TITLE_OVERVIEW_FILE_ANALYZED_KIND = "file_analyzed";
export const TITLE_OVERVIEW_SUBTITLE_DOWNLOADED_KIND = "subtitle_downloaded";

/** Activity that changes what a title overview shows about its downloads. */
export const TITLE_OVERVIEW_IMPORT_REFRESH_KINDS: ReadonlySet<string> = new Set(
  [
    "movie_downloaded",
    "series_episode_imported",
    "file_upgraded",
    "import_rejected",
  ],
);

export type TitleOverviewReactiveRefreshPlan =
  | { type: "none" }
  | { type: "hydrationStarted" }
  | { type: "hydrationCompleted" }
  | { type: "hydrationFailed" }
  | {
      type: "refresh";
      mode: "immediate" | "bulk";
    };

export function shouldHandleTitleOverviewActivity(
  currentTitleId?: string | null,
  activityTitleId?: string | null,
) {
  return Boolean(
    currentTitleId && activityTitleId && activityTitleId === currentTitleId,
  );
}

export function titleOverviewReactiveRefreshKinds(
  importKinds: ReadonlySet<string>,
) {
  return new Set([
    ...importKinds,
    TITLE_OVERVIEW_FILE_ANALYZED_KIND,
    TITLE_OVERVIEW_SUBTITLE_DOWNLOADED_KIND,
    TITLE_OVERVIEW_HYDRATION_STARTED_KIND,
    TITLE_OVERVIEW_HYDRATION_COMPLETED_KIND,
    TITLE_OVERVIEW_HYDRATION_FAILED_KIND,
  ]);
}

/**
 * Split the collections a refresh asks for into the ones to fetch now and the
 * ones whose own load is still in flight. A collection loading right now cannot
 * be fetched again - the second query would race the first - but its in-flight
 * answer predates the event that asked for the refresh, so the request has to
 * survive until that load resolves rather than being dropped.
 */
export function planCollectionEpisodeRefresh(
  requestedCollectionIds: readonly string[],
  inFlightCollectionIds: ReadonlySet<string>,
): { refreshNow: string[]; deferred: string[] } {
  const refreshNow: string[] = [];
  const deferred: string[] = [];
  for (const collectionId of requestedCollectionIds) {
    if (inFlightCollectionIds.has(collectionId)) {
      deferred.push(collectionId);
    } else {
      refreshNow.push(collectionId);
    }
  }
  return { refreshNow, deferred };
}

/**
 * Answer, as a load for `completedCollectionId` resolves, which deferred
 * refreshes that release and what stays pending.
 */
export function drainDeferredCollectionEpisodeRefresh(
  deferredCollectionIds: ReadonlySet<string>,
  completedCollectionId: string,
): { rerun: string[]; remaining: Set<string> } {
  const remaining = new Set(deferredCollectionIds);
  const rerun = remaining.delete(completedCollectionId)
    ? [completedCollectionId]
    : [];
  return { rerun, remaining };
}

export function titleOverviewReactiveRefreshPlan(
  activityKind: string,
  importKinds: ReadonlySet<string>,
): TitleOverviewReactiveRefreshPlan {
  switch (activityKind) {
    case TITLE_OVERVIEW_HYDRATION_STARTED_KIND:
      return { type: "hydrationStarted" };
    case TITLE_OVERVIEW_HYDRATION_COMPLETED_KIND:
      return { type: "hydrationCompleted" };
    case TITLE_OVERVIEW_HYDRATION_FAILED_KIND:
      return { type: "hydrationFailed" };
    case TITLE_OVERVIEW_FILE_ANALYZED_KIND:
      return { type: "refresh", mode: "bulk" };
    case TITLE_OVERVIEW_SUBTITLE_DOWNLOADED_KIND:
      return { type: "refresh", mode: "immediate" };
    default:
      if (importKinds.has(activityKind)) {
        return { type: "refresh", mode: "immediate" };
      }
      return { type: "none" };
  }
}
