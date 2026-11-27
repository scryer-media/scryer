import assert from "node:assert/strict";
import test from "node:test";

import {
  MEDIA_ISSUE_HARD_LINKED_COPY_REASON,
  mediaIssueReasonKey,
  mediaIssueRecycleApprovalVariables,
  mediaIssueRefusalKey,
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
    )?.previewFingerprint,
    preview.fingerprint,
  );
  for (const mediaCount of [0, 3]) {
    assert.equal(
      mediaIssueRecycleApprovalVariables(
        issue,
        { ...preview, mediaCount },
        "DELETE",
        true,
      ),
      null,
    );
  }
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

test("refusal codes map to messages and unknown codes fall back", () => {
  assert.equal(
    mediaIssueRefusalKey("MEDIA_RECYCLE_HARDLINKED_FILE"),
    "error.reason.MEDIA_RECYCLE_HARDLINKED_FILE",
  );
  assert.equal(mediaIssueRefusalKey("MEDIA_SOMETHING_NEW"), undefined);
  assert.equal(mediaIssueRefusalKey(null), undefined);
});

test("an unknown report reason reads as other", () => {
  assert.equal(
    mediaIssueReasonKey("wrong_language"),
    "mediaIssue.reason.wrongLanguage",
  );
  assert.equal(mediaIssueReasonKey("bad_media"), "mediaIssue.reason.other");
});

test("a hard-linked download copy is recycled only as an explicit pair", () => {
  const pair = { ...preview, mediaCount: 2 };
  assert.deepEqual(
    mediaIssueRecycleApprovalVariables(
      issue,
      pair,
      "DELETE",
      true,
      MEDIA_ISSUE_HARD_LINKED_COPY_REASON,
    ),
    {
      issueId: issue.id,
      previewFingerprint: pair.fingerprint,
      typedConfirmation: "DELETE",
      recycleBothNames: true,
    },
  );
  assert.equal(
    "recycleBothNames" in
      (mediaIssueRecycleApprovalVariables(issue, pair, "DELETE", true) ?? {}),
    false,
  );
  // The pair is the reported file and its one other name.
  assert.equal(
    mediaIssueRecycleApprovalVariables(
      issue,
      preview,
      "DELETE",
      true,
      MEDIA_ISSUE_HARD_LINKED_COPY_REASON,
    ),
    null,
  );
  // A reason this client does not know is never confirmed.
  assert.equal(
    mediaIssueRecycleApprovalVariables(
      issue,
      pair,
      "DELETE",
      true,
      "MEDIA_SOMETHING_NEW",
    ),
    null,
  );
  assert.equal(
    mediaIssueRefusalKey(MEDIA_ISSUE_HARD_LINKED_COPY_REASON),
    "error.reason.MEDIA_RECYCLE_HARDLINKED_DOWNLOAD_COPY",
  );
});
