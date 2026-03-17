
import * as React from "react";
import {
  activitySubscriptionQuery,
  adminSettingsQuery,
  buildCollectionEpisodesBatchQuery,
  searchQuery,
  searchSeriesEpisodeQuery,
  titleMediaFilesQuery,
  titleOverviewInitQuery,
  subtitleDownloadsQuery,
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
  updateTitleMutation,
} from "@/lib/graphql/mutations";
import { downloadQueueQuery, rootFoldersQuery, searchSeasonQuery } from "@/lib/graphql/queries";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import type { AdminSetting } from "@/lib/types/admin-settings";
import type { Release } from "@/lib/types";
import { QUALITY_PROFILE_CATALOG_KEY, SERIES_FOLDER_KEY, DEFAULT_SERIES_LIBRARY_PATH } from "@/lib/constants/settings";
import { getSettingDisplayValue } from "@/lib/utils/settings";
import { parseQualityProfileCatalog } from "@/lib/utils/quality-profiles";
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
    setTitle(data.title ?? null);
    setCollections(data.titleCollections ?? []);
    setEvents(data.titleEvents ?? []);
    setReleaseBlocklistEntries(data.titleReleaseBlocklist ?? []);
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
        const [systemResult, mediaResult] = await Promise.all([
          client.query(adminSettingsQuery, {
            scope: "system",
            category: "media",
          }).toPromise(),
          client.query(adminSettingsQuery, {
            scope: "media",
            category: "media",
          }).toPromise(),
        ]);
        if (systemResult.error) throw systemResult.error;
        if (mediaResult.error) throw mediaResult.error;
        if (cancelled) return;
        const catalogRecord = systemResult.data.adminSettings.items.find(
          (item: AdminSetting) => item.keyName === QUALITY_PROFILE_CATALOG_KEY,
        );
        const catalogJson = getSettingDisplayValue(catalogRecord);
        setQualityProfiles(parseQualityProfileCatalog(catalogJson));
        const folderRecord = mediaResult.data.adminSettings.items.find(
          (item: AdminSetting) => item.keyName === SERIES_FOLDER_KEY,
        );
        const folder = getSettingDisplayValue(folderRecord).trim();
        if (folder) setDefaultRootFolder(folder);

        const facet = title?.facet === "anime" ? "anime" : "tv";
        const rfResult = await client.query(rootFoldersQuery, { facet }).toPromise();
        if (!cancelled && Array.isArray(rfResult.data?.rootFolders)) {
          setRootFolders(rfResult.data.rootFolders);
        }
      } catch {
        // Settings fetch is best-effort
      }
    };
    void load();
    return () => { cancelled = true; };
  }, [client, title?.facet]);

  const handleUpdateTitleTags = React.useCallback(
    async (newTags: string[]) => {
      const { error } = await client.mutation(updateTitleMutation, {
        input: { titleId, tags: newTags },
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
        prev.map((c) => c.id === payload.id ? { ...c, monitored: payload.monitored } : c),
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

  // Reusable callback to (re)fetch media files for the title
  const refreshMediaFiles = React.useCallback(async () => {
    if (!title) return;
    try {
      const { data, error } = await client.query(titleMediaFilesQuery, { titleId: title.id }).toPromise();
      if (error) throw error;
      const grouped: Record<string, EpisodeMediaFile[]> = {};
      for (const file of data.titleMediaFiles ?? []) {
        const key = file.episodeId ?? "__unlinked__";
        (grouped[key] ??= []).push(file);
      }
      setMediaFilesByEpisode(grouped);
      // Also fetch subtitle downloads
      client.query(subtitleDownloadsQuery, { titleId: title.id }).toPromise().then((subResult) => {
        setSubtitleDownloads(subResult.data?.subtitleDownloads ?? []);
      }).catch(() => {});
    } catch {
      // Media files fetch is best-effort
    }
  }, [title, client]);

  const handleDeleteMediaFile = React.useCallback(async (fileId: string) => {
    try {
      const { error } = await client.mutation(deleteMediaFileMutation, {
        input: { fileId, deleteFromDisk: true },
      }).toPromise();
      if (error) throw error;
      await refreshMediaFiles();
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    }
  }, [client, refreshMediaFiles, setGlobalStatus, t]);

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
      await Promise.all([refreshTitleDetail(), refreshMediaFiles()]);
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("settings.libraryScanFailed"));
    } finally {
      setRefreshAndScanLoading(false);
    }
  }, [client, refreshMediaFiles, refreshTitleDetail, setGlobalStatus, t, title]);

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

      const tvdbId = title.externalIds
        ?.find((id) => id.source.toLowerCase() === "tvdb")
        ?.value?.trim() || null;
      const anidbId = title.externalIds
        ?.find((id) => id.source.toLowerCase() === "anidb")
        ?.value?.trim() || null;
      const collection = collections.find((item) => item.id === episode.collectionId);
      const seasonNum =
        episode.seasonNumber?.trim().replace(/\D+/g, "") ||
        collection?.collectionIndex?.trim().replace(/\D+/g, "") ||
        "1";
      const episodeNum = episode.episodeNumber?.trim().replace(/\D+/g, "") || "1";

      const runEpisodeSearch = async (searchTvdbId: string | null) => {
        const { data, error } = await client.query(searchSeriesEpisodeQuery, {
          title: title.name,
          season: seasonNum,
          episode: episodeNum,
          tvdbId: searchTvdbId,
          anidbId,
          category: title.facet,
          limit: 25,
        }).toPromise();
        if (error) throw error;
        return data;
      };

      let payload = await runEpisodeSearch(tvdbId);
      if (payload.searchIndexersEpisode.length === 0 && tvdbId) {
        payload = await runEpisodeSearch(null);
      }

      const top = payload.searchIndexersEpisode.find(
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
            sourceHint,
            sourceKind: top.sourceKind ?? null,
            sourceTitle: top.title,
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

      const top = data.searchIndexers.find(
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
          sourceHint,
          sourceKind: top.sourceKind ?? null,
          sourceTitle: top.title,
        },
      }).toPromise();
      if (queueError) throw queueError;
      setGlobalStatus(t("status.queuedLatest", { name: movie.name }));
      await refreshTitleDetail();
    },
    [refreshTitleDetail, client, title, t, setGlobalStatus],
  );

  const [seasonSearchResultsByCollection, setSeasonSearchResultsByCollection] = React.useState<
    Record<string, Release[]>
  >({});
  const [seasonSearchLoadingByCollection, setSeasonSearchLoadingByCollection] = React.useState<
    Record<string, boolean>
  >({});

  const handleRunSeasonSearch = React.useCallback(
    async (collection: TitleCollection) => {
      if (!title) return;
      const tvdbId =
        title.externalIds?.find((id) => id.source.toLowerCase() === "tvdb")?.value?.trim() || null;
      const seasonNum = collection.collectionIndex?.trim().replace(/\D+/g, "") || "";
      if (!seasonNum) return;

      setSeasonSearchLoadingByCollection((prev) => ({ ...prev, [collection.id]: true }));
      try {
        const { data } = await client
          .query(searchSeasonQuery, {
            title: title.name,
            season: seasonNum,
            tvdbId,
            category: title.facet,
            limit: 50,
          })
          .toPromise();
        setSeasonSearchResultsByCollection((prev) => ({
          ...prev,
          [collection.id]: data?.searchIndexersSeason ?? [],
        }));
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
              sourceHint,
              sourceKind: release.sourceKind ?? null,
              sourceTitle: release.title,
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

  // Only re-fetch episodes when the set of collection IDs changes (add/remove),
  // not when a property like `monitored` is updated on an existing collection.
  const collectionIdKey = collections.map((c) => c.id).join("\0");

  React.useEffect(() => {
    if (collections.length === 0) {
      setEpisodesByCollection({});
      return;
    }

    let cancelled = false;
    const loadCollectionEpisodes = async () => {
      const collectionIds = collections.map((c) => c.id);
      const { query, variables } = buildCollectionEpisodesBatchQuery(collectionIds);

      try {
        const { data, error } = await client.query(query, variables).toPromise();
        if (error) throw error;
        if (cancelled) return;

        const result: Record<string, CollectionEpisode[]> = {};
        for (let i = 0; i < collectionIds.length; i++) {
          result[collectionIds[i]] = data[`c${i}`] ?? [];
        }
        setEpisodesByCollection(result);
      } catch {
        if (!cancelled) {
          setEpisodesByCollection({});
        }
      }
    };

    void loadCollectionEpisodes();
    return () => {
      cancelled = true;
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [collectionIdKey, client]);

  // Fetch completed downloads for this title (for manual import button)
  React.useEffect(() => {
    if (!title) {
      setCompletedDownloads([]);
      return;
    }
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client.query(downloadQueueQuery, {
          includeAllActivity: true,
          includeHistoryOnly: false,
        }).toPromise();
        if (error) throw error;
        if (cancelled) return;
        const completed = (data.downloadQueue ?? []).filter(
          (item: DownloadQueueItem) =>
            item.titleId === titleId &&
            (item.state.toLowerCase() === "completed" ||
              item.state.toLowerCase() === "import_pending" ||
              item.state.toLowerCase() === "importpending"),
        );
        setCompletedDownloads(completed);
      } catch {
        // best-effort
      }
    };
    void load();
    return () => { cancelled = true; };
  }, [title, titleId, client]);

  const handleOpenManualImport = React.useCallback(
    (item: DownloadQueueItem) => {
      setManualImportItem(item);
    },
    [],
  );

  const handleManualImportComplete = React.useCallback(async () => {
    await refreshTitleDetail();
    await refreshMediaFiles();
  }, [refreshTitleDetail, refreshMediaFiles]);

  // Fetch media files on initial load / title change
  React.useEffect(() => {
    if (!title) {
      setMediaFilesByEpisode({});
      return;
    }
    void refreshMediaFiles();
  }, [title, refreshMediaFiles]);

  // Fallback: also listen to importHistoryChanged — fires from a different
  // broadcast channel so the page still refreshes even if the activity
  // subscription misses an event (e.g. transient WebSocket gap).
  useImportHistorySubscription(refreshMediaFiles);

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
  // not when refreshMediaFiles() updates state (which would loop).
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
        void refreshMediaFiles();
        return;
      }
    }
  }, [
    HYDRATION_COMPLETED_KIND,
    HYDRATION_FAILED_KIND,
    HYDRATION_STARTED_KIND,
    IMPORT_KINDS,
    refreshMediaFiles,
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
        onUpdateTitleTags={handleUpdateTitleTags}
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
