const DYNAMIC_IMPORT_FAILURE_MESSAGES = [
  "failed to fetch dynamically imported module",
  "error loading dynamically imported module",
  "importing a module script failed",
] as const;

export const VITE_IMPORT_RECOVERY_STORAGE_KEY =
  "scryer:vite-import-recovery";
export const VITE_IMPORT_RECOVERY_WINDOW_MS = 5_000;

type RecoveryStorage = Pick<Storage, "getItem" | "setItem">;
type StoredRecoveryAttempt = { key: string; attemptedAt: number };

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

export function shouldRetryStaleViteImport(
  error: unknown,
  previousRecoveryAttempts: ReadonlyMap<string, number>,
  now: number,
): boolean {
  const recoveryKey = viteImportRecoveryKey(error);
  if (recoveryKey === null) {
    return false;
  }
  const previousAttemptAt = previousRecoveryAttempts.get(recoveryKey);
  return (
    previousAttemptAt === undefined ||
    previousAttemptAt + VITE_IMPORT_RECOVERY_WINDOW_MS < now
  );
}

function isStoredRecoveryAttempt(value: unknown): value is StoredRecoveryAttempt {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.key === "string" &&
    typeof candidate.attemptedAt === "number" &&
    Number.isFinite(candidate.attemptedAt)
  );
}

function storedRecoveryAttempts(value: string | null, now: number): Map<string, number> {
  const attempts = new Map<string, number>();
  if (!value) {
    return attempts;
  }

  try {
    const parsed: unknown = JSON.parse(value);
    if (Array.isArray(parsed)) {
      for (const entry of parsed) {
        if (typeof entry === "string") {
          attempts.set(entry, now);
        } else if (isStoredRecoveryAttempt(entry)) {
          attempts.set(entry.key, entry.attemptedAt);
        }
      }
      return attempts;
    }
  } catch {
    // Preserve compatibility with the previous single-value storage format.
  }
  attempts.set(value, now);
  return attempts;
}

export function claimViteImportRecovery(
  error: unknown,
  storage: RecoveryStorage,
  now = Date.now(),
): boolean {
  const recoveryKey = viteImportRecoveryKey(error);
  if (!recoveryKey) {
    return false;
  }

  try {
    const previousRecoveryAttempts = storedRecoveryAttempts(storage.getItem(
      VITE_IMPORT_RECOVERY_STORAGE_KEY,
    ), now);
    if (!shouldRetryStaleViteImport(error, previousRecoveryAttempts, now)) {
      return false;
    }
    for (const [key, attemptedAt] of previousRecoveryAttempts) {
      if (attemptedAt + VITE_IMPORT_RECOVERY_WINDOW_MS < now) {
        previousRecoveryAttempts.delete(key);
      }
    }
    previousRecoveryAttempts.set(recoveryKey, now);
    storage.setItem(
      VITE_IMPORT_RECOVERY_STORAGE_KEY,
      JSON.stringify(
        [...previousRecoveryAttempts].map(([key, attemptedAt]) => ({ key, attemptedAt })),
      ),
    );
    return true;
  } catch {
    return false;
  }
}
