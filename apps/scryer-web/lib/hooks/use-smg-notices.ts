import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { backendClient } from "@/lib/graphql/urql-client";
import {
  smgScryerUpdateNoticeQuery,
  smgVersionCompatibilityNoticeQuery,
} from "@/lib/graphql/queries";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import { scheduleAfterFirstPaint } from "@/lib/utils/scheduling";
import type {
  SmgScryerUpdateNotice,
  SmgVersionCompatibilityNotice,
} from "@/components/root/types";

const SMG_VERSION_COMPATIBILITY_NOTICE_KEY = "smg.version_compatibility_notice";
const SMG_SCRYER_UPDATE_NOTICE_KEY = "smg.scryer_update_notice";
const SMG_SCRYER_UPDATE_DISMISSED_KEY = "scryer.smgUpdate.dismissed";
const SMG_NOTICE_SESSION_CACHE_KEY = "scryer.smgNotices.session.v1";
const SMG_NOTICE_REFRESH_INTERVAL_MS = 5 * 60 * 1_000;

type SmgNoticeSessionCache = {
  refreshedAt: number;
  versionCompatibilityNotice: SmgVersionCompatibilityNotice | null;
  scryerUpdateNotice: SmgScryerUpdateNotice | null;
};

function readSmgNoticeSessionCache(): SmgNoticeSessionCache | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const raw = window.sessionStorage.getItem(SMG_NOTICE_SESSION_CACHE_KEY);
    if (!raw) {
      return null;
    }
    const parsed = JSON.parse(raw) as Partial<SmgNoticeSessionCache>;
    if (
      typeof parsed.refreshedAt !== "number" ||
      Date.now() - parsed.refreshedAt >= SMG_NOTICE_REFRESH_INTERVAL_MS
    ) {
      window.sessionStorage.removeItem(SMG_NOTICE_SESSION_CACHE_KEY);
      return null;
    }
    return {
      refreshedAt: parsed.refreshedAt,
      versionCompatibilityNotice:
        parsed.versionCompatibilityNotice ?? null,
      scryerUpdateNotice: parsed.scryerUpdateNotice ?? null,
    };
  } catch {
    window.sessionStorage.removeItem(SMG_NOTICE_SESSION_CACHE_KEY);
    return null;
  }
}

function writeSmgNoticeSessionCache(cache: SmgNoticeSessionCache) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.sessionStorage.setItem(
      SMG_NOTICE_SESSION_CACHE_KEY,
      JSON.stringify(cache),
    );
  } catch {
    // Best effort only; notice checks are allowed to fall back to network.
  }
}

function buildSmgScryerUpdateDismissalValue(
  notice: SmgScryerUpdateNotice | null,
): string | null {
  if (!notice?.available) {
    return null;
  }
  const latest = notice.latestTag.trim() || notice.latestVersion.trim();
  if (!latest) {
    return null;
  }
  return `${latest}:${notice.latestVersion.trim()}`;
}

