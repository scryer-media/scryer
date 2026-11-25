const DYNAMIC_IMPORT_FAILURE_MESSAGES = [
  "failed to fetch dynamically imported module",
  "error loading dynamically imported module",
  "importing a module script failed",
] as const;

export const VITE_IMPORT_RECOVERY_STORAGE_KEY = "scryer:vite-import-recovery";
/** Attempts older than this no longer count against a chunk's budget. */
export const VITE_IMPORT_RECOVERY_WINDOW_MS = 60_000;
/** Back-off before each automatic reload, in order. */
export const VITE_IMPORT_RECOVERY_DELAYS_MS = [500, 2_000, 5_000] as const;
/**
 * Total reloads for one chunk inside the window. Attempts past the back-off
 * schedule happen only once the server answers again.
 */
export const VITE_IMPORT_RECOVERY_MAX_ATTEMPTS = 6;
export const VITE_IMPORT_RECOVERY_PROBE_INTERVAL_MS = 15_000;

type RecoveryStorage = Pick<Storage, "getItem" | "setItem">;
type StoredRecoveryAttempt = {
  key: string;
  attemptedAt: number;
  attempts?: number;
};
type RecoveryAttempt = { attemptedAt: number; attempts: number };

/**
 * What to do about a failed chunk load: reload after `delayMs`, wait for the
 * server to answer before reloading, or stop and leave the error on screen.
 */
export type ViteImportRecoveryPlan =
  | { kind: "reload"; delayMs: number }
  | { kind: "await-reachable" }
  | { kind: "give-up" };

export function viteImportRecoveryKey(error: unknown): string | null {
  if (!(error instanceof Error)) {
    return null;
  }

  const message = error.message.trim();
  const normalized = message.toLowerCase();
  return DYNAMIC_IMPORT_FAILURE_MESSAGES.some((candidate) =>
    normalized.includes(candidate),
  )
    ? message
    : null;
}

function isStoredRecoveryAttempt(
  value: unknown,
): value is StoredRecoveryAttempt {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.key === "string" &&
    typeof candidate.attemptedAt === "number" &&
    Number.isFinite(candidate.attemptedAt) &&
    (candidate.attempts === undefined ||
      (typeof candidate.attempts === "number" &&
        Number.isInteger(candidate.attempts) &&
        candidate.attempts > 0))
  );
}

function storedRecoveryAttempts(
  value: string | null,
  now: number,
): Map<string, RecoveryAttempt> {
  const attempts = new Map<string, RecoveryAttempt>();
  if (!value) {
    return attempts;
  }

  try {
    const parsed: unknown = JSON.parse(value);
    if (Array.isArray(parsed)) {
      for (const entry of parsed) {
        if (typeof entry === "string") {
          attempts.set(entry, { attemptedAt: now, attempts: 1 });
        } else if (isStoredRecoveryAttempt(entry)) {
          attempts.set(entry.key, {
            attemptedAt: entry.attemptedAt,
            attempts: entry.attempts ?? 1,
          });
        }
      }
      return attempts;
    }
  } catch {
    // Preserve compatibility with the previous single-value storage format.
  }
  attempts.set(value, { attemptedAt: now, attempts: 1 });
  return attempts;
}

function isExpired(attempt: RecoveryAttempt, now: number): boolean {
  return attempt.attemptedAt + VITE_IMPORT_RECOVERY_WINDOW_MS < now;
}

/** Attempts already spent on this chunk inside the window. */
function spentAttempts(
  previous: RecoveryAttempt | undefined,
  now: number,
): number {
  return previous === undefined || isExpired(previous, now)
    ? 0
    : previous.attempts;
}

export function planViteImportRecovery(
  error: unknown,
  storage: RecoveryStorage,
  now = Date.now(),
): ViteImportRecoveryPlan | null {
  const recoveryKey = viteImportRecoveryKey(error);
  if (!recoveryKey) {
    return null;
  }

  try {
    const previous = storedRecoveryAttempts(
      storage.getItem(VITE_IMPORT_RECOVERY_STORAGE_KEY),
      now,
    ).get(recoveryKey);
    const spent = spentAttempts(previous, now);
    if (spent < VITE_IMPORT_RECOVERY_DELAYS_MS.length) {
      return { kind: "reload", delayMs: VITE_IMPORT_RECOVERY_DELAYS_MS[spent] };
    }
    return spent < VITE_IMPORT_RECOVERY_MAX_ATTEMPTS
      ? { kind: "await-reachable" }
      : { kind: "give-up" };
  } catch {
    return { kind: "give-up" };
  }
}

/**
 * Records one reload attempt for the failed chunk. Returns false when storage
 * is unavailable, so callers never reload without a record that bounds the
 * loop.
 */
