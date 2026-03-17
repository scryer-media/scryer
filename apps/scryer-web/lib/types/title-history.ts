export type TitleHistoryEvent = {
  id: string;
  titleId: string;
  episodeId: string | null;
  collectionId: string | null;
  eventType: string;
  sourceTitle: string | null;
  quality: string | null;
  downloadId: string | null;
  dataJson: string | null;
  occurredAt: string;
  createdAt: string;
};

export type TitleHistoryPage = {
  records: TitleHistoryEvent[];
  totalCount: number;
};
