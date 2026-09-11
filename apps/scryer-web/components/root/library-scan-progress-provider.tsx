import * as React from "react";
import { useNavigate } from "react-router";

import { LibraryScanToast } from "@/components/root/library-scan-toast";
import { Toaster, toast } from "@/components/ui/sonner";
import { LibraryScanProgressContext } from "@/lib/context/library-scan-progress-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useLibraryScanEventStream } from "@/lib/hooks/use-library-scan-event-stream";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { Facet, LibraryScanStatus } from "@/lib/types";
import type { ViewId } from "@/components/root/types";
import { dispatchNavigationBadgesRefresh } from "@/lib/events/navigation-badges";
import { facetById } from "@/lib/facets/registry";
import { buildViewPath } from "@/lib/utils/routing";

const AUTO_DISMISS_DESKTOP_MS = 5_000;
const AUTO_DISMISS_MOBILE_MS = 3_000;
const MOBILE_HIDE_RUNNING_MS = 3_000;
const TOAST_EXIT_GRACE_MS = 200;
/** The top-right stack the library-scan and location-move toasts share. */
export const LIBRARY_SCAN_TOASTER_ID = "library-scans";
const MAX_VISIBLE_LIBRARY_SCAN_TOASTS = 3;
const MAX_VISIBLE_GENERAL_TOASTS = 3;

// Strip the default sonner container chrome so the toast's own glass card is the
// only visible surface, and widen the stack to the design's 392px card.
const LIBRARY_SCAN_TOASTER_STYLE = { "--width": "392px" } as React.CSSProperties;
const LIBRARY_SCAN_TOAST_OPTIONS = {
  className: "",
  classNames: { toast: "!bg-transparent !border-0 !p-0 !shadow-none" },
};

function isTerminal(status: LibraryScanStatus): boolean {
  return (
    status === "COMPLETED" ||
    status === "CANCELED" ||
    status === "WARNING" ||
    status === "FAILED"
  );
}

function LiveLibraryScanToast({
  sessionId,
  autoDismissMs,
  onRunInBackground,
  onDismiss,
  onViewTitles,
  onReviewUnmatched,
}: {
  sessionId: string;
  autoDismissMs: number;
  onRunInBackground?: () => void;
  onDismiss?: () => void;
  onViewTitles?: () => void;
  onReviewUnmatched?: () => void;
}) {
  const t = useTranslate();
  const value = React.useContext(LibraryScanProgressContext);
  if (!value) {
    return null;
  }

  const session = value.getSessionById(sessionId);
  if (!session) {
    return null;
  }

  return (
    <LibraryScanToast
      session={session}
      t={t}
      autoDismissMs={autoDismissMs}
      onRunInBackground={onRunInBackground}
      onDismiss={onDismiss}
      onViewTitles={onViewTitles}
      onReviewUnmatched={onReviewUnmatched}
    />
  );
}

