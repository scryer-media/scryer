import assert from "node:assert/strict";
import test from "node:test";
import {
  planViteImportRecovery,
  recordViteImportRecoveryAttempt,
  recoverFromViteImportError,
  type RecoveryEnvironment,
  VITE_IMPORT_RECOVERY_DELAYS_MS,
  VITE_IMPORT_RECOVERY_MAX_ATTEMPTS,
  VITE_IMPORT_RECOVERY_PROBE_INTERVAL_MS,
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

function spend(
  error: unknown,
  storage: ReturnType<typeof memoryStorage>,
  n: number,
) {
  for (let i = 0; i < n; i += 1) {
    assert.equal(recordViteImportRecoveryAttempt(error, storage, NOW), true);
  }
}

test("backs off between the quick reloads for one chunk", () => {
  const storage = memoryStorage();
  for (const delayMs of VITE_IMPORT_RECOVERY_DELAYS_MS) {
    assert.deepEqual(
      planViteImportRecovery(dynamicImportFailure, storage, NOW),
      {
        kind: "reload",
        delayMs,
      },
    );
    spend(dynamicImportFailure, storage, 1);
  }
  assert.deepEqual(planViteImportRecovery(dynamicImportFailure, storage, NOW), {
    kind: "await-reachable",
  });
});

test("stops reloading once the attempt budget is spent", () => {
  const storage = memoryStorage();
  spend(dynamicImportFailure, storage, VITE_IMPORT_RECOVERY_MAX_ATTEMPTS);
  assert.deepEqual(planViteImportRecovery(dynamicImportFailure, storage, NOW), {
    kind: "give-up",
  });
});

test("keeps a separate budget for each failed chunk", () => {
  const storage = memoryStorage();
  spend(dynamicImportFailure, storage, VITE_IMPORT_RECOVERY_MAX_ATTEMPTS);
  assert.deepEqual(
    planViteImportRecovery(secondDynamicImportFailure, storage, NOW),
    { kind: "reload", delayMs: VITE_IMPORT_RECOVERY_DELAYS_MS[0] },
  );
  const stored = JSON.parse(
    storage.getItem(VITE_IMPORT_RECOVERY_STORAGE_KEY) ?? "[]",
  );
  assert.deepEqual(stored, [
    {
      key: viteImportRecoveryKey(dynamicImportFailure),
      attemptedAt: NOW,
      attempts: VITE_IMPORT_RECOVERY_MAX_ATTEMPTS,
    },
  ]);
});

test("restores the budget after the window passes", () => {
  const storage = memoryStorage();
  const safariFailure = new TypeError("Importing a module script failed.");
  spend(safariFailure, storage, VITE_IMPORT_RECOVERY_MAX_ATTEMPTS);
  assert.deepEqual(
    planViteImportRecovery(
      safariFailure,
      storage,
      NOW + VITE_IMPORT_RECOVERY_WINDOW_MS,
    ),
    { kind: "give-up" },
  );
  assert.deepEqual(
    planViteImportRecovery(
      safariFailure,
      storage,
      NOW + VITE_IMPORT_RECOVERY_WINDOW_MS + 1,
    ),
    { kind: "reload", delayMs: VITE_IMPORT_RECOVERY_DELAYS_MS[0] },
  );
});

test("reads attempts stored by the previous release as one attempt", () => {
  const storage = memoryStorage();
  storage.setItem(
    VITE_IMPORT_RECOVERY_STORAGE_KEY,
    JSON.stringify([
      { key: viteImportRecoveryKey(dynamicImportFailure), attemptedAt: NOW },
    ]),
  );
  assert.deepEqual(planViteImportRecovery(dynamicImportFailure, storage, NOW), {
    kind: "reload",
    delayMs: VITE_IMPORT_RECOVERY_DELAYS_MS[1],
  });
});

test("ignores unrelated route errors", () => {
  assert.equal(
    planViteImportRecovery(
      new Error("route loader failed"),
      memoryStorage(),
      NOW,
    ),
    null,
  );
});

test("recognizes Safari-style dynamic import failures", () => {
  assert.notEqual(
    viteImportRecoveryKey(new TypeError("Importing a module script failed.")),
    null,
  );
});

type FakeEnv = RecoveryEnvironment & {
  reloads: number;
  probes: number;
  online: boolean;
  visible: boolean;
  serverUp: boolean;
  advance: (ms: number) => Promise<void>;
  hint: () => Promise<void>;
};

function fakeEnv(storage = memoryStorage()): FakeEnv {
  let clock = NOW;
  let timers: { at: number; callback: () => void; id: number }[] = [];
  let nextId = 1;
  const hints = new Set<() => void>();
  const flush = () => new Promise<void>((resolve) => setImmediate(resolve));
  const env: FakeEnv = {
    storage,
    probeUrl: "/health",
    reloads: 0,
    probes: 0,
    online: true,
    visible: true,
    serverUp: true,
    reload: () => {
      env.reloads += 1;
    },
    now: () => clock,
    isOnline: () => env.online,
    isVisible: () => env.visible,
    fetch: async () => {
      env.probes += 1;
      if (!env.serverUp) {
        throw new TypeError("Failed to fetch");
      }
      return { ok: true };
    },
    setTimeout: (callback, ms) => {
      const id = nextId++;
      timers.push({ at: clock + ms, callback, id });
      return id;
    },
    clearTimeout: (handle) => {
      timers = timers.filter((timer) => timer.id !== handle);
    },
    onReachabilityHint: (callback) => {
      hints.add(callback);
      return () => hints.delete(callback);
    },
    advance: async (ms) => {
      const until = clock + ms;
      for (;;) {
        await flush();
        const due = timers
          .filter((timer) => timer.at <= until)
          .sort((a, b) => a.at - b.at)[0];
        if (!due) break;
        timers = timers.filter((timer) => timer !== due);
        clock = due.at;
        due.callback();
      }
      clock = until;
      await flush();
    },
    hint: async () => {
      for (const callback of [...hints]) callback();
      await flush();
    },
  };
  return env;
}

test("reloads after the back-off delay, not at once", async () => {
  const env = fakeEnv();
  recoverFromViteImportError(dynamicImportFailure, env);
  await env.advance(VITE_IMPORT_RECOVERY_DELAYS_MS[0] - 1);
  assert.equal(env.reloads, 0);
  await env.advance(1);
  assert.equal(env.reloads, 1);
});

test("waits for the network to come back before reloading", async () => {
  const env = fakeEnv();
  env.online = false;
  recoverFromViteImportError(dynamicImportFailure, env);
  await env.advance(60_000);
  assert.equal(env.reloads, 0);
  env.online = true;
  await env.hint();
  assert.equal(env.reloads, 1);
});

test("waits for a hidden tab to come back before reloading", async () => {
  const env = fakeEnv();
  env.visible = false;
  recoverFromViteImportError(dynamicImportFailure, env);
  await env.advance(60_000);
  assert.equal(env.reloads, 0);
  env.visible = true;
  await env.hint();
  assert.equal(env.reloads, 1);
});

test("after the quick reloads, reloads only once the server answers", async () => {
  const storage = memoryStorage();
  spend(dynamicImportFailure, storage, VITE_IMPORT_RECOVERY_DELAYS_MS.length);
  const env = fakeEnv(storage);
  env.serverUp = false;
  recoverFromViteImportError(dynamicImportFailure, env);
  await env.advance(VITE_IMPORT_RECOVERY_PROBE_INTERVAL_MS * 3);
  assert.equal(env.reloads, 0);
  assert.ok(env.probes >= 3);
  env.serverUp = true;
  await env.advance(VITE_IMPORT_RECOVERY_PROBE_INTERVAL_MS);
  assert.equal(env.reloads, 1);
});

test("runs one recovery per page and does nothing once the budget is spent", async () => {
  const storage = memoryStorage();
  const env = fakeEnv(storage);
  const cancel = recoverFromViteImportError(dynamicImportFailure, env);
  assert.equal(recoverFromViteImportError(dynamicImportFailure, env), cancel);
  await env.advance(VITE_IMPORT_RECOVERY_DELAYS_MS[0]);
  assert.equal(env.reloads, 1);

  spend(dynamicImportFailure, storage, VITE_IMPORT_RECOVERY_MAX_ATTEMPTS);
  assert.equal(
    recoverFromViteImportError(dynamicImportFailure, fakeEnv(storage)),
    null,
  );
});

test("a cancelled recovery never reloads", async () => {
  const env = fakeEnv();
  const cancel = recoverFromViteImportError(dynamicImportFailure, env);
  cancel?.();
  await env.advance(60_000);
  await env.hint();
  assert.equal(env.reloads, 0);
});
