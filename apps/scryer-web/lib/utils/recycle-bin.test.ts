import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRecycleBinSettingsInput,
  groupRecycleBinItems,
  parseRecycleBinRetentionDays,
} from "./recycle-bin.ts";

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
    fileName: "The.Meridian.1999.mkv",
    titleId: "meridian",
    titleName: "The Meridian",
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

test("recycle bin settings input carries only the changed fields", () => {
  assert.deepEqual(buildRecycleBinSettingsInput({ enabled: false }), { enabled: false });
  assert.equal("path" in buildRecycleBinSettingsInput({ enabled: true }), false);
  assert.equal("retentionDays" in buildRecycleBinSettingsInput({ enabled: true }), false);
  assert.deepEqual(buildRecycleBinSettingsInput({ path: "  /srv/bin  ", retentionDays: 14 }), {
    path: "/srv/bin",
    retentionDays: 14,
  });
  assert.deepEqual(buildRecycleBinSettingsInput({ path: "   ", retentionDays: 7 }), {
    path: null,
    retentionDays: 7,
  });
  assert.deepEqual(buildRecycleBinSettingsInput({ path: null }), { path: null });
  assert.deepEqual(buildRecycleBinSettingsInput({}), {});
});

test("recycle bin retention accepts only whole days in the server range", () => {
  assert.equal(parseRecycleBinRetentionDays(" 30 "), 30);
  assert.equal(parseRecycleBinRetentionDays("1"), 1);
  assert.equal(parseRecycleBinRetentionDays("3650"), 3650);
  for (const rejected of ["", "0", "3651", "-1", "1.5", "7 days"]) {
    assert.equal(parseRecycleBinRetentionDays(rejected), null, rejected);
  }
});
