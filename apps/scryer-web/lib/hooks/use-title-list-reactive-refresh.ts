import { useEffect, useMemo, useRef } from "react";

import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";

type UseTitleListReactiveRefreshOptions = {
  facet?: string | null;
  pause?: boolean;
  debounceMs?: number;
  onTitleUpdated: (titleId: string) => Promise<void> | void;
};

const TITLE_UPDATED_KIND = "title_updated";

// Canonical reactive bridge for catalog tables. Title-list consumers should
// react to semantic title update events instead of workflow-specific signals.
export function useTitleListReactiveRefresh({
  facet,
  pause = false,
  debounceMs = 300,
  onTitleUpdated,
}: UseTitleListReactiveRefreshOptions) {
  const onTitleUpdatedRef = useRef(onTitleUpdated);
  const refreshTimersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(
    new Map(),
  );

  useEffect(() => {
    onTitleUpdatedRef.current = onTitleUpdated;
  });

  useEffect(
    () => () => {
      for (const timer of refreshTimersRef.current.values()) {
        clearTimeout(timer);
      }
      refreshTimersRef.current.clear();
    },
    [],
  );

  useEffect(() => {
    for (const timer of refreshTimersRef.current.values()) {
      clearTimeout(timer);
    }
    refreshTimersRef.current.clear();
  }, [facet]);

  useEffect(() => {
    if (!pause) {
      return;
    }

    for (const timer of refreshTimersRef.current.values()) {
      clearTimeout(timer);
    }
    refreshTimersRef.current.clear();
  }, [pause]);

  const kinds = useMemo(() => new Set([TITLE_UPDATED_KIND]), []);

  useActivityEventStream({
    kinds,
    facet,
    pause,
    onEvent(activity) {
      const titleId = activity.titleId;
      if (!titleId || refreshTimersRef.current.has(titleId)) {
        return;
      }

      const timer = setTimeout(() => {
        refreshTimersRef.current.delete(titleId);
        void Promise.resolve(onTitleUpdatedRef.current(titleId)).catch(
          (error) => {
            console.error(
              "[title-list-reactive-refresh] refresh failed:",
              error,
            );
          },
        );
      }, debounceMs);

      refreshTimersRef.current.set(titleId, timer);
    },
  });
}
