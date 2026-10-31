// Mirrors COMPLETED_JOB_TTL_MINUTES in interactive_release_search.rs.
const RESULT_LIFETIME_MS = 5 * 60 * 1000;

export function indexerSearchExpiresAt(snapshot: {
  startedAt?: string;
  completedAt?: string | null;
}): number {
  // Partial results may outlive polling (cancel, disconnect). Using the start
  // time until completion is known expires conservatively in those cases.
  const timestamp = Date.parse(
    snapshot.completedAt ?? snapshot.startedAt ?? "",
  );
  return Number.isFinite(timestamp) ? timestamp + RESULT_LIFETIME_MS : 0;
}

export function earliestSearchExpiry(
  deadlines: ReadonlyMap<string, number>,
): number | null {
  return deadlines.size ? Math.min(...deadlines.values()) : null;
}

export type IndexerSearchExpiry = {
  /** Searches whose results the server no longer holds. */
  expired: ReadonlySet<string>;
  /** Deadlines of the searches that are still live. */
  deadlines: Map<string, number>;
  /** Row ownership with the expired searches' rows removed. */
  rowOwners: Map<string, string>;
  /** Whether the search still being polled is one of the expired ones. */
  activeExpired: boolean;
  /** Whether a row survives: rows of expired searches do not. */
  isRowLive: (rowKey: string) => boolean;
};

/**
 * Expire each search on its own deadline. The results table merges rows from
 * the first search and any "retry failed" searches, and every row records the
 * search it arrived on; only the rows of searches whose deadline has passed
 * go, so a retry still running keeps its rows until its own deadline. Null
 * when nothing has expired at `now`.
 */
export function expireIndexerSearches({
  deadlines,
  rowOwners,
  activeSearchId,
  now,
}: {
  deadlines: ReadonlyMap<string, number>;
  rowOwners: ReadonlyMap<string, string>;
  activeSearchId: string | null;
  now: number;
}): IndexerSearchExpiry | null {
  const expired = new Set<string>();
  const remainingDeadlines = new Map<string, number>();
  for (const [searchId, deadline] of deadlines) {
    if (now >= deadline) expired.add(searchId);
    else remainingDeadlines.set(searchId, deadline);
  }
  if (expired.size === 0) return null;
  const remainingOwners = new Map<string, string>();
  for (const [rowKey, searchId] of rowOwners) {
    if (!expired.has(searchId)) remainingOwners.set(rowKey, searchId);
  }
  return {
    expired,
    deadlines: remainingDeadlines,
    rowOwners: remainingOwners,
    activeExpired: activeSearchId !== null && expired.has(activeSearchId),
    isRowLive: (rowKey) => {
      const owner = rowOwners.get(rowKey);
      return owner === undefined || !expired.has(owner);
    },
  };
}
