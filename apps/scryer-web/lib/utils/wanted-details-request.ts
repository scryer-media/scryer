type DetailsRequest = { itemId: string | null };

/** Keep responses scoped to the selected Wanted row and its latest request. */
export function createWantedDetailsRequests() {
  let current: DetailsRequest = { itemId: null };
  return {
    select(itemId: string | null) {
      current = { itemId };
      return current;
    },
    refresh(itemId: string) {
      if (current.itemId !== itemId) return null;
      current = { itemId };
      return current;
    },
    isCurrent(request: DetailsRequest) {
      return current === request;
    },
  };
}
