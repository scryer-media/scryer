import assert from "node:assert/strict";
import test from "node:test";

import type {
  LibrarySettingsDraft,
  LibrarySettingsRecord,
} from "../types/titles.ts";
import {
  libraryChoiceSettingsInput,
  librarySettingsInput,
  storedLibraryOverridesInput,
} from "./library-settings-input.ts";

const settings: LibrarySettingsDraft = {
  requiredAudioLanguages: ["eng"],
  preferredAudioLanguages: ["jpn"],
  optionalAudioLanguages: ["spa"],
  optionalAudioLanguagesKeepSearching: false,
  metadataLanguage: "eng",
  useSeasonFolders: true,
  defaultMonitorType: null,
  defaultTitleTags: ["synthetic-remux"],
  qualityProfileId: "profile",
  requestQualityProfileIds: ["request-profile"],
  scoringPersona: null,
  fillerPolicy: null,
  recapPolicy: null,
  monitorSpecials: false,
  interSeasonMovies: false,
  monitorFillerMovies: false,
  nfoWriteOnImport: true,
  plexmatchWriteOnImport: null,
  importMode: "HARDLINK_OR_COPY",
  setPermissionsLinux: false,
  fileChmod: null,
  folderChmod: null,
  chownGroup: null,
  defaultMovieAvailability: "released",
  movieReleaseMarket: "US",
  movieAvailabilityDelayDays: 3,
  indexerRouting: [],
  downloadClientRouting: [],
};

// The server keeps the stored list when searchLanguages is null or omitted
// and clears it only for an empty list, so each form must reach it unchanged.
test("library mutation settings pass selected, cleared and kept release search languages through", () => {
  for (const searchLanguages of [["eng"], ["eng", "jpn"]]) {
    const draft = { ...settings, searchLanguages };
    assert.deepEqual(librarySettingsInput(draft), draft);
  }
  assert.deepEqual(
    librarySettingsInput({ ...settings, searchLanguages: [] })?.searchLanguages,
    [],
  );
  assert.equal(
    librarySettingsInput({ ...settings, searchLanguages: null })
      ?.searchLanguages,
    null,
  );
});

test("omitted settings and search languages remain omitted", () => {
  assert.equal(librarySettingsInput(undefined), undefined);
  assert.equal(librarySettingsInput(settings)?.searchLanguages, undefined);
});

test("optional movie availability fields retain their existing null defaults", () => {
  const input = librarySettingsInput({
    ...settings,
    defaultMovieAvailability: undefined,
    movieReleaseMarket: undefined,
    movieAvailabilityDelayDays: undefined,
  });
  assert.equal(input?.defaultMovieAvailability, null);
  assert.equal(input?.movieReleaseMarket, null);
  assert.equal(input?.movieAvailabilityDelayDays, null);
});

const saved: LibrarySettingsRecord = {
  searchLanguages: ["eng"],
  requiredAudioLanguagesOverride: ["jpn"],
  requiredAudioLanguages: ["jpn"],
  preferredAudioLanguagesOverride: null,
  preferredAudioLanguages: ["eng"],
  optionalAudioLanguagesOverride: null,
  optionalAudioLanguages: [],
  optionalAudioLanguagesKeepSearchingOverride: false,
  optionalAudioLanguagesKeepSearching: false,
  metadataLanguageOverride: null,
  metadataLanguage: "eng",
  useSeasonFoldersOverride: false,
  useSeasonFolders: false,
  defaultMonitorTypeOverride: "ALL_EPISODES",
  defaultTitleTags: ["synthetic-remux"],
  qualityProfileIdOverride: "saved-profile",
  qualityProfileId: "saved-profile",
  requestQualityProfileIdsOverride: ["request-profile"],
  requestQualityProfileIds: ["request-profile"],
  requestQualityProfileDefaultId: "request-profile",
  scoringPersonaOverride: null,
  scoringPersona: "BALANCED",
  fillerPolicyOverride: "SKIP_FILLER",
  fillerPolicy: "SKIP_FILLER",
  recapPolicyOverride: null,
  recapPolicy: "DOWNLOAD_ALL",
  monitorSpecialsOverride: true,
  monitorSpecials: true,
  interSeasonMoviesOverride: null,
  interSeasonMovies: false,
  monitorFillerMoviesOverride: null,
  monitorFillerMovies: false,
  nfoWriteOnImportOverride: true,
  nfoWriteOnImport: true,
  plexmatchWriteOnImportOverride: null,
  plexmatchWriteOnImport: false,
  importModeOverride: "HARDLINK_OR_COPY",
  importMode: "HARDLINK_OR_COPY",
  setPermissionsLinuxOverride: null,
  setPermissionsLinux: false,
  fileChmodOverride: "664",
  fileChmod: "664",
  folderChmodOverride: null,
  folderChmod: "775",
  chownGroupOverride: null,
  chownGroup: null,
  defaultMovieAvailabilityOverride: null,
  defaultMovieAvailability: "released",
  movieReleaseMarketOverride: "US",
  movieReleaseMarket: "US",
  movieAvailabilityDelayDaysOverride: 3,
  movieAvailabilityDelayDays: 3,
  indexerRoutingOverride: [{ indexerId: "indexer" }],
  downloadClientRoutingOverride: [],
};

