
import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { useClient, useMutation } from "urql";

import { AssignTrackedDownloadTitleDialog } from "@/components/dialogs/assign-tracked-download-title-dialog";
import { ManualImportDialog } from "@/components/dialogs/manual-import-dialog";
import { ActivityView } from "@/components/views/activity-view";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  assignTrackedDownloadTitleMutation,
  buildDeleteDownloadBatchMutation,
  buildIgnoreTrackedDownloadBatchMutation,
  ignoreTrackedDownloadMutation,
  markTrackedDownloadFailedMutation,
  beginManualImportSelectionMutation,
  queueManualImportMutation,
  pauseDownloadMutation,
  resumeDownloadMutation,
  deleteDownloadMutation,
} from "@/lib/graphql/mutations";
import { downloadClientsQuery } from "@/lib/graphql/queries";
import { useDownloadHistory } from "@/lib/hooks/use-download-history";
import { useDownloadImport } from "@/lib/hooks/use-download-import";
import { useDownloadQueue } from "@/lib/hooks/use-download-queue";
import { useImportHistorySubscription } from "@/lib/hooks/use-import-history-subscription";
import { dispatchNavigationBadgesRefresh } from "@/lib/events/navigation-badges";
import type { ActivitySection } from "@/components/root/types";
import type {
  DownloadClientRecord,
  DownloadActivityStatus,
  DownloadClientFilterOption,
  DownloadHistoryStatus,
  DownloadImportStatus,
  DownloadQueueItem,
  SortConfig,
} from "@/lib/types";
import {
  collectDownloadClientFilterOptions,
  downloadQueueClientFilterKey,
  downloadQueueItemIdentityKey,
  matchesActivityStatuses,
  matchesImportStatuses,
} from "@/lib/utils/download-queue";

const HISTORY_STATES = new Set(["completed", "failed", "import_pending", "importpending"]);
type ActivityTab = ActivitySection;
type SortConfigByTab = Record<ActivityTab, SortConfig>;

const IMPORT_STATUS_OPTIONS: DownloadImportStatus[] = [
  "IMPORTING",
  "PENDING",
  "BLOCKED",
  "FAILED",
];
const ACTIVITY_STATUS_OPTIONS: DownloadActivityStatus[] = [
  "DOWNLOADING",
  "QUEUED",
  "PAUSED",
  "POST_PROCESSING",
];
const HISTORY_STATUS_OPTIONS: DownloadHistoryStatus[] = ["SUCCESS", "FAILED"];
const DEFAULT_SORT_CONFIG_BY_TAB: SortConfigByTab = {
  import: { key: "STATUS", direction: "ASC" },
  activity: { key: "STATUS", direction: "ASC" },
  history: { key: "STATUS", direction: "ASC" },
};

