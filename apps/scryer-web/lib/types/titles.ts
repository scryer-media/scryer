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
  tags: string[];
  imdbId?: string | null;
  externalIds: ExternalId[];
  qualityTier?: string | null;
  sizeBytes?: number | null;
  contentStatus?: string | null;
  posterUrl?: string | null;
  posterSourceUrl?: string | null;
  bannerUrl?: string | null;
  bannerSourceUrl?: string | null;
  backgroundUrl?: string | null;
  backgroundSourceUrl?: string | null;
};

export type RootFolderOption = {
  path: string;
  isDefault: boolean;
};

export type LibraryScanSummary = {
  scanned: number;
  matched: number;
  imported: number;
  skipped: number;
  unmatched: number;
};
