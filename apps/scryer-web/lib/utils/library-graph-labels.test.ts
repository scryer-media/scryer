import assert from "node:assert/strict";
import test from "node:test";
import { showOverviewTitleLabels } from "./library-graph-labels.ts";

test("close overview shows labels including when some nodes are tiny", () => {
  const nodes = Array.from({ length: 24 }, (_, i) => ({ r: i === 0 ? 4 : 28 }));
  assert.equal(showOverviewTitleLabels(nodes, 1200, 1000), true);
});

test("small library zoomed out keeps title labels hidden", () => {
  const nodes = Array.from({ length: 24 }, () => ({ r: 13 }));
  assert.equal(showOverviewTitleLabels(nodes, 1200, 1000), false);
});

test("crowded views hide labels even with large nodes", () => {
  const nodes = Array.from({ length: 200 }, () => ({ r: 28 }));
  assert.equal(showOverviewTitleLabels(nodes, 2400, 1800), false);
  assert.equal(showOverviewTitleLabels(nodes.slice(0, 30), 400, 400), false);
});

test("empty view has no title labels", () => {
  assert.equal(showOverviewTitleLabels([], 1200, 1000), false);
});
