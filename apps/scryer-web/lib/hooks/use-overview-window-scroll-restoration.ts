import * as React from "react";
import { useLocation } from "react-router";

const OVERVIEW_SCROLL_STORAGE_PREFIX = "scryer:overview-scroll:";
const MAX_RESTORE_ATTEMPTS = 12;
const overviewScrollMemory = new Map<string, number>();

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
}: {
  enabled: boolean;
  ready: boolean;
  storageKeySuffix: string;
  kind: ScrollTargetKind;
  getTarget: () => ScrollTarget | null;
  getMaxScrollTop?: (target: ScrollTarget) => number;
  restoreScrollTop?: (nextTop: number) => void;
}) {
  const location = useLocation();
  const pendingScrollTopRef = React.useRef<number | null>(null);
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
    restoreAttemptsRef.current = 0;
  }, [enabled, location.pathname, storageKey, storageKeySuffix]);

  React.useEffect(() => {
    if (!enabled || typeof window === "undefined") {
      return;
    }

    const target = getTarget();
    if (!target) {
      return;
    }

    const persistScroll = () => {
      const nextScrollTop = readScrollTop(target, kind);
      overviewScrollMemory.set(storageKey, nextScrollTop);
      window.sessionStorage.setItem(storageKey, String(nextScrollTop));
    };

    const unsubscribe = subscribeToScroll(target, kind, persistScroll);
    return () => {
      unsubscribe();
    };
  }, [enabled, getTarget, kind, storageKey]);

  React.useLayoutEffect(() => {
    if (!enabled || !ready || typeof window === "undefined") {
      return;
    }

    let frameId = 0;

    const restore = () => {
      const target = getTarget();
      const pendingScrollTop = pendingScrollTopRef.current;
      if (!target || pendingScrollTop == null) {
        return;
      }

      const nextMaxScroll = getMaxScrollTop
        ? Math.max(0, getMaxScrollTop(target))
        : maxScrollTop(target, kind);
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
    };
  }, [enabled, getMaxScrollTop, getTarget, kind, ready, restoreScrollTop]);
}

export function useOverviewWindowScrollRestoration({
  enabled,
  ready,
  storageKeySuffix = "window",
}: {
  enabled: boolean;
  ready: boolean;
  storageKeySuffix?: string;
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
}: {
  enabled: boolean;
  ready: boolean;
  storageKeySuffix: string;
  scrollRef: React.RefObject<HTMLElement | null>;
  getMaxScrollTop?: (target: HTMLElement) => number;
  restoreScrollTop?: (nextTop: number) => void;
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
  });
}
