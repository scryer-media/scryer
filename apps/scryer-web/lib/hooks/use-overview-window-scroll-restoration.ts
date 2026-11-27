import * as React from "react";
import { useLocation } from "react-router";

const OVERVIEW_SCROLL_STORAGE_PREFIX = "scryer:overview-scroll:";
const MAX_RESTORE_ATTEMPTS = 12;
const MAX_SCROLL_ANCHORS = 3;
const ANCHOR_CAPTURE_DELAY_MS = 120;
// A restored row is held in place until the layout has stayed put for this
// many frames in a row, and never longer than the cap. Frames, not time: a
// tab that renders slowly settles slowly.
const ANCHOR_HOLD_QUIET_FRAMES = 45;
const ANCHOR_HOLD_MAX_MS = 6000;
const ANCHOR_RELEASE_EVENTS = ["wheel", "touchstart", "pointerdown"] as const;
const overviewScrollMemory = new Map<string, number>();
const overviewScrollAnchorMemory = new Map<string, ScrollAnchor[]>();

/**
 * A row the scroll position is tied to: its key and how far its top sat below
 * the top of the scroll viewport. A pixel offset alone drifts when rows above
 * it change height, which rows sized by estimate until rendered do.
 */
export type ScrollAnchor = { key: string; offset: number };

type AnchorRow = { key: string; top: number; bottom: number };

/**
 * The first rows still showing at `viewportTop`, as anchors. `rowAt` reads
 * rows in document order, and their bottoms never decrease, so the first
 * showing row is found by bisection instead of measuring every row.
 */
export function pickScrollAnchors(
  rowCount: number,
  rowAt: (index: number) => AnchorRow | null,
  viewportTop: number,
): ScrollAnchor[] {
  let low = 0;
  let high = rowCount;
  while (low < high) {
    const middle = (low + high) >> 1;
    const row = rowAt(middle);
    if (row && row.bottom > viewportTop) {
      high = middle;
    } else {
      low = middle + 1;
    }
  }
  const anchors: ScrollAnchor[] = [];
  for (
    let index = low;
    index < rowCount && anchors.length < MAX_SCROLL_ANCHORS;
    index += 1
  ) {
    const row = rowAt(index);
    if (row) {
      anchors.push({ key: row.key, offset: row.top - viewportTop });
    }
  }
  return anchors;
}

/**
 * The scroll offset that puts the first anchor still present back where it
 * was, or null when none of them is. `rowTop` is a row's current top in the
 * same frame as `viewportTop`.
 */
export function resolveAnchoredScrollTop(
  anchors: readonly ScrollAnchor[],
  currentScrollTop: number,
  viewportTop: number,
  rowTop: (key: string) => number | null,
): number | null {
  for (const anchor of anchors) {
    const top = rowTop(anchor.key);
    if (top != null) {
      return Math.max(
        0,
        currentScrollTop + (top - viewportTop) - anchor.offset,
      );
    }
  }
  return null;
}

function parseScrollAnchors(raw: string | null): ScrollAnchor[] | null {
  if (!raw) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return null;
    }
    const anchors = parsed.filter(
      (entry): entry is ScrollAnchor =>
        typeof entry === "object" &&
        entry !== null &&
        typeof (entry as ScrollAnchor).key === "string" &&
        Number.isFinite((entry as ScrollAnchor).offset),
    );
    return anchors.length > 0 ? anchors : null;
  } catch {
    return null;
  }
}

function anchorStorageKey(storageKey: string) {
  return `${storageKey}:anchors`;
}

function readSavedScrollAnchors(storageKey: string) {
  const cached = overviewScrollAnchorMemory.get(storageKey);
  if (cached) {
    return cached;
  }
  if (typeof window === "undefined") {
    return null;
  }
  return parseScrollAnchors(
    window.sessionStorage.getItem(anchorStorageKey(storageKey)),
  );
}

/** Where anchored rows are looked up, and the top edge they are measured from. */
function anchorFrame(target: ScrollTarget, kind: ScrollTargetKind) {
  if (kind === "window") {
    return { root: document as ParentNode, viewportTop: 0 };
  }
  const element = target as HTMLElement;
  return {
    root: element as ParentNode,
    viewportTop: element.getBoundingClientRect().top,
  };
}

function captureScrollAnchors(
  target: ScrollTarget,
  kind: ScrollTargetKind,
  anchorAttribute: string,
) {
  const { root, viewportTop } = anchorFrame(target, kind);
  const rows = root.querySelectorAll<HTMLElement>(`[${anchorAttribute}]`);
  return pickScrollAnchors(
    rows.length,
    (index) => {
      const row = rows[index];
      const key = row?.getAttribute(anchorAttribute);
      if (!row || !key) {
        return null;
      }
      const rect = row.getBoundingClientRect();
      return { key, top: rect.top, bottom: rect.bottom };
    },
    viewportTop,
  );
}

