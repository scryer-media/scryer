import type { LibrarySettingsDraft } from "../types/titles.ts";

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
    audioLanguagePriority: settings.audioLanguagePriority,
    optionalAudioLanguagesKeepSearching:
      settings.optionalAudioLanguagesKeepSearching,
    metadataLanguage: settings.metadataLanguage,
    useSeasonFolders: settings.useSeasonFolders,
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
