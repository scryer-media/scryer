export type WantedItem = {
  id: string;
  titleId: string;
  titleName: string | null;
  episodeId: string | null;
  mediaType: string;
  searchPhase: string;
  nextSearchAt: string | null;
  lastSearchAt: string | null;
  searchCount: number;
  baselineDate: string | null;
  status: string;
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
  status: string;
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
