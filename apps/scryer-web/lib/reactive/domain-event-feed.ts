// ---------------------------------------------------------------------------
// Domain event feed — freshness-first reactive-refresh engine
// ---------------------------------------------------------------------------
//
// This module is intentionally framework-agnostic (no React, no `@/` aliases)
// so the dispatch logic can be unit-tested directly with `node --test`.
//
// The engine drives ReactiveRefresh v2: a single `domainEventFeed`
// subscription feeds `handleEvent`, views register `(aliasKey, predicate)`
// pairs, and each domain event dispatches the `run` callbacks of the aliases
// whose predicate matches. Bursts are coalesced through a shared debounce, and
// the last-seen `sequence` is tracked so the transport can resubscribe with
// `afterSequence` for lossless catch-up.

export type DomainEvent = {
  sequence: number | string | null;
  eventId: string | null;
  eventType: string | null;
  titleId: string | null;
  facet: string | null;
  streamKind: string | null;
  streamId: string | null;
  occurredAt?: string | null;
  payloadJson?: unknown;
};

export type DomainEventPredicate = (event: DomainEvent) => boolean;

// ---------------------------------------------------------------------------
// Prebuilt predicates + combinators
// ---------------------------------------------------------------------------

/** Matches events carrying the given title id (empty/undefined id never matches). */
export function forTitle(
  titleId: string | null | undefined,
): DomainEventPredicate {
  const target = typeof titleId === "string" ? titleId : null;
  return (event) => target !== null && event.titleId === target;
}

/** Matches events whose `eventType` is one of the provided types. */
export function forEventTypes(
  ...types: ReadonlyArray<string>
): DomainEventPredicate {
  const set = new Set(types);
  return (event) => event.eventType !== null && set.has(event.eventType);
}

/** Matches events published on the given `streamKind`. */
export function forStreamKind(kind: string): DomainEventPredicate {
  return (event) => event.streamKind === kind;
}

/** Matches events for the given facet (case-insensitive). */
export function forFacet(facet: string): DomainEventPredicate {
  const target = facet.trim().toLowerCase();
  return (event) =>
    typeof event.facet === "string" &&
    event.facet.trim().toLowerCase() === target;
}

/** True when any of the provided predicates matches. */
export function anyOf(
  ...predicates: ReadonlyArray<DomainEventPredicate>
): DomainEventPredicate {
  return (event) => predicates.some((predicate) => predicate(event));
}

/** True only when every provided predicate matches. */
export function allOf(
  ...predicates: ReadonlyArray<DomainEventPredicate>
): DomainEventPredicate {
  return (event) => predicates.every((predicate) => predicate(event));
}

/** Inverts a predicate. */
export function not(predicate: DomainEventPredicate): DomainEventPredicate {
  return (event) => !predicate(event);
}

/** Matches every event. Reserve for genuinely global views. */
export const always: DomainEventPredicate = () => true;

// ---------------------------------------------------------------------------
// Sequence cursor helpers
// ---------------------------------------------------------------------------

/**
 * Coerce a `Long` sequence (which may arrive as a number or a numeric string)
 * into a finite number for monotonic comparison, or `null` when unusable.
 */
