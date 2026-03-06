
import { memo, useCallback, useEffect, useState } from "react";
import { useClient, useMutation } from "urql";

import { ActivityView } from "@/components/views/activity-view";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  triggerImportMutation,
  pauseDownloadMutation,
  resumeDownloadMutation,
  deleteDownloadMutation,
} from "@/lib/graphql/mutations";
import { importHistoryQuery } from "@/lib/graphql/queries";
import { useDownloadQueue } from "@/lib/hooks/use-download-queue";
import type { DownloadQueueItem, ImportRecord } from "@/lib/types";

const HISTORY_STATES = new Set(["completed", "failed", "import_pending", "importpending"]);
type QueueMode = "scryer" | "all" | "history";

export const ActivityContainer = memo(function ActivityContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [, executeTriggerImport] = useMutation(triggerImportMutation);
  const [, executePauseDownload] = useMutation(pauseDownloadMutation);
  const [, executeResumeDownload] = useMutation(resumeDownloadMutation);
  const [, executeDeleteDownload] = useMutation(deleteDownloadMutation);

  const [queueMode, setQueueMode] = useState<QueueMode>("scryer");

  const { queueItems, queueLoading, queueError, lastRefreshedAt, refreshQueue } = useDownloadQueue({
    includeAllActivity: queueMode !== "scryer",
    includeHistoryOnly: queueMode === "history",
  });

  const [importHistory, setImportHistory] = useState<ImportRecord[]>([]);
  const [importHistoryLoading, setImportHistoryLoading] = useState(false);

  const refreshImportHistory = useCallback(async () => {
    setImportHistoryLoading(true);
    try {
      const { data, error } = await client.query(importHistoryQuery, {
        limit: 50,
      }).toPromise();
      if (error) throw error;
      setImportHistory(data?.importHistory ?? []);
    } catch {
      // silently fail — import history is non-critical
    } finally {
      setImportHistoryLoading(false);
    }
  }, [client]);

  const requestManualImport = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executeTriggerImport({
        input: {
          downloadClientItemId: item.downloadClientItemId,
          titleId: item.titleId,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.manualImportFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.manualImportQueued"));
      await refreshQueue();
    },
    [refreshQueue, executeTriggerImport, setGlobalStatus, t],
  );

  const requestPause = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executePauseDownload({
        input: { downloadClientItemId: item.downloadClientItemId },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.pauseFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.pauseSuccess"));
      await refreshQueue();
    },
    [refreshQueue, executePauseDownload, setGlobalStatus, t],
  );

  const requestResume = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executeResumeDownload({
        input: { downloadClientItemId: item.downloadClientItemId },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.resumeFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.resumeSuccess"));
      await refreshQueue();
    },
    [refreshQueue, executeResumeDownload, setGlobalStatus, t],
  );

  const requestDelete = useCallback(
    async (item: DownloadQueueItem) => {
      const stateNormalized = item.state.trim().toLowerCase();
      const isHistory = HISTORY_STATES.has(stateNormalized);
      const result = await executeDeleteDownload({
        input: {
          downloadClientItemId: item.downloadClientItemId,
          isHistory,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.deleteFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.deleteSuccess"));
      await refreshQueue();
    },
    [refreshQueue, executeDeleteDownload, setGlobalStatus, t],
  );

  useEffect(() => {
    void refreshImportHistory();
  }, [refreshImportHistory]);

  return (
    <ActivityView
      state={{
        queueItems,
        queueLoading,
        queueError,
        lastRefreshedAt,
        requestManualImport,
        requestPause,
        requestResume,
        requestDelete,
        queueMode,
        setQueueMode,
        importHistory,
        importHistoryLoading,
        refreshImportHistory,
      }}
    />
  );
});
