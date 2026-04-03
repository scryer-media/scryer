import { useEffect, useRef } from "react";

import { settingsChangedSubscription } from "@/lib/graphql/queries";

import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";

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

  useDeferredWsSubscription<{ data?: { settingsChanged?: string[] } }>({
    requestKey: "settingsChanged",
    request: { query: settingsChangedSubscription },
    onNext(result) {
      const keys = result.data?.settingsChanged;
      if (keys?.length) {
        onChangedRef.current(keys);
      }
    },
    onError(err) {
      console.error("[settings-changed] subscription error:", err);
    },
  });
}
