
import * as React from "react";
import { useAutomaticSearch } from "@/lib/hooks/use-automatic-search";
import { parseSearchSeason } from "@/lib/utils/automatic-search";
import { facetById } from "@/lib/facets/registry";
import {
  deleteEpisodeFilesPreviewQuery,
  deleteTitlePreviewQuery,
  episodeCollectionRefQuery,
  episodeSidePanelDetailQuery,
  librariesQuery,
  seriesCollectionEpisodesQuery,
  movieEntityDetailQuery,
  seriesSidePanelOverviewQuery,
} from "@/lib/graphql/queries";
import {
  addListExclusionMutation,
  deleteEpisodeFilesMutation,
  deleteTitleMutation,
  setCollectionMonitoredMutation,
  queueBestReleaseMutation,
  queueExistingMutation,
  queueReplacementMutation,
  setEpisodeMonitoredMutation,
  setPrimaryMovieFileMutation,
  setSeriesMovieMonitoredMutation,
  setTitleMonitoredMutation,
  updateTitleMutation,
} from "@/lib/graphql/mutations";
import type { Release } from "@/lib/types";
import type { CatalogDiscoveryItem } from "@/lib/types/discovery";
import type { TitleRatings } from "@/components/views/title-ratings-strip";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  runIterativeReleaseSearch,
  titleReleaseSearchInput,
} from "@/lib/graphql/release-search";
import { isAbortError } from "@/lib/graphql/urql-client";
import {
  queueScopeReplacesPrimary,
  releaseQueueScopeInput,
} from "@/lib/utils/release-queue-scope";
import {
  drainDeferredCollectionEpisodeRefresh,
  planCollectionEpisodeRefresh,
  TITLE_OVERVIEW_IMPORT_REFRESH_KINDS,
} from "@/lib/utils/title-overview-refresh-policy";
import {
  episodeIdsForEpisodeRecord,
  mergeLoadedEpisodeDetailsForCollections,
  pruneEpisodeRecord,
  pruneSeriesMovieLinkMediaFiles,
} from "@/lib/utils/series-episode-details";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { handleFixTitleMatchComplete as applyFixTitleMatchCompletion } from "@/lib/fix-title-match";
import {
  fetchTitleMoreLikeThis,
  fetchTitleSidePanelOverviewSnapshot,
} from "@/lib/title-overview-loader";
import { SeriesOverviewView } from "@/components/views/series-overview";
import { FixTitleMatchDialog } from "@/components/dialogs/fix-title-match-dialog";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import { useMediaFileDeletion } from "@/components/common/media-file-deletion";
import { useManualImportLauncher } from "@/components/common/manual-import-launcher";
import { Checkbox } from "@/components/ui/checkbox";
import type { OverviewTitleTarget } from "@/components/root/types";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import type {
  CanonicalMediaTag,
  LibraryRecord,
  TitleCreditRecord,
} from "@/lib/types/titles";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import { normalizeJobRun } from "@/lib/utils/job-runs";
import { mediaFileOwnerKeys } from "@/lib/utils/media-file-owners";
import type { JobRun } from "@/lib/types/jobs";
import {
  episodeIdsCoveredByEpisodeFileDelete,
  type DeleteEpisodeFilesPreview,
} from "@/lib/types/delete-preview";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";
import type { TitleSidePanelOverviewSnapshot } from "@/lib/title-overview-loader";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import { useAuth } from "@/lib/hooks/use-auth";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  hasAnyLibraryPermission,
  hasAppPermission,
} from "@/lib/utils/permissions";
import { exclusionInputFromTitle } from "@/lib/utils/lists";
import { useExperimentalFeaturesEnabled } from "@/lib/context/instance-features-context";
import type { Facet } from "@/lib/types/titles";
import { useTitleMoreLikeThisActions } from "@/lib/hooks/use-title-more-like-this-actions";
import { useTitleOverviewReactiveRefresh } from "@/lib/hooks/use-title-overview-reactive-refresh";
import { useCanManageOverviewTitle } from "@/lib/hooks/use-title-overview-access";
import { useTitleDownloadFeedback } from "@/lib/hooks/use-title-download-feedback";
import { useTitleRefreshAndScan } from "@/lib/hooks/use-title-refresh-and-scan";
import { useTitleReleaseBlocklistClear } from "@/lib/hooks/use-title-release-blocklist-clear";
import { useTitleRename } from "@/lib/hooks/use-title-rename";
import {
  reportAutomaticSearchFailure,
  useTitleSearchAction,
} from "@/lib/hooks/use-title-search-action";
import { useTrackedJobRuns } from "@/lib/hooks/use-tracked-job-runs";

export type TitleDetail = {
  id: string;
  sizeBytes?: number | null;
  name: string;
  facet: string;
  libraryId: string;
  libraryName?: string | null;
  librarySlug?: string | null;
  monitored: boolean;
  tags: string[];
  externalIds: { source: string; value: string }[];
  year: number | null;
  overview: string | null;
  posterUrl: string | null;
  posterSourceUrl: string | null;
  backgroundUrl: string | null;
  sortTitle: string | null;
  slug: string | null;
  imdbId: string | null;
  runtimeMinutes: number | null;
  canonicalTags?: CanonicalMediaTag[];
  contentStatus: string | null;
  language: string | null;
  firstAired: string | null;
  network: string | null;
  studio: string | null;
  country: string | null;
  aliases: string[];
  metadataLanguage: string | null;
  metadataLanguageOverride?: string | null;
  effectiveMetadataLanguage?: string | null;
  inheritsMetadataLanguage?: boolean;
  metadataFetchedAt: string | null;
  requiredAudioLanguagesOverride?: string[] | null;
  effectiveRequiredAudioLanguages?: string[];
  inheritsRequiredAudioLanguages?: boolean;
  qualityProfileId?: string | null;
  qualityTier?: string | null;
  rootFolderId?: string;
  rootFolderPath?: string;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  useSeasonFoldersOverride?: boolean | null;
  effectiveUseSeasonFolders?: boolean;
  inheritsUseSeasonFolders?: boolean;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
  releaseNumbering?: string | null;
  effectiveFillerPolicy?: string | null;
  effectiveRecapPolicy?: string | null;
  renameEnabled?: boolean;
  seriesMovieLinks?: SeriesMovieLink[];
  ratings?: TitleRatings | null;
  credits?: TitleCreditRecord[] | null;
  moreLikeThis?: CatalogDiscoveryItem[];
  createdAt: string;
};

export type TitleCollection = {
  id: string;
  titleId: string;
  collectionType: string;
  collectionIndex: string;
  label: string | null;
  orderedPath: string | null;
  narrativeOrder: string | null;
  sizeBytes?: number | null;
  fileSizeBytes: number | null;
  firstEpisodeNumber: string | null;
  lastEpisodeNumber: string | null;
  monitored: boolean;
  episodesOwned: number | null;
  episodesMonitored: number | null;
  episodesTotal: number | null;
  episodeRecordsTotal: number | null;
  createdAt: string;
};

export type MovieEntity = {
  id: string;
  title: string;
  sortTitle: string | null;
  slug: string | null;
  year: number | null;
  overview: string | null;
  posterUrl: string | null;
  backgroundUrl: string | null;
  language: string | null;
  runtimeMinutes: number | null;
  contentStatus: string | null;
  studio: string | null;
  digitalReleaseDate: string | null;
  imdbId: string | null;
  tvdbId: string | null;
  tmdbId: string | null;
  malId: string | null;
  anidbId: string | null;
  ratings?: TitleRatings | null;
  credits?: TitleCreditRecord[] | null;
  createdAt: string;
  updatedAt: string;
};

export type SeriesMovieLink = {
  id: string;
  seriesTitleId: string;
  movie: MovieEntity;
  placement: string | null;
  narrativeOrder: string | null;
  afterSeason: number | null;
  beforeSeason: number | null;
  linkedEpisodeId: string | null;
  associationConfidence: string | null;
  continuityStatus: string | null;
  movieForm: string | null;
  confidence: string | null;
  signalSummary: string | null;
  source: string | null;
  monitoringOverride: boolean | null;
  metadataActive: boolean;
  monitored: boolean;
  /// User tags on the link itself. A series movie is not a title, so this is
  /// its own bag rather than the series title's.
  tags?: string[] | null;
  createdAt: string;
  updatedAt: string;
};

import type { TitleHistoryEvent } from "@/lib/types";
export type { TitleHistoryEvent };

export type TitleReleaseBlocklistEntry = {
  id: string;
  releaseName: string;
  errorMessage: string | null;
  attemptedAt: string;
};

export type CollectionEpisode = {
  id: string;
  titleId: string;
  collectionId: string | null;
  episodeType: string;
  episodeNumber: string | null;
  seasonNumber: string | null;
  episodeLabel: string | null;
  title: string | null;
  sizeBytes?: number | null;
  overview?: string | null;
  airDate: string | null;
  durationSeconds: number | null;
  isFiller: boolean;
  isRecap: boolean;
  absoluteNumber: string | null;
  imageUrl?: string | null;
  monitored: boolean;
  playbackLinks?: import("@/components/common/watch-in-media-server-menu").MediaServerPlaybackLink[];
  mediaAvailability: {
    state: "AVAILABLE" | "PENDING_SCAN" | "SCAN_FAILED" | "REVIEW_REQUIRED" | "MISSING" | "UNMONITORED";
    primaryQualityLabel: string | null;
  };
  createdAt: string;
};

