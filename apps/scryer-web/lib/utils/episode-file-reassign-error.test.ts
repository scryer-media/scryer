import assert from "node:assert/strict";
import test from "node:test";
import en from "../i18n/locales/en.ts";
import {
  EPISODE_FILE_REASSIGN_REFUSAL_KEYS,
  episodeFileReassignConflictingFileId,
  episodeFileReassignErrorMessage,
} from "./episode-file-reassign-error.ts";

const t = (key: string) => en[key] ?? key;
const refusal = (reason: string, extra: Record<string, string> = {}) => ({
  graphQLErrors: [
    {
      message: "unrelated wording",
      extensions: {
        code: "VALIDATION_ERROR",
        reason,
        ...extra,
      },
    },
  ],
});

test("every server refusal reason has a translation", () => {
  // Mirrors crates/scryer-application/src/episode_file_reassign_refusal.rs ALL.
  const serverReasons = [
    "EPISODE_FILE_REASSIGN_NO_EPISODES",
    "EPISODE_FILE_REASSIGN_EPISODE_OUTSIDE_TITLE",
    "EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT",
    "EPISODE_FILE_REASSIGN_NOT_EPISODIC_TITLE",
    "EPISODE_FILE_REASSIGN_UNSUPPORTED_FILE",
    "EPISODE_FILE_REASSIGN_DESTINATION_OCCUPIED",
    "EPISODE_FILE_REASSIGN_RENAME_REFUSED",
    "EPISODE_FILE_REASSIGN_RENAME_FAILED",
  ];
  assert.deepEqual(
    Object.keys(EPISODE_FILE_REASSIGN_REFUSAL_KEYS).sort(),
    [...serverReasons].sort(),
  );
  for (const key of Object.values(EPISODE_FILE_REASSIGN_REFUSAL_KEYS)) {
    assert.ok(en[key], `missing en translation for ${key}`);
  }
});

test("a reassignment refusal is translated by reason regardless of the sentence", () => {
  assert.equal(
    episodeFileReassignErrorMessage(
      refusal("EPISODE_FILE_REASSIGN_DESTINATION_OCCUPIED"),
      t,
      "failed",
    ),
    en["episodeAssign.refusal.destinationOccupied"],
  );
});

test("an unknown reason keeps the server explanation", () => {
  assert.equal(
    episodeFileReassignErrorMessage(
      {
        graphQLErrors: [
          {
            message: "validation: new condition",
            extensions: {
              code: "VALIDATION_ERROR",
              reason: "EPISODE_FILE_REASSIGN_FUTURE",
            },
          },
        ],
      },
      t,
      "failed",
    ),
    "new condition",
  );
});

test("the conflicting file id is read from its own field, only for a primary conflict", () => {
  assert.equal(
    episodeFileReassignConflictingFileId(
      refusal("EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT", {
        conflictingFileId: "file-7",
      }),
    ),
    "file-7",
  );
  assert.equal(
    episodeFileReassignConflictingFileId(
      refusal("EPISODE_FILE_REASSIGN_RENAME_FAILED", {
        conflictingFileId: "file-7",
      }),
    ),
    null,
  );
});
