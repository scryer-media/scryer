type CatalogSummaryLoad = () => Promise<() => void>;

export type CatalogSummaryLoaderOptions = {
  /** Shortest time between the starts of two loads of the same scope. */
  minimumGapMs?: number;
  now?: () => number;
};

const DEFAULT_MINIMUM_GAP_MS = 1_500;

function defaultNow() {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

/**
 * One summary load in flight per owner. An invalidation during a load queues
 * the latest reload; the finished load is still published while its scope is
 * current, and a reload of the same scope starts no sooner than
 * max(minimumGapMs, 2 x the last load's duration) after the previous start. A
 * scope change drops the in-flight result and loads the new scope next,
 * without the gap.
 */
export function createCatalogSummaryLoader(
  onError: (error: unknown) => void,
  options: CatalogSummaryLoaderOptions = {},
) {
  const minimumGapMs = options.minimumGapMs ?? DEFAULT_MINIMUM_GAP_MS;
  const now = options.now ?? defaultNow;
  let active = true;
  let generation = 0;
  let scope = "";
  let ready = false;
  let pending: Promise<void> | undefined;
  let queued: CatalogSummaryLoad | undefined;
  let previous:
    { key: string; startedAt: number; duration: number } | undefined;
  let wake: (() => void) | undefined;

  const current = (started: number) => active && generation === started;
  const interrupt = () => {
    const resume = wake;
    wake = undefined;
    resume?.();
  };
  const pause = (ms: number) =>
    new Promise<void>((resolve) => {
      const resume = () => {
        clearTimeout(timer);
        if (wake === resume) wake = undefined;
        resolve();
      };
      const timer = setTimeout(resume, ms);
      wake = resume;
    });

  return {
    activate() {
      active = true;
    },
    dispose() {
      active = false;
      generation += 1;
      pending = undefined;
      queued = undefined;
      previous = undefined;
      ready = false;
      scope = "";
      interrupt();
    },
    run(key: string, load: CatalogSummaryLoad, invalidate = false) {
      if (!active) return Promise.resolve();
      const changed = scope !== key;
      scope = key;
      if (changed || invalidate) {
        ready = false;
        queued = load;
      }
      if (pending) {
        // A new scope does not wait out the previous scope's gap.
        if (changed) interrupt();
        return pending;
      }
      if (ready) return Promise.resolve();
      queued = load;
      const started = generation;
      // Started on a microtask so `pending` is set before the first load runs.
      pending = Promise.resolve().then(async () => {
        try {
          while (current(started) && queued) {
            const key = scope;
            if (previous?.key === key) {
              const gap = Math.max(minimumGapMs, 2 * previous.duration);
              const delay = previous.startedAt + gap - now();
              if (delay > 0) {
                await pause(delay);
                if (!current(started)) return;
                // Look again: the scope may have changed, or changed and
                // come back, while paused; the same scope still waits out
                // its gap.
                continue;
              }
            }
            const next = queued;
            queued = undefined;
            if (!next) break;
            const startedAt = now();
            try {
              const publish = await next();
              if (current(started) && scope === key) {
                publish();
                if (!queued) ready = true;
              }
            } catch (error) {
              if (current(started) && scope === key && !queued) onError(error);
            }
            if (!current(started)) return;
            previous = { key, startedAt, duration: now() - startedAt };
          }
        } finally {
          // Cleared in the same turn as the last look at `queued`, so a
          // reload asked for from here on starts a new run instead of
          // joining one that is already done.
          if (generation === started) pending = undefined;
        }
      });
      return pending;
    },
  };
}
