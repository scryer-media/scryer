import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import type { SeedingProfileDraft } from "../types/seeding-profiles.ts";
import {
  parseMinimumSeeders,
  seedingProfileDraftToInput,
  buildSeedingProfileTemplate,
  extractSeedingProfileErrorMessage,
  formatSeasonPackSummary,
  formatSeedDuration,
  handsOffAfterImport,
  parseSeedDuration,
  POST_IMPORT_TRACKING_MODES,
  formatSeedingProfileRatio,
  formatSeedingProfileSeedTime,
  SEEDING_PROFILE_INHERIT_VALUE,
  seedingProfileInheritOptionKey,
  seedingProfileMinimumSeedersChip,
  seedingProfileSelectValue,
  seedingProfileSelectValueToId,
  seedingProfileToDraft,
  supportsSeedingProfileAssignment,
  toCreateSeedingProfileInput,
  toUpdateSeedingProfileInput,
  validateSeedingProfileDraft,
  validateSeedingProfileFields,
} from "./seeding-profiles.ts";

function draft(
  overrides: Partial<SeedingProfileDraft> = {},
): SeedingProfileDraft {
  return { ...buildSeedingProfileTemplate(), name: "Private", ...overrides };
}

test("template opts in to nothing but the safe defaults", () => {
  const template = buildSeedingProfileTemplate();
  assert.equal(template.ratio, "");
  assert.equal(template.seedTimeMinutes, "");
  assert.equal(template.seasonPackMode, "INHERIT");
  assert.equal(template.honorTrackerMinimums, true);
  assert.equal(template.goalMetAction, "REMOVE_ENTRY");
  assert.equal(template.neverRemove, false);
  // Parking is the fail-closed default: Scryer keeps managing the torrent.
  assert.equal(template.postImportTracking, "PARK");
});

test("post-import tracking round-trips and drives the moot-field helper", () => {
  assert.deepEqual([...POST_IMPORT_TRACKING_MODES], ["PARK", "HAND_OFF"]);
  assert.equal(handsOffAfterImport("HAND_OFF"), true);
  assert.equal(handsOffAfterImport("PARK"), false);

  const input = toCreateSeedingProfileInput(
    draft({ postImportTracking: "HAND_OFF" }),
  );
  assert.equal(input.postImportTracking, "HAND_OFF");
  assert.equal(toCreateSeedingProfileInput(draft()).postImportTracking, "PARK");
});

test("a stored profile round-trips through the draft", () => {
  const record = {
    id: "profile-1",
    name: "Private",
    ratio: 1.5,
    seedTimeMinutes: 4320,
    seasonPackMode: "OVERRIDE" as const,
    seasonPackRatio: 2,
    seasonPackSeedTimeMinutes: 10080,
    honorTrackerMinimums: false,
    goalMetAction: "STOP_SEEDING" as const,
    neverRemove: true,
    minimumSeeders: 5,
    postImportTracking: "HAND_OFF" as const,
  };

  const asDraft = seedingProfileToDraft(record);
  assert.equal(asDraft.ratio, "1.5");
  // Stored minutes come back as the duration syntax the field accepts.
  assert.equal(asDraft.seedTimeMinutes, "3d");
  assert.equal(asDraft.seasonPackSeedTimeMinutes, "1w");
  assert.equal(asDraft.minimumSeeders, "5");

  assert.deepEqual(toUpdateSeedingProfileInput(asDraft), {
    id: "profile-1",
    name: "Private",
    ratio: 1.5,
    seedTimeMinutes: 4320,
    seasonPackMode: "OVERRIDE",
    seasonPackRatio: 2,
    seasonPackSeedTimeMinutes: 10080,
    honorTrackerMinimums: false,
    goalMetAction: "STOP_SEEDING",
    neverRemove: true,
    minimumSeeders: 5,
    postImportTracking: "HAND_OFF",
  });
});

