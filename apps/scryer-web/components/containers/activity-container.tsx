
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
  cancelActiveImportMutation,
  queueManualImportMutation,
  pauseDownloadMutation,
  resumeDownloadMutation,
  deleteDownloadMutation,
} from "@/lib/graphql/mutations";
import { downloadClientsQuery } from "@/lib/graphql/queries";
import { useDownloadImport } from "@/lib/hooks/use-download-import";
import { useActiveImportStreams } from "@/lib/hooks/use-active-import-streams";
import { useDownloadQueuePage } from "@/lib/hooks/use-download-queue-page";
import { useImportHistorySubscription } from "@/lib/hooks/use-import-history-subscription";
import { dispatchNavigationBadgesRefresh } from "@/lib/events/navigation-badges";
import type { ActivitySection } from "@/components/root/types";
import type {
  DownloadClientRecord,
  DownloadActivityStatus,
  ActiveImportStream,
  DownloadClientFilterOption,
  DownloadImportStatus,
  DownloadQueueItem,
  SortConfig,
} from "@/lib/types";
import {
  downloadQueueItemIdentityKey,
  IMPORT_ATTENTION_STATUSES,
  isHistoryQueueState,
  matchesImportStatuses,
} from "@/lib/utils/download-queue";
import {
  type DirectMovieManualImportCandidate,
  directMovieManualImportMappings,
} from "@/lib/utils/manual-import-actions";

type ActivityTab = Exclude<ActivitySection, "history">;
type SortConfigByTab = Record<ActivityTab, SortConfig>;

