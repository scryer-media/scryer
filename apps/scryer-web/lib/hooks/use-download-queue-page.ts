import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useClient } from "urql";
import { ACTIVITY_READ_TIMEOUT_MS, boundedQuery, QueryTimeoutError } from "@/lib/graphql/bounded-query";
import { useQueryAuthScope } from "@/lib/hooks/use-query-auth-scope";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { useTranslate } from "@/lib/context/translate-context";

import { GlobalStatusContext } from "@/lib/context/global-status-context";
import {
  downloadQueuePageQuery,
  downloadQueueSyncSubscription,
} from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import type {
  DownloadActivityStatus,
  DownloadClientFilterOption,
  DownloadQueueItem,
  SortConfig,
} from "@/lib/types";
import {
  DOWNLOAD_QUEUE_PAGE_SIZE,
  type DownloadQueueRetainedPage,
  downloadQueueSyncRefreshRanges,
  flattenDownloadQueuePages,
  markDownloadQueuePagesStale,
  mergeDownloadQueuePageRange,
  nextContiguousDownloadQueueOffset,
  retainedDownloadQueuePageNeedsRefresh,
  shouldApplyDownloadQueuePageResponse,
  shouldRefreshDownloadQueueSync,
} from "@/lib/utils/download-queue-page";

const SYNC_DEBOUNCE_MS = 300;

type DownloadQueuePagePayload = {
  items: DownloadQueueItem[];
  hasMore: boolean;
  totalCount: number;
  availableClients: DownloadClientFilterOption[];
  revision: number;
  updatedAt: string | null;
  ready: boolean;
  stale: boolean;
};

type QueryPageOptions = {
  deadline?: number;
  reset?: boolean;
  markRetainedStale?: boolean;
  minimumRevision?: number;
};

type UseDownloadQueuePageArgs = {
  enabled: boolean;
  filters: DownloadActivityStatus[];
  clientIds: string[] | null;
  scryerSubmittedOnly: boolean;
  sort: SortConfig;
  titleId?: string | null;
  onErrorStatus?: (message: string) => void;
};

export type UseDownloadQueuePageResult = {
  queueItems: DownloadQueueItem[];
  queueLoading: boolean;
  queueLoadingMore: boolean;
  queueError: string | null;
  queueHasMore: boolean;
  queueTotalCount: number;
  queueAvailableClients: DownloadClientFilterOption[];
  queueReady: boolean;
  queueStale: boolean;
  lastRefreshedAt: Date | null;
  refreshQueue: () => Promise<void>;
  loadMoreQueue: () => Promise<void>;
  setVisibleQueueOffset: (offset: number) => void;
};