test("empty goal fields become nulls so the client's own limits apply", () => {
  const input = toCreateSeedingProfileInput(draft());
  assert.equal(input.ratio, null);
  assert.equal(input.seedTimeMinutes, null);
  assert.equal(input.seasonPackRatio, null);
  assert.equal(input.seasonPackSeedTimeMinutes, null);
});

test("inherit mode drops season-pack goals, matching server normalization", () => {
  const input = toCreateSeedingProfileInput(
    draft({
      seasonPackMode: "INHERIT",
      seasonPackRatio: "3",
      seasonPackSeedTimeMinutes: "60",
    }),
  );
  assert.equal(input.seasonPackRatio, null);
  assert.equal(input.seasonPackSeedTimeMinutes, null);
});

test("override mode keeps season-pack goals", () => {
  const input = toCreateSeedingProfileInput(
    draft({
      seasonPackMode: "OVERRIDE",
      seasonPackRatio: "3",
      seasonPackSeedTimeMinutes: "60",
    }),
  );
  assert.equal(input.seasonPackRatio, 3);
  assert.equal(input.seasonPackSeedTimeMinutes, 60);
});

test("validation rejects what the backend rejects", () => {
  assert.equal(validateSeedingProfileDraft(draft()), null);
  assert.match(
    validateSeedingProfileDraft(draft({ name: "  " })) ?? "",
    /Enter a name/,
  );
  for (const ratio of ["0", "-1", "abc"]) {
    assert.match(
      validateSeedingProfileDraft(draft({ ratio })) ?? "",
      /number greater than zero/,
      ratio,
    );
  }
  assert.match(
    validateSeedingProfileDraft(draft({ seedTimeMinutes: "1.5" })) ?? "",
    /duration like/,
  );
  assert.equal(validateSeedingProfileDraft(draft({ ratio: "1.25" })), null);
});

test("each bad field reports under itself, not as one banner message", () => {
  const errors = validateSeedingProfileFields(
    draft({
      name: "  ",
      ratio: "abc",
      seedTimeMinutes: "1.5",
      seasonPackMode: "OVERRIDE",
      seasonPackRatio: "nope",
      seasonPackSeedTimeMinutes: "later",
    }),
  );
  assert.deepEqual(Object.keys(errors).sort(), [
    "name",
    "ratio",
    "seasonPackRatio",
    "seasonPackSeedTimeMinutes",
    "seedTimeMinutes",
  ]);

  // A valid draft leaves every field clean.
  assert.deepEqual(validateSeedingProfileFields(draft()), {});
  // One bad field does not implicate its neighbours.
  assert.deepEqual(
    Object.keys(validateSeedingProfileFields(draft({ ratio: "x" }))),
    ["ratio"],
  );
});

test("season-pack goals are only validated in override mode", () => {
  assert.equal(
    validateSeedingProfileDraft(
      draft({ seasonPackMode: "INHERIT", seasonPackRatio: "nonsense" }),
    ),
    null,
  );
  assert.match(
    validateSeedingProfileDraft(
      draft({ seasonPackMode: "OVERRIDE", seasonPackRatio: "nonsense" }),
    ) ?? "",
    /number greater than zero/,
  );
});

test("blocked-delete messages are surfaced verbatim", () => {
  const blocked =
    "seeding profile is still assigned to indexer 'Private', the global default seeding profile";
  assert.equal(
    extractSeedingProfileErrorMessage({
      graphQLErrors: [{ message: blocked }],
    }),
    blocked,
  );
  assert.equal(
    extractSeedingProfileErrorMessage(new Error("network down")),
    "network down",
  );
  assert.equal(extractSeedingProfileErrorMessage({}), null);
});

test("null assignments map to the inherit sentinel and back", () => {
  assert.equal(seedingProfileSelectValue(null), SEEDING_PROFILE_INHERIT_VALUE);
  assert.equal(seedingProfileSelectValue("profile-1"), "profile-1");
  assert.equal(
    seedingProfileSelectValueToId(SEEDING_PROFILE_INHERIT_VALUE),
    null,
  );
  assert.equal(seedingProfileSelectValueToId("profile-1"), "profile-1");
});

