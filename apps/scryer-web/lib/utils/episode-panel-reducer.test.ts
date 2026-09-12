import assert from "node:assert/strict";
import test from "node:test";

import {
  episodePanelReducer,
  initialEpisodePanelState,
} from "../../components/views/series-overview/episode-panel-reducer.ts";
import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";

function progress(
  name: string,
  status: InteractiveSearchIndexerProgress["status"],
): InteractiveSearchIndexerProgress {
  return {
    indexerId: name.toLowerCase(),
    name,
    priority: 0,
    status,
    resultCount: 1,
    elapsedMs: null,
    failureReason: null,
  };
}

function release(id: string): Release {
  return { id } as unknown as Release;
}

test("episode search snapshots update results and progress atomically", () => {
  const state = episodePanelReducer(initialEpisodePanelState, {
    type: "SET_SEARCH_SNAPSHOT",
    episodeId: "episode-1",
    results: [release("release-1")],
    indexers: [progress("Indexer", "COMPLETED")],
  });

  assert.deepEqual(state.searchResultsByEpisode["episode-1"], [release("release-1")]);
  assert.deepEqual(state.searchIndexerProgressByEpisode["episode-1"], [
    progress("Indexer", "COMPLETED"),
  ]);
});

test("episode search snapshots remain isolated by episode id", () => {
  const first = episodePanelReducer(initialEpisodePanelState, {
    type: "SET_SEARCH_SNAPSHOT",
    episodeId: "episode-1",
    results: [release("release-1")],
    indexers: [progress("First", "COMPLETED")],
  });
  const second = episodePanelReducer(first, {
    type: "SET_SEARCH_SNAPSHOT",
    episodeId: "episode-2",
    results: [release("release-2")],
    indexers: [progress("Second", "SEARCHING")],
  });

  assert.deepEqual(second.searchResultsByEpisode["episode-1"], [release("release-1")]);
  assert.deepEqual(second.searchIndexerProgressByEpisode["episode-1"], [
    progress("First", "COMPLETED"),
  ]);
  assert.deepEqual(second.searchResultsByEpisode["episode-2"], [release("release-2")]);
  assert.deepEqual(second.searchIndexerProgressByEpisode["episode-2"], [
    progress("Second", "SEARCHING"),
  ]);
});

test("replacement searches clear stale episode results and progress", () => {
  const populated = episodePanelReducer(initialEpisodePanelState, {
    type: "SET_SEARCH_SNAPSHOT",
    episodeId: "episode-1",
    results: [release("stale")],
    indexers: [progress("Old", "FAILED")],
  });
  const reset = episodePanelReducer(populated, {
    type: "RESET_SEARCH",
    episodeId: "episode-1",
  });

  assert.equal(reset.searchResultsByEpisode["episode-1"], undefined);
  assert.equal(reset.searchIndexerProgressByEpisode["episode-1"], undefined);
});
