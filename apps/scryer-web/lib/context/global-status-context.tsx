import { createContext, useContext } from "react";

import type { StatusToastKind } from "@/lib/utils/status-toast";

export type GlobalStatusOptions = {
  toastId?: string;
  /** Set when the caller renders its own richer toast for the same event. */
  suppressToast?: boolean;
  /**
   * The level this status toasts at. Leave it out and the status shows no
   * toast at all, which is right only for progress notes and form hints.
   *
   * Nothing infers a level from the wording, so state it from the code path:
   * a catch block or an error result is "ERROR", the success branch is
   * "SUCCESS", and an outcome that went through with a caveat is "WARNING".
   */
  level?: StatusToastKind;
};

export type SetGlobalStatus = (status: string, options?: GlobalStatusOptions) => void;

export const GlobalStatusContext = createContext<SetGlobalStatus | null>(null);

export function useGlobalStatus(): SetGlobalStatus {
  const fn = useContext(GlobalStatusContext);
  if (!fn) throw new Error("useGlobalStatus must be used within GlobalStatusContext.Provider");
  return fn;
}