function arraysEqual<T>(left: T[], right: T[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function toggleSelectedValue<T extends string>(current: T[], nextValue: T): T[] {
  return current.includes(nextValue)
    ? current.filter((value) => value !== nextValue)
    : [...current, nextValue];
}

function uniqueQueueItems(items: DownloadQueueItem[]): DownloadQueueItem[] {
  const seen = new Set<string>();
  const uniqueItems: DownloadQueueItem[] = [];
  for (const item of items) {
    const key = downloadQueueItemIdentityKey(item);
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    uniqueItems.push(item);
  }
  return uniqueItems;
}

function isHistoryQueueItem(item: DownloadQueueItem): boolean {
  return HISTORY_STATES.has(item.state.trim().toLowerCase());
}

function mergeDownloadClientFilterOptions(
  configuredOptions: DownloadClientFilterOption[],
  visibleOptions: DownloadClientFilterOption[],
): DownloadClientFilterOption[] {
  const merged = new Map<string, DownloadClientFilterOption>();

  for (const option of visibleOptions) {
    merged.set(option.clientId, option);
  }

  for (const option of configuredOptions) {
    if (!merged.has(option.clientId)) {
      merged.set(option.clientId, option);
    }
  }

  return Array.from(merged.values()).sort((left, right) =>
    (left.clientName || left.clientType).localeCompare(right.clientName || right.clientType, undefined, {
      sensitivity: "base",
    }),
  );
}

export const ActivityContainer = memo(function ActivityContainer({
  activitySection,
}: {
  activitySection: ActivitySection;
}) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [, executeQueueManualImport] = useMutation(queueManualImportMutation);
  const [, executeBeginManualImportSelection] = useMutation(
    beginManualImportSelectionMutation,
  );
  const [, executeAssignTrackedDownloadTitle] = useMutation(assignTrackedDownloadTitleMutation);
  const [, executeIgnoreTrackedDownload] = useMutation(ignoreTrackedDownloadMutation);
  const [, executeMarkTrackedDownloadFailed] = useMutation(markTrackedDownloadFailedMutation);
  const [, executePauseDownload] = useMutation(pauseDownloadMutation);
  const [, executeResumeDownload] = useMutation(resumeDownloadMutation);
  const [, executeDeleteDownload] = useMutation(deleteDownloadMutation);

  const activeTab = activitySection;
  const importTabActive = activeTab === "import";
  const activityTabActive = activeTab === "activity";
  const historyTabActive = activeTab === "history";
  const [selectedImportStatuses, setSelectedImportStatuses] = useState<DownloadImportStatus[]>([
    ...IMPORT_STATUS_OPTIONS,
  ]);
  const [selectedActivityStatuses, setSelectedActivityStatuses] = useState<
    DownloadActivityStatus[]
  >([...ACTIVITY_STATUS_OPTIONS]);
  const [selectedHistoryStatuses, setSelectedHistoryStatuses] = useState<DownloadHistoryStatus[]>(
    [...HISTORY_STATUS_OPTIONS],
  );
  const [activityScryerSubmittedOnly, setActivityScryerSubmittedOnly] = useState(true);
  const [historyScryerSubmittedOnly, setHistoryScryerSubmittedOnly] = useState(true);
  const [selectedActivityClientIds, setSelectedActivityClientIds] = useState<string[] | null>(
    null,
  );
  const [selectedHistoryClientIds, setSelectedHistoryClientIds] = useState<string[] | null>(
    null,
  );
  const [sortConfigByTab, setSortConfigByTab] =
    useState<SortConfigByTab>(DEFAULT_SORT_CONFIG_BY_TAB);
  const [configuredClientOptions, setConfiguredClientOptions] = useState<
    DownloadClientFilterOption[]
  >([]);
  const [historyPage, setHistoryPage] = useState(1);
  const [manualImportItem, setManualImportItem] = useState<DownloadQueueItem | null>(null);
  const [assignTitleItem, setAssignTitleItem] = useState<DownloadQueueItem | null>(null);
  const [optimisticallyRemovedKeys, setOptimisticallyRemovedKeys] = useState<
    Record<string, true>
  >({});

  const {
    queueItems: activityQueueItems,
    queueLoading,
    queueError,
    lastRefreshedAt: queueLastRefreshedAt,
    refreshQueue,
  } = useDownloadQueue({
    enabled: activityTabActive,
    includeAllActivity: !activityScryerSubmittedOnly,
    includeHistoryOnly: false,
    activityFilter: "ALL",
  });
  const {
    importItems,
    importLoading,
    importLoadingMore,
    importError,
    importHasMore,
    lastRefreshedAt: importLastRefreshedAt,
    refreshImport,
    loadMoreImport,
  } = useDownloadImport({
    enabled: importTabActive,
    filter: "ALL",
  });
  const {
    historyItems,
    historyLoading,
    historyError,
    historyTotalPages,
    historyAvailableClients,
    lastRefreshedAt: historyLastRefreshedAt,
    refreshHistory,
  } = useDownloadHistory({
    enabled: historyTabActive,
    filters: selectedHistoryStatuses,
    clientIds: selectedHistoryClientIds,
    scryerSubmittedOnly: historyScryerSubmittedOnly,
    page: historyPage,
    sort: sortConfigByTab.history,
  });

  const filteredImportItems = useMemo(() => {
    return importItems.filter((item) => matchesImportStatuses(item, selectedImportStatuses));
  }, [importItems, selectedImportStatuses]);
  const statusFilteredActivityItems = useMemo(() => {
    return activityQueueItems.filter((item) => matchesActivityStatuses(item, selectedActivityStatuses));
  }, [activityQueueItems, selectedActivityStatuses]);
  const activityAvailableClients = useMemo<DownloadClientFilterOption[]>(() => {
    return mergeDownloadClientFilterOptions(
      configuredClientOptions,
      collectDownloadClientFilterOptions(statusFilteredActivityItems),
    );
  }, [configuredClientOptions, statusFilteredActivityItems]);
  const mergedHistoryAvailableClients = useMemo<DownloadClientFilterOption[]>(() => {
    return mergeDownloadClientFilterOptions(configuredClientOptions, historyAvailableClients);
  }, [configuredClientOptions, historyAvailableClients]);
  const filteredActivityItems = useMemo(() => {
    if (selectedActivityClientIds === null) {
      return statusFilteredActivityItems;
    }
    if (selectedActivityClientIds.length === 0) {
      return [];
    }
    const selectedClientIds = new Set(selectedActivityClientIds);
    return statusFilteredActivityItems.filter((item) =>
      selectedClientIds.has(downloadQueueClientFilterKey(item)),
    );
  }, [selectedActivityClientIds, statusFilteredActivityItems]);
  const visibleItems = useMemo(() => {
    const sourceItems =
      activeTab === "import"
        ? filteredImportItems
        : activeTab === "history"
          ? historyItems
          : filteredActivityItems;
    return sourceItems.filter(
      (item) => !optimisticallyRemovedKeys[downloadQueueItemIdentityKey(item)],
    );
  }, [
    activeTab,
    filteredActivityItems,
    filteredImportItems,
    historyItems,
    optimisticallyRemovedKeys,
  ]);
  const initialImportLoading =
    importLoading && filteredImportItems.length === 0 && importLastRefreshedAt === null;
  const initialHistoryLoading =
    historyLoading && historyItems.length === 0 && historyLastRefreshedAt === null;
  const initialActivityLoading =
    queueLoading && filteredActivityItems.length === 0 && queueLastRefreshedAt === null;
  const visibleLoading =
    activeTab === "import"
      ? initialImportLoading
      : activeTab === "history"
        ? initialHistoryLoading
        : initialActivityLoading;
  const visibleLoadingMore = activeTab === "import" ? importLoadingMore : false;
  const visibleHasMore = activeTab === "import" ? importHasMore : false;
  const visibleError =
    activeTab === "import"
      ? importError
      : activeTab === "history"
      ? historyError
      : queueError;
  const historyHasPreviousPage = historyPage > 1;
  const historyHasNextPage = historyPage < historyTotalPages;

  const refreshConfiguredClients = useCallback(async () => {
    try {
      const { data, error } = await client.query(downloadClientsQuery, {}).toPromise();
      if (error) {
        throw error;
      }
      const configuredClients: DownloadClientRecord[] = data?.downloadClientConfigs || [];
      setConfiguredClientOptions(
        configuredClients
          .filter((downloadClient) => downloadClient.isEnabled)
          .map((downloadClient) => ({
            clientId: downloadClient.id,
            clientName: downloadClient.name,
            clientType: downloadClient.clientType,
          })),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    if (!activityTabActive && !historyTabActive) {
      return;
    }
    void refreshConfiguredClients();
  }, [activityTabActive, historyTabActive, refreshConfiguredClients]);

  useEffect(() => {
    const availableClientIds = activityAvailableClients.map((client) => client.clientId);
    setSelectedActivityClientIds((current) => {
      if (availableClientIds.length === 0) {
        if (current === null || current.length === 0) {
          return current;
        }
        return [];
      }
      if (current === null) {
        return availableClientIds;
      }
      const next = current.filter((clientId) => availableClientIds.includes(clientId));
      return arraysEqual(current, next) ? current : next;
    });
  }, [activityAvailableClients]);

  useEffect(() => {
    const availableClientIds = mergedHistoryAvailableClients.map((client) => client.clientId);
    setSelectedHistoryClientIds((current) => {
      if (availableClientIds.length === 0) {
        if (current === null || current.length === 0) {
          return current;
        }
        return [];
      }
      if (current === null) {
        return availableClientIds;
      }
      const next = current.filter((clientId) => availableClientIds.includes(clientId));
      return arraysEqual(current, next) ? current : next;
    });
  }, [mergedHistoryAvailableClients]);

  useEffect(() => {
    setHistoryPage(1);
  }, [
    selectedHistoryStatuses,
    selectedHistoryClientIds,
    historyScryerSubmittedOnly,
    sortConfigByTab.history.direction,
    sortConfigByTab.history.key,
  ]);

  useEffect(() => {
    if (historyTotalPages > 0 && historyPage > historyTotalPages) {
      setHistoryPage(historyTotalPages);
    }
  }, [historyPage, historyTotalPages]);

  const refreshVisibleTab = useCallback(async () => {
    switch (activeTab) {
      case "activity":
        await Promise.all([refreshQueue(), refreshConfiguredClients()]);
        break;
      case "history":
        await Promise.all([refreshHistory(), refreshConfiguredClients()]);
        break;
      case "import":
      default:
        await refreshImport();
        break;
    }
  }, [activeTab, refreshConfiguredClients, refreshHistory, refreshImport, refreshQueue]);

  const refreshImportDrivenViews = useCallback(async () => {
    if (importTabActive) {
      await refreshImport();
      return;
    }

    if (historyTabActive) {
      await Promise.all([refreshHistory(), refreshConfiguredClients()]);
    }
  }, [
    historyTabActive,
    importTabActive,
    refreshConfiguredClients,
    refreshHistory,
    refreshImport,
  ]);

  useImportHistorySubscription(() => {
    void refreshImportDrivenViews();
  }, { pause: activityTabActive });

  useEffect(() => {
    if (Object.keys(optimisticallyRemovedKeys).length === 0) {
      return;
    }

    const authoritativeItems = [...activityQueueItems, ...importItems, ...historyItems];
    const authoritativeByKey = new Map(
      authoritativeItems.map((item) => [downloadQueueItemIdentityKey(item), item]),
    );

    setOptimisticallyRemovedKeys((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(([key]) => {
          const item = authoritativeByKey.get(key);
          if (!item) {
            return false;
          }

          return item.deleteStatus !== "FAILED";
        }),
      );

      return Object.keys(next).length === Object.keys(current).length ? current : next;
    });
  }, [activityQueueItems, historyItems, importItems, optimisticallyRemovedKeys]);

  const decrementImportBadges = useCallback((count = 1) => {
    dispatchNavigationBadgesRefresh({ delta: -Math.max(1, count) });
  }, []);

  const requestManualImport = useCallback(
    async (item: DownloadQueueItem) => {
      if (!item.titleId) {
        setGlobalStatus(t("queue.assignTitleBeforeImport"));
        return;
      }

      if (item.facet === "SERIES" || item.facet === "ANIME") {
        setManualImportItem(item);
        return;
      }

      // queueManualImport is selection-based: it takes a selectionId plus
      // per-candidate mappings, not the raw client/title identity. Open a
      // selection first and import every candidate it reports.
      //
      // Movies carry no episode or series-movie target, so each mapping is just
      // its candidateId — both target fields on ManualImportCandidateMappingInput
      // are optional. Series and anime never reach here; they open the dialog
      // above so the user can map files to episodes.
      const selection = await executeBeginManualImportSelection({
        input: {
          clientId: item.clientId,
          clientType: item.clientType,
          downloadClientItemId: item.downloadClientItemId,
          titleId: item.titleId,
        },
      });
      if (selection.error) {
        const message = selection.error.message ?? t("queue.manualImportFailed");
        setGlobalStatus(message);
        throw selection.error;
      }

      const preview = selection.data?.beginManualImportSelection;
      const files = (preview?.files ?? []).map((file: { candidateId: string }) => ({
        candidateId: file.candidateId,
      }));
      if (!preview?.selectionId || files.length === 0) {
        setGlobalStatus(t("queue.manualImportFailed"));
        return;
      }

      const result = await executeQueueManualImport({
        input: {
          selectionId: preview.selectionId,
          files,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.manualImportFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.manualImportQueued"));
      await refreshVisibleTab();
    },
    [
      executeBeginManualImportSelection,
      executeQueueManualImport,
      refreshVisibleTab,
      setGlobalStatus,
      t,
    ],
  );

  const requestAssignTitle = useCallback(
    async (item: DownloadQueueItem, titleId: string) => {
      const result = await executeAssignTrackedDownloadTitle({
        input: {
          clientId: item.clientId,
          clientType: item.clientType,
          downloadClientItemId: item.downloadClientItemId,
          titleId,
          scope: { title: true },
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.assignTitleFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.assignTitleQueued"));
      await refreshVisibleTab();
    },
    [executeAssignTrackedDownloadTitle, refreshVisibleTab, setGlobalStatus, t],
  );

  const requestIgnore = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executeIgnoreTrackedDownload({
        input: {
          clientId: item.clientId,
          clientType: item.clientType,
          downloadClientItemId: item.downloadClientItemId,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.ignoreFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.ignoreSuccess"));
      await refreshVisibleTab();
    },
    [executeIgnoreTrackedDownload, refreshVisibleTab, setGlobalStatus, t],
  );

  const requestMarkFailed = useCallback(
    async (item: DownloadQueueItem, skipReacquire: boolean) => {
      const result = await executeMarkTrackedDownloadFailed({
        input: {
          clientId: item.clientId,
          clientType: item.clientType,
          downloadClientItemId: item.downloadClientItemId,
          skipReacquire,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.markFailedFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(
        skipReacquire ? t("queue.markFailedOnlySuccess") : t("queue.markFailedSearchSuccess"),
      );
      await refreshVisibleTab();
    },
    [executeMarkTrackedDownloadFailed, refreshVisibleTab, setGlobalStatus, t],
  );

  const requestIgnoreItems = useCallback(
    async (items: DownloadQueueItem[]) => {
      const targets = uniqueQueueItems(items);
      if (targets.length === 0) {
        return;
      }

      const variables = Object.fromEntries(
        targets.map((item, index) => [
          `input${index}`,
          {
            clientId: item.clientId,
            clientType: item.clientType,
            downloadClientItemId: item.downloadClientItemId,
          },
        ]),
      );
      const result = await client
        .mutation<Record<string, unknown>>(
          buildIgnoreTrackedDownloadBatchMutation(targets.length),
          variables,
        )
        .toPromise();
      const data = result.data ?? {};
      const succeeded = targets.filter((_, index) => Boolean(data[`item${index}`])).length;
      const failed = targets.length - succeeded;

      if (succeeded === 0) {
        setGlobalStatus(t("queue.bulkIgnoreFailed"));
      } else if (failed > 0) {
        setGlobalStatus(t("queue.bulkIgnorePartial", { count: succeeded, failed }));
      } else {
        setGlobalStatus(t("queue.bulkIgnoreSuccess", { count: succeeded }));
      }
      await refreshVisibleTab();
    },
    [client, refreshVisibleTab, setGlobalStatus, t],
  );

  const requestPause = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executePauseDownload({
        input: {
          clientId: item.clientId,
          downloadClientItemId: item.downloadClientItemId,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.pauseFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.pauseSuccess"));
      await refreshVisibleTab();
    },
    [executePauseDownload, refreshVisibleTab, setGlobalStatus, t],
  );

  const requestResume = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executeResumeDownload({
        input: {
          clientId: item.clientId,
          downloadClientItemId: item.downloadClientItemId,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.resumeFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.resumeSuccess"));
      await refreshVisibleTab();
    },
    [executeResumeDownload, refreshVisibleTab, setGlobalStatus, t],
  );

  const requestDelete = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executeDeleteDownload({
        input: {
          clientId: item.clientId,
          clientType: item.clientType,
          downloadClientItemId: item.downloadClientItemId,
          isHistory: isHistoryQueueItem(item),
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.deleteFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setOptimisticallyRemovedKeys((current) => ({
        ...current,
        [downloadQueueItemIdentityKey(item)]: true,
      }));
      if (matchesImportStatuses(item, IMPORT_STATUS_OPTIONS)) {
        decrementImportBadges();
      }
      setGlobalStatus(t("queue.deleteQueued"));
      void refreshQueue();
      void refreshImport();
      void refreshHistory();
    },
    [
      decrementImportBadges,
      executeDeleteDownload,
      refreshHistory,
      refreshImport,
      refreshQueue,
      setGlobalStatus,
      t,
    ],
  );

  const requestDeleteItems = useCallback(
    async (items: DownloadQueueItem[]) => {
      const targets = uniqueQueueItems(items);
      if (targets.length === 0) {
        return;
      }

      const variables = Object.fromEntries(
        targets.map((item, index) => [
          `input${index}`,
          {
            clientId: item.clientId,
            clientType: item.clientType,
            downloadClientItemId: item.downloadClientItemId,
            isHistory: isHistoryQueueItem(item),
          },
        ]),
      );
      const result = await client
        .mutation<Record<string, unknown>>(
          buildDeleteDownloadBatchMutation(targets.length),
          variables,
        )
        .toPromise();
      const data = result.data ?? {};
      const succeededItems = targets.filter((_, index) => Boolean(data[`item${index}`]));

      const succeeded = succeededItems.length;
      const failed = targets.length - succeeded;

      if (succeeded > 0) {
        setOptimisticallyRemovedKeys((current) => {
          const next = { ...current };
          for (const item of succeededItems) {
            next[downloadQueueItemIdentityKey(item)] = true;
          }
          return next;
        });
        const importSucceeded = succeededItems.filter((item) =>
          matchesImportStatuses(item, IMPORT_STATUS_OPTIONS),
        ).length;
        if (importSucceeded > 0) {
          decrementImportBadges(importSucceeded);
        }
      }

      if (succeeded === 0) {
        setGlobalStatus(t("queue.bulkDeleteFailed"));
      } else if (failed > 0) {
        setGlobalStatus(t("queue.bulkDeletePartial", { count: succeeded, failed }));
      } else {
        setGlobalStatus(t("queue.bulkDeleteQueued", { count: succeeded }));
      }

      void refreshQueue();
      void refreshImport();
      void refreshHistory();
    },
    [
      client,
      decrementImportBadges,
      refreshHistory,
      refreshImport,
      refreshQueue,
      setGlobalStatus,
      t,
    ],
  );

  return (
    <>
      <ActivityView
        state={{
          queueItems: visibleItems,
          queueLoading: visibleLoading,
          queueLoadingMore: visibleLoadingMore,
          queueError: visibleError,
          requestManualImport,
          requestAssignTitle: async (item) => {
            setAssignTitleItem(item);
          },
          requestIgnore,
          requestMarkFailed,
          requestIgnoreItems,
          requestPause,
          requestResume,
          requestDelete,
          requestDeleteItems,
          activeTab,
          sortConfigByTab,
          toggleSort: (tab, nextKey) => {
            setSortConfigByTab((current) => {
              const currentConfig = current[tab];
              return {
                ...current,
                [tab]:
                  currentConfig.key === nextKey
                    ? {
                        key: nextKey,
                        direction: currentConfig.direction === "ASC" ? "desc" : "asc",
                      }
                    : DEFAULT_SORT_CONFIG_BY_TAB[tab].key === nextKey
                      ? DEFAULT_SORT_CONFIG_BY_TAB[tab]
                      : { key: nextKey, direction: "asc" },
              };
            });
          },
          activityScryerSubmittedOnly,
          toggleActivityScryerSubmittedOnly: () => {
            setActivityScryerSubmittedOnly((current) => !current);
          },
          historyScryerSubmittedOnly,
          toggleHistoryScryerSubmittedOnly: () => {
            setHistoryScryerSubmittedOnly((current) => !current);
          },
          selectedImportStatuses,
          toggleImportStatus: (status) => {
            setSelectedImportStatuses((current) => toggleSelectedValue(current, status));
          },
          selectedActivityStatuses,
          toggleActivityStatus: (status) => {
            setSelectedActivityStatuses((current) => toggleSelectedValue(current, status));
          },
          selectedHistoryStatuses,
          toggleHistoryStatus: (status) => {
            setSelectedHistoryStatuses((current) => toggleSelectedValue(current, status));
          },
          activityAvailableClients,
          selectedActivityClientIds:
            selectedActivityClientIds ?? activityAvailableClients.map((client) => client.clientId),
          toggleActivityClientId: (clientId) => {
            setSelectedActivityClientIds((current) =>
              toggleSelectedValue(current ?? activityAvailableClients.map((client) => client.clientId), clientId),
            );
          },
          historyAvailableClients: mergedHistoryAvailableClients,
          selectedHistoryClientIds:
            selectedHistoryClientIds ??
            mergedHistoryAvailableClients.map((client) => client.clientId),
          toggleHistoryClientId: (clientId) => {
            setSelectedHistoryClientIds((current) =>
              toggleSelectedValue(
                current ?? mergedHistoryAvailableClients.map((client) => client.clientId),
                clientId,
              ),
            );
          },
          historyPage,
          historyTotalPages,
          goToPreviousHistoryPage: async () => {
            setHistoryPage((current) => Math.max(1, current - 1));
          },
          goToNextHistoryPage: async () => {
            setHistoryPage((current) => Math.min(historyTotalPages, current + 1));
          },
          historyHasPreviousPage,
          historyHasNextPage,
          visibleHasMore,
          requestMoreItems:
            activeTab === "import" ? loadMoreImport : async () => {},
        }}
      />
      {manualImportItem?.titleId ? (
        <ManualImportDialog
          open={manualImportItem !== null}
          onOpenChange={(open) => {
            if (!open) {
              setManualImportItem(null);
            }
          }}
          titleId={manualImportItem.titleId}
          titleName={manualImportItem.titleName}
          clientId={manualImportItem.clientId}
          clientType={manualImportItem.clientType}
          downloadClientItemId={manualImportItem.downloadClientItemId}
          onImportComplete={() => {
            setOptimisticallyRemovedKeys((current) => ({
              ...current,
              [downloadQueueItemIdentityKey(manualImportItem)]: true,
            }));
            decrementImportBadges();
            void refreshVisibleTab();
          }}
        />
      ) : null}
      <AssignTrackedDownloadTitleDialog
        open={assignTitleItem !== null}
        onOpenChange={(open) => {
          if (!open) {
            setAssignTitleItem(null);
          }
        }}
        queueItem={assignTitleItem}
        onAssign={requestAssignTitle}
      />
    </>
  );
});
