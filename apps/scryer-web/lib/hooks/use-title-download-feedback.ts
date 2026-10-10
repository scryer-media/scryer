import * as React from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { useTranslate } from "@/lib/context/translate-context";
import { titleSearchSetupStatusQuery } from "@/lib/graphql/queries";
import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import { useTitleDownloadQueue } from "@/lib/hooks/use-title-download-queue";
import {
  createEmptyTitleOverviewDownloadFeedbackSnapshot,
  fetchTitleOverviewDownloadFeedbackSnapshot,
  type TitleOverviewDownloadFeedbackSnapshot,
} from "@/lib/title-overview-loader";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import { reconcileDownloadQueueItems } from "@/lib/utils/download-queue";
import {
  shouldHandleTitleOverviewActivity,
  TITLE_OVERVIEW_IMPORT_REFRESH_KINDS,
} from "@/lib/utils/title-overview-refresh-policy";

/**
 * Whether any download client is configured, or `null` until that is known.
 * Read again whenever the overview moves to another title. For an overview
 * whose own query does not report it; the series and anime overview reads it
 * from its overview query instead.
 */
export function useDownloadClientsConfigured(titleId: string | null): boolean | null {
  const client = useClient();
  const [answer, setAnswer] = React.useState<{
    titleId: string;
    configured: boolean;
  } | null>(null);

  React.useEffect(() => {
    if (!titleId) {
      return;
    }
    let cancelled = false;
    void client
      .query<{ setupStatus?: { hasDownloadClients?: boolean } | null }>(
        titleSearchSetupStatusQuery,
        {},
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data }) => {
        if (!cancelled) {
          // Only a definite "no" hides download feedback; an unreadable answer
          // behaves like a configured client, as the server decides anyway.
          setAnswer({
            titleId,
            configured: data?.setupStatus?.hasDownloadClients !== false,
          });
        }
      })
      .catch(() => {
        if (!cancelled) {
          setAnswer({ titleId, configured: true });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [client, titleId]);

  return titleId !== null && answer?.titleId === titleId ? answer.configured : null;
}

type FeedbackState = {
  titleId: string;
  seed: DownloadQueueItem[];
  completed: DownloadQueueItem[];
  warning: string | null;
  settled: boolean;
};

const EMPTY_ITEMS: DownloadQueueItem[] = [];

/**
 * A title's download activity: what is downloading now, streamed live, and the
 * finished downloads a manual import can pick up. Both title overviews read it
 * from here and it refreshes itself when an import for the title happens, so
 * they always show the same activity.
 */
export function useTitleDownloadFeedback({
  titleId,
  hasDownloadClients,
}: {
  titleId: string | null;
  /** `null` while that is still being checked. */
  hasDownloadClients: boolean | null;
}): {
  queueItems: DownloadQueueItem[];
  completedDownloads: DownloadQueueItem[];
  refresh: () => Promise<void>;
} {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const { queueTitleOverviewDownloadFeedbackRefresh } = useReactiveRefresh();
  const [state, setState] = React.useState<FeedbackState | null>(null);
  const currentRef = React.useRef({ titleId, hasDownloadClients });
  React.useLayoutEffect(() => {
    currentRef.current = { titleId, hasDownloadClients };
  });
  const lastShownWarningRef = React.useRef<string | null>(null);
  const current = titleId !== null && state?.titleId === titleId ? state : null;

  const apply = React.useCallback(
    (
      requestedTitleId: string,
      snapshot: TitleOverviewDownloadFeedbackSnapshot,
    ) => {
      setState((previous) => {
        const base = previous?.titleId === requestedTitleId ? previous : null;
        return {
          titleId: requestedTitleId,
          seed: reconcileDownloadQueueItems(
            base?.seed ?? [],
            snapshot.downloadQueueItems,
          ),
          completed: reconcileDownloadQueueItems(
            base?.completed ?? [],
            snapshot.completedDownloadQueueItems,
          ),
          warning: snapshot.downloadFeedbackWarning,
          settled: true,
        };
      });
    },
    [],
  );

  const refresh = React.useCallback(async () => {
    const requestedTitleId = currentRef.current.titleId;
    if (!requestedTitleId || currentRef.current.hasDownloadClients !== true) {
      return;
    }
    const stillCurrent = () =>
      currentRef.current.titleId === requestedTitleId &&
      currentRef.current.hasDownloadClients === true;
    try {
      const snapshot = await fetchTitleOverviewDownloadFeedbackSnapshot(
        client,
        requestedTitleId,
      );
      if (stillCurrent()) {
        apply(requestedTitleId, snapshot);
      }
    } catch (error: unknown) {
      if (!stillCurrent()) {
        return;
      }
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"), { level: "ERROR" });
      setState((previous) =>
        previous?.titleId === requestedTitleId
          ? { ...previous, settled: true }
          : {
              titleId: requestedTitleId,
              seed: [],
              completed: [],
              warning: null,
              settled: true,
            },
      );
    }
  }, [apply, client, setGlobalStatus, t]);

  React.useEffect(() => {
    if (!titleId || hasDownloadClients === null) {
      return;
    }
    if (!hasDownloadClients) {
      apply(titleId, createEmptyTitleOverviewDownloadFeedbackSnapshot());
      return;
    }
    void refresh();
  }, [apply, hasDownloadClients, refresh, titleId]);

  const warning = current?.warning ?? null;
  React.useEffect(() => {
    if (warning === null) {
      lastShownWarningRef.current = null;
      return;
    }
    if (lastShownWarningRef.current === warning) {
      return;
    }
    lastShownWarningRef.current = warning;
    setGlobalStatus(warning, { level: "WARNING" });
  }, [setGlobalStatus, warning]);

  useActivityEventStream({
    kinds: TITLE_OVERVIEW_IMPORT_REFRESH_KINDS,
    titleId,
    pause: !titleId || hasDownloadClients !== true,
    onEvent(activity) {
      const requestedTitleId = titleId;
      if (
        !requestedTitleId ||
        !shouldHandleTitleOverviewActivity(requestedTitleId, activity.titleId)
      ) {
        return;
      }
      // Imports arrive in bursts; the shared queue folds them into one fetch.
      queueTitleOverviewDownloadFeedbackRefresh({
        titleId: requestedTitleId,
        apply(snapshot) {
          if (
            currentRef.current.titleId === requestedTitleId &&
            currentRef.current.hasDownloadClients === true
          ) {
            apply(requestedTitleId, snapshot);
          }
        },
        onError(error) {
          console.error("[title-download-feedback] refresh failed:", error);
        },
      });
    },
  });

  const queueItems = useTitleDownloadQueue({
    enabled: titleId !== null && hasDownloadClients === true && current?.settled === true,
    titleId,
    initialItems: current?.seed ?? EMPTY_ITEMS,
  });

  return {
    queueItems: hasDownloadClients === true ? queueItems : EMPTY_ITEMS,
    completedDownloads: current?.completed ?? EMPTY_ITEMS,
    refresh,
  };
}
