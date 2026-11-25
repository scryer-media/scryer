// How many slow requests may hold a connection at once. A browser allows six
// HTTP/1.1 connections per origin; this leaves four for everything else.
export const SLOW_LANE_WIDTH = 2;

let inFlight = 0;
const waiting: Array<() => void> = [];

function release() {
  inFlight--;
  waiting.shift()?.();
}

function abortError(): DOMException {
  return new DOMException("Request canceled", "AbortError");
}

// Resolves once the caller may send. A caller that gives up while waiting
// leaves the line without ever taking a place.
function enter(signal: AbortSignal | null | undefined): Promise<void> {
  if (signal?.aborted) {
    return Promise.reject(abortError());
  }
  if (inFlight < SLOW_LANE_WIDTH) {
    inFlight++;
    return Promise.resolve();
  }
  return new Promise((resolve, reject) => {
    const admit = () => {
      signal?.removeEventListener("abort", leave);
      inFlight++;
      resolve();
    };
    const leave = () => {
      const index = waiting.indexOf(admit);
      if (index !== -1) waiting.splice(index, 1);
      reject(abortError());
    };
    signal?.addEventListener("abort", leave, { once: true });
    waiting.push(admit);
  });
}

/**
 * Wrap a fetch so that at most `SLOW_LANE_WIDTH` requests sent through any
 * slow-lane fetch are in flight together. The rest wait their turn in the page,
 * holding no connection, and are sent in the order they were made.
 */
export function slowLane(send: typeof fetch): typeof fetch {
  return async (input, init) => {
    await enter(init?.signal);
    try {
      return await send(input, init);
    } finally {
      release();
    }
  };
}
