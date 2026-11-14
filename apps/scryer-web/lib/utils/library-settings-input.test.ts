import assert from "node:assert/strict";
import test from "node:test";

import type {
  LibrarySettingsDraft,
  LibrarySettingsRecord,
} from "../types/titles.ts";
import {
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

test("stored overrides go back as they are, without the effective values", () => {
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

  const input = storedLibraryOverridesInput(saved);

  assert.deepEqual(input, {
    requiredAudioLanguages: ["jpn"],
    preferredAudioLanguages: null,
    optionalAudioLanguages: null,
    optionalAudioLanguagesKeepSearching: false,
    metadataLanguage: null,
    useSeasonFolders: false,
    defaultMonitorType: "ALL_EPISODES",
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
