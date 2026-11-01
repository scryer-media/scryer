import assert from "node:assert/strict";
import test from "node:test";

import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import {
  summarizeUnroutableResults,
  unroutableResultsKey,
} from "./unroutable-results.ts";

function indexer(
  name: string,
  unroutableResults: InteractiveSearchIndexerProgress["unroutableResults"],
): InteractiveSearchIndexerProgress {
  return {
    indexerId: name.toLowerCase(),
    name,
    priority: 0,
    status: "COMPLETED",
    resultCount: 0,
    elapsedMs: null,
    failureReason: null,
    rateLimited: false,
    unroutableResults,
  };
}

test("known reason codes map to their own keys", () => {
  assert.equal(
    unroutableResultsKey("RELEASE_SEARCH_NO_TORRENT_CLIENT"),
    "title.contextReleaseSearchUnroutable.noTorrentClient",
  );
  assert.equal(
    unroutableResultsKey("RELEASE_SEARCH_NO_USENET_CLIENT"),
    "title.contextReleaseSearchUnroutable.noUsenetClient",
  );
  assert.equal(
    unroutableResultsKey("RELEASE_SEARCH_NO_HTTP_CLIENT"),
    "title.contextReleaseSearchUnroutable.noHttpClient",
  );
});

test("plugin-unavailable codes map to their own keys, not the no-client ones", () => {
  assert.equal(
    unroutableResultsKey("RELEASE_SEARCH_USENET_CLIENT_PLUGIN_UNAVAILABLE"),
    "title.contextReleaseSearchUnroutable.usenetClientPluginUnavailable",
  );
  assert.equal(
    unroutableResultsKey("RELEASE_SEARCH_TORRENT_CLIENT_PLUGIN_UNAVAILABLE"),
    "title.contextReleaseSearchUnroutable.torrentClientPluginUnavailable",
  );
  assert.equal(
    unroutableResultsKey("RELEASE_SEARCH_HTTP_CLIENT_PLUGIN_UNAVAILABLE"),
    "title.contextReleaseSearchUnroutable.httpClientPluginUnavailable",
  );
});

test("an unknown reason code falls back to the generic key", () => {
  assert.equal(
    unroutableResultsKey("RELEASE_SEARCH_SOMETHING_NEW"),
    "title.contextReleaseSearchUnroutable.unknown",
  );
});

test("removed releases are totalled per reason across indexers", () => {
  const summary = summarizeUnroutableResults([
    indexer("Alpha", [
      { reason: "RELEASE_SEARCH_NO_TORRENT_CLIENT", count: 40 },
    ]),
    indexer("Beta", [
      { reason: "RELEASE_SEARCH_NO_TORRENT_CLIENT", count: 2 },
      { reason: "RELEASE_SEARCH_NO_USENET_CLIENT", count: 3 },
    ]),
    indexer("Gamma", null),
    indexer("Delta", [{ reason: "RELEASE_SEARCH_NO_HTTP_CLIENT", count: 0 }]),
  ]);

  assert.deepEqual(summary, [
    {
      reason: "RELEASE_SEARCH_NO_USENET_CLIENT",
      count: 3,
      indexerNames: ["Beta"],
    },
    {
      reason: "RELEASE_SEARCH_NO_TORRENT_CLIENT",
      count: 42,
      indexerNames: ["Alpha", "Beta"],
    },
  ]);
});
