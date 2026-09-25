// `null` selects every client. It stays `null` until the user picks clients,
// so the queue query does not change when the list of known clients arrives.
// The known list is briefly empty while a queue page for a new filter loads;
// an explicit selection is only pruned against a list that has loaded.
export function reconcileActivityClientSelection(
  current: string[] | null,
  availableClientIds: string[],
): string[] | null {
  if (current === null || availableClientIds.length === 0) {
    return current;
  }
  const next = current.filter((clientId) => availableClientIds.includes(clientId));
  return next.length === current.length ? current : next;
}
