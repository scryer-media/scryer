export type DownloadSourceKind = "NZB_FILE" | "NZB_URL" | "TORRENT_FILE" | "MAGNET_URI";

export type ReleaseQueueScope =
  | { __typename: "EpisodeScopePayload"; episodeId: string }
  | { __typename: "EpisodeSetScopePayload"; episodeIds: string[] }
  | { __typename: "CollectionScopePayload"; collectionId: string }
  | { __typename: "SeriesMovieScopePayload"; seriesMovieLinkId: string }
  | { __typename: "TitleScopePayload"; wholeTitle: boolean }
  | { __typename: "OrphanScopePayload"; orphaned: boolean };

export type Release = {
  source: string | null;
  /** Indexer configuration that returned the release; null when the indexer is unknown. */
  indexerId?: string | null;
  title: string;
  link: string | null;
  downloadUrl: string | null;
  candidateToken?: string | null;
  queueScope?: ReleaseQueueScope | null;
  sourceKind?: DownloadSourceKind | null;
  sizeBytes: number | null;
  publishedAt: string | null;
  thumbsUp?: number | null;
  thumbsDown?: number | null;
  /** Grab count the indexer reports; usenet's counterpart to seeders/peers. */
  grabs?: number | null;
  /** Torrent swarm counts; null for usenet results. Selected by the search documents. */
  seeders?: number | null;
  peers?: number | null;
  freeleech?: boolean | null;
  parsedRelease?: {
    rawTitle: string;
    normalizedTitle: string;
    releaseGroup?: string | null;
    quality?: string | null;
    source?: string | null;
    videoCodec?: string | null;
    videoEncoding?: string | null;
    audio?: string | null;
    isDualAudio: boolean;
    isAtmos: boolean;
    isDolbyVision: boolean;
    detectedHdr: boolean;
    parseConfidence: number;
    isProperUpload: boolean;
    isRemux: boolean;
    isBdDisk: boolean;
    isAiEnhanced: boolean;
  } | null;
  qualityProfileDecision?: {
    allowed: boolean;
    blockCodes: string[];
    releaseScore: number;
    preferenceScore: number;
    scoringLog: { code: string; delta: number; source: string; ruleSetName?: string | null }[];
  } | null;
  autoEligible?: boolean | null;
  autoDecisionCode?: string | null;
  autoDecisionSummary?: string | null;
};
