import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

// Exercise the production serializer and parser with the web alias resolved.
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier.startsWith("@/lib/")) {
      const path = specifier.slice("@/lib/".length);
      return nextResolve(
        new URL(`../${path}.ts`, import.meta.url).href,
        context,
      );
    }
    return nextResolve(specifier, context);
  },
});
const {
  buildQualityProfileTemplate,
  dedupeOrdered,
  moveQualityTier,
  parseQualityProfileCatalogEntries,
  qualityProfileEntryToMutationInput,
  toQualityProfileDraft,
} = await import("./quality-profiles.ts");
const { commitQualityProfileDraftToEntries } =
  await import("./quality-profile-draft-commit.ts");

test("nonnumeric preference survives draft commit, serialization and reload", () => {
  const draft = buildQualityProfileTemplate("fixture", "Fixture");
  draft.quality_tiers = ["720P", "2160P", "1440P", "1080P"];
  draft.quality_tiers = moveQualityTier(draft.quality_tiers, "1080P", 1);
  const { draftEntry } = commitQualityProfileDraftToEntries([], draft);
  assert.deepEqual(
    qualityProfileEntryToMutationInput(draftEntry).criteria.qualityTiers,
    ["720P", "1080P", "2160P", "1440P"],
  );
  const loaded = parseQualityProfileCatalogEntries(
    JSON.stringify([draftEntry]),
  );
  assert.deepEqual(
    toQualityProfileDraft(loaded[0], "fixture", "Fixture").quality_tiers,
    draft.quality_tiers,
  );
});

test("drag and step moves preserve every tier and do not mutate the input", () => {
  const tiers = ["2160P", "1080P", "720P", "1440P"];
  assert.deepEqual(moveQualityTier(tiers, "1440P", 1), [
    "2160P",
    "1440P",
    "1080P",
    "720P",
  ]);
  assert.deepEqual(moveQualityTier(tiers, "1080P", 0), [
    "1080P",
    "2160P",
    "720P",
    "1440P",
  ]);
  assert.deepEqual(moveQualityTier(tiers, "1080P", 2), [
    "2160P",
    "720P",
    "1080P",
    "1440P",
  ]);
  assert.deepEqual(tiers, ["2160P", "1080P", "720P", "1440P"]);
  assert.strictEqual(moveQualityTier(tiers, "2160P", -1), tiers);
  assert.strictEqual(moveQualityTier(tiers, "1440P", tiers.length), tiers);
  assert.strictEqual(moveQualityTier(tiers, "unknown", 0), tiers);
});

test("adding appends once and removing preserves preference", () => {
  let tiers = ["720P", "2160P"];
  tiers = dedupeOrdered([...tiers, "1080P", "720P"]);
  assert.deepEqual(tiers, ["720P", "2160P", "1080P"]);
  assert.deepEqual(
    tiers.filter((tier) => tier !== "2160P"),
    ["720P", "1080P"],
  );
});
