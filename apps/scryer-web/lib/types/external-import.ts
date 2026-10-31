// Result-type companions for the multi-instance external-import flow.
// Keep in sync with the payload types in
// crates/scryer-interface-media-types/src/lib.rs and the selection sets in
// lib/graphql/{mutations,queries}.ts.

export type ExternalArrSourceKind = "SONARR" | "RADARR";
export type ExternalImportConnectionKind = "SONARR" | "RADARR" | "PROWLARR";

export type ExternalImportConnection = {
  baseUrl: string;
  apiKey: string;
};

export type ExternalImportConnectionValidation = {
  kind: ExternalImportConnectionKind;
  baseUrl: string;
  connected: boolean;
  version: string | null;
  error: string | null;
};

/** One warmed Sonarr/Radarr instance as reflected by `previewExternalImport`. */
export type ExternalImportArrSource = {
  sessionId: string;
  sourceKey: string;
  kind: ExternalArrSourceKind;
  baseUrl: string;
  connected: boolean;
  version: string | null;
  status: ExternalImportMonitorWarmupStatus;
  error: string | null;
};

/** A root folder discovered during a Sonarr/Radarr warmup. */
export type ExternalImportRootFolder = {
  sourceWarmupSessionId: string;
  sourceKey: string;
  kind: ExternalArrSourceKind;
  arrRootPath: string;
};

export type ExternalImportDownloadClient = {
  sourceKeys: string[];
  name: string;
  implementation: string;
  scryerClientType: string | null;
  host: string | null;
  port: string | null;
  useSsl: boolean;
  urlBase: string | null;
  username: string | null;
  apiKeyPresent: boolean;
  dedupKey: string;
  supported: boolean;
  requiresPasswordOverride: boolean;
};

export type ExternalImportIndexer = {
  sourceKeys: string[];
  name: string;
  implementation: string;
  scryerProviderType: string | null;
  baseUrl: string | null;
  apiKeyPresent: boolean;
  dedupKey: string;
  supported: boolean;
  childCount: number;
  childNames: string[];
  requiresApiKeyOverride: boolean;
  apiKeyHelpUrl: string | null;
};

export type ExternalImportPreview = {
  prowlarrConnected: boolean;
  prowlarrVersion: string | null;
  prowlarrError: string | null;
  arrSources: ExternalImportArrSource[];
  rootFolders: ExternalImportRootFolder[];
  downloadClients: ExternalImportDownloadClient[];
  indexers: ExternalImportIndexer[];
};

export type ExternalImportResult = {
  mediaPathsSaved: boolean;
  downloadClientsCreated: number;
  indexersCreated: number;
  pluginsInstalled: string[];
  errors: string[];
};

export type ExternalImportMonitorWarmupStatus =
  "QUEUED" | "RUNNING" | "COMPLETED" | "CANCELED" | "FAILED";

export type ExternalImportMonitorWarmupPhase =
  | "LOADING_INDEXERS"
  | "LOADING_MOVIES"
  | "LOADING_SERIES"
  | "LOADING_EPISODES"
  | "BUILDING_SNAPSHOT"
  | "READY";

export type ExternalImportMonitorWarmupPhaseProgress = {
  total: number;
  completed: number;
  failed: number;
};

export type ExternalImportMonitorWarmupProgress = {
  sessionId: string;
  status: ExternalImportMonitorWarmupStatus;
  phase: ExternalImportMonitorWarmupPhase;
  startedAt: string;
  updatedAt: string;
  overallTotalKnown: boolean;
  overallProgress: ExternalImportMonitorWarmupPhaseProgress;
  moviesTotalKnown: boolean;
  moviesProgress: ExternalImportMonitorWarmupPhaseProgress;
  seriesTotalKnown: boolean;
  seriesProgress: ExternalImportMonitorWarmupPhaseProgress;
  episodeFetchTotalKnown: boolean;
  episodeFetchExpectedTotal: number | null;
  episodeFetchExpectedMonitoredTotal: number | null;
  episodeFetchProgress: ExternalImportMonitorWarmupPhaseProgress;
  snapshotBuildTotalKnown: boolean;
  snapshotBuildProgress: ExternalImportMonitorWarmupPhaseProgress;
  matchedMovieCount: number;
  matchedSeriesCount: number;
  unmatchedMovieCount: number;
  unmatchedSeriesCount: number;
  ambiguousMovieCount: number;
  ambiguousSeriesCount: number;
  errorMessage: string | null;
};

/** Aggregated title-fetch progress across all per-instance warmup sessions. */
export type ExternalImportAggregateWarmupProgress = {
  status: ExternalImportMonitorWarmupStatus;
  titlesTotalKnown: boolean;
  titlesFetched: number;
  titlesTotal: number;
  errorMessage: string | null;
};
