import assert from "node:assert/strict";
import test from "node:test";

import { episodeIdsCoveredByEpisodeFileDelete } from "./delete-preview.ts";

test("a shared multi-episode file counts every episode it covers", () => {
  const covered = episodeIdsCoveredByEpisodeFileDelete([
    {
      fileId: "file-shared",
      episodeId: "episode-4",
      episodeIds: ["episode-4", "episode-5"],
      error: null,
    },
  ]);
  assert.deepEqual([...covered].sort(), ["episode-4", "episode-5"]);
});

test("episodes covered by several files are counted once", () => {
  const covered = episodeIdsCoveredByEpisodeFileDelete([
    {
      fileId: "file-a",
      episodeId: "episode-1",
      episodeIds: ["episode-1"],
      error: null,
    },
    {
      fileId: "file-b",
      episodeId: "episode-1",
      episodeIds: ["episode-1", "episode-2"],
      error: null,
    },
  ]);
  assert.equal(covered.size, 2);
});
