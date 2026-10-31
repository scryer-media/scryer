import assert from "node:assert/strict";
import test from "node:test";

import {
  allOf,
  anyOf,
  createReactiveRefreshEngine,
  type DomainEvent,
  forEventTypes,
  forStreamKind,
  forTitle,
  normalizeDomainEvent,
  normalizeSequence,
  not,
} from "./domain-event-feed.ts";

function makeEvent(overrides: Partial<DomainEvent> = {}): DomainEvent {
  return {
    sequence: null,
    eventId: null,
    eventType: null,
    titleId: null,
    facet: null,
    streamKind: null,
    streamId: null,
    ...overrides,
  };
}

// Deterministic scheduler so debounce timing is fully controlled by the test.
function createManualScheduler() {
  let nextId = 1;
  const tasks = new Map<number, () => void>();
  return {
    scheduler: {
      setTimeout(handler: () => void) {
        const id = nextId++;
        tasks.set(id, handler);
        return id;
      },
      clearTimeout(handle: unknown) {
        tasks.delete(handle as number);
      },
    },
    flushAll() {
      const pending = Array.from(tasks.values());
      tasks.clear();
      for (const handler of pending) {
        handler();
      }
    },
    pendingCount() {
      return tasks.size;
    },
  };
}

test("registry dispatch fires only aliases whose predicate matches", () => {
  const { scheduler, flushAll } = createManualScheduler();
  const engine = createReactiveRefreshEngine({ debounceMs: 300, scheduler });
  const runs: string[] = [];
  engine.register({
    aliasKey: "titleX",
    predicate: forTitle("X"),
    run: () => runs.push("titleX"),
  });
  engine.register({
    aliasKey: "titleY",
    predicate: forTitle("Y"),
    run: () => runs.push("titleY"),
  });
  engine.register({
    aliasKey: "config",
    predicate: forEventTypes("configuration_changed"),
    run: () => runs.push("config"),
  });

  const event = makeEvent({
    sequence: 1,
    titleId: "X",
    eventType: "title_updated",
  });

  // Pure matching: only the forTitle("X") alias matches.
  assert.deepEqual(engine.matchingAliasKeys(event), ["titleX"]);

  assert.deepEqual(engine.handleEvent(event), ["titleX"]);
  flushAll();
  assert.deepEqual(runs, ["titleX"]);
});

test("unregister removes an alias from dispatch", () => {
  const { scheduler, flushAll } = createManualScheduler();
  const engine = createReactiveRefreshEngine({ scheduler });
  const runs: string[] = [];
  const off = engine.register({
    aliasKey: "x",
    predicate: forTitle("X"),
    run: () => runs.push("x"),
  });
  off();
  engine.handleEvent(makeEvent({ sequence: 1, titleId: "X" }));
  flushAll();
  assert.equal(runs.length, 0);
  assert.equal(engine.size(), 0);
});

test("sequence cursor tracks the last seen sequence for afterSequence resubscribe", () => {
  const { scheduler } = createManualScheduler();
  const engine = createReactiveRefreshEngine({ scheduler });
  assert.equal(engine.afterSequence(), null);

  engine.handleEvent(
    makeEvent({ sequence: 5, eventType: "title_updated", titleId: "A" }),
  );
  // A non-matching event still advances the cursor so catch-up stays lossless.
  engine.handleEvent(
    makeEvent({ sequence: 6, eventType: "configuration_changed" }),
  );

  assert.equal(engine.afterSequence(), 6);
  assert.equal(engine.lastSequence(), 6);
});

test("sequence cursor preserves raw Long strings and ignores replays", () => {
  const { scheduler, flushAll } = createManualScheduler();
  const engine = createReactiveRefreshEngine({ scheduler });
  const runs: number[] = [];
  engine.register({
    aliasKey: "any",
    predicate: () => true,
    run: () => runs.push(1),
  });

  engine.handleEvent(makeEvent({ sequence: "42", eventType: "title_updated" }));
  // Raw value is preserved for the resubscribe variable; numeric for compare.
  assert.equal(engine.afterSequence(), "42");
  assert.equal(engine.lastSequence(), 42);

  // A replayed (already-seen) sequence dispatches nothing.
  assert.deepEqual(
    engine.handleEvent(makeEvent({ sequence: 42, eventType: "title_updated" })),
    [],
  );
  flushAll();
  assert.equal(runs.length, 1);
});

