import { useEffect, useMemo, useRef } from "react";

import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import { useImportHistorySubscription } from "@/lib/hooks/use-import-history-subscription";

type UseTitleOverviewReactiveRefreshOptions = {
  titleId?: string | null;
  refresh: () => Promise<void> | void;
  importKinds: ReadonlySet<string>;
  pause?: boolean;
  debounceMs?: number;
  onHydrationStarted?: () => void;
  onHydrationCompleted?: () => void;
  onHydrationFailed?: () => void;
};

const HYDRATION_STARTED_KIND = "metadata_hydration_started";
const HYDRATION_COMPLETED_KIND = "metadata_hydration_completed";
const HYDRATION_FAILED_KIND = "metadata_hydration_failed";

export function useTitleOverviewReactiveRefresh({
  titleId,
  refresh,
  importKinds,
  pause = false,
  debounceMs = 500,
  onHydrationStarted,
  onHydrationCompleted,
  onHydrationFailed,
}: UseTitleOverviewReactiveRefreshOptions) {
  const refreshRef = useRef(refresh);
  const onHydrationStartedRef = useRef(onHydrationStarted);
  const onHydrationCompletedRef = useRef(onHydrationCompleted);
  const onHydrationFailedRef = useRef(onHydrationFailed);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    refreshRef.current = refresh;
    onHydrationStartedRef.current = onHydrationStarted;
    onHydrationCompletedRef.current = onHydrationCompleted;
    onHydrationFailedRef.current = onHydrationFailed;
  });

  useEffect(
    () => () => {
      if (refreshTimerRef.current) {
        clearTimeout(refreshTimerRef.current);
        refreshTimerRef.current = null;
      }
    },
    [],
  );

  const scheduleRefresh = () => {
    if (refreshTimerRef.current) {
      return;
    }

    refreshTimerRef.current = setTimeout(() => {
      refreshTimerRef.current = null;
      void Promise.resolve(refreshRef.current()).catch((error) => {
        console.error("[title-overview-reactive-refresh] refresh failed:", error);
      });
    }, debounceMs);
  };

  const activityKinds = useMemo(
    () =>
      new Set([
        ...importKinds,
        HYDRATION_STARTED_KIND,
        HYDRATION_COMPLETED_KIND,
        HYDRATION_FAILED_KIND,
      ]),
    [importKinds],
  );

  useActivityEventStream({
    kinds: activityKinds,
    titleId,
    pause,
    onEvent(activity) {
      switch (activity.kind) {
        case HYDRATION_STARTED_KIND:
          onHydrationStartedRef.current?.();
          return;
        case HYDRATION_COMPLETED_KIND:
          onHydrationCompletedRef.current?.();
          scheduleRefresh();
          return;
        case HYDRATION_FAILED_KIND:
          onHydrationFailedRef.current?.();
          return;
        default:
          scheduleRefresh();
      }
    },
  });

  useImportHistorySubscription(scheduleRefresh, { pause });
}
