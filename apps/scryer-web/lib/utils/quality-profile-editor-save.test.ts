import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier.startsWith("@/lib/")) {
      return nextResolve(
        new URL(`../${specifier.slice("@/lib/".length)}.ts`, import.meta.url)
          .href,
        context,
      );
    }
    return nextResolve(specifier, context);
  },
});
const { buildQualityProfileTemplate } = await import("./quality-profiles.ts");
const { commitQualityProfileDraftToEntries, qualityProfileEditorSaveInput } =
  await import("./quality-profile-draft-commit.ts");

test("saving an editor with a stale catalog cannot send an unrelated profile's old tiers", () => {
  const series = buildQualityProfileTemplate("series", "Series");
  series.quality_tiers = ["720P", "1080P"];
  const staleCatalog = commitQualityProfileDraftToEntries(
    [],
    series,
  ).catalogEntries;
  // Another save has already added 2160P on the server. This editor still has
  // the two-tier snapshot while the operator creates an Anime profile.
  const anime = buildQualityProfileTemplate("anime", "Anime");
  anime.quality_tiers = ["720P", "1080P"];
  const committed = commitQualityProfileDraftToEntries(staleCatalog, anime);
  const input = qualityProfileEditorSaveInput(committed.draftEntry);

  assert.deepEqual(
    input.profiles.map((profile) => profile.id),
    ["anime"],
  );
  assert.equal(input.replaceExisting, false);
  assert.deepEqual(staleCatalog[0].criteria.quality_tiers, ["720P", "1080P"]);
  assert.equal(input.globalProfileId, null);
  assert.equal(input.globalScoringPersona, null);
  assert.deepEqual(input.categorySelections, []);
  assert.deepEqual(input.categoryPersonaSelections, []);
});

test("renaming an existing profile preserves its id and the edited tier order", () => {
  const draft = buildQualityProfileTemplate("series", "Series");
  const original = commitQualityProfileDraftToEntries([], draft);
  draft.name = "Renamed series";
  draft.quality_tiers = ["720P", "2160P", "1080P"];
  const committed = commitQualityProfileDraftToEntries(
    original.catalogEntries,
    draft,
  );
  const input = qualityProfileEditorSaveInput(committed.draftEntry);

  assert.equal(input.profiles[0].id, "series");
  assert.equal(input.profiles[0].name, "Renamed series");
  assert.deepEqual(
    input.profiles[0].criteria.qualityTiers,
    draft.quality_tiers,
  );
});
