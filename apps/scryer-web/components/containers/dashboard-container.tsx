import * as React from "react";
import { useClient, useMutation } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useManualImportLauncher } from "@/components/common/manual-import-launcher";
import { DashboardView } from "@/components/views/dashboard-view";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  dispatchNavigationBadgesRefresh,
  NAVIGATION_BADGES_REFRESH_EVENT,
  type NavigationBadgesRefreshDetail,
} from "@/lib/events/navigation-badges";
import {
  approveMediaRequestMutation,
  deleteDownloadMutation,
  dismissMediaRequestMutation,
  markTrackedDownloadFailedMutation,
} from "@/lib/graphql/mutations";
import {
  dashboardOverviewQuery,
  dashboardStorageQuery,
  dashboardPendingRequestsQuery,
  dashboardRecentImportsQuery,
  downloadImportQuery,
  downloadQueuePageQuery,
} from "@/lib/graphql/queries";
import { usePluginManagement } from "@/lib/hooks/use-plugin-management";
import type {
  DashboardImportedItem,
  DashboardOverview,
  DashboardStorageRoot,
  DashboardPluginUpdate,
  DashboardRequest,
  DashboardRequestLibrary,
  DownloadQueueItem,
} from "@/lib/types";
import { isBreakingVersionChange } from "@/lib/utils/dashboard";
import { createDashboardRefresh, initialDashboardPanelStates } from "@/lib/utils/dashboard-refresh";
import { isHistoryQueueState } from "@/lib/utils/download-queue";

/** Trailing window the two 24h tiles compare against the window before it. */
const ACTIVITY_WINDOW_HOURS = 24;
/**
 * The top panels show roughly three rows but scroll, so they fetch a short page
 * rather than only what is visible. Totals in the badges come from the server's
 * own counts, not from these lists.
 */
const PREVIEW_FETCH_LIMIT = 15;
/**
 * Enough of the queue to aggregate per-client activity from. `downloadQueuePage`
 * has no per-client aggregate, so the counts are folded client-side; 200 rows
 * covers a realistic queue without pulling the whole history.
 */
const QUEUE_FETCH_LIMIT = 200;

