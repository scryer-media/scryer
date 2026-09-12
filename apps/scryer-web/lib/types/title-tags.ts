/// Administrator-defined title tags. The registry in Settings is the only
/// source of labels a user may apply, so every picker in the app reads this
/// shape rather than inventing free text.

export type TitleTagDefinition = {
  id: string;
  label: string;
  description: string | null;
  titleCount: number;
  /// Series movies carrying the label. Counted apart from `titleCount` because
  /// a series movie is a link inside a series rather than a title of its own.
  seriesMovieCount: number;
  createdAt: string;
};

/// What a rename or a delete rewrote, and what it could not rewrite. Rule
/// sources are immutable revisions, so the last four counts are warnings
/// rather than results.
export type TitleTagRewriteCounts = {
  titles: number;
  seriesMovies: number;
  delayProfiles: number;
  maintenanceRuleSets: number;
  releaseRuleSets: number;
  managedTagFilters: number;
  requestRuleSets: number;
};

export type TitleTagDefinitionMutationResult = {
  definition: TitleTagDefinition;
  counts: TitleTagRewriteCounts;
};

export type TitleTagDefinitionDeletionResult = {
  id: string;
  label: string;
  counts: TitleTagRewriteCounts;
};

/// The patch `updateTitleTags` takes. Reserved `scryer:` entries never appear
/// in either list; the backend leaves them exactly as they were.
export type TitleTagsDelta = {
  add: string[];
  remove: string[];
};

/// Editor state for one registry row.
export type TitleTagDefinitionDraft = {
  id: string;
  label: string;
  description: string;
};

/// The two registry-backed pickers in the bulk edit dialog.
export type BulkTitleTagsDraft = {
  add: string[];
  remove: string[];
};
