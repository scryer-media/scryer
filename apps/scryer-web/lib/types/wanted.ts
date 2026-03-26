export type WantedMediaType = "movie" | "episode" | "interstitial_movie";
export type WantedSearchPhase = "pre_air" | "pre_release" | "primary" | "secondary" | "long_tail";
export type WantedStatus = "wanted" | "grabbed" | "paused" | "completed";
export type PendingReleaseStatus =
  | "waiting"
  | "standby"
  | "processing"
  | "grabbed"
  | "superseded"
  | "expired"
  | "dismissed";

export type WantedItem = {
  id: string;
  titleId: string;
  titleName: string | null;
  episodeId: string | null;
  mediaType: WantedMediaType;
  searchPhase: WantedSearchPhase;
  nextSearchAt: string | null;
  lastSearchAt: string | null;
  searchCount: number;
  baselineDate: string | null;
  status: WantedStatus;
  grabbedRelease: string | null;
  currentScore: number | null;
  createdAt: string;
  updatedAt: string;
};

export type PendingReleaseItem = {
  id: string;
  wantedItemId: string;
  titleId: string;
  releaseTitle: string;
  releaseUrl: string | null;
  releaseSizeBytes: string | null;
  releaseScore: number;
  scoringLogJson: string | null;
  indexerSource: string | null;
  addedAt: string;
  delayUntil: string;
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
  explanationJson: string | null;
  createdAt: string;
};
