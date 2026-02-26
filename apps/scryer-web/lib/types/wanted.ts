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

export type WantedItemsList = {
  items: WantedItem[];
  total: number;
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
