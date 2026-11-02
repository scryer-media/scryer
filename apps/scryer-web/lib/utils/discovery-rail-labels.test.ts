import assert from "node:assert/strict";
import { readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import railLabels from "../contracts/discovery-rail-labels.json" with { type: "json" };
import en from "../i18n/locales/en.ts";
import {
  discoveryBrowseLabelTitle,
  discoveryBrowseLabelTitleKey,
  discoveryRailLabelTitle,
  discoveryRailLabelTitleKey,
  discoverySectionIsBrowse,
} from "./discovery-sections.ts";

const RAIL_PREFIX = "discovery.rail.becauseYouLike.";

const contractKeys = [...railLabels.genre, ...railLabels.theme];

test("the rail label contract lists each server label once", () => {
  assert.equal(new Set(contractKeys).size, contractKeys.length);
  for (const key of contractKeys) {
    assert.match(key, /^[a-z0-9]+(-[a-z0-9]+)*$/);
  }
});

test("English carries exactly the server's rail label keys", () => {
  const englishKeys = Object.keys(en)
    .filter((key) => key.startsWith(RAIL_PREFIX))
    .map((key) => key.slice(RAIL_PREFIX.length));
  assert.deepEqual([...englishKeys].sort(), [...contractKeys].sort());
  for (const key of contractKeys) {
    assert.equal(discoveryRailLabelTitleKey(key), `${RAIL_PREFIX}${key}`);
  }
});

test("every locale names every rail label with a sentence of its own", async () => {
  const localesDir = path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    "../i18n/locales",
  );
  const files = readdirSync(localesDir).filter((name) => name.endsWith(".ts"));
  assert.equal(files.length, 14);
  for (const file of files) {
    const locale = (await import(path.join(localesDir, file)))
      .default as Record<string, string>;
    const titles = new Set<string>();
    for (const key of contractKeys) {
      const title = locale[discoveryRailLabelTitleKey(key)];
      assert.equal(
        typeof title,
        "string",
        `${file} has no rail title for ${key}`,
      );
      assert.ok(
        title.trim().length > 0,
        `${file} rail title for ${key} is empty`,
      );
      assert.ok(!titles.has(title), `${file} repeats a rail title for ${key}`);
      titles.add(title);
    }
    const extra = Object.keys(locale).filter(
      (key) =>
        key.startsWith(RAIL_PREFIX) &&
        !contractKeys.includes(key.slice(RAIL_PREFIX.length)),
    );
    assert.deepEqual(
      extra,
      [],
      `${file} has rail titles the server never sends`,
    );
  }
});

test("a rail title resolves from its key and leaves unknown keys to the caller", () => {
  const t = (key: string) => (en as Record<string, string>)[key] ?? key;
  assert.equal(
    discoveryRailLabelTitle("science-fiction", t),
    "Because you like science fiction",
  );
  assert.equal(discoveryRailLabelTitle("not-a-rail-label", t), null);
  assert.equal(discoveryRailLabelTitle(null, t), null);
  assert.equal(discoveryRailLabelTitle("  ", t), null);
});

test("English names every browse card the server can send, and no other", () => {
  const nonLabelKeys = new Set(["genreHeading", "themeHeading", "titleCount"]);
  const prefix = "discovery.browseBy.";
  const englishKeys = Object.keys(en)
    .filter((key) => key.startsWith(prefix))
    .map((key) => key.slice(prefix.length))
    .filter((key) => !nonLabelKeys.has(key));
  assert.deepEqual([...englishKeys].sort(), [...contractKeys].sort());
  const nouns = new Set<string>();
  for (const key of contractKeys) {
    assert.equal(discoveryBrowseLabelTitleKey(key), `${prefix}${key}`);
    const noun = (en as Record<string, string>)[`${prefix}${key}`];
    assert.ok(noun?.trim(), `no browse noun for ${key}`);
    assert.ok(!nouns.has(noun), `browse noun for ${key} repeats`);
    nouns.add(noun);
  }
});

test("a browse title resolves from its key and leaves unknown keys to the caller", () => {
  const t = (key: string) => (en as Record<string, string>)[key] ?? key;
  assert.equal(discoveryBrowseLabelTitle("time-travel", t), "Time Travel");
  assert.equal(discoveryBrowseLabelTitle("not-a-rail-label", t), null);
  assert.equal(discoveryBrowseLabelTitle(undefined, t), null);
  assert.equal(discoverySectionIsBrowse({ sectionType: "browse_genre" }), true);
  assert.equal(discoverySectionIsBrowse({ sectionType: "BROWSE_THEME" }), true);
  assert.equal(discoverySectionIsBrowse({ sectionType: "FOR_YOU" }), false);
});