export function LibraryScanProgressProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const { sessions, getActiveSession, refreshSessions, dismissSession } =
    useLibraryScanEventStream();
  const isMobile = useIsMobile();
  const navigate = useNavigate();
  const mobileDismissTimersRef = React.useRef<
    Record<string, ReturnType<typeof setTimeout>>
  >({});
  const shownToastIdsRef = React.useRef<Set<string>>(new Set());
  const backgroundedSessionIdsRef = React.useRef<Set<string>>(new Set());
  const refreshedPendingImportSessionsRef = React.useRef<Set<string>>(new Set());

  const getSessionById = React.useCallback(
    (sessionId: string) =>
      sessions.find((session) => session.sessionId === sessionId) ?? null,
    [sessions],
  );

  // Backgrounding ("Run in background") only hides the toast — the scan keeps
  // running and the session stays in state.
  const hideSessionToast = React.useCallback((sessionId: string) => {
    backgroundedSessionIdsRef.current.add(sessionId);
    const mobileTimer = mobileDismissTimersRef.current[sessionId];
    if (mobileTimer) {
      clearTimeout(mobileTimer);
      delete mobileDismissTimersRef.current[sessionId];
    }
    toast.dismiss(sessionId);
  }, []);

  // Dismissing a terminal toast removes it and drops the session from state.
  const dismissSessionToast = React.useCallback(
    (sessionId: string) => {
      toast.dismiss(sessionId);
      window.setTimeout(() => {
        dismissSession(sessionId);
        shownToastIdsRef.current.delete(sessionId);
        backgroundedSessionIdsRef.current.delete(sessionId);
        refreshedPendingImportSessionsRef.current.delete(sessionId);
        const mobileTimer = mobileDismissTimersRef.current[sessionId];
        if (mobileTimer) {
          clearTimeout(mobileTimer);
          delete mobileDismissTimersRef.current[sessionId];
        }
      }, TOAST_EXIT_GRACE_MS);
    },
    [dismissSession],
  );

  const dismissFacetReviewToasts = React.useCallback(
    (facet: Facet) => {
      for (const session of sessions) {
        if (
          session.facet !== facet ||
          (session.summary?.unmatched ?? 0) === 0 ||
          !shownToastIdsRef.current.has(session.sessionId) ||
          backgroundedSessionIdsRef.current.has(session.sessionId)
        ) {
          continue;
        }
        dismissSessionToast(session.sessionId);
      }
    },
    [dismissSessionToast, sessions],
  );

  React.useEffect(() => {
    for (const session of sessions) {
      if (isTerminal(session.status)) {
        if (!refreshedPendingImportSessionsRef.current.has(session.sessionId)) {
          dispatchNavigationBadgesRefresh();
          refreshedPendingImportSessionsRef.current.add(session.sessionId);
        }
        // Terminal toasts own their own lifecycle: success/canceled
        // auto-dismiss (with countdown) inside the toast; issues/failed wait
        // for the user. Clear any leftover running-scan mobile timer.
        const mobileTimer = mobileDismissTimersRef.current[session.sessionId];
        if (mobileTimer) {
          clearTimeout(mobileTimer);
          delete mobileDismissTimersRef.current[session.sessionId];
        }
      } else {
        refreshedPendingImportSessionsRef.current.delete(session.sessionId);

        if (
          isMobile &&
          !backgroundedSessionIdsRef.current.has(session.sessionId)
        ) {
          if (!mobileDismissTimersRef.current[session.sessionId]) {
            mobileDismissTimersRef.current[session.sessionId] = setTimeout(() => {
              hideSessionToast(session.sessionId);
            }, MOBILE_HIDE_RUNNING_MS);
          }
        } else if (!isMobile) {
          const mobileTimer = mobileDismissTimersRef.current[session.sessionId];
          if (mobileTimer) {
            clearTimeout(mobileTimer);
            delete mobileDismissTimersRef.current[session.sessionId];
          }
        }
      }

      if (!shownToastIdsRef.current.has(session.sessionId)) {
        const sessionId = session.sessionId;
        const viewId = facetById(session.facet)?.viewId as ViewId | undefined;
        toast.custom(
          () => (
            <LiveLibraryScanToast
              sessionId={sessionId}
              autoDismissMs={
                isMobile ? AUTO_DISMISS_MOBILE_MS : AUTO_DISMISS_DESKTOP_MS
              }
              onRunInBackground={() => hideSessionToast(sessionId)}
              onDismiss={() => dismissSessionToast(sessionId)}
              onViewTitles={
                viewId
                  ? () => {
                      navigate(buildViewPath(viewId));
                      dismissSessionToast(sessionId);
                    }
                  : undefined
              }
              onReviewUnmatched={
                viewId
                  ? () => {
                      navigate(buildViewPath(viewId, undefined, "import"));
                      dismissSessionToast(sessionId);
                    }
                  : undefined
              }
            />
          ),
          {
            id: sessionId,
            toasterId: LIBRARY_SCAN_TOASTER_ID,
            duration: Infinity,
          },
        );
        shownToastIdsRef.current.add(sessionId);
      }
    }
  }, [
    dismissSessionToast,
    hideSessionToast,
    isMobile,
    navigate,
    sessions,
  ]);

  React.useEffect(
    () => () => {
      for (const timer of Object.values(mobileDismissTimersRef.current)) {
        clearTimeout(timer);
      }
      shownToastIdsRef.current.clear();
      backgroundedSessionIdsRef.current.clear();
      refreshedPendingImportSessionsRef.current.clear();
    },
    [],
  );

  const value = React.useMemo(
    () => ({
      sessions: sessions.filter((session) => !isTerminal(session.status)),
      getActiveSession,
      getSessionById,
      dismissFacetReviewToasts,
      refreshSessions,
    }),
    [
      dismissFacetReviewToasts,
      getActiveSession,
      getSessionById,
      refreshSessions,
      sessions,
    ],
  );

  return (
    <LibraryScanProgressContext.Provider value={value}>
      {children}
      <Toaster
        id={LIBRARY_SCAN_TOASTER_ID}
        position="top-right"
        duration={10000}
        expand={false}
        visibleToasts={MAX_VISIBLE_LIBRARY_SCAN_TOASTS}
        style={LIBRARY_SCAN_TOASTER_STYLE}
        toastOptions={LIBRARY_SCAN_TOAST_OPTIONS}
      />
      <Toaster
        position="bottom-right"
        duration={10000}
        expand={false}
        visibleToasts={MAX_VISIBLE_GENERAL_TOASTS}
      />
    </LibraryScanProgressContext.Provider>
  );
}