function anchoredScrollTop(
  target: ScrollTarget,
  kind: ScrollTargetKind,
  anchorAttribute: string,
  anchors: readonly ScrollAnchor[],
) {
  const { root, viewportTop } = anchorFrame(target, kind);
  return resolveAnchoredScrollTop(
    anchors,
    readScrollTop(target, kind),
    viewportTop,
    (key) =>
      root
        .querySelector<HTMLElement>(`[${anchorAttribute}="${CSS.escape(key)}"]`)
        ?.getBoundingClientRect().top ?? null,
  );
}

type ScrollTarget = Window | HTMLElement;
type ScrollTargetKind = "window" | "element";

function storageKeyForPath(pathname: string, suffix: string) {
  return `${OVERVIEW_SCROLL_STORAGE_PREFIX}${pathname}:${suffix}`;
}

export function readOverviewSavedScroll(
  pathname: string,
  storageKeySuffix = "window",
) {
  const storageKey = storageKeyForPath(pathname, storageKeySuffix);
  const cached = overviewScrollMemory.get(storageKey);
  if (typeof cached === "number" && Number.isFinite(cached) && cached >= 0) {
    return cached;
  }

  if (typeof window === "undefined") {
    return null;
  }

  const saved = window.sessionStorage.getItem(storageKey);
  if (!saved) {
    return null;
  }

  const parsed = Number(saved);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return null;
  }

  overviewScrollMemory.set(storageKey, parsed);
  return parsed;
}

export function persistOverviewWindowScroll(
  pathname: string,
  storageKeySuffix = "window",
) {
  persistOverviewScrollValue(pathname, storageKeySuffix, window.scrollY);
}

export function persistOverviewScrollValue(
  pathname: string,
  storageKeySuffix: string,
  offset: number | null | undefined,
) {
  if (!Number.isFinite(offset) || offset == null || offset < 0) {
    return;
  }

  const storageKey = storageKeyForPath(pathname, storageKeySuffix);
  overviewScrollMemory.set(storageKey, offset);

  if (typeof window === "undefined") {
    return;
  }

  window.sessionStorage.setItem(storageKey, String(offset));
}

export function persistOverviewElementScroll(
  pathname: string,
  storageKeySuffix: string,
  element: HTMLElement | null,
) {
  if (!element) {
    return;
  }
  persistOverviewScrollValue(pathname, storageKeySuffix, element.scrollTop);
}

function readScrollTop(target: ScrollTarget, kind: ScrollTargetKind) {
  if (kind === "window") {
    return (target as Window).scrollY;
  }

  return (target as HTMLElement).scrollTop;
}

function writeScrollTop(
  target: ScrollTarget,
  kind: ScrollTargetKind,
  nextTop: number,
) {
  if (kind === "window") {
    (target as Window).scrollTo({ top: nextTop, left: 0, behavior: "auto" });
    return;
  }

  (target as HTMLElement).scrollTop = nextTop;
}

function maxScrollTop(target: ScrollTarget, kind: ScrollTargetKind) {
  if (kind === "window") {
    return Math.max(
      0,
      document.documentElement.scrollHeight - window.innerHeight,
    );
  }

  const element = target as HTMLElement;
  return Math.max(0, element.scrollHeight - element.clientHeight);
}

function subscribeToScroll(
  target: ScrollTarget,
  kind: ScrollTargetKind,
  onScroll: () => void,
) {
  if (kind === "window") {
    target.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      target.removeEventListener("scroll", onScroll);
    };
  }

  target.addEventListener("scroll", onScroll, { passive: true });
  return () => {
    target.removeEventListener("scroll", onScroll);
  };
}

