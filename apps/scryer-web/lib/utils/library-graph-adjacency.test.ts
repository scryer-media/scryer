import assert from "node:assert/strict";
import test from "node:test";

import { GraphAdjacency } from "./library-graph-adjacency.ts";

test("adjacency links known nodes once and answer adjacency both ways", () => {
  const relations = new GraphAdjacency<{ weight: number }>(["a", "b", "c"]);
  assert.equal(relations.add("a", "b", { weight: 1 }), true);
  assert.equal(relations.add("b", "a", { weight: 2 }), false);
  assert.equal(relations.add("a", "a", { weight: 3 }), false);
  assert.equal(relations.add("a", "missing", { weight: 4 }), false);
  assert.equal(relations.add("c", "a", { weight: 5 }), true);

  assert.equal(relations.hasEdgeBetween("b", "a"), true);
  assert.equal(relations.hasEdgeBetween("b", "c"), false);
  assert.deepEqual(relations.neighbors("a").sort(), ["b", "c"]);
  assert.deepEqual(relations.neighbors("b"), ["a"]);
  assert.deepEqual(relations.edges("missing"), []);

  const [key] = relations.edges("b");
  assert.equal(relations.hasEdge(key), true);
  assert.deepEqual(relations.extremities(key), ["a", "b"]);
  assert.deepEqual(relations.getEdgeAttributes(key), { weight: 1 });
});
