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
  qualityProfileEntryToMutationInput,
  toQualityProfileDraft,
} = await import("./quality-profiles.ts");
import { QUALITY_TIER_CHOICES } from "../constants/quality-profiles.ts";
import type {
  ParsedQualityProfileEntry,
  QualityProfileDraft,
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
      min_upgrade_score_delta: 120,
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
    ignore_proper_repack: false,
    scoring_overrides: {},
    cutoff_tier: "",
    min_score_to_grab: null,
    min_upgrade_score_delta: 120,
  };
}

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

test("profile draft save and clone preserve Ignore PROPER/REPACK", () => {
  const draft = {
    ...makeDraft("default", "Cinema"),
    ignore_proper_repack: true,
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
