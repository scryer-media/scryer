import assert from "node:assert/strict";
import test from "node:test";

import {
  confirmedContentBlockRemoval,
  mediaIssueRecycleApprovalVariables,
} from "./media-issue-review.ts";

const issue = {
  id: "report-1",
  status: "PENDING",
  remediation: "RECYCLE_AND_SEARCH",
} as const;
const preview = { fingerprint: "reviewed-fingerprint", mediaCount: 1 };

test("recycle submits the preview fingerprint and explicit typed confirmation", () => {
  assert.deepEqual(
    mediaIssueRecycleApprovalVariables(issue, preview, " DELETE ", true),
    {
      issueId: issue.id,
      previewFingerprint: preview.fingerprint,
      typedConfirmation: "DELETE",
    },
  );
  assert.equal(
    mediaIssueRecycleApprovalVariables(issue, preview, "", true),
    null,
  );
  assert.equal(
    mediaIssueRecycleApprovalVariables(issue, null, "DELETE", true),
    null,
  );
  assert.equal(
    mediaIssueRecycleApprovalVariables(
      issue,
      { ...preview, fingerprint: "" },
      "DELETE",
      true,
    ),
    null,
  );
  assert.equal(
    mediaIssueRecycleApprovalVariables(
      issue,
      { ...preview, mediaCount: 2 },
      "DELETE",
      true,
    ),
    null,
  );
});

test("recycle cannot be escalated from mark-only or used when the bin is unavailable", () => {
  assert.equal(
    mediaIssueRecycleApprovalVariables(
      { ...issue, remediation: "MARK_BAD" },
      preview,
      "DELETE",
      true,
    ),
    null,
  );
  assert.equal(
    mediaIssueRecycleApprovalVariables(
      { ...issue, status: "APPROVED" },
      preview,
      "DELETE",
      true,
    ),
    null,
  );
  assert.equal(
    mediaIssueRecycleApprovalVariables(issue, preview, "DELETE", false),
    null,
  );
});

test("a partial, missing, or mismatched clear result does not remove a block", () => {
  const data = { clearLibraryContentBlock: { id: "block-1" } };
  assert.equal(confirmedContentBlockRemoval("block-1", { data }), true);
  assert.equal(
    confirmedContentBlockRemoval("block-1", {
      data,
      error: new Error("failed"),
    }),
    false,
  );
  assert.equal(confirmedContentBlockRemoval("block-1", { data: null }), false);
  assert.equal(confirmedContentBlockRemoval("block-2", { data }), false);
});
