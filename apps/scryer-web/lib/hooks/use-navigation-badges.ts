import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { backendClient } from "@/lib/graphql/urql-client";
import {
  navigationBadgeCountsQuery,
  scryerVersionQuery,
} from "@/lib/graphql/queries";
import { useMediaRequestsSubscription } from "@/lib/hooks/use-media-requests-subscription";
import { scheduleAfterFirstPaint } from "@/lib/utils/scheduling";
import {
  dispatchNavigationBadgesRefresh,
  NAVIGATION_BADGES_REFRESH_EVENT,
  type NavigationBadgesRefreshDetail,
} from "@/lib/events/navigation-badges";
import type { PendingImportCounts } from "@/lib/types";
import { boundedQuery } from "@/lib/graphql/bounded-query";
import { useQueryAuthScope } from "@/lib/hooks/use-query-auth-scope";
import { getAuthToken } from "@/lib/hooks/use-auth";

type NavigationBadgeCountsPayload = {
  pendingImportCounts?: PendingImportCounts | null;
  pendingMediaRequestCounts?: PendingImportCounts | null;
  activityImportCount?: number | null;
  pluginUpdateCount?: number | null;
  pluginBlockedCount?: number | null;
};

const EMPTY_PENDING_IMPORT_COUNTS: PendingImportCounts = {
  movie: 0,
  series: 0,
  anime: 0,
};

function samePendingImportCounts(
  current: PendingImportCounts | null,
  next: PendingImportCounts,
) {
  return (
    current !== null &&
    current.movie === next.movie &&
    current.series === next.series &&
    current.anime === next.anime
  );
}

const BADGE_REFRESH_COALESCE_MS = 2_000;