const ACTIVITY_STATUS_OPTIONS: DownloadActivityStatus[] = [
  "DOWNLOADING",
  "QUEUED",
  "PAUSED",
  "POST_PROCESSING",
  "SEEDING",
  "WARNING",
];
const ACTIVITY_QUEUE_SORT: SortConfig = { key: "PROGRESS", direction: "DESC" };
const DEFAULT_SORT_CONFIG_BY_TAB: SortConfigByTab = {
  import: { key: "STATUS", direction: "ASC" },
  activity: ACTIVITY_QUEUE_SORT,
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
  activitySection: ActivityTab;
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
  const [, executeCancelActiveImport] = useMutation(cancelActiveImportMutation);

  const activeTab = activitySection;
  const importTabActive = activeTab === "import";
  const activityTabActive = activeTab === "activity";
  const { streams: activeImportStreams } = useActiveImportStreams(activityTabActive);
  const [selectedImportStatuses, setSelectedImportStatuses] = useState<DownloadImportStatus[]>([
    ...IMPORT_ATTENTION_STATUSES,
  ]);
  const [selectedActivityStatuses, setSelectedActivityStatuses] = useState<
    DownloadActivityStatus[]
  >([...ACTIVITY_STATUS_OPTIONS]);
  const [activityScryerSubmittedOnly, setActivityScryerSubmittedOnly] = useState(true);
  const [selectedActivityClientIds, setSelectedActivityClientIds] = useState<string[] | null>(
    null,
  );
  const [sortConfigByTab, setSortConfigByTab] =
    useState<SortConfigByTab>(DEFAULT_SORT_CONFIG_BY_TAB);
  const [configuredClientOptions, setConfiguredClientOptions] = useState<
    DownloadClientFilterOption[]
  >([]);
  const [manualImportItem, setManualImportItem] = useState<DownloadQueueItem | null>(null);
  const [assignTitleItem, setAssignTitleItem] = useState<DownloadQueueItem | null>(null);
  const [optimisticallyRemovedKeys, setOptimisticallyRemovedKeys] = useState<
    Record<string, true>
  >({});
  const [optimisticQueueStates, setOptimisticQueueStates] = useState<
    Record<string, Pick<DownloadQueueItem, "state" | "displayState">>
  >({});

  const {
    queueItems: activityQueueItems,
    queueLoading,
    queueLoadingMore,
    queueError,
    queueHasMore,
    queueAvailableClients,
    queueStale,
    lastRefreshedAt: queueLastRefreshedAt,
    refreshQueue,
    loadMoreQueue,
    setVisibleQueueOffset,
  } = useDownloadQueuePage({
    enabled: activityTabActive,
    filters: selectedActivityStatuses,
    clientIds: selectedActivityClientIds,
    scryerSubmittedOnly: activityScryerSubmittedOnly,
    sort: ACTIVITY_QUEUE_SORT,
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
    filter: "ATTENTION",
  });
  const filteredImportItems = useMemo(() => {
    return importItems.filter((item) => matchesImportStatuses(item, selectedImportStatuses));
  }, [importItems, selectedImportStatuses]);
  const activityAvailableClients = useMemo<DownloadClientFilterOption[]>(() => {
    return mergeDownloadClientFilterOptions(
      configuredClientOptions,
      queueAvailableClients,
    );
  }, [configuredClientOptions, queueAvailableClients]);
  const visibleItems = useMemo(() => {
    const sourceItems =
      activeTab === "import"
        ? filteredImportItems
        : activityQueueItems;
    return sourceItems
      .filter((item) => !optimisticallyRemovedKeys[downloadQueueItemIdentityKey(item)])
      .map((item) => {
        const optimistic = optimisticQueueStates[downloadQueueItemIdentityKey(item)];
        return optimistic ? { ...item, ...optimistic } : item;
      });
  }, [
    activeTab,
    activityQueueItems,
    filteredImportItems,
    optimisticQueueStates,
    optimisticallyRemovedKeys,
  ]);
  const initialImportLoading =
    importLoading && filteredImportItems.length === 0 && importLastRefreshedAt === null;
  const initialActivityLoading =
    queueLoading && activityQueueItems.length === 0 && queueLastRefreshedAt === null;
  const visibleLoading =
    activeTab === "import"
      ? initialImportLoading
      : initialActivityLoading;
  const visibleLoadingMore =
    activeTab === "import" ? importLoadingMore : activeTab === "activity" ? queueLoadingMore : false;
  const visibleHasMore =
    activeTab === "import" ? importHasMore : activeTab === "activity" ? queueHasMore : false;
  const visibleError =
    activeTab === "import"
      ? importError
      : queueError;

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
    if (!activityTabActive) {
      return;
    }
    void refreshConfiguredClients();
  }, [activityTabActive, refreshConfiguredClients]);

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

  const refreshVisibleTab = useCallback(async () => {
    switch (activeTab) {
      case "activity":
        await Promise.all([refreshQueue(), refreshConfiguredClients()]);
        break;
      case "import":
      default:
        await refreshImport();
        break;
    }
  }, [activeTab, refreshConfiguredClients, refreshImport, refreshQueue]);

  const refreshImportDrivenViews = useCallback(async () => {
    if (importTabActive) {
      await refreshImport();
      return;
    }

  }, [importTabActive, refreshImport]);

  useImportHistorySubscription(() => {
    void refreshImportDrivenViews();
  }, { pause: activityTabActive });

  useEffect(() => {
    if (Object.keys(optimisticallyRemovedKeys).length === 0) {
      return;
    }

    const authoritativeItems = [...activityQueueItems, ...importItems];
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
  }, [activityQueueItems, importItems, optimisticallyRemovedKeys]);

  useEffect(() => {
    if (Object.keys(optimisticQueueStates).length === 0) {
      return;
    }
    const authoritativeByKey = new Map(
      [...activityQueueItems, ...importItems].map((item) => [
        downloadQueueItemIdentityKey(item),
        item,
      ]),
    );
    setOptimisticQueueStates((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(([key, optimistic]) => {
          const authoritative = authoritativeByKey.get(key);
          if (!authoritative) {
            return false;
          }
          return (
            (authoritative.state !== optimistic.state ||
              authoritative.displayState !== optimistic.displayState)
          );
        }),
      );
      return Object.keys(next).length === Object.keys(current).length ? current : next;
    });
  }, [activityQueueItems, importItems, optimisticQueueStates]);

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
      // selection first and map its primary candidate.
      //
      // Movies carry no episode or series-movie target, so the mapping is just
      // a candidateId — both target fields on ManualImportCandidateMappingInput
      // are optional. A movie import lands exactly one file, so only the
      // largest candidate is mapped; the server independently picks the primary
      // among whatever is mapped and skips the rest. Series and anime never
      // reach here; they open the dialog above so the user can map files to
      // episodes.
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

      let preview = selection.data?.beginManualImportSelection;
      if (preview?.archiveExtractionNeeded) {
        const extracted = await executeBeginManualImportSelection({
          input: {
            clientId: item.clientId,
            clientType: item.clientType,
            downloadClientItemId: item.downloadClientItemId,
            titleId: item.titleId,
            extractArchives: true,
          },
        });
        if (extracted.error) {
          const message = extracted.error.message ?? t("queue.manualImportFailed");
          setGlobalStatus(message);
          throw extracted.error;
        }
        preview = extracted.data?.beginManualImportSelection;
      }
      const candidates: DirectMovieManualImportCandidate[] = preview?.files ?? [];
      const files = directMovieManualImportMappings(candidates);
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
      const itemKey = downloadQueueItemIdentityKey(item);
      setOptimisticQueueStates((current) => ({
        ...current,
        [itemKey]: { state: "PAUSED", displayState: "PAUSED" },
      }));
      const result = await executePauseDownload({
        input: {
          clientId: item.clientId,
          downloadClientItemId: item.downloadClientItemId,
        },
      });
      if (result.error) {
        setOptimisticQueueStates((current) => {
          const { [itemKey]: _removed, ...next } = current;
          return next;
        });
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
      const itemKey = downloadQueueItemIdentityKey(item);
      setOptimisticQueueStates((current) => ({
        ...current,
        [itemKey]: { state: "DOWNLOADING", displayState: "DOWNLOADING" },
      }));
      const result = await executeResumeDownload({
        input: {
          clientId: item.clientId,
          downloadClientItemId: item.downloadClientItemId,
        },
      });
      if (result.error) {
        setOptimisticQueueStates((current) => {
          const { [itemKey]: _removed, ...next } = current;
          return next;
        });
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
          isHistory: isHistoryQueueState(item.state),
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
      if (matchesImportStatuses(item, IMPORT_ATTENTION_STATUSES)) {
        decrementImportBadges();
      }
      setGlobalStatus(t("queue.deleteQueued"));
      void refreshQueue();
      void refreshImport();
    },
    [
      decrementImportBadges,
      executeDeleteDownload,
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
            isHistory: isHistoryQueueState(item.state),
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
          matchesImportStatuses(item, IMPORT_ATTENTION_STATUSES),
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
    },
    [
      client,
      decrementImportBadges,
      refreshImport,
      refreshQueue,
      setGlobalStatus,
      t,
    ],
  );

  const requestCancelActiveImport = useCallback(
    async (stream: ActiveImportStream) => {
      const result = await executeCancelActiveImport({ streamId: stream.id });
      if (result.error) {
        const message = result.error.message ?? "Unable to cancel import.";
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus("Import cancellation requested.");
    },
    [executeCancelActiveImport, setGlobalStatus],
  );

  return (
    <>
      <ActivityView
        state={{
          queueItems: visibleItems,
          queueLoading: visibleLoading,
          queueLoadingMore: visibleLoadingMore,
          queueError: visibleError,
          queueStale: activeTab === "activity" && queueStale,
          activeImportStreams,
          onVisibleQueueOffsetChange:
            activeTab === "activity" ? setVisibleQueueOffset : undefined,
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
          requestCancelActiveImport,
          activeTab,
          sortConfigByTab,
          toggleSort: (tab, nextKey) => {
            if (tab === "activity") {
              return;
            }
            setSortConfigByTab((current) => {
              const currentConfig = current[tab];
              return {
                ...current,
                [tab]:
                  currentConfig.key === nextKey
                    ? {
                        key: nextKey,
                        direction: currentConfig.direction === "ASC" ? "DESC" : "ASC",
                      }
                    : DEFAULT_SORT_CONFIG_BY_TAB[tab].key === nextKey
                      ? DEFAULT_SORT_CONFIG_BY_TAB[tab]
                      : { key: nextKey, direction: "ASC" },
              };
            });
          },
          activityScryerSubmittedOnly,
          toggleActivityScryerSubmittedOnly: () => {
            setActivityScryerSubmittedOnly((current) => !current);
          },
          selectedImportStatuses,
          toggleImportStatus: (status) => {
            setSelectedImportStatuses((current) => toggleSelectedValue(current, status));
          },
          selectedActivityStatuses,
          toggleActivityStatus: (status) => {
            setSelectedActivityStatuses((current) => toggleSelectedValue(current, status));
          },
          activityAvailableClients,
          selectedActivityClientIds:
            selectedActivityClientIds ?? activityAvailableClients.map((client) => client.clientId),
          toggleActivityClientId: (clientId) => {
            setSelectedActivityClientIds((current) =>
              toggleSelectedValue(current ?? activityAvailableClients.map((client) => client.clientId), clientId),
            );
          },
          visibleHasMore,
          requestMoreItems:
            activeTab === "import"
              ? loadMoreImport
              : loadMoreQueue,
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
          onImportQueued={() => {
            setOptimisticQueueStates((current) => ({
              ...current,
              [downloadQueueItemIdentityKey(manualImportItem)]: {
                state: manualImportItem.state,
                displayState: "IMPORT_PENDING",
              },
            }));
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
