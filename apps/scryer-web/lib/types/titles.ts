import type { CanonicalMediaTag } from "./canonical-tags";
import type { DownloadClientRoutingEntry } from "./download-clients";
import type { CatalogDiscoveryItem } from "./discovery";
import type {
  MediaRequestLeaseRecord,
  RequestRuleDecisionRecord,
} from "./request-rule-sets";
import type { ImportMode } from "./settings";

export type { CanonicalMediaTag };

export type Facet = "MOVIE" | "SERIES" | "ANIME";

export type ExternalId = {
  source: string;
  value: string;
};

/**
 * Advanced monitoring: the seasons (and, for anime, the canon series movies)
 * the user asked to have monitored. Anything absent stays unmonitored. The
 * shape matches the `MonitorSelectionInput`/`MonitorSelectionPayload` pair, so
 * records and drafts are the same thing on the wire.
 */
export type MonitorSelectionMovieDraft = {
  name: string;
  externalIds: ExternalId[];
};

export type MonitorSelectionDraft = {
  seasonNumbers: number[];
  seriesMovies: MonitorSelectionMovieDraft[];
};

export type TitleExternalRatingRecord = {
  source: string;
  value?: number | null;
  score?: number | null;
  normalized: number;
  votes?: number | null;
  url?: string | null;
};

export type TitleRatingRecord = {
  rating?: number | null;
  ratingSources: string[];
  externalRatings: TitleExternalRatingRecord[];
};

/**
 * One cast or crew credit cached from the title's last metadata hydration.
 * `kind` mirrors the metadata provider's own vocabulary (`actor`,
 * `voice_actor`, `director`, ...) rather than a Scryer enum.
 */
export type TitleCreditRecord = {
  kind: string;
  personName: string;
  personOriginalName?: string | null;
  personImageUrl?: string | null;
  character?: string | null;
  language?: string | null;
  billingOrder?: number | null;
  episodeCount?: number | null;
};

export type TitleCollectionEpisodeRecord = {
  id: string;
  titleId: string;
  collectionId?: string | null;
  episodeType?: 'STANDARD' | 'SPECIAL' | 'OFFICIAL' | 'OVA' | 'ONA' | 'ALTERNATE' | null;
  episodeNumber?: string | number | null;
  seasonNumber?: string | number | null;
  episodeLabel?: string | null;
  title?: string | null;
  overview?: string | null;
  airDate?: string | null;
  durationSeconds?: number | null;
  hasMultiAudio?: boolean | null;
  hasSubtitle?: boolean | null;
  isFiller?: boolean | null;
  isRecap?: boolean | null;
  absoluteNumber?: string | number | null;
  imageUrl?: string | null;
  monitored?: boolean | null;
  createdAt?: string | null;
};

export type TitleCollectionRecord = {
  id: string;
  titleId: string;
  collectionType?: 'SEASON' | 'MOVIE' | 'ARC' | 'SPECIALS' | null;
  collectionIndex?: string | number | null;
  label?: string | null;
  orderedPath?: string | null;
  narrativeOrder?: string | number | null;
  fileSizeBytes?: number | null;
  firstEpisodeNumber?: string | number | null;
  lastEpisodeNumber?: string | number | null;
  monitored?: boolean | null;
  episodesOwned?: number | null;
  episodesMonitored?: number | null;
  episodesTotal?: number | null;
  episodes?: TitleCollectionEpisodeRecord[] | null;
  createdAt?: string | null;
};

export type TitleMediaFileRecord = {
  id: string;
  titleId: string;
  episodeId?: string | null;
  seriesMovieLinkIds?: string[] | null;
  role?: string | null;
  filePath?: string | null;
  sizeBytes?: number | null;
  qualityLabel?: string | null;
  scanStatus?: string | null;
  createdAt?: string | null;
  videoCodec?: string | null;
  videoWidth?: number | null;
  videoHeight?: number | null;
  videoBitrateKbps?: number | null;
  videoBitDepth?: number | null;
  videoHdrFormat?: string | null;
  videoFrameRate?: string | null;
  videoProfile?: string | null;
  audioCodec?: string | null;
  audioChannels?: number | null;
  audioBitrateKbps?: number | null;
  audioLanguages?: string[] | null;
  audioStreams?:
    | {
        codec: string | null;
        channels: number | null;
        language: string | null;
        bitrateKbps: number | null;
      }[]
    | null;
  subtitleLanguages?: string[] | null;
  subtitleCodecs?: string[] | null;
  subtitleStreams?:
    | {
        codec: string | null;
        language: string | null;
        name: string | null;
        forced: boolean | null;
        default: boolean | null;
      }[]
    | null;
  hasMultiaudio?: boolean | null;
  durationSeconds?: number | null;
  numChapters?: number | null;
  containerFormat?: string | null;
  sceneName?: string | null;
  releaseGroup?: string | null;
  sourceType?: string | null;
  resolution?: string | null;
  videoCodecParsed?: string | null;
  audioCodecParsed?: string | null;
  acquisitionScore?: number | null;
  scoringLog?: string | null;
  indexerSource?: string | null;
  grabbedReleaseTitle?: string | null;
  grabbedAt?: string | null;
  edition?: string | null;
  originalFilePath?: string | null;
  releaseHash?: string | null;
};

