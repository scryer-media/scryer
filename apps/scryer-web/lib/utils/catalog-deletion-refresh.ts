import {
  allOf,
  forEventTypes,
  forFacet,
  type ReactiveRefreshRegistration,
} from "../reactive/domain-event-feed.ts";

export type CatalogDeletionHandlers = {
  /** Committed deletions of these titles, so the list drops just those rows. */
  removed: (titleIds: string[]) => void;
  /**
   * The feed could not say what was deleted (degraded interval or a reconnect
   * without a cursor). The list keeps its rows rather than reloading in full.
   */
  unknown: () => void;
};

export function catalogDeletionRefresh(
  facet: string,
  handlers: CatalogDeletionHandlers,
): ReactiveRefreshRegistration {
  return {
    aliasKey: `catalog-deletion:${facet}`,
    predicate: allOf(forEventTypes("title_deleted"), forFacet(facet)),
    run: (events) => {
      if (events === null) {
        handlers.unknown();
        return;
      }
      const titleIds = [
        ...new Set(
          events
            .map((event) => event.titleId)
            .filter((titleId): titleId is string => Boolean(titleId)),
        ),
      ];
      if (titleIds.length > 0) {
        handlers.removed(titleIds);
      }
    },
  };
}

/**
 * Adds titles whose deletion was queued to the set the catalog hides while
 * the deletion runs. Returns the same set when every id is already in it.
 */
export function withPendingDeletedTitleIds(
  current: Set<string>,
  titleIds: readonly string[],
): Set<string> {
  if (titleIds.every((titleId) => current.has(titleId))) {
    return current;
  }
  const next = new Set(current);
  for (const titleId of titleIds) {
    next.add(titleId);
  }
  return next;
}

/**
 * The titles a finished deletion run left in place. An empty list means every
 * title was deleted. `null` means the run failed without listing which titles
 * it could not delete, so none of them is known to be gone.
 */
export function titleDeletionFailedTitleIds(run: {
  status: string;
  summaryJson: unknown;
}): string[] | null {
  if (run.status === "COMPLETED") {
    return [];
  }
  let summary: unknown = run.summaryJson;
  if (typeof summary === "string") {
    try {
      summary = JSON.parse(summary);
    } catch {
      return null;
    }
  }
  if (typeof summary !== "object" || summary === null) {
    return null;
  }
  const failures = (summary as { failures?: unknown }).failures;
  if (!Array.isArray(failures) || failures.length === 0) {
    return null;
  }
  const titleIds: string[] = [];
  for (const failure of failures) {
    const titleId =
      typeof failure === "object" && failure !== null
        ? (failure as { titleId?: unknown }).titleId
        : null;
    if (typeof titleId !== "string") {
      return null;
    }
    titleIds.push(titleId);
  }
  return titleIds;
}
