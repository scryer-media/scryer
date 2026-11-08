import type { LibrarySettingsDraft } from "../types/titles.ts";

export function librarySettingsInput(
  settings: LibrarySettingsDraft | undefined,
): LibrarySettingsDraft | undefined {
  if (!settings) {
    return undefined;
  }
  return {
    searchLanguages: settings.searchLanguages,
    requiredAudioLanguages: settings.requiredAudioLanguages,
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
}
