import test from "node:test";
import assert from "node:assert/strict";
import { registerHooks } from "node:module";

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
const { commitQualityProfileDraftToEntries, hasDuplicateQualityProfileName } =
  await import("./quality-profile-draft-commit.ts");
const {
  DEFAULT_MIN_UPGRADE_SCORE_DELTA,
  normalizeQualityProfileEntry,
  qualityProfileEntryFromGraph,
  qualityProfileEntryToMutationInput,
  toQualityProfileDraft,
} = await import("./quality-profiles.ts");
import { QUALITY_TIER_CHOICES } from "../constants/quality-profiles.ts";
import type {
  ParsedQualityProfileEntry,
  QualityProfileDraft,
  QualityProfileGraphCriteriaPayload,
} from "@/lib/types";

function makeEntry(id: string, name: string): ParsedQualityProfileEntry {
  return {
    id,
    name,
    criteria: {
      quality_tiers: ["2160P"],
      archival_quality: "2160P",
      allow_unknown_quality: false,
      source_allowlist: [],
      source_blocklist: [],
      video_codec_allowlist: [],
      video_codec_blocklist: [],
      audio_codec_allowlist: [],
      audio_codec_blocklist: [],
      dolby_vision_allowed: true,
      detected_hdr_allowed: true,
      prefer_remux: false,
      allow_bd_disk: true,
      allow_upgrades: true,
      ignore_proper_repack: false,
      scoring_overrides: {},
      cutoff_tier: null,
      min_score_to_grab: null,
      cutoff_score: null,
      min_upgrade_score_delta: 120,
      min_missing_episodes_for_pack: 2,
      min_missing_percent_for_series_pack: 30,
      decision_pipeline: [
        "QUALITY_TIER",
        "REVISION",
        "PREFERRED_LANGUAGE",
        "OPTIONAL_LANGUAGES",
      ],
    },
  };
}

function makeDraft(id: string, name: string): QualityProfileDraft {
  return {
    id,
    name,
    quality_tiers: ["2160P"],
    archival_quality: "2160P",
    allow_unknown_quality: false,
    source_allowlist: [],
    source_blocklist: [],
    video_codec_allowlist: [],
    video_codec_blocklist: [],
    audio_codec_allowlist: [],
    audio_codec_blocklist: [],
    dolby_vision_allowed: true,
    detected_hdr_allowed: true,
    prefer_remux: false,
    allow_bd_disk: true,
    allow_upgrades: true,
    scoring_overrides: {},
    cutoff_tier: "",
    min_score_to_grab: null,
    cutoff_score: null,
    min_upgrade_score_delta: 120,
    min_missing_episodes_for_pack: 2,
    min_missing_percent_for_series_pack: 30,
    decision_pipeline: [
      "QUALITY_TIER",
      "REVISION",
      "PREFERRED_LANGUAGE",
      "OPTIONAL_LANGUAGES",
    ],
  };
}

test("pack thresholds survive profile editing and mutation serialization", () => {
  const raw = makeEntry("packs", "Pack thresholds");
  const entry = normalizeQualityProfileEntry({
    ...raw,
    criteria: {
      ...raw.criteria,
      min_missing_episodes_for_pack: 1,
      min_missing_percent_for_series_pack: 55,
    },
  });
  assert.ok(entry);
  assert.equal(entry.criteria.min_missing_episodes_for_pack, 1);
  assert.equal(entry.criteria.min_missing_percent_for_series_pack, 55);
  const draft = toQualityProfileDraft(entry, "fallback", "Fallback");
  assert.equal(draft.min_missing_episodes_for_pack, 1);
  assert.equal(draft.min_missing_percent_for_series_pack, 55);
  const committed = commitQualityProfileDraftToEntries([entry], draft);
  const input = qualityProfileEntryToMutationInput(committed.draftEntry);
  assert.equal(input.criteria.minMissingEpisodesForPack, 1);
  assert.equal(input.criteria.minMissingPercentForSeriesPack, 55);

  const hydrated = qualityProfileEntryFromGraph({
    id: "packs",
    name: "Pack thresholds",
    criteria: graphCriteria({
      minMissingEpisodesForPack: 3,
      minMissingPercentForSeriesPack: 0,
    }),
  });
  assert.equal(hydrated.criteria.min_missing_episodes_for_pack, 3);
  assert.equal(hydrated.criteria.min_missing_percent_for_series_pack, 0);
});

