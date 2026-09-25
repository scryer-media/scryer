import assert from "node:assert/strict";
import test from "node:test";

import { titleReleaseSearchInput } from "./release-search-input.ts";

test("a season search names the season and no episode", () => {
  const input = titleReleaseSearchInput("title-1", { kind: "season", season: "2" });
  assert.deepEqual(input, { titleId: "title-1", season: "2" });
  assert.equal("episode" in input, false);
});

test("an episode search names both the season and the episode", () => {
  assert.deepEqual(
    titleReleaseSearchInput("title-1", { kind: "episode", season: "2", episode: "5" }),
    { titleId: "title-1", season: "2", episode: "5" },
  );
});

test("a whole-title search names neither", () => {
  assert.deepEqual(titleReleaseSearchInput("title-1", { kind: "title" }), {
    titleId: "title-1",
  });
});
