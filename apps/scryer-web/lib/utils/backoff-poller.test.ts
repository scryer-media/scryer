import assert from "node:assert/strict";
import test from "node:test";

import { createBackoffPoller } from "./backoff-poller.ts";

/** A clock the test advances by hand: nothing fires on its own. */
function manualClock() {
  let nextHandle = 0;
  const pending = new Map<number, { callback: () => void; delayMs: number }>();
  return {
    clock: {
      setTimeout(callback: () => void, delayMs: number) {
        nextHandle += 1;
        pending.set(nextHandle, { callback, delayMs });
        return nextHandle;
      },
      clearTimeout(handle: unknown) {
        pending.delete(handle as number);
      },
    },
    pendingDelays: () => [...pending.values()].map((timer) => timer.delayMs),
    /** Fire every pending timer, then let the polls they started settle. */
    async fire() {
      const due = [...pending.values()];
      pending.clear();
      for (const timer of due) timer.callback();
      await settle();
    },
  };
}

async function settle() {
  for (let turn = 0; turn < 5; turn += 1) await Promise.resolve();
}

test("the waits back off and the last one repeats", async () => {
  const time = manualClock();
  const poller = createBackoffPoller({
    delaysMs: [1, 5, 30],
    poll: async () => true,
    isHidden: () => false,
    clock: time.clock,
  });

  poller.restart();
  const delays: number[] = [];
  for (let index = 0; index < 5; index += 1) {
    delays.push(...time.pendingDelays());
    await time.fire();
  }

  assert.deepEqual(delays, [1, 5, 30, 30, 30]);
});

test("no timer is left once there is nothing to watch", async () => {
  const time = manualClock();
  let watching = 2;
  const poller = createBackoffPoller({
    delaysMs: [1],
    poll: async () => (watching -= 1) > 0,
    isHidden: () => false,
    clock: time.clock,
  });

  poller.restart();
  await time.fire();
  assert.deepEqual(time.pendingDelays(), [1]);
  await time.fire();

  assert.deepEqual(time.pendingDelays(), []);
  assert.equal(poller.isActive(), false);
});

test("stopping clears the timer and ignores a poll still under way", async () => {
  const time = manualClock();
  let finishPoll: (keepWatching: boolean) => void = () => {};
  const poller = createBackoffPoller({
    delaysMs: [1],
    poll: () => new Promise<boolean>((resolve) => (finishPoll = resolve)),
    isHidden: () => false,
    clock: time.clock,
  });

  poller.restart();
  await time.fire();
  poller.stop();
  finishPoll(true);
  await settle();

  assert.deepEqual(time.pendingDelays(), []);
  assert.equal(poller.isActive(), false);
});

test("a hidden page holds the due poll until it is shown", async () => {
  const time = manualClock();
  let hidden = true;
  let polls = 0;
  const poller = createBackoffPoller({
    delaysMs: [1],
    poll: async () => {
      polls += 1;
      return true;
    },
    isHidden: () => hidden,
    clock: time.clock,
  });

  poller.restart();
  await time.fire();
  assert.equal(polls, 0);
  assert.deepEqual(time.pendingDelays(), [], "no timer runs while hidden");

  hidden = false;
  poller.resume();
  await settle();
  assert.equal(polls, 1);
  assert.deepEqual(time.pendingDelays(), [1]);
});

test("a poll that throws keeps watching", async () => {
  const time = manualClock();
  const poller = createBackoffPoller({
    delaysMs: [1],
    poll: async () => {
      throw new Error("offline");
    },
    isHidden: () => false,
    clock: time.clock,
  });

  poller.restart();
  await time.fire();

  assert.deepEqual(time.pendingDelays(), [1]);
  assert.equal(poller.isActive(), true);
});
