import * as React from "react";
import { useNavigate } from "react-router";

import { useJobRunToasts } from "@/components/root/job-run-provider";
import { LIBRARY_SCAN_TOASTER_ID } from "@/components/root/library-scan-progress-provider";
import { LocationMoveToast } from "@/components/root/location-move-toast";
import { toast } from "@/components/ui/sonner";
import { LocationMoveProgressContext } from "@/lib/context/location-move-progress-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useLocationTransfer } from "@/lib/hooks/use-location-transfer";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import {
  locationOperationIdFromJobRun,
  moveToastVisualState,
} from "@/lib/location-move-toasts";
import { isTerminalJobRunStatus } from "@/lib/utils/job-runs";

const AUTO_DISMISS_DESKTOP_MS = 5_000;
const AUTO_DISMISS_MOBILE_MS = 3_000;
const MOBILE_HIDE_RUNNING_MS = 3_000;
const TOAST_EXIT_GRACE_MS = 200;

function moveToastId(operationId: string): string {
  return `location-move:${operationId}`;
}

/**
 * One toast's live feed. The transfer summary stream is owned here so a
 * backgrounded toast (unmounted) stops listening; the card itself stays pure.
 */
function LiveLocationMoveToast({
  operationId,
  autoDismissMs,
  onRunInBackground,
  onDismiss,
  onSeeInActivity,
  onSettled,
}: {
  operationId: string;
  autoDismissMs: number;
  onRunInBackground: () => void;
  onDismiss: () => void;
  onSeeInActivity: () => void;
  onSettled: () => void;
}) {
  const t = useTranslate();
  const { snapshot } = useLocationTransfer(operationId, null, true);
  const settled = moveToastVisualState(snapshot?.operation.state) !== "moving";
  React.useEffect(() => {
    if (settled) {
      onSettled();
    }
  }, [onSettled, settled]);

  return (
    <LocationMoveToast
      snapshot={snapshot}
      t={t}
      autoDismissMs={autoDismissMs}
      onRunInBackground={onRunInBackground}
      onDismiss={onDismiss}
      onSeeInActivity={onSeeInActivity}
    />
  );
}

/**
 * Semi-persistent toasts for location operations, on the library-scan stack.
 * An operation is picked up from the Activity job run it reports through —
 * so a page load mid-move shows it — or the instant a dialog's start is
 * accepted. "Run in background" only hides the card until the next page
 * load, exactly like the library-scan toast; dismissing a settled card
 * forgets the operation.
 */
