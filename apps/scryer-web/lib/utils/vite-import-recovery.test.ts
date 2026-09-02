import assert from "node:assert/strict";
import test from "node:test";
import {
  claimViteImportRecovery,
  shouldRetryStaleViteImport,
  VITE_IMPORT_RECOVERY_STORAGE_KEY,
  viteImportRecoveryKey,
} from "./vite-import-recovery.ts";

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
  assert.equal(shouldRetryStaleViteImport(dynamicImportFailure, new Set()), true);
});

test("does not loop when the recovery reload also fails", () => {
  const recoveryKey = viteImportRecoveryKey(dynamicImportFailure);
  assert.notEqual(recoveryKey, null);
  assert.equal(
    shouldRetryStaleViteImport(dynamicImportFailure, new Set([recoveryKey!])),
    false,
  );
});

test("allows one recovery for each distinct failed chunk", () => {
  const storage = memoryStorage();
  assert.equal(claimViteImportRecovery(dynamicImportFailure, storage), true);
  assert.equal(claimViteImportRecovery(dynamicImportFailure, storage), false);
  assert.equal(claimViteImportRecovery(secondDynamicImportFailure, storage), true);
  assert.equal(claimViteImportRecovery(dynamicImportFailure, storage), false);
  const storedKeys = JSON.parse(
    storage.getItem(VITE_IMPORT_RECOVERY_STORAGE_KEY) ?? "[]",
  );
  assert.deepEqual(storedKeys, [
    viteImportRecoveryKey(dynamicImportFailure),
    viteImportRecoveryKey(secondDynamicImportFailure),
  ]);
});

test("does not retry unrelated route errors", () => {
  assert.equal(
    shouldRetryStaleViteImport(new Error("route loader failed"), new Set()),
    false,
  );
});

test("recognizes Safari-style dynamic import failures", () => {
  assert.equal(
    shouldRetryStaleViteImport(
      new TypeError("Importing a module script failed."),
      new Set(),
    ),
    true,
  );
});