test("absent or out-of-range pack thresholds read as the defaults", () => {
  const raw = makeEntry("packs", "Pack thresholds");
  const { min_missing_episodes_for_pack, min_missing_percent_for_series_pack, ...rest } =
    raw.criteria;
  void min_missing_episodes_for_pack;
  void min_missing_percent_for_series_pack;
  const absent = normalizeQualityProfileEntry({ ...raw, criteria: rest });
  assert.equal(absent?.criteria.min_missing_episodes_for_pack, 2);
  assert.equal(absent?.criteria.min_missing_percent_for_series_pack, 30);
  const invalid = normalizeQualityProfileEntry({
    ...raw,
    criteria: {
      ...raw.criteria,
      min_missing_episodes_for_pack: 0,
      min_missing_percent_for_series_pack: 101,
    },
  });
  assert.equal(invalid?.criteria.min_missing_episodes_for_pack, 2);
  assert.equal(invalid?.criteria.min_missing_percent_for_series_pack, 30);
  const template = toQualityProfileDraft(null, "new", "New");
  assert.equal(template.min_missing_episodes_for_pack, 2);
  assert.equal(template.min_missing_percent_for_series_pack, 30);
});

test("commitQualityProfileDraftToEntries preserves id when renaming an existing profile", () => {
  const entries = [makeEntry("default", "4K")];

  const committed = commitQualityProfileDraftToEntries(
    entries,
    makeDraft("default", "Cinema Prime"),
  );

  assert.equal(committed.catalogEntries.length, 1);
  assert.equal(committed.draftEntry.id, "default");
  assert.equal(committed.draftEntry.name, "Cinema Prime");
  assert.equal(committed.catalogEntries[0]?.id, "default");
  assert.equal(committed.catalogEntries[0]?.name, "Cinema Prime");
});

test("commitQualityProfileDraftToEntries appends a new profile for a create draft", () => {
  const entries = [makeEntry("default", "4K")];

  const committed = commitQualityProfileDraftToEntries(
    entries,
    makeDraft("", "Anime Max"),
  );

  assert.equal(committed.catalogEntries.length, 2);
  assert.equal(committed.draftEntry.id, "anime-max");
  assert.equal(committed.catalogEntries[1]?.id, "anime-max");
  assert.equal(committed.catalogEntries[1]?.name, "Anime Max");
});

test("hasDuplicateQualityProfileName ignores the current profile id during rename", () => {
  const entries = [
    makeEntry("default", "4K"),
    makeEntry("anime-max", "Anime Max"),
  ];

  assert.equal(hasDuplicateQualityProfileName(entries, "4K", "default"), false);
  assert.equal(
    hasDuplicateQualityProfileName(entries, " anime max ", "default"),
    true,
  );
});

test("quality profile choices expose 8k", () => {
  assert.ok(
    QUALITY_TIER_CHOICES.some(
      (choice) => choice.value === "4320P" && choice.label.includes("8K"),
    ),
  );
});

test("profile draft save and clone derive Ignore PROPER/REPACK from the decision order", () => {
  const draft = {
    ...makeDraft("default", "Cinema"),
    decision_pipeline: ["QUALITY_TIER", "PREFERRED_LANGUAGE"],
  };
  const saved = commitQualityProfileDraftToEntries(
    [makeEntry("default", "Cinema")],
    draft,
  );
  assert.equal(saved.catalogEntries[0]?.criteria.ignore_proper_repack, true);
  const cloned = commitQualityProfileDraftToEntries(saved.catalogEntries, {
    ...draft,
    id: "",
    name: "Clone",
  });
  assert.equal(cloned.draftEntry.criteria.ignore_proper_repack, true);
});