export function LocationMoveProgressProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const { runs } = useJobRunToasts();
  const isMobile = useIsMobile();
  const navigate = useNavigate();
  const [trackedOperationIds, setTrackedOperationIds] = React.useState<
    string[]
  >([]);
  const shownIdsRef = React.useRef<Set<string>>(new Set());
  const backgroundedIdsRef = React.useRef<Set<string>>(new Set());
  const settledIdsRef = React.useRef<Set<string>>(new Set());
  const dismissedIdsRef = React.useRef<Set<string>>(new Set());
  const mobileHideTimersRef = React.useRef<
    Record<string, ReturnType<typeof setTimeout>>
  >({});

  const clearMobileHideTimer = React.useCallback((operationId: string) => {
    const timer = mobileHideTimersRef.current[operationId];
    if (timer) {
      clearTimeout(timer);
      delete mobileHideTimersRef.current[operationId];
    }
  }, []);

  const trackOperation = React.useCallback((operationId: string) => {
    setTrackedOperationIds((current) =>
      current.includes(operationId) ? current : [...current, operationId],
    );
  }, []);

  // A running location job run names its operation in its progress. Settled
  // runs are left alone: a finished move belongs to Activity, not a toast —
  // and a toast the user dismissed stays dismissed even if its run lingers.
  React.useEffect(() => {
    for (const run of runs) {
      if (isTerminalJobRunStatus(run.status)) {
        continue;
      }
      const operationId = locationOperationIdFromJobRun(run);
      if (operationId && !dismissedIdsRef.current.has(operationId)) {
        trackOperation(operationId);
      }
    }
  }, [runs, trackOperation]);

  // Backgrounding only hides the toast — the move keeps running.
  const hideToast = React.useCallback(
    (operationId: string) => {
      backgroundedIdsRef.current.add(operationId);
      clearMobileHideTimer(operationId);
      toast.dismiss(moveToastId(operationId));
    },
    [clearMobileHideTimer],
  );

  // Dismissing a settled toast removes it and forgets the operation.
  const dismissToast = React.useCallback(
    (operationId: string) => {
      dismissedIdsRef.current.add(operationId);
      toast.dismiss(moveToastId(operationId));
      window.setTimeout(() => {
        setTrackedOperationIds((current) =>
          current.filter((id) => id !== operationId),
        );
        shownIdsRef.current.delete(operationId);
        backgroundedIdsRef.current.delete(operationId);
        settledIdsRef.current.delete(operationId);
        clearMobileHideTimer(operationId);
      }, TOAST_EXIT_GRACE_MS);
    },
    [clearMobileHideTimer],
  );

  const markSettled = React.useCallback(
    (operationId: string) => {
      settledIdsRef.current.add(operationId);
      clearMobileHideTimer(operationId);
    },
    [clearMobileHideTimer],
  );

  const seeInActivity = React.useCallback(
    (operationId: string) => {
      void navigate(`/activity?operation=${encodeURIComponent(operationId)}`);
      if (settledIdsRef.current.has(operationId)) {
        dismissToast(operationId);
      } else {
        hideToast(operationId);
      }
    },
    [dismissToast, hideToast, navigate],
  );

  React.useEffect(() => {
    for (const operationId of trackedOperationIds) {
      const settled = settledIdsRef.current.has(operationId);
      if (
        isMobile &&
        !settled &&
        !backgroundedIdsRef.current.has(operationId)
      ) {
        if (!mobileHideTimersRef.current[operationId]) {
          mobileHideTimersRef.current[operationId] = setTimeout(() => {
            delete mobileHideTimersRef.current[operationId];
            if (!settledIdsRef.current.has(operationId)) {
              hideToast(operationId);
            }
          }, MOBILE_HIDE_RUNNING_MS);
        }
      } else if (!isMobile) {
        clearMobileHideTimer(operationId);
      }

      if (shownIdsRef.current.has(operationId)) {
        continue;
      }
      shownIdsRef.current.add(operationId);
      toast.custom(
        () => (
          <LiveLocationMoveToast
            operationId={operationId}
            autoDismissMs={
              isMobile ? AUTO_DISMISS_MOBILE_MS : AUTO_DISMISS_DESKTOP_MS
            }
            onRunInBackground={() => hideToast(operationId)}
            onDismiss={() => dismissToast(operationId)}
            onSeeInActivity={() => seeInActivity(operationId)}
            onSettled={() => markSettled(operationId)}
          />
        ),
        {
          id: moveToastId(operationId),
          toasterId: LIBRARY_SCAN_TOASTER_ID,
          duration: Infinity,
        },
      );
    }
  }, [
    clearMobileHideTimer,
    dismissToast,
    hideToast,
    isMobile,
    markSettled,
    seeInActivity,
    trackedOperationIds,
  ]);

  React.useEffect(
    () => () => {
      for (const timer of Object.values(mobileHideTimersRef.current)) {
        clearTimeout(timer);
      }
      mobileHideTimersRef.current = {};
      shownIdsRef.current.clear();
      backgroundedIdsRef.current.clear();
      settledIdsRef.current.clear();
      dismissedIdsRef.current.clear();
    },
    [],
  );

  const value = React.useMemo(
    () => ({ trackedOperationIds, trackOperation }),
    [trackOperation, trackedOperationIds],
  );

  return (
    <LocationMoveProgressContext.Provider value={value}>
      {children}
    </LocationMoveProgressContext.Provider>
  );
}