export function useNavigationBadges({
  serviceRestarting,
  canManageTitle,
  canRequestMedia,
}: {
  serviceRestarting: boolean;
  canManageTitle: boolean;
  canRequestMedia: boolean;
}) {
  const authScope = useQueryAuthScope();
  const scope = JSON.stringify([authScope, canManageTitle, canRequestMedia]);
  const scopeRef = useRef(scope);
  useLayoutEffect(() => { scopeRef.current = scope; }, [scope]);
  const activeRequestRef = useRef<AbortController | null>(null);
  const epochRef = useRef(0);
  const pendingRef = useRef(false);
  const [loadedAuth, setLoadedAuth] = useState<string | null>(null);
  const [manualImportCountStale, setManualImportCountStale] = useState(false);
  const [pendingImportCounts, setPendingImportCounts] =
    useState<PendingImportCounts | null>(null);
  const [pendingMediaRequestCounts, setPendingMediaRequestCounts] =
    useState<PendingImportCounts | null>(null);
  const [manualImportRequiredCount, setManualImportRequiredCount] = useState(0);
  const [pluginUpdateCount, setPluginUpdateCount] = useState(0);
  const [pluginBlockedCount, setPluginBlockedCount] = useState(0);
  const [scryerVersion, setScryerVersion] = useState<string | null>(null);

  const refreshScryerVersion = useCallback(async () => {
    try {
      const { data, error } = await backendClient
        .query<{ scryerVersion?: string | null }>(scryerVersionQuery, {})
        .toPromise();
      if (error) {
        throw error;
      }
      setScryerVersion(data?.scryerVersion ?? null);
    } catch (error) {
      console.warn("Failed to refresh Scryer version", error);
    }
  }, []);

  useEffect(() => {
    if (!serviceRestarting) {
      return scheduleAfterFirstPaint(() => {
        void refreshScryerVersion();
      });
    }
  }, [refreshScryerVersion, serviceRestarting]);

  const lastBadgeRefreshStartedAtRef = useRef(0);

  // A request action reaches this hook twice: the explicit dispatch from the
  // page that ran the mutation and the subscription echo, in either order and
  // up to a second apart. Triggers inside the window ride the earlier fetch;
  // the delayed delta confirmation forces through.
  const refreshNavigationBadges = useCallback(async (options?: { force?: boolean }) => {
    if (activeRequestRef.current) {
      pendingRef.current = true;
      return;
    }
    const now = Date.now();
    if (
      !options?.force &&
      now - lastBadgeRefreshStartedAtRef.current < BADGE_REFRESH_COALESCE_MS
    ) {
      return;
    }
    lastBadgeRefreshStartedAtRef.current = now;
    const epoch = epochRef.current;
    const current = () => epochRef.current === epoch && getAuthToken() === authScope && scopeRef.current === scope;
    do {
    pendingRef.current = false;
    const controller = new AbortController();
    activeRequestRef.current = controller;
    let succeeded = false;
    try {
      const badgeCountsResult = await boundedQuery(backendClient
        .query(navigationBadgeCountsQuery, {}), controller.signal);
      if (!current()) return;

      if (badgeCountsResult.error) {
        throw badgeCountsResult.error;
      }

      const badgeCounts = badgeCountsResult.data?.navigationBadgeCounts as
        | NavigationBadgeCountsPayload
        | undefined;
      if (!badgeCounts || !Number.isSafeInteger(badgeCounts.activityImportCount) ||
          (badgeCounts.activityImportCount ?? -1) < 0) {
        throw new Error("Invalid navigation badge response.");
      }
      const nextPendingImportCounts =
        badgeCounts?.pendingImportCounts ?? EMPTY_PENDING_IMPORT_COUNTS;
      const nextPendingMediaRequestCounts =
        badgeCounts?.pendingMediaRequestCounts ?? EMPTY_PENDING_IMPORT_COUNTS;
      setPendingImportCounts((current) =>
        samePendingImportCounts(current, nextPendingImportCounts)
          ? current
          : nextPendingImportCounts,
      );
      setPendingMediaRequestCounts((current) =>
        samePendingImportCounts(current, nextPendingMediaRequestCounts)
          ? current
          : nextPendingMediaRequestCounts,
      );
      setManualImportRequiredCount(
        Number(badgeCounts?.activityImportCount ?? 0),
      );
      setPluginUpdateCount(Number(badgeCounts?.pluginUpdateCount ?? 0));
      setPluginBlockedCount(Number(badgeCounts?.pluginBlockedCount ?? 0));
      setLoadedAuth(scope);
      setManualImportCountStale(false);
      succeeded = true;
    } catch (error) {
      if (!current()) return;
      setManualImportCountStale(true);
      console.warn("Failed to refresh navigation badges", error);
    } finally {
      if (current()) activeRequestRef.current = null;
    }
    if (!current() || !succeeded) break;
    } while (pendingRef.current);
  }, [authScope, scope]);

  useEffect(() => {
    epochRef.current += 1;
    lastBadgeRefreshStartedAtRef.current = 0;
    setLoadedAuth(null);
    setPendingImportCounts(null);
    setPendingMediaRequestCounts(null);
    setManualImportRequiredCount(0);
    setManualImportCountStale(false);
    const cancelScheduled = scheduleAfterFirstPaint(() => {
      void refreshNavigationBadges();
    });
    return () => {
      epochRef.current += 1;
      activeRequestRef.current?.abort();
      activeRequestRef.current = null;
      pendingRef.current = false;
      cancelScheduled();
    };
  }, [refreshNavigationBadges]);

  useEffect(() => {
    let confirmationTimer: ReturnType<typeof setTimeout> | undefined;
    const refreshFromPulse = () => {
      dispatchNavigationBadgesRefresh({ source: "poll" });
    };
    const refreshFromFocus = () => {
      dispatchNavigationBadgesRefresh({ source: "focus" });
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        refreshFromFocus();
      }
    };
    const handleNavigationBadgeRefresh = (event: Event) => {
      const delta =
        event instanceof CustomEvent &&
        typeof (event as CustomEvent<NavigationBadgesRefreshDetail>).detail
          ?.delta === "number"
          ? Number(
              (event as CustomEvent<NavigationBadgesRefreshDetail>).detail
                ?.delta,
            )
          : 0;
      if (delta !== 0) {
        clearTimeout(confirmationTimer);
        confirmationTimer = setTimeout(() => {
          void refreshNavigationBadges({ force: true });
        }, 2_000);
        return;
      }
      void refreshNavigationBadges();
    };
    window.addEventListener(
      NAVIGATION_BADGES_REFRESH_EVENT,
      handleNavigationBadgeRefresh,
    );
    window.addEventListener("focus", refreshFromFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    const intervalId = window.setInterval(() => {
      refreshFromPulse();
    }, 30_000);
    return () => {
      window.removeEventListener(
        NAVIGATION_BADGES_REFRESH_EVENT,
        handleNavigationBadgeRefresh,
      );
      window.removeEventListener("focus", refreshFromFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.clearInterval(intervalId);
      clearTimeout(confirmationTimer);
    };
  }, [refreshNavigationBadges]);

  useMediaRequestsSubscription(
    () => {
      void refreshNavigationBadges();
    },
    { pause: !canManageTitle && !canRequestMedia },
  );

  return {
    pendingImportCounts: loadedAuth === scope ? pendingImportCounts : null,
    pendingMediaRequestCounts: loadedAuth === scope ? pendingMediaRequestCounts : null,
    manualImportRequiredCount: loadedAuth === scope ? manualImportRequiredCount : 0,
    manualImportCountKnown: loadedAuth === scope,
    manualImportCountStale: loadedAuth === scope && manualImportCountStale,
    pluginUpdateCount: loadedAuth === scope ? pluginUpdateCount : 0,
    pluginBlockedCount: loadedAuth === scope ? pluginBlockedCount : 0,
    scryerVersion,
  };
}
