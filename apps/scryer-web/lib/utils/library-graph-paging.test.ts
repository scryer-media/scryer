import assert from "node:assert/strict";
import test from "node:test";

import {
  INITIAL_GRAPH_PAGING_STATE,
  graphPagingStep,
  type GraphPagingInput,
  type GraphPagingState,
} from "./library-graph-paging.ts";

const base: GraphPagingInput = {
  active: true,
  hasMore: true,
  loadingMore: false,
  pageFailed: false,
  loadedCount: 50,
};

function run(inputs: Partial<GraphPagingInput>[]) {
  let state: GraphPagingState = INITIAL_GRAPH_PAGING_STATE;
  return inputs.map((input) => {
    const step = graphPagingStep(state, { ...base, ...input });
    state = step.state;
    return { request: step.request, complete: step.complete };
  });
}

test("pages until the catalog has no more, then the titles are complete", () => {
  assert.deepEqual(
    run([
      { loadedCount: 50 },
      { loadedCount: 50, loadingMore: true },
      { loadedCount: 100 },
      { loadedCount: 100, loadingMore: true },
      { loadedCount: 120, hasMore: false },
    ]),
    [
      { request: true, complete: false },
      { request: false, complete: false },
      { request: true, complete: false },
      { request: false, complete: false },
      { request: false, complete: true },
    ],
  );
});

test("a page that adds no titles stops paging instead of asking again", () => {
  assert.deepEqual(
    run([
      { loadedCount: 50 },
      { loadedCount: 50, loadingMore: true },
      // The page came back empty but the catalog still says there is more.
      { loadedCount: 50 },
      { loadedCount: 50 },
    ]),
    [
      { request: true, complete: false },
      { request: false, complete: false },
      { request: false, complete: true },
      { request: false, complete: true },
    ],
  );
});

test("a failed page ends paging with what loaded", () => {
  assert.deepEqual(run([{ pageFailed: true }]), [
    { request: false, complete: true },
  ]);
});

test("a new listing pages again after a stall", () => {
  assert.deepEqual(
    run([
      { loadedCount: 50 },
      { loadedCount: 50 },
      // A filter change starts the listing over with fewer titles.
      { loadedCount: 20 },
    ]),
    [
      { request: true, complete: false },
      { request: false, complete: true },
      { request: true, complete: false },
    ],
  );
});

test("nothing is requested while the graph is not shown", () => {
  const step = graphPagingStep(INITIAL_GRAPH_PAGING_STATE, {
    ...base,
    active: false,
  });
  assert.equal(step.request, false);
  assert.deepEqual(step.state, INITIAL_GRAPH_PAGING_STATE);
});