test("stored overrides go back as they are, without the effective values", () => {
  const input = storedLibraryOverridesInput(saved);

  assert.deepEqual(input, {
    requiredAudioLanguages: ["jpn"],
    preferredAudioLanguages: null,
    optionalAudioLanguages: null,
    optionalAudioLanguagesKeepSearching: false,
    metadataLanguage: null,
    useSeasonFolders: false,
    defaultMonitorType: "ALL_EPISODES",
    defaultTitleTags: ["synthetic-remux"],
    qualityProfileId: "saved-profile",
    requestQualityProfileIds: ["request-profile"],
    scoringPersona: null,
    fillerPolicy: "SKIP_FILLER",
    recapPolicy: null,
    monitorSpecials: true,
    interSeasonMovies: null,
    monitorFillerMovies: null,
    nfoWriteOnImport: true,
    plexmatchWriteOnImport: null,
    importMode: "HARDLINK_OR_COPY",
    setPermissionsLinux: null,
    fileChmod: "664",
    folderChmod: null,
    chownGroup: null,
    defaultMovieAvailability: null,
    movieReleaseMarket: "US",
    movieAvailabilityDelayDays: 3,
  });
  // The server keeps these when a save leaves them out.
  for (const kept of [
    "searchLanguages",
    "indexerRouting",
    "downloadClientRouting",
  ]) {
    assert.equal(kept in input, false);
  }
});

test("a chosen profile and persona replace the stored ones and keep the rest", () => {
  const input = libraryChoiceSettingsInput(saved, {
    qualityProfileId: "chosen-profile",
    scoringPersona: "AUDIOPHILE",
  });

  assert.equal(input.qualityProfileId, "chosen-profile");
  assert.equal(input.scoringPersona, "AUDIOPHILE");
  assert.equal(input.fileChmod, "664");
  assert.equal(input.defaultMonitorType, "ALL_EPISODES");
});

test("a choice equal to the inherited value stays inherited", () => {
  const inherited: LibrarySettingsRecord = {
    ...saved,
    qualityProfileIdOverride: null,
    qualityProfileId: "global-profile",
  };

  const kept = libraryChoiceSettingsInput(inherited, {
    qualityProfileId: "global-profile",
    scoringPersona: "BALANCED",
  });
  assert.equal(kept.qualityProfileId, null);
  assert.equal(kept.scoringPersona, null);

  const changed = libraryChoiceSettingsInput(inherited, {
    qualityProfileId: "chosen-profile",
    scoringPersona: "EFFICIENT",
  });
  assert.equal(changed.qualityProfileId, "chosen-profile");
  assert.equal(changed.scoringPersona, "EFFICIENT");
});

test("a choice equal to a stored override stays an override", () => {
  const input = libraryChoiceSettingsInput(saved, {
    qualityProfileId: "saved-profile",
    scoringPersona: "BALANCED",
  });

  assert.equal(input.qualityProfileId, "saved-profile");
  assert.equal(input.scoringPersona, null);
});
