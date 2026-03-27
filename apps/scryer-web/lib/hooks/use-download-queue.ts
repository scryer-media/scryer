import { useCallback, useEffect, useRef, useState } from "react";
import { useClient } from "urql";

import {
  downloadQueueQuery,
  downloadQueueSubscription,
} from "@/lib/graphql/queries";
import { wsClient } from "@/lib/graphql/ws-client";
import type { DownloadQueueItem } from "@/lib/types";
import { useGlobalStatus } from "@/lib/context/global-status-context";

type UseDownloadQueueArgs = {
  enabled: boolean;
  includeAllActivity: boolean;
  includeHistoryOnly: boolean;
};

export type UseDownloadQueueResult = {
  queueItems: DownloadQueueItem[];
  queueLoading: boolean;
  queueError: string | null;
  lastRefreshedAt: Date | null;
  refreshQueue: () => Promise<void>;
};

export function useDownloadQueue({
  enabled,
  includeAllActivity,
  includeHistoryOnly,
}: UseDownloadQueueArgs): UseDownloadQueueResult {
  const setGlobalStatus = useGlobalStatus();
  const client = useClient();
  const [queueItems, setQueueItems] = useState<DownloadQueueItem[]>([]);
  const [queueLoading, setQueueLoading] = useState(false);
  const [queueError, setQueueError] = useState<string | null>(null);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Track whether the initial HTTP query has completed so the WS subscription
  // doesn't race with it and overwrite the authoritative query data.
  const [initialFetchDone, setInitialFetchDone] = useState(false);
  const initialFetchDoneRef = useRef(false);
  // Keep ref in sync for use in refreshQueue without adding it as a dep
  initialFetchDoneRef.current = initialFetchDone;

  // --- WS subscription via graphql-ws ---
  // Deferred-cleanup pattern to survive React StrictMode's fake unmount/remount.
  // On cleanup, we delay the actual unsubscribe. If the effect re-runs within
  // the grace period (StrictMode re-mount), we cancel the teardown and keep
  // the existing subscription alive.
  //
  // The subscription is gated on `initialFetchDone` so the first broadcast
  // (which may carry stale/un-enriched data) cannot overwrite the HTTP query
  // result that the user is already looking at.
  const unsubRef = useRef<(() => void) | null>(null);
  const teardownTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!enabled || includeHistoryOnly || !initialFetchDone) {
      if (teardownTimer.current) {
        clearTimeout(teardownTimer.current);
        teardownTimer.current = null;
      }
      if (unsubRef.current) {
        unsubRef.current();
        unsubRef.current = null;
      }
      return;
    }

    // StrictMode re-run: cancel the pending teardown, subscription is still alive
    if (teardownTimer.current) {
      clearTimeout(teardownTimer.current);
      teardownTimer.current = null;
      return;
    }

    // Fresh subscription
    const unsubscribe = wsClient.subscribe(
      {
        query: downloadQueueSubscription,
        variables: { includeAllActivity, includeHistoryOnly },
      },
      {
        next(result) {
          const items = result.data?.downloadQueue as
            | DownloadQueueItem[]
            | undefined;
          if (items) {
            // Merge: the subscription only carries live jobs from the
            // download client — it does NOT include terminal/historical
            // items (completed, failed, import_pending). Preserve those
            // from the existing state so the table doesn't flash empty.
            setQueueItems((prev) => {
              if (!includeAllActivity) {
                return items;
              }

              const TERMINAL_STATES = new Set([
                "completed",
                "failed",
                "import_pending",
                "importpending",
              ]);
              const liveIds = new Set(
                items.map((i) => i.downloadClientItemId),
              );
              const kept = prev.filter(
                (p) =>
                  TERMINAL_STATES.has(p.state.toLowerCase()) &&
                  !liveIds.has(p.downloadClientItemId),
              );
              return [...items, ...kept];
            });
            setQueueError(null);
            setLastRefreshedAt(new Date());
            if (pollingRef.current) {
              clearInterval(pollingRef.current);
              pollingRef.current = null;
            }
          }
        },
        error(err) {
          console.error("[download-queue] subscription error:", err);
        },
        complete() {
          unsubRef.current = null;
        },
      },
    );

    unsubRef.current = unsubscribe;

    return () => {
      // Defer unsubscribe — if StrictMode re-runs within 200ms we cancel this
      teardownTimer.current = setTimeout(() => {
        teardownTimer.current = null;
        unsubscribe();
        unsubRef.current = null;
      }, 200);
    };
  }, [enabled, includeAllActivity, includeHistoryOnly, initialFetchDone]);

  // --- Query fetch (initial load + manual refresh) ---
  // The query is authoritative — it returns enriched data with import status,
  // submission linkage, and history items that the WS subscription doesn't carry.
  // To avoid wiping live socket data that may be fresher for active downloads,
  // we merge: query items win for terminal states (completed, failed,
  // import_pending) and the socket's version wins for active states if the
  // subscription is already running.
  const refreshQueue = useCallback(async () => {
    if (!enabled) {
      return;
    }
    setQueueLoading(true);
    try {
      const { data, error } = await client
        .query(downloadQueueQuery, {
          includeAllActivity,
          includeHistoryOnly,
        })
        .toPromise();
      if (error) throw error;
      const queryItems = data?.downloadQueue || [];
      // If the subscription isn't active yet (initial load), full replace.
      // Once the subscription is running, merge so we don't clobber live data.
      if (!initialFetchDoneRef.current) {
        setQueueItems(queryItems);
      } else {
        setQueueItems((prev) => {
          // Build a map of query items keyed by downloadClientItemId
          const queryMap = new Map(
            queryItems.map((i: DownloadQueueItem) => [
              i.downloadClientItemId,
              i,
            ]),
          );
          // Keep existing active items that the query didn't return
          // (subscription may have fresher live data)
          const ACTIVE_STATES = new Set([
            "downloading",
            "queued",
            "paused",
            "verifying",
            "repairing",
            "extracting",
          ]);
          const merged = [...queryItems];
          for (const item of prev) {
            if (
              ACTIVE_STATES.has(item.state.toLowerCase()) &&
              !queryMap.has(item.downloadClientItemId)
            ) {
              merged.push(item);
            }
          }
          return merged;
        });
      }
      setQueueError(null);
      setLastRefreshedAt(new Date());
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Failed to load queue.";
      setQueueError(message);
      setGlobalStatus(message);
    } finally {
      setQueueLoading(false);
    }
  }, [client, enabled, includeAllActivity, includeHistoryOnly, setGlobalStatus]);

  // --- Initial fetch + polling for history-only mode ---
  useEffect(() => {
    if (!enabled) {
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
      return;
    }

    setInitialFetchDone(false);
    refreshQueue().finally(() => setInitialFetchDone(true));

    if (includeHistoryOnly) {
      pollingRef.current = setInterval(() => void refreshQueue(), 10_000);
      return () => {
        if (pollingRef.current) {
          clearInterval(pollingRef.current);
          pollingRef.current = null;
        }
      };
    }
  }, [enabled, includeAllActivity, includeHistoryOnly, refreshQueue]);

  return {
    queueItems,
    queueLoading,
    queueError,
    lastRefreshedAt,
    refreshQueue,
  };
}
