import { useCallback, useRef } from "react";
import { toast } from "sonner";

import { classifyStatusToastLevel } from "@/lib/utils/status-toast";

type SetGlobalStatus = (status: string) => void;

type UseGlobalStatusToastOptions = {
  dedupeMs?: number;
  onServiceRestarting?: () => void;
};

const DEFAULT_DEDUPE_MS = 1200;

export function useGlobalStatusToast(setGlobalStatus: SetGlobalStatus, {
  dedupeMs = DEFAULT_DEDUPE_MS,
  onServiceRestarting,
}: UseGlobalStatusToastOptions = {}) {
  const lastToastRef = useRef({
    key: "",
    at: 0,
  });

  return useCallback((rawStatus: string) => {
    // When the backend is restarting, the splash page returns HTML instead of
    // JSON.  Show a full-screen overlay instead of dumping raw HTML.
    if (/<!doctype\s+html/i.test(rawStatus)) {
      onServiceRestarting?.();
      return;
    }

    setGlobalStatus(rawStatus);

    const toastLevel = classifyStatusToastLevel(rawStatus);
    if (!toastLevel) {
      return;
    }

    const now = Date.now();
    const key = `${toastLevel}:${rawStatus.trim()}`;
    if (lastToastRef.current.key === key && now - lastToastRef.current.at < dedupeMs) {
      return;
    }

    if (toastLevel === "success") {
      toast.success(rawStatus);
    } else if (toastLevel === "error") {
      toast.error(rawStatus);
    } else {
      toast.warning(rawStatus);
    }

    lastToastRef.current = { key, at: now };
  }, [dedupeMs, onServiceRestarting, setGlobalStatus]);
}