export function normalizeSequence(
  sequence: number | string | null | undefined,
): number | null {
  if (typeof sequence === "number") {
    return Number.isFinite(sequence) ? sequence : null;
  }
  if (typeof sequence === "string" && sequence.trim() !== "") {
    const parsed = Number(sequence);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

/**
 * Defensively map a raw `domainEventFeed` payload object into a `DomainEvent`.
 */
export function normalizeDomainEvent(raw: unknown): DomainEvent {
  const record =
    typeof raw === "object" && raw !== null
      ? (raw as Record<string, unknown>)
      : {};
  const asStringOrNull = (value: unknown): string | null =>
    typeof value === "string" ? value : null;
  const sequence =
    typeof record.sequence === "number" || typeof record.sequence === "string"
      ? (record.sequence as number | string)
      : null;
  return {
    sequence,
    eventId: asStringOrNull(record.eventId),
    eventType: asStringOrNull(record.eventType),
    titleId: asStringOrNull(record.titleId),
    facet: asStringOrNull(record.facet),
    streamKind: asStringOrNull(record.streamKind),
    streamId: asStringOrNull(record.streamId),
    occurredAt: asStringOrNull(record.occurredAt),
    payloadJson: record.payloadJson,
  };
}

// ---------------------------------------------------------------------------
// Registry / engine
// ---------------------------------------------------------------------------

export type ReactiveRefreshRegistration = {
  /** Stable, unique key. Re-registering the same key replaces the entry. */
  aliasKey: string;
  /** Decides whether a given domain event should refresh this alias. */
  predicate: DomainEventPredicate;
  /** Performs the refresh. Invoked (coalesced) after a matching event. */
  run: () => void;
  /** Optional error sink for throws from `run`. */
  onError?: (error: unknown) => void;
};

type Scheduler = {
  setTimeout: (handler: () => void, timeoutMs: number) => unknown;
  clearTimeout: (handle: unknown) => void;
};

const defaultScheduler: Scheduler = {
  setTimeout: (handler, timeoutMs) => setTimeout(handler, timeoutMs),
  clearTimeout: (handle) =>
    clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export const REACTIVE_REFRESH_DEBOUNCE_MS = 300;

export type ReactiveRefreshEngine = {
  register: (registration: ReactiveRefreshRegistration) => () => void;
  unregister: (aliasKey: string) => void;
  /** Pure: alias keys whose predicate matches, without mutating cursor/queue. */
  matchingAliasKeys: (event: DomainEvent) => string[];
  /** Ingest an event: advance the cursor, queue matching aliases. */
  handleEvent: (event: DomainEvent) => string[];
  /** Raw last-seen sequence for the `afterSequence` resubscribe variable. */
  afterSequence: () => number | string | null;
  /** Normalized last-seen sequence (for diagnostics/tests). */
  lastSequence: () => number | null;
  /** Fallback: queue every registered alias to run (degraded interval mode). */
  runAll: () => void;
  /** Force any pending coalesced runs to fire immediately. */
  flush: () => void;
  /** Registered alias count. */
  size: () => number;
  /** Clear registrations, pending queue, and cursor (test helper). */
  reset: () => void;
};

export function createReactiveRefreshEngine(options?: {
  debounceMs?: number;
  scheduler?: Scheduler;
}): ReactiveRefreshEngine {
  const debounceMs = options?.debounceMs ?? REACTIVE_REFRESH_DEBOUNCE_MS;
  const scheduler = options?.scheduler ?? defaultScheduler;

  const registrations = new Map<string, ReactiveRefreshRegistration>();
  const pendingAliasKeys = new Set<string>();
  let flushHandle: unknown = null;
  let lastSequenceRaw: number | string | null = null;
  let lastSequenceNum: number | null = null;

  const flush = () => {
    if (flushHandle !== null) {
      scheduler.clearTimeout(flushHandle);
      flushHandle = null;
    }
    if (pendingAliasKeys.size === 0) {
      return;
    }
    const aliasKeys = Array.from(pendingAliasKeys);
    pendingAliasKeys.clear();
    for (const aliasKey of aliasKeys) {
      const registration = registrations.get(aliasKey);
      if (!registration) {
        continue;
      }
      try {
        registration.run();
      } catch (error) {
        if (registration.onError) {
          registration.onError(error);
        } else {
          console.error(
            `[reactive-refresh] run failed for alias "${aliasKey}":`,
            error,
          );
        }
      }
    }
  };

  const scheduleFlush = () => {
    if (flushHandle !== null || pendingAliasKeys.size === 0) {
      return;
    }
    flushHandle = scheduler.setTimeout(() => {
      flushHandle = null;
      flush();
    }, debounceMs);
  };

  const matchingAliasKeys = (event: DomainEvent): string[] => {
    const matched: string[] = [];
    for (const registration of registrations.values()) {
      if (registration.predicate(event)) {
        matched.push(registration.aliasKey);
      }
    }
    return matched;
  };

  const advanceCursor = (event: DomainEvent) => {
    const seq = normalizeSequence(event.sequence);
    if (seq === null) {
      return { stale: false };
    }
    if (lastSequenceNum !== null && seq <= lastSequenceNum) {
      // Already processed (e.g. transport replayed after reconnect).
      return { stale: true };
    }
    lastSequenceNum = seq;
    lastSequenceRaw = event.sequence;
    return { stale: false };
  };

  return {
    register(registration) {
      registrations.set(registration.aliasKey, registration);
      return () => {
        if (registrations.get(registration.aliasKey) === registration) {
          registrations.delete(registration.aliasKey);
          pendingAliasKeys.delete(registration.aliasKey);
        }
      };
    },
    unregister(aliasKey) {
      registrations.delete(aliasKey);
      pendingAliasKeys.delete(aliasKey);
    },
    matchingAliasKeys,
    handleEvent(event) {
      const { stale } = advanceCursor(event);
      if (stale) {
        return [];
      }
      const matched = matchingAliasKeys(event);
      if (matched.length === 0) {
        return [];
      }
      for (const aliasKey of matched) {
        pendingAliasKeys.add(aliasKey);
      }
      scheduleFlush();
      return matched;
    },
    afterSequence() {
      return lastSequenceRaw;
    },
    lastSequence() {
      return lastSequenceNum;
    },
    runAll() {
      if (registrations.size === 0) {
        return;
      }
      for (const aliasKey of registrations.keys()) {
        pendingAliasKeys.add(aliasKey);
      }
      scheduleFlush();
    },
    flush,
    size() {
      return registrations.size;
    },
    reset() {
      registrations.clear();
      pendingAliasKeys.clear();
      if (flushHandle !== null) {
        scheduler.clearTimeout(flushHandle);
        flushHandle = null;
      }
      lastSequenceRaw = null;
      lastSequenceNum = null;
    },
  };
}
