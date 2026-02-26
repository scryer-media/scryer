import { useCallback, useEffect, useRef, useState } from "react";
import { useClient } from "urql";

import {
  downloadQueueQuery,
  downloadQueueSubscription,
} from "@/lib/graphql/queries";
import { wsClient } from "@/lib/graphql/ws-client";
import type { DownloadQueueItem } from "@/lib/types";

type UseDownloadQueueArgs = {
  setGlobalStatus: (status: string) => void;
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
  setGlobalStatus,
  includeAllActivity,
  includeHistoryOnly,
}: UseDownloadQueueArgs): UseDownloadQueueResult {
  const client = useClient();
  const [queueItems, setQueueItems] = useState<DownloadQueueItem[]>([]);
  const [queueLoading, setQueueLoading] = useState(false);
  const [queueError, setQueueError] = useState<string | null>(null);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // --- WS subscription via graphql-ws ---
  // Deferred-cleanup pattern to survive React StrictMode's fake unmount/remount.
  // On cleanup, we delay the actual unsubscribe. If the effect re-runs within
  // the grace period (StrictMode re-mount), we cancel the teardown and keep
  // the existing subscription alive.
  const unsubRef = useRef<(() => void) | null>(null);
  const teardownTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (includeHistoryOnly) {
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
            setQueueItems(items);
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
  }, [includeAllActivity, includeHistoryOnly]);

  // --- Query fetch (initial load + manual refresh) ---
  const refreshQueue = useCallback(async () => {
    setQueueLoading(true);
    try {
      const { data, error } = await client
        .query(downloadQueueQuery, {
          includeAllActivity,
          includeHistoryOnly,
        })
        .toPromise();
      if (error) throw error;
      setQueueItems(data?.downloadQueue || []);
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
  }, [client, includeAllActivity, includeHistoryOnly, setGlobalStatus]);

  // --- Polling for history-only mode (no subscription) ---
  useEffect(() => {
    void refreshQueue();

    if (includeHistoryOnly) {
      pollingRef.current = setInterval(() => void refreshQueue(), 10_000);
      return () => {
        if (pollingRef.current) {
          clearInterval(pollingRef.current);
          pollingRef.current = null;
        }
      };
    }
  }, [includeAllActivity, includeHistoryOnly, refreshQueue]);

  return {
    queueItems,
    queueLoading,
    queueError,
    lastRefreshedAt,
    refreshQueue,
  };
}
