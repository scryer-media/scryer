/**
 * A poll that backs off while it has something to watch and stops when it
 * has nothing.
 *
 * It corrects state that a live stream normally keeps current, for the times
 * the stream never connects, drops, or misses the final event. Each poll
 * returns whether there is still something to watch; when there is not, no
 * timer is left behind. While the page is hidden a due poll is held back and
 * runs once when `resume` is called.
 */

export type BackoffPollerClock = {
  setTimeout: (callback: () => void, delayMs: number) => unknown;
  clearTimeout: (handle: unknown) => void;
};

export type BackoffPollerOptions = {
  /** Waits before each poll; the last one repeats. */
  delaysMs: readonly number[];
  /** Resolves true while there is still something to watch. */
  poll: () => Promise<boolean>;
  isHidden?: () => boolean;
  clock?: BackoffPollerClock;
};

export type BackoffPoller = {
  /** Start polling, or start the waits over from the first one. */
  restart: () => void;
  /** Stop polling; a poll already under way is ignored when it ends. */
  stop: () => void;
  /** Run a poll that came due while the page was hidden. */
  resume: () => void;
  isActive: () => boolean;
};

const defaultClock: BackoffPollerClock = {
  setTimeout: (callback, delayMs) => globalThis.setTimeout(callback, delayMs),
  clearTimeout: (handle) =>
    globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
};

function documentIsHidden(): boolean {
  return (
    typeof document !== "undefined" && document.visibilityState === "hidden"
  );
}

export function createBackoffPoller({
  delaysMs,
  poll,
  isHidden = documentIsHidden,
  clock = defaultClock,
}: BackoffPollerOptions): BackoffPoller {
  // Every start and stop moves the generation on, so a timer or poll from an
  // earlier start never schedules another one.
  let generation = 0;
  let active = false;
  let step = 0;
  let timer: unknown = null;
  let heldWhileHidden = false;

  const clearTimer = () => {
    if (timer !== null) {
      clock.clearTimeout(timer);
      timer = null;
    }
  };

  const schedule = (forGeneration: number) => {
    const delayMs = delaysMs[Math.min(step, delaysMs.length - 1)] ?? 0;
    step += 1;
    timer = clock.setTimeout(() => {
      timer = null;
      void run(forGeneration);
    }, delayMs);
  };

  const run = async (forGeneration: number) => {
    if (forGeneration !== generation) return;
    if (isHidden()) {
      heldWhileHidden = true;
      return;
    }
    let keepWatching = true;
    try {
      keepWatching = await poll();
    } catch {
      // The poll decides when failures are enough to give up; a throw is
      // treated as "still watching".
    }
    if (forGeneration !== generation) return;
    if (keepWatching) {
      schedule(forGeneration);
    } else {
      active = false;
    }
  };

  return {
    restart() {
      clearTimer();
      generation += 1;
      active = true;
      step = 0;
      heldWhileHidden = false;
      schedule(generation);
    },
    stop() {
      clearTimer();
      generation += 1;
      active = false;
      heldWhileHidden = false;
    },
    resume() {
      if (!active || !heldWhileHidden) return;
      heldWhileHidden = false;
      void run(generation);
    },
    isActive: () => active,
  };
}
