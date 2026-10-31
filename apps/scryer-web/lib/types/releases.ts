import type { ScoringEntryKind } from "@/lib/utils/release-decision-explanation";

export type DownloadSourceKind =
  "NZB_FILE" | "NZB_URL" | "TORRENT_FILE" | "MAGNET_URI" | "HTTP_RESOURCE";

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
  /** Seeders plus leechers when the indexer reported those but no peer count. */
  peers?: number | null;
  leechers?: number | null;
  /** The indexer's freeleech claim, or a download volume factor of zero. */
  freeleech?: boolean | null;
  downloadVolumeFactor?: number | null;
  uploadVolumeFactor?: number | null;
  /** Seeding the tracker requires; null when it reported none. */
  minimumSeedRatio?: number | null;
  minimumSeedTimeMinutes?: number | null;
  /** Indexer flags such as freeleech or internal, in indexer order. */
  indexerFlags?: string[];
  languages?: string[];
  subtitles?: string[];
  files?: number | null;
  year?: number | null;
  posterUrl?: string | null;
  parsedRelease?: {
    rawTitle: string;
    normalizedTitle: string;
    year?: number | null;
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
    revision: number;
    isRemux: boolean;
    isBdDisk: boolean;
    isAiEnhanced: boolean;
  } | null;
  qualityProfileDecision?: {
    allowed: boolean;
    blockCodes: string[];
    releaseScore: number;
    preferenceScore: number;
    scoringLog: {
      code: string;
      delta: number;
      kind?: ScoringEntryKind;
      source: string;
      ruleSetName?: string | null;
    }[];
  } | null;
  autoEligible?: boolean | null;
  autoDecisionCode?: string | null;
  autoDecisionSummary?: string | null;
};
