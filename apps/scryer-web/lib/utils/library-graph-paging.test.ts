import assert from "node:assert/strict";
import test from "node:test";

import {
  INITIAL_GRAPH_PAGING_STATE,
  graphPagingStep,
  graphTitlesComplete,
  type GraphPagingInput,
  type GraphPagingState,
} from "./library-graph-paging.ts";

const base: GraphPagingInput = {
  active: true,
  listingKey: "listing-a",
  hasMore: true,
  loadingMore: false,
  pageFailed: false,
  loadedCount: 100,
};

function run(inputs: Partial<GraphPagingInput>[]) {
  let state: GraphPagingState = INITIAL_GRAPH_PAGING_STATE;
  return inputs.map((input) => {
    const step = graphPagingStep(state, { ...base, ...input });
    state = step.state;
    return { request: step.request, stalled: state.stalled };
  });
}

test("pages until the listing has no more", () => {
  assert.deepEqual(
    run([
      { loadedCount: 100 },
      { loadedCount: 100, loadingMore: true },
      { loadedCount: 200 },
      { loadedCount: 200, loadingMore: true },
      { loadedCount: 220, hasMore: false },
    ]),
    [
      { request: true, stalled: false },
      { request: false, stalled: false },
      { request: true, stalled: false },
      { request: false, stalled: false },
      { request: false, stalled: false },
    ],
  );
});

test("a page that adds no titles stops paging instead of asking again", () => {
  assert.deepEqual(
    run([
      { loadedCount: 100 },
      { loadedCount: 100, loadingMore: true },
      // The page came back empty but the catalog still says there is more.
      { loadedCount: 100 },
      { loadedCount: 100 },
    ]),
    [
      { request: true, stalled: false },
      { request: false, stalled: false },
      { request: false, stalled: true },
      { request: false, stalled: true },
    ],
  );
});

test("a new listing pages afresh even when its first page is as long as the old one", () => {
  assert.deepEqual(
    run([
      // Page 1 of the old listing, then page 2 is asked for.
      { listingKey: "listing-a", loadedCount: 100 },
      { listingKey: "listing-a", loadedCount: 100, loadingMore: true },
      // A filter changes while page 2 is in flight; the old page is dropped
      // and the new listing's first page is a full page with more to come.
      { listingKey: "listing-b", loadedCount: 100, loadingMore: true },
      { listingKey: "listing-b", loadedCount: 100 },
      { listingKey: "listing-b", loadedCount: 100, loadingMore: true },
      { listingKey: "listing-b", loadedCount: 160, hasMore: false },
    ]),
    [
      { request: true, stalled: false },
      { request: false, stalled: false },
      { request: false, stalled: false },
      { request: true, stalled: false },
      { request: false, stalled: false },
      { request: false, stalled: false },
    ],
  );
});

test("a stall belongs to its listing", () => {
  assert.deepEqual(
    run([
      { listingKey: "listing-a" },
      { listingKey: "listing-a" },
      { listingKey: "listing-b", loadedCount: 40 },
    ]),
    [
      { request: true, stalled: false },
      { request: false, stalled: true },
      { request: true, stalled: false },
    ],
  );
});

test("a failed page ends paging, and nothing pages while the graph is hidden", () => {
  assert.deepEqual(run([{ pageFailed: true }]), [
    { request: false, stalled: false },
  ]);
  const hidden = graphPagingStep(INITIAL_GRAPH_PAGING_STATE, {
    ...base,
    active: false,
  });
  assert.equal(hidden.request, false);
  assert.deepEqual(hidden.state, INITIAL_GRAPH_PAGING_STATE);
});

test("titles are complete only once the listing in view has finished paging", () => {
  const complete = {
    listingKey: "listing-b",
    titleLoading: false,
    hasMore: false,
    pageFailed: false,
    stalledListingKey: null,
  };
  assert.equal(graphTitlesComplete(complete), true);
  // The first page of a new listing has landed with more to come.
  assert.equal(graphTitlesComplete({ ...complete, hasMore: true }), false);
  // The new listing's first page is still loading.
  assert.equal(graphTitlesComplete({ ...complete, titleLoading: true }), false);
  assert.equal(graphTitlesComplete({ ...complete, listingKey: "" }), false);
  assert.equal(
    graphTitlesComplete({ ...complete, hasMore: true, pageFailed: true }),
    true,
  );
  // A stall in another listing does not finish this one.
  assert.equal(
    graphTitlesComplete({
      ...complete,
      hasMore: true,
      stalledListingKey: "listing-a",
    }),
    false,
  );
  assert.equal(
    graphTitlesComplete({
      ...complete,
      hasMore: true,
      stalledListingKey: "listing-b",
    }),
    true,
  );
});
