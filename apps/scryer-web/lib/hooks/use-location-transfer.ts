import { useEffect, useRef, useState } from "react";
import { useClient } from "urql";
import { wsClient } from "@/lib/graphql/ws-client";
import {
  locationTransferSummaryQuery,
  locationTransferPageQuery,
  locationTransferSummarySubscription,
  locationTransferPageSubscription,
} from "@/lib/graphql/queries";
import { isTerminalOperationState } from "@/lib/location-operations";
import {
  acceptTransferSnapshot,
  type TransferSnapshot,
  type TransferView,
} from "@/lib/location-transfers";

/** A scope owns its stream, watchdog and sole fallback request. Both transports
 * enter the same version gate; cleanup invalidates even uncancelable requests. */
export function useLocationTransfer(
  operationId: string,
  page: number | null,
  enabled = true,
  refresh = 0,
) {
  const client = useClient();
  const generation = useRef(0);
  const retained = useRef<TransferView | null>(null);
  const pendingQueries = useRef(new Set<string>());
  const [view, setView] = useState<TransferView | null>(null);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [visible, setVisible] = useState(
    () =>
      typeof document === "undefined" || document.visibilityState !== "hidden",
  );
  useEffect(() => {
    const update = () => setVisible(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  useEffect(() => {
    const scope = {
      operationId,
      page,
      requestGeneration: ++generation.current,
    };
    const previous = retained.current;
    let current: TransferView = {
      scope,
      snapshot:
        previous?.scope.operationId === operationId &&
        previous.scope.page === page
          ? previous.snapshot
          : null,
    };
    retained.current = current;
    setView(current);
    setConnected(false);
    setError(null);
    if (!enabled || !visible) return;
    if (
      current.snapshot &&
      isTerminalOperationState(current.snapshot.operation.state)
    )
      return;
    let active = true;
    let terminal = false;
    let inFlight = false;
    let disconnected = false;
    let lastHeartbeat = Date.now();
    let lastPoll = 0;
    let streamGeneration = 0;
    let stopStream: (() => void) | undefined;
    const field =
      page === null ? "locationTransferSummary" : "locationTransferPage";
    const queryScope = JSON.stringify([operationId, page]);
    const variables =
      page === null
        ? { id: operationId }
        : { id: operationId, offset: page * 50 };
    const accept = (snapshot: TransferSnapshot | null | undefined) => {
      if (!active || !snapshot) return;
      const next = acceptTransferSnapshot(current, scope, snapshot);
      if (next === current) return;
      current = next;
      retained.current = next;
      setView(next);
      setError(null);
      terminal = isTerminalOperationState(snapshot.operation.state);
      if (terminal) {
        clearInterval(watchdog);
        stopStream?.();
      }
    };
    const poll = async () => {
      if (
        !active ||
        terminal ||
        inFlight ||
        pendingQueries.current.has(queryScope)
      )
        return;
      pendingQueries.current.add(queryScope);
      inFlight = true;
      lastPoll = Date.now();
      try {
        const result = await client
          .query(
            page === null
              ? locationTransferSummaryQuery
              : locationTransferPageQuery,
            variables,
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (!active) return;
        if (result.error) throw result.error;
        if (!result.data?.[field]) setError("missing");
        accept(result.data?.[field]);
      } catch {
        if (active) setError("load");
      } finally {
        inFlight = false;
        pendingQueries.current.delete(queryScope);
      }
    };
    const connect = () => {
      const attempt = ++streamGeneration;
      stopStream?.();
      stopStream = wsClient.subscribe(
        {
          query:
            page === null
              ? locationTransferSummarySubscription
              : locationTransferPageSubscription,
          variables,
        },
        {
          next(result) {
            if (!active || attempt !== streamGeneration) return;
            if (result.errors?.length) {
              disconnected = true;
              setConnected(false);
              void poll();
              return;
            }
            lastHeartbeat = Date.now();
            disconnected = false;
            setConnected(true);
            accept(result.data?.[field] as TransferSnapshot | undefined);
          },
          error() {
            if (active && !terminal && attempt === streamGeneration) {
              disconnected = true;
              setConnected(false);
              void poll();
            }
          },
          complete() {
            if (active && !terminal && attempt === streamGeneration) {
              disconnected = true;
              setConnected(false);
              void poll();
            }
          },
        },
      );
    };
    // The stream provides initial synchronization. Poll only if it cannot.
    const watchdog = setInterval(() => {
      if (terminal) return;
      const stale = Date.now() - lastHeartbeat >= 45_000;
      if (disconnected || stale) {
        setConnected(false);
        if (Date.now() - lastPoll >= 15_000) {
          void poll();
          connect();
        }
      }
    }, 1000);
    connect();
    return () => {
      active = false;
      clearInterval(watchdog);
      stopStream?.();
    };
  }, [client, operationId, page, enabled, visible, refresh]);
  return {
    snapshot:
      view?.scope.operationId === operationId && view.scope.page === page
        ? view.snapshot
        : null,
    connected,
    error,
  };
}
