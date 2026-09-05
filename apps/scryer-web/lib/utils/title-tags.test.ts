import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import type { TitleTagDefinition, TitleTagRewriteCounts } from "../types/title-tags.ts";
import {
  availableTitleTagLabels,
  buildBulkTitleTagsDelta,
  EMPTY_TITLE_TAG_REWRITE_COUNTS,
  formatTitleTagRenameSummary,
  formatTitleTagRenameWarning,
  hasBulkTitleTagsChanges,
  isEmptyTitleTagsDelta,
  isReservedTitleTag,
  normalizeTitleTagLabel,
  titleTagLabelErrorKey,
  titleTagRenameWarning,
  titleTagsDelta,
  userTitleTags,
} from "./title-tags.ts";

function definition(
  label: string,
  overrides: Partial<TitleTagDefinition> = {},
): TitleTagDefinition {
  return {
    id: `tag-${label.replace(/\s+/g, "-")}`,
    label,
    description: null,
    titleCount: 0,
    seriesMovieCount: 0,
    createdAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function counts(overrides: Partial<TitleTagRewriteCounts>): TitleTagRewriteCounts {
  return { ...EMPTY_TITLE_TAG_REWRITE_COUNTS, ...overrides };
}

/// A translate stub that renders the key and its values, so a test asserts the
/// composition rather than the English wording.
function stubTranslate(key: string, values?: Record<string, string | number>) {
  const rendered = values
    ? Object.entries(values)
        .map(([name, value]) => `${name}=${value}`)
        .join("|")
    : "";
  return rendered ? `${key}(${rendered})` : key;
}

test("normalizes labels the way the registry stores them", () => {
  assert.equal(normalizeTitleTagLabel("  Needs   Review "), "needs review");
  assert.equal(normalizeTitleTagLabel("KEEP"), "keep");
  assert.equal(normalizeTitleTagLabel("   "), "");
});

test("treats scryer-prefixed entries as reserved regardless of spelling", () => {
  assert.equal(isReservedTitleTag("scryer:quality-profile:hd"), true);
  assert.equal(isReservedTitleTag("  SCRYER:monitor-type:all "), true);
  assert.equal(isReservedTitleTag("keep"), false);
  assert.equal(isReservedTitleTag("season 1: opener"), false);
});

test("user tags drop reserved entries and normalize the rest", () => {
  assert.deepEqual(
    userTitleTags([
      "scryer:quality-profile:hd",
      "Needs  Review",
      "keep",
      "KEEP",
      "   ",
      "scryer:mal-score:9.1",
    ]),
    ["keep", "needs review"],
  );
  assert.deepEqual(userTitleTags(null), []);
  assert.deepEqual(userTitleTags([]), []);
});

test("picker delta names only what actually changed", () => {
  const delta = titleTagsDelta(
    ["keep", "needs review", "scryer:monitor-type:all"],
    ["keep", "archive"],
  );
  assert.deepEqual(delta, { add: ["archive"], remove: ["needs review"] });
  assert.equal(isEmptyTitleTagsDelta(delta), false);
});

test("picker delta is empty when only spelling differs", () => {
  const delta = titleTagsDelta(["needs review"], ["Needs  Review"]);
  assert.deepEqual(delta, { add: [], remove: [] });
  assert.equal(isEmptyTitleTagsDelta(delta), true);
});

test("picker delta never touches reserved entries", () => {
  const delta = titleTagsDelta(
    ["scryer:quality-profile:hd", "keep"],
    ["scryer:quality-profile:hd", "keep", "archive"],
  );
  assert.deepEqual(delta, { add: ["archive"], remove: [] });
});

test("picker delta adds everything when a title starts bare", () => {
  assert.deepEqual(titleTagsDelta([], ["archive", "keep"]), {
    add: ["archive", "keep"],
    remove: [],
  });
});

test("available labels exclude what the title already carries", () => {
  const registry = [definition("keep"), definition("archive"), definition("Needs Review")];
  assert.deepEqual(availableTitleTagLabels(registry, ["keep"]), [
    "archive",
    "needs review",
  ]);
  assert.deepEqual(availableTitleTagLabels(registry, ["keep", "archive", "needs review"]), []);
  assert.deepEqual(availableTitleTagLabels([], ["keep"]), []);
});

test("bulk change builder is empty when both pickers are empty", () => {
  const draft = { add: [], remove: [] };
  assert.deepEqual(buildBulkTitleTagsDelta(draft), { add: [], remove: [] });
  assert.equal(hasBulkTitleTagsChanges(draft), false);
});

test("bulk change builder normalizes and sorts each side", () => {
  const draft = { add: ["Needs  Review", "archive"], remove: ["KEEP"] };
  assert.deepEqual(buildBulkTitleTagsDelta(draft), {
    add: ["archive", "needs review"],
    remove: ["keep"],
  });
  assert.equal(hasBulkTitleTagsChanges(draft), true);
});

test("bulk change builder lets removal win a contradictory label", () => {
  const draft = { add: ["keep", "archive"], remove: ["keep"] };
  assert.deepEqual(buildBulkTitleTagsDelta(draft), {
    add: ["archive"],
    remove: ["keep"],
  });
});

test("bulk change builder drops reserved entries outright", () => {
  const draft = { add: ["scryer:monitor-type:all"], remove: ["scryer:filler-policy:skip"] };
  assert.deepEqual(buildBulkTitleTagsDelta(draft), { add: [], remove: [] });
  assert.equal(hasBulkTitleTagsChanges(draft), false);
});

test("rename warning is null when nothing was left behind", () => {
  assert.equal(titleTagRenameWarning(counts({ titles: 12, delayProfiles: 2 })), null);
  assert.equal(
    formatTitleTagRenameWarning(
      counts({ titles: 12, delayProfiles: 2 }),
      "needs review",
      stubTranslate,
    ),
    null,
  );
});

test("rename warning totals every rule reference the rename could not rewrite", () => {
  const warning = titleTagRenameWarning(
    counts({ maintenanceRuleSets: 2, releaseRuleSets: 1, managedTagFilters: 3 }),
  );
  assert.deepEqual(warning, {
    total: 6,
    references: [
      { kind: "maintenanceRuleSets", count: 2 },
      { kind: "releaseRuleSets", count: 1 },
      { kind: "managedTagFilters", count: 3 },
    ],
  });
});

test("rename warning lists only the non-zero reference kinds, in a fixed order", () => {
  const warning = titleTagRenameWarning(
    counts({ maintenanceRuleSets: 0, releaseRuleSets: 4, managedTagFilters: 1 }),
  );
  assert.deepEqual(warning, {
    total: 5,
    references: [
      { kind: "releaseRuleSets", count: 4 },
      { kind: "managedTagFilters", count: 1 },
    ],
  });
});

test("rename warning names the old label and the places that still use it", () => {
  const message = formatTitleTagRenameWarning(
    counts({ maintenanceRuleSets: 2, managedTagFilters: 1 }),
    "needs review",
    stubTranslate,
  );
  assert.equal(
    message,
    "settings.titleTagRenameWarning(label=needs review|count=3|references=" +
      "settings.titleTagReferenceMaintenanceRuleSets(count=2), " +
      "settings.titleTagReferenceManagedTagFilters(count=1))",
  );
});

test("rename summary always states what was rewritten", () => {
  assert.equal(
    formatTitleTagRenameSummary(
      counts({ titles: 12, seriesMovies: 3, delayProfiles: 2 }),
      "archive",
      stubTranslate,
    ),
    "settings.titleTagRenameSummary(label=archive|titles=12|seriesMovies=3|delayProfiles=2)",
  );
});

test("every locale key the rename warning composes exists in English", () => {
  for (const key of [
    "settings.titleTagRenameWarning",
    "settings.titleTagRenameSummary",
    "settings.titleTagReferenceMaintenanceRuleSets",
    "settings.titleTagReferenceReleaseRuleSets",
    "settings.titleTagReferenceManagedTagFilters",
  ]) {
    assert.equal(typeof en[key], "string", `${key} missing from en.ts`);
    assert.notEqual(en[key], "");
  }
});

test("label validation names the reason a label cannot be defined", () => {
  assert.equal(titleTagLabelErrorKey("keep"), null);
  assert.equal(titleTagLabelErrorKey("  Needs  Review "), null);
  assert.equal(titleTagLabelErrorKey("   "), "settings.titleTagLabelRequired");
  assert.equal(
    titleTagLabelErrorKey("scryer:quality-profile:hd"),
    "settings.titleTagLabelReserved",
  );
  assert.equal(titleTagLabelErrorKey("x".repeat(65)), "settings.titleTagLabelTooLong");
  assert.equal(titleTagLabelErrorKey("bad\u0007label"), "settings.titleTagLabelInvalid");
});
