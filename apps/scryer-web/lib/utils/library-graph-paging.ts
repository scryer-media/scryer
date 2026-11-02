// The graph draws the whole filtered catalog, so while it is shown the catalog
// keeps paging until every title is loaded. This decides, each time the
// catalog's paging state changes, whether to ask for the next page and
// whether the loaded titles are complete enough to build the graph from.

export type GraphPagingState = {
  /** How many titles were loaded when the outstanding request was made. */
  requestedAtCount: number | null;
  /** A page came back without adding a title, so paging stopped. */
  stalled: boolean;
  lastCount: number;
  lastHasMore: boolean;
};

export type GraphPagingInput = {
  active: boolean;
  hasMore: boolean;
  loadingMore: boolean;
  pageFailed: boolean;
  loadedCount: number;
};

export type GraphPagingStep = {
  state: GraphPagingState;
  /** Ask for the next page now. */
  request: boolean;
  /** Paging is over: every page is in, or it stopped on a failure or a stall. */
  complete: boolean;
};

export const INITIAL_GRAPH_PAGING_STATE: GraphPagingState = {
  requestedAtCount: null,
  stalled: false,
  lastCount: 0,
  lastHasMore: false,
};

export function graphPagingStep(
  previous: GraphPagingState,
  input: GraphPagingInput,
): GraphPagingStep {
  if (!input.active) {
    return {
      state: INITIAL_GRAPH_PAGING_STATE,
      request: false,
      complete: !input.hasMore,
    };
  }
  // Fewer titles than before, or more pages after the end, is a new listing:
  // a filter changed or the catalog was refreshed. Paging starts over.
  const restarted =
    input.loadedCount < previous.lastCount ||
    (input.hasMore && !previous.lastHasMore && previous.stalled);
  let state: GraphPagingState = {
    ...(restarted ? INITIAL_GRAPH_PAGING_STATE : previous),
    lastCount: input.loadedCount,
    lastHasMore: input.hasMore,
  };
  if (input.loadingMore) {
    return { state, request: false, complete: false };
  }
  if (state.requestedAtCount !== null) {
    // The outstanding request finished; a page that added nothing would be
    // asked for again forever, so paging stops there.
    state = {
      ...state,
      stalled: input.loadedCount <= state.requestedAtCount,
      requestedAtCount: null,
    };
  }
  const complete = !input.hasMore || input.pageFailed || state.stalled;
  if (complete) {
    return { state, request: false, complete };
  }
  return {
    state: { ...state, requestedAtCount: input.loadedCount },
    request: true,
    complete: false,
  };
}