export type TitleReleaseBlocklistEntry = {
  id: string;
  releaseName: string;
  errorMessage: string | null;
  attemptedAt: string;
};

export type TitleRecord = {
  id: string;
  name: string;
  facet: Facet;
  libraryId: string;
  libraryName?: string | null;
  librarySlug?: string | null;
  monitored: boolean;
  playbackLinks?: import("@/components/common/watch-in-media-server-menu").MediaServerPlaybackLink[];
  tags: string[];
  createdAt?: string | null;
  year?: number | null;
  overview?: string | null;
  sortTitle?: string | null;
  slug?: string | null;
  imdbId?: string | null;
  externalIds?: ExternalId[] | null;
  qualityTier?: string | null;
  currentQualityTier?: string | null;
  sizeBytes?: number | null;
  episodesOwned?: number | null;
  episodesMonitored?: number | null;
  episodesTotal?: number | null;
  contentStatus?: string | null;
  posterUrl?: string | null;
  posterSourceUrl?: string | null;
  backgroundUrl?: string | null;
  backgroundSourceUrl?: string | null;
  runtimeMinutes?: number | null;
  popularity?: number | null;
  mediaResolution?: string | null;
  mediaHdr?: string | null;
  mediaAudioCodec?: string | null;
  ratings?: TitleRatingRecord | null;
  canonicalTags?: CanonicalMediaTag[];
  language?: string | null;
  firstAired?: string | null;
  network?: string | null;
  studio?: string | null;
  country?: string | null;
  aliases?: string[];
  metadataLanguage?: string | null;
  metadataLanguageOverride?: string | null;
  effectiveMetadataLanguage?: string | null;
  inheritsMetadataLanguage?: boolean;
  requiredAudioLanguagesOverride?: string[] | null;
  effectiveRequiredAudioLanguages?: string[];
  inheritsRequiredAudioLanguages?: boolean;
  metadataFetchedAt?: string | null;
  minAvailability?: string | null;
  qualityProfileId?: string | null;
  rootFolderId?: string;
  rootFolderPath?: string;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  useSeasonFoldersOverride?: boolean | null;
  effectiveUseSeasonFolders?: boolean;
  inheritsUseSeasonFolders?: boolean;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: 'DOWNLOAD_ALL' | 'SKIP_FILLER' | null;
  recapPolicy?: 'DOWNLOAD_ALL' | 'SKIP_RECAP' | null;
  collections?: TitleCollectionRecord[] | null;
  mediaFiles?: TitleMediaFileRecord[] | null;
  credits?: TitleCreditRecord[] | null;
  moreLikeThis?: CatalogDiscoveryItem[] | null;
};

export type RootFolderOption = {
  id?: string;
  path: string;
  isDefault: boolean;
};

export type TitleCatalogTagFilterOption = {
  key: string;
  name: string;
};

export type TitleCatalogFilterOptionsRecord = {
  genres: TitleCatalogTagFilterOption[];
  themes: TitleCatalogTagFilterOption[];
  minimumYear: number | null;
  maximumYear: number | null;
};

export type LibraryRootRecord = {
  id: string;
  path: string;
  isDefault: boolean;
};

export type LibraryRecord = {
  id: string;
  facet: Facet;
  name: string;
  slug: string;
  isDefault: boolean;
  isBootstrapDefaultRootSet?: boolean;
  roots: LibraryRootRecord[];
  qualityProfileId?: string | null;
  requestQualityProfileIds?: string[];
  requestQualityProfileDefaultId?: string | null;
};

export type MediaRequestRequesterRecord = {
  userId: string;
  username: string;
  avatarUrl?: string | null;
  requestedAt: string;
};

