export type Facet = "movie" | "tv" | "anime";

export type ExternalId = {
  source: string;
  value: string;
};

export type TitleRecord = {
  id: string;
  name: string;
  facet: string;
  monitored: boolean;
  externalIds: ExternalId[];
  qualityTier?: string | null;
  sizeBytes?: number | null;
  posterUrl?: string | null;
};

export type LibraryScanSummary = {
  scanned: number;
  matched: number;
  imported: number;
  skipped: number;
  unmatched: number;
};
