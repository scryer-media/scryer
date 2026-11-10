import * as React from "react";
import { useLocation, useNavigate, useNavigationType } from "react-router";
import {
  createGraphHistorySync,
  graphHistoryState,
} from "../utils/library-graph-history";

/** Store only the bounded exploration, never the library's graph or posters. */
export function useLibraryGraphHistory<T>(
  scope: string,
  snapshot: T,
  restore: (snapshot: T) => void,
) {
  const location = useLocation();
  const navigate = useNavigate();
  const navigationType = useNavigationType();
  const [sync] = React.useState(() => createGraphHistorySync<T>());

  React.useLayoutEffect(() => {
    const action = sync.visit(
      location.key,
      navigationType,
      location.state,
      scope,
    );
    if (action.kind === "skip") return;
    if (action.kind === "restore") {
      restore(action.snapshot);
      return;
    }
    // Replace the current snapshot as asynchronous recommendations arrive.
    // Keep the router's key/index and unrelated user state intact. This does
    // not navigate or create another entry for loading or poster dismissal.
    const state = window.history.state;
    // A browser POP changes the native entry before React commits its route.
    if ((state?.key ?? "default") !== location.key) return;
    window.history.replaceState(
      { ...state, usr: graphHistoryState(state?.usr, scope, snapshot) },
      "",
    );
  }, [
    location.key,
    location.state,
    navigationType,
    restore,
    scope,
    snapshot,
    sync,
  ]);

  return React.useCallback(() => {
    // Router navigation can be a transition. Do not write the destination's
    // React state into the old entry while that transition is pending.
    sync.beginPush();
    void navigate(
      {
        pathname: location.pathname,
        search: location.search,
        hash: location.hash,
      },
      { state: graphHistoryState(location.state, scope, snapshot) },
    );
  }, [location, navigate, scope, snapshot, sync]);
}