test("only torrent-capable indexers accept a seeding profile", () => {
  assert.equal(supportsSeedingProfileAssignment(["torrent"]), true);
  assert.equal(supportsSeedingProfileAssignment(["Torrent", "usenet"]), true);
  assert.equal(supportsSeedingProfileAssignment(["usenet"]), false);
  assert.equal(supportsSeedingProfileAssignment([]), false);
  assert.equal(supportsSeedingProfileAssignment(undefined), false);
});

test("table cells fall back to a dash when a goal defers to the client", () => {
  assert.equal(formatSeedingProfileRatio(null), "—");
  assert.equal(formatSeedingProfileRatio(1.5), "1.5");
  assert.equal(formatSeedingProfileSeedTime(null), "—");
  assert.equal(formatSeedingProfileSeedTime(60), "1h");
  assert.equal(
    formatSeasonPackSummary(
      {
        seasonPackMode: "INHERIT",
        seasonPackRatio: 2,
        seasonPackSeedTimeMinutes: 60,
      },
      "Inherit",
    ),
    "Inherit",
  );
  assert.equal(
    formatSeasonPackSummary(
      {
        seasonPackMode: "OVERRIDE",
        seasonPackRatio: 2,
        seasonPackSeedTimeMinutes: 60,
      },
      "Inherit",
    ),
    "2 / 1h",
  );
  assert.equal(
    formatSeasonPackSummary(
      {
        seasonPackMode: "OVERRIDE",
        seasonPackRatio: null,
        seasonPackSeedTimeMinutes: null,
      },
      "Inherit",
    ),
    "—",
  );
});

test("every seeding-profile string the UI renders has an English entry", () => {
  const requiredKeys = [
    "settings.seedingProfiles",
    "settings.seedingProfileExisting",
    "settings.seedingProfileNone",
    "settings.seedingProfileCreate",
    "settings.seedingProfileCreateNew",
    "settings.seedingProfileEdit",
    "settings.seedingProfileConfirmDiscardTitle",
    "settings.seedingProfileConfirmDiscardDescription",
    "settings.seedingProfileDeleteConfirm",
    "settings.seedingProfileNameLabel",
    "settings.seedingProfileNamePlaceholder",
    "settings.seedingProfileGoalPlaceholder",
    "settings.seedingProfileRatioLabel",
    "settings.seedingProfileRatioHelp",
    "settings.seedingProfileSeedTimeLabel",
    "settings.seedingProfileSeedTimeHelp",
    "settings.seedingProfileSeedTimeTransmissionHelp",
    "settings.seedingProfileSeasonPackAdvanced",
    "settings.seedingProfileSeasonPackModeLabel",
    "settings.seedingProfileSeasonPackModeHelp",
    "settings.seedingProfileSeasonPackInherit",
    "settings.seedingProfileSeasonPackOverride",
    "settings.seedingProfileSeasonPackRatioLabel",
    "settings.seedingProfileSeasonPackSeedTimeLabel",
    "settings.seedingProfileSeasonPackGoalsHelp",
    "settings.seedingProfileHonorTrackerMinimumsLabel",
    "settings.seedingProfileHonorTrackerMinimumsHelp",
    "settings.seedingProfileGoalMetActionLabel",
    "settings.seedingProfileGoalMetActionHelp",
    "settings.seedingProfileGoalMetRemoveEntry",
    "settings.seedingProfileGoalMetStopSeeding",
    "settings.seedingProfileGoalMetKeep",
    "settings.seedingProfileNeverRemoveLabel",
    "settings.seedingProfileNeverRemoveHelp",
    "settings.seedingProfileNeverRemoveBadge",
    "settings.seedingProfilePostImportTrackingLabel",
    "settings.seedingProfilePostImportTrackingHelp",
    "settings.seedingProfilePostImportTrackingPark",
    "settings.seedingProfilePostImportTrackingHandOff",
    "settings.seedingProfilePostImportTrackingHandOffBadge",
    "settings.seedingProfilePostImportTrackingHandOffHelp",
    "settings.seedingProfileDefaultTitle",
    "settings.seedingProfileDefaultLabel",
    "settings.seedingProfileDefaultHelp",
    "settings.seedingProfileDefaultNone",
    "settings.seedingProfileDefaultBadge",
    "settings.seedingProfileMissing",
    "settings.seedingProfileSaved",
    "settings.seedingProfileSaveError",
    "settings.seedingProfileDeleted",
    "settings.seedingProfileDeleteError",
    "settings.seedingProfileDefaultSaved",
    "settings.seedingProfileInherit",
    "settings.seedingProfileRoutingInherit",
    "settings.seedingProfileColumn",
    "settings.seedingProfileIndexerLabel",
    "settings.seedingProfileRoutingLabel",
    "settings.seedingProfileNotApplicable",
    "settings.seedingProfileMinimumSeedersLabel",
    "settings.seedingProfileMinimumSeedersHelp",
    "settings.seedingProfileMinimumSeedersUnknownHelp",
    "settings.seedingProfileMinimumSeedersRoutingHelp",
    "settings.seedingProfileMinimumSeedersInheritHelp",
    "settings.seedingProfileMinimumSeedersPlaceholder",
    "settings.seedingProfileMinimumSeedersOffBadge",
    "settings.seedingProfileMinimumSeedersBadge",
    "status.indexerSeedingProfileSaving",
    "status.indexerSeedingProfileSaved",
  ];

  for (const key of requiredKeys) {
    assert.equal(typeof en[key], "string", `missing English string for ${key}`);
  }
});

