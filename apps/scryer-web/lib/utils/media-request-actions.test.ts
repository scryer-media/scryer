import assert from "node:assert/strict";
import test from "node:test";

import { mediaRequestRowActions } from "./media-request-actions.ts";

test("a manager can reopen a dismissed request and nothing else", () => {
  assert.deepEqual(mediaRequestRowActions("admin", "REJECTED"), {
    resolve: false,
    reopen: true,
    editOwn: false,
  });
  for (const status of ["PENDING", "APPROVED", "CANCELED"] as const) {
    assert.equal(mediaRequestRowActions("admin", status).reopen, false, status);
  }
});

test("a requester never sees reopen, even on their own dismissed request", () => {
  for (const status of ["PENDING", "APPROVED", "REJECTED", "CANCELED"] as const) {
    assert.equal(mediaRequestRowActions("mine", status).reopen, false, status);
    assert.equal(mediaRequestRowActions("mine", status).resolve, false, status);
  }
});

test("pending requests keep their existing resolve and edit actions", () => {
  assert.equal(mediaRequestRowActions("admin", "PENDING").resolve, true);
  assert.equal(mediaRequestRowActions("mine", "PENDING").editOwn, true);
  assert.equal(mediaRequestRowActions("admin", "PENDING").editOwn, false);
});
