import { useEffect, useMemo, useRef } from "react";

import { wsClient } from "@/lib/graphql/ws-client";

type WsSubscriptionRequest = Parameters<typeof wsClient.subscribe>[0];

type UseDeferredWsSubscriptionOptions<TResult> = {
  enabled?: boolean;
  request: WsSubscriptionRequest;
  requestKey?: string;
  onStart?: () => void;
  onNext: (result: TResult) => void;
  onError?: (error: unknown) => void;
  onComplete?: () => void;
  teardownDelayMs?: number;
};

function buildRequestSignature({
  operationName,
  queryText,
  variables,
}: {
  operationName?: string | null;
  queryText: string;
  variables?: unknown;
}): string {
  return JSON.stringify({
    operationName: operationName ?? null,
    query: queryText,
    variables: variables ?? null,
  });
}

export function useDeferredWsSubscription<TResult>({
  enabled = true,
  request,
  requestKey,
  onStart,
  onNext,
  onError,
  onComplete,
  teardownDelayMs = 200,
}: UseDeferredWsSubscriptionOptions<TResult>) {
  const onStartRef = useRef(onStart);
  const onNextRef = useRef(onNext);
  const onErrorRef = useRef(onError);
  const onCompleteRef = useRef(onComplete);
  const requestRef = useRef(request);
  const unsubscribeRef = useRef<(() => void) | null>(null);
  const teardownTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeRequestSignatureRef = useRef<string | null>(null);
  const requestOperation = request as {
    operationName?: string;
    query?: unknown;
    variables?: unknown;
  };
  const queryText =
    typeof requestOperation.query === "string"
      ? requestOperation.query
      : String(requestOperation.query ?? "");

  useEffect(() => {
    requestRef.current = request;
    onStartRef.current = onStart;
    onNextRef.current = onNext;
    onErrorRef.current = onError;
    onCompleteRef.current = onComplete;
  });

  const requestSignature = useMemo(() => {
    if (requestKey) {
      return requestKey;
    }

    return buildRequestSignature({
      operationName: requestOperation.operationName ?? null,
      queryText,
      variables: requestOperation.variables,
    });
  }, [requestKey, requestOperation.operationName, requestOperation.variables, queryText]);

  useEffect(() => {
    if (!enabled) {
      if (teardownTimerRef.current) {
        clearTimeout(teardownTimerRef.current);
        teardownTimerRef.current = null;
      }
      if (unsubscribeRef.current) {
        unsubscribeRef.current();
        unsubscribeRef.current = null;
      }
      activeRequestSignatureRef.current = null;
      return;
    }

    if (teardownTimerRef.current && unsubscribeRef.current) {
      if (activeRequestSignatureRef.current === requestSignature) {
        clearTimeout(teardownTimerRef.current);
        teardownTimerRef.current = null;
        return;
      }

      clearTimeout(teardownTimerRef.current);
      teardownTimerRef.current = null;
      unsubscribeRef.current();
      unsubscribeRef.current = null;
      activeRequestSignatureRef.current = null;
    }

    const unsubscribe = wsClient.subscribe(requestRef.current, {
      next(result) {
        onNextRef.current(result as TResult);
      },
      error(error) {
        onErrorRef.current?.(error);
      },
      complete() {
        if (unsubscribeRef.current === unsubscribe) {
          unsubscribeRef.current = null;
          activeRequestSignatureRef.current = null;
        }
        onCompleteRef.current?.();
      },
    });

    unsubscribeRef.current = unsubscribe;
    activeRequestSignatureRef.current = requestSignature;
    onStartRef.current?.();

    return () => {
      teardownTimerRef.current = setTimeout(() => {
        teardownTimerRef.current = null;
        unsubscribe();
        if (unsubscribeRef.current === unsubscribe) {
          unsubscribeRef.current = null;
          activeRequestSignatureRef.current = null;
        }
      }, teardownDelayMs);
    };
  }, [enabled, requestSignature, teardownDelayMs]);
}
