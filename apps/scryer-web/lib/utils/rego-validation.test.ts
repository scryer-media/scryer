import assert from "node:assert/strict";
import test from "node:test";
import {
  RegoValidationSession,
  type ValidationClock,
  type ValidationSnapshot,
} from "./rego-validation.ts";
import type { RegoValidationResult } from "./rego-diagnostics.ts";

const valid = { valid: true, errors: [] };
function harness() {
  let pending: (() => void) | undefined;
  const timers: ValidationClock = {
    set(run, delay) {
      assert.equal(delay, 500);
      pending = run;
      return run;
    },
    clear(id) {
      if (id === pending) pending = undefined;
    },
  };
  const requests: {
    source: string;
    resolve: (result: RegoValidationResult) => void;
    reject: (error: Error) => void;
  }[] = [];
  const snapshots: ValidationSnapshot[] = [];
  const session = new RegoValidationSession(
    (source) =>
      new Promise((resolve, reject) =>
        requests.push({ source, resolve, reject }),
      ),
    (s) => snapshots.push(s),
    "Validation unavailable",
    timers,
  );
  return {
    session,
    requests,
    snapshots,
    tick() {
      assert.ok(pending);
      const run = pending;
      pending = undefined;
      run();
    },
    pending: () => Boolean(pending),
  };
}
// Flush promise continuations only. No real timer or scheduling-order assumption.
async function settle() {
  await new Promise<void>((resolve) => queueMicrotask(resolve));
}

test("debounces edits, clears obsolete diagnostics, and shares explicit validation", async () => {
  const h = harness();
  h.session.setDocument("one", "one", true);
  h.session.setDocument("two", "two", true);
  assert.equal(h.requests.length, 0);
  h.tick();
  await settle();
  assert.deepEqual(
    h.requests.map((r) => r.source),
    ["two"],
  );
  const explicit = h.session.validate();
  h.requests[0].resolve(valid);
  assert.deepEqual(await explicit, valid);
  assert.deepEqual(await h.session.validate(), valid);
  assert.equal(h.requests.length, 1);
  h.session.setDocument("three", "three", true);
  assert.equal(h.snapshots.at(-1)?.result, null);
  h.session.dispose();
});

test("coalesces pending requests and discards old results", async () => {
  const h = harness();
  h.session.setDocument("old", "a", true);
  const old = h.session.validate();
  await settle();
  h.session.setDocument("intermediate", "b", true);
  const intermediate = h.session.validate();
  h.session.setDocument("latest", "c", true);
  const latest = h.session.validate();
  const duplicate = h.session.validate();
  h.requests[0].resolve({ valid: false, errors: ["old error"] });
  assert.equal(await old, null);
  assert.equal(await intermediate, null);
  await settle();
  assert.deepEqual(
    h.requests.map((r) => r.source),
    ["old", "latest"],
  );
  assert.equal(h.snapshots.at(-1)?.key, "c");
  assert.equal(h.snapshots.at(-1)?.result, null);
  h.requests[1].resolve(valid);
  assert.deepEqual(await latest, valid);
  assert.deepEqual(await duplicate, valid);
  assert.equal(h.requests.length, 2);
});

test("rule switch and closing prevent publication even with the same source", async () => {
  const h = harness();
  h.session.setDocument("same", "rule-a", true);
  const a = h.session.validate();
  await settle();
  h.session.setDocument("same", "rule-b", true);
  h.requests[0].resolve(valid);
  assert.equal(await a, null);
  assert.equal(h.snapshots.at(-1)?.result, null);
  const b = h.session.validate();
  await settle();
  h.session.dispose();
  const count = h.snapshots.length;
  h.requests[1].resolve(valid);
  assert.equal(await b, null);
  assert.equal(h.snapshots.length, count);
  assert.equal(h.pending(), false);
});

test("empty and read-only documents do not validate; transport failure is retryable", async () => {
  const h = harness();
  h.session.setDocument(" ", "empty", true);
  assert.equal(await h.session.validate(), null);
  h.session.setDocument("source", "readonly", false);
  assert.equal(await h.session.validate(), null);
  assert.equal(h.pending(), false);
  h.session.setDocument("source", "edit", true);
  const first = h.session.validate();
  await settle();
  h.requests[0].reject(new Error("offline"));
  assert.deepEqual(await first, {
    valid: false,
    errors: ["Validation unavailable"],
    unavailable: true,
  });
  const second = h.session.validate();
  await settle();
  assert.equal(h.requests.length, 2);
  h.requests[1].resolve(valid);
  assert.deepEqual(await second, valid);
});
