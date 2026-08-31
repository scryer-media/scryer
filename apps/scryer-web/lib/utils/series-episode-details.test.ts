import assert from "node:assert/strict";
import test from "node:test";

import {
  episodeIdsForCollections,
  mergeLoadedEpisodeDetailsForCollections,
  pruneEpisodeRecord,
  pruneSeriesMovieLinkMediaFiles,
} from "./series-episode-details.ts";

test("series overview refresh preserves loaded episode detail fields", () => {
  const refreshedCollections = [
    {
      id: "season-1",
      episodes: [
        {
          id: "episode-1",
          title: "Episode 1 refreshed",
          overview: null,
          imageUrl: null,
        },
        {
          id: "episode-2",
          title: "Episode 2 refreshed",
          overview: null,
          imageUrl: null,
        },
      ],
    },
  ];
  const currentEpisodesByCollection = {
    "season-1": [
        {
          id: "episode-1",
          title: "Episode 1",
          overview: "Loaded overview",
          imageUrl: "https://example.test/episode-1.jpg",
          playbackLinks: [
            {
              connectionId: "jellyfin-1",
              displayName: "Jellyfin",
              provider: "JELLYFIN",
              href: "https://jellyfin.example.test/web/index.html#!/details?id=episode-1",
            },
          ],
        },
      {
        id: "episode-2",
        title: "Episode 2",
        overview: "Not loaded",
        imageUrl: "https://example.test/episode-2.jpg",
      },
    ],
  };

  const merged = mergeLoadedEpisodeDetailsForCollections(
    refreshedCollections,
    currentEpisodesByCollection,
    new Set(["episode-1"]),
  );

  assert.equal(merged["season-1"]?.[0]?.title, "Episode 1 refreshed");
  assert.equal(merged["season-1"]?.[0]?.overview, "Loaded overview");
  assert.equal(
    merged["season-1"]?.[0]?.imageUrl,
    "https://example.test/episode-1.jpg",
  );
  assert.deepEqual(
    (merged["season-1"]?.[0] as { playbackLinks?: unknown } | undefined)
      ?.playbackLinks,
    [
      {
        connectionId: "jellyfin-1",
        displayName: "Jellyfin",
        provider: "JELLYFIN",
        href: "https://jellyfin.example.test/web/index.html#!/details?id=episode-1",
      },
    ],
  );
  assert.equal(merged["season-1"]?.[1]?.overview, null);
  assert.equal(merged["season-1"]?.[1]?.imageUrl, null);
});

test("series overview refresh prunes stale loaded episode caches", () => {
  const episodeIds = episodeIdsForCollections([
    {
      id: "season-1",
      episodes: [{ id: "episode-1" }],
    },
  ]);

  assert.deepEqual(
    pruneEpisodeRecord({ "episode-1": true, removed: true }, episodeIds),
    {
      "episode-1": true,
    },
  );
  const retainedEpisodeCache = { "episode-1": true };
  assert.equal(
    pruneEpisodeRecord(retainedEpisodeCache, episodeIds),
    retainedEpisodeCache,
  );
  assert.deepEqual(
    pruneSeriesMovieLinkMediaFiles(
      {
        link: [
          { id: "file-1", episodeId: "episode-1" },
          { id: "file-2", episodeId: "removed" },
          { id: "file-3", episodeId: null },
        ],
        removedLink: [{ id: "file-4", episodeId: "removed" }],
      },
      episodeIds,
    ),
    {
      link: [
        { id: "file-1", episodeId: "episode-1" },
        { id: "file-3", episodeId: null },
      ],
    },
  );
  const retainedSeriesMovieCache = {
    link: [{ id: "file-1", episodeId: "episode-1" }],
  };
  assert.equal(
    pruneSeriesMovieLinkMediaFiles(retainedSeriesMovieCache, episodeIds),
    retainedSeriesMovieCache,
  );
});
