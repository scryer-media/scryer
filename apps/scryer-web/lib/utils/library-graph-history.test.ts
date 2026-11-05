import assert from "node:assert/strict";
import test from "node:test";
import {
  createGraphHistorySync,
  graphHistorySnapshot,
  graphHistoryState,
} from "./library-graph-history.ts";

test("overview → title → title restores in both browser directions", () => {
  const sync = createGraphHistorySync<string>();
  const scope = "movie|library";
  const states = ["overview", "title-a", "title-b"].map((snapshot) =>
    structuredClone(graphHistoryState({ unrelated: true }, scope, snapshot)),
  );
  assert.equal(sync.visit("0", "POP", null, scope).kind, "persist");
  for (const index of [1, 2]) {
    sync.beginPush();
    assert.equal(
      sync.visit(String(index - 1), "PUSH", states[index - 1], scope).kind,
      "skip",
    );
    assert.equal(
      sync.visit(String(index), "PUSH", states[index], scope).kind,
      "persist",
    );
  }
  for (const index of [1, 0, 1, 2]) {
    assert.deepEqual(sync.visit(String(index), "POP", states[index], scope), {
      kind: "restore",
      snapshot: ["overview", "title-a", "title-b"][index],
    });
  }
});

test("asynchronous results and poster dismissal replace rather than navigate", () => {
  const sync = createGraphHistorySync();
  const scope = "movie|library";
  sync.visit("a", "PUSH", null, scope);
  const snapshot = {
    selected: "title-a",
    posterOpen: false,
    ghosts: new Map([["ghost:b", { title: "B" }]]),
  };
  const state = structuredClone(
    graphHistoryState({ routeData: 42 }, scope, snapshot),
  );
  assert.equal(sync.visit("a", "PUSH", state, scope).kind, "persist");
  assert.equal(Reflect.get(state, "routeData"), 42);
  sync.visit("b", "PUSH", null, scope);
  assert.deepEqual(sync.visit("a", "POP", state, scope), {
    kind: "restore",
    snapshot,
  });
});

test("history from another facet or library set is not restored", () => {
  const state = graphHistoryState(null, "movie|a", { selected: "a" });
  assert.equal(graphHistorySnapshot(state, "series|a"), null);
  assert.equal(graphHistorySnapshot(state, "movie|b"), null);
  assert.equal(graphHistorySnapshot(null, "movie|a"), null);
});
