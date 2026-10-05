import type { MediaRequestRecord } from "@/lib/types/titles";

export type MediaRequestStatusFilter = "all" | MediaRequestRecord["status"];

// The requests page loads every status at once and filters here, so each tab's
// count comes from the same list the other tabs are drawn from. Counting a list
// the server had already narrowed to one status left every other tab at zero.

export function requestsWithStatus(
  requests: readonly MediaRequestRecord[],
  status: MediaRequestStatusFilter,
): MediaRequestRecord[] {
  if (status === "all") {
    return [...requests];
  }
  return requests.filter((request) => request.status === status);
}

export function requestCountByStatus(
  requests: readonly MediaRequestRecord[],
  status: MediaRequestStatusFilter,
): number {
  return requestsWithStatus(requests, status).length;
}

export function requestCountByFacet(
  requests: readonly MediaRequestRecord[],
  facet: MediaRequestRecord["facet"],
): number {
  return requests.filter((request) => request.facet === facet).length;
}

export type RequesterFilterOption = {
  userId: string;
  username: string;
};

// The requester filter offers only people who already appear on a request the
// reader can see, so it never lists accounts the reader has no other way to
// learn about. While a requester is selected the server returns only their
// rows, so the options seen before the selection are kept rather than
// collapsing to the one person picked.
export function requesterFilterOptions(
  requests: readonly MediaRequestRecord[],
  previous: readonly RequesterFilterOption[],
  keepPrevious: boolean,
): RequesterFilterOption[] {
  const byUserId = new Map<string, RequesterFilterOption>();
  if (keepPrevious) {
    for (const option of previous) {
      byUserId.set(option.userId, option);
    }
  }
  for (const request of requests) {
    for (const requester of request.requesters ?? []) {
      const userId = requester.userId?.trim();
      if (!userId) {
        continue;
      }
      byUserId.set(userId, {
        userId,
        username: requester.username?.trim() || userId,
      });
    }
  }
  return Array.from(byUserId.values()).sort(
    (left, right) =>
      left.username.localeCompare(right.username) ||
      left.userId.localeCompare(right.userId),
  );
}
