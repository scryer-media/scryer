
import * as React from "react";
import {
  activitySubscriptionQuery,
  searchQuery,
  searchForEpisodeQuery,
  seriesOverviewSettingsInitQuery,
  titleOverviewInitQuery,
} from "@/lib/graphql/queries";
import {
  deleteMediaFileMutation,
  deleteTitleMutation,
  scanTitleLibraryMutation,
  setCollectionMonitoredMutation,
  queueExistingMutation,
  setEpisodeMonitoredMutation,
  setTitleMonitoredMutation,
  triggerTitleWantedSearchMutation,
  triggerSeasonWantedSearchMutation,
  updateTitleMutation,
} from "@/lib/graphql/mutations";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import type { Release } from "@/lib/types";
import { DEFAULT_SERIES_LIBRARY_PATH } from "@/lib/constants/settings";
import { qualityProfileSettingsToEntries } from "@/lib/utils/quality-profiles";
import {
  collectActivityEventsFromPayload,
  normalizeActivityEvent,
} from "@/lib/utils/activity";
import { useClient, useSubscription } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useImportHistorySubscription } from "@/lib/hooks/use-import-history-subscription";
import { SeriesOverviewView } from "@/components/views/series-overview-view";
import { ManualImportDialog } from "@/components/dialogs/manual-import-dialog";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Checkbox } from "@/components/ui/checkbox";
import type { TitleOptionUpdates } from "@/lib/types/title-options";

export type TitleDetail = {
  id: string;
  name: string;
  facet: string;
  monitored: boolean;
  tags: string[];
  externalIds: { source: string; value: string }[];
  year: number | null;
  overview: string | null;
  posterUrl: string | null;
  posterSourceUrl: string | null;
  bannerUrl: string | null;
  backgroundUrl: string | null;
  sortTitle: string | null;
  slug: string | null;
  imdbId: string | null;
  runtimeMinutes: number | null;
  genres: string[];
  contentStatus: string | null;
  language: string | null;
  firstAired: string | null;
  network: string | null;
  studio: string | null;
  country: string | null;
  aliases: string[];
  metadataLanguage: string | null;
  metadataFetchedAt: string | null;
  qualityProfileId?: string | null;
  rootFolderPath?: string | null;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
  downloadQueueItems?: DownloadQueueItem[];
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
  fileSizeBytes: number | null;
  firstEpisodeNumber: string | null;
  lastEpisodeNumber: string | null;
  interstitialMovie: InterstitialMovieMetadata | null;
  specialsMovies: InterstitialMovieMetadata[];
  interstitialSeasonEpisode: string | null;
  monitored: boolean;
  episodes?: CollectionEpisode[];
  createdAt: string;
};

export type InterstitialMovieMetadata = {
  tvdbId: string;
  name: string;
  slug: string;
  year: number | null;
  contentStatus: string;
  overview: string;
  posterUrl: string;
  language: string;
  runtimeMinutes: number;
  sortTitle: string;
  imdbId: string;
  genres: string[];
  studio: string;
  digitalReleaseDate: string | null;
  associationConfidence: string | null;
  continuityStatus: string | null;
  movieForm: string | null;
  confidence: string | null;
  signalSummary: string | null;
  placement: string | null;
  movieTmdbId: string | null;
  movieMalId: string | null;
};

import type { TitleHistoryEvent } from "@/lib/types";
export type { TitleHistoryEvent };

export type TitleReleaseBlocklistEntry = {
  sourceHint: string | null;
  sourceTitle: string | null;
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
  overview: string | null;
  airDate: string | null;
  durationSeconds: number | null;
  hasMultiAudio: boolean;
  hasSubtitle: boolean;
  isFiller: boolean;
  isRecap: boolean;
  absoluteNumber: string | null;
  monitored: boolean;
  createdAt: string;
};

