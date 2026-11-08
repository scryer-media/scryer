import assert from "node:assert/strict";
import test from "node:test";

import type { LibrarySettingsDraft } from "../types/titles.ts";
import { librarySettingsInput } from "./library-settings-input.ts";

const settings: LibrarySettingsDraft = {
  requiredAudioLanguages: ["eng"],
  metadataLanguage: "eng",
  useSeasonFolders: true,
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

test("library mutation settings preserve selected and cleared release search languages", () => {
  for (const searchLanguages of [["eng"], ["eng", "jpn"], [], null]) {
    const draft = { ...settings, searchLanguages };
    assert.deepEqual(librarySettingsInput(draft), draft);
  }
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
