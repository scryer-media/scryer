import { useCallback, useEffect, useState } from "react";

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

type NavigationBadgeCountsPayload = {
  pendingImportCounts?: PendingImportCounts | null;
  pendingMediaRequestCounts?: PendingImportCounts | null;
  activityImportCount?: number | null;
  pluginUpdateCount?: number | null;
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

export function useNavigationBadges({
  serviceRestarting,
  canManageTitle,
  canRequestMedia,
}: {
  serviceRestarting: boolean;
  canManageTitle: boolean;
  canRequestMedia: boolean;
}) {
  const [pendingImportCounts, setPendingImportCounts] =
    useState<PendingImportCounts | null>(null);
  const [pendingMediaRequestCounts, setPendingMediaRequestCounts] =
    useState<PendingImportCounts | null>(null);
  const [manualImportRequiredCount, setManualImportRequiredCount] = useState(0);
  const [pluginUpdateCount, setPluginUpdateCount] = useState(0);
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

  const refreshNavigationBadges = useCallback(async () => {
    try {
      const badgeCountsResult = await backendClient
        .query(navigationBadgeCountsQuery, {})
        .toPromise();

      if (badgeCountsResult.error) {
        throw badgeCountsResult.error;
      }

      const badgeCounts = badgeCountsResult.data?.navigationBadgeCounts as
        | NavigationBadgeCountsPayload
        | undefined;
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
    } catch (error) {
      console.warn("Failed to refresh navigation badges", error);
    }
  }, []);

  useEffect(() => {
    return scheduleAfterFirstPaint(() => {
      void refreshNavigationBadges();
    });
  }, [refreshNavigationBadges]);

  useEffect(() => {
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
        setManualImportRequiredCount((current) => Math.max(0, current + delta));
        window.setTimeout(() => {
          void refreshNavigationBadges();
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
    };
  }, [refreshNavigationBadges]);

  useMediaRequestsSubscription(
    () => {
      void refreshNavigationBadges();
    },
    { pause: !canManageTitle && !canRequestMedia },
  );

  return {
    pendingImportCounts,
    pendingMediaRequestCounts,
    manualImportRequiredCount,
    pluginUpdateCount,
    scryerVersion,
  };
}
