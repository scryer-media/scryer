import assert from "node:assert/strict";
import test from "node:test";

import { parseLifecycleFileResults } from "./lifecycle-file-results.ts";

test("legacy deletion checkpoints keep their file and grace details", () => {
  const detail = parseLifecycleFileResults(
    JSON.stringify({
      grace_deadline: "2026-09-09T12:00:00Z",
      total_size_bytes: 42,
      files: [
        {
          file_id: "file-a",
          file_path: "/library/a.mkv",
          completed: true,
          error: null,
        },
        { file_id: "file-b", completed: false, error: "temporary failure" },
      ],
    }),
  );

  assert.deepEqual(detail, {
    graceDeadline: "2026-09-09T12:00:00Z",
    totalSizeBytes: 42,
    completedFileCount: 1,
    remainingFileCount: 1,
    files: [
      {
        fileId: "file-a",
        filePath: "/library/a.mkv",
        completed: true,
        error: null,
      },
      {
        fileId: "file-b",
        filePath: null,
        completed: false,
        error: "temporary failure",
      },
    ],
  });
});

test("a checkpoint written for one root still reports its partial progress", () => {
  const detail = parseLifecycleFileResults(
    JSON.stringify({
      storage_root_id: "root-a",
      storage_root_identity: "volume-a",
      files: [
        { file_id: "file-a", completed: true, error: null },
        { file_id: "file-b", completed: false, error: null },
        {
          file_id: "file-c",
          completed: false,
          error: "capacity no longer matches",
        },
      ],
    }),
  );

  assert.equal(detail?.files.length, 3);
  assert.equal(detail?.completedFileCount, 1);
  assert.equal(detail?.remainingFileCount, 2);
});

test("sequence history unwraps its scoped deletion outcomes without another history row", () => {
  const detail = parseLifecycleFileResults(
    JSON.stringify({
      maintenance_sequence_history: true,
      deletion_outcomes: [
        {
          action_run_id: "internal-delete-run",
          status: "held",
          detail: JSON.stringify({
            storage_root_id: "root-a",
            total_size_bytes: 42,
            files: [
              { file_id: "file-a", completed: true, error: null },
              {
                file_id: "file-b",
                completed: false,
                error: "capacity changed",
              },
            ],
          }),
        },
      ],
    }),
  );

  assert.equal(detail?.files.length, 2);
  assert.equal(detail?.completedFileCount, 1);
  assert.equal(detail?.remainingFileCount, 1);
  assert.equal(detail?.totalSizeBytes, 42);
});

test("non-checkpoint action details do not render a file results table", () => {
  assert.equal(parseLifecycleFileResults('{"deleted_title":true}'), null);
  assert.equal(parseLifecycleFileResults("not json"), null);
});
