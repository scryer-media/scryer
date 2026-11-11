const STATE_KEY = "folderBrowser";

/**
 * One folder of an open folder browser, kept in the router's user state of a
 * history entry on the page that opened it. `session` ties the entry to one
 * opening of the browser; `depth` counts this browser's entries up to and
 * including this one, so closing it can step back over all of them at once.
 */
export type FolderBrowserHistoryEntry = {
  session: string;
  path: string;
  depth: number;
};

type HistoryLocation = {
  pathname: string;
  search: string;
  hash: string;
  state: unknown;
};

let sessionCounter = 0;

/** Unique per opening, also against entries left from an earlier page load. */
export function newFolderBrowserSession(now: number = Date.now()): string {
  sessionCounter += 1;
  return `${now.toString(36)}-${sessionCounter}`;
}

/** `state` with the browser's entry set, keeping unrelated route state. */
export function folderBrowserHistoryState(
  state: unknown,
  entry: FolderBrowserHistoryEntry,
) {
  return {
    ...(state && typeof state === "object" ? state : {}),
    [STATE_KEY]: entry,
  };
}

export function folderBrowserHistoryEntry(
  state: unknown,
): FolderBrowserHistoryEntry | null {
  if (!state || typeof state !== "object" || !(STATE_KEY in state)) {
    return null;
  }
  const entry = (state as Record<string, unknown>)[STATE_KEY];
  if (!entry || typeof entry !== "object") return null;
  const { session, path, depth } = entry as Record<string, unknown>;
  return typeof session === "string" &&
    typeof path === "string" &&
    typeof depth === "number" &&
    Number.isSafeInteger(depth) &&
    depth > 0
    ? { session, path, depth }
    : null;
}

/**
 * What a browser back or forward step means to the open browser: show the
 * entry's folder, or close because the step left this browser's entries.
 */
export function folderBrowserPopAction(
  state: unknown,
  session: string,
): { kind: "browse"; path: string; depth: number } | { kind: "close" } {
  const entry = folderBrowserHistoryEntry(state);
  return entry?.session === session
    ? { kind: "browse", path: entry.path, depth: entry.depth }
    : { kind: "close" };
}

/**
 * Whether a navigation only moves between a folder browser's entries on the
 * same page. Unsaved-changes guards let these through: they never leave the
 * page.
 */
export function isFolderBrowserHistoryStep(
  current: HistoryLocation,
  next: HistoryLocation,
): boolean {
  return (
    current.pathname === next.pathname &&
    current.search === next.search &&
    current.hash === next.hash &&
    (folderBrowserHistoryEntry(current.state) !== null ||
      folderBrowserHistoryEntry(next.state) !== null)
  );
}