test("debounce coalesces a burst of matching events into one run per alias", () => {
  const { scheduler, flushAll, pendingCount } = createManualScheduler();
  const engine = createReactiveRefreshEngine({ debounceMs: 300, scheduler });
  let runCount = 0;
  engine.register({
    aliasKey: "config",
    predicate: forEventTypes("configuration_changed"),
    run: () => {
      runCount += 1;
    },
  });

  for (let i = 1; i <= 5; i += 1) {
    engine.handleEvent(
      makeEvent({ sequence: i, eventType: "configuration_changed" }),
    );
  }

  // Nothing has run yet and only one debounce timer is pending.
  assert.equal(runCount, 0);
  assert.equal(pendingCount(), 1);

  flushAll();
  assert.equal(runCount, 1);
});

test("runAll queues every registered alias for the degraded fallback path", () => {
  const { scheduler, flushAll } = createManualScheduler();
  const engine = createReactiveRefreshEngine({ scheduler });
  const runs: string[] = [];
  engine.register({
    aliasKey: "a",
    predicate: () => false,
    run: () => runs.push("a"),
  });
  engine.register({
    aliasKey: "b",
    predicate: () => false,
    run: () => runs.push("b"),
  });
  engine.runAll();
  flushAll();
  assert.deepEqual(runs.sort(), ["a", "b"]);
});

test("a throwing run is routed to onError without aborting the flush", () => {
  const { scheduler, flushAll } = createManualScheduler();
  const engine = createReactiveRefreshEngine({ scheduler });
  const errors: unknown[] = [];
  const runs: string[] = [];
  engine.register({
    aliasKey: "boom",
    predicate: () => true,
    run: () => {
      throw new Error("boom");
    },
    onError: (error) => errors.push(error),
  });
  engine.register({
    aliasKey: "ok",
    predicate: () => true,
    run: () => runs.push("ok"),
  });
  engine.handleEvent(makeEvent({ sequence: 1 }));
  flushAll();
  assert.equal(errors.length, 1);
  assert.deepEqual(runs, ["ok"]);
});

test("predicate combinators compose", () => {
  const p = allOf(
    forEventTypes("title_updated"),
    not(forStreamKind("archive")),
  );
  assert.equal(
    p(makeEvent({ eventType: "title_updated", streamKind: "title" })),
    true,
  );
  assert.equal(
    p(makeEvent({ eventType: "title_updated", streamKind: "archive" })),
    false,
  );

  const q = anyOf(forTitle("A"), forStreamKind("jobs"));
  assert.equal(q(makeEvent({ titleId: "A" })), true);
  assert.equal(q(makeEvent({ streamKind: "jobs" })), true);
  assert.equal(q(makeEvent({ titleId: "B" })), false);
});

test("normalizeDomainEvent and normalizeSequence are defensive", () => {
  const event = normalizeDomainEvent({
    sequence: 7,
    eventType: "title_added",
    titleId: "t1",
    facet: "movie",
    streamKind: "title",
    streamId: "s1",
    payloadJson: { a: 1 },
  });
  assert.equal(event.sequence, 7);
  assert.equal(event.eventType, "title_added");
  assert.equal(event.titleId, "t1");
  assert.deepEqual(event.payloadJson, { a: 1 });

  const empty = normalizeDomainEvent(null);
  assert.equal(empty.eventType, null);
  assert.equal(empty.titleId, null);

  assert.equal(normalizeSequence("nan"), null);
  assert.equal(normalizeSequence(3), 3);
  assert.equal(normalizeSequence(null), null);
});