test("the same-tier upgrade delta survives draft, save and mutation input", () => {
  const draft = {
    ...makeDraft("default", "Cinema"),
    min_upgrade_score_delta: 5,
  };
  const saved = commitQualityProfileDraftToEntries(
    [makeEntry("default", "Cinema")],
    draft,
  );
  const entry = saved.catalogEntries[0];
  assert.ok(entry);
  assert.equal(entry.criteria.min_upgrade_score_delta, 5);
  assert.equal(
    toQualityProfileDraft(entry, "x", "x").min_upgrade_score_delta,
    5,
  );
  assert.equal(
    qualityProfileEntryToMutationInput(entry).criteria.minUpgradeScoreDelta,
    5,
  );
});

test("a cutoff score survives draft, save and mutation input, and clearing it sends null", () => {
  const saved = commitQualityProfileDraftToEntries(
    [makeEntry("default", "Cinema")],
    { ...makeDraft("default", "Cinema"), cutoff_score: 450 },
  );
  const entry = saved.catalogEntries[0];
  assert.ok(entry);
  assert.equal(entry.criteria.cutoff_score, 450);
  assert.equal(toQualityProfileDraft(entry, "x", "x").cutoff_score, 450);
  assert.equal(
    qualityProfileEntryToMutationInput(entry).criteria.cutoffScore,
    450,
  );

  const cleared = commitQualityProfileDraftToEntries(saved.catalogEntries, {
    ...toQualityProfileDraft(entry, "x", "x"),
    cutoff_score: null,
  });
  const clearedInput = qualityProfileEntryToMutationInput(
    cleared.catalogEntries[0]!,
  ).criteria;
  assert.ok("cutoffScore" in clearedInput);
  assert.equal(clearedInput.cutoffScore, null);
});

test("a cutoff score from the server is read, and a missing one reads as none", () => {
  const withScore = qualityProfileEntryFromGraph({
    id: "graph",
    name: "Graph",
    criteria: graphCriteria({ cutoffScore: -20 }),
  });
  assert.equal(withScore.criteria.cutoff_score, -20);
  assert.equal(
    normalizeQualityProfileEntry({
      id: "imported",
      name: "Imported",
      criteria: { quality_tiers: ["1080P"] },
    })?.criteria.cutoff_score,
    null,
  );
});

test("a profile without a same-tier upgrade delta reads as the default", () => {
  const parsed = normalizeQualityProfileEntry({
    id: "imported",
    name: "Imported",
    criteria: { quality_tiers: ["1080P"] },
  });
  assert.equal(
    parsed?.criteria.min_upgrade_score_delta,
    DEFAULT_MIN_UPGRADE_SCORE_DELTA,
  );
  const negative = normalizeQualityProfileEntry({
    id: "imported",
    name: "Imported",
    criteria: { quality_tiers: ["1080P"], min_upgrade_score_delta: -3 },
  });
  assert.equal(
    negative?.criteria.min_upgrade_score_delta,
    DEFAULT_MIN_UPGRADE_SCORE_DELTA,
  );
});

