import type { Facet } from "./titles";

/** One trailing window of activity counters. */
export type DashboardActivityWindow = {
  grabbed: number;
  upgraded: number;
  imported: number;
  importFailed: number;
};

export type DashboardActivityStats = {
  current: DashboardActivityWindow;
  /** The equally long window immediately before `current`, for the deltas. */
  previous: DashboardActivityWindow;
};

export type DashboardIndexerStat = {
  indexerId: string;
  indexerName: string;
  queriesLast24H: number;
  failedLast24H: number;
  grabsLast24H: number;
  /** Null on an unmetered indexer, which renders as an infinity glyph. */
  apiCurrent: number | null;
  apiMax: number | null;
};

export type DashboardIndexer = {
  id: string;
  name: string;
  providerType: string;
  isEnabled: boolean;
  lastHealthStatus: string | null;
  lastErrorMessage: string | null;
  lastErrorAt: string | null;
  /**
   * When this indexer's rate-limit cooldown lifts, or null when it is not
   * cooling down. A cooling indexer is quiet, not broken.
   */
  rateLimitedUntil: string | null;
};

export type DashboardDownloadClient = {
  id: string;
  name: string;
  clientType: string;
  isEnabled: boolean;
  status: string | null;
  lastError: string | null;
  lastSeenAt: string | null;
};

export type DashboardStorageRoot = {
  path: string;
  libraryId: string;
  libraryName: string;
  facet: Facet;
  /** Null when the filesystem could not be inspected; never treat as zero. */
  usedBytes: number | null;
  totalBytes: number | null;
  /** The owning library's own size across all its roots, from the catalog. */
  libraryBytes: number | null;
};

export type DashboardOverview = {
  username: string | null;
  pendingRequestCount: number;
  /** Items in the Activity → Imports list: downloads that could not auto-import. */
  activityImportCount: number;
  library: {
    movies: number;
    series: number;
    anime: number;
  };
  activity: DashboardActivityStats;
  indexerStats: DashboardIndexerStat[];
  indexers: DashboardIndexer[];
  downloadClients: DashboardDownloadClient[];
  storageRoots: DashboardStorageRoot[];
};

export type DashboardRequestRequester = {
  userId: string;
  username: string;
  avatarUrl: string | null;
};

export type DashboardRequest = {
  id: string;
  libraryId: string;
  facet: Facet;
  title: string;
  year: number | null;
  posterUrl: string | null;
  requestedQualityProfileId: string | null;
  requestedMonitorType: string | null;
  createdAt: string;
  requesters: DashboardRequestRequester[];
};

/** The subset of a library the dashboard needs to approve a request. */
export type DashboardRequestLibrary = {
  id: string;
  name: string;
  facet: Facet;
  qualityProfileId: string | null;
  requestQualityProfileDefaultId: string | null;
};

export type DashboardImportedItem = {
  id: string;
  titleId: string;
  titleName: string | null;
  posterUrl: string | null;
  facet: string | null;
  libraryId: string | null;
  kind: "IMPORTED" | "NEW_IMPORT" | "UPGRADE";
  episode: {
    id: string;
    seasonNumber: string | null;
    episodeNumber: string | null;
    title: string | null;
    overview: string | null;
    imageUrl: string | null;
    airDate: string | null;
  } | null;
  quality: string | null;
  sizeBytes: number | null;
  occurredAt: string;
};

export type DashboardPluginUpdate = {
  id: string;
  name: string;
  fromVersion: string | null;
  toVersion: string | null;
  breaking: boolean;
};