export function useDownloadQueuePage({
  enabled,
  filters,
  clientIds,
  scryerSubmittedOnly,
  sort,
  titleId = null,
  onErrorStatus,
}: UseDownloadQueuePageArgs): UseDownloadQueuePageResult {
  const contextGlobalStatus = useContext(GlobalStatusContext);
  const client = useClient();
  const authScope = useQueryAuthScope();
  const t = useTranslate();
  const [pages, setPages] = useState<Map<number, DownloadQueueRetainedPage>>(new Map());
  const [queueLoading, setQueueLoading] = useState(false);
  const [queueLoadingMore, setQueueLoadingMore] = useState(false);
  const [queueError, setQueueError] = useState<string | null>(null);
  const [queueHasMore, setQueueHasMore] = useState(false);
  const [queueTotalCount, setQueueTotalCount] = useState(0);
  const [queueAvailableClients, setQueueAvailableClients] = useState<
    DownloadClientFilterOption[]
  >([]);
  const [queueReady, setQueueReady] = useState(false);
  const [queueSnapshotStale, setQueueSnapshotStale] = useState(false);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const nextOffsetRef = useRef(0);
  const visibleOffsetRef = useRef(0);
  const revisionRef = useRef(0);
  const targetRevisionRef = useRef(0);
  const scopeEpochRef = useRef(0);
  const activeRequestRef = useRef<AbortController | null>(null);
  const queuedRefreshRef = useRef(false);
  const [loadedScope, setLoadedScope] = useState<{ key: string; auth: string | null } | null>(null);
  const requestSequenceRef = useRef(new Map<string, number>());
  const pagesRef = useRef<Map<number, DownloadQueueRetainedPage>>(new Map());
  const pendingSyncRef = useRef(false);
  const syncTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastReportedErrorRef = useRef<string | null>(null);
  const filtersKey = filters.join(",");
  const clientIdsKey = clientIds === null ? "all" : clientIds.join(",");
  const scopeKey = `${filtersKey}:${clientIdsKey}:${scryerSubmittedOnly ? 1 : 0}:${sort.key}:${sort.direction}:${titleId ?? "all"}`;
  const activeScopeKeyRef = useRef(scopeKey);

  const queueItems = useMemo(() => flattenDownloadQueuePages(pages), [pages]);

  useLayoutEffect(() => {
    activeScopeKeyRef.current = scopeKey;
  }, [scopeKey]);

  const applyPage = useCallback(
    (
      payload: DownloadQueuePagePayload,
      offset: number,
      limit: number,
      options: QueryPageOptions,
    ) => {
      const next = mergeDownloadQueuePageRange(
        pagesRef.current,
        payload.items,
        offset,
        limit,
        {
          reset: options.reset ?? false,
          revision: payload.revision,
          totalCount: payload.totalCount,
          markRetainedStale: options.markRetainedStale,
        },
      );
      pagesRef.current = next;
      setPages(next);
      nextOffsetRef.current = nextContiguousDownloadQueueOffset(next, payload.totalCount);
      setQueueHasMore(nextOffsetRef.current < payload.totalCount);
      setQueueTotalCount(payload.totalCount);
      setQueueAvailableClients(payload.availableClients);
      setQueueReady(payload.ready);
      // Retained-page staleness drives revalidation; only the server snapshot
      // health belongs in the user-facing stale warning.
      setQueueSnapshotStale(payload.stale);
      revisionRef.current = Math.max(revisionRef.current, payload.revision);
      setLastRefreshedAt(payload.updatedAt ? new Date(payload.updatedAt) : new Date());
      setQueueError(null);
      lastReportedErrorRef.current = null;
    },
    [],
  );

  const queryPage = useCallback(
    async (offset: number, limit: number, signal: AbortSignal, options: QueryPageOptions = {}) => {
      const requestScopeKey = scopeKey;
      const requestEpoch = scopeEpochRef.current;
      const rangeKey = `${offset}:${limit}`;
      const requestSequence = (requestSequenceRef.current.get(rangeKey) ?? 0) + 1;
      requestSequenceRef.current.set(rangeKey, requestSequence);
      const minimumRevision = Math.max(
        options.minimumRevision ?? 0,
        targetRevisionRef.current,
      );

      for (let attempt = 0; attempt < 2; attempt += 1) {
        const { data, error } = await boundedQuery(client
          .query(
            downloadQueuePageQuery,
            {
              limit,
              offset,
              filters,
              clientIds,
              scryerSubmittedOnly,
              titleId,
              sortKey: sort.key,
              sortDirection: sort.direction,
            },
            { requestPolicy: "network-only" },
          ), signal, Math.max(0, (options.deadline ?? (Date.now() + ACTIVITY_READ_TIMEOUT_MS)) - Date.now()));
        if (
          activeScopeKeyRef.current !== requestScopeKey ||
          authScope !== getAuthToken() ||
          scopeEpochRef.current !== requestEpoch ||
          requestSequenceRef.current.get(rangeKey) !== requestSequence
        ) {
          return null;
        }
        if (error) {
          throw error;
        }
        const payload = data?.downloadQueuePage as DownloadQueuePagePayload | undefined;
        if (!payload || !Array.isArray(payload.items) || !Array.isArray(payload.availableClients) ||
            !Number.isSafeInteger(payload.totalCount) || payload.totalCount < 0 ||
            !Number.isSafeInteger(payload.revision) || typeof payload.hasMore !== "boolean" ||
            typeof payload.ready !== "boolean" || typeof payload.stale !== "boolean") {
          throw new Error("Failed to load queue page.");
        }
        if (
          !shouldApplyDownloadQueuePageResponse(
            payload.revision,
            revisionRef.current,
            minimumRevision,
          )
        ) {
          if (payload.revision < minimumRevision && attempt === 0) {
            continue;
          }
          throw new Error(t("activity.refreshBehind"));
        }
        return payload;
      }
      return null;
    },
    [
      authScope,
      client,
      clientIds,
      filters,
      scopeKey,
      scryerSubmittedOnly,
      sort.direction,
      sort.key,
      t,
      titleId,
    ],
  );

  const reportError = useCallback(
    (error: unknown) => {
      const message = error instanceof QueryTimeoutError ? t("activity.refreshTimedOut") :
        error instanceof Error ? error.message : "Failed to load queue.";
      setQueueError(message);
      setQueueSnapshotStale(true);
      if (lastReportedErrorRef.current !== message) {
        lastReportedErrorRef.current = message;
        (onErrorStatus ?? contextGlobalStatus)?.(message);
      }
    },
    [contextGlobalStatus, onErrorStatus, t],
  );

  const refreshQueue = useCallback(async () => {
    if (!enabled) {
      pendingSyncRef.current = true;
      return;
    }
    if (activeRequestRef.current) {
      queuedRefreshRef.current = true;
      return;
    }
    const epoch = scopeEpochRef.current;
    const current = () => scopeEpochRef.current === epoch && authScope === getAuthToken() && activeScopeKeyRef.current === scopeKey;
    do {
    queuedRefreshRef.current = false;
    const controller = new AbortController();
    activeRequestRef.current = controller;
    let succeeded = false;
    setQueueLoading(true);
    try {
      const targetRevision = targetRevisionRef.current;
      const deadline = Date.now() + ACTIVITY_READ_TIMEOUT_MS;
      const ranges = downloadQueueSyncRefreshRanges(
        nextOffsetRef.current,
        visibleOffsetRef.current,
      );
      let reconciled = true;
      const updates: Array<{ payload: DownloadQueuePagePayload; offset: number; limit: number; options: QueryPageOptions }> = [];
      for (const [index, range] of ranges.entries()) {
        const options: QueryPageOptions = {
          reset: index === 0 && nextOffsetRef.current === 0,
          markRetainedStale: index === 0,
          minimumRevision: updates.at(-1)?.payload.revision ?? targetRevision,
          deadline,
        };
        const payload = await queryPage(range.offset, range.limit, controller.signal, options);
        if (!current()) return;
        reconciled = reconciled && payload !== null;
        if (payload) updates.push({ payload, ...range, options });
      }
      // Publish a refresh only when every requested range succeeded. A failed
      // later range must leave the last successful rows and pagination intact.
      if (reconciled) {
        for (const update of updates) applyPage(update.payload, update.offset, update.limit, update.options);
        setLoadedScope({ key: scopeKey, auth: authScope });
      }
      if (reconciled && revisionRef.current >= targetRevision) {
        pendingSyncRef.current = false;
      }
      succeeded = reconciled;
    } catch (error) {
      if (current()) reportError(error);
    } finally {
      if (current()) {
        activeRequestRef.current = null;
        setQueueLoading(false);
      }
    }
    if (!current() || !succeeded) break;
    } while (queuedRefreshRef.current);
  }, [applyPage, authScope, enabled, queryPage, reportError, scopeKey]);

  const loadMoreQueue = useCallback(async () => {
    if (!enabled || activeRequestRef.current || !queueHasMore) {
      return;
    }
    const epoch = scopeEpochRef.current;
    const current = () => scopeEpochRef.current === epoch && authScope === getAuthToken() && activeScopeKeyRef.current === scopeKey;
    const controller = new AbortController();
    activeRequestRef.current = controller;
    let succeeded = false;
    setQueueLoadingMore(true);
    try {
      const offset = nextOffsetRef.current;
      const payload = await queryPage(offset, DOWNLOAD_QUEUE_PAGE_SIZE, controller.signal, {
        minimumRevision: targetRevisionRef.current,
        deadline: Date.now() + ACTIVITY_READ_TIMEOUT_MS,
      });
      if (current() && payload) {
        applyPage(payload, offset, DOWNLOAD_QUEUE_PAGE_SIZE, {});
        setLoadedScope({ key: scopeKey, auth: authScope });
      }
      succeeded = payload !== null;
    } catch (error) {
      if (current()) reportError(error);
    } finally {
      if (current()) {
        activeRequestRef.current = null;
        setQueueLoadingMore(false);
      }
    }
    if (current() && succeeded && queuedRefreshRef.current) await refreshQueue();
  }, [applyPage, authScope, enabled, queryPage, queueHasMore, refreshQueue, reportError, scopeKey]);

  const scheduleSync = useCallback(() => {
    pendingSyncRef.current = true;
    if (
      !shouldRefreshDownloadQueueSync(enabled, document.visibilityState) ||
      syncTimerRef.current
    ) {
      return;
    }
    syncTimerRef.current = setTimeout(() => {
      syncTimerRef.current = null;
      void refreshQueue();
    }, SYNC_DEBOUNCE_MS);
  }, [enabled, refreshQueue]);

  useDeferredWsSubscription<{
    data?: { downloadQueueSync?: { revision: number; updatedAt: string | null } };
  }>({
    enabled,
    requestKey: `downloadQueueSync:${scopeKey}`,
    request: { query: downloadQueueSyncSubscription },
    onNext(result) {
      const sync = result.data?.downloadQueueSync;
      if (!sync || sync.revision <= targetRevisionRef.current) {
        return;
      }
      targetRevisionRef.current = sync.revision;
      const next = markDownloadQueuePagesStale(pagesRef.current, sync.revision);
      pagesRef.current = next;
      setPages(next);
      scheduleSync();
    },
    onError(error) {
      console.error("[download-queue] sync subscription error:", error);
    },
  });

  useLayoutEffect(() => {
    scopeEpochRef.current += 1;
    requestSequenceRef.current.clear();
    nextOffsetRef.current = 0;
    visibleOffsetRef.current = 0;
    revisionRef.current = 0;
    targetRevisionRef.current = 0;
    pendingSyncRef.current = false;
    queuedRefreshRef.current = false;
    const emptyPages = new Map<number, DownloadQueueRetainedPage>();
    pagesRef.current = emptyPages;
    setPages(emptyPages);
    setQueueHasMore(false);
    setQueueTotalCount(0);
    setQueueSnapshotStale(false);
    setQueueError(null);
    setQueueReady(false);
    setQueueAvailableClients([]);
    setLoadedScope({ key: scopeKey, auth: authScope });
    setLastRefreshedAt(null);
    setQueueLoading(false);
    setQueueLoadingMore(false);
    if (enabled) void refreshQueue();
    return () => {
      scopeEpochRef.current += 1;
      activeRequestRef.current?.abort();
      activeRequestRef.current = null;
      queuedRefreshRef.current = false;
    };
  }, [authScope, enabled, refreshQueue, scopeKey]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    const reconcileOnVisibility = () => {
      if (document.visibilityState === "visible" && pendingSyncRef.current) {
        scheduleSync();
      }
    };
    document.addEventListener("visibilitychange", reconcileOnVisibility);
    window.addEventListener("focus", reconcileOnVisibility);
    return () => {
      document.removeEventListener("visibilitychange", reconcileOnVisibility);
      window.removeEventListener("focus", reconcileOnVisibility);
      if (syncTimerRef.current) {
        clearTimeout(syncTimerRef.current);
        syncTimerRef.current = null;
      }
    };
  }, [enabled, scheduleSync]);

  const setVisibleQueueOffset = useCallback(
    (offset: number) => {
      const normalizedOffset = Math.max(0, offset);
      visibleOffsetRef.current = normalizedOffset;
      if (!enabled || document.visibilityState !== "visible") {
        return;
      }
      if (pendingSyncRef.current) {
        scheduleSync();
        return;
      }
      if (
        retainedDownloadQueuePageNeedsRefresh(
          pagesRef.current,
          normalizedOffset,
          targetRevisionRef.current,
        )
      ) {
        scheduleSync();
      }
    },
    [enabled, scheduleSync],
  );

  const sameScope = loadedScope?.key === scopeKey && loadedScope.auth === authScope;
  return {
    queueItems: sameScope ? queueItems : [],
    queueLoading: enabled && (!sameScope || queueLoading),
    queueLoadingMore: sameScope && queueLoadingMore,
    queueError: sameScope ? queueError : null,
    queueHasMore: sameScope && queueHasMore,
    queueTotalCount: sameScope ? queueTotalCount : 0,
    queueAvailableClients: sameScope ? queueAvailableClients : [],
    queueReady: sameScope && queueReady,
    queueStale: sameScope && queueSnapshotStale,
    lastRefreshedAt: sameScope ? lastRefreshedAt : null,
    refreshQueue,
    loadMoreQueue,
    setVisibleQueueOffset,
  };
}
