import assert from "node:assert/strict";
import test from "node:test";
import { isResolvableConflict } from "./pending-import-conflicts.ts";

test("folder resolution offers only supported pending movie conflicts", () => {
  const conflict = {
    facet: "MOVIE",
    status: "PENDING",
    reason: "title_already_owns_another_folder",
  };
  assert.equal(isResolvableConflict(conflict), true);
  for (const facet of ["SERIES", "ANIME"])
    assert.equal(isResolvableConflict({ ...conflict, facet }), false);
  assert.equal(isResolvableConflict({ ...conflict, status: "IGNORED" }), false);
  assert.equal(
    isResolvableConflict({
      ...conflict,
      reason: "folder_ownership_changed_by_user",
    }),
    false,
  );
  assert.equal(
    isResolvableConflict({
      ...conflict,
      reason: "invalid_title_folder_ownership",
    }),
    false,
  );
});
