export type WantedMediaType = "MOVIE" | "EPISODE" | "SERIES_MOVIE";
export type WantedStatus = "WANTED" | "GRABBED" | "PAUSED" | "COMPLETED";
export type ConvergenceState = "QUEUED" | "SEARCHING" | "CONVERGED" | "DEFERRED";
export type RecencyLane = "HOT" | "COLD";
export type WantedKind = "MISSING" | "CUTOFF_UPGRADE";
export type PendingReleaseStatus =
  | "WAITING"
  | "STANDBY"
  | "PROCESSING"
  | "GRABBED"
  | "SUPERSEDED"
  | "EXPIRED"
  | "DISMISSED"
  | "NEEDS_REVIEW";

export type WantedItem = {
  id: string;
  titleId: string;
  titleName: string | null;
  titleSlug: string | null;
  titleFacet: string | null;
  libraryId: string | null;
  libraryName: string | null;
  librarySlug: string | null;
  episodeId: string | null;
  collectionId: string | null;
  seasonNumber: string | null;
  episodeNumber: string | null;
  mediaType: WantedMediaType;
  lastSearchAt: string | null;
  status: WantedStatus;
  grabbedRelease: string | null;
  sourceProvider: string | null;
  currentScore: number | null;
  latestReleaseDecision?: {
    decisionCode: string;
    createdAt: string;
  } | null;
  standbyCount: number;
  mismatchRecoveryEligible?: boolean;
  convergenceState: ConvergenceState;
  indexersCovered: number;
  indexersRouted: number;
  recencyLane: RecencyLane;
  createdAt: string;
  updatedAt: string;
};

// Progress snapshot of the server-side interactive acquisition-search job
// — survives navigation/refresh, polled by id.
export type AcquisitionSearchJob = {
  id: string;
  state: "RUNNING" | "COMPLETED" | "CANCELLED" | "FAILED";
  total: number;
  processed: number;
  grabbedCount: number;
  failedCount: number;
  currentTitle: string | null;
  startedAt: string;
  finishedAt: string | null;
};

export type PendingReleaseItem = {
  id: string;
  wantedItemId: string;
  titleId: string;
  releaseTitle: string;
  releaseUrl: string | null;
  releaseSizeBytes: number | null;
  releaseScore: number;
  scoringLogJson: unknown;
  indexerSource: string | null;
  publishedAt: string | null;
  seeders: number | null;
  addedAt: string;
  delayUntil: string;
  lastDecisionCode: string | null;
  role: "PRIMARY" | "FALLBACK";
  status: PendingReleaseStatus;
};

export type ReleaseDecisionItem = {
  id: string;
  wantedItemId: string;
  titleId: string;
  releaseTitle: string;
  releaseUrl: string | null;
  releaseSizeBytes: number | null;
  decisionCode: string;
  candidateScore: number;
  currentScore: number | null;
  scoreDelta: number | null;
  explanationJson: unknown;
  createdAt: string;
};

export type TitleAcquisitionDiagnostics = {
  recentDecisions: ReleaseDecisionItem[];
  decisionCounts: { code: string; count: number }[];
  wantedStatusCounts: { status: WantedStatus; count: number }[];
  pendingReleaseCounts: { status: PendingReleaseStatus; count: number }[];
  mismatchRecoveryEligibleCount: number;
  latestDecisionAt: string | null;
  latestWantedSearchAt: string | null;
};
