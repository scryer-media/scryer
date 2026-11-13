export type TitleOptionUpdates = {
  minAvailability?: string | null;
  searchLanguages?: string[] | null;
  searchAliases?: string[] | null;
  metadataLanguage?: string | null;
  qualityProfileId?: string | null;
  rootFolderId?: string | null;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
  releaseNumbering?: string | null;
  /** The title's own preferred audio ladder; null or an empty list inherits. */
  preferredAudioLanguages?: string[] | null;
};
