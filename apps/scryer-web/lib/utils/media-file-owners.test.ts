import assert from "node:assert/strict";
import test from "node:test";
import {
  deletedMediaFileIds,
  dropDeletedMediaFiles,
  mediaFileOwnerKeys,
} from "./media-file-owners.ts";

const byEpisode = {
  "episode-1": [{ id: "file-1" }, { id: "file-2" }],
  "episode-2": [{ id: "file-3" }],
};
const byMovieLink = { "link-1": [{ id: "file-4" }] };

test("a file's owning rows are found across every supplied map", () => {
  assert.deepEqual(
    [...mediaFileOwnerKeys("file-2", [byEpisode, byMovieLink])],
    ["episode-1"],
  );
  assert.deepEqual(
    [...mediaFileOwnerKeys("file-4", [byEpisode, byMovieLink])],
    ["link-1"],
  );
});

test("an unknown file owns nothing, so a deletion targets no rows", () => {
  assert.equal(mediaFileOwnerKeys("file-9", [byEpisode, byMovieLink]).size, 0);
  assert.equal(mediaFileOwnerKeys("file-1", []).size, 0);
});

test("a file cached under several rows marks all of them", () => {
  const shared = {
    "episode-1": [{ id: "file-1" }],
    "episode-2": [{ id: "file-1" }],
  };
  assert.deepEqual(
    [...mediaFileOwnerKeys("file-1", [shared])],
    ["episode-1", "episode-2"],
  );
});

test("a completed single-file run removes only the file it was asked to delete", () => {
  const cached = {
    "episode-1": [
      { id: "file-1", role: "primary" },
      { id: "file-2", role: "additional" },
    ],
  };
  const run = {
    status: "COMPLETED",
    summaryJson: JSON.stringify({ fileId: "file-2", deleteFromDisk: true }),
  };
  const deleted = deletedMediaFileIds(run);
  assert.deepEqual([...(deleted ?? [])], ["file-2"]);
  assert.deepEqual(
    dropDeletedMediaFiles(cached, deleted, new Set(["episode-1"])),
    {
      "episode-1": [{ id: "file-1", role: "primary" }],
    },
  );
});

test("a failed single-file run removes nothing from the cache", () => {
  const cached = { "episode-1": [{ id: "file-1" }, { id: "file-2" }] };
  const deleted = deletedMediaFileIds({
    status: "FAILED",
    summaryJson: { fileId: "file-2", deleteFromDisk: true },
  });
  assert.equal(deleted?.size, 0);
  assert.deepEqual(
    dropDeletedMediaFiles(cached, deleted, new Set(["episode-1"])),
    cached,
  );
});

test("a batch run's reported ids are trusted whatever the run status", () => {
  const deleted = deletedMediaFileIds({
    status: "WARNING",
    summaryJson: JSON.stringify({ deletedFileIds: ["file-1", "file-3"] }),
  });
  assert.deepEqual([...(deleted ?? [])], ["file-1", "file-3"]);
});

test("an unusable summary drops every targeted row instead", () => {
  const cached = {
    "episode-1": [{ id: "file-1" }, { id: "file-2" }],
    "episode-2": [{ id: "file-3" }],
  };
  for (const summaryJson of [null, "not json", { deletedFileIds: [1] }, {}]) {
    const deleted = deletedMediaFileIds({ status: "COMPLETED", summaryJson });
    assert.equal(deleted, null);
    assert.deepEqual(
      dropDeletedMediaFiles(cached, deleted, new Set(["episode-1"])),
      {
        "episode-2": [{ id: "file-3" }],
      },
    );
  }
});
