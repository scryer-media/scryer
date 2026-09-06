import assert from "node:assert/strict";
import test from "node:test";

import type { ReleaseQueueScope } from "@/lib/types/releases";
import {
  hasPrimaryMediaFile,
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