export function useSmgNotices({
  settingsSubscriptionEnabled = true,
}: {
  settingsSubscriptionEnabled?: boolean;
} = {}) {
  const initialSessionCache = useMemo(() => readSmgNoticeSessionCache(), []);
  const [smgVersionCompatibilityNotice, setSmgVersionCompatibilityNotice] =
    useState<SmgVersionCompatibilityNotice | null>(
      () => initialSessionCache?.versionCompatibilityNotice ?? null,
    );
  const [smgScryerUpdateNotice, setSmgScryerUpdateNotice] =
    useState<SmgScryerUpdateNotice | null>(
      () => initialSessionCache?.scryerUpdateNotice ?? null,
    );
  const lastRoutineSmgNoticeRefreshAtRef = useRef(
    initialSessionCache?.refreshedAt ?? 0,
  );
  const [dismissedSmgScryerUpdate, setDismissedSmgScryerUpdate] = useState(
    () => {
      if (typeof window === "undefined") {
        return "";
      }
      return (
        window.localStorage.getItem(SMG_SCRYER_UPDATE_DISMISSED_KEY) ?? ""
      );
    },
  );

  const refreshSmgVersionCompatibilityNotice = useCallback(async () => {
    try {
      const { data, error } = await backendClient
        .query<{
          smgVersionCompatibilityNotice?: SmgVersionCompatibilityNotice | null;
        }>(smgVersionCompatibilityNoticeQuery, {})
        .toPromise();
      if (error) {
        throw error;
      }
      const notice = data?.smgVersionCompatibilityNotice ?? null;
      setSmgVersionCompatibilityNotice(notice);
      return notice;
    } catch (error) {
      console.warn("Failed to refresh SMG version compatibility notice", error);
      return null;
    }
  }, []);

  const refreshSmgScryerUpdateNotice = useCallback(async () => {
    try {
      const { data, error } = await backendClient
        .query<{
          smgScryerUpdateNotice?: SmgScryerUpdateNotice | null;
        }>(smgScryerUpdateNoticeQuery, {})
        .toPromise();
      if (error) {
        throw error;
      }
      const notice = data?.smgScryerUpdateNotice ?? null;
      setSmgScryerUpdateNotice(notice);
      return notice;
    } catch (error) {
      console.warn("Failed to refresh SMG Scryer update notice", error);
      return null;
    }
  }, []);

  const refreshSmgNotices = useCallback(
    async ({ force = false }: { force?: boolean } = {}) => {
      const now = Date.now();
      if (!force) {
        const sessionCache = readSmgNoticeSessionCache();
        if (sessionCache) {
          lastRoutineSmgNoticeRefreshAtRef.current =
            sessionCache.refreshedAt;
          setSmgVersionCompatibilityNotice(
            sessionCache.versionCompatibilityNotice,
          );
          setSmgScryerUpdateNotice(sessionCache.scryerUpdateNotice);
          return;
        }
      }
      if (
        !force &&
        now - lastRoutineSmgNoticeRefreshAtRef.current <
          SMG_NOTICE_REFRESH_INTERVAL_MS
      ) {
        return;
      }
      lastRoutineSmgNoticeRefreshAtRef.current = now;
      const [versionCompatibilityNotice, scryerUpdateNotice] = await Promise.all([
        refreshSmgVersionCompatibilityNotice(),
        refreshSmgScryerUpdateNotice(),
      ]);
      writeSmgNoticeSessionCache({
        refreshedAt: now,
        versionCompatibilityNotice,
        scryerUpdateNotice,
      });
    },
    [refreshSmgScryerUpdateNotice, refreshSmgVersionCompatibilityNotice],
  );

  useEffect(() => {
    return scheduleAfterFirstPaint(() => {
      void refreshSmgNotices();
    });
  }, [refreshSmgNotices]);

  useSettingsSubscription(
    useCallback(
      (changedKeys) => {
        if (
          changedKeys.includes(SMG_VERSION_COMPATIBILITY_NOTICE_KEY) ||
          changedKeys.includes(SMG_SCRYER_UPDATE_NOTICE_KEY)
        ) {
          void refreshSmgNotices({ force: true });
        }
      },
      [refreshSmgNotices],
    ),
    { enabled: settingsSubscriptionEnabled },
  );

  useEffect(() => {
    if (typeof window === "undefined" || typeof document === "undefined") {
      return;
    }

    const handleFocus = () => {
      void refreshSmgNotices();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void refreshSmgNotices();
      }
    };

    window.addEventListener("focus", handleFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    const intervalId = window.setInterval(() => {
      void refreshSmgNotices();
    }, SMG_NOTICE_REFRESH_INTERVAL_MS);
    return () => {
      window.removeEventListener("focus", handleFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.clearInterval(intervalId);
    };
  }, [refreshSmgNotices]);

  const smgScryerUpdateDismissalValue = useMemo(
    () => buildSmgScryerUpdateDismissalValue(smgScryerUpdateNotice),
    [smgScryerUpdateNotice],
  );
  const showSmgScryerUpdateReminder =
    !smgVersionCompatibilityNotice &&
    Boolean(smgScryerUpdateNotice?.available) &&
    Boolean(smgScryerUpdateDismissalValue) &&
    dismissedSmgScryerUpdate !== smgScryerUpdateDismissalValue;

  const dismissSmgScryerUpdateReminder = useCallback(() => {
    if (!smgScryerUpdateDismissalValue) {
      return;
    }
    setDismissedSmgScryerUpdate(smgScryerUpdateDismissalValue);
    if (typeof window !== "undefined") {
      window.localStorage.setItem(
        SMG_SCRYER_UPDATE_DISMISSED_KEY,
        smgScryerUpdateDismissalValue,
      );
    }
  }, [smgScryerUpdateDismissalValue]);

  return {
    smgVersionCompatibilityNotice,
    smgScryerUpdateNotice,
    showSmgScryerUpdateReminder,
    dismissSmgScryerUpdateReminder,
    refreshSmgNotices,
  };
}
