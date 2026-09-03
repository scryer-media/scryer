import { useEffect, useRef } from "react";

import { mediaRequestsChangedSubscription } from "@/lib/graphql/queries";

import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";

export interface MediaRequestsChangedEvent {
  eventId: string;
  eventType: string;
  requestId: string;
  libraryId: string;
}

export function useMediaRequestsSubscription(
  onChanged: (event?: MediaRequestsChangedEvent) => void,
  options?: { pause?: boolean },
) {
  const onChangedRef = useRef(onChanged);
  useEffect(() => {
    onChangedRef.current = onChanged;
  });

  useDeferredWsSubscription<{
    data?: {
      mediaRequestsChanged?: MediaRequestsChangedEvent;
    };
  }>({
    enabled: !(options?.pause ?? false),
    requestKey: "mediaRequestsChanged",
    request: { query: mediaRequestsChangedSubscription },
    onNext(result) {
      onChangedRef.current(result.data?.mediaRequestsChanged);
    },
    onError(err) {
      console.error("[media-requests] subscription error:", err);
    },
  });
}
