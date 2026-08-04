import assert from "node:assert/strict";
import test from "node:test";

import { groupRecycleBinItems } from "./recycle-bin.ts";

const items = [
  {
    id: "one-old",
    fileName: "Arrival.2016.1080p.mkv",
    titleId: "arrival",
    titleName: "Arrival",
    libraryId: "movies",
    libraryName: "Movies",
    recycledAt: "2026-08-01T08:00:00Z",
  },
  {
    id: "one-new",
    fileName: "Arrival.2016.2160p.mkv",
    titleId: "arrival",
    titleName: "Arrival",
    libraryId: "movies",
    libraryName: "Movies",
    recycledAt: "2026-08-02T08:00:00Z",
  },
  {
    id: "two",
    fileName: "The.Matrix.1999.mkv",
    titleId: "matrix",
    titleName: "The Matrix",
    libraryId: "movies",
    libraryName: "Movies",
    recycledAt: "2026-08-03T08:00:00Z",
  },
  {
    id: "unassociated",
    fileName: "manual-extra.mkv",
    titleId: null,
    titleName: null,
    libraryId: "movies",
    libraryName: "Movies",
    recycledAt: "2026-08-04T08:00:00Z",
  },
];

test("recycle-bin grouping keeps a matched title's files together and newest first", () => {
  const groups = groupRecycleBinItems(items, "arrival", "Unassociated files");

  assert.equal(groups.length, 1);
  assert.equal(groups[0]?.titleName, "Arrival");
  assert.deepEqual(groups[0]?.items.map((item) => item.id), ["one-new", "one-old"]);
});

test("recycle-bin file filtering narrows only matching files and supports unassociated files", () => {
  const matchingFile = groupRecycleBinItems(items, "2160p", "Unassociated files");
  assert.equal(matchingFile.length, 1);
  assert.deepEqual(matchingFile[0]?.items.map((item) => item.id), ["one-new"]);

  const unassociated = groupRecycleBinItems(items, "manual-extra", "Unassociated files");
  assert.equal(unassociated.length, 1);
  assert.equal(unassociated[0]?.titleName, "Unassociated files");
  assert.deepEqual(unassociated[0]?.items.map((item) => item.id), ["unassociated"]);
});