test("seed-time durations parse into the minutes the API stores", () => {
  const minutes = (raw: string) => {
    const parsed = parseSeedDuration(raw);
    assert.equal(parsed.ok, true, raw);
    return parsed.ok ? parsed.value : null;
  };

  // A bare number stays minutes: it is what the field took before, and what
  // trackers usually quote.
  assert.equal(minutes("90"), 90);
  assert.equal(minutes("5m"), 5);
  assert.equal(minutes("36h"), 2_160);
  assert.equal(minutes("1d"), 1_440);
  assert.equal(minutes("2w"), 20_160);
  assert.equal(minutes("1d 12h"), 2_160);
  assert.equal(minutes("1d12h"), 2_160);
  assert.equal(minutes("2W"), 20_160);
  assert.equal(minutes(""), null);
  assert.equal(minutes("  "), null);

  for (const bad of [
    "0",
    "-5",
    "2x",
    "2w rubbish",
    "rubbish",
    "1.5d",
    "1d 1d",
    "0m",
  ]) {
    assert.equal(parseSeedDuration(bad).ok, false, bad);
  }
});

test("minutes render back into the duration syntax the field accepts", () => {
  assert.equal(formatSeedDuration(5), "5m");
  assert.equal(formatSeedDuration(60), "1h");
  assert.equal(formatSeedDuration(90), "1h 30m");
  assert.equal(formatSeedDuration(1_440), "1d");
  assert.equal(formatSeedDuration(2_160), "1d 12h");
  assert.equal(formatSeedDuration(20_160), "2w");
  assert.equal(formatSeedDuration(null), "");
  assert.equal(formatSeedDuration(0), "");
});

test("every duration the formatter emits parses back to the same minutes", () => {
  for (const value of [1, 59, 60, 90, 1_439, 1_440, 2_160, 10_080, 20_161]) {
    const parsed = parseSeedDuration(formatSeedDuration(value));
    assert.equal(parsed.ok && parsed.value, value, String(value));
  }
});

