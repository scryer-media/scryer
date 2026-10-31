export type PendingImportCounts = {
  movie: number;
  series: number;
  anime: number;
};

export type PendingImportStatus = "PENDING" | "IGNORED";

export type PendingImportSearchAttempt = {
  query: string;
  resultCount: number;
  topResults: string[];
  summary: string;
};

export type PendingImportItem = {
  id: string;
  libraryId: string;
  libraryName?: string | null;
  librarySlug?: string | null;
  facet: "MOVIE" | "SERIES" | "ANIME";
  status: PendingImportStatus;
  titleId?: string | null;
  titleName?: string | null;
  titleSlug?: string | null;
  titleFolderPath?: string | null;
  displayName: string;
  path: string;
  folderPath?: string | null;
  query: string;
  yearHint?: number | null;
  reason: string;
  searchAttempts: PendingImportSearchAttempt[];
};

export type PendingImportConnection = {
  totalCount: number;
  items: PendingImportItem[];
};

export type PendingImportBindingEpisode = {
  id: string;
  titleId: string;
  collectionId?: string | null;
  episodeType:
    "STANDARD" | "SPECIAL" | "OFFICIAL" | "OVA" | "ONA" | "ALTERNATE";
  episodeNumber?: string | null;
  seasonNumber?: string | null;
  absoluteNumber?: string | null;
  episodeLabel?: string | null;
  title?: string | null;
  monitored: boolean;
};

export type PendingImportBindingPreview = {
  title: {
    id: string;
    name: string;
    facet: string;
    monitored: boolean;
  };
  file: {
    filePath: string;
    fileName: string;
    sizeBytes: number;
    parsedSeason?: number | null;
    parsedEpisodes: number[];
    parsedAbsoluteNumbers: number[];
    suggestedEpisodeIds: string[];
  };
  availableEpisodes: PendingImportBindingEpisode[];
};

export type ResolvePendingImportResult = {
  title: {
    id: string;
    libraryId?: string | null;
    name: string;
    facet: string;
    monitored: boolean;
    slug?: string | null;
  };
  created: boolean;
  metadataHydrationState?: "pending" | "complete" | "not_required";
  libraryScan?: {
    scanned: number;
    matched: number;
    imported: number;
    skipped: number;
    unmatched: number;
  };
};

export function pendingImportCountForView(
  counts: PendingImportCounts | null | undefined,
  view: string,
): number {
  if (!counts) {
    return 0;
  }

  switch (view) {
    case "movies":
      return counts.movie;
    case "series":
      return counts.series;
    case "anime":
      return counts.anime;
    default:
      return 0;
  }
}

export function hasImportItemsForView(
  pendingCounts: PendingImportCounts | null | undefined,
  view: string,
): boolean {
  return pendingImportCountForView(pendingCounts, view) > 0;
}

export function pendingImportFacetValueForView(
  view: string,
): "MOVIE" | "SERIES" | "ANIME" {
  switch (view) {
    case "movies":
      return "MOVIE";
    case "series":
      return "SERIES";
    case "anime":
      return "ANIME";
    default:
      throw new Error(`unsupported pending import view: ${view}`);
  }
}
