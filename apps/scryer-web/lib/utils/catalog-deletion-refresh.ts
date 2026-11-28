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
 * The deletion runs the catalog follows, each with the titles it was queued
 * for. A run's outcome is `null` while it runs; once it ends, the outcome is
 * the titles it deleted.
 */
export type TitleDeletionRuns = ReadonlyMap<
  string,
  {
    titleIds: readonly string[];
    deletedTitleIds: readonly string[] | null;
  }
>;

/** Starts following a deletion run queued for these titles. */
export function withQueuedTitleDeletionRun(
  runs: TitleDeletionRuns,
  runId: string,
  titleIds: readonly string[],
): TitleDeletionRuns {
  const next = new Map(runs);
  const queued = next.get(runId);
  next.set(runId, {
    titleIds: [...new Set([...(queued?.titleIds ?? []), ...titleIds])],
    deletedTitleIds: queued?.deletedTitleIds ?? null,
  });
  return next;
}

/**
 * Records how a followed run ended. `failedTitleIds` is what
 * {@link titleDeletionFailedTitleIds} read from the run: the titles it left in
 * place, or `null` when it did not say, in which case none of that run's titles
 * is known to be gone. Other runs keep their own outcome.
 */
export function withSettledTitleDeletionRun(
  runs: TitleDeletionRuns,
  runId: string,
  failedTitleIds: readonly string[] | null,
): TitleDeletionRuns {
  const queued = runs.get(runId);
  if (!queued) {
    return runs;
  }
  const kept = new Set(failedTitleIds ?? queued.titleIds);
  const next = new Map(runs);
  next.set(runId, {
    titleIds: queued.titleIds,
    deletedTitleIds: queued.titleIds.filter((titleId) => !kept.has(titleId)),
  });
  return next;
}

/** Whether a followed run is still going. */
export function titleDeletionRunPending(
  runs: TitleDeletionRuns,
  runId: string,
): boolean {
  return runs.get(runId)?.deletedTitleIds === null;
}

/** Whether every followed run has ended. */
export function titleDeletionRunsSettled(runs: TitleDeletionRuns): boolean {
  for (const run of runs.values()) {
    if (run.deletedTitleIds === null) {
      return false;
    }
  }
  return true;
}

/**
 * The hidden titles that are gone once every followed run has ended: those a
 * run deleted, and those queued without a run to follow that no run reported
 * keeping. A title whose run failed or did not report leaves the set.
 */
export function deletedTitleIdsOfSettledRuns(
  runs: TitleDeletionRuns,
  pendingTitleIds: ReadonlySet<string>,
): Set<string> {
  const deleted = new Set<string>();
  const followed = new Set<string>();
  for (const run of runs.values()) {
    for (const titleId of run.titleIds) {
      followed.add(titleId);
    }
    for (const titleId of run.deletedTitleIds ?? []) {
      deleted.add(titleId);
    }
  }
  for (const titleId of pendingTitleIds) {
    if (!followed.has(titleId)) {
      deleted.add(titleId);
    }
  }
  return deleted;
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
