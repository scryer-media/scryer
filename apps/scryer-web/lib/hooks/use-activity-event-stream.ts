import { useEffect, useRef } from "react";

import { activitySubscriptionQuery } from "@/lib/graphql/queries";
import type { ActivityEvent } from "@/lib/types";
import {
  collectActivityEventsFromPayload,
  normalizeActivityEvent,
} from "@/lib/utils/activity";

import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";

type UseActivityEventStreamOptions = {
  kinds?: ReadonlySet<string>;
  titleId?: string | null;
  facet?: string | null;
  pause?: boolean;
  onEvent: (activity: ActivityEvent) => void;
};

export function useActivityEventStream({
  kinds,
  titleId,
  facet,
  pause = false,
  onEvent,
}: UseActivityEventStreamOptions) {
  const onEventRef = useRef(onEvent);
  const kindsRef = useRef(kinds);
  const titleIdRef = useRef(titleId);
  const facetRef = useRef(facet);
  const processedIdsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    onEventRef.current = onEvent;
    kindsRef.current = kinds;
    titleIdRef.current = titleId;
    facetRef.current = facet;
  });

  useEffect(() => {
    processedIdsRef.current.clear();
  }, [facet, titleId]);

  useDeferredWsSubscription<{ data?: { activityEvents?: unknown } }>({
    enabled: !pause,
    requestKey: "activityEvents",
    request: { query: activitySubscriptionQuery },
    onNext(result) {
      const payload = result.data?.activityEvents;
      if (!payload) {
        return;
      }

      const rawEvents = collectActivityEventsFromPayload(payload);
      for (const raw of rawEvents) {
        const activity = normalizeActivityEvent(raw as Partial<ActivityEvent>);
        const filterTitleId = titleIdRef.current;
        if (filterTitleId && activity.titleId !== filterTitleId) {
          continue;
        }

        const filterFacet = facetRef.current;
        if (filterFacet && activity.facet !== filterFacet) {
          continue;
        }

        if (kindsRef.current && !kindsRef.current.has(activity.kind)) {
          continue;
        }

        if (processedIdsRef.current.has(activity.id)) {
          continue;
        }
        processedIdsRef.current.add(activity.id);
        if (processedIdsRef.current.size > 200) {
          const oldestId = processedIdsRef.current.values().next().value;
          if (oldestId) {
            processedIdsRef.current.delete(oldestId);
          }
        }

        onEventRef.current(activity);
      }
    },
    onError(error) {
      console.error("[activity-subscription] error:", error);
    },
    onComplete() {
      processedIdsRef.current.clear();
    },
  });
}
