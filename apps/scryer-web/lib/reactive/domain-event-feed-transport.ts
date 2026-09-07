// ---------------------------------------------------------------------------
// Domain event feed transport — subscription lifecycle for ReactiveRefresh v2
// ---------------------------------------------------------------------------
//
// Framework-agnostic (no React, no `@/` aliases) so the reconnect/catch-up
// behavior is directly unit-testable with `node --test`. The provider owns a
// single instance of this transport per mount.
//
// Behavior:
// - Opens ONE subscription against `domainEventFeed`, feeding every payload
//   into the engine (which advances the `sequence` cursor).
// - On error/complete, resubscribes after a short delay passing
//   `afterSequence` = the engine's last-seen sequence, so the server replays
//   missed events from the store (lossless catch-up).
// - After repeated consecutive failures, degrades to refreshing every
//   registered alias on a slow interval and surfaces a console.warn. The
//   fallback stops as soon as the feed delivers again.

import {
  normalizeDomainEvent,
  type ReactiveRefreshEngine,
} from "./domain-event-feed.ts";

export const DOMAIN_EVENT_FEED_RECONNECT_DELAY_MS = 3_000;
export const DOMAIN_EVENT_FEED_FALLBACK_FAILURE_THRESHOLD = 3;
export const DOMAIN_EVENT_FEED_FALLBACK_INTERVAL_MS = 30_000;

type DomainEventFeedSink = {
  next: (result: unknown) => void;
  error: (error: unknown) => void;
  complete: () => void;
};

export type DomainEventFeedSubscribe = (
  request: { query: string; variables: { afterSequence: number | string | null } },
  sink: DomainEventFeedSink,
) => () => void;

type TransportScheduler = {
  setTimeout: (handler: () => void, timeoutMs: number) => unknown;
  clearTimeout: (handle: unknown) => void;
  setInterval: (handler: () => void, intervalMs: number) => unknown;
  clearInterval: (handle: unknown) => void;
};

const defaultTransportScheduler: TransportScheduler = {
  setTimeout: (handler, timeoutMs) => setTimeout(handler, timeoutMs),
  clearTimeout: (handle) =>
    clearTimeout(handle as ReturnType<typeof setTimeout>),
  setInterval: (handler, intervalMs) => setInterval(handler, intervalMs),
  clearInterval: (handle) =>
    clearInterval(handle as ReturnType<typeof setInterval>),
};

export type DomainEventFeedTransportOptions = {
  query: string;
  engine: ReactiveRefreshEngine;
  subscribe: DomainEventFeedSubscribe;
  reconnectDelayMs?: number;
  fallbackFailureThreshold?: number;
  fallbackIntervalMs?: number;
  scheduler?: TransportScheduler;
  warn?: (message: string) => void;
  logError?: (message: string, error: unknown) => void;
};

export type DomainEventFeedTransport = {
  stop: () => void;
  /** Whether the degraded interval fallback is currently active. */
  isDegraded: () => boolean;
};

export function startDomainEventFeedTransport(
  options: DomainEventFeedTransportOptions,
): DomainEventFeedTransport {
  const {
    query,
    engine,
    subscribe,
    reconnectDelayMs = DOMAIN_EVENT_FEED_RECONNECT_DELAY_MS,
    fallbackFailureThreshold = DOMAIN_EVENT_FEED_FALLBACK_FAILURE_THRESHOLD,
    fallbackIntervalMs = DOMAIN_EVENT_FEED_FALLBACK_INTERVAL_MS,
    scheduler = defaultTransportScheduler,
    warn = (message) => console.warn(message),
    logError = (message, error) => console.error(message, error),
  } = options;

  let disposed = false;
  let unsubscribe: (() => void) | null = null;
  let reconnectHandle: unknown = null;
  let fallbackHandle: unknown = null;
  let consecutiveFailures = 0;

  const startFallback = () => {
    if (fallbackHandle !== null) {
      return;
    }
    warn(
      "[reactive-refresh] domain event feed unavailable; degrading to interval refresh",
    );
    fallbackHandle = scheduler.setInterval(() => {
      engine.runAll();
    }, fallbackIntervalMs);
  };

  const stopFallback = () => {
    if (fallbackHandle !== null) {
      scheduler.clearInterval(fallbackHandle);
      fallbackHandle = null;
    }
  };

  const scheduleReconnect = () => {
    if (disposed) {
      return;
    }
    unsubscribe = null;
    consecutiveFailures += 1;
    if (consecutiveFailures >= fallbackFailureThreshold) {
      startFallback();
    }
    if (reconnectHandle !== null) {
      return;
    }
    reconnectHandle = scheduler.setTimeout(() => {
      reconnectHandle = null;
      connect();
    }, reconnectDelayMs);
  };

  const connect = () => {
    if (disposed) {
      return;
    }
    // A reconnect before any event has established a cursor cannot replay the
    // interval while the feed was unavailable. Refresh now and keep the
    // interval active until a delivered event anchors subsequent catch-up.
    if (consecutiveFailures > 0 && engine.afterSequence() === null) {
      startFallback();
      engine.runAll();
    }
    unsubscribe = subscribe(
      {
        query,
        // Lossless catch-up: replay everything after the last seen sequence.
        variables: { afterSequence: engine.afterSequence() },
      },
      {
        next(result) {
          const payload = (
            result as { data?: { domainEventFeed?: unknown } } | null
          )?.data?.domainEventFeed;
          if (payload) {
            const establishesFirstCursor =
              fallbackHandle !== null && engine.afterSequence() === null;
            consecutiveFailures = 0;
            stopFallback();
            engine.handleEvent(normalizeDomainEvent(payload));
            // The immediate reconnect refresh can finish before the server
            // snapshots its tail. Refresh once more after the first cursor so
            // a change in that setup window cannot remain stale forever.
            if (establishesFirstCursor) {
              engine.runAll();
            }
          }
        },
        error(error) {
          logError("[reactive-refresh] domain event feed error:", error);
          scheduleReconnect();
        },
        complete() {
          scheduleReconnect();
        },
      },
    );
  };

  connect();

  return {
    stop() {
      disposed = true;
      if (unsubscribe) {
        unsubscribe();
        unsubscribe = null;
      }
      if (reconnectHandle !== null) {
        scheduler.clearTimeout(reconnectHandle);
        reconnectHandle = null;
      }
      stopFallback();
    },
    isDegraded() {
      return fallbackHandle !== null;
    },
  };
}
