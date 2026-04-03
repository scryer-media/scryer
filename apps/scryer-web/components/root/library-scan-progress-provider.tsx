import * as React from "react";
import { useClient } from "urql";

import { LibraryScanToast } from "@/components/root/library-scan-toast";
import { toast } from "@/components/ui/sonner";
import { LibraryScanProgressContext } from "@/lib/context/library-scan-progress-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  activeLibraryScansQuery,
  libraryScanProgressSubscriptionQuery,
} from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import type { Facet, LibraryScanMode, LibraryScanProgress, LibraryScanStatus } from "@/lib/types";

const TERMINAL_TOAST_DURATION_MS = 6_000;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeFacet(value: unknown): Facet {
  return value === "anime" ? "anime" : value === "tv" ? "tv" : "movie";
}

function normalizeStatus(value: unknown): LibraryScanStatus {
  switch (value) {
    case "discovering":
    case "running":
    case "completed":
    case "warning":
    case "failed":
      return value;
    default:
      return "running";
  }
}

function normalizeMode(value: unknown): LibraryScanMode {
  return value === "additive" ? "additive" : "full";
}

function normalizeNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function normalizePhaseProgress(value: unknown) {
  const record = isRecord(value) ? value : {};
  return {
    total: normalizeNumber(record.total),
    completed: normalizeNumber(record.completed),
    failed: normalizeNumber(record.failed),
  };
}

function normalizeSummary(value: unknown) {
  if (!isRecord(value)) {
    return null;
  }
  return {
    scanned: normalizeNumber(value.scanned),
    matched: normalizeNumber(value.matched),
    imported: normalizeNumber(value.imported),
    skipped: normalizeNumber(value.skipped),
    unmatched: normalizeNumber(value.unmatched),
  };
}

function normalizeLibraryScanProgress(
  value: unknown,
): LibraryScanProgress | null {
  if (!isRecord(value) || typeof value.sessionId !== "string") {
    return null;
  }

  return {
    sessionId: value.sessionId,
    facet: normalizeFacet(value.facet),
    mode: normalizeMode(value.mode),
    status: normalizeStatus(value.status),
    startedAt:
      typeof value.startedAt === "string"
        ? value.startedAt
        : new Date().toISOString(),
    updatedAt:
      typeof value.updatedAt === "string"
        ? value.updatedAt
        : new Date().toISOString(),
    foundTitles: normalizeNumber(value.foundTitles),
    metadataTotalKnown: value.metadataTotalKnown === true,
    fileTotalKnown: value.fileTotalKnown === true,
    metadataProgress: normalizePhaseProgress(value.metadataProgress),
    fileProgress: normalizePhaseProgress(value.fileProgress),
    summary: normalizeSummary(value.summary),
  };
}

function isTerminal(status: LibraryScanStatus): boolean {
  return status === "completed" || status === "warning" || status === "failed";
}

function preferSessionSnapshot(
  existing: LibraryScanProgress | undefined,
  incoming: LibraryScanProgress,
): LibraryScanProgress {
  if (!existing) {
    return incoming;
  }

  const updatedAtComparison = existing.updatedAt.localeCompare(incoming.updatedAt);
  if (updatedAtComparison > 0) {
    return existing;
  }
  if (
    updatedAtComparison === 0 &&
    isTerminal(existing.status) &&
    !isTerminal(incoming.status)
  ) {
    return existing;
  }

  return incoming;
}

export function LibraryScanProgressProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const client = useClient();
  const t = useTranslate();
  const [sessionsById, setSessionsById] = React.useState<
    Record<string, LibraryScanProgress>
  >({});
  const dismissTimersRef = React.useRef<
    Record<string, ReturnType<typeof setTimeout>>
  >({});

  const upsertSession = React.useCallback((session: LibraryScanProgress) => {
    setSessionsById((current) => ({
      ...current,
      [session.sessionId]: preferSessionSnapshot(
        current[session.sessionId],
        session,
      ),
    }));
  }, []);

  React.useEffect(() => {
    let cancelled = false;

    (async () => {
      const { data, error } = await client.query(activeLibraryScansQuery, {}).toPromise();
      if (cancelled || error) {
        if (error) {
          console.error("[library-scan-progress] failed to load active scans:", error);
        }
        return;
      }

      const rawSessions: unknown[] = Array.isArray(data?.activeLibraryScans)
        ? data.activeLibraryScans
        : [];
      const normalizedSessions = rawSessions
        .map(normalizeLibraryScanProgress)
        .filter(
          (session): session is LibraryScanProgress =>
            session !== null && session.mode === "full",
        );

      setSessionsById((current) => {
        const next = { ...current };
        for (const session of normalizedSessions) {
          next[session.sessionId] = preferSessionSnapshot(
            next[session.sessionId],
            session,
          );
        }
        return next;
      });
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  useDeferredWsSubscription<{ data?: { libraryScanProgress?: unknown } }>({
    requestKey: "libraryScanProgress",
    request: { query: libraryScanProgressSubscriptionQuery },
    onNext(result) {
      const normalized = normalizeLibraryScanProgress(
        result.data?.libraryScanProgress,
      );
      if (normalized && normalized.mode === "full") {
        upsertSession(normalized);
      }
    },
    onError(error) {
      console.error("[library-scan-progress] subscription error:", error);
    },
  });

  React.useEffect(() => {
    for (const session of Object.values(sessionsById)) {
      if (isTerminal(session.status)) {
        const existingTimer = dismissTimersRef.current[session.sessionId];
        if (!existingTimer) {
          dismissTimersRef.current[session.sessionId] = setTimeout(() => {
            toast.dismiss(session.sessionId);
            setSessionsById((current) => {
              const next = { ...current };
              delete next[session.sessionId];
              return next;
            });
            delete dismissTimersRef.current[session.sessionId];
          }, TERMINAL_TOAST_DURATION_MS);
        }
      } else {
        const existingTimer = dismissTimersRef.current[session.sessionId];
        if (existingTimer) {
          clearTimeout(existingTimer);
          delete dismissTimersRef.current[session.sessionId];
        }
      }

      toast.custom(() => <LibraryScanToast session={session} t={t} />, {
        id: session.sessionId,
        className: "rounded-lg overflow-hidden p-0",
        duration: isTerminal(session.status) ? TERMINAL_TOAST_DURATION_MS : Infinity,
      });
    }
  }, [sessionsById, t]);

  React.useEffect(
    () => () => {
      for (const timer of Object.values(dismissTimersRef.current)) {
        clearTimeout(timer);
      }
    },
    [],
  );

  const value = React.useMemo(
    () => ({
      sessions: Object.values(sessionsById).sort((left, right) =>
        left.startedAt.localeCompare(right.startedAt),
      ),
      getActiveSession: (facet: Facet) =>
        Object.values(sessionsById).find(
          (session) =>
            session.mode === "full" &&
            session.facet === facet &&
            !isTerminal(session.status),
        ) ?? null,
    }),
    [sessionsById],
  );

  return (
    <LibraryScanProgressContext.Provider value={value}>
      {children}
    </LibraryScanProgressContext.Provider>
  );
}
