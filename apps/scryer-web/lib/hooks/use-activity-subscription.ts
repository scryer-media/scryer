import { useEffect, useRef } from "react";

import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";

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
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (pause) {
      if (debounceTimer.current) {
        clearTimeout(debounceTimer.current);
        debounceTimer.current = null;
      }
      return;
    }
  }, [debounceMs, pause]);

  useEffect(
    () => () => {
      if (debounceTimer.current) {
        clearTimeout(debounceTimer.current);
        debounceTimer.current = null;
      }
    },
    [],
  );

  useActivityEventStream({
    kinds,
    titleId: options?.titleId,
    facet: options?.facet,
    pause,
    onEvent() {
      if (debounceTimer.current) {
        return;
      }

      debounceTimer.current = setTimeout(() => {
        debounceTimer.current = null;
        onMatchRef.current();
      }, debounceMs);
    },
  });
}
