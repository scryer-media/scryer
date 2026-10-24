import assert from "node:assert/strict";
import test from "node:test";

import {
  UNASSIGNED_TARGET,
  buildEpisodeTargetRows,
  episodeSelectionTargetValue,
  episodeTargetIds,
  episodeTargetLabel,
  episodeTargetValue,
  groupEpisodesBySeason,
  sameEpisodeSelection,
  seasonGroupNumber,
} from "./episode-target.ts";

const t = (key: string) => (key === "episode.titleTba" ? "TBA" : key);
const episode = (id: string, season: string, number: string, title: string | null = null) => ({
  id,
  seasonNumber: season,
  episodeNumber: number,
  absoluteNumber: null,
  title,
});
const labels = {
  unassigned: "Skip",
  seriesMovies: "Movies",
  seasonGroup: (key: string) => `Season ${seasonGroupNumber(key)}`,
};

test("an episode target value round-trips several episodes", () => {
  const value = episodeTargetValue(["ep-1", "ep-2"]);
  assert.deepEqual(episodeTargetIds(value), ["ep-1", "ep-2"]);
  assert.deepEqual(episodeTargetIds(UNASSIGNED_TARGET), []);
  assert.equal(episodeSelectionTargetValue([]), UNASSIGNED_TARGET);
  assert.equal(episodeSelectionTargetValue(["ep-1"]), episodeTargetValue(["ep-1"]));
});

test("episodes group by season in numeric order", () => {
  const groups = groupEpisodesBySeason([
    episode("s10e1", "10", "1"),
    episode("s2e2", "2", "2"),
    episode("s2e1", "2", "1"),
  ]);
  assert.deepEqual(Array.from(groups.keys()), ["Season 02", "Season 10"]);
  assert.deepEqual(groups.get("Season 02")?.map((ep) => ep.id), ["s2e1", "s2e2"]);
});

test("rows filter by label and offer the unassigned option only when labelled", () => {
  const groups = groupEpisodesBySeason([
    episode("ep-1", "1", "1", "Harbor Lights"),
    episode("ep-2", "1", "2", null),
  ]);
  const all = buildEpisodeTargetRows(groups, [], "", t, labels);
  assert.deepEqual(all.map((row) => row.label), ["Skip", "Season 1", "S01E01 - Harbor Lights", "S01E02 - TBA"]);

  const withoutSkip = buildEpisodeTargetRows(groups, [], "", t, { ...labels, unassigned: undefined });
  assert.equal(withoutSkip.some((row) => row.key === UNASSIGNED_TARGET), false);

  const filtered = buildEpisodeTargetRows(groups, [], "harbor", t, labels);
  assert.deepEqual(filtered.map((row) => row.label), ["Season 1", "S01E01 - Harbor Lights"]);
  // An untitled episode does not match the translated placeholder.
  assert.deepEqual(buildEpisodeTargetRows(groups, [], "tba", t, labels), []);
});

test("labels carry the absolute number when there is one", () => {
  assert.equal(
    episodeTargetLabel({ ...episode("ep", "1", "3", "Tide"), absoluteNumber: "27" }, t),
    "S01E03 (27) Tide",
  );
});

test("selections compare as sets", () => {
  assert.equal(sameEpisodeSelection(["a", "b"], ["b", "a"]), true);
  assert.equal(sameEpisodeSelection(["a"], ["a", "b"]), false);
});
