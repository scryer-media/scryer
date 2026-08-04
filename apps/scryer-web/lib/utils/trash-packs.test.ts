import assert from "node:assert/strict";
import test from "node:test";

import type { RuleSetRecord } from "../types/rule-sets.ts";
import {
  conflictingFrenchPack,
  formatTagFilter,
  isTrashLocalePack,
  parseTagFilterInput,
  trashLocalePacks,
} from "./trash-packs.ts";

function record(overrides: Partial<RuleSetRecord>): RuleSetRecord {
  return {
    id: "rule-1",
    name: "rule",
    description: "",
    regoSource: "",
    enabled: false,
    priority: 0,
    appliedFacets: [],
    isManaged: false,
    managedKey: null,
    managedTagFilter: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function pack(key: string, overrides: Partial<RuleSetRecord> = {}): RuleSetRecord {
  return record({
    id: key,
    name: key,
    isManaged: true,
    managedKey: key,
    ...overrides,
  });
}

test("only managed trash locale rows count as packs", () => {
  assert.equal(isTrashLocalePack(pack("trash-guides:locale:german")), true);
  assert.equal(
    isTrashLocalePack(record({ managedKey: "trash-guides:locale:german" })),
    false,
  );
  assert.equal(
    isTrashLocalePack(pack("convenience:required-audio:anime")),
    false,
  );
});

test("packs sort in display order with french variants first", () => {
  const records = [
    pack("trash-guides:locale:asian"),
    record({ id: "user", name: "user rule" }),
    pack("trash-guides:locale:german"),
    pack("trash-guides:locale:french-vostfr"),
    pack("trash-guides:locale:french-vf"),
    pack("trash-guides:locale:french-vo"),
  ];
  assert.deepEqual(
    trashLocalePacks(records).map((r) => r.managedKey),
    [
      "trash-guides:locale:french-vf",
      "trash-guides:locale:french-vo",
      "trash-guides:locale:french-vostfr",
      "trash-guides:locale:german",
      "trash-guides:locale:asian",
    ],
  );
});

test("enabling a second french pack reports the enabled one", () => {
  const vf = pack("trash-guides:locale:french-vf", { enabled: true });
  const vo = pack("trash-guides:locale:french-vo");
  const german = pack("trash-guides:locale:german");

  assert.equal(conflictingFrenchPack([vf, vo, german], vo), vf);
  // Non-french packs never conflict, and re-toggling the enabled pack itself
  // is not a conflict.
  assert.equal(conflictingFrenchPack([vf, vo, german], german), null);
  assert.equal(conflictingFrenchPack([vf, vo, german], vf), null);
});

test("tag filter input normalizes and round-trips", () => {
  assert.deepEqual(parseTagFilterInput(" Locale:French ,, locale:vf ,LOCALE:FRENCH"), [
    "locale:french",
    "locale:vf",
  ]);
  assert.deepEqual(parseTagFilterInput("   "), []);
  assert.equal(formatTagFilter(["locale:french", "locale:vf"]), "locale:french, locale:vf");
  assert.equal(formatTagFilter(null), "");
});
