import assert from "node:assert/strict";
import test from "node:test";
import {
  claimViteImportRecovery,
  shouldRetryStaleViteImport,
  VITE_IMPORT_RECOVERY_STORAGE_KEY,
  VITE_IMPORT_RECOVERY_WINDOW_MS,
  viteImportRecoveryKey,
} from "./vite-import-recovery.ts";

const NOW = 1_000_000;

const dynamicImportFailure = new TypeError(
  "Failed to fetch dynamically imported module: http://localhost:3000/src/pages/login.tsx",
);
const secondDynamicImportFailure = new TypeError(
  "Failed to fetch dynamically imported module: http://localhost:3000/src/pages/setup.tsx",
);

function memoryStorage() {
  const values = new Map<string, string>();
  return {
    getItem(key: string) {
      return values.get(key) ?? null;
    },
    setItem(key: string, value: string) {
      values.set(key, value);
    },
  };
}

test("retries a failed dynamic import when no recent recovery was attempted", () => {
  assert.equal(shouldRetryStaleViteImport(dynamicImportFailure, new Map(), NOW), true);
});

test("does not loop when the recovery reload also fails", () => {
  const recoveryKey = viteImportRecoveryKey(dynamicImportFailure);
  assert.notEqual(recoveryKey, null);
  assert.equal(
    shouldRetryStaleViteImport(
      dynamicImportFailure,
      new Map([[recoveryKey!, NOW]]),
      NOW,
    ),
    false,
  );
});

test("allows one recovery for each distinct failed chunk", () => {
  const storage = memoryStorage();
  assert.equal(claimViteImportRecovery(dynamicImportFailure, storage, NOW), true);
  assert.equal(claimViteImportRecovery(dynamicImportFailure, storage, NOW), false);
  assert.equal(claimViteImportRecovery(secondDynamicImportFailure, storage, NOW), true);
  assert.equal(claimViteImportRecovery(dynamicImportFailure, storage, NOW), false);
  const storedAttempts = JSON.parse(
    storage.getItem(VITE_IMPORT_RECOVERY_STORAGE_KEY) ?? "[]",
  );
  assert.deepEqual(storedAttempts, [
    { key: viteImportRecoveryKey(dynamicImportFailure), attemptedAt: NOW },
    { key: viteImportRecoveryKey(secondDynamicImportFailure), attemptedAt: NOW },
  ]);
});

test("allows the same generic recovery claim after the retry window", () => {
  const storage = memoryStorage();
  const safariFailure = new TypeError("Importing a module script failed.");

  assert.equal(claimViteImportRecovery(safariFailure, storage, NOW), true);
  assert.equal(
    claimViteImportRecovery(safariFailure, storage, NOW + VITE_IMPORT_RECOVERY_WINDOW_MS),
    false,
  );
  assert.equal(
    claimViteImportRecovery(safariFailure, storage, NOW + VITE_IMPORT_RECOVERY_WINDOW_MS + 1),
    true,
  );
});

test("does not retry unrelated route errors", () => {
  assert.equal(
    shouldRetryStaleViteImport(new Error("route loader failed"), new Map(), NOW),
    false,
  );
});

test("recognizes Safari-style dynamic import failures", () => {
  assert.equal(
    shouldRetryStaleViteImport(
      new TypeError("Importing a module script failed."),
      new Map(),
      NOW,
    ),
    true,
  );
});
