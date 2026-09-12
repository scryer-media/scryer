import assert from "node:assert/strict";
import test from "node:test";
import { mediaFileOwnerKeys } from "./media-file-owners.ts";

const byEpisode = {
  "episode-1": [{ id: "file-1" }, { id: "file-2" }],
  "episode-2": [{ id: "file-3" }],
};
const byMovieLink = { "link-1": [{ id: "file-4" }] };

test("a file's owning rows are found across every supplied map", () => {
  assert.deepEqual([...mediaFileOwnerKeys("file-2", [byEpisode, byMovieLink])], ["episode-1"]);
  assert.deepEqual([...mediaFileOwnerKeys("file-4", [byEpisode, byMovieLink])], ["link-1"]);
});

test("an unknown file owns nothing, so a deletion targets no rows", () => {
  assert.equal(mediaFileOwnerKeys("file-9", [byEpisode, byMovieLink]).size, 0);
  assert.equal(mediaFileOwnerKeys("file-1", []).size, 0);
});

test("a file cached under several rows marks all of them", () => {
  const shared = { "episode-1": [{ id: "file-1" }], "episode-2": [{ id: "file-1" }] };
  assert.deepEqual([...mediaFileOwnerKeys("file-1", [shared])], ["episode-1", "episode-2"]);
});
