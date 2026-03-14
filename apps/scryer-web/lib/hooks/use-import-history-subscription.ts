import { useEffect, useRef } from "react";

import { importHistoryChangedSubscription } from "@/lib/graphql/queries";
import { wsClient } from "@/lib/graphql/ws-client";

/**
 * Subscribes to import history change notifications via WebSocket.
 * Calls `onChanged` whenever an import status update occurs so the
 * consumer can refetch the import history table.
 */
export function useImportHistorySubscription(onChanged: () => void) {
  const onChangedRef = useRef(onChanged);
  useEffect(() => {
    onChangedRef.current = onChanged;
  });

  const unsubRef = useRef<(() => void) | null>(null);
  const teardownTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    // StrictMode re-run: cancel the pending teardown, subscription is still alive
    if (teardownTimer.current) {
      clearTimeout(teardownTimer.current);
      teardownTimer.current = null;
      return;
    }

    const unsubscribe = wsClient.subscribe(
      { query: importHistoryChangedSubscription },
      {
        next() {
          onChangedRef.current();
        },
        error(err) {
          console.error("[import-history] subscription error:", err);
        },
        complete() {
          unsubRef.current = null;
        },
      },
    );

    unsubRef.current = unsubscribe;

    return () => {
      teardownTimer.current = setTimeout(() => {
        teardownTimer.current = null;
        unsubscribe();
        unsubRef.current = null;
      }, 200);
    };
  }, []);
}