export type EpisodeMediaFile = {
  id: string;
  titleId: string;
  episodeId: string | null;
  seriesMovieLinkIds: string[];
  role: string;
  filePath: string;
  sizeBytes: number;
  qualityLabel: string | null;
  scanStatus: string;
  createdAt: string;
  videoCodec: string | null;
  videoWidth: number | null;
  videoHeight: number | null;
  videoBitrateKbps: number | null;
  analysis?: import("@/lib/types/media-analysis").MediaAnalysisDetails;
  analysisAttempt?: import("@/lib/types/media-analysis").MediaAnalysisAttempt | null;
  videoBitDepth: number | null;
  videoHdrFormat: string | null;
  videoFrameRate: string | null;
  videoProfile: string | null;
  audioCodec: string | null;
  audioChannels: number | null;
  audioBitrateKbps: number | null;
  audioLanguages: string[];
  audioStreams: { codec: string | null; channels: number | null; language: string | null; bitrateKbps: number | null }[];
  subtitleLanguages: string[];
  subtitleCodecs: string[];
  subtitleStreams: { codec: string | null; language: string | null; name: string | null; forced: boolean; default: boolean }[];
  hasMultiaudio: boolean;
  durationSeconds: number | null;
  numChapters: number | null;
  containerFormat: string | null;
  sceneName: string | null;
  releaseGroup: string | null;
  sourceType: string | null;
  resolution: string | null;
  videoCodecParsed: string | null;
  audioCodecParsed: string | null;
  acquisitionScore: number | null;
  scoringLog: string | null;
  indexerSource: string | null;
  grabbedReleaseTitle: string | null;
  grabbedAt: string | null;
  edition: string | null;
  originalFilePath: string | null;
  releaseHash: string | null;
};

type SeriesOverviewSnapshotTitle = TitleDetail & {
  collections?: TitleCollection[];
};

type SeriesOverviewContainerProps = {
  titleId: string;
  fullBleedHero?: boolean;
  onTitleNotFound?: () => void;
  onBackToList?: () => void;
  onTitleResolved?: (title: OverviewTitleTarget) => void;
  initialEpisodeId?: string | null;
};

function groupMediaFilesByEpisode(
  files: EpisodeMediaFile[],
): Record<string, EpisodeMediaFile[]> {
  const grouped: Record<string, EpisodeMediaFile[]> = {};
  for (const file of files) {
    const key = file.episodeId ?? "__unlinked__";
    (grouped[key] ??= []).push(file);
  }
  return grouped;
}

function groupMediaFilesBySeriesMovieLink(
  files: EpisodeMediaFile[],
): Record<string, EpisodeMediaFile[]> {
  const grouped: Record<string, EpisodeMediaFile[]> = {};
  for (const file of files) {
    for (const linkId of file.seriesMovieLinkIds ?? []) {
      (grouped[linkId] ??= []).push(file);
    }
  }
  return grouped;
}

function retainEquivalentSnapshot<T>(current: T, next: T): T {
  if (Object.is(current, next) || JSON.stringify(current) === JSON.stringify(next)) {
    return current;
  }
  return next;
}

/**
 * Read the media-file ids a finished episode-file deletion run reports removing.
 * Returns null when the run carried no usable summary, so the caller can fall
 * back to dropping the whole cached episode instead of trusting a partial list.
 */
function readDeletedFileIds(summaryJson: unknown): Set<string> | null {
  const parsed =
    typeof summaryJson === "string"
      ? (() => {
          try {
            return JSON.parse(summaryJson) as unknown;
          } catch {
            return null;
          }
        })()
      : summaryJson;
  if (!parsed || typeof parsed !== "object") {
    return null;
  }
  const ids = (parsed as { deletedFileIds?: unknown }).deletedFileIds;
  if (!Array.isArray(ids) || ids.some((id) => typeof id !== "string")) {
    return null;
  }
  return new Set(ids as string[]);
}

/**
 * The batch episode-file preview wraps the shared `DeletePreview` alongside the
 * per-file breakdown, so the shared hook needs to be told where the preview is.
 */
function selectEpisodeFilesDeletePreview(
  payload: DeleteEpisodeFilesPreview,
): DeleteEpisodeFilesPreview["preview"] {
  return payload.preview;
}

