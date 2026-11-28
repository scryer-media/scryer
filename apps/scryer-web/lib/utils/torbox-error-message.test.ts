import assert from "node:assert/strict";
import test from "node:test";

import { userFacingGraphQlErrorMessage } from "../graphql/error-message.ts";
import { t } from "../i18n/index.ts";
import {
  TORBOX_REFUSAL_CODES,
  TORBOX_REFUSAL_KEYS,
} from "./torbox-error-message.ts";

// Mirrors `ALL` in crates/scryer-application/src/torbox_refusal.rs.
const SERVER_TORBOX_REASONS = [
  "TORBOX_ACTION_UNSUPPORTED",
  "TORBOX_ACTIVE_LIMIT",
  "TORBOX_COOLDOWN_LIMIT",
  "TORBOX_DOWNLOAD_TOO_LARGE",
  "TORBOX_DUPLICATE_ITEM",
  "TORBOX_FILE_INFECTED",
  "TORBOX_FILE_PATHS_UNUSABLE",
  "TORBOX_FILE_ZIPPED",
  "TORBOX_HOURLY_LIMIT",
  "TORBOX_INVALID_NZB",
  "TORBOX_INVALID_TORRENT",
  "TORBOX_ITEM_MISSING",
  "TORBOX_ITEM_NOT_FOUND",
  "TORBOX_JOB_NOT_FOUND",
  "TORBOX_LINK_OFFLINE",
  "TORBOX_MONTHLY_LIMIT",
  "TORBOX_NOT_CACHED",
  "TORBOX_NO_VIDEO_FILE",
  "TORBOX_PLAN_RESTRICTED",
  "TORBOX_RATE_LIMITED",
  "TORBOX_REMOTE_FAILED",
  "TORBOX_SOURCE_MISSING",
  "TORBOX_STALLED",
  "TORBOX_TOKEN_REJECTED",
  "TORBOX_TOKEN_REQUIRED",
  "TORBOX_UNAVAILABLE",
];

function coded(reason: string, code: string) {
  return {
    graphQLErrors: [
      {
        message: "This sentence was reworded",
        extensions: { code, reason },
      },
    ],
  };
}

test("every TorBox reason the server defines has a translation key and a code", () => {
  assert.deepEqual(
    Object.keys(TORBOX_REFUSAL_KEYS).sort(),
    [...SERVER_TORBOX_REASONS].sort(),
  );
  assert.deepEqual(
    Object.keys(TORBOX_REFUSAL_CODES).sort(),
    [...SERVER_TORBOX_REASONS].sort(),
  );
  for (const [reason, key] of Object.entries(TORBOX_REFUSAL_KEYS)) {
    assert.notEqual(t(key, "eng"), key, `${reason} has English text`);
  }
});

test("a coded TorBox error shows its translated sentence, not the server's", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("TORBOX_TOKEN_REJECTED", "VALIDATION_ERROR"),
      "Failed",
    ),
    t("error.reason.TORBOX_TOKEN_REJECTED", "eng"),
  );
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("TORBOX_HOURLY_LIMIT", "TEMPORARY_UNAVAILABLE"),
      "Failed",
    ),
    t("error.reason.TORBOX_HOURLY_LIMIT", "eng"),
  );
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("TORBOX_JOB_NOT_FOUND", "NOT_FOUND"),
      "Failed",
    ),
    t("error.reason.TORBOX_JOB_NOT_FOUND", "eng"),
  );
  assert.notEqual(
    userFacingGraphQlErrorMessage(
      coded("TORBOX_HOURLY_LIMIT", "VALIDATION_ERROR"),
      "Failed",
    ),
    t("error.reason.TORBOX_HOURLY_LIMIT", "eng"),
  );
});