function useOverviewScrollRestoration({
  enabled,
  ready,
  storageKeySuffix,
  kind,
  getTarget,
  getMaxScrollTop,
  restoreScrollTop,
  anchorAttribute,
}: {
  enabled: boolean;
  ready: boolean;
  storageKeySuffix: string;
  kind: ScrollTargetKind;
  getTarget: () => ScrollTarget | null;
  getMaxScrollTop?: (target: ScrollTarget) => number;
  restoreScrollTop?: (nextTop: number) => void;
  /** The attribute whose value keys each row. */
  anchorAttribute?: string;
}) {
  const location = useLocation();
  const pendingScrollTopRef = React.useRef<number | null>(null);
  const pendingAnchorsRef = React.useRef<ScrollAnchor[] | null>(null);
  const restoreAttemptsRef = React.useRef(0);
  const storageKey = React.useMemo(
    () => storageKeyForPath(location.pathname, storageKeySuffix),
    [location.pathname, storageKeySuffix],
  );

  React.useLayoutEffect(() => {
    if (!enabled || typeof window === "undefined") {
      pendingScrollTopRef.current = null;
      restoreAttemptsRef.current = 0;
      return;
    }

    const parsed = readOverviewSavedScroll(location.pathname, storageKeySuffix);
    if (parsed == null) {
      pendingScrollTopRef.current = null;
      restoreAttemptsRef.current = 0;
      return;
    }

    pendingScrollTopRef.current = parsed;
    pendingAnchorsRef.current = anchorAttribute
      ? readSavedScrollAnchors(storageKey)
      : null;
    restoreAttemptsRef.current = 0;
  }, [
    anchorAttribute,
    enabled,
    location.pathname,
    storageKey,
    storageKeySuffix,
  ]);

  React.useEffect(() => {
    if (!enabled || typeof window === "undefined") {
      return;
    }

    const target = getTarget();
    if (!target) {
      return;
    }

    // Anchors cost a few layout reads, so they are taken once scrolling
    // settles instead of on every scroll event.
    let anchorTimer = 0;
    const persistAnchors = () => {
      anchorTimer = 0;
      if (!anchorAttribute) {
        return;
      }
      // A restore still in flight owns the saved anchors.
      if (pendingScrollTopRef.current != null) {
        return;
      }
      const anchors = captureScrollAnchors(target, kind, anchorAttribute);
      overviewScrollAnchorMemory.set(storageKey, anchors);
      window.sessionStorage.setItem(
        anchorStorageKey(storageKey),
        JSON.stringify(anchors),
      );
    };

    const persistScroll = () => {
      const nextScrollTop = readScrollTop(target, kind);
      overviewScrollMemory.set(storageKey, nextScrollTop);
      window.sessionStorage.setItem(storageKey, String(nextScrollTop));
      if (anchorAttribute && anchorTimer === 0) {
        anchorTimer = window.setTimeout(
          persistAnchors,
          ANCHOR_CAPTURE_DELAY_MS,
        );
      }
    };

    // Rows can still settle after the last scroll event, so the anchors are
    // taken again at the moment the user acts on a row.
    const persistAnchorsNow = (event: Event) => {
      // Only a key that opens a row counts, and only where this target is
      // the one that scrolls.
      if (
        event instanceof KeyboardEvent &&
        event.key !== "Enter" &&
        event.key !== " "
      ) {
        return;
      }
      if (maxScrollTop(target, kind) <= 0) {
        return;
      }
      if (anchorTimer !== 0) {
        window.clearTimeout(anchorTimer);
      }
      persistAnchors();
    };
    const anchorElement = anchorAttribute ? target : null;
    anchorElement?.addEventListener("pointerdown", persistAnchorsNow, {
      capture: true,
      passive: true,
    });
    anchorElement?.addEventListener("keydown", persistAnchorsNow, {
      capture: true,
      passive: true,
    });

    const unsubscribe = subscribeToScroll(target, kind, persistScroll);
    return () => {
      unsubscribe();
      anchorElement?.removeEventListener("pointerdown", persistAnchorsNow, {
        capture: true,
      });
      anchorElement?.removeEventListener("keydown", persistAnchorsNow, {
        capture: true,
      });
      if (anchorTimer !== 0) {
        window.clearTimeout(anchorTimer);
      }
    };
  }, [anchorAttribute, enabled, getTarget, kind, storageKey]);

  React.useLayoutEffect(() => {
    if (!enabled || !ready || typeof window === "undefined") {
      return;
    }

    let frameId = 0;
    let holdStartedAt = 0;
    let quietFrames = 0;
    let releaseHold: (() => void) | null = null;

    const finishAnchoredRestore = () => {
      pendingScrollTopRef.current = null;
      pendingAnchorsRef.current = null;
      restoreAttemptsRef.current = 0;
      releaseHold?.();
      releaseHold = null;
    };

    const restore = () => {
      frameId = 0;
      const target = getTarget();
      const pendingScrollTop = pendingScrollTopRef.current;
      if (!target || pendingScrollTop == null) {
        return;
      }

      const nextMaxScroll = getMaxScrollTop
        ? Math.max(0, getMaxScrollTop(target))
        : maxScrollTop(target, kind);
      const anchors = pendingAnchorsRef.current;
      const anchoredTop =
        anchorAttribute && anchors
          ? anchoredScrollTop(target, kind, anchorAttribute, anchors)
          : null;
      if (anchoredTop != null) {
        // Rows above the anchor take their real height as they render, some
        // of them well after the first frames, so the row is held where it
        // was until the layout stays put or the user takes over.
        const now = window.performance.now();
        if (!releaseHold) {
          const element = target;
          const onInput = () => finishAnchoredRestore();
          for (const type of ANCHOR_RELEASE_EVENTS) {
            element.addEventListener(type, onInput, { passive: true });
          }
          window.addEventListener("keydown", onInput);
          releaseHold = () => {
            for (const type of ANCHOR_RELEASE_EVENTS) {
              element.removeEventListener(type, onInput);
            }
            window.removeEventListener("keydown", onInput);
          };
          holdStartedAt = now;
          quietFrames = 0;
        }
        const anchoredTarget = Math.min(anchoredTop, nextMaxScroll);
        if (Math.abs(readScrollTop(target, kind) - anchoredTarget) > 2) {
          writeScrollTop(target, kind, anchoredTarget);
          quietFrames = 0;
        } else {
          quietFrames += 1;
        }
        if (
          quietFrames >= ANCHOR_HOLD_QUIET_FRAMES ||
          now - holdStartedAt >= ANCHOR_HOLD_MAX_MS
        ) {
          finishAnchoredRestore();
          return;
        }
        frameId = window.requestAnimationFrame(restore);
        return;
      }
      if (releaseHold) {
        // The rows are briefly gone while the list re-renders. Keep waiting
        // for the held row instead of settling for the pixel offset.
        if (window.performance.now() - holdStartedAt >= ANCHOR_HOLD_MAX_MS) {
          finishAnchoredRestore();
          return;
        }
        frameId = window.requestAnimationFrame(restore);
        return;
      }

      const nextTop = Math.min(pendingScrollTop, nextMaxScroll);
      if (restoreScrollTop) {
        restoreScrollTop(nextTop);
      } else {
        writeScrollTop(target, kind, nextTop);
      }

      const canFullyRestore = nextMaxScroll >= pendingScrollTop - 4;
      const reachedClampedTarget =
        Math.abs(readScrollTop(target, kind) - nextTop) <= 2;
      if (
        (canFullyRestore && reachedClampedTarget) ||
        restoreAttemptsRef.current >= MAX_RESTORE_ATTEMPTS
      ) {
        pendingScrollTopRef.current = null;
        restoreAttemptsRef.current = 0;
        return;
      }

      restoreAttemptsRef.current += 1;
      frameId = window.requestAnimationFrame(restore);
    };

    restore();
    return () => {
      if (frameId !== 0) {
        window.cancelAnimationFrame(frameId);
      }
      releaseHold?.();
    };
  }, [
    anchorAttribute,
    enabled,
    getMaxScrollTop,
    getTarget,
    kind,
    ready,
    restoreScrollTop,
  ]);
}

