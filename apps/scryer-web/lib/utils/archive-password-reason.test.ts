import test from "node:test";
import assert from "node:assert/strict";
import { isArchivePasswordFailure } from "./archive-password-reason.ts";

test("typed archive conditions survive sentence changes", () => {
  assert.equal(
    isArchivePasswordFailure(
      "ARCHIVE_PASSWORD_REQUIRED",
      "arbitrary new words",
    ),
    true,
  );
  assert.equal(
    isArchivePasswordFailure("ARCHIVE_PASSWORD_OR_CORRUPTION"),
    true,
  );
  assert.equal(
    isArchivePasswordFailure("OTHER", "ARCHIVE_PASSWORD_REQUIRED: old words"),
    false,
  );
});

test("older persisted archive rows retain their retry action", () => {
  assert.equal(
    isArchivePasswordFailure(null, "ARCHIVE_PASSWORD_REQUIRED: old words"),
    true,
  );
  assert.equal(
    isArchivePasswordFailure(undefined, "archive_password_or_corruption"),
    true,
  );
  assert.equal(isArchivePasswordFailure(null, "an unrelated failure"), false);
});
