import assert from "node:assert/strict";
import test from "node:test";

import type { ReleaseQueueScope } from "@/lib/types/releases";
import {
  hasPrimaryMediaFile,
  queueScopeReplacesPrimary,
  releaseCoversMultipleEpisodes,
  releaseSupportsAdditionalFileQueue,
} from "./release-queue-scope.ts";

test("additional-file queue eligibility uses the signed release queue scope", () => {
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { __typename: "TitleScopePayload", wholeTitle: true } },
      "movie",
    ),
    true,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { __typename: "TitleScopePayload", wholeTitle: true } },
      "series",
    ),
    false,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { __typename: "TitleScopePayload", wholeTitle: true } },
      "anime",
    ),
    false,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { __typename: "EpisodeScopePayload", episodeId: "episode-1" } },
      "series",
    ),
    true,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      {
        queueScope: {
          __typename: "SeriesMovieScopePayload",
          seriesMovieLinkId: "series-movie-1",
        },
      },
      "anime",
    ),
    true,
  );

  const unsupportedScopes: ReleaseQueueScope[] = [
    { __typename: "CollectionScopePayload", collectionId: "season-1" },
    { __typename: "EpisodeSetScopePayload", episodeIds: ["episode-1", "episode-2"] },
    { __typename: "OrphanScopePayload", orphaned: true },
  ];
  for (const queueScope of unsupportedScopes) {
    assert.equal(releaseSupportsAdditionalFileQueue({ queueScope }, "movie"), false);
  }

  assert.equal(releaseSupportsAdditionalFileQueue({ queueScope: null }, "movie"), false);
});

test("manual replacement selection requires an existing primary file", () => {
  assert.equal(hasPrimaryMediaFile(undefined), false);
  assert.equal(hasPrimaryMediaFile([]), false);
  assert.equal(hasPrimaryMediaFile([{ role: "additional" }]), false);
  assert.equal(hasPrimaryMediaFile([{ role: "PRIMARY" }]), true);
  assert.equal(hasPrimaryMediaFile([{ role: null }, { role: "primary" }]), true);
});

test("the pack badge follows the signed scope, not the release name", () => {
  const packScopes: ReleaseQueueScope[] = [
    { __typename: "CollectionScopePayload", collectionId: "season-1" },
    { __typename: "EpisodeSetScopePayload", episodeIds: ["episode-1", "episode-2"] },
  ];
  for (const queueScope of packScopes) {
    assert.equal(releaseCoversMultipleEpisodes({ queueScope }), true);
  }

  // A one-member set covers exactly the wanted episode; calling that a pack
  // would put the badge on ordinary single-episode grabs.
  const singleEpisodeScopes: ReleaseQueueScope[] = [
    { __typename: "EpisodeScopePayload", episodeId: "episode-1" },
    { __typename: "EpisodeSetScopePayload", episodeIds: ["episode-1"] },
    { __typename: "EpisodeSetScopePayload", episodeIds: [] },
    { __typename: "CollectionScopePayload", collectionId: "" },
    { __typename: "SeriesMovieScopePayload", seriesMovieLinkId: "link-1" },
    { __typename: "TitleScopePayload", wholeTitle: true },
    { __typename: "OrphanScopePayload", orphaned: true },
  ];
  for (const queueScope of singleEpisodeScopes) {
    assert.equal(releaseCoversMultipleEpisodes({ queueScope }), false);
  }

  assert.equal(releaseCoversMultipleEpisodes({ queueScope: null }), false);
  assert.equal(releaseCoversMultipleEpisodes({}), false);
});

test("a season grab replaces only primary files inside its queue scope", () => {
  const episodesByCollection = {
    "season-1": [{ id: "s1e1" }, { id: "s1e2" }],
    "season-2": [{ id: "s2e1" }, { id: "s2e2" }],
  };
  const mediaFilesByEpisode = {
    s1e1: [{ role: "primary" }],
    s2e1: [{ role: "additional" }],
  };
  const replaces = (scope: Parameters<typeof queueScopeReplacesPrimary>[0]) =>
    queueScopeReplacesPrimary(scope, episodesByCollection, mediaFilesByEpisode);

  // The scope covers an episode that has a primary file: replace.
  assert.equal(replaces({ collection: "season-1" }), true);
  assert.equal(replaces({ episode: "s1e1" }), true);
  assert.equal(replaces({ episodeSet: ["s1e2", "s1e1"] }), true);
  assert.equal(replaces({ title: true }), true);

  // The scope covers no episode at all: plain queue.
  assert.equal(replaces({ collection: "season-3" }), false);
  assert.equal(replaces({ episodeSet: [] }), false);

  // The season has a primary file, but outside the release's scope: plain queue.
  assert.equal(replaces({ episode: "s1e2" }), false);
  assert.equal(replaces({ episodeSet: ["s1e2"] }), false);
  assert.equal(replaces({ collection: "season-2" }), false);
});