export function useOverviewWindowScrollRestoration({
  enabled,
  ready,
  storageKeySuffix = "window",
  anchorAttribute,
}: {
  enabled: boolean;
  ready: boolean;
  storageKeySuffix?: string;
  /**
   * The attribute whose value keys each row. When given, the position is
   * restored to the same row instead of the same pixel offset.
   */
  anchorAttribute?: string;
}) {
  const getTarget = React.useCallback(
    () => (typeof window === "undefined" ? null : window),
    [],
  );

  useOverviewScrollRestoration({
    enabled,
    ready,
    storageKeySuffix,
    kind: "window",
    anchorAttribute,
    getTarget,
  });
}

export function useOverviewElementScrollRestoration({
  enabled,
  ready,
  storageKeySuffix,
  scrollRef,
  getMaxScrollTop,
  restoreScrollTop,
  anchorAttribute,
}: {
  enabled: boolean;
  ready: boolean;
  storageKeySuffix: string;
  scrollRef: React.RefObject<HTMLElement | null>;
  getMaxScrollTop?: (target: HTMLElement) => number;
  restoreScrollTop?: (nextTop: number) => void;
  /**
   * The attribute whose value keys each row. When given, the position is
   * restored to the same row instead of the same pixel offset.
   */
  anchorAttribute?: string;
}) {
  const getTarget = React.useCallback(() => scrollRef.current, [scrollRef]);
  const resolveMaxScrollTop = React.useCallback(
    (target: ScrollTarget) =>
      getMaxScrollTop
        ? getMaxScrollTop(target as HTMLElement)
        : maxScrollTop(target, "element"),
    [getMaxScrollTop],
  );

  useOverviewScrollRestoration({
    enabled,
    ready,
    storageKeySuffix,
    kind: "element",
    getTarget,
    getMaxScrollTop: resolveMaxScrollTop,
    restoreScrollTop,
    anchorAttribute,
  });
}
