import assert from "node:assert/strict";
import test from "node:test";

import type { DownloadDisplayState } from "@/lib/types/download-queue";
import type { ReleaseQueueScope } from "@/lib/types/releases";
import {
  collectActiveDownloadEpisodeIds,
  type EpisodeDownloadActivityInput,
} from "./episode-download-activity.ts";

function queueItem(
  displayState: DownloadDisplayState,
  episodeId: string | null = null,
  queueScope: ReleaseQueueScope | null = null,
): EpisodeDownloadActivityInput {
  return { displayState, episodeId, queueScope };
}

test("collects active episode ids from direct, set, and collection queue scopes", () => {
  const activeEpisodeIds = collectActiveDownloadEpisodeIds(
    [
      queueItem("DOWNLOADING", "episode-direct"),
      queueItem("QUEUED", null, {
        __typename: "EpisodeScopePayload",
        episodeId: "episode-scope",
      }),
      queueItem("IMPORT_PENDING", null, {
        __typename: "EpisodeSetScopePayload",
        episodeIds: ["episode-set-1", "episode-set-2"],
      }),
      queueItem("POST_PROCESSING", null, {
        __typename: "CollectionScopePayload",
        collectionId: "collection-1",
      }),
    ],
    {
      "collection-1": [{ id: "episode-collection-1" }, { id: "episode-collection-2" }],
    },
  );

  assert.deepEqual([...activeEpisodeIds].sort(), [
    "episode-collection-1",
    "episode-collection-2",
    "episode-direct",
    "episode-scope",
    "episode-set-1",
    "episode-set-2",
  ]);
});

test("inert items do not mask active work for the same episode", () => {
  const activeEpisodeIds = collectActiveDownloadEpisodeIds(
    [
      queueItem("PAUSED", "episode-1"),
      queueItem("FAILED", "episode-2"),
      queueItem("IMPORTING", "episode-1"),
    ],
    {},
  );

  assert.deepEqual([...activeEpisodeIds], ["episode-1"]);
});