export type EpisodeMediaFile = {
  id: string;
  titleId: string;
  episodeId: string | null;
  filePath: string;
  sizeBytes: string;
  qualityLabel: string | null;
  scanStatus: string;
  createdAt: string;
  videoCodec: string | null;
  videoWidth: number | null;
  videoHeight: number | null;
  videoBitrateKbps: number | null;
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

type SeriesOverviewContainerProps = {
  titleId: string;
  onTitleNotFound?: () => void;
  onBackToList?: () => void;
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

export const SeriesOverviewContainer = React.memo(function SeriesOverviewContainer({
  titleId,
  onTitleNotFound,
  onBackToList,
  initialEpisodeId,
}: SeriesOverviewContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [title, setTitle] = React.useState<TitleDetail | null>(null);
  const [collections, setCollections] = React.useState<TitleCollection[]>([]);
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
  const [releaseBlocklistEntries, setReleaseBlocklistEntries] = React.useState<
    TitleReleaseBlocklistEntry[]
  >([]);
  const [loading, setLoading] = React.useState(true);
  const [episodesByCollection, setEpisodesByCollection] = React.useState<
    Record<string, CollectionEpisode[]>
  >({});
  const [qualityProfiles, setQualityProfiles] = React.useState<{ id: string; name: string }[]>([]);
  const [defaultRootFolder, setDefaultRootFolder] = React.useState(DEFAULT_SERIES_LIBRARY_PATH);
  const [rootFolders, setRootFolders] = React.useState<{ path: string; isDefault: boolean }[]>([]);
  const [mediaFilesByEpisode, setMediaFilesByEpisode] = React.useState<
    Record<string, EpisodeMediaFile[]>
  >({});
  const [subtitleDownloads, setSubtitleDownloads] = React.useState<
    import("@/components/containers/movie-overview-container").SubtitleDownloadRecord[]
  >([]);
  const [completedDownloads, setCompletedDownloads] = React.useState<DownloadQueueItem[]>([]);
  const [manualImportItem, setManualImportItem] = React.useState<DownloadQueueItem | null>(null);
  const [hydratingFromActivity, setHydratingFromActivity] = React.useState(false);
  const [monitoredUpdating, setMonitoredUpdating] = React.useState(false);
  const [searchMonitoredLoading, setSearchMonitoredLoading] = React.useState(false);
  const [refreshAndScanLoading, setRefreshAndScanLoading] = React.useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = React.useState(false);
  const [deleteFilesOnDisk, setDeleteFilesOnDisk] = React.useState(false);
  const [deleteLoading, setDeleteLoading] = React.useState(false);

  const refreshTitleDetail = React.useCallback(async (_options?: { quiet?: boolean }) => {
    const { data, error } = await client.query(titleOverviewInitQuery, { id: titleId, blocklistLimit: 300 }, { requestPolicy: "network-only" }).toPromise();
    if (error) throw error;
    const nextTitle = data.title ?? null;
    const nextCollections = nextTitle?.collections ?? [];
    const nextMediaFiles = nextTitle?.mediaFiles ?? [];
    const nextDownloadQueueItems = nextTitle?.downloadQueueItems ?? [];
    setTitle(nextTitle);
    setCollections(nextCollections);
    setEpisodesByCollection(
      Object.fromEntries(
        nextCollections.map((collection: TitleCollection) => [
          collection.id,
          collection.episodes ?? [],
        ]),
      ),
    );
    setMediaFilesByEpisode(groupMediaFilesByEpisode(nextMediaFiles));
    setEvents(data.titleEvents ?? []);
    setReleaseBlocklistEntries(data.titleReleaseBlocklist ?? []);
    setSubtitleDownloads(data.subtitleDownloads ?? []);
    setCompletedDownloads(
      nextDownloadQueueItems.filter(
        (item: DownloadQueueItem) => item.state === "completed" || item.state === "import_pending",
      ),
    );
  }, [titleId, client]);

  React.useEffect(() => {
    let cancelled = false;
    setLoading(true);
    refreshTitleDetail()
      .catch((error: unknown) => {
        if (!cancelled) {
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.apiError"),
          );
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [refreshTitleDetail, setGlobalStatus, t]);

  React.useEffect(() => {
    if (!loading && !title) {
      onTitleNotFound?.();
    }
  }, [loading, title, onTitleNotFound]);

  React.useEffect(() => {
    setHydratingFromActivity(false);
  }, [titleId]);

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

  const hydrating = inferredHydrating || hydratingFromActivity;

  React.useEffect(() => {
    if (!inferredHydrating && hydratingFromActivity) {
      setHydratingFromActivity(false);
    }
  }, [inferredHydrating, hydratingFromActivity]);

  // Fetch quality profile catalog and default root folder
  React.useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client.query(seriesOverviewSettingsInitQuery, {
          scope: title?.facet === "anime" ? "anime" : "series",
        }).toPromise();
        if (error) throw error;
        if (cancelled) return;
        setQualityProfiles(
          qualityProfileSettingsToEntries(data.qualityProfileSettings).map((profile) => ({
            id: profile.id,
            name: profile.name,
          })),
        );
        const folder = (data.mediaSettings?.libraryPath ?? "").trim();
        if (folder) setDefaultRootFolder(folder);
        if (Array.isArray(data.mediaSettings?.rootFolders)) {
          setRootFolders(data.mediaSettings.rootFolders);
        }
      } catch {
        // Settings fetch is best-effort
      }
    };
    void load();
    return () => { cancelled = true; };
  }, [client, title?.facet]);

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
      const { error, data } = await client.mutation(
        setCollectionMonitoredMutation,
        { input: { collectionId, monitored } },
      ).toPromise();
      if (error) throw error;
      const payload = data?.setCollectionMonitored;
      if (!payload) return;
      // Update collection monitored flag from the mutation response.
      setCollections((prev) =>
        prev.map((c) => (
          c.id === payload.id
            ? { ...c, monitored: payload.monitored, episodes: payload.episodes ?? c.episodes }
            : c
        )),
      );
      // Update episode monitored flags from the projected episodes.
      if (payload.episodes) {
        setEpisodesByCollection((prev) => ({
          ...prev,
          [collectionId]: payload.episodes,
        }));
      }
    },
    [client],
  );

  const handleSetEpisodeMonitored = React.useCallback(
    async (episodeId: string, monitored: boolean) => {
      const { error } = await client.mutation(
        setEpisodeMonitoredMutation,
        { input: { episodeId, monitored } },
      ).toPromise();
      if (error) throw error;
      // Update just this episode from the mutation response.
      setEpisodesByCollection((prev) => {
        for (const [cid, episodes] of Object.entries(prev)) {
          const idx = episodes.findIndex((ep) => ep.id === episodeId);
          if (idx !== -1) {
            return {
              ...prev,
              [cid]: episodes.map((ep) =>
                ep.id === episodeId ? { ...ep, monitored } : ep,
              ),
            };
          }
        }
        return prev;
      });
    },
    [client],
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

  const handleSearchMonitored = React.useCallback(async () => {
    if (!title) return;

    setSearchMonitoredLoading(true);
    try {
      const { data, error } = await client.mutation(triggerTitleWantedSearchMutation, {
        input: { titleId: title.id },
      }).toPromise();
      if (error) throw error;

      const queued = data?.triggerTitleWantedSearch ?? 0;
      setGlobalStatus(
        queued > 0
          ? t("status.searchMonitoredQueued", { count: queued })
          : t("status.searchMonitoredEmpty"),
      );
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    } finally {
      setSearchMonitoredLoading(false);
    }
  }, [client, setGlobalStatus, t, title]);

  const handleDeleteMediaFile = React.useCallback(async (fileId: string) => {
    try {
      const { error } = await client.mutation(deleteMediaFileMutation, {
        input: { fileId, deleteFromDisk: true },
      }).toPromise();
      if (error) throw error;
      await refreshTitleDetail();
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    }
  }, [client, refreshTitleDetail, setGlobalStatus, t]);

  const handleRefreshAndScan = React.useCallback(async () => {
    if (!title) return;

    setRefreshAndScanLoading(true);
    try {
      const { data, error } = await client.mutation(scanTitleLibraryMutation, {
        input: { titleId: title.id },
      }).toPromise();
      if (error) throw error;

      const summary = data?.scanTitleLibrary;
      setGlobalStatus(
        t("status.titleScanSuccess", {
          imported: summary?.imported ?? 0,
          skipped: summary?.skipped ?? 0,
          unmatched: summary?.unmatched ?? 0,
        }),
      );
      await refreshTitleDetail();
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("settings.libraryScanFailed"));
    } finally {
      setRefreshAndScanLoading(false);
    }
  }, [client, refreshTitleDetail, setGlobalStatus, t, title]);

  const handleRequestDeleteTitle = React.useCallback(() => {
    setDeleteFilesOnDisk(false);
    setDeleteDialogOpen(true);
  }, []);

  const handleCancelDeleteTitle = React.useCallback(() => {
    if (deleteLoading) return;
    setDeleteDialogOpen(false);
    setDeleteFilesOnDisk(false);
  }, [deleteLoading]);

  const handleConfirmDeleteTitle = React.useCallback(async () => {
    if (!title) return;
    setDeleteLoading(true);
    try {
      const payload: { titleId: string; deleteFilesOnDisk?: boolean } = {
        titleId: title.id,
      };
      if (deleteFilesOnDisk) {
        payload.deleteFilesOnDisk = true;
      }

      const { error } = await client.mutation(deleteTitleMutation, {
        input: payload,
      }).toPromise();
      if (error) throw error;

      setGlobalStatus(t("status.titleDeleted", { name: title.name }));
      setDeleteDialogOpen(false);
      setDeleteFilesOnDisk(false);

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
    client,
    deleteFilesOnDisk,
    onBackToList,
    onTitleNotFound,
    setGlobalStatus,
    t,
    title,
  ]);

  const handleAutoSearchEpisode = React.useCallback(
    async (episode: CollectionEpisode) => {
      if (!title) return;

      const collection = collections.find((item) => item.id === episode.collectionId);
      const seasonNum =
        episode.seasonNumber?.trim().replace(/\D+/g, "") ||
        collection?.collectionIndex?.trim().replace(/\D+/g, "") ||
        "1";
      const episodeNum = episode.episodeNumber?.trim().replace(/\D+/g, "") || "1";

      const { data: payload, error } = await client.query(searchForEpisodeQuery, {
        titleId: title.id,
        season: seasonNum,
        episode: episodeNum,
      }).toPromise();
      if (error) throw error;

      const top = payload.searchReleases.find(
        (release: Release) => release.qualityProfileDecision?.allowed ?? true,
      );
      if (!top) {
        setGlobalStatus(t("status.noReleaseForTitle", { name: title.name }));
        return;
      }

      const sourceHint = top.downloadUrl || top.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noSource", { name: title.name }));
        return;
      }

      try {
        const { error } = await client.mutation(queueExistingMutation, {
          input: {
            titleId: title.id,
            release: {
              sourceHint,
              sourceKind: top.sourceKind ?? null,
              sourceTitle: top.title,
            },
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.queuedLatest", { name: title.name }));
        await refreshTitleDetail();
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [collections, refreshTitleDetail, client, title, t, setGlobalStatus],
  );

  const handleAutoSearchInterstitialMovie = React.useCallback(
    async (collection: TitleCollection) => {
      if (!title || !collection.interstitialMovie) return;
      const movie = collection.interstitialMovie;
      const imdbId = movie.imdbId || null;
      const movieQuery = movie.year ? `${movie.name} ${movie.year}` : movie.name;

      const { data, error } = await client.query(searchQuery, {
        query: movieQuery,
        imdbId,
        tvdbId: null,
        category: "movies",
        limit: 25,
      }).toPromise();
      if (error) throw error;

      const top = data.searchReleases.find(
        (release: Release) => release.qualityProfileDecision?.allowed ?? true,
      );
      if (!top) {
        setGlobalStatus(t("status.noReleaseForTitle", { name: movie.name }));
        return;
      }

      const sourceHint = top.downloadUrl || top.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noSource", { name: movie.name }));
        return;
      }

      const { error: queueError } = await client.mutation(queueExistingMutation, {
        input: {
          titleId: title.id,
          release: {
            sourceHint,
            sourceKind: top.sourceKind ?? null,
            sourceTitle: top.title,
          },
        },
      }).toPromise();
      if (queueError) throw queueError;
      setGlobalStatus(t("status.queuedLatest", { name: movie.name }));
      await refreshTitleDetail();
    },
    [refreshTitleDetail, client, title, t, setGlobalStatus],
  );

  const [seasonSearchResultsByCollection] = React.useState<
    Record<string, Release[]>
  >({});
  const [seasonSearchLoadingByCollection, setSeasonSearchLoadingByCollection] = React.useState<
    Record<string, boolean>
  >({});

  const handleRunSeasonSearch = React.useCallback(
    async (collection: TitleCollection) => {
      if (!title) return;
      const seasonNum = parseInt(collection.collectionIndex?.trim().replace(/\D+/g, "") || "0", 10);
      if (!seasonNum) return;

      setSeasonSearchLoadingByCollection((prev) => ({ ...prev, [collection.id]: true }));
      try {
        await client
          .mutation(triggerSeasonWantedSearchMutation, {
            input: { titleId: title.id, seasonNumber: seasonNum },
          })
          .toPromise();
      } finally {
        setSeasonSearchLoadingByCollection((prev) => ({ ...prev, [collection.id]: false }));
      }
    },
    [client, title],
  );

  const handleQueueFromSeasonSearch = React.useCallback(
    async (release: Release) => {
      if (!title) return;
      const sourceHint = release.downloadUrl || release.link;
      if (!sourceHint) return;
      try {
        const { error } = await client
          .mutation(queueExistingMutation, {
            input: {
              titleId: title.id,
              release: {
                sourceHint,
                sourceKind: release.sourceKind ?? null,
                sourceTitle: release.title,
              },
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.queuedLatest", { name: title.name }));
        await refreshTitleDetail();
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [client, title, refreshTitleDetail, setGlobalStatus, t],
  );

  const handleOpenManualImport = React.useCallback(
    (item: DownloadQueueItem) => {
      setManualImportItem(item);
    },
    [],
  );

  const handleManualImportComplete = React.useCallback(async () => {
    await refreshTitleDetail();
  }, [refreshTitleDetail]);

  // Fallback: also listen to importHistoryChanged — fires from a different
  // broadcast channel so the page still refreshes even if the activity
  // subscription misses an event (e.g. transient WebSocket gap).
  useImportHistorySubscription(refreshTitleDetail);

  // Subscribe to activity events via WebSocket — refetch media files when an
  // import completes for this title (movie_downloaded / series_episode_imported / file_upgraded).
  const IMPORT_KINDS = React.useMemo(
    () => new Set(["movie_downloaded", "series_episode_imported", "file_upgraded"]),
    [],
  );
  const HYDRATION_STARTED_KIND = "metadata_hydration_started";
  const HYDRATION_COMPLETED_KIND = "metadata_hydration_completed";
  const HYDRATION_FAILED_KIND = "metadata_hydration_failed";

  // Use a ref for title so the effect only fires on new subscription data,
  // not when refreshTitleDetail() updates state (which would loop).
  const titleRef = React.useRef(title);
  titleRef.current = title;
  const processedActivityEventIdsRef = React.useRef<Set<string>>(new Set());

  React.useEffect(() => {
    processedActivityEventIdsRef.current.clear();
  }, [titleId]);

  const [activitySub] = useSubscription({
    query: activitySubscriptionQuery,
    pause: !title,
  });

  React.useEffect(() => {
    const currentTitle = titleRef.current;
    if (!currentTitle || !activitySub.data?.activityEvents) return;
    const rawEvents = collectActivityEventsFromPayload(activitySub.data.activityEvents);
    for (const raw of rawEvents) {
      const activity = normalizeActivityEvent(
        raw as Partial<ReturnType<typeof normalizeActivityEvent>>,
      );
      const processedEventIds = processedActivityEventIdsRef.current;
      if (processedEventIds.has(activity.id)) {
        continue;
      }
      processedEventIds.add(activity.id);
      if (processedEventIds.size > 200) {
        const oldestProcessedEventId = processedEventIds.values().next().value;
        if (oldestProcessedEventId) {
          processedEventIds.delete(oldestProcessedEventId);
        }
      }
      if (activity.titleId !== currentTitle.id) {
        continue;
      }

      if (activity.kind === HYDRATION_STARTED_KIND) {
        setHydratingFromActivity(true);
        continue;
      }

      if (activity.kind === HYDRATION_COMPLETED_KIND) {
        setHydratingFromActivity(false);
        void refreshTitleDetail();
        return;
      }

      if (activity.kind === HYDRATION_FAILED_KIND) {
        setHydratingFromActivity(false);
        continue;
      }

      if (IMPORT_KINDS.has(activity.kind)) {
        void refreshTitleDetail();
        return;
      }
    }
  }, [
    HYDRATION_COMPLETED_KIND,
    HYDRATION_FAILED_KIND,
    HYDRATION_STARTED_KIND,
    IMPORT_KINDS,
    refreshTitleDetail,
    activitySub.data,
  ]);

  return (
    <>
      <SeriesOverviewView
        loading={loading}
        hydrating={hydrating}
        title={title}
        collections={collections}
        events={events}
        episodesByCollection={episodesByCollection}
        mediaFilesByEpisode={mediaFilesByEpisode}
        subtitleDownloads={subtitleDownloads}
        releaseBlocklistEntries={releaseBlocklistEntries}
        onTitleChanged={refreshTitleDetail}
        onBackToList={onBackToList}
        onSetCollectionMonitored={handleSetCollectionMonitored}
        onSetEpisodeMonitored={handleSetEpisodeMonitored}
        onSetTitleMonitored={handleSetTitleMonitored}
        onSearchMonitored={handleSearchMonitored}
        onAutoSearchEpisode={handleAutoSearchEpisode}
        onAutoSearchInterstitialMovie={handleAutoSearchInterstitialMovie}
        qualityProfiles={qualityProfiles}
        defaultRootFolder={defaultRootFolder}
        rootFolders={rootFolders}
        onUpdateTitleOptions={handleUpdateTitleOptions}
        completedDownloads={completedDownloads}
        onOpenManualImport={handleOpenManualImport}
        initialEpisodeId={initialEpisodeId}
        seasonSearchResultsByCollection={seasonSearchResultsByCollection}
        seasonSearchLoadingByCollection={seasonSearchLoadingByCollection}
        onRunSeasonSearch={handleRunSeasonSearch}
        onQueueFromSeasonSearch={handleQueueFromSeasonSearch}
        monitoredUpdating={monitoredUpdating}
        searchMonitoredLoading={searchMonitoredLoading}
        onRefreshAndScan={handleRefreshAndScan}
        refreshAndScanLoading={refreshAndScanLoading}
        onRequestDeleteTitle={handleRequestDeleteTitle}
        deleteLoading={deleteLoading}
        onDeleteFile={handleDeleteMediaFile}
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
        isBusy={deleteLoading}
        onConfirm={handleConfirmDeleteTitle}
        onCancel={handleCancelDeleteTitle}
      >
        <label className="flex items-center gap-2">
          <Checkbox
            checked={deleteFilesOnDisk}
            onCheckedChange={(checked) => setDeleteFilesOnDisk(checked === true)}
            disabled={deleteLoading}
          />
          <span className="text-sm text-muted-foreground">{t("title.deleteFilesOnDisk")}</span>
        </label>
      </ConfirmDialog>
      {manualImportItem && title && (
        <ManualImportDialog
          open={true}
          onOpenChange={(open) => { if (!open) setManualImportItem(null); }}
          titleId={title.id}
          titleName={title.name}
          downloadClientItemId={manualImportItem.downloadClientItemId}
          onImportComplete={() => void handleManualImportComplete()}
        />
      )}
    </>
  );
});
