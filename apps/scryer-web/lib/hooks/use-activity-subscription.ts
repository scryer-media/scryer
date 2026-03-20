import { useEffect, useRef } from "react";

import { activitySubscriptionQuery } from "@/lib/graphql/queries";
import { wsClient } from "@/lib/graphql/ws-client";
import {
  collectActivityEventsFromPayload,
  normalizeActivityEvent,
} from "@/lib/utils/activity";
import type { ActivityEvent } from "@/lib/types";

/**
 * Subscribes to activity events via WebSocket and calls `onMatch`
 * when any event matches one of the given `kinds`. Debounces rapid
 * bursts (e.g. bulk hydration) so the callback fires at most once
 * per `debounceMs` window.
 */
export function useActivitySubscription(
  kinds: ReadonlySet<string>,
  onMatch: () => void,
  options?: {
    debounceMs?: number;
    titleId?: string | null;
    facet?: string | null;
    pause?: boolean;
  },
) {
  const onMatchRef = useRef(onMatch);
  const kindsRef = useRef(kinds);
  const titleIdRef = useRef(options?.titleId);
  const facetRef = useRef(options?.facet);
  useEffect(() => {
    onMatchRef.current = onMatch;
    kindsRef.current = kinds;
    titleIdRef.current = options?.titleId;
    facetRef.current = options?.facet;
  });

  const debounceMs = options?.debounceMs ?? 500;
  const pause = options?.pause ?? false;

  const unsubRef = useRef<(() => void) | null>(null);
  const teardownTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const processedIds = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (pause) {
      // Tear down any existing subscription when paused.
      if (teardownTimer.current) {
        clearTimeout(teardownTimer.current);
        teardownTimer.current = null;
      }
      if (debounceTimer.current) {
        clearTimeout(debounceTimer.current);
        debounceTimer.current = null;
      }
      if (unsubRef.current) {
        unsubRef.current();
        unsubRef.current = null;
      }
      return;
    }

    // StrictMode re-run: cancel the pending teardown, subscription is still alive
    if (teardownTimer.current) {
      clearTimeout(teardownTimer.current);
      teardownTimer.current = null;
      return;
    }

    const unsubscribe = wsClient.subscribe(
      { query: activitySubscriptionQuery },
      {
        next(result: { data?: { activityEvents?: unknown } }) {
          const payload = result.data?.activityEvents;
          if (!payload) return;

          const rawEvents = collectActivityEventsFromPayload(payload);
          let matched = false;

          for (const raw of rawEvents) {
            const activity = normalizeActivityEvent(
              raw as Partial<ActivityEvent>,
            );
            if (processedIds.current.has(activity.id)) continue;
            processedIds.current.add(activity.id);
            if (processedIds.current.size > 200) {
              const oldest = processedIds.current.values().next().value;
              if (oldest) processedIds.current.delete(oldest);
            }

            const filterTitleId = titleIdRef.current;
            if (filterTitleId && activity.titleId !== filterTitleId) continue;

            const filterFacet = facetRef.current;
            if (filterFacet && activity.facet !== filterFacet) continue;

            if (kindsRef.current.has(activity.kind)) {
              matched = true;
            }
          }

          if (matched && !debounceTimer.current) {
            debounceTimer.current = setTimeout(() => {
              debounceTimer.current = null;
              onMatchRef.current();
            }, debounceMs);
          }
        },
        error(err: unknown) {
          console.error("[activity-subscription] error:", err);
        },
        complete() {
          unsubRef.current = null;
        },
      },
    );

    unsubRef.current = unsubscribe;

    const ids = processedIds.current;
    return () => {
      teardownTimer.current = setTimeout(() => {
        teardownTimer.current = null;
        unsubscribe();
        unsubRef.current = null;
        if (debounceTimer.current) {
          clearTimeout(debounceTimer.current);
          debounceTimer.current = null;
        }
        ids.clear();
      }, 200);
    };
  }, [debounceMs, pause]);
}
