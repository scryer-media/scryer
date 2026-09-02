const DYNAMIC_IMPORT_FAILURE_MESSAGES = [
  "failed to fetch dynamically imported module",
  "error loading dynamically imported module",
  "importing a module script failed",
] as const;

export const VITE_IMPORT_RECOVERY_STORAGE_KEY =
  "scryer:vite-import-recovery";

type RecoveryStorage = Pick<Storage, "getItem" | "setItem">;

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
  previousRecoveryKeys: ReadonlySet<string>,
): boolean {
  const recoveryKey = viteImportRecoveryKey(error);
  return recoveryKey !== null && !previousRecoveryKeys.has(recoveryKey);
}

function storedRecoveryKeys(value: string | null): Set<string> {
  if (!value) {
    return new Set();
  }

  try {
    const parsed: unknown = JSON.parse(value);
    if (Array.isArray(parsed)) {
      return new Set(parsed.filter((entry): entry is string => typeof entry === "string"));
    }
  } catch {
    // Preserve compatibility with the previous single-value storage format.
  }
  return new Set([value]);
}

export function claimViteImportRecovery(
  error: unknown,
  storage: RecoveryStorage,
): boolean {
  const recoveryKey = viteImportRecoveryKey(error);
  if (!recoveryKey) {
    return false;
  }

  try {
    const previousRecoveryKeys = storedRecoveryKeys(storage.getItem(
      VITE_IMPORT_RECOVERY_STORAGE_KEY,
    ));
    if (!shouldRetryStaleViteImport(error, previousRecoveryKeys)) {
      return false;
    }
    previousRecoveryKeys.add(recoveryKey);
    storage.setItem(
      VITE_IMPORT_RECOVERY_STORAGE_KEY,
      JSON.stringify([...previousRecoveryKeys]),
    );
    return true;
  } catch {
    return false;
  }
}
