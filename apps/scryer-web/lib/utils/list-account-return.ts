import {
  captureListAccountReturn,
  LIST_ACCOUNT_LINK_STORAGE_KEY,
} from "./list-account-link";

// Capture and erase callback material while the router loads, before any render.
export const listAccountReturn = (() => {
  if (typeof window === "undefined" || !/\/lists\/oauth\/(return|callback)$/.test(window.location.pathname)) return null;
  return captureListAccountReturn(window.location, window.history, () => window.sessionStorage.getItem(LIST_ACCOUNT_LINK_STORAGE_KEY));
})();