export function DashboardContainer() {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();

  const [overview, setOverview] = React.useState<DashboardOverview | null>(null);
  const [requests, setRequests] = React.useState<DashboardRequest[]>([]);
  const [requestLibraries, setRequestLibraries] = React.useState<
    DashboardRequestLibrary[]
  >([]);
  const [importActivity, setImportActivity] = React.useState<DownloadQueueItem[]>(
    [],
  );
  const [importActivityTotal, setImportActivityTotal] = React.useState(0);
  const [recentImports, setRecentImports] = React.useState<DashboardImportedItem[]>(
    [],
  );
  const [queueItems, setQueueItems] = React.useState<DownloadQueueItem[]>([]);
  const [queueTotal, setQueueTotal] = React.useState(0);
  const [storageRoots, setStorageRoots] = React.useState<DashboardStorageRoot[]>([]);
  const [panels, setPanels] = React.useState(initialDashboardPanelStates);
  const refresh = React.useMemo(() => createDashboardRefresh((key, patch) => {
    setPanels((current) => ({ ...current, [key]: { ...current[key], ...patch } }));
  }), []);
  React.useEffect(() => {
    refresh.activate();
    return () => refresh.dispose();
  }, [client, refresh]);
  const [actionRequestId, setActionRequestId] = React.useState<string | null>(null);
  const [deleteConfirmItem, setDeleteConfirmItem] =
    React.useState<DownloadQueueItem | null>(null);
  const [importActionItemId, setImportActionItemId] = React.useState<string | null>(
    null,
  );
  const [deleteInProgress, setDeleteInProgress] = React.useState(false);
  const [, executeMarkTrackedDownloadFailed] = useMutation(
    markTrackedDownloadFailedMutation,
  );
  const [, executeDeleteDownload] = useMutation(deleteDownloadMutation);

  const reportError = React.useCallback(
    (error: unknown) => {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.apiError"),
        { level: "ERROR" },
      );
    },
    [setGlobalStatus, t],
  );

  // The plugin strip is the settings page's own plugin machinery: the same
  // registry query, the same upgrade mutation and the same in-flight tracking,
  // so an update started here behaves exactly as it does under Settings.
  const noopRefreshProviderOptions = React.useCallback(async () => {}, []);
  const { plugins, mutatingPluginIds, upgradePlugin, refreshPluginsRegistry } =
    usePluginManagement({
      client,
      t,
      refreshProviderOptions: noopRefreshProviderOptions,
    });

  const pluginUpdates = React.useMemo<DashboardPluginUpdate[]>(
    () =>
      plugins
        .filter((plugin) => plugin.updateAvailable && plugin.isInstalled)
        .map((plugin) => ({
          id: plugin.id,
          name: plugin.name || plugin.id,
          fromVersion: plugin.installedVersion ?? plugin.version ?? null,
          toVersion: plugin.latestVersion ?? null,
          breaking: isBreakingVersionChange(
            plugin.installedVersion ?? plugin.version ?? null,
            plugin.latestVersion ?? null,
          ),
        })),
    [plugins],
  );

  const refreshOverview = React.useCallback(
    (invalidate = false) =>
      refresh.run(
        "overview",
        async () => {
          const { data, error } = await client
            .query(dashboardOverviewQuery, {
              activityWindowHours: ACTIVITY_WINDOW_HOURS,
            })
            .toPromise();
          if (error) throw error;

          const badges = data?.navigationBadgeCounts;
          const health = data?.dashboardSummary;
          return () =>
            setOverview({
              username: data?.me?.username ?? null,
              pendingRequestCount: sumFacetCounts(
                badges?.pendingMediaRequestCounts,
              ),
              activityImportCount: badges?.activityImportCount ?? 0,
              library: {
                movies: health?.titlesMovie ?? 0,
                series: health?.titlesSeries ?? 0,
                anime: health?.titlesAnime ?? 0,
              },
              activity: data?.dashboardActivityStats ?? {
                current: {
                  grabbed: 0,
                  upgraded: 0,
                  imported: 0,
                  importFailed: 0,
                },
                previous: {
                  grabbed: 0,
                  upgraded: 0,
                  imported: 0,
                  importFailed: 0,
                },
              },
              indexerStats: health?.indexerStats ?? [],
              indexers: data?.indexers ?? [],
              downloadClients: data?.downloadClientConfigs ?? [],
              storageRoots: [],
            });
        },
        invalidate,
      ),
    [client, refresh],
  );

  const refreshRequests = React.useCallback(
    (invalidate = false) =>
      refresh.run(
        "requests",
        async () => {
          const { data, error } = await client
            .query(dashboardPendingRequestsQuery, {})
            .toPromise();
          if (error) throw error;

          const loaded = (data?.mediaRequests ?? []) as DashboardRequest[];
          // Oldest first: the request that has been waiting longest leads.
          return () => {
            setRequests(
              [...loaded]
                .sort(
                  (left, right) =>
                    Date.parse(left.createdAt) - Date.parse(right.createdAt),
                )
                .slice(0, PREVIEW_FETCH_LIMIT),
            );
            setRequestLibraries(
              (data?.libraries ?? []) as DashboardRequestLibrary[],
            );
          };
        },
        invalidate,
      ),
    [client, refresh],
  );

  // The panel mirrors Activity → Imports: downloads that could not be
  // auto-imported, in the same list the nav badge counts.
  const refreshImportActivity = React.useCallback(
    (invalidate = false) =>
      refresh.run(
        "imports",
        async () => {
          const { data, error } = await client
            .query(downloadImportQuery, {
              limit: PREVIEW_FETCH_LIMIT,
              offset: 0,
              filter: "ATTENTION",
            })
            .toPromise();
          if (error) throw error;

          const items = (data?.downloadImport?.items ??
            []) as DownloadQueueItem[];
          // Oldest first: the download that has been stuck longest leads.
          return () => {
            setImportActivity(
              [...items].sort(
                (left, right) =>
                  Date.parse(left.queuedAt ?? left.lastUpdatedAt ?? "") -
                  Date.parse(right.queuedAt ?? right.lastUpdatedAt ?? ""),
              ),
            );
            setImportActivityTotal(data?.downloadImport?.totalCount ?? 0);
          };
        },
        invalidate,
      ),
    [client, refresh],
  );

  const refreshRecentImports = React.useCallback(
    (invalidate = false) =>
      refresh.run(
        "recent",
        async () => {
          const { data, error } = await client
            .query(dashboardRecentImportsQuery, { limit: PREVIEW_FETCH_LIMIT })
            .toPromise();
          if (error) throw error;

          return () =>
            setRecentImports(
              (data?.dashboardRecentImports ?? []) as DashboardImportedItem[],
            );
        },
        invalidate,
      ),
    [client, refresh],
  );

  const refreshQueue = React.useCallback(
    (invalidate = false) =>
      refresh.run(
        "queue",
        async () => {
          const { data, error } = await client
            .query(downloadQueuePageQuery, {
              limit: QUEUE_FETCH_LIMIT,
              scryerSubmittedOnly: false,
            })
            .toPromise();
          if (error) throw error;

          return () => {
            setQueueItems(
              (data?.downloadQueuePage?.items ?? []) as DownloadQueueItem[],
            );
            setQueueTotal(data?.downloadQueuePage?.totalCount ?? 0);
          };
        },
        invalidate,
      ),
    [client, refresh],
  );

  const refreshStorage = React.useCallback(
    (invalidate = false) =>
      refresh.run(
        "storage",
        async () => {
          const { data, error } = await client
            .query(dashboardStorageQuery, {})
            .toPromise();
          if (error) throw error;
          return () => setStorageRoots(data?.storageRoots ?? []);
        },
        invalidate,
      ),
    [client, refresh],
  );

  // One import-affecting action finished: the nav badge shrinks and every panel
  // the action touched re-reads its source.
  const refreshAfterImportAction = React.useCallback(() => {
    dispatchNavigationBadgesRefresh({ delta: -1 });
    void refreshOverview(true);
    void refreshImportActivity(true);
    void refreshQueue(true);
  }, [refreshImportActivity, refreshOverview, refreshQueue]);

  // The activity page's action subset: movies import directly, series and anime
  // open the mapper dialog. Everything richer lives on the import page.
  const manualImport = useManualImportLauncher({
    onImportQueued: refreshAfterImportAction,
    onImportRetried: () => {
      dispatchNavigationBadgesRefresh();
      void refreshOverview(true);
      void refreshImportActivity(true);
      void refreshQueue(true);
    },
  });

  const markImportFailed = React.useCallback(
    async (item: DownloadQueueItem) => {
      setImportActionItemId(item.id);
      try {
        const result = await executeMarkTrackedDownloadFailed({
          input: {
            clientId: item.clientId,
            clientType: item.clientType,
            downloadClientItemId: item.downloadClientItemId,
            skipReacquire: false,
          },
        });
        if (result.error) {
          setGlobalStatus(result.error.message ?? t("queue.markFailedFailed"), { level: "ERROR" });
          return;
        }
        setGlobalStatus(t("queue.markFailedSearchSuccess"), { level: "SUCCESS" });
        refreshAfterImportAction();
      } finally {
        setImportActionItemId(null);
      }
    },
    [executeMarkTrackedDownloadFailed, refreshAfterImportAction, setGlobalStatus, t],
  );

  const removeFromClient = React.useCallback(async () => {
    const item = deleteConfirmItem;
    if (!item) {
      return;
    }
    setDeleteInProgress(true);
    try {
      const result = await executeDeleteDownload({
        input: {
          clientId: item.clientId,
          clientType: item.clientType,
          downloadClientItemId: item.downloadClientItemId,
          isHistory: isHistoryQueueState(item.state),
        },
      });
      if (result.error) {
        setGlobalStatus(result.error.message ?? t("queue.deleteFailed"), { level: "ERROR" });
        return;
      }
      setGlobalStatus(t("queue.deleteQueued"), { level: "SUCCESS" });
      setDeleteConfirmItem(null);
      refreshAfterImportAction();
    } finally {
      setDeleteInProgress(false);
    }
  }, [
    deleteConfirmItem,
    executeDeleteDownload,
    refreshAfterImportAction,
    setGlobalStatus,
    t,
  ]);

  const refreshAll = React.useCallback(async () => {
    await Promise.all([
      refreshOverview(), refreshStorage(), refreshRequests(),
      refreshImportActivity(), refreshRecentImports(), refreshQueue(),
    ]);
  }, [refreshOverview, refreshStorage, refreshRequests, refreshImportActivity, refreshRecentImports, refreshQueue]);

  React.useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  // A request action only changes the overview counts and the request rail;
  // the queue, import activity, and recent imports are untouched, so skip them.
  const refreshAfterRequestAction = React.useCallback(async () => {
    await Promise.all([refreshOverview(true), refreshRequests(true)]);
  }, [refreshOverview, refreshRequests]);

  // The shell already pulses the badge counts on poll and on window focus;
  // riding that pulse keeps the dashboard fresh without a timer of its own.
  React.useEffect(() => {
    let lastPulse = 0;
    const handlePulse = (event: Event) => {
      if (!(event instanceof CustomEvent)) {
        return;
      }
      const source = (event as CustomEvent<NavigationBadgesRefreshDetail>).detail
        ?.source;
      if (!document.hidden && (source === "poll" || source === "focus") && Date.now() - lastPulse > 2000) {
        lastPulse = Date.now();
        void refreshAll();
      }
    };

    window.addEventListener(NAVIGATION_BADGES_REFRESH_EVENT, handlePulse);
    return () => {
      window.removeEventListener(NAVIGATION_BADGES_REFRESH_EVENT, handlePulse);
    };
  }, [refreshAll]);

  const approveRequest = React.useCallback(
    async (request: DashboardRequest) => {
      if (actionRequestId) {
        return;
      }
      const qualityProfileId = resolveApprovalProfileId(request, requestLibraries);
      if (!qualityProfileId) {
        // Approving needs a profile and the dashboard has no picker, so send the
        // operator to the requests page rather than guessing.
        setGlobalStatus(t("status.apiError"), { level: "ERROR" });
        return;
      }

      setActionRequestId(request.id);
      try {
        const { error } = await client
          .mutation(approveMediaRequestMutation, {
            input: {
              requestId: request.id,
              qualityProfileId,
              monitorType:
                request.facet === "MOVIE" ? null : request.requestedMonitorType,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestApproved", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refreshAfterRequestAction();
      } catch (error) {
        reportError(error);
      } finally {
        setActionRequestId(null);
      }
    },
    [
      actionRequestId,
      client,
      refreshAfterRequestAction,
      reportError,
      requestLibraries,
      setGlobalStatus,
      t,
    ],
  );

  const dismissRequest = React.useCallback(
    async (request: DashboardRequest) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      try {
        const { error } = await client
          .mutation(dismissMediaRequestMutation, { requestId: request.id })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestDismissed", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refreshAfterRequestAction();
      } catch (error) {
        reportError(error);
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, refreshAfterRequestAction, reportError, setGlobalStatus, t],
  );

  const updatePlugin = React.useCallback(
    (pluginId: string) => {
      const plugin = plugins.find((candidate) => candidate.id === pluginId);
      if (!plugin) {
        return;
      }
      void upgradePlugin(plugin);
    },
    [plugins, upgradePlugin],
  );

  const updateAllPlugins = React.useCallback(() => {
    for (const update of pluginUpdates) {
      const plugin = plugins.find((candidate) => candidate.id === update.id);
      if (plugin) {
        void upgradePlugin(plugin);
      }
    }
  }, [pluginUpdates, plugins, upgradePlugin]);

  // A finished upgrade changes the registry, so refresh the strip's source.
  React.useEffect(() => {
    if (mutatingPluginIds.length > 0) {
      return;
    }
    void refreshPluginsRegistry();
  }, [mutatingPluginIds.length, refreshPluginsRegistry]);

  return (
    <>
      <DashboardView
        panels={panels}
        storageRoots={storageRoots}
        overview={overview}
        requests={requests}
        importActivity={importActivity}
        importActivityTotal={importActivityTotal}
        recentImports={recentImports}
        queueItems={queueItems}
        queueTotal={queueTotal}
        pluginUpdates={pluginUpdates}
        updatingPluginIds={mutatingPluginIds}
        actionRequestId={actionRequestId}
        importActionItemId={importActionItemId ?? manualImport.busyItemId}
        onApproveRequest={(request) => void approveRequest(request)}
        onDismissRequest={(request) => void dismissRequest(request)}
        onImportItem={(item) => void manualImport.launch(item)}
        onMarkImportFailed={(item) => void markImportFailed(item)}
        onRemoveImportItem={setDeleteConfirmItem}
        onUpdatePlugin={updatePlugin}
        onUpdateAllPlugins={updateAllPlugins}
      />
      <ConfirmDialog
        open={deleteConfirmItem !== null}
        title={t("queue.deleteConfirmTitle")}
        description={t("queue.deleteConfirmDescription")}
        confirmLabel={t("queue.removeFromDownloader")}
        cancelLabel={t("label.cancel")}
        isBusy={deleteInProgress}
        onConfirm={() => void removeFromClient()}
        onCancel={() => setDeleteConfirmItem(null)}
      />
      {manualImport.dialog}
    </>
  );
}

function sumFacetCounts(
  counts: { movie?: number; series?: number; anime?: number } | null | undefined,
): number {
  return (counts?.movie ?? 0) + (counts?.series ?? 0) + (counts?.anime ?? 0);
}

/**
 * Which quality profile an inline approval should use, following the same
 * precedence the requests page's approval dialog pre-selects: what the
 * requester asked for, then the library's own profile, then the library's
 * request default. Returns null when none is known, and the caller declines to
 * approve rather than picking one arbitrarily.
 */
function resolveApprovalProfileId(
  request: DashboardRequest,
  libraries: DashboardRequestLibrary[],
): string | null {
  const requested = request.requestedQualityProfileId?.trim();
  if (requested) {
    return requested;
  }

  const library = libraries.find((entry) => entry.id === request.libraryId);
  return (
    library?.qualityProfileId?.trim() ||
    library?.requestQualityProfileDefaultId?.trim() ||
    null
  );
}
