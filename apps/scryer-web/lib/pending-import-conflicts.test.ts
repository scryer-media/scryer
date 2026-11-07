import assert from "node:assert/strict";
import test from "node:test";
import {
  canApplyConflictPreview,
  isResolvableConflict,
} from "./pending-import-conflicts.ts";

test("apply requires an actionable preview and an idle dialog", () => {
  assert.equal(canApplyConflictPreview(null, false, false), false);
  assert.equal(
    canApplyConflictPreview({ actionableCount: 0 }, false, false),
    false,
  );
  assert.equal(
    canApplyConflictPreview({ actionableCount: 1 }, false, false),
    true,
  );
  assert.equal(
    canApplyConflictPreview({ actionableCount: 1 }, true, false),
    false,
  );
  assert.equal(
    canApplyConflictPreview({ actionableCount: 1 }, false, true),
    false,
  );
});

test("apply uses complete counts even when every sampled row failed", () => {
  const preview = {
    actionableCount: 1,
    failedCount: 100,
    totalCount: 101,
    sampled: true,
    rows: Array.from({ length: 100 }, () => ({
      errorReason: "LOCATION_CONFLICT_PATH_MISSING",
    })),
  };
  assert.equal(canApplyConflictPreview(preview, false, false), true);
  assert.equal(
    canApplyConflictPreview({ ...preview, actionableCount: 0 }, false, false),
    false,
  );
});

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
