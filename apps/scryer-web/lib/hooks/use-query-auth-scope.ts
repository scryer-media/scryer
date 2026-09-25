import { useSyncExternalStore } from "react";
import { AUTH_SESSION_CHANGED_EVENT, getAuthToken } from "./use-auth";

function subscribe(onChange: () => void) {
  window.addEventListener(AUTH_SESSION_CHANGED_EVENT, onChange);
  window.addEventListener("storage", onChange);
  return () => {
    window.removeEventListener(AUTH_SESSION_CHANGED_EVENT, onChange);
    window.removeEventListener("storage", onChange);
  };
}

export function useQueryAuthScope() {
  return useSyncExternalStore(subscribe, getAuthToken, () => null);
}
