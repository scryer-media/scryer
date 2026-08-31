import { useCallback, useEffect, useRef, useState } from "react";
import { useClient } from "urql";

import {
  activeImportStreamsQuery,
  activeImportStreamsSyncSubscription,
} from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import type { ActiveImportStream } from "@/lib/types";

const SYNC_DEBOUNCE_MS = 300;

export function useActiveImportStreams(enabled: boolean) {
  const client = useClient();
  const [streams, setStreams] = useState<ActiveImportStream[]>([]);
  const [loading, setLoading] = useState(false);
  const revisionRef = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingSyncRef = useRef(false);

  const refresh = useCallback(async () => {
    if (!enabled) {
      return;
    }
    setLoading(true);
    try {
      const { data, error } = await client.query(activeImportStreamsQuery, {}).toPromise();
      if (error) {
        throw error;
      }
      setStreams(data?.activeImportStreams ?? []);
    } catch (error) {
      console.error("[active-import-streams] refresh failed:", error);
    } finally {
      setLoading(false);
    }
  }, [client, enabled]);

  const scheduleRefresh = useCallback(() => {
    pendingSyncRef.current = true;
    if (!enabled || document.visibilityState !== "visible" || timerRef.current) {
      return;
    }
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      pendingSyncRef.current = false;
      void refresh();
    }, SYNC_DEBOUNCE_MS);
  }, [enabled, refresh]);

  useDeferredWsSubscription<{
    data?: { activeImportStreamsSync?: { revision: number } };
  }>({
    enabled,
    requestKey: "activeImportStreamsSync",
    request: { query: activeImportStreamsSyncSubscription },
    onNext(result) {
      const revision = result.data?.activeImportStreamsSync?.revision;
      if (revision === undefined || revision <= revisionRef.current) {
        return;
      }
      revisionRef.current = revision;
      scheduleRefresh();
    },
    onError(error) {
      console.error("[active-import-streams] sync failed:", error);
    },
  });

  useEffect(() => {
    revisionRef.current = 0;
    pendingSyncRef.current = false;
    if (!enabled) {
      setStreams([]);
      return;
    }
    void refresh();
  }, [enabled, refresh]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    const reconcileOnVisibility = () => {
      if (document.visibilityState === "visible" && pendingSyncRef.current) {
        scheduleRefresh();
      }
    };
    document.addEventListener("visibilitychange", reconcileOnVisibility);
    window.addEventListener("focus", reconcileOnVisibility);
    return () => {
      document.removeEventListener("visibilitychange", reconcileOnVisibility);
      window.removeEventListener("focus", reconcileOnVisibility);
    };
  }, [enabled, scheduleRefresh]);

  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, []);

  return { streams, loading, refresh };
}
