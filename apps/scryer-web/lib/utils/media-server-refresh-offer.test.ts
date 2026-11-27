import assert from "node:assert/strict";
import test from "node:test";

import {
  MEDIA_SERVER_REFRESH_OFFER_EVENTS,
  mediaServerRefreshOffer,
  subscribeMediaServerRefreshes,
} from "./media-server-refresh-offer.ts";

const ADVERTISED_BY_PATH_REFRESH_PLUGIN = [
  "import_complete",
  "upgrade",
  "rename",
  "file_deleted",
  "file_deleted_for_upgrade",
  "title_moved",
];

test("the offer is the four refresh events the installed plugin advertises", () => {
  assert.deepEqual(
    mediaServerRefreshOffer("JELLYFIN", [
      {
        providerType: "jellyfin",
        supportedEvents: ADVERTISED_BY_PATH_REFRESH_PLUGIN,
      },
    ]),
    {
      kind: "available",
      eventTypes: ["import_complete", "rename", "file_deleted", "title_moved"],
    },
  );
});

test("the offer never adds grab, title or upgrade events a plugin advertises", () => {
  const offer = mediaServerRefreshOffer("PLEX", [
    {
      providerType: "plex",
      supportedEvents: [
        "download",
        "upgrade",
        "import_complete",
        "rename",
        "file_deleted",
        "file_deleted_for_upgrade",
        "title_added",
        "title_deleted",
        "title_moved",
      ],
    },
  ]);
  assert.equal(offer.kind, "available");
  assert.deepEqual(offer.kind === "available" ? offer.eventTypes : [], [
    ...MEDIA_SERVER_REFRESH_OFFER_EVENTS,
  ]);
});

test("the offer keeps only the events the plugin handles", () => {
  assert.deepEqual(
    mediaServerRefreshOffer("EMBY", [
      {
        providerType: "emby",
        supportedEvents: ["import_complete", "title_moved"],
      },
    ]),
    { kind: "available", eventTypes: ["import_complete", "title_moved"] },
  );
});

test("a provider without an installed notification plugin reports the plugin missing", () => {
  assert.deepEqual(
    mediaServerRefreshOffer("JELLYFIN", [
      { providerType: "plex", supportedEvents: ["import_complete"] },
    ]),
    { kind: "plugin_missing" },
  );
  assert.deepEqual(mediaServerRefreshOffer("EMBY", []), {
    kind: "plugin_missing",
  });
});

test("a plugin that handles none of the offered events gets no offer", () => {
  assert.deepEqual(
    mediaServerRefreshOffer("JELLYFIN", [
      { providerType: "jellyfin", supportedEvents: ["upgrade"] },
    ]),
    { kind: "none" },
  );
});

test("each offered event is subscribed for the new connection and failures are reported", async () => {
  const created: string[] = [];
  const rejection = new Error("synthetic refusal");
  const failures = await subscribeMediaServerRefreshes(
    "connection-synthetic-1",
    ["import_complete", "rename", "file_deleted", "title_moved"],
    async (input) => {
      assert.equal(input.targetKind, "media_server_connection");
      assert.equal(input.targetId, "connection-synthetic-1");
      assert.equal(input.scope, "global");
      assert.equal(input.isEnabled, true);
      if (input.eventType === "rename") {
        throw rejection;
      }
      created.push(input.eventType);
    },
  );
  assert.deepEqual(created, ["import_complete", "file_deleted", "title_moved"]);
  assert.deepEqual(failures, [{ eventType: "rename", error: rejection }]);
});
