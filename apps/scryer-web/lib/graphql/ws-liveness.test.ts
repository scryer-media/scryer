import assert from "node:assert/strict";
import test from "node:test";

import { createWsLiveness, WS_PONG_TIMEOUT_CLOSE_CODE } from "./ws-liveness.ts";

function fakeTimers() {
  const pending = new Map<number, () => void>();
  let next = 0;
  return {
    timers: {
      setTimeout: (fn: () => void) => {
        next += 1;
        pending.set(next, fn);
        return next;
      },
      clearTimeout: (handle: unknown) => {
        pending.delete(handle as number);
      },
    },
    pendingCount: () => pending.size,
    fireAll: () => {
      const due = [...pending.values()];
      pending.clear();
      for (const fn of due) fn();
    },
  };
}

function fakeSocket() {
  const socket = {
    readyState: 1,
    sent: [] as string[],
    closedWith: null as number | null,
    send(data: string) {
      socket.sent.push(data);
    },
    close(code?: number) {
      socket.closedWith = code ?? null;
      socket.readyState = 3;
    },
  };
  return socket;
}

test("an unanswered ping closes the socket so it reconnects", () => {
  const clock = fakeTimers();
  const liveness = createWsLiveness({
    pongTimeoutMs: 5000,
    timers: clock.timers,
  });
  const socket = fakeSocket();
  liveness.connected(socket, false);

  liveness.ping(false);
  clock.fireAll();

  assert.equal(socket.closedWith, WS_PONG_TIMEOUT_CLOSE_CODE);
});

test("a pong in time keeps the socket open", () => {
  const clock = fakeTimers();
  const liveness = createWsLiveness({
    pongTimeoutMs: 5000,
    timers: clock.timers,
  });
  const socket = fakeSocket();
  liveness.connected(socket, false);

  liveness.ping(false);
  liveness.pong(true);
  clock.fireAll();

  assert.equal(socket.closedWith, null);
  assert.equal(clock.pendingCount(), 0);
});

test("probe pings an open socket at once and waits for the pong", () => {
  const clock = fakeTimers();
  const liveness = createWsLiveness({
    pongTimeoutMs: 5000,
    timers: clock.timers,
  });
  const socket = fakeSocket();
  liveness.connected(socket, false);

  liveness.probe();
  liveness.probe();

  assert.deepEqual(socket.sent, [JSON.stringify({ type: "ping" })]);
  clock.fireAll();
  assert.equal(socket.closedWith, WS_PONG_TIMEOUT_CLOSE_CODE);
});

test("probe does nothing without an open socket", () => {
  const clock = fakeTimers();
  const liveness = createWsLiveness({
    pongTimeoutMs: 5000,
    timers: clock.timers,
  });
  liveness.probe();
  const socket = fakeSocket();
  liveness.connected(socket, false);
  liveness.closed();
  liveness.probe();

  assert.deepEqual(socket.sent, []);
  assert.equal(clock.pendingCount(), 0);
});

test("a stale pong timer never closes the socket that replaced it", () => {
  const clock = fakeTimers();
  const liveness = createWsLiveness({
    pongTimeoutMs: 5000,
    timers: clock.timers,
  });
  const first = fakeSocket();
  liveness.connected(first, false);
  liveness.ping(false);
  const second = fakeSocket();
  liveness.connected(second, true);
  clock.fireAll();

  assert.equal(first.closedWith, null);
  assert.equal(second.closedWith, null);
});

test("reconnect listeners hear retries only, until they unsubscribe", () => {
  const liveness = createWsLiveness({ pongTimeoutMs: 5000 });
  let heard = 0;
  const stop = liveness.onReconnected(() => {
    heard += 1;
  });

  liveness.connected(fakeSocket(), false);
  liveness.connected(fakeSocket(), true);
  stop();
  liveness.connected(fakeSocket(), true);

  assert.equal(heard, 1);
});
