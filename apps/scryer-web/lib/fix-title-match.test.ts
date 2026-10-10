import assert from "node:assert/strict";
import test from "node:test";

import type { Translate } from "@/components/root/types";
import {
  buildFixTitleMatchSearchVariables,
  fixTitleMatchDialogIdentity,
  fixTitleMatchTarget,
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

test("Fix Match applies a result that has an SMG id, a TVDB id or both", () => {
  assert.deepEqual(fixTitleMatchTarget({ smgId: 4101, tvdbId: "" }), { smgId: 4101 });
  assert.deepEqual(fixTitleMatchTarget({ smgId: null, tvdbId: " 72001 " }), { tvdbId: "72001" });
  assert.deepEqual(fixTitleMatchTarget({ smgId: 4101, tvdbId: "72001" }), {
    smgId: 4101,
    tvdbId: "72001",
  });
});

test("Fix Match cannot apply a result without a usable identity", () => {
  assert.equal(fixTitleMatchTarget(null), null);
  assert.equal(fixTitleMatchTarget(undefined), null);
  assert.equal(fixTitleMatchTarget({ smgId: null, tvdbId: "  " }), null);
  assert.equal(fixTitleMatchTarget({ smgId: 0, tvdbId: "" }), null);
  assert.deepEqual(fixTitleMatchTarget({ smgId: -3, tvdbId: "72001" }), { tvdbId: "72001" });
});

test("Fix Match completion refreshes before reporting success", async () => {
  const events: string[] = [];

  await handleFixTitleMatchComplete({
    warnings: [],
    refreshTitleDetail: async () => {
      events.push("refresh");
    },
    setGlobalStatus: (message, options) => events.push(`${options?.level}:${message}`),
    t: translate,
    titleName: "Correct Movie",
  });

  assert.deepEqual(events, ["refresh", "SUCCESS:updated:Correct Movie"]);
});

test("Fix Match completion reports warnings after refreshing", async () => {
  const events: string[] = [];

  await handleFixTitleMatchComplete({
    warnings: ["Artwork refresh delayed.", "Rename skipped."],
    refreshTitleDetail: async () => {
      events.push("refresh");
    },
    setGlobalStatus: (message, options) => events.push(`${options?.level}:${message}`),
    t: translate,
    titleName: "Correct Movie",
  });

  assert.deepEqual(events, [
    "refresh",
    "WARNING:Artwork refresh delayed. Rename skipped.",
  ]);
});

test("Fix Match completion surfaces refresh failures without false success", async () => {
  const messages: string[] = [];

  await handleFixTitleMatchComplete({
    warnings: [],
    refreshTitleDetail: async () => {
      throw new Error("movie refresh failed");
    },
    setGlobalStatus: (message, options) => messages.push(`${options?.level}:${message}`),
    t: translate,
    titleName: "Correct Movie",
  });

  assert.deepEqual(messages, ["ERROR:movie refresh failed"]);
});
