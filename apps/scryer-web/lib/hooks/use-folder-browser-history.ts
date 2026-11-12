import * as React from "react";
import { useLocation, useNavigate, useNavigationType } from "react-router";
import {
  folderBrowserHistoryEntry,
  folderBrowserHistoryState,
  folderBrowserPopAction,
  newFolderBrowserSession,
} from "../utils/folder-browser-history";

/**
 * Browser back and forward move an open folder browser between the folders
 * it visited. Opening adds one entry for the starting folder, so stepping
 * back past it closes the browser instead of leaving the page; closing it any
 * other way steps back over its entries, so the page's own history is where
 * it was. The folder lives in the router's user state, never the URL.
 */
export function useFolderBrowserHistory({
  open,
  initialPath,
  onPopToFolder,
  onPopPastStart,
}: {
  open: boolean;
  initialPath: string;
  /** Show this folder; the entry already exists, so do not push another. */
  onPopToFolder: (path: string) => void;
  /** A back step left the browser's entries: close it. */
  onPopPastStart: () => void;
}) {
  const location = useLocation();
  const navigate = useNavigate();
  const navigationType = useNavigationType();
  const locationRef = React.useRef(location);
  const seenKeyRef = React.useRef(location.key);
  const sessionRef = React.useRef<string | null>(null);
  const depthRef = React.useRef(0);
  const callbacksRef = React.useRef({ onPopToFolder, onPopPastStart });
  React.useLayoutEffect(() => {
    callbacksRef.current = { onPopToFolder, onPopPastStart };
  }, [onPopPastStart, onPopToFolder]);

  const pushEntry = React.useCallback(
    (session: string, path: string, depth: number) => {
      const current = locationRef.current;
      void navigate(
        {
          pathname: current.pathname,
          search: current.search,
          hash: current.hash,
        },
        {
          state: folderBrowserHistoryState(current.state, {
            session,
            path,
            depth,
          }),
        },
      );
    },
    [navigate],
  );

  React.useEffect(() => {
    if (open) {
      // Already open: a re-run of this effect, not a new opening.
      if (sessionRef.current !== null) return;
      const session = newFolderBrowserSession();
      sessionRef.current = session;
      depthRef.current = 1;
      pushEntry(session, initialPath || "/", 1);
      return;
    }
    // Closed. The starting folder is read only when the browser opens, so a
    // changed `initialPath` re-runs this effect without effect.
    const session = sessionRef.current;
    if (session === null) return;
    const depth = depthRef.current;
    sessionRef.current = null;
    depthRef.current = 0;
    const entry = folderBrowserHistoryEntry(locationRef.current.state);
    if (entry?.session === session && depth > 0) {
      void navigate(-depth);
    }
  }, [initialPath, navigate, open, pushEntry]);

  // A host that unmounts the browser to close it never renders it closed, so
  // the unmount steps back over its entries instead. StrictMode's rehearsal
  // unmount remounts within the same commit, before the microtask runs.
  const mountedRef = React.useRef(false);
  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      const session = sessionRef.current;
      const depth = depthRef.current;
      if (session === null || depth <= 0) return;
      queueMicrotask(() => {
        if (mountedRef.current || sessionRef.current !== session) return;
        sessionRef.current = null;
        depthRef.current = 0;
        // The browser's own entry, not the last one this component saw: a
        // route change that unmounted it has already left these entries.
        const entry = folderBrowserHistoryEntry(window.history.state?.usr);
        if (entry?.session === session) void navigate(-depth);
      });
    };
  }, [navigate]);

  React.useLayoutEffect(() => {
    locationRef.current = location;
    if (location.key === seenKeyRef.current) return;
    seenKeyRef.current = location.key;
    const session = sessionRef.current;
    if (navigationType !== "POP" || session === null) return;
    const action = folderBrowserPopAction(location.state, session);
    if (action.kind === "browse") {
      depthRef.current = action.depth;
      callbacksRef.current.onPopToFolder(action.path);
      return;
    }
    // Already back on the page's own entry; closing must not step again.
    sessionRef.current = null;
    depthRef.current = 0;
    callbacksRef.current.onPopPastStart();
  }, [location, navigationType]);

  /** Record a folder the user moved to, unless it is the one shown. */
  return React.useCallback(
    (path: string) => {
      const session = sessionRef.current;
      if (session === null) return;
      const current = folderBrowserHistoryEntry(locationRef.current.state);
      if (current?.session === session && current.path === path) return;
      depthRef.current += 1;
      pushEntry(session, path, depthRef.current);
    },
    [pushEntry],
  );
}
