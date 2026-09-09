import assert from "node:assert/strict";
import test from "node:test";

import { parseMaintenanceFileResults } from "./maintenance-file-results.ts";

test("legacy deletion checkpoints keep their file and grace details", () => {
  const detail = parseMaintenanceFileResults(
    JSON.stringify({
      grace_deadline: "2026-09-09T12:00:00Z",
      total_size_bytes: 42,
      files: [
        { file_id: "file-a", file_path: "/library/a.mkv", completed: true, error: null },
        { file_id: "file-b", completed: false, error: "temporary failure" },
      ],
    }),
  );

  assert.deepEqual(detail, {
    graceDeadline: "2026-09-09T12:00:00Z",
    totalSizeBytes: 42,
    storageRootId: null,
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

test("storage checkpoints report their selected root and partial progress", () => {
  const detail = parseMaintenanceFileResults(
    JSON.stringify({
      storage_root_id: "root-a",
      storage_root_identity: "volume-a",
      files: [
        { file_id: "file-a", completed: true, error: null },
        { file_id: "file-b", completed: false, error: null },
        { file_id: "file-c", completed: false, error: "capacity no longer matches" },
      ],
    }),
  );

  assert.equal(detail?.storageRootId, "root-a");
  assert.equal(detail?.completedFileCount, 1);
  assert.equal(detail?.remainingFileCount, 2);
});

test("non-checkpoint action details do not render a file results table", () => {
  assert.equal(parseMaintenanceFileResults('{"deleted_title":true}'), null);
  assert.equal(parseMaintenanceFileResults("not json"), null);
});