export const SeriesOverviewContainer = React.memo(function SeriesOverviewContainer({
  titleId,
  fullBleedHero,
  onTitleNotFound,
  onBackToList,
  onTitleResolved,
  initialEpisodeId,
}: SeriesOverviewContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const trackJobRun = useTrackedJobRuns();
  const t = useTranslate();
  const { startAutomaticSearch, isSearching } = useAutomaticSearch();
  const client = useClient();
  const auth = useAuth();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();
  const [title, setTitle] = React.useState<TitleDetail | null>(null);
  const canManageTitle = useCanManageOverviewTitle(title?.libraryId);
  const canAddDiscoveryItems = hasAnyLibraryPermission(
    auth.user,
    LIBRARY_PERMISSIONS.manageTitles,
  );
  const canRequestDiscoveryItems = hasAnyLibraryPermission(
    auth.user,
    LIBRARY_PERMISSIONS.request,
  );
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const canManageLists =
    experimentalFeaturesEnabled && hasAppPermission(auth.user, APP_PERMISSIONS.manageLists);
  const [collections, setCollections] = React.useState<TitleCollection[]>([]);
  const [seriesMovieLinks, setSeriesMovieLinks] = React.useState<SeriesMovieLink[]>([]);
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
  const [releaseBlocklistEntries, setReleaseBlocklistEntries] = React.useState<
    TitleReleaseBlocklistEntry[]
  >([]);
  const [loading, setLoading] = React.useState(true);
  const [episodesByCollection, setEpisodesByCollection] = React.useState<
    Record<string, CollectionEpisode[]>
  >({});
  const episodesByCollectionRef = React.useRef(episodesByCollection);
  episodesByCollectionRef.current = episodesByCollection;
  const [collectionEpisodesLoading, setCollectionEpisodesLoading] = React.useState<
    Record<string, boolean>
  >({});
  const collectionEpisodesLoadingRef = React.useRef<Set<string>>(new Set());
  // Refresh requests that arrived while the collection's own load was in
  // flight; released by releaseDeferredCollectionRefresh as each load resolves.
  const deferredCollectionRefreshRef = React.useRef<Set<string>>(new Set());
  const refreshLoadedCollectionEpisodesRef = React.useRef<
    (collectionIds: readonly string[]) => void
  >(() => {});
  const deepLinkResolveAttemptedRef = React.useRef<string | null>(null);
  // Every library this facet offers, not just the title's own: the move
  // workflow needs the full list to reach a cross-library destination.
  const [libraries, setLibraries] = React.useState<LibraryRecord[]>([]);
  const [mediaFilesByEpisode, setMediaFilesByEpisode] = React.useState<
    Record<string, EpisodeMediaFile[]>
  >({});
  const [mediaFilesBySeriesMovieLink, setMediaFilesBySeriesMovieLink] = React.useState<
    Record<string, EpisodeMediaFile[]>
  >({});
  const [episodeDetailsLoaded, setEpisodeDetailsLoaded] = React.useState<
    ReadonlySet<string>
  >(() => new Set());
  const episodeDetailsLoadedRef = React.useRef(episodeDetailsLoaded);
  episodeDetailsLoadedRef.current = episodeDetailsLoaded;
  const [episodeDetailsLoading, setEpisodeDetailsLoading] = React.useState<
    Record<string, boolean>
  >({});
  const [subtitleDownloads, setSubtitleDownloads] = React.useState<
    ExternalSubtitleRecord[]
  >([]);
  const [monitoredUpdating, setMonitoredUpdating] = React.useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = React.useState(false);
  const [deleteFilesOnDisk, setDeleteFilesOnDisk] = React.useState(false);
  const [alsoExcludeFromLists, setAlsoExcludeFromLists] = React.useState(false);
  const [deleteLoading, setDeleteLoading] = React.useState(false);
  const [titleDeleteTypedConfirmation, setTitleDeleteTypedConfirmation] =
    React.useState("");
  const [primaryMovieFileUpdatingId, setPrimaryMovieFileUpdatingId] =
    React.useState<string | null>(null);
  const [episodeFilesToDelete, setEpisodeFilesToDelete] =
    React.useState<string[] | null>(null);
  const [episodeFilesDeleteLoading, setEpisodeFilesDeleteLoading] =
    React.useState(false);
  const [episodeFilesDeleteTypedConfirmation, setEpisodeFilesDeleteTypedConfirmation] =
    React.useState("");
  const [episodeSelectionResetToken, setEpisodeSelectionResetToken] = React.useState(0);
  // Episodes whose media files are being deleted by an in-flight job; their
  // rows cannot be re-selected until the run reaches a terminal status.
  const [pendingEpisodeFileDeletionEpisodeIds, setPendingEpisodeFileDeletionEpisodeIds] =
    React.useState<Set<string>>(() => new Set());
  const [fixMatchOpen, setFixMatchOpen] = React.useState(false);
  const [titleLookupAttempted, setTitleLookupAttempted] = React.useState(false);
  const [titleLookupFailed, setTitleLookupFailed] = React.useState(false);
  const currentTitleIdRef = React.useRef<string | null>(titleId ?? null);
  React.useEffect(() => {
    currentTitleIdRef.current = titleId ?? null;
  }, [titleId]);
  const seriesMovieDetailLoadingRef = React.useRef<Set<string>>(new Set());
  // The page's own overview query reports this, so a download client added
  // while the page is open is picked up by its next refresh.
  const [downloadClientsAnswer, setDownloadClientsAnswer] = React.useState<{
    titleId: string;
    configured: boolean;
  } | null>(null);
  const hasDownloadClients =
    titleId && downloadClientsAnswer?.titleId === titleId
      ? downloadClientsAnswer.configured
      : null;
  const {
    queueItems: downloadQueueItems,
    completedDownloads,
    refresh: refreshDownloadFeedback,
  } = useTitleDownloadFeedback({
    titleId: titleId ?? null,
    hasDownloadClients,
  });
  const searchAction = useTitleSearchAction({
    titleId: title?.id ?? null,
    titleName: title?.name ?? "",
    hasDownloadClients,
  });

  const titleDeletePreviewVariables = React.useMemo(
    () =>
      title && deleteDialogOpen && deleteFilesOnDisk
        ? { titleId: title.id }
        : null,
    [deleteDialogOpen, deleteFilesOnDisk, title],
  );
  const {
    preview: titleDeletePreview,
    loading: titleDeletePreviewLoading,
    error: titleDeletePreviewError,
  } = useDeletePreview(
    deleteTitlePreviewQuery,
    "deleteTitlePreview",
    titleDeletePreviewVariables,
    deleteDialogOpen && title !== null && deleteFilesOnDisk,
  );
  const episodeFilesDeletePreviewVariables = React.useMemo(
    () =>
      title && episodeFilesToDelete
        ? {
            input: {
              titleId: title.id,
              episodeIds: [...episodeFilesToDelete].sort(),
            },
          }
        : null,
    [episodeFilesToDelete, title],
  );
  const {
    preview: episodeFilesDeletePreview,
    payload: episodeFilesDeletePreviewPayload,
    loading: episodeFilesDeletePreviewLoading,
    error: episodeFilesDeletePreviewError,
  } = useDeletePreview<Record<string, unknown>, DeleteEpisodeFilesPreview>(
    deleteEpisodeFilesPreviewQuery,
    "deleteEpisodeFilesPreview",
    episodeFilesDeletePreviewVariables,
    episodeFilesToDelete !== null,
    selectEpisodeFilesDeletePreview,
  );

  const releaseDeferredCollectionRefresh = React.useCallback(
    (collectionId: string) => {
      const { rerun, remaining } = drainDeferredCollectionEpisodeRefresh(
        deferredCollectionRefreshRef.current,
        collectionId,
      );
      deferredCollectionRefreshRef.current = remaining;
      if (rerun.length > 0) {
        refreshLoadedCollectionEpisodesRef.current(rerun);
      }
    },
    [],
  );

  const loadCollectionEpisodes = React.useCallback(
    async (collectionId: string, options: { force?: boolean } = {}) => {
      if (!titleId) {
        return;
      }
      if (collectionEpisodesLoadingRef.current.has(collectionId)) {
        return;
      }
      if (!options.force && collectionId in episodesByCollectionRef.current) {
        return;
      }

      const requestedTitleId = titleId;
      collectionEpisodesLoadingRef.current.add(collectionId);
      setCollectionEpisodesLoading((current) => ({
        ...current,
        [collectionId]: true,
      }));
      try {
        const { data, error } = await client
          .query(
            seriesCollectionEpisodesQuery,
            { id: collectionId },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }
        if (currentTitleIdRef.current !== requestedTitleId) {
          return;
        }
        const collectionDetail = data?.collectionById as
          | { episodes?: CollectionEpisode[] | null }
          | null
          | undefined;
        const episodes = (collectionDetail?.episodes ?? []) as CollectionEpisode[];
        setEpisodesByCollection((current) => {
          const nextEpisodes =
            mergeLoadedEpisodeDetailsForCollections(
              [{ id: collectionId, episodes }],
              current,
              episodeDetailsLoadedRef.current,
            )[collectionId] ?? episodes;
          return JSON.stringify(current[collectionId]) === JSON.stringify(nextEpisodes)
            ? current
            : { ...current, [collectionId]: nextEpisodes };
        });
      } catch (error: unknown) {
        if (currentTitleIdRef.current === requestedTitleId) {
          // Mark the season as hydrated (empty) so the expand effect does not
          // hot-loop on a persistent failure; the next overview refresh
          // force-refetches it.
          setEpisodesByCollection((current) =>
            collectionId in current
              ? current
              : { ...current, [collectionId]: [] },
          );
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.apiError"),
          );
        }
      } finally {
        collectionEpisodesLoadingRef.current.delete(collectionId);
        setCollectionEpisodesLoading((current) => {
          const next = { ...current };
          delete next[collectionId];
          return next;
        });
        releaseDeferredCollectionRefresh(collectionId);
      }
    },
    [client, releaseDeferredCollectionRefresh, setGlobalStatus, t, titleId],
  );

  const refreshLoadedCollectionEpisodes = React.useCallback(
    async (collectionIds: readonly string[]) => {
      if (!titleId) {
        return;
      }

      const { refreshNow: collectionIdsToRefresh, deferred } =
        planCollectionEpisodeRefresh(
          collectionIds,
          collectionEpisodesLoadingRef.current,
        );
      // The in-flight query was issued before the event that asked for this
      // refresh, so its answer cannot carry the change. Hold the request until
      // that load resolves instead of dropping it.
      for (const collectionId of deferred) {
        deferredCollectionRefreshRef.current.add(collectionId);
      }
      if (collectionIdsToRefresh.length === 0) {
        return;
      }

      const requestedTitleId = titleId;
      for (const collectionId of collectionIdsToRefresh) {
        collectionEpisodesLoadingRef.current.add(collectionId);
      }
      setCollectionEpisodesLoading((current) => {
        const next = { ...current };
        for (const collectionId of collectionIdsToRefresh) {
          next[collectionId] = true;
        }
        return next;
      });

      try {
        const settled = await Promise.allSettled(
          collectionIdsToRefresh.map(async (collectionId) => {
            const { data, error } = await client
              .query(
                seriesCollectionEpisodesQuery,
                { id: collectionId },
                { requestPolicy: "network-only" },
              )
              .toPromise();
            if (error) {
              throw error;
            }
            const collectionDetail = data?.collectionById as
              | { episodes?: CollectionEpisode[] | null }
              | null
              | undefined;
            return {
              id: collectionId,
              episodes: (collectionDetail?.episodes ?? []) as CollectionEpisode[],
            };
          }),
        );
        if (currentTitleIdRef.current !== requestedTitleId) {
          return;
        }

        const results = settled.flatMap((result) =>
          result.status === "fulfilled" ? [result.value] : [],
        );

        if (results.length > 0) {
          setEpisodesByCollection((current) => {
            const merged = mergeLoadedEpisodeDetailsForCollections(
              results,
              current,
              episodeDetailsLoadedRef.current,
            );
            let next = current;
            for (const { id, episodes } of results) {
              const nextEpisodes = merged[id] ?? episodes;
              if (JSON.stringify(current[id]) === JSON.stringify(nextEpisodes)) {
                continue;
              }
              if (next === current) {
                next = { ...current };
              }
              next[id] = nextEpisodes;
            }
            return next;
          });
        }
        const failed = settled.find((result) => result.status === "rejected");
        if (failed?.status === "rejected") {
          setGlobalStatus(
            failed.reason instanceof Error ? failed.reason.message : t("status.apiError"),
          );
        }
      } catch (error: unknown) {
        if (currentTitleIdRef.current === requestedTitleId) {
          setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
        }
      } finally {
        for (const collectionId of collectionIdsToRefresh) {
          collectionEpisodesLoadingRef.current.delete(collectionId);
        }
        setCollectionEpisodesLoading((current) => {
          let changed = false;
          const next = { ...current };
          for (const collectionId of collectionIdsToRefresh) {
            changed ||= collectionId in next;
            delete next[collectionId];
          }
          return changed ? next : current;
        });
        for (const collectionId of collectionIdsToRefresh) {
          releaseDeferredCollectionRefresh(collectionId);
        }
      }
    },
    [client, releaseDeferredCollectionRefresh, setGlobalStatus, t, titleId],
  );

  React.useEffect(() => {
    refreshLoadedCollectionEpisodesRef.current = (collectionIds) => {
      void refreshLoadedCollectionEpisodes(collectionIds);
    };
  }, [refreshLoadedCollectionEpisodes]);

  const applySidePanelOverviewSnapshot = React.useCallback(
    (
      snapshot: TitleSidePanelOverviewSnapshot<
        SeriesOverviewSnapshotTitle,
        unknown,
        TitleHistoryEvent,
        TitleReleaseBlocklistEntry,
        ExternalSubtitleRecord
      >,
    ) => {
      // Callers apply a snapshot only while its title is still the one shown.
      const snapshotTitleId = currentTitleIdRef.current;
      if (snapshotTitleId) {
        setDownloadClientsAnswer((current) =>
          current?.titleId === snapshotTitleId &&
          current.configured === snapshot.hasDownloadClients
            ? current
            : { titleId: snapshotTitleId, configured: snapshot.hasDownloadClients },
        );
      }
      const nextTitle = snapshot.title;
      const nextCollections = nextTitle?.collections ?? [];
      const nextSeriesMovieLinks = nextTitle?.seriesMovieLinks ?? [];
      setTitle((current) => {
        const resolvedTitle =
          nextTitle &&
          current &&
          nextTitle.moreLikeThis === undefined &&
          current.id === nextTitle.id
            ? { ...nextTitle, moreLikeThis: current.moreLikeThis }
            : (nextTitle ?? null);
        return retainEquivalentSnapshot(current, resolvedTitle);
      });
      if (nextTitle) {
        onTitleResolved?.({
          id: nextTitle.id,
          slug: nextTitle.slug,
          libraryId: nextTitle.libraryId,
          librarySlug: nextTitle.librarySlug,
        });
      }
      setCollections((current) => retainEquivalentSnapshot(current, nextCollections));
      setSeriesMovieLinks((current) =>
        retainEquivalentSnapshot(current, nextSeriesMovieLinks),
      );
      // Episodes hydrate per collection as seasons are opened; the overview
      // snapshot only prunes cached seasons that no longer exist and refreshes
      // the ones already loaded.
      const nextCollectionIds = new Set(
        nextCollections.map((collection) => collection.id),
      );
      const retainedEpisodesByCollection = Object.fromEntries(
        Object.entries(episodesByCollectionRef.current).filter(([collectionId]) =>
          nextCollectionIds.has(collectionId),
        ),
      );
      const retainedEpisodeIds = episodeIdsForEpisodeRecord(
        retainedEpisodesByCollection,
      );
      // Prune the live state, not the render-phase ref. A snapshot that applies
      // between an episode-detail response and its commit would otherwise write
      // the pre-detail copy back over it, silently dropping that episode's
      // playback links, overview and image until the title is reopened.
      setEpisodesByCollection((current) => {
        const retained = Object.fromEntries(
          Object.entries(current).filter(([collectionId]) =>
            nextCollectionIds.has(collectionId),
          ),
        );
        return retainEquivalentSnapshot(current, retained);
      });
      setEpisodeDetailsLoaded((current) => {
        const retained = new Set<string>();
        for (const episodeId of current) {
          if (retainedEpisodeIds.has(episodeId)) {
            retained.add(episodeId);
          }
        }
        return retained.size === current.size ? current : retained;
      });
      setEpisodeDetailsLoading((current) =>
        pruneEpisodeRecord(current, retainedEpisodeIds),
      );
      setMediaFilesByEpisode((current) =>
        pruneEpisodeRecord(current, retainedEpisodeIds),
      );
      setMediaFilesBySeriesMovieLink((current) =>
        pruneSeriesMovieLinkMediaFiles(current, retainedEpisodeIds),
      );
      void refreshLoadedCollectionEpisodes(Object.keys(retainedEpisodesByCollection));
      if (!nextTitle) {
        setMediaFilesByEpisode({});
        setMediaFilesBySeriesMovieLink({});
        setEpisodeDetailsLoaded(new Set());
        setEpisodeDetailsLoading({});
      }
      setEvents((current) => retainEquivalentSnapshot(current, snapshot.titleHistory));
      setReleaseBlocklistEntries((current) =>
        retainEquivalentSnapshot(current, snapshot.titleReleaseBlocklist),
      );
      setSubtitleDownloads((current) =>
        retainEquivalentSnapshot(current, snapshot.externalSubtitles),
      );
    },
    [onTitleResolved, refreshLoadedCollectionEpisodes],
  );

  useTitleOverviewReactiveRefresh<
    SeriesOverviewSnapshotTitle,
    unknown,
    TitleHistoryEvent,
    TitleReleaseBlocklistEntry,
    ExternalSubtitleRecord
  >({
    titleId,
    blocklistLimit: 300,
    projection: "SERIES",
    applyOverviewSnapshot: applySidePanelOverviewSnapshot,
    importKinds: TITLE_OVERVIEW_IMPORT_REFRESH_KINDS,
    pause: !titleId,
  });

  const refreshTitleMoreLikeThis = React.useCallback(
    async (requestedTitleId: string) => {
      try {
        const moreLikeThis = await fetchTitleMoreLikeThis(
          client,
          requestedTitleId,
        );
        if (currentTitleIdRef.current !== requestedTitleId) {
          return;
        }
        setTitle((current) =>
          current?.id === requestedTitleId
            ? retainEquivalentSnapshot(current, { ...current, moreLikeThis })
            : current,
        );
      } catch (error) {
        if (currentTitleIdRef.current === requestedTitleId) {
          console.error("[series-more-like-this-refresh] refresh failed:", error);
        }
      }
    },
    [client],
  );

  const refreshTitleDetail = React.useCallback(async (
    { refreshMoreLikeThis = false }: { refreshMoreLikeThis?: boolean } = {},
  ) => {
    if (!titleId) {
      return;
    }

    const requestedTitleId = titleId;
    const snapshot = await fetchTitleSidePanelOverviewSnapshot<
      SeriesOverviewSnapshotTitle,
      unknown,
      TitleHistoryEvent,
      TitleReleaseBlocklistEntry,
      ExternalSubtitleRecord
    >(client, requestedTitleId, 300, seriesSidePanelOverviewQuery);
    if (currentTitleIdRef.current !== requestedTitleId) {
      return;
    }
    applySidePanelOverviewSnapshot(snapshot);
    if (!snapshot.title) {
      return;
    }
    if (refreshMoreLikeThis) {
      void refreshTitleMoreLikeThis(requestedTitleId);
    }
  }, [
    applySidePanelOverviewSnapshot,
    client,
    refreshTitleMoreLikeThis,
    titleId,
  ]);
  const reloadTitle = React.useCallback(
    () => refreshTitleDetail(),
    [refreshTitleDetail],
  );
  const reloadTitleAndDownloads = React.useCallback(() => {
    void refreshDownloadFeedback();
    return refreshTitleDetail();
  }, [refreshDownloadFeedback, refreshTitleDetail]);
  const refreshAndScan = useTitleRefreshAndScan({
    titleId: title?.id ?? null,
    onScanned: reloadTitleAndDownloads,
  });
  const blocklistClear = useTitleReleaseBlocklistClear({ onCleared: reloadTitle });
  const handleMoreLikeThisCatalogChanged = React.useCallback(
    () => refreshTitleDetail({ refreshMoreLikeThis: true }),
    [refreshTitleDetail],
  );
  const moreLikeThisActions = useTitleMoreLikeThisActions({
    canAddItems: canAddDiscoveryItems,
    canRequestItems: canRequestDiscoveryItems,
    onCatalogChanged: handleMoreLikeThisCatalogChanged,
  });
  const {
    canAddItem,
    canRequestItem,
    onAction,
    onOpenResolved,
  } = moreLikeThisActions.stripProps;
  const moreLikeThisStripProps = React.useMemo(
    () => ({ canAddItem, canRequestItem, onAction, onOpenResolved }),
    [canAddItem, canRequestItem, onAction, onOpenResolved],
  );
  const refreshTitleDetailRef = React.useRef(refreshTitleDetail);
  React.useEffect(() => {
    refreshTitleDetailRef.current = refreshTitleDetail;
  }, [refreshTitleDetail]);

  const fetchEpisodeDetail = React.useCallback(
    async (episodeId: string) => {
      if (!titleId) {
        return;
      }

      const requestedTitleId = titleId;
      setEpisodeDetailsLoading((current) => ({
        ...current,
        [episodeId]: true,
      }));
      try {
        const { data, error } = await client
          .query(
            episodeSidePanelDetailQuery,
            { titleId: requestedTitleId, episodeId },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }
        if (currentTitleIdRef.current !== requestedTitleId) {
          return;
        }

        const episodeDetail = data?.episode as
          | (Partial<CollectionEpisode> & { mediaFiles?: EpisodeMediaFile[] | null })
          | null
          | undefined;
        const mediaFiles = (episodeDetail?.mediaFiles ?? []) as EpisodeMediaFile[];
        const mediaFilesByEpisode = groupMediaFilesByEpisode(mediaFiles);
        const mediaFilesForEpisode = mediaFilesByEpisode[episodeId] ?? [];
        setEpisodesByCollection((current) =>
          Object.fromEntries(
            Object.entries(current).map(([collectionId, episodes]) => [
              collectionId,
              episodes.map((episode) =>
                episode.id === episodeId
                  ? {
                      ...episode,
                      overview: episodeDetail?.overview ?? episode.overview ?? null,
                      imageUrl: episodeDetail?.imageUrl ?? episode.imageUrl ?? null,
                      playbackLinks:
                        episodeDetail?.playbackLinks ?? episode.playbackLinks ?? [],
                    }
                  : episode,
              ),
            ]),
          ),
        );
        setMediaFilesByEpisode((current) => ({
          ...current,
          [episodeId]: mediaFilesForEpisode,
        }));
        setMediaFilesBySeriesMovieLink((current) => ({
          ...current,
          ...groupMediaFilesBySeriesMovieLink(mediaFiles),
        }));
        setEpisodeDetailsLoaded((current) => {
          const loaded = new Set(current);
          loaded.add(episodeId);
          return loaded;
        });
      } catch (error: unknown) {
        if (currentTitleIdRef.current === requestedTitleId) {
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.apiError"),
          );
        }
      } finally {
        setEpisodeDetailsLoading((current) => {
          const next = { ...current };
          delete next[episodeId];
          return next;
        });
      }
    },
    [client, setGlobalStatus, t, titleId],
  );

  const loadEpisodeDetail = React.useCallback(
    async (episodeId: string) => {
      if (episodeDetailsLoaded.has(episodeId) || episodeDetailsLoading[episodeId]) {
        return;
      }
      await fetchEpisodeDetail(episodeId);
    },
    [episodeDetailsLoaded, episodeDetailsLoading, fetchEpisodeDetail],
  );

  const loadSeriesMovieDetail = React.useCallback(
    async (link: SeriesMovieLink) => {
      if (!titleId) {
        return;
      }
      if (link.linkedEpisodeId) {
        await loadEpisodeDetail(link.linkedEpisodeId);
      }

      const requestedTitleId = titleId;
      const requestKey = `${requestedTitleId}:${link.id}`;
      if (seriesMovieDetailLoadingRef.current.has(requestKey)) {
        return;
      }
      seriesMovieDetailLoadingRef.current.add(requestKey);

      try {
        const { data, error } = await client
          .query(
            movieEntityDetailQuery,
            { titleId: requestedTitleId, movieId: link.movie.id },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }
        if (currentTitleIdRef.current !== requestedTitleId) {
          return;
        }

        const titleDetail = data?.title as
          | { mediaFiles?: EpisodeMediaFile[] | null }
          | null
          | undefined;
        const mediaFiles = (titleDetail?.mediaFiles ?? []) as EpisodeMediaFile[];
        const movieDetail = data?.movieEntity as
          | { id: string; credits?: TitleCreditRecord[] | null }
          | null
          | undefined;
        const linkFiles = mediaFiles.filter((file) =>
          (file.seriesMovieLinkIds ?? []).includes(link.id),
        );
        const knownEpisodeIds = episodeIdsForEpisodeRecord(
          episodesByCollectionRef.current,
        );
        const mediaFilesByEpisode = groupMediaFilesByEpisode(mediaFiles);

        setMediaFilesBySeriesMovieLink((current) => ({
          ...current,
          [link.id]: linkFiles,
        }));
        if (movieDetail) {
          setSeriesMovieLinks((current) =>
            current.map((currentLink) =>
              currentLink.id === link.id
                ? {
                    ...currentLink,
                    movie: {
                      ...currentLink.movie,
                      credits: movieDetail.credits ?? [],
                    },
                  }
                : currentLink,
            ),
          );
        }
        setMediaFilesByEpisode((current) => {
          const next = { ...current };
          for (const [episodeId, files] of Object.entries(mediaFilesByEpisode)) {
            if (episodeId !== "__unlinked__" && knownEpisodeIds.has(episodeId)) {
              next[episodeId] = files;
            }
          }
          return next;
        });
      } catch (error: unknown) {
        if (currentTitleIdRef.current === requestedTitleId) {
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.apiError"),
          );
        }
      } finally {
        seriesMovieDetailLoadingRef.current.delete(requestKey);
      }
    },
    [client, loadEpisodeDetail, setGlobalStatus, t, titleId],
  );

  // A rename moves files without changing their ids, so the files of episodes
  // and linked movies already open are read again once the title has reloaded.
  const reloadRenamedTitle = React.useCallback(
    async (renamedTitle: { id: string }) => {
      if (renamedTitle.id !== titleId) {
        return;
      }
      await refreshTitleDetail();
      const openedMovieLinks = seriesMovieLinks.filter(
        (link) => !link.linkedEpisodeId && link.id in mediaFilesBySeriesMovieLink,
      );
      await Promise.all([
        ...[...episodeDetailsLoaded].map((episodeId) => fetchEpisodeDetail(episodeId)),
        ...openedMovieLinks.map((link) => loadSeriesMovieDetail(link)),
      ]);
    },
    [
      episodeDetailsLoaded,
      fetchEpisodeDetail,
      loadSeriesMovieDetail,
      mediaFilesBySeriesMovieLink,
      refreshTitleDetail,
      seriesMovieLinks,
      titleId,
    ],
  );
  const rename = useTitleRename({
    title,
    canManageTitle,
    onApplied: reloadRenamedTitle,
  });

  // A deep-linked episode may live in a season whose episodes are not loaded
  // yet; resolve its collection and hydrate that season so the view can expand
  // it and scroll to the episode.
  React.useEffect(() => {
    if (!titleId || !initialEpisodeId) {
      return;
    }
    for (const episodes of Object.values(episodesByCollection)) {
      if (episodes.some((episode) => episode.id === initialEpisodeId)) {
        return;
      }
    }
    const attemptKey = `${titleId}:${initialEpisodeId}`;
    if (deepLinkResolveAttemptedRef.current === attemptKey) {
      return;
    }
    deepLinkResolveAttemptedRef.current = attemptKey;
    let cancelled = false;
    void (async () => {
      try {
        const { data, error } = await client
          .query(
            episodeCollectionRefQuery,
            { titleId, episodeId: initialEpisodeId },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }
        if (cancelled || currentTitleIdRef.current !== titleId) {
          return;
        }
        const collectionId = (
          data?.episode as { collectionId?: string | null } | null | undefined
        )?.collectionId;
        if (collectionId) {
          void loadCollectionEpisodes(collectionId);
        }
      } catch {
        // Best-effort: a failed lookup degrades to the default season view.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, episodesByCollection, initialEpisodeId, loadCollectionEpisodes, titleId]);

  React.useEffect(() => {
    let cancelled = false;

    if (!titleId) {
      setTitle(null);
      setCollections([]);
      setSeriesMovieLinks([]);
      setEvents([]);
      setReleaseBlocklistEntries([]);
      setEpisodesByCollection({});
      setCollectionEpisodesLoading({});
      collectionEpisodesLoadingRef.current = new Set();
      deepLinkResolveAttemptedRef.current = null;
      setMediaFilesByEpisode({});
      setSubtitleDownloads([]);
      setTitleLookupAttempted(false);
      setTitleLookupFailed(false);
      setLoading(false);
      return () => {
        cancelled = true;
      };
    }

    setTitleLookupAttempted(false);
    setTitleLookupFailed(false);
    setEpisodesByCollection({});
    setCollectionEpisodesLoading({});
    collectionEpisodesLoadingRef.current = new Set();
    deferredCollectionRefreshRef.current = new Set();
    deepLinkResolveAttemptedRef.current = null;
    setMediaFilesByEpisode({});
    setMediaFilesBySeriesMovieLink({});
    setEpisodeDetailsLoaded(new Set());
    setEpisodeDetailsLoading({});
    setLoading(true);
    refreshTitleDetailRef.current({ refreshMoreLikeThis: true })
      .catch((error: unknown) => {
        if (!cancelled) {
          setTitleLookupFailed(true);
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.apiError"),
          );
        }
      })
      .finally(() => {
        if (!cancelled) {
          setTitleLookupAttempted(true);
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [setGlobalStatus, t, titleId]);

  React.useEffect(() => {
    if (titleId && titleLookupAttempted && !loading && !titleLookupFailed && !title) {
      onTitleNotFound?.();
    }
  }, [loading, onTitleNotFound, title, titleId, titleLookupAttempted, titleLookupFailed]);

  const inferredHydrating = React.useMemo(() => {
    if (!title) {
      return false;
    }

    const metadataFetchedAt = title.metadataFetchedAt ? Date.parse(title.metadataFetchedAt) : NaN;
    const metadataJustHydrated =
      Number.isFinite(metadataFetchedAt) &&
      Date.now() - metadataFetchedAt < 30_000;

    return title.metadataFetchedAt === null || (collections.length === 0 && metadataJustHydrated);
  }, [title, collections.length]);

  const hydrating = inferredHydrating;

  React.useEffect(() => {
    let cancelled = false;
    const load = async () => {
      const libraryId = title?.libraryId;
      if (!libraryId) {
        setLibraries([]);
        return;
      }
      const facet = facetById(title?.facet)?.id;
      if (!facet) {
        setLibraries([]);
        return;
      }
      try {
        const { data, error } = await client
          .query(
            librariesQuery,
            { facet, permission: "MANAGE_TITLES" },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) throw error;
        if (cancelled) return;
        setLibraries(Array.isArray(data.libraries) ? data.libraries : []);
      } catch {
        if (!cancelled) {
          setLibraries([]);
        }
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [client, title?.facet, title?.libraryId]);

  const handleUpdateTitleOptions = React.useCallback(
    async (options: TitleOptionUpdates) => {
      const { error } = await client.mutation(updateTitleMutation, {
        input: { titleId, options },
      }).toPromise();
      if (error) throw error;
      await refreshTitleDetail();
    },
    [titleId, client, refreshTitleDetail],
  );

  const handleSetCollectionMonitored = React.useCallback(
    async (collectionId: string, monitored: boolean) => {
      const { error } = await client.mutation(
        setCollectionMonitoredMutation,
        { input: { collectionId, monitored } },
      ).toPromise();
      if (error) throw error;
      await refreshTitleDetail();
    },
    [client, refreshTitleDetail],
  );

  const handleSetEpisodeMonitored = React.useCallback(
    async (episodeId: string, monitored: boolean) => {
      const { error } = await client.mutation(
        setEpisodeMonitoredMutation,
        { input: { episodeId, monitored } },
      ).toPromise();
      if (error) throw error;
      await refreshTitleDetail();
    },
    [client, refreshTitleDetail],
  );

  const handleSetSeriesMovieMonitored = React.useCallback(
    async (seriesMovieLinkId: string, monitored: boolean) => {
      const { error } = await client.mutation(
        setSeriesMovieMonitoredMutation,
        { input: { seriesMovieLinkId, monitored } },
      ).toPromise();
      if (error) throw error;
      await refreshTitleDetail();
    },
    [client, refreshTitleDetail],
  );

  const handleSetTitleMonitored = React.useCallback(
    async (monitored: boolean) => {
      if (!title) return;
      setMonitoredUpdating(true);
      try {
        const { error } = await client.mutation(setTitleMonitoredMutation, {
          input: { titleId: title.id, monitored },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(
          monitored
            ? t("status.titleMonitoringEnabled")
            : t("status.titleMonitoringDisabled"),
        );
        await refreshTitleDetail();
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setMonitoredUpdating(false);
      }
    },
    [client, refreshTitleDetail, setGlobalStatus, t, title],
  );

  const handleRequestDeleteTitle = React.useCallback(() => {
    setDeleteFilesOnDisk(false);
    setTitleDeleteTypedConfirmation("");
    setDeleteDialogOpen(true);
  }, []);

  const handleFixMatchComplete = React.useCallback(
    async (warnings: string[]) => {
      await applyFixTitleMatchCompletion({
        warnings,
        refreshTitleDetail,
        setGlobalStatus,
        t,
        titleName: title?.name,
      });
    },
    [refreshTitleDetail, setGlobalStatus, t, title?.name],
  );

  const handleCancelDeleteTitle = React.useCallback(() => {
    if (deleteLoading) return;
    setDeleteDialogOpen(false);
    setDeleteFilesOnDisk(false);
    setTitleDeleteTypedConfirmation("");
    setAlsoExcludeFromLists(false);
  }, [deleteLoading]);

  React.useEffect(() => {
    if (!deleteFilesOnDisk) {
      setTitleDeleteTypedConfirmation("");
    }
  }, [deleteFilesOnDisk]);

  const handleConfirmDeleteTitle = React.useCallback(async () => {
    if (!title) return;
    setDeleteLoading(true);
    try {
      const payload: {
        titleId: string;
        deleteFilesOnDisk?: boolean;
        previewFingerprint?: string;
        typedConfirmation?: string;
      } = {
        titleId: title.id,
      };
      if (deleteFilesOnDisk) {
        if (!titleDeletePreview) {
          throw new Error("Delete preview is not ready yet.");
        }
        payload.deleteFilesOnDisk = true;
        payload.previewFingerprint = titleDeletePreview.fingerprint;
        if (titleDeleteTypedConfirmation.trim()) {
          payload.typedConfirmation = titleDeleteTypedConfirmation.trim();
        }
      }

      const { error } = await client.mutation(deleteTitleMutation, {
        input: payload,
      }).toPromise();
      if (error) throw error;

      setGlobalStatus(t("status.titleDeleted", { name: title.name }));
      setDeleteDialogOpen(false);
      setDeleteFilesOnDisk(false);

      // The exclusion is a separate request made only after the delete has
      // succeeded, so it can never change what the delete removes.
      if (canManageLists && alsoExcludeFromLists) {
        setAlsoExcludeFromLists(false);
        const exclusion = exclusionInputFromTitle({
          facet: title.facet as Facet,
          name: title.name,
          year: title.year,
          externalIds: title.externalIds,
        });
        if (!exclusion) {
          setGlobalStatus(t("lists.exclusions.deleteNoIds", { name: title.name }));
        } else {
          const exclusionFailed = await client
            .mutation(addListExclusionMutation, { input: exclusion })
            .toPromise()
            .then((result) => Boolean(result.error))
            .catch(() => true);
          if (exclusionFailed) {
            setGlobalStatus(
              t("lists.exclusions.deleteFailed", { name: title.name }),
            );
          }
        }
      }

      if (onBackToList) {
        onBackToList();
        return;
      }
      onTitleNotFound?.();
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setDeleteLoading(false);
    }
  }, [
    alsoExcludeFromLists,
    canManageLists,
    client,
    deleteFilesOnDisk,
    onBackToList,
    onTitleNotFound,
    titleDeletePreview,
    titleDeleteTypedConfirmation,
    setGlobalStatus,
    t,
    title,
  ]);

  const markEpisodeFileDeletionPending = React.useCallback(
    (targetedEpisodeIds: ReadonlySet<string>) => {
      setPendingEpisodeFileDeletionEpisodeIds((current) => {
        const next = new Set(current);
        for (const episodeId of targetedEpisodeIds) {
          next.add(episodeId);
        }
        return next;
      });
    },
    [],
  );

  // Once a deletion job has run: unlock the rows it held, forget the files it
  // removed, and reload the title. Returns how many files it removed.
  const settleEpisodeFileDeletion = React.useCallback(
    async (run: JobRun, targetedEpisodeIds: ReadonlySet<string>) => {
      setPendingEpisodeFileDeletionEpisodeIds((current) => {
        const next = new Set(current);
        for (const episodeId of targetedEpisodeIds) {
          next.delete(episodeId);
        }
        return next;
      });

      // The run's summary names the files it actually removed. When it is
      // missing or unparseable, drop the cached files for every targeted
      // episode instead so nothing stale is shown.
      const deletedFileIds = readDeletedFileIds(run.summaryJson);
      const dropCachedFiles = (
        current: Record<string, EpisodeMediaFile[]>,
      ): Record<string, EpisodeMediaFile[]> => {
        if (deletedFileIds) {
          return Object.fromEntries(
            Object.entries(current).map(([key, files]) => [
              key,
              files.filter((file) => !deletedFileIds.has(file.id)),
            ]),
          );
        }
        return Object.fromEntries(
          Object.entries(current).filter(([key]) => !targetedEpisodeIds.has(key)),
        );
      };
      setMediaFilesByEpisode(dropCachedFiles);
      setMediaFilesBySeriesMovieLink(dropCachedFiles);
      await refreshTitleDetail();
      return deletedFileIds?.size ?? targetedEpisodeIds.size;
    },
    [refreshTitleDetail],
  );

  // Deleting one file accepts a background job exactly like deleting a whole
  // episode's files does. The row's availability, its quality pill included,
  // only changes once that job has run, so the shared deletion waits for the
  // run to finish rather than refetching a title the job has not touched yet.
  const mediaFileDeletion = useMediaFileDeletion<EpisodeMediaFile, Set<string>>({
    captureContext: (file) =>
      mediaFileOwnerKeys(file.id, [mediaFilesByEpisode, mediaFilesBySeriesMovieLink]),
    onQueued: (_file, targetedEpisodeIds) =>
      markEpisodeFileDeletionPending(targetedEpisodeIds),
    onFinished: async (run, _file, targetedEpisodeIds) => {
      await settleEpisodeFileDeletion(run, targetedEpisodeIds);
    },
  });
  const requestMediaFileDelete = mediaFileDeletion.requestDelete;
  const handleDeleteMediaFile = React.useCallback(
    (fileId: string) => {
      const file = [
        ...Object.values(mediaFilesByEpisode),
        ...Object.values(mediaFilesBySeriesMovieLink),
      ]
        .flat()
        .find((candidate) => candidate.id === fileId);
      if (file) {
        requestMediaFileDelete(file);
      }
    },
    [mediaFilesByEpisode, mediaFilesBySeriesMovieLink, requestMediaFileDelete],
  );

  const handleRequestDeleteEpisodeFiles = React.useCallback((episodeIds: string[]) => {
    if (episodeIds.length === 0) return;
    setEpisodeFilesDeleteTypedConfirmation("");
    setEpisodeFilesToDelete([...episodeIds].sort());
  }, []);

  const handleCancelDeleteEpisodeFiles = React.useCallback(() => {
    if (episodeFilesDeleteLoading) return;
    setEpisodeFilesToDelete(null);
    setEpisodeFilesDeleteTypedConfirmation("");
  }, [episodeFilesDeleteLoading]);

  const handleConfirmDeleteEpisodeFiles = React.useCallback(async () => {
    if (!title || !episodeFilesToDelete || !episodeFilesDeletePreview) return;
    const requestedEpisodeIds = episodeFilesToDelete;
    // Captured before the request so the terminal handler can restore exactly
    // the rows this run locked, and drop their cached files if the run does not
    // report which files it removed.
    const targetedEpisodeIds = episodeIdsCoveredByEpisodeFileDelete(
      episodeFilesDeletePreviewPayload?.items ?? [],
    );
    setEpisodeFilesDeleteLoading(true);
    try {
      const { data, error } = await client.mutation<{
        deleteEpisodeFiles?: { acceptedFileIds?: string[]; jobRun?: unknown };
      }>(deleteEpisodeFilesMutation, {
        input: {
          titleId: title.id,
          episodeIds: requestedEpisodeIds,
          deleteFromDisk: true,
          previewFingerprint: episodeFilesDeletePreview.fingerprint,
          typedConfirmation: episodeFilesDeleteTypedConfirmation.trim() || undefined,
        },
      }).toPromise();
      if (error) throw error;

      const run = normalizeJobRun(data?.deleteEpisodeFiles?.jobRun);
      if (!run) {
        throw new Error(t("status.apiError"));
      }
      const acceptedFileIds = data?.deleteEpisodeFiles?.acceptedFileIds ?? [];

      markEpisodeFileDeletionPending(targetedEpisodeIds);
      trackJobRun(run, (terminalRun) => {
        void settleEpisodeFileDeletion(terminalRun, targetedEpisodeIds).then(
          (deletedCount) => {
            setGlobalStatus(
              terminalRun.status === "COMPLETED"
                ? t("status.episodeFilesDeleted", { count: deletedCount })
                : (terminalRun.errorText ??
                    terminalRun.summaryText ??
                    t("status.apiError")),
            );
          },
        );
      });

      setGlobalStatus(
        t("status.episodeFilesDeleteQueued", { count: acceptedFileIds.length }),
      );
      setEpisodeSelectionResetToken((current) => current + 1);
      setEpisodeFilesToDelete(null);
      setEpisodeFilesDeleteTypedConfirmation("");
    } catch (error: unknown) {
      setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.apiError")));
    } finally {
      setEpisodeFilesDeleteLoading(false);
    }
  }, [
    client,
    episodeFilesDeletePreview,
    episodeFilesDeletePreviewPayload,
    episodeFilesDeleteTypedConfirmation,
    episodeFilesToDelete,
    markEpisodeFileDeletionPending,
    setGlobalStatus,
    settleEpisodeFileDeletion,
    t,
    title,
    trackJobRun,
  ]);

  const handleMakePrimaryMovieFile = React.useCallback(
    async (fileId: string) => {
      if (!title) return;
      setPrimaryMovieFileUpdatingId(fileId);
      try {
        const { error } = await client.mutation(setPrimaryMovieFileMutation, {
          input: {
            titleId: title.id,
            fileId,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.primaryMovieFileUpdated"));
        await refreshTitleDetail();
      } catch (error: unknown) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.apiError")));
      } finally {
        setPrimaryMovieFileUpdatingId(null);
      }
    },
    [client, refreshTitleDetail, setGlobalStatus, t, title],
  );

  const deleteTitleConfirmDisabled =
    deleteFilesOnDisk &&
    (titleDeletePreviewLoading ||
      !!titleDeletePreviewError ||
      !titleDeletePreview ||
      (titleDeletePreview.requiresTypedConfirmation &&
        titleDeleteTypedConfirmation.trim() !== "DELETE"));
  // Selected episodes without any media file contribute nothing to the delete,
  // so the summary counts the episodes the preview actually resolved files for,
  // including every episode a shared multi-episode file covers.
  const episodeFilesDeleteEpisodeCount = React.useMemo(
    () =>
      episodeIdsCoveredByEpisodeFileDelete(episodeFilesDeletePreviewPayload?.items ?? []).size,
    [episodeFilesDeletePreviewPayload],
  );
  const deleteEpisodeFilesConfirmDisabled =
    episodeFilesDeletePreviewLoading ||
    !!episodeFilesDeletePreviewError ||
    !episodeFilesDeletePreview ||
    (episodeFilesDeletePreviewPayload?.fileCount ?? 0) === 0 ||
    (episodeFilesDeletePreview.requiresTypedConfirmation &&
      episodeFilesDeleteTypedConfirmation.trim() !== "DELETE");

  const handleAutoSearchEpisode = React.useCallback(
    async (episode: CollectionEpisode) => {
      if (!title) return;

      try {
        const input = {
          titleId: title.id,
          scope: { episode: episode.id },
        };
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client.mutation(queueBestReleaseMutation, {
              input: nextInput,
            }).toPromise();
            if (error) throw error;
            return data?.queueBestRelease;
          },
          "A download is already in progress for this episode.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this episode.");
        setGlobalStatus(t("status.queuedLatest", { name: title.name }));
        await refreshTitleDetail();
      } catch (error: unknown) {
        reportAutomaticSearchFailure(setGlobalStatus, t, error, title.name);
      }
    },
    [refreshTitleDetail, client, confirmReplaceConflict, title, t, setGlobalStatus],
  );

  const handleAutoSearchSeriesMovie = React.useCallback(
    async (link: SeriesMovieLink) => {
      if (!title) return;
      try {
        const input = {
          titleId: title.id,
          scope: { seriesMovie: link.id },
        };
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client.mutation(queueBestReleaseMutation, {
              input: nextInput,
            }).toPromise();
            if (error) throw error;
            return data?.queueBestRelease;
          },
          "A download is already in progress for this series movie.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this series movie.");
        setGlobalStatus(t("status.queuedLatest", { name: link.movie.title }));
        await refreshTitleDetail();
      } catch (error: unknown) {
        reportAutomaticSearchFailure(setGlobalStatus, t, error, link.movie.title);
      }
    },
    [refreshTitleDetail, client, confirmReplaceConflict, title, t, setGlobalStatus],
  );

  const [seasonSearchResultsByCollection, setSeasonSearchResultsByCollection] =
    React.useState<Record<string, Release[]>>({});
  const [
    seasonInteractiveSearchLoadingByCollection,
    setSeasonInteractiveSearchLoadingByCollection,
  ] = React.useState<Record<string, boolean>>({});
  const seasonSearchAbortByCollectionRef = React.useRef<Record<string, AbortController>>({});
  const seasonSearchTitleId = title?.id ?? null;
  React.useEffect(() => {
    setSeasonSearchResultsByCollection({});
    setSeasonInteractiveSearchLoadingByCollection({});
    return () => {
      Object.values(seasonSearchAbortByCollectionRef.current).forEach((controller) =>
        controller.abort(),
      );
      seasonSearchAbortByCollectionRef.current = {};
    };
  }, [seasonSearchTitleId]);
  const [seasonSearchLoadingByCollection, setSeasonSearchLoadingByCollection] = React.useState<
    Record<string, boolean>
  >({});

  const handleRunSeasonSearch = React.useCallback(
    async (collection: TitleCollection) => {
      if (!title) return;
      const seasonNum = parseSearchSeason(collection.collectionIndex);
      if (seasonNum === null) {
        setGlobalStatus(t("wanted.searchInvalidSeason"));
        return;
      }

      setSeasonSearchLoadingByCollection((prev) => ({ ...prev, [collection.id]: true }));
      try {
        await startAutomaticSearch(title.id, seasonNum);
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setSeasonSearchLoadingByCollection((prev) => ({ ...prev, [collection.id]: false }));
      }
    },
    [startAutomaticSearch, setGlobalStatus, t, title],
  );

  // An interactive search of the whole season. Its releases carry candidate
  // tokens already bound to what each one covers, so a grab queues them
  // directly with the season's collection as the fallback scope.
  const handleRunSeasonInteractiveSearch = React.useCallback(
    (collection: TitleCollection) => {
      if (!title) return;
      const seasonNum = parseSearchSeason(collection.collectionIndex);
      if (seasonNum === null) {
        setGlobalStatus(t("wanted.searchInvalidSeason"));
        return;
      }

      const collectionId = collection.id;
      seasonSearchAbortByCollectionRef.current[collectionId]?.abort();
      const abortController = new AbortController();
      seasonSearchAbortByCollectionRef.current[collectionId] = abortController;
      setSeasonSearchResultsByCollection((prev) => ({ ...prev, [collectionId]: [] }));
      setSeasonInteractiveSearchLoadingByCollection((prev) => ({ ...prev, [collectionId]: true }));

      runIterativeReleaseSearch(
        client,
        titleReleaseSearchInput(title.id, { kind: "season", season: String(seasonNum) }),
        {
          signal: abortController.signal,
          onUpdate: (snapshot) => {
            if (abortController.signal.aborted) return;
            setSeasonSearchResultsByCollection((prev) => ({
              ...prev,
              [collectionId]: snapshot.releases,
            }));
          },
        },
      )
        .catch((error: unknown) => {
          if (isAbortError(error) || abortController.signal.aborted) return;
          setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.apiError")), {
            level: "ERROR",
          });
        })
        .finally(() => {
          if (seasonSearchAbortByCollectionRef.current[collectionId] === abortController) {
            delete seasonSearchAbortByCollectionRef.current[collectionId];
            setSeasonInteractiveSearchLoadingByCollection((prev) => ({
              ...prev,
              [collectionId]: false,
            }));
          }
        });
    },
    [client, setGlobalStatus, t, title],
  );

  const handleQueueFromSeasonSearch = React.useCallback(
    async (collection: TitleCollection, release: Release) => {
      if (!title) return;
      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }
      try {
        const input = {
          titleId: title.id,
          scope: releaseQueueScopeInput(release, { collection: collection.id }),
          candidateToken: release.candidateToken,
        };
        const replacesPrimary = queueScopeReplacesPrimary(
          input.scope,
          episodesByCollection,
          mediaFilesByEpisode,
        );
        const mutation = replacesPrimary
          ? queueReplacementMutation
          : queueExistingMutation;
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client
              .mutation(mutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return replacesPrimary
              ? data?.queueReplacementRelease
              : data?.queueExistingTitleDownload;
          },
          "A download is already in progress for this collection.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this collection.");
        setGlobalStatus(t("status.queuedLatest", { name: title.name }));
        await refreshTitleDetail();
      } catch (error: unknown) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")), { level: "ERROR" });
      }
    },
    [
      client,
      confirmReplaceConflict,
      episodesByCollection,
      mediaFilesByEpisode,
      title,
      refreshTitleDetail,
      setGlobalStatus,
      t,
    ],
  );

  const manualImportTitle = React.useMemo(
    () =>
      title ? { id: title.id, name: title.name, facet: title.facet } : null,
    [title],
  );
  const manualImport = useManualImportLauncher({
    title: manualImportTitle,
    onImportQueued: reloadTitleAndDownloads,
  });
  const handleOpenFixMatch = React.useCallback(() => {
    setFixMatchOpen(true);
  }, []);

  return (
    <>
      <SeriesOverviewView
        canManageTitle={canManageTitle}
        fullBleedHero={fullBleedHero}
        loading={loading}
        hydrating={hydrating}
        title={title}
        collections={collections}
        seriesMovieLinks={seriesMovieLinks}
        events={events}
        episodesByCollection={episodesByCollection}
        collectionEpisodesLoading={collectionEpisodesLoading}
        onLoadCollectionEpisodes={loadCollectionEpisodes}
        mediaFilesByEpisode={mediaFilesByEpisode}
        mediaFilesBySeriesMovieLink={mediaFilesBySeriesMovieLink}
        onLoadEpisodeDetail={loadEpisodeDetail}
        onLoadSeriesMovieDetail={loadSeriesMovieDetail}
        subtitleDownloads={subtitleDownloads}
        onRefreshSubtitles={refreshTitleDetail}
        releaseBlocklistEntries={releaseBlocklistEntries}
        clearingReleaseBlocklistEntryId={blocklistClear.clearingEntryId}
        onClearReleaseBlocklistEntry={blocklistClear.clear}
        onTitleChanged={refreshTitleDetail}
        onBackToList={onBackToList}
        onSetCollectionMonitored={handleSetCollectionMonitored}
        onSetEpisodeMonitored={handleSetEpisodeMonitored}
        onSetSeriesMovieMonitored={handleSetSeriesMovieMonitored}
        onSeriesMovieTagsChanged={refreshTitleDetail}
        onSetTitleMonitored={handleSetTitleMonitored}
        onSearchMonitored={searchAction.search}
        onAutoSearchEpisode={handleAutoSearchEpisode}
        onAutoSearchSeriesMovie={handleAutoSearchSeriesMovie}
        downloadQueueItems={downloadQueueItems}
        hasDownloadClients={hasDownloadClients !== false}
        showSearchPrerequisiteNotice={searchAction.showDownloadClientNotice}
        rename={rename}
        libraries={libraries}
        onUpdateTitleOptions={handleUpdateTitleOptions}
        completedDownloads={completedDownloads}
        manualImport={manualImport}
        initialEpisodeId={initialEpisodeId}
        seasonSearchResultsByCollection={seasonSearchResultsByCollection}
        seasonSearchLoadingByCollection={Object.fromEntries(collections.map((collection) => {
          const season = parseSearchSeason(collection.collectionIndex);
          return [collection.id, seasonSearchLoadingByCollection[collection.id] === true ||
            (!!title && season !== null && isSearching(title.id, season))];
        }))}
        onRunSeasonSearch={handleRunSeasonSearch}
        seasonInteractiveSearchLoadingByCollection={seasonInteractiveSearchLoadingByCollection}
        onRunSeasonInteractiveSearch={handleRunSeasonInteractiveSearch}
        onQueueFromSeasonSearch={handleQueueFromSeasonSearch}
        monitoredUpdating={monitoredUpdating}
        searchMonitoredLoading={searchAction.searching}
        onRefreshAndScan={refreshAndScan.refreshAndScan}
        refreshAndScanLoading={refreshAndScan.loading}
        onRequestDeleteTitle={handleRequestDeleteTitle}
        deleteLoading={deleteLoading}
        onDeleteFile={handleDeleteMediaFile}
        onRequestDeleteEpisodeFiles={handleRequestDeleteEpisodeFiles}
        episodeSelectionResetToken={episodeSelectionResetToken}
        pendingEpisodeIds={pendingEpisodeFileDeletionEpisodeIds}
        onMakePrimaryFile={canManageTitle ? handleMakePrimaryMovieFile : undefined}
        primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
        onOpenFixMatch={handleOpenFixMatch}
        moreLikeThisActions={moreLikeThisStripProps}
      />
      {moreLikeThisActions.dialogs}
      <FixTitleMatchDialog
        open={fixMatchOpen}
        onOpenChange={setFixMatchOpen}
        title={title}
        onFixed={handleFixMatchComplete}
      />
      <ConfirmDialog
        open={deleteDialogOpen && title !== null}
        title={t("label.delete")}
        description={
          title
            ? t("status.deleteCatalogConfirm", { name: title.name })
            : t("label.delete")
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        contentId="title-delete-dialog"
        confirmButtonId="title-delete-confirm"
        cancelButtonId="title-delete-cancel"
        isBusy={deleteLoading}
        confirmDisabled={deleteTitleConfirmDisabled}
        onConfirm={handleConfirmDeleteTitle}
        onCancel={handleCancelDeleteTitle}
      >
        <div className="space-y-3">
          <label className="flex items-center gap-2">
            <Checkbox
              id="title-delete-files-on-disk"
              checked={deleteFilesOnDisk}
              onCheckedChange={(checked) => setDeleteFilesOnDisk(checked === true)}
              disabled={deleteLoading}
            />
            <span className="text-sm text-muted-foreground">{t("title.deleteFilesOnDisk")}</span>
          </label>
          {canManageLists ? (
            <label className="flex items-center gap-2">
              <Checkbox
                id="title-delete-exclude-from-lists"
                checked={alsoExcludeFromLists}
                onCheckedChange={(checked) => setAlsoExcludeFromLists(checked === true)}
                disabled={deleteLoading}
              />
              <span className="text-sm text-muted-foreground">
                {t("lists.exclusions.deleteCheckbox")}
              </span>
            </label>
          ) : null}
          {deleteFilesOnDisk ? (
            <DeletePreviewSummary
              preview={titleDeletePreview}
              loading={titleDeletePreviewLoading}
              error={titleDeletePreviewError}
              typedConfirmation={titleDeleteTypedConfirmation}
              onTypedConfirmationChange={setTitleDeleteTypedConfirmation}
              typedConfirmationPromptId="title-delete-typed-confirmation-prompt"
              typedConfirmationInputId="title-delete-typed-confirmation"
            />
          ) : null}
        </div>
      </ConfirmDialog>
      {mediaFileDeletion.dialog}
      <ConfirmDialog
        open={episodeFilesToDelete !== null}
        title={t("seriesOverview.deleteEpisodeFilesTitle")}
        description={t("seriesOverview.deleteEpisodeFilesDescription")}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={episodeFilesDeleteLoading}
        confirmDisabled={deleteEpisodeFilesConfirmDisabled}
        onConfirm={handleConfirmDeleteEpisodeFiles}
        onCancel={handleCancelDeleteEpisodeFiles}
      >
        <div className="space-y-3">
          <p
            id="series-overview-delete-episode-files-summary"
            className="text-sm text-muted-foreground"
          >
            {t("seriesOverview.deleteEpisodeFilesSummary", {
              files: episodeFilesDeletePreviewPayload?.fileCount ?? 0,
              episodes: episodeFilesDeleteEpisodeCount,
            })}
          </p>
          {(episodeFilesDeletePreviewPayload?.failedCount ?? 0) > 0 ? (
            <p
              id="series-overview-delete-episode-files-preview-failures"
              className="text-sm text-destructive"
            >
              {t("seriesOverview.deleteEpisodeFilesPreviewFailures", {
                count: episodeFilesDeletePreviewPayload?.failedCount ?? 0,
              })}
            </p>
          ) : null}
          <DeletePreviewSummary
            preview={episodeFilesDeletePreview}
            loading={episodeFilesDeletePreviewLoading}
            error={episodeFilesDeletePreviewError}
            typedConfirmation={episodeFilesDeleteTypedConfirmation}
            onTypedConfirmationChange={setEpisodeFilesDeleteTypedConfirmation}
            typedConfirmationPromptId="series-overview-delete-episode-files-typed-confirmation-prompt"
            typedConfirmationInputId="series-overview-delete-episode-files-typed-confirmation"
          />
        </div>
      </ConfirmDialog>
      {manualImport.dialog}
      {replaceConflictDialog}
    </>
  );
});
