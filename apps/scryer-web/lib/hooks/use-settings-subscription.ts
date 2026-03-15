import { useEffect, useRef } from "react";

import { settingsChangedSubscription } from "@/lib/graphql/queries";
import { wsClient } from "@/lib/graphql/ws-client";

/**
 * Subscribes to settings change notifications via WebSocket.
 * Calls `onChanged` with the list of changed setting key names
 * whenever any client saves admin settings, so consumers can
 * selectively refetch only when their data is affected.
 */
export function useSettingsSubscription(
  onChanged: (changedKeys: string[]) => void,
) {
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
      { query: settingsChangedSubscription },
      {
        next(result: { data?: { settingsChanged?: string[] } }) {
          const keys = result.data?.settingsChanged;
          if (keys?.length) {
            onChangedRef.current(keys);
          }
        },
        error(err: unknown) {
          console.error("[settings-changed] subscription error:", err);
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
