import assert from "node:assert/strict";
import { test } from "node:test";
import { boundedQuery, QueryTimeoutError, ACTIVITY_READ_TIMEOUT_MS } from "./bounded-query.ts";

function fixture() {
  let next: (value: { stale?: boolean; data?: string }) => void = () => {};
  let unsubscribed = 0;
  return {
    source: { subscribe(callback: typeof next) { next = callback; return { unsubscribe() { unsubscribed++; } }; } },
    emit(data: string, stale = false) { next({ data, stale }); },
    get unsubscribed() { return unsubscribed; },
  };
}

test("a request deadline unsubscribes and ignores late success", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = fixture();
  const request = boundedQuery(f.source, new AbortController().signal);
  const rejected = assert.rejects(request, QueryTimeoutError);
  t.mock.timers.tick(ACTIVITY_READ_TIMEOUT_MS);
  await rejected;
  assert.equal(f.unsubscribed, 1);
  f.emit("late result");
  assert.equal(f.unsubscribed, 1);
});

test("scope cancellation ends the operation before its deadline", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = fixture();
  const controller = new AbortController();
  const request = boundedQuery(f.source, controller.signal);
  const rejected = assert.rejects(request, { name: "AbortError" });
  controller.abort();
  await rejected;
  f.emit("old scope");
  t.mock.timers.tick(ACTIVITY_READ_TIMEOUT_MS);
  assert.equal(f.unsubscribed, 1);
});

test("stale emissions wait for the fresh result and clear the deadline", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = fixture();
  const request = boundedQuery(f.source, new AbortController().signal);
  f.emit("cached", true);
  assert.equal(f.unsubscribed, 0);
  f.emit("fresh");
  assert.equal((await request).data, "fresh");
  t.mock.timers.tick(ACTIVITY_READ_TIMEOUT_MS);
  assert.equal(f.unsubscribed, 1);
});

test("synchronous results also release the subscription", async () => {
  let unsubscribed = false;
  const source = { subscribe(next: (value: { data: string; stale: boolean }) => void) {
    next({ data: "ready", stale: false });
    return { unsubscribe() { unsubscribed = true; } };
  } };
  assert.equal((await boundedQuery(source, new AbortController().signal)).data, "ready");
  assert.equal(unsubscribed, true);
});