export type MediaRequestRecord = {
  id: string;
  libraryId: string;
  facet: Facet;
  status: "PENDING" | "APPROVED" | "REJECTED" | "CANCELED";
  identityFingerprint: string;
  title: string;
  sortTitle?: string | null;
  slug?: string | null;
  posterUrl?: string | null;
  backgroundUrl?: string | null;
  year?: number | null;
  overview?: string | null;
  runtimeMinutes?: number | null;
  language?: string | null;
  contentStatus?: string | null;
  rating?: number | null;
  ratingSources: string[];
  externalRatings: TitleExternalRatingRecord[];
  requestedQualityProfileId?: string | null;
  requestedQualityProfileName?: string | null;
  requestedMonitorType?: string | null;
  requestedMonitorSelection?: MonitorSelectionDraft | null;
  resolvedByUserId?: string | null;
  resolvedAt?: string | null;
  createdTitleId?: string | null;
  approvedQualityProfileId?: string | null;
  approvedQualityProfileName?: string | null;
  externalIds: ExternalId[];
  requesters: MediaRequestRequesterRecord[];
  createdByUserId: string;
  createdAt: string;
  updatedAt: string;
  /// How long the requester asked the media to be kept, in days. Null means
  /// forever: there is no separate flag on the requester side.
  requestedLeaseDays?: number | null;
  /// What the approver granted. Null means forever, and it stays null until the
  /// request is approved.
  approvedLeaseDays?: number | null;
  /// The claim actually holding the created title. Null until an approval
  /// creates one, and `DORMANT` with no window until the title first imports.
  lease?: MediaRequestLeaseRecord | null;
  /// What request rules decided. A requester reading their own request gets
  /// this with `votes` emptied; there is no flag saying which you got, so
  /// render `reasons` unless you know the reader manages the library.
  decision?: RequestRuleDecisionRecord | null;
  /// Tags the policy emitted. Stamped on the title only when the request is
  /// approved, so on a pending row these are what *would* be applied.
  policyTags?: string[];
};

export type LibrarySettingsRecord = {
  requiredAudioLanguagesOverride: string[] | null;
  requiredAudioLanguages: string[];
  metadataLanguageOverride: string | null;
  metadataLanguage: string;
  useSeasonFoldersOverride: boolean | null;
  useSeasonFolders: boolean;
  qualityProfileIdOverride: string | null;
  qualityProfileId: string;
  requestQualityProfileIdsOverride: string[] | null;
  requestQualityProfileIds: string[];
  requestQualityProfileDefaultId: string;
  scoringPersonaOverride: string | null;
  scoringPersona: string;
  fillerPolicyOverride: string | null;
  fillerPolicy: 'DOWNLOAD_ALL' | 'SKIP_FILLER' | null;
  recapPolicyOverride: string | null;
  recapPolicy: 'DOWNLOAD_ALL' | 'SKIP_RECAP' | null;
  monitorSpecialsOverride: boolean | null;
  monitorSpecials: boolean | null;
  interSeasonMoviesOverride: boolean | null;
  interSeasonMovies: boolean | null;
  monitorFillerMoviesOverride: boolean | null;
  monitorFillerMovies: boolean | null;
  nfoWriteOnImportOverride: boolean | null;
  nfoWriteOnImport: boolean;
  plexmatchWriteOnImportOverride: boolean | null;
  plexmatchWriteOnImport: boolean | null;
  importModeOverride: ImportMode | null;
  importMode: ImportMode;
  setPermissionsLinuxOverride: boolean | null;
  setPermissionsLinux: boolean;
  fileChmodOverride: string | null;
  fileChmod: string | null;
  folderChmodOverride: string | null;
  folderChmod: string | null;
  chownGroupOverride: string | null;
  chownGroup: string | null;
  indexerRoutingOverride: unknown[] | null;
  downloadClientRoutingOverride: DownloadClientRoutingEntry[] | null;
};

export type LibrarySettingsDraft = {
  requiredAudioLanguages: string[] | null;
  metadataLanguage: string | null;
  useSeasonFolders: boolean | null;
  qualityProfileId: string | null;
  requestQualityProfileIds: string[] | null;
  scoringPersona: string | null;
  fillerPolicy: 'DOWNLOAD_ALL' | 'SKIP_FILLER' | null;
  recapPolicy: 'DOWNLOAD_ALL' | 'SKIP_RECAP' | null;
  monitorSpecials: boolean | null;
  interSeasonMovies: boolean | null;
  monitorFillerMovies: boolean | null;
  nfoWriteOnImport: boolean | null;
  plexmatchWriteOnImport: boolean | null;
  importMode: ImportMode | null;
  setPermissionsLinux: boolean | null;
  fileChmod: string | null;
  folderChmod: string | null;
  chownGroup: string | null;
  indexerRouting?: unknown[] | null;
  downloadClientRouting?: DownloadClientRoutingEntry[] | null;
};

export type LibraryScanSummary = {
  scanned: number;
  matched: number;
  imported: number;
  skipped: number;
  unmatched: number;
};
