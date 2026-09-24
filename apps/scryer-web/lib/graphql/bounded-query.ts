export const ACTIVITY_READ_TIMEOUT_MS = 30_000;

export class QueryTimeoutError extends Error {
  constructor() {
    super("Refresh timed out");
    this.name = "QueryTimeoutError";
  }
}

/** End the frontend subscription on timeout or scope cancellation. */
export function boundedQuery<T extends { stale?: boolean; hasNext?: boolean }>(
  source: { subscribe: (next: (value: T) => void) => { unsubscribe: () => void } },
  signal: AbortSignal,
  timeoutMs = ACTIVITY_READ_TIMEOUT_MS,
): Promise<T> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let subscription: { unsubscribe: () => void } | undefined;
    const finish = (value?: T, error?: unknown) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal.removeEventListener("abort", abort);
      subscription?.unsubscribe();
      if (error) reject(error);
      else resolve(value!);
    };
    const abort = () => finish(undefined, new DOMException("Request canceled", "AbortError"));
    const timer = setTimeout(() => finish(undefined, new QueryTimeoutError()), timeoutMs);
    if (signal.aborted) {
      abort();
      return;
    }
    signal.addEventListener("abort", abort, { once: true });
    try {
      subscription = source.subscribe((value) => {
        if (!value.stale && !value.hasNext) finish(value);
      });
      if (settled) subscription.unsubscribe();
    } catch (error) {
      finish(undefined, error);
    }
  });
}
