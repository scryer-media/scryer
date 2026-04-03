import { useEffect, useRef } from "react";

import { importHistoryChangedSubscription } from "@/lib/graphql/queries";

import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";

/**
 * Subscribes to import history change notifications via WebSocket.
 * Calls `onChanged` whenever an import status update occurs so the
 * consumer can refetch the import history table.
 */
export function useImportHistorySubscription(
  onChanged: () => void,
  options?: { pause?: boolean },
) {
  const onChangedRef = useRef(onChanged);
  useEffect(() => {
    onChangedRef.current = onChanged;
  });

  useDeferredWsSubscription({
    enabled: !(options?.pause ?? false),
    requestKey: "importHistoryChanged",
    request: { query: importHistoryChangedSubscription },
    onNext() {
      onChangedRef.current();
    },
    onError(err) {
      console.error("[import-history] subscription error:", err);
    },
  });
}