export function recordViteImportRecoveryAttempt(
  error: unknown,
  storage: RecoveryStorage,
  now = Date.now(),
): boolean {
  const recoveryKey = viteImportRecoveryKey(error);
  if (!recoveryKey) {
    return false;
  }

  try {
    const attempts = storedRecoveryAttempts(
      storage.getItem(VITE_IMPORT_RECOVERY_STORAGE_KEY),
      now,
    );
    const spent = spentAttempts(attempts.get(recoveryKey), now);
    for (const [key, attempt] of attempts) {
      if (isExpired(attempt, now)) {
        attempts.delete(key);
      }
    }
    attempts.delete(recoveryKey);
    attempts.set(recoveryKey, { attemptedAt: now, attempts: spent + 1 });
    storage.setItem(
      VITE_IMPORT_RECOVERY_STORAGE_KEY,
      JSON.stringify(
        [...attempts].map(([key, attempt]) => ({
          key,
          attemptedAt: attempt.attemptedAt,
          attempts: attempt.attempts,
        })),
      ),
    );
    return true;
  } catch {
    return false;
  }
}

export type RecoveryEnvironment = {
  storage: RecoveryStorage;
  /** A same-origin URL that answers cheaply when the server is reachable. */
  probeUrl: string;
  reload: () => void;
  now?: () => number;
  isOnline: () => boolean;
  isVisible: () => boolean;
  fetch: (url: string) => Promise<{ ok: boolean }>;
  setTimeout: (callback: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
  /** Subscribes to `online` and to the page becoming visible again. */
  onReachabilityHint: (callback: () => void) => () => void;
};

let pendingRecovery: { error: unknown; cancel: () => void } | null = null;

/**
 * Reloads the page to recover from a failed chunk load, without hammering a
 * server that is down: it backs off between reloads, waits while the browser
 * is offline or the tab is hidden, and once the quick reloads are spent it
 * reloads only after the server answers again. Only one recovery runs per
 * page; later calls while one is pending do nothing. Returns a cancel
 * function, or null when the error is not a chunk-load failure or the budget
 * is spent.
 */
export function recoverFromViteImportError(
  error: unknown,
  env: RecoveryEnvironment,
): (() => void) | null {
  if (pendingRecovery !== null) {
    return pendingRecovery.cancel;
  }
  const plan = planViteImportRecovery(
    error,
    env.storage,
    env.now?.() ?? Date.now(),
  );
  if (plan === null || plan.kind === "give-up") {
    return null;
  }

  let timer: unknown = null;
  let unsubscribe: (() => void) | null = null;
  let probing = false;
  let finished = false;

  const finish = () => {
    finished = true;
    if (timer !== null) {
      env.clearTimeout(timer);
      timer = null;
    }
    unsubscribe?.();
    unsubscribe = null;
    if (pendingRecovery?.cancel === cancel) {
      pendingRecovery = null;
    }
  };
  const cancel = () => finish();

  const reloadNow = () => {
    if (finished) {
      return;
    }
    const recorded = recordViteImportRecoveryAttempt(
      error,
      env.storage,
      env.now?.() ?? Date.now(),
    );
    finish();
    if (recorded) {
      env.reload();
    }
  };

  const schedule = (ms: number, callback: () => void) => {
    if (timer !== null) {
      env.clearTimeout(timer);
    }
    timer = env.setTimeout(() => {
      timer = null;
      callback();
    }, ms);
  };

  // Reloads once the browser reports a network and the tab is in view. With
  // `probeFirst`, it also waits until the server itself answers.
  const reloadWhenReachable = (probeFirst: boolean) => {
    if (finished) {
      return;
    }
    if (!env.isOnline() || !env.isVisible()) {
      return;
    }
    if (!probeFirst) {
      reloadNow();
      return;
    }
    if (probing) {
      return;
    }
    probing = true;
    env
      .fetch(env.probeUrl)
      .then(
        (response) => response.ok,
        () => false,
      )
      .then((reachable) => {
        probing = false;
        if (finished) {
          return;
        }
        if (reachable) {
          reloadNow();
        } else {
          schedule(VITE_IMPORT_RECOVERY_PROBE_INTERVAL_MS, () =>
            reloadWhenReachable(true),
          );
        }
      });
  };

  const probeFirst = plan.kind === "await-reachable";
  unsubscribe = env.onReachabilityHint(() => reloadWhenReachable(probeFirst));
  if (plan.kind === "reload") {
    schedule(plan.delayMs, () => reloadWhenReachable(false));
  } else {
    reloadWhenReachable(true);
    if (timer === null && !probing) {
      schedule(VITE_IMPORT_RECOVERY_PROBE_INTERVAL_MS, () =>
        reloadWhenReachable(true),
      );
    }
  }

  pendingRecovery = { error, cancel };
  return cancel;
}

/** The browser's recovery environment for the current page. */
export function browserRecoveryEnvironment(
  probeUrl: string,
): RecoveryEnvironment {
  return {
    storage: window.sessionStorage,
    probeUrl,
    reload: () => window.location.reload(),
    isOnline: () => navigator.onLine !== false,
    isVisible: () => document.visibilityState !== "hidden",
    fetch: (url) => window.fetch(url, { cache: "no-store" }),
    setTimeout: (callback, ms) => window.setTimeout(callback, ms),
    clearTimeout: (handle) => window.clearTimeout(handle as number),
    onReachabilityHint: (callback) => {
      const onVisibility = () => {
        if (document.visibilityState === "visible") {
          callback();
        }
      };
      window.addEventListener("online", callback);
      document.addEventListener("visibilitychange", onVisibility);
      return () => {
        window.removeEventListener("online", callback);
        document.removeEventListener("visibilitychange", onVisibility);
      };
    },
  };
}
