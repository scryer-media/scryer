import type {
  LibrarySettingsDraft,
  LibrarySettingsRecord,
} from "../types/titles.ts";

// Every draft field must be named here, optional ones included, so a field
// added to LibrarySettingsDraft without being mapped fails the typecheck
// instead of being silently dropped from the saved settings.
type CompleteLibrarySettingsInput = {
  [K in keyof Required<LibrarySettingsDraft>]: LibrarySettingsDraft[K];
};

export function librarySettingsInput(
  settings: LibrarySettingsDraft | undefined,
): LibrarySettingsDraft | undefined {
  if (!settings) {
    return undefined;
  }
  const input: CompleteLibrarySettingsInput = {
    searchLanguages: settings.searchLanguages,
    requiredAudioLanguages: settings.requiredAudioLanguages,
    preferredAudioLanguages: settings.preferredAudioLanguages,
    optionalAudioLanguages: settings.optionalAudioLanguages,
    optionalAudioLanguagesKeepSearching:
      settings.optionalAudioLanguagesKeepSearching,
    metadataLanguage: settings.metadataLanguage,
    useSeasonFolders: settings.useSeasonFolders,
    defaultMonitorType: settings.defaultMonitorType ?? null,
    qualityProfileId: settings.qualityProfileId,
    requestQualityProfileIds: settings.requestQualityProfileIds,
    scoringPersona: settings.scoringPersona,
    fillerPolicy: settings.fillerPolicy,
    recapPolicy: settings.recapPolicy,
    monitorSpecials: settings.monitorSpecials,
    interSeasonMovies: settings.interSeasonMovies,
    monitorFillerMovies: settings.monitorFillerMovies,
    nfoWriteOnImport: settings.nfoWriteOnImport,
    plexmatchWriteOnImport: settings.plexmatchWriteOnImport,
    importMode: settings.importMode,
    setPermissionsLinux: settings.setPermissionsLinux,
    fileChmod: settings.fileChmod,
    folderChmod: settings.folderChmod,
    chownGroup: settings.chownGroup,
    defaultMovieAvailability: settings.defaultMovieAvailability ?? null,
    movieReleaseMarket: settings.movieReleaseMarket ?? null,
    movieAvailabilityDelayDays: settings.movieAvailabilityDelayDays ?? null,
    indexerRouting: settings.indexerRouting,
    downloadClientRouting: settings.downloadClientRouting,
  };
  return input;
}

// The server keeps these three when a save leaves them out, so a save that
// only carries the stored overrides back does not name them.
type KeptWhenOmitted =
  | "searchLanguages"
  | "indexerRouting"
  | "downloadClientRouting";

type CompleteStoredOverrides = {
  [K in keyof Required<
    Omit<LibrarySettingsDraft, KeptWhenOmitted>
  >]: LibrarySettingsDraft[K];
};

/**
 * The settings input that saves a library's stored overrides back unchanged.
 * A library save that names `settings` replaces every override, so a caller
 * that changes only some of them starts from this and overwrites its own.
 */
export function storedLibraryOverridesInput(
  saved: LibrarySettingsRecord,
): LibrarySettingsDraft {
  const input: CompleteStoredOverrides = {
    requiredAudioLanguages: saved.requiredAudioLanguagesOverride,
    preferredAudioLanguages: saved.preferredAudioLanguagesOverride,
    optionalAudioLanguages: saved.optionalAudioLanguagesOverride,
    optionalAudioLanguagesKeepSearching:
      saved.optionalAudioLanguagesKeepSearchingOverride,
    metadataLanguage: saved.metadataLanguageOverride,
    useSeasonFolders: saved.useSeasonFoldersOverride,
    defaultMonitorType: saved.defaultMonitorTypeOverride ?? null,
    qualityProfileId: saved.qualityProfileIdOverride,
    requestQualityProfileIds: saved.requestQualityProfileIdsOverride,
    scoringPersona: saved.scoringPersonaOverride,
    fillerPolicy:
      saved.fillerPolicyOverride as LibrarySettingsDraft["fillerPolicy"],
    recapPolicy:
      saved.recapPolicyOverride as LibrarySettingsDraft["recapPolicy"],
    monitorSpecials: saved.monitorSpecialsOverride,
    interSeasonMovies: saved.interSeasonMoviesOverride,
    monitorFillerMovies: saved.monitorFillerMoviesOverride,
    nfoWriteOnImport: saved.nfoWriteOnImportOverride,
    plexmatchWriteOnImport: saved.plexmatchWriteOnImportOverride,
    importMode: saved.importModeOverride,
    setPermissionsLinux: saved.setPermissionsLinuxOverride,
    fileChmod: saved.fileChmodOverride,
    folderChmod: saved.folderChmodOverride,
    chownGroup: saved.chownGroupOverride,
    defaultMovieAvailability: saved.defaultMovieAvailabilityOverride ?? null,
    movieReleaseMarket: saved.movieReleaseMarketOverride ?? null,
    movieAvailabilityDelayDays: saved.movieAvailabilityDelayDaysOverride ?? null,
  };
  return input;
}

/**
 * The settings input for a save that chooses a library's quality profile and
 * scoring persona and leaves its other stored overrides as they are. A choice
 * equal to the value the library already inherits stays inherited.
 */
export function libraryChoiceSettingsInput(
  saved: LibrarySettingsRecord,
  chosen: { qualityProfileId: string | null; scoringPersona: string },
): LibrarySettingsDraft {
  return {
    ...storedLibraryOverridesInput(saved),
    qualityProfileId:
      saved.qualityProfileIdOverride === null &&
      chosen.qualityProfileId === saved.qualityProfileId
        ? null
        : chosen.qualityProfileId,
    scoringPersona:
      saved.scoringPersonaOverride === null &&
      chosen.scoringPersona === saved.scoringPersona
        ? null
        : chosen.scoringPersona,
  };
}
