import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  deriveSeedingProgress,
  formatSeedDuration,
  formatSeedRatio,
  isImportedSeedingRow,
  isPrivateTorrentRow,
  type SeedingProgressInput,
} from "./seeding-progress.ts";

function row(
  overrides: Partial<SeedingProgressInput> = {},
): SeedingProgressInput {
  return {
    seedingState: "SEEDING",
    seedRatio: null,
    seedRatioGoal: null,
    seedTimeSeconds: null,
    seedTimeGoalSeconds: null,
    ...overrides,
  };
}

test("a usenet row reports no seeding at all", () => {
  assert.equal(deriveSeedingProgress(row({ seedingState: null })), null);
});

test("a torrent with nothing to report yet renders nothing", () => {
  // Still downloading, a silent client, or torrent-blackhole, which the backend
  // deliberately maps to NONE because it has no session to observe.
  assert.equal(
    deriveSeedingProgress(
      row({ seedingState: "NONE", seedRatio: 0.4, seedRatioGoal: 1.5 }),
    ),
    null,
  );
});

test("an observed ratio is shown against its goal", () => {
  const progress = deriveSeedingProgress(
    row({ seedRatio: 0.8, seedRatioGoal: 1.5 }),
  );

  assert.equal(progress?.ratioLabel, "0.8 / 1.5");
  assert.equal(progress?.seedTimeLabel, null);
});

test("an observed ratio with no resolved goal is shown on its own", () => {
  const progress = deriveSeedingProgress(row({ seedRatio: 2.25 }));

  assert.equal(progress?.ratioLabel, "2.25");
});

test("a goal with no observation behind it renders nothing", () => {
  // The field is nullable because the client may report no ratio at all, and
  // "0 / 1.5" would be a claim the queue cannot make.
  const progress = deriveSeedingProgress(row({ seedRatioGoal: 1.5 }));

  assert.equal(progress?.ratioLabel, null);
});

test("a zero observation is a real reading, not an unknown one", () => {
  const progress = deriveSeedingProgress(
    row({ seedRatio: 0, seedTimeSeconds: 0, seedRatioGoal: 1 }),
  );

  assert.equal(progress?.ratioLabel, "0 / 1");
  // Sub-minute readings render as "<1m" so a real observation is never
  // mistakable for the zero-for-unknown this module refuses to show.
  assert.equal(progress?.seedTimeLabel, "<1m");
});

test("seed time is humanized against its goal", () => {
  const progress = deriveSeedingProgress(
    row({ seedTimeSeconds: 273_600, seedTimeGoalSeconds: 604_800 }),
  );

  assert.equal(progress?.seedTimeLabel, "3d 4h / 7d");
});

test("each reportable state carries its own label and tone", () => {
  const states = [
    "SEEDING",
    "GOAL_MET",
    "HELD_PRIVATE",
    "NEVER_REMOVE",
  ] as const;
  const seen = new Set<string>();

  for (const seedingState of states) {
    const progress = deriveSeedingProgress(row({ seedingState }));
    assert.ok(progress, `${seedingState} should be reportable`);
    assert.equal(progress.stateKey, seedingState);
    assert.ok(
      progress.labelKey in en,
      `${progress.labelKey} is missing from the English locale`,
    );
    assert.ok(progress.toneClass.length > 0);
    seen.add(progress.labelKey);
  }

  assert.equal(seen.size, states.length, "every state needs a distinct label");
});

test("a held state is still reported when the client observed nothing", () => {
  // "Held because the tracker is private" is knowable without a single number,
  // and it is the reason the entry is still in the client.
  const progress = deriveSeedingProgress(row({ seedingState: "HELD_PRIVATE" }));

  assert.equal(progress?.stateKey, "HELD_PRIVATE");
  assert.equal(progress?.ratioLabel, null);
  assert.equal(progress?.seedTimeLabel, null);
});

test("ratios round to two decimals and drop trailing zeros", () => {
  assert.equal(formatSeedRatio(0.833333), "0.83");
  assert.equal(formatSeedRatio(1.5), "1.5");
  assert.equal(formatSeedRatio(2), "2");
  assert.equal(formatSeedRatio(1.999), "2");
});

test("an unusable ratio reading is dropped rather than shown as zero", () => {
  assert.equal(formatSeedRatio(null), null);
  assert.equal(formatSeedRatio(undefined), null);
  assert.equal(formatSeedRatio(Number.NaN), null);
  assert.equal(formatSeedRatio(Number.POSITIVE_INFINITY), null);
  assert.equal(formatSeedRatio(-1), null);
});

test("seed durations read coarsely, largest unit first", () => {
  assert.equal(formatSeedDuration(0), "<1m");
  assert.equal(formatSeedDuration(59), "<1m");
  assert.equal(formatSeedDuration(90), "1m");
  assert.equal(formatSeedDuration(3600), "1h");
  assert.equal(formatSeedDuration(5_460), "1h 31m");
  assert.equal(formatSeedDuration(86_400), "1d");
  assert.equal(formatSeedDuration(90_000), "1d 1h");
  assert.equal(formatSeedDuration(604_800), "7d");
});

test("an unusable duration reading is dropped rather than shown as zero", () => {
  assert.equal(formatSeedDuration(null), null);
  assert.equal(formatSeedDuration(undefined), null);
  assert.equal(formatSeedDuration(Number.NaN), null);
  assert.equal(formatSeedDuration(-30), null);
});

test("an unknown private flag is never rendered as public", () => {
  assert.equal(isPrivateTorrentRow({ isPrivate: true }), true);
  assert.equal(isPrivateTorrentRow({ isPrivate: false }), false);
  assert.equal(isPrivateTorrentRow({ isPrivate: null }), false);
});

test("imported-and-still-seeding is read off the tracked state", () => {
  assert.equal(
    isImportedSeedingRow({ trackedState: "IMPORTED_SEEDING" }),
    true,
  );
  assert.equal(isImportedSeedingRow({ trackedState: "IMPORTED" }), false);
  assert.equal(isImportedSeedingRow({ trackedState: null }), false);
});

test("every new queue seeding string is in the English locale", () => {
  for (const key of [
    "queue.state.importedSeeding",
    "queue.seeding.ratio",
    "queue.seeding.seedTime",
    "queue.seeding.private",
    "queue.seeding.privateTooltip",
  ]) {
    assert.ok(key in en, `${key} is missing from the English locale`);
  }
});
