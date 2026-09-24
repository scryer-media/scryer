import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { downloadImportQuery } from "@/lib/graphql/queries";
import type {
  DownloadImportFilter,
  DownloadImportPage,
  DownloadQueueItem,
} from "@/lib/types";
import { downloadQueueItemIdentityKey } from "@/lib/utils/download-queue";
import { boundedQuery, QueryTimeoutError } from "@/lib/graphql/bounded-query";
import { useQueryAuthScope } from "@/lib/hooks/use-query-auth-scope";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { useTranslate } from "@/lib/context/translate-context";

const IMPORT_PAGE_SIZE = 50;

type UseDownloadImportArgs = {
  enabled: boolean;
  filter: DownloadImportFilter;
};

export type UseDownloadImportResult = {
  importItems: DownloadQueueItem[];
  importLoading: boolean;
  importLoadingMore: boolean;
  importError: string | null;
  importHasMore: boolean;
  importTotalCount: number;
  lastRefreshedAt: Date | null;
  refreshImport: () => Promise<void>;
  loadMoreImport: () => Promise<void>;
};

function mergeImportItems(
  previousItems: DownloadQueueItem[],
  nextItems: DownloadQueueItem[],
): DownloadQueueItem[] {
  const seen = new Set(
    previousItems.map(downloadQueueItemIdentityKey),
  );
  const merged = [...previousItems];
  for (const item of nextItems) {
    const key = downloadQueueItemIdentityKey(item);
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    merged.push(item);
  }
  return merged;
}

export function useDownloadImport({
  enabled,
  filter,
}: UseDownloadImportArgs): UseDownloadImportResult {
  const client = useClient();
  const authScope = useQueryAuthScope();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [importItems, setImportItems] = useState<DownloadQueueItem[]>([]);
  const [importLoading, setImportLoading] = useState(false);
  const [importLoadingMore, setImportLoadingMore] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const [importHasMore, setImportHasMore] = useState(false);
  const [importTotalCount, setImportTotalCount] = useState(0);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const nextOffsetRef = useRef(0);
  const hasMoreRef = useRef(false);
  const activeRequestRef = useRef<AbortController | null>(null);
  const pendingRefreshRef = useRef(false);
  const epochRef = useRef(0);
  const [loadedScope, setLoadedScope] = useState<{ filter: DownloadImportFilter; auth: string | null } | null>(null);
  const sameScope = loadedScope?.filter === filter && loadedScope.auth === authScope;

  const runRequest = useCallback(async (loadMore: boolean) => {
    if (!enabled) return;
    if (activeRequestRef.current) {
      if (!loadMore) pendingRefreshRef.current = true;
      return;
    }
    if (loadMore && !hasMoreRef.current) return;
    const epoch = epochRef.current;
    const current = () => epoch === epochRef.current && authScope === getAuthToken();
    // One queued refresh is consumed after a successful request. A failure waits
    // for the regular poll or an explicit retry, avoiding immediate retry loops.
    let more = loadMore;
    do {
      pendingRefreshRef.current = false;
      const controller = new AbortController();
      activeRequestRef.current = controller;
      if (more) setImportLoadingMore(true);
      else setImportLoading(true);
      let succeeded = false;
      try {
        const offset = more ? nextOffsetRef.current : 0;
        const limit = more ? IMPORT_PAGE_SIZE : Math.max(nextOffsetRef.current, IMPORT_PAGE_SIZE);
        const { data, error } = await boundedQuery(
          client.query<{ downloadImport: DownloadImportPage }>(downloadImportQuery, { limit, offset, filter }),
          controller.signal,
        );
        if (!current()) return;
        if (error) throw error;
        const page = data?.downloadImport;
        if (!page || !Array.isArray(page.items) || typeof page.hasMore !== "boolean" ||
            !Number.isSafeInteger(page.totalCount) || page.totalCount < 0) {
          throw new Error("Invalid import activity response.");
        }
        const append = more;
        setImportItems((items) => mergeImportItems(append ? items : [], page.items));
        nextOffsetRef.current = offset + page.items.length;
        hasMoreRef.current = page.hasMore;
        setImportHasMore(page.hasMore);
        setImportTotalCount(page.totalCount);
        setLoadedScope({ filter, auth: authScope });
        setImportError(null);
        setLastRefreshedAt(new Date());
        succeeded = true;
      } catch (error) {
        if (!current()) return;
        const message = error instanceof QueryTimeoutError ? t("activity.refreshTimedOut") :
          error instanceof Error ? error.message : "Failed to load import activity.";
        setImportError(message);
        setGlobalStatus(message);
      } finally {
        if (current()) {
          activeRequestRef.current = null;
          setImportLoading(false);
          setImportLoadingMore(false);
        }
      }
      if (!succeeded || !current()) break;
      more = false;
    } while (pendingRefreshRef.current);
  }, [authScope, client, enabled, filter, setGlobalStatus, t]);

  const refreshImport = useCallback(() => runRequest(false), [runRequest]);
  const loadMoreImport = useCallback(() => runRequest(true), [runRequest]);

  useLayoutEffect(() => {
    epochRef.current += 1;
    nextOffsetRef.current = 0;
    hasMoreRef.current = false;
    pendingRefreshRef.current = false;
    setLoadedScope({ filter, auth: authScope });
    setImportItems([]);
    setImportHasMore(false);
    setImportTotalCount(0);
    setLastRefreshedAt(null);
    setImportError(null);
    setImportLoading(false);
    setImportLoadingMore(false);
    if (!enabled) {
      return;
    }
    void refreshImport();
    return () => {
      epochRef.current += 1;
      activeRequestRef.current?.abort();
      activeRequestRef.current = null;
      pendingRefreshRef.current = false;
    };
  }, [authScope, enabled, filter, refreshImport]);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    const intervalId = setInterval(() => {
      void refreshImport();
    }, 10_000);

    return () => clearInterval(intervalId);
  }, [enabled, refreshImport]);

  return {
    importItems: sameScope ? importItems : [],
    importLoading: enabled && (!sameScope || importLoading),
    importLoadingMore: sameScope && importLoadingMore,
    importError: sameScope ? importError : null,
    importHasMore: sameScope && importHasMore,
    importTotalCount: sameScope ? importTotalCount : 0,
    lastRefreshedAt: sameScope ? lastRefreshedAt : null,
    refreshImport,
    loadMoreImport,
  };
}
