export type TitleHistoryEvent = {
  scoring?: {
    candidateScore: number;
    previousScore: number | null;
    delta: number | null;
    decisionCode: string;
    entries: import("./file-score").ScoreEntry[];
    comparisons: {
      fileId: string;
      episodeIds: string[];
      previousScore: number;
      delta: number;
    }[];
  } | null;
  id: string;
  // Null for an event with no catalog title behind it: an unlinked grab is
  // recorded against the release and the indexer (FR-026).
  titleId: string | null;
  titleName: string | null;
  facet: string | null;
  episodeId: string | null;
  episodeIds: string[];
  collectionId: string | null;
  eventType: string;
  actorKind: "USER" | "ANONYMOUS" | "SYSTEM" | null;
  actorUserId: string | null;
  actorDisplayName: string | null;
  sourceTitle: string | null;
  displayTitle: string | null;
  sourceSystem: string | null;
  sourceRef: string | null;
  sourceProvider: string | null;
  sourceHint: string | null;
  quality: string | null;
  downloadId: string | null;
  clientId: string | null;
  clientName: string | null;
  importId: string | null;
  skipReason: string | null;
  retryRequiresPassword: boolean;
  failureReasonCode?: string | null;
  failureReason: string | null;
  blocklistReason: string | null;
  sourcePath: string | null;
  destPath: string | null;
  dataJson: unknown;
  occurredAt: string;
  createdAt: string;
};

export type TitleHistoryPage = {
  items: TitleHistoryEvent[];
  totalCount: number;
  hasMore: boolean;
};
