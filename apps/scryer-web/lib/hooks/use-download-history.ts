import { useCallback, useEffect, useRef, useState } from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { downloadHistoryQuery } from "@/lib/graphql/queries";
import type { DownloadHistoryPage, DownloadQueueItem } from "@/lib/types";

const HISTORY_PAGE_SIZE = 50;

type UseDownloadHistoryArgs = {
  enabled: boolean;
};

export type UseDownloadHistoryResult = {
  historyItems: DownloadQueueItem[];
  historyLoading: boolean;
  historyLoadingMore: boolean;
  historyError: string | null;
  historyHasMore: boolean;
  lastRefreshedAt: Date | null;
  refreshHistory: () => Promise<void>;
  loadMoreHistory: () => Promise<void>;
};

function mergeHistoryItems(
  previousItems: DownloadQueueItem[],
  nextItems: DownloadQueueItem[],
): DownloadQueueItem[] {
  const seen = new Set(previousItems.map((item) => `${item.clientType}:${item.downloadClientItemId}`));
  const merged = [...previousItems];
  for (const item of nextItems) {
    const key = `${item.clientType}:${item.downloadClientItemId}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    merged.push(item);
  }
  return merged;
}

export function useDownloadHistory({
  enabled,
}: UseDownloadHistoryArgs): UseDownloadHistoryResult {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const [historyItems, setHistoryItems] = useState<DownloadQueueItem[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyLoadingMore, setHistoryLoadingMore] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [historyHasMore, setHistoryHasMore] = useState(false);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const historyItemCountRef = useRef(0);
  historyItemCountRef.current = historyItems.length;

  const fetchHistoryPage = useCallback(
    async (limit: number, offset: number): Promise<DownloadHistoryPage> => {
      const { data, error } = await client
        .query(downloadHistoryQuery, { limit, offset })
        .toPromise();
      if (error) {
        throw error;
      }
      return (
        data?.downloadHistory ?? {
          items: [],
          hasMore: false,
        }
      );
    },
    [client],
  );

  const refreshHistory = useCallback(async () => {
    if (!enabled) {
      return;
    }

    setHistoryLoading(true);
    try {
      const limit = Math.max(historyItemCountRef.current, HISTORY_PAGE_SIZE);
      const page = await fetchHistoryPage(limit, 0);
      setHistoryItems(page.items);
      setHistoryHasMore(page.hasMore);
      setHistoryError(null);
      setLastRefreshedAt(new Date());
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Failed to load activity history.";
      setHistoryError(message);
      setGlobalStatus(message);
    } finally {
      setHistoryLoading(false);
    }
  }, [enabled, fetchHistoryPage, setGlobalStatus]);

  const loadMoreHistory = useCallback(async () => {
    if (!enabled || historyLoadingMore || !historyHasMore) {
      return;
    }

    setHistoryLoadingMore(true);
    try {
      const page = await fetchHistoryPage(HISTORY_PAGE_SIZE, historyItems.length);
      setHistoryItems((current) => mergeHistoryItems(current, page.items));
      setHistoryHasMore(page.hasMore);
      setHistoryError(null);
      setLastRefreshedAt(new Date());
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Failed to load more activity history.";
      setHistoryError(message);
      setGlobalStatus(message);
    } finally {
      setHistoryLoadingMore(false);
    }
  }, [
    enabled,
    fetchHistoryPage,
    historyHasMore,
    historyItems.length,
    historyLoadingMore,
    setGlobalStatus,
  ]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    void refreshHistory();
  }, [enabled, refreshHistory]);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    const intervalId = setInterval(() => {
      void refreshHistory();
    }, 10_000);

    return () => clearInterval(intervalId);
  }, [enabled, refreshHistory]);

  return {
    historyItems,
    historyLoading,
    historyLoadingMore,
    historyError,
    historyHasMore,
    lastRefreshedAt,
    refreshHistory,
    loadMoreHistory,
  };
}
