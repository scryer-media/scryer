import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { useClient } from "urql";

import {
  downloadQueueQuery,
  downloadQueueSubscription,
} from "@/lib/graphql/queries";
import type { DownloadQueueItem } from "@/lib/types";
import { GlobalStatusContext, type SetGlobalStatus } from "@/lib/context/global-status-context";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import {
  mergeAuthoritativeQueueItems,
  mergeLiveQueueItems,
  sortDownloadQueueItems,
} from "@/lib/utils/download-queue";
import type { DownloadActivityFilter } from "@/lib/types";

type UseDownloadQueueArgs = {
  enabled: boolean;
  includeAllActivity: boolean;
  includeHistoryOnly: boolean;
  includeImportActivity?: boolean;
  titleId?: string | null;
  activityFilter: DownloadActivityFilter;
  onErrorStatus?: SetGlobalStatus;
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
  includeImportActivity = false,
  titleId = null,
  activityFilter,
  onErrorStatus,
}: UseDownloadQueueArgs): UseDownloadQueueResult {
  const contextGlobalStatus = useContext(GlobalStatusContext);
  const client = useClient();
  const [queueItems, setQueueItems] = useState<DownloadQueueItem[]>([]);
  const [queueLoading, setQueueLoading] = useState(false);
  const [queueError, setQueueError] = useState<string | null>(null);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const scopeKey = `${includeAllActivity ? 1 : 0}:${includeHistoryOnly ? 1 : 0}:${includeImportActivity ? 1 : 0}:${titleId ?? "all"}:${activityFilter}`;
  const activeScopeKeyRef = useRef(scopeKey);
  const lastReportedErrorRef = useRef<string | null>(null);

  // Track whether the initial HTTP query has completed so the WS subscription
  // doesn't race with it and overwrite the authoritative query data.
  const [initialFetchDone, setInitialFetchDone] = useState(false);
  const initialFetchDoneRef = useRef(false);

  useEffect(() => {
    activeScopeKeyRef.current = scopeKey;
  }, [scopeKey]);

  // Keep ref in sync for use in refreshQueue without adding it as a dep
  useEffect(() => {
    initialFetchDoneRef.current = initialFetchDone;
  }, [initialFetchDone]);

  // --- WS subscription via graphql-ws ---
  // Deferred-cleanup pattern to survive React StrictMode's fake unmount/remount.
  // On cleanup, we delay the actual unsubscribe. If the effect re-runs within
  // the grace period (StrictMode re-mount), we cancel the teardown and keep
  // the existing subscription alive.
  //
  // The subscription is gated on `initialFetchDone` so the first broadcast
  // (which may carry stale/un-enriched data) cannot overwrite the HTTP query
  // result that the user is already looking at.
  useDeferredWsSubscription<{ data?: { downloadQueue?: DownloadQueueItem[] } }>({
    enabled: enabled && !includeHistoryOnly && initialFetchDone,
    requestKey: `downloadQueue:${scopeKey}`,
    request: {
      query: downloadQueueSubscription,
      variables: {
        includeAllActivity,
        includeHistoryOnly,
        includeImportActivity,
        titleId,
        activityFilter,
      },
    },
    onNext(result) {
      const items = result.data?.downloadQueue;
      if (!items) {
        return;
      }

      // The subscription payload is a full filtered snapshot. Merge same-key
      // items to retain enrichment-only fields, but let absence in the latest
      // snapshot remove stale rows immediately.
      setQueueItems((prev) => mergeLiveQueueItems(items, prev));
      setQueueError(null);
      lastReportedErrorRef.current = null;
      setLastRefreshedAt(new Date());
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
    },
    onError(err) {
      console.error("[download-queue] subscription error:", err);
    },
  });

  // --- Query fetch (initial load + manual refresh) ---
  // The query is authoritative for enrichment fields and history/import overlays.
  // Once the subscription is running, merge carefully so a stale query refresh
  // cannot downgrade an actively progressing item with the same identity key.
  const refreshQueue = useCallback(async () => {
    if (!enabled) {
      return;
    }
    const requestScopeKey = scopeKey;
    setQueueLoading(true);
    try {
      const { data, error } = await client
        .query(downloadQueueQuery, {
          includeAllActivity,
          includeHistoryOnly,
          includeImportActivity,
          titleId,
          activityFilter,
        })
        .toPromise();
      if (activeScopeKeyRef.current !== requestScopeKey) {
        return;
      }
      if (error) throw error;
      const queryItems = data?.downloadQueue || [];
      // If the subscription isn't active yet (initial load), full replace.
      // Once the subscription is running, merge so we don't clobber live data.
      if (!initialFetchDoneRef.current) {
        setQueueItems(sortDownloadQueueItems(queryItems));
      } else {
        setQueueItems((prev) => mergeAuthoritativeQueueItems(queryItems, prev));
      }
      setQueueError(null);
      lastReportedErrorRef.current = null;
      setLastRefreshedAt(new Date());
    } catch (error) {
      if (activeScopeKeyRef.current !== requestScopeKey) {
        return;
      }
      const message =
        error instanceof Error ? error.message : "Failed to load queue.";
      setQueueError(message);
      if (lastReportedErrorRef.current !== message) {
        lastReportedErrorRef.current = message;
        (onErrorStatus ?? contextGlobalStatus)?.(message, { level: "ERROR" });
      }
    } finally {
      if (activeScopeKeyRef.current === requestScopeKey) {
        setQueueLoading(false);
      }
    }
  }, [
    activityFilter,
    client,
    contextGlobalStatus,
    enabled,
    includeAllActivity,
    includeHistoryOnly,
    includeImportActivity,
    titleId,
    onErrorStatus,
    scopeKey,
  ]);

  // --- Initial fetch + polling for history-only mode ---
  useEffect(() => {
    lastReportedErrorRef.current = null;

    if (!enabled) {
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
      return;
    }

    initialFetchDoneRef.current = false;
    setInitialFetchDone(false);
    setQueueItems([]);
    const requestScopeKey = scopeKey;
    refreshQueue().finally(() => {
      if (activeScopeKeyRef.current === requestScopeKey) {
        setInitialFetchDone(true);
      }
    });

    if (includeHistoryOnly) {
      pollingRef.current = setInterval(() => void refreshQueue(), 10_000);
      return () => {
        if (pollingRef.current) {
          clearInterval(pollingRef.current);
          pollingRef.current = null;
        }
      };
    }
  }, [
    activityFilter,
    enabled,
    includeAllActivity,
    includeHistoryOnly,
    includeImportActivity,
    scopeKey,
    titleId,
    refreshQueue,
  ]);

  return {
    queueItems,
    queueLoading,
    queueError,
    lastRefreshedAt,
    refreshQueue,
  };
}
