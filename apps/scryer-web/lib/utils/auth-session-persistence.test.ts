import assert from "node:assert/strict";
import test from "node:test";

import {
  PERSISTENT_STORAGE_KEY,
  SESSION_STORAGE_KEY,
  authSessionPersistence,
  storeAuthToken,
} from "./auth-session-persistence.ts";

function storage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  };
}

test("persistent external login sessions survive browser restarts", () => {
  assert.equal(authSessionPersistence(true), "persistent");
});

test("non-persistent external login sessions stay in the current tab", () => {
  assert.equal(authSessionPersistence(false), "tab");
});

test("local MFA enrollment and completion retain a persistent storage choice", () => {
  const session = storage();
  const persistent = storage();
  Object.defineProperties(globalThis, {
    sessionStorage: { configurable: true, value: session },
    localStorage: { configurable: true, value: persistent },
  });

  storeAuthToken("mfa-enrollment-token", true);
  assert.equal(
    persistent.getItem(PERSISTENT_STORAGE_KEY),
    "mfa-enrollment-token",
  );
  assert.equal(session.getItem(SESSION_STORAGE_KEY), null);

  storeAuthToken("full-session-token", true);
  assert.equal(
    persistent.getItem(PERSISTENT_STORAGE_KEY),
    "full-session-token",
  );
  assert.equal(session.getItem(SESSION_STORAGE_KEY), null);
});
