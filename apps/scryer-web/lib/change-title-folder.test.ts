import assert from "node:assert/strict";
import test from "node:test";

import type { Translate } from "@/components/root/types";
import {
  defaultFolderMatchResolution,
  folderMatchOutcomeMessage,
  isInsideRoot,
  normalizeFolderPath,
  parentWithinRoot,
  segmentsWithinRoot,
  type ChangeFolderResult,
} from "./change-title-folder.ts";

const translate: Translate = (key, values) =>
  `${key}:${JSON.stringify(values ?? {})}`;

test("folder paths normalize to their stored form", () => {
  assert.equal(normalizeFolderPath("  /data/Movies/  "), "/data/Movies");
  assert.equal(normalizeFolderPath("/data/Movies"), "/data/Movies");
  assert.equal(normalizeFolderPath("/"), "/");
});

test("browsing stays inside the title's library root", () => {
  assert.equal(isInsideRoot("/data/Movies", "/data/Movies"), true);
  assert.equal(isInsideRoot("/data/Movies/Arrival", "/data/Movies/"), true);
  assert.equal(isInsideRoot("/data/MoviesExtra", "/data/Movies"), false);
  assert.equal(isInsideRoot("/data", "/data/Movies"), false);
  assert.equal(isInsideRoot("/data/Movies", ""), false);
});

test("the parent of a browsed folder is clamped to the root", () => {
  assert.equal(
    parentWithinRoot("/data/Movies/Arrival/Extras", "/data/Movies"),
    "/data/Movies/Arrival",
  );
  assert.equal(parentWithinRoot("/data/Movies/Arrival", "/data/Movies"), "/data/Movies");
  // At the root, and outside it, there is nowhere to go up to.
  assert.equal(parentWithinRoot("/data/Movies", "/data/Movies"), null);
  assert.equal(parentWithinRoot("/elsewhere/Arrival", "/data/Movies"), null);
});

test("breadcrumb segments are relative to the root", () => {
  assert.deepEqual(
    segmentsWithinRoot("/data/Movies/Arrival/Extras", "/data/Movies"),
    ["Arrival", "Extras"],
  );
  assert.deepEqual(segmentsWithinRoot("/data/Movies", "/data/Movies"), []);
  assert.deepEqual(segmentsWithinRoot("/elsewhere", "/data/Movies"), []);
});

test("only an unowned folder preselects a resolution", () => {
  assert.equal(
    defaultFolderMatchResolution({
      ownership: "UNOWNED",
      availableResolutions: ["ASSIGN"],
    }),
    "ASSIGN",
  );
  // A contested folder is never resolved on the user's behalf.
  assert.equal(
    defaultFolderMatchResolution({
      ownership: "OWNED_BY_ANOTHER_TITLE",
      availableResolutions: ["SWAP", "TAKE_OVER"],
    }),
    null,
  );
  assert.equal(
    defaultFolderMatchResolution({
      ownership: "OWNED_BY_THIS_TITLE",
      availableResolutions: [],
    }),
    null,
  );
});

function result(overrides: Partial<ChangeFolderResult>): ChangeFolderResult {
  return {
    outcome: "ASSIGNED",
    title: { id: "1", name: "Arrival", folderPath: "/data/Movies/Arrival" },
    previousFolderPath: null,
    detachedMediaFileCount: 0,
    scan: null,
    swappedTitle: null,
    swappedTitleScan: null,
    displacedTitle: null,
    ...overrides,
  };
}

test("outcome messages name the titles and folders involved", () => {
  assert.equal(
    folderMatchOutcomeMessage(result({}), translate),
    'title.changeFolderOutcomeAssigned:{"name":"Arrival","folder":"/data/Movies/Arrival"}',
  );
  assert.equal(
    folderMatchOutcomeMessage(
      result({
        outcome: "SWAPPED",
        swappedTitle: { id: "2", name: "Contact", folderPath: "/data/Movies/Contact" },
      }),
      translate,
    ),
    'title.changeFolderOutcomeSwapped:{"name":"Arrival","other":"Contact"}',
  );
  assert.equal(
    folderMatchOutcomeMessage(result({ outcome: "ALREADY_OWNED" }), translate),
    'title.changeFolderOutcomeAlreadyOwned:{"name":"Arrival"}',
  );
  assert.equal(
    folderMatchOutcomeMessage(result({ outcome: "TAKEN_OVER" }), translate),
    'title.changeFolderOutcomeAssigned:{"name":"Arrival","folder":"/data/Movies/Arrival"}',
  );
});
