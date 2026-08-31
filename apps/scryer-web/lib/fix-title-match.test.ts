import assert from "node:assert/strict";
import test from "node:test";

import type { Translate } from "@/components/root/types";
import {
  buildFixTitleMatchSearchVariables,
  fixTitleMatchDialogIdentity,
  handleFixTitleMatchComplete,
} from "./fix-title-match.ts";

const translate: Translate = (key, values) =>
  key === "status.titleMatchUpdated"
    ? `updated:${String(values?.name ?? "")}`
    : key;

test("Fix Match search variables use canonical GraphQL facet enums", () => {
  for (const [facet, expected] of [
    ["movie", "MOVIE"],
    [" MOVIE ", "MOVIE"],
    ["series", "SERIES"],
    ["SERIES", "SERIES"],
    ["anime", "ANIME"],
    [" ANIME ", "ANIME"],
  ] as const) {
    assert.deepEqual(buildFixTitleMatchSearchVariables("Quiet Meridian", facet), {
      query: "Quiet Meridian",
      type: expected,
      limit: 8,
    });
  }
});

test("Fix Match dialog identity is stable across equivalent title objects", () => {
  assert.equal(
    fixTitleMatchDialogIdentity({ id: "movie-1", facet: "MOVIE" }),
    fixTitleMatchDialogIdentity({ id: "movie-1", facet: " movie " }),
  );
  assert.notEqual(
    fixTitleMatchDialogIdentity({ id: "movie-1", facet: "MOVIE" }),
    fixTitleMatchDialogIdentity({ id: "movie-2", facet: "MOVIE" }),
  );
});

test("Fix Match completion refreshes before reporting success", async () => {
  const events: string[] = [];

  await handleFixTitleMatchComplete({
    warnings: [],
    refreshTitleDetail: async () => {
      events.push("refresh");
    },
    setGlobalStatus: (message) => events.push(message),
    t: translate,
    titleName: "Correct Movie",
  });

  assert.deepEqual(events, ["refresh", "updated:Correct Movie"]);
});

test("Fix Match completion reports warnings after refreshing", async () => {
  const events: string[] = [];

  await handleFixTitleMatchComplete({
    warnings: ["Artwork refresh delayed.", "Rename skipped."],
    refreshTitleDetail: async () => {
      events.push("refresh");
    },
    setGlobalStatus: (message) => events.push(message),
    t: translate,
    titleName: "Correct Movie",
  });

  assert.deepEqual(events, [
    "refresh",
    "Artwork refresh delayed. Rename skipped.",
  ]);
});

test("Fix Match completion surfaces refresh failures without false success", async () => {
  const messages: string[] = [];

  await handleFixTitleMatchComplete({
    warnings: [],
    refreshTitleDetail: async () => {
      throw new Error("movie refresh failed");
    },
    setGlobalStatus: (message) => messages.push(message),
    t: translate,
    titleName: "Correct Movie",
  });

  assert.deepEqual(messages, ["movie refresh failed"]);
});
