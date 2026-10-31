import assert from "node:assert/strict";
import test from "node:test";

import { createTrailingThrottle } from "./trailing-throttle.ts";

test("a burst runs once at once and once more after the interval", (t) => {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 0 });
  let runs = 0;
  const throttle = createTrailingThrottle(() => {
    runs += 1;
  }, 1_000);

  throttle.request();
  assert.equal(runs, 1, "the first request runs immediately");

  for (let index = 0; index < 20; index += 1) {
    t.mock.timers.tick(10);
    throttle.request();
  }
  assert.equal(runs, 1, "requests inside the interval wait");

  t.mock.timers.tick(799);
  assert.equal(runs, 1);
  t.mock.timers.tick(1);
  assert.equal(
    runs,
    2,
    "one trailing run follows the burst at the interval's end",
  );

  t.mock.timers.tick(5_000);
  assert.equal(runs, 2, "nothing else runs without a new request");
});

test("a request after a quiet interval runs immediately", (t) => {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 0 });
  let runs = 0;
  const throttle = createTrailingThrottle(() => {
    runs += 1;
  }, 1_000);

  throttle.request();
  t.mock.timers.tick(1_500);
  throttle.request();
  assert.equal(runs, 2);
});

test("a steady stream never runs more than once per interval", (t) => {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 0 });
  const runAt: number[] = [];
  const throttle = createTrailingThrottle(() => {
    runAt.push(Date.now());
  }, 1_000);

  for (let elapsed = 0; elapsed < 5_000; elapsed += 50) {
    throttle.request();
    t.mock.timers.tick(50);
  }
  t.mock.timers.tick(1_000);

  for (let index = 1; index < runAt.length; index += 1) {
    assert.ok(runAt[index] - runAt[index - 1] >= 1_000);
  }
  assert.ok(runAt.at(-1)! >= 4_950, "the last request is followed by a run");
});

test("cancel drops the queued trailing run", (t) => {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 0 });
  let runs = 0;
  const throttle = createTrailingThrottle(() => {
    runs += 1;
  }, 1_000);

  throttle.request();
  throttle.request();
  throttle.cancel();
  t.mock.timers.tick(2_000);
  assert.equal(runs, 1);
});
