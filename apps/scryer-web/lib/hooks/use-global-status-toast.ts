import { createElement, useCallback, useRef } from "react";
import { Unplug } from "lucide-react";
import { toast } from "sonner";

import type { GlobalStatusOptions, SetGlobalStatus } from "@/lib/context/global-status-context";
import { normalizeGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { classifyStatusToastLevel } from "@/lib/utils/status-toast";

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

  return useCallback((rawStatus: string, options?: GlobalStatusOptions) => {
    setGlobalStatus(rawStatus);

    if (options?.suppressToast) {
      return;
    }

    const toastLevel = classifyStatusToastLevel(rawStatus);
    if (!toastLevel) {
      return;
    }

    const displayStatus = normalizeGraphQlErrorMessage(rawStatus) || rawStatus.trim();

    const now = Date.now();
    const key = `${toastLevel}:${displayStatus.trim()}`;
    if (lastToastRef.current.key === key && now - lastToastRef.current.at < dedupeMs) {
      return;
    }

    const isDisconnected = /^\[network\]\s+failed to fetch$/i.test(displayStatus);
    const toastMessage = isDisconnected ? "Disconnected" : displayStatus;
    const content = options?.toastId
      ? createElement("span", { id: options.toastId }, toastMessage)
      : toastMessage;
    // The stable id lives on the rendered span only. Handing it to sonner as
    // the toast id re-uses one toast slot across saves: a dismissal sonner
    // still has in flight for the previous toast (its exit runs on a deferred
    // frame) then lands on the freshly created one, which vanishes within a
    // frame or two and never gets read. Fresh sonner ids keep every toast
    // independent while the span id stays a stable DOM contract.
    const toastOptions = {
      ...(isDisconnected
        ? { icon: createElement(Unplug, { "aria-hidden": true }) }
        : {}),
    };

    if (toastLevel === "SUCCESS") {
      toast.success(content, toastOptions);
    } else if (toastLevel === "ERROR") {
      toast.error(content, toastOptions);
    } else {
      toast.warning(content, toastOptions);
    }

    lastToastRef.current = { key, at: now };
  }, [dedupeMs, setGlobalStatus]);
}
