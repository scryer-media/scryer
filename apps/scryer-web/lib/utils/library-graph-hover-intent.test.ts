import assert from "node:assert/strict";
import test from "node:test";
import {
  createGraphHoverIntent,
  GRAPH_HOVER_DELAY_MS,
} from "./library-graph-hover-intent.ts";

test("sweeping across a hundred posters does not highlight any of them", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const intent = createGraphHoverIntent();
  const activations: number[] = [];
  for (let index = 0; index < 100; index++) {
    intent.enter(String(index), () => activations.push(index));
    t.mock.timers.tick(1);
    intent.leave(String(index));
  }
  t.mock.timers.tick(GRAPH_HOVER_DELAY_MS);
  assert.equal(activations.length, 0);
  intent.enter("settled", () => activations.push(100));
  t.mock.timers.tick(GRAPH_HOVER_DELAY_MS - 1);
  assert.equal(activations.length, 0);
  t.mock.timers.tick(1);
  assert.deepEqual(activations, [100]);
  t.mock.timers.tick(GRAPH_HOVER_DELAY_MS);
  assert.deepEqual(activations, [100]);
});

test("a replacement gets its own delay and an old leave event cannot cancel it", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const intent = createGraphHoverIntent();
  const activations: string[] = [];
  intent.enter("first", () => activations.push("first"));
  t.mock.timers.tick(40);
  intent.enter("second", () => activations.push("second"));
  intent.leave("first");
  t.mock.timers.tick(35);
  assert.equal(activations.length, 0);
  t.mock.timers.tick(40);
  assert.deepEqual(activations, ["second"]);
});

test("replacement or teardown cancels pending work, and repeated moves do not postpone intent", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const intent = createGraphHoverIntent();
  const activations: string[] = [];
  intent.enter("old", () => activations.push("old"));
  intent.cancel();
  t.mock.timers.tick(GRAPH_HOVER_DELAY_MS);
  assert.equal(activations.length, 0);
  intent.enter("new", () => activations.push("new"));
  t.mock.timers.tick(40);
  intent.enter("new", () => activations.push("duplicate"));
  t.mock.timers.tick(GRAPH_HOVER_DELAY_MS - 40);
  assert.deepEqual(activations, ["new"]);
});