test("minimum seeders distinguishes empty from zero", () => {
  // Empty means "inherit the system floor"; 0 means "turn the check off for
  // this profile". Collapsing them would silently re-enable a check the
  // operator disabled.
  assert.deepEqual(parseMinimumSeeders(""), { ok: true, value: null });
  assert.deepEqual(parseMinimumSeeders("   "), { ok: true, value: null });
  assert.deepEqual(parseMinimumSeeders("0"), { ok: true, value: 0 });
  assert.deepEqual(parseMinimumSeeders("3"), { ok: true, value: 3 });
  assert.deepEqual(parseMinimumSeeders("-1"), { ok: false });
  assert.deepEqual(parseMinimumSeeders("1.5"), { ok: false });
  assert.deepEqual(parseMinimumSeeders("many"), { ok: false });
});

test("the behavior cell only inks a minimum that deviates from the floor", () => {
  // Inheriting says nothing the floor field does not already say.
  assert.equal(seedingProfileMinimumSeedersChip(null, 1), null);
  assert.equal(seedingProfileMinimumSeedersChip(1, 1), null);

  // Explicitly off stays visible even when the floor itself is off: it is an
  // opt-out that survives the floor being raised again.
  assert.deepEqual(seedingProfileMinimumSeedersChip(0, 1), {
    kind: "disabled",
  });
  assert.deepEqual(seedingProfileMinimumSeedersChip(0, 0), {
    kind: "disabled",
  });

  // Tighter and looser than the floor both deviate; the looser one is the
  // reason this is not a `> floor` test.
  assert.deepEqual(seedingProfileMinimumSeedersChip(5, 1), {
    kind: "threshold",
    value: 5,
  });
  assert.deepEqual(seedingProfileMinimumSeedersChip(1, 3), {
    kind: "threshold",
    value: 1,
  });
});

test("a profile minimum of zero round-trips as zero, not as empty", () => {
  const draft = {
    ...buildSeedingProfileTemplate(),
    name: "Public tracker",
    minimumSeeders: "0",
  };
  assert.equal(seedingProfileDraftToInput(draft).minimumSeeders, 0);
});

test("the inherit option names Prowlarr's imported minimum", () => {
  // Without a value the option keeps the two labels it had: the sentinel is
  // about seed goals, and W1 deliberately stopped it claiming a minimum.
  assert.equal(
    seedingProfileInheritOptionKey(false, null),
    "settings.seedingProfileInherit",
  );
  assert.equal(
    seedingProfileInheritOptionKey(true, null),
    "settings.seedingProfileProwlarrManaged",
  );
  assert.equal(
    seedingProfileInheritOptionKey(false, undefined),
    "settings.seedingProfileInherit",
  );

  // A live Prowlarr always pushes a minimum (its app profile defaults to 1),
  // so the singular is the common case, not an edge one.
  assert.equal(
    seedingProfileInheritOptionKey(false, 1),
    "settings.seedingProfileInheritProwlarrMinimumOne",
  );
  assert.equal(
    seedingProfileInheritOptionKey(false, 4),
    "settings.seedingProfileInheritProwlarrMinimum",
  );
  assert.equal(
    seedingProfileInheritOptionKey(true, 4),
    "settings.seedingProfileProwlarrManagedMinimum",
  );
  assert.equal(
    seedingProfileInheritOptionKey(true, 1),
    "settings.seedingProfileProwlarrManagedMinimumOne",
  );

  // Zero is Prowlarr's explicit "do not enforce", so it must not read as a
  // threshold of zero seeders nor fall back to "Inherit default".
  assert.equal(
    seedingProfileInheritOptionKey(false, 0),
    "settings.seedingProfileInheritProwlarrMinimumOff",
  );
  assert.equal(
    seedingProfileInheritOptionKey(true, 0),
    "settings.seedingProfileProwlarrManagedMinimumOff",
  );

  for (const managed of [false, true]) {
    for (const minimum of [null, 0, 1, 4]) {
      const key = seedingProfileInheritOptionKey(managed, minimum);
      assert.equal(
        typeof en[key],
        "string",
        `missing English string for ${key}`,
      );
    }
  }
});
