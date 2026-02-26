import { useCallback, useRef } from "react";
import { toast } from "sonner";

import { classifyStatusToastLevel } from "@/lib/utils/status-toast";

type SetGlobalStatus = (status: string) => void;

type UseGlobalStatusToastOptions = {
  dedupeMs?: number;
};

const DEFAULT_DEDUPE_MS = 1200;

export function useGlobalStatusToast(setGlobalStatus: SetGlobalStatus, {
  dedupeMs = DEFAULT_DEDUPE_MS,
}: UseGlobalStatusToastOptions = {}) {
  const lastToastRef = useRef({
    key: "",
    at: 0,
  });

  return useCallback((status: string) => {
    setGlobalStatus(status);

    const toastLevel = classifyStatusToastLevel(status);
    if (!toastLevel) {
      return;
    }

    const now = Date.now();
    const key = `${toastLevel}:${status.trim()}`;
    if (lastToastRef.current.key === key && now - lastToastRef.current.at < dedupeMs) {
      return;
    }

    if (toastLevel === "success") {
      toast.success(status);
    } else if (toastLevel === "error") {
      toast.error(status);
    } else {
      toast.warning(status);
    }

    lastToastRef.current = { key, at: now };
  }, [dedupeMs, setGlobalStatus]);
}
