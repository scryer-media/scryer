// The graph draws the whole filtered catalog, so while it is shown the catalog
// keeps paging until every title is loaded. A listing is one query: its key
// changes when a filter, the sort or the libraries change, and paging state
// never carries from one listing to the next.

export type GraphPagingState = {
  /** The listing this state belongs to. */
  listingKey: string;
  /** How many titles were loaded when the outstanding request was made. */
  requestedAtCount: number | null;
  /** A page came back without adding a title, so paging stopped. */
  stalled: boolean;
};

export type GraphPagingInput = {
  active: boolean;
  listingKey: string;
  hasMore: boolean;
  loadingMore: boolean;
  pageFailed: boolean;
  loadedCount: number;
};

export type GraphPagingStep = {
  state: GraphPagingState;
  /** Ask for the next page now. */
  request: boolean;
};

export const INITIAL_GRAPH_PAGING_STATE: GraphPagingState = {
  listingKey: "",
  requestedAtCount: null,
  stalled: false,
};

export function graphPagingStep(
  previous: GraphPagingState,
  input: GraphPagingInput,
): GraphPagingStep {
  if (!input.active) {
    return { state: INITIAL_GRAPH_PAGING_STATE, request: false };
  }
  let state: GraphPagingState =
    previous.listingKey === input.listingKey
      ? previous
      : { ...INITIAL_GRAPH_PAGING_STATE, listingKey: input.listingKey };
  if (input.loadingMore || !input.listingKey) {
    return { state, request: false };
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
  if (!input.hasMore || input.pageFailed || state.stalled) {
    return { state, request: false };
  }
  return {
    state: { ...state, requestedAtCount: input.loadedCount },
    request: true,
  };
}

export type GraphTitlesCompleteInput = {
  listingKey: string;
  /** The first page of the listing is still loading. */
  titleLoading: boolean;
  hasMore: boolean;
  pageFailed: boolean;
  /** The listing whose paging stalled, if any. */
  stalledListingKey: string | null;
};

/**
 * Whether the loaded titles are the whole listing, worked out from the
 * catalog's own state on each render: every page is in, or paging ended on a
 * failure or a stall in this same listing.
 */
export function graphTitlesComplete(input: GraphTitlesCompleteInput): boolean {
  if (!input.listingKey || input.titleLoading) {
    return false;
  }
  return (
    !input.hasMore ||
    input.pageFailed ||
    input.stalledListingKey === input.listingKey
  );
}
