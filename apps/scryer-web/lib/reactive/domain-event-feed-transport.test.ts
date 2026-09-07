import assert from "node:assert/strict";
import test from "node:test";

import { createReactiveRefreshEngine } from "./domain-event-feed.ts";
import { startDomainEventFeedTransport } from "./domain-event-feed-transport.ts";

type TransportSink = {
  next: (result: unknown) => void;
  error: (error: unknown) => void;
  complete: () => void;
};

// The transport calls `subscribe` synchronously, but TS can't prove the closure
// ran, so capture the sink behind a getter that reads its declared type.
function createSinkCapture() {
  let sink: TransportSink | null = null;
  return {
    set(next: TransportSink) {
      sink = next;
    },
    get(): TransportSink {
      if (!sink) {
        throw new Error("subscription sink was not captured");
      }
      return sink;
    },
  };
}

// Deterministic scheduler so reconnect/fallback timing is driven by the test.
function createManualScheduler() {
  let nextId = 1;
  const timeouts = new Map<number, () => void>();
  const intervals = new Map<number, () => void>();
  return {
    scheduler: {
      setTimeout(handler: () => void) {
        const id = nextId++;
        timeouts.set(id, handler);
        return id;
      },
      clearTimeout(handle: unknown) {
        timeouts.delete(handle as number);
      },
      setInterval(handler: () => void) {
        const id = nextId++;
        intervals.set(id, handler);
        return id;
      },
      clearInterval(handle: unknown) {
        intervals.delete(handle as number);
      },
    },
    runTimeouts() {
      const pending = Array.from(timeouts.values());
      timeouts.clear();
      for (const handler of pending) {
        handler();
      }
    },
    tickIntervals() {
      for (const handler of Array.from(intervals.values())) {
        handler();
      }
    },
    intervalCount() {
      return intervals.size;
    },
  };
}

test("resubscribes with the last-seen afterSequence for lossless catch-up", () => {
  const engine = createReactiveRefreshEngine();
  const sched = createManualScheduler();
  const capture = createSinkCapture();
  const afterSequences: Array<number | string | null> = [];

  const transport = startDomainEventFeedTransport({
    query: "SUB",
    engine,
    subscribe: (request, sink) => {
      afterSequences.push(request.variables.afterSequence);
      capture.set(sink);
      return () => {};
    },
    scheduler: sched.scheduler,
  });

  // The first subscription opens with a null cursor.
  assert.deepEqual(afterSequences, [null]);

  // The server delivers an event at sequence 5; the engine advances its cursor.
  capture.get().next({
    data: { domainEventFeed: { sequence: 5, eventType: "title_updated", titleId: "A" } },
  });
  assert.equal(engine.afterSequence(), 5);

  // The subscription drops; the transport schedules and runs a reconnect.
  capture.get().complete();
  sched.runTimeouts();

  // The resubscribe passes afterSequence = 5 so the store replays the gap.
  assert.deepEqual(afterSequences, [null, 5]);
  transport.stop();
});

test("degrades to an interval refresh after repeated failures and recovers on delivery", () => {
  const engineSched = createManualScheduler();
  const engine = createReactiveRefreshEngine({ scheduler: engineSched.scheduler });
  let ran = 0;
  engine.register({
    aliasKey: "a",
    predicate: () => false,
    run: () => {
      ran += 1;
    },
  });

  const sched = createManualScheduler();
  const capture = createSinkCapture();
  const warnings: string[] = [];

  const transport = startDomainEventFeedTransport({
    query: "SUB",
    engine,
    subscribe: (_request, sink) => {
      capture.set(sink);
      return () => {};
    },
    fallbackFailureThreshold: 3,
    scheduler: sched.scheduler,
    warn: (message) => warnings.push(message),
    logError: () => {},
  });

  assert.equal(transport.isDegraded(), false);

  // Establish the replay cursor first. An unanchored reconnect uses the
  // stronger immediate fallback covered by the next test.
  capture.get().next({ data: { domainEventFeed: { sequence: 1 } } });

  // Three consecutive drops (each followed by a reconnect) trip the fallback.
  for (let i = 0; i < 3; i += 1) {
    capture.get().error(new Error("drop"));
    sched.runTimeouts();
  }
  assert.equal(transport.isDegraded(), true);
  assert.equal(warnings.length, 1);
  assert.equal(sched.intervalCount(), 1);

  // The degraded interval refreshes every registered alias (predicate ignored).
  sched.tickIntervals();
  engineSched.runTimeouts();
  assert.equal(ran, 1);

  // A successful delivery clears the degraded state and stops the interval.
  capture.get().next({ data: { domainEventFeed: { sequence: 2 } } });
  assert.equal(transport.isDegraded(), false);
  assert.equal(sched.intervalCount(), 0);

  transport.stop();
});

test("refreshes while reconnecting without an event cursor", () => {
  const engineSched = createManualScheduler();
  const engine = createReactiveRefreshEngine({ scheduler: engineSched.scheduler });
  let ran = 0;
  engine.register({
    aliasKey: "a",
    predicate: () => false,
    run: () => {
      ran += 1;
    },
  });
  const sched = createManualScheduler();
  const capture = createSinkCapture();
  const afterSequences: Array<number | string | null> = [];
  const transport = startDomainEventFeedTransport({
    query: "SUB",
    engine,
    subscribe: (request, sink) => {
      afterSequences.push(request.variables.afterSequence);
      capture.set(sink);
      return () => {};
    },
    scheduler: sched.scheduler,
    logError: () => {},
    warn: () => {},
  });

  capture.get().error(new Error("drop"));
  sched.runTimeouts();
  engineSched.runTimeouts();
  assert.deepEqual(afterSequences, [null, null]);
  assert.equal(transport.isDegraded(), true);
  assert.equal(ran, 1);

  sched.tickIntervals();
  engineSched.runTimeouts();
  assert.equal(ran, 2);

  capture.get().next({ data: { domainEventFeed: { sequence: 7 } } });
  engineSched.runTimeouts();
  assert.equal(ran, 3);
  assert.equal(transport.isDegraded(), false);

  capture.get().complete();
  sched.runTimeouts();
  assert.deepEqual(afterSequences, [null, null, 7]);
  transport.stop();
});
