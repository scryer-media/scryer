import assert from "node:assert/strict";
import test from "node:test";

import {
  FINALIZE_POLL_INTERVAL_MS,
  canRetryProwlarrDiscovery,
  continueExternalImportFromConnect,
  finalizeProgressPercent,
  isFinalizeBlocked,
  isFinalizeSettled,
  isProwlarrDiscoveryReady,
  pollExternalImportFinalize,
  runFinalizeStart,
  type FinalizeStatusSample,
} from "./external-import-wizard-orchestration.ts";

test("an unresolved preview cannot delay Connect navigation", () => {
  const events: string[] = [];
  const neverResolves = new Promise<void>(() => {});

  continueExternalImportFromConnect(
    () => {
      events.push("preview-started");
      return neverResolves;
    },
    () => events.push("navigated"),
  );

  assert.deepEqual(events, ["preview-started", "navigated"]);
});

test("Prowlarr discovery gates Sources only until completion", () => {
  assert.equal(isProwlarrDiscoveryReady(false, null, null), true);
  assert.equal(isProwlarrDiscoveryReady(true, null, null), false);
  assert.equal(isProwlarrDiscoveryReady(true, "session", "RUNNING"), false);
  assert.equal(isProwlarrDiscoveryReady(true, "session", "FAILED"), false);
  assert.equal(isProwlarrDiscoveryReady(true, "session", "COMPLETED"), true);
});

test("failed or canceled Prowlarr discovery exposes retry", () => {
  assert.equal(canRetryProwlarrDiscovery("RUNNING"), false);
  assert.equal(canRetryProwlarrDiscovery("COMPLETED"), false);
  assert.equal(canRetryProwlarrDiscovery("FAILED"), true);
  assert.equal(canRetryProwlarrDiscovery("CANCELED"), true);
});

// ── Finalize (background apply) ─────────────────────────────────────────────

test("the finalize bar tracks applied entries and pins to 100% when done", () => {
  assert.equal(finalizeProgressPercent(0, 0, false), 0);
  assert.equal(finalizeProgressPercent(0, 400, false), 0);
  assert.equal(finalizeProgressPercent(100, 400, false), 25);
  assert.equal(finalizeProgressPercent(500, 400, false), 100);
  assert.equal(finalizeProgressPercent(0, 0, true), 100);
});

test("finalize polling advances then resolves once the apply completes", async () => {
  const samples: FinalizeStatusSample[] = [];
  const waits: number[] = [];
  const statuses: FinalizeStatusSample[] = [
    { status: "RUNNING", completed: 0, total: 400, errorMessage: null },
    { status: "RUNNING", completed: 250, total: 400, errorMessage: null },
    { status: "COMPLETED", completed: 400, total: 400, errorMessage: null },
  ];

  const outcome = await pollExternalImportFinalize("apply-session", {
    fetchStatus: async () => ({
      sample: statuses.shift() ?? null,
      error: null,
    }),
    onSample: (sample) => samples.push(sample),
    wait: async (ms) => {
      waits.push(ms);
    },
  });

  assert.deepEqual(outcome, { ok: true, error: null });
  assert.deepEqual(
    samples.map((sample) => sample.completed),
    [0, 250, 400],
  );
  assert.equal(waits.length, 2);
});

test("a failed apply reports its durable error instead of polling forever", async () => {
  const outcome = await pollExternalImportFinalize("apply-session", {
    fetchStatus: async () => ({
      sample: {
        status: "FAILED",
        completed: 12,
        total: 400,
        errorMessage: "missing mapping for source arr-one root '/media/shows'",
      },
      error: null,
    }),
    onSample: () => {},
    wait: async () => {},
  });

  assert.equal(outcome.ok, false);
  assert.equal(
    outcome.error,
    "missing mapping for source arr-one root '/media/shows'",
  );
});

test("a transient status read backs off instead of aborting the apply", async () => {
  const waits: number[] = [];
  let call = 0;
  const outcome = await pollExternalImportFinalize("apply-session", {
    fetchStatus: async () => {
      call += 1;
      if (call <= 2) return { sample: null, error: "[Network] offline" };
      return {
        sample: {
          status: "COMPLETED",
          completed: 5,
          total: 5,
          errorMessage: null,
        },
        error: null,
      };
    },
    onSample: () => {},
    wait: async (ms) => {
      waits.push(ms);
    },
  });

  assert.deepEqual(outcome, { ok: true, error: null });
  assert.deepEqual(waits, [
    FINALIZE_POLL_INTERVAL_MS,
    FINALIZE_POLL_INTERVAL_MS * 2,
  ]);
});

test("a lost apply session is terminal, not retried", async () => {
  const outcome = await pollExternalImportFinalize("apply-session", {
    fetchStatus: async () => ({
      sample: null,
      error: "The server used different words",
      sessionGone: true,
    }),
    onSample: () => {},
    wait: async () => {
      throw new Error("a lost session must not be polled again");
    },
  });

  assert.equal(outcome.ok, false);
  assert.equal(outcome.error, "The server used different words");
});

test("isFinalizeSettled only accepts terminal statuses", () => {
  assert.equal(isFinalizeSettled(null), false);
  assert.equal(isFinalizeSettled("QUEUED"), false);
  assert.equal(isFinalizeSettled("RUNNING"), false);
  assert.equal(isFinalizeSettled("COMPLETED"), true);
  assert.equal(isFinalizeSettled("FAILED"), true);
  assert.equal(isFinalizeSettled("CANCELED"), true);
});

test("a throw while starting finalize reports a message and clears the gate", async () => {
  const failures: string[] = [];
  const outcome = await runFinalizeStart(
    async () => {
      throw new Error("createLibrary exploded");
    },
    (message) => failures.push(message),
  );

  assert.deepEqual(outcome, { ok: false, error: "createLibrary exploded" });
  assert.deepEqual(failures, ["createLibrary exploded"]);
});

test("a rejected finalize start reports its own message once", async () => {
  const failures: string[] = [];
  const outcome = await runFinalizeStart(
    async () => ({ ok: false, error: "Failed to create library: Movies" }),
    (message) => failures.push(message),
  );

  assert.equal(outcome.ok, false);
  assert.deepEqual(failures, ["Failed to create library: Movies"]);
});

test("an accepted finalize start reports no failure", async () => {
  const failures: string[] = [];
  const outcome = await runFinalizeStart(
    async () => ({ ok: true, error: null }),
    (message) => failures.push(message),
  );

  assert.deepEqual(outcome, { ok: true, error: null });
  assert.deepEqual(failures, []);
});

test("Finish is disabled for the whole apply and re-enables on failure", () => {
  const ready = {
    warmupComplete: true,
    previewSettled: true,
    mappingReady: true,
  };
  assert.equal(isFinalizeBlocked({ ...ready, finalizing: false }), false);
  // While the background apply runs.
  assert.equal(isFinalizeBlocked({ ...ready, finalizing: true }), true);
  // The apply failed → `finalizing` is cleared and a retry is offered.
  assert.equal(isFinalizeBlocked({ ...ready, finalizing: false }), false);
  assert.equal(
    isFinalizeBlocked({ ...ready, warmupComplete: false, finalizing: false }),
    true,
  );
});