function graphCriteria(
  overrides: Partial<QualityProfileGraphCriteriaPayload>,
): QualityProfileGraphCriteriaPayload {
  return {
    qualityTiers: ["2160P"],
    archivalQuality: "2160P",
    allowUnknownQuality: false,
    sourceAllowlist: [],
    sourceBlocklist: [],
    videoCodecAllowlist: [],
    videoCodecBlocklist: [],
    audioCodecAllowlist: [],
    audioCodecBlocklist: [],
    dolbyVisionAllowed: true,
    detectedHdrAllowed: true,
    preferRemux: false,
    allowBdDisk: true,
    allowUpgrades: true,
    ignoreProperRepack: false,
    scoringOverrides: {},
    cutoffTier: null,
    minScoreToGrab: null,
    cutoffScore: null,
    minUpgradeScoreDelta: 120,
    minMissingEpisodesForPack: 2,
    minMissingPercentForSeriesPack: 30,
    decisionPipeline: [
      "QUALITY_TIER",
      "REVISION",
      "PREFERRED_LANGUAGE",
      "OPTIONAL_LANGUAGES",
    ],
    ...overrides,
  };
}

test("the decision order survives draft, save and mutation input", () => {
  const draft: QualityProfileDraft = {
    ...makeDraft("default", "Cinema"),
    cutoff_tier: "2160P",
    decision_pipeline: ["PREFERRED_LANGUAGE", "REVISION"],
  };
  const saved = commitQualityProfileDraftToEntries(
    [makeEntry("default", "Cinema")],
    draft,
  );
  const entry = saved.catalogEntries[0];
  assert.ok(entry);
  assert.deepEqual(entry.criteria.decision_pipeline, [
    "PREFERRED_LANGUAGE",
    "REVISION",
  ]);
  // The cutoff tier is kept while it does not apply.
  assert.equal(entry.criteria.cutoff_tier, "2160P");
  assert.deepEqual(toQualityProfileDraft(entry, "x", "x").decision_pipeline, [
    "PREFERRED_LANGUAGE",
    "REVISION",
  ]);
  const input = qualityProfileEntryToMutationInput(entry).criteria;
  assert.deepEqual(input.decisionPipeline, ["PREFERRED_LANGUAGE", "REVISION"]);
  assert.equal(input.ignoreProperRepack, false);
});

test("a score-only order is sent as an empty list that ignores PROPER/REPACK", () => {
  const saved = commitQualityProfileDraftToEntries(
    [makeEntry("default", "Cinema")],
    { ...makeDraft("default", "Cinema"), decision_pipeline: [] },
  );
  const entry = saved.catalogEntries[0];
  assert.ok(entry);
  const input = qualityProfileEntryToMutationInput(entry).criteria;
  assert.deepEqual(input.decisionPipeline, []);
  assert.equal(input.ignoreProperRepack, true);
});

test("a catalog entry without a decision order falls back to its PROPER/REPACK setting", () => {
  const plain = normalizeQualityProfileEntry({
    id: "imported",
    name: "Imported",
    criteria: { quality_tiers: ["1080P"] },
  });
  assert.deepEqual(plain?.criteria.decision_pipeline, [
    "QUALITY_TIER",
    "REVISION",
  ]);
  const ignoring = normalizeQualityProfileEntry({
    id: "imported",
    name: "Imported",
    criteria: { quality_tiers: ["1080P"], ignore_proper_repack: true },
  });
  assert.deepEqual(ignoring?.criteria.decision_pipeline, ["QUALITY_TIER"]);
  assert.equal(ignoring?.criteria.ignore_proper_repack, true);
});

test("a server order with an unknown gate is kept in place", () => {
  const fromGraph = qualityProfileEntryFromGraph({
    id: "graph",
    name: "Graph",
    criteria: graphCriteria({
      decisionPipeline: ["PREFERRED_LANGUAGE", "FUTURE_GATE", "QUALITY_TIER"],
    }),
  });
  assert.deepEqual(fromGraph.criteria.decision_pipeline, [
    "PREFERRED_LANGUAGE",
    "FUTURE_GATE",
    "QUALITY_TIER",
  ]);
  assert.equal(fromGraph.criteria.ignore_proper_repack, true);
  assert.deepEqual(
    qualityProfileEntryToMutationInput(fromGraph).criteria.decisionPipeline,
    ["PREFERRED_LANGUAGE", "FUTURE_GATE", "QUALITY_TIER"],
  );
});
