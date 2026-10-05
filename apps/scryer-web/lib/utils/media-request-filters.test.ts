import assert from "node:assert/strict";
import test from "node:test";

import type { MediaRequestRecord } from "../types/titles.ts";
import {
  requestCountByFacet,
  requestCountByStatus,
  requesterFilterOptions,
  requestsWithStatus,
} from "./media-request-filters.ts";

function request(
  id: string,
  status: MediaRequestRecord["status"],
  facet: MediaRequestRecord["facet"],
): MediaRequestRecord {
  return { id, status, facet } as MediaRequestRecord;
}

const loaded = [
  request("request-1", "PENDING", "MOVIE"),
  request("request-2", "APPROVED", "MOVIE"),
  request("request-3", "APPROVED", "SERIES"),
  request("request-4", "REJECTED", "ANIME"),
];

test("every tab counts from the full list, not only the selected tab", () => {
  assert.equal(requestCountByStatus(loaded, "PENDING"), 1);
  assert.equal(requestCountByStatus(loaded, "APPROVED"), 2);
  assert.equal(requestCountByStatus(loaded, "CANCELED"), 0);
  assert.equal(requestCountByStatus(loaded, "all"), 4);
});

test("the selected tab shows only its status, and facets count within it", () => {
  const approved = requestsWithStatus(loaded, "APPROVED");
  assert.deepEqual(
    approved.map((entry) => entry.id),
    ["request-2", "request-3"],
  );
  assert.equal(requestCountByFacet(approved, "MOVIE"), 1);
  assert.equal(requestCountByFacet(approved, "ANIME"), 0);
  assert.equal(requestsWithStatus(loaded, "all").length, 4);
});

function requestedBy(
  id: string,
  requesters: Array<[string, string]>,
): MediaRequestRecord {
  return {
    id,
    status: "PENDING",
    facet: "MOVIE",
    requesters: requesters.map(([userId, username]) => ({
      userId,
      username,
      requestedAt: "2026-01-01T00:00:00Z",
    })),
  } as MediaRequestRecord;
}

test("requester options come only from requesters on visible requests", () => {
  const options = requesterFilterOptions(
    [
      requestedBy("request-a", [["user-b", "bravo"], ["user-a", "alpha"]]),
      requestedBy("request-b", [["user-a", "alpha"], ["user-c", ""]]),
    ],
    [{ userId: "user-z", username: "zulu" }],
    false,
  );
  assert.deepEqual(options, [
    { userId: "user-a", username: "alpha" },
    { userId: "user-b", username: "bravo" },
    { userId: "user-c", username: "user-c" },
  ]);
});

test("a selected requester keeps the options seen before the selection", () => {
  const before = requesterFilterOptions(
    [requestedBy("request-a", [["user-a", "alpha"]]), requestedBy("request-b", [["user-b", "bravo"]])],
    [],
    false,
  );
  const whileFiltered = requesterFilterOptions(
    [requestedBy("request-b", [["user-b", "bravo"]])],
    before,
    true,
  );
  assert.deepEqual(
    whileFiltered.map((option) => option.userId),
    ["user-a", "user-b"],
  );
});
