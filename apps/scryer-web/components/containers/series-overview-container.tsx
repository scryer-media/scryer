
import * as React from "react";
import {
  activitySubscriptionQuery,
  adminSettingsQuery,
  buildCollectionEpisodesBatchQuery,
  searchSeriesEpisodeQuery,
  titleMediaFilesQuery,
  titleOverviewInitQuery,
} from "@/lib/graphql/queries";
import {
  setCollectionMonitoredMutation,
  queueExistingMutation,
  setEpisodeMonitoredMutation,
  updateTitleMutation,
} from "@/lib/graphql/mutations";
import { downloadQueueQuery } from "@/lib/graphql/queries";
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
import type { Translate } from "@/components/root/types";
import { SeriesOverviewView } from "@/components/views/series-overview-view";
import { ManualImportDialog } from "@/components/dialogs/manual-import-dialog";

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
  monitored: boolean;
  createdAt: string;
};

export type TitleEvent = {
  id: string;
  eventType: string;
  actorUserId: string | null;
  titleId: string | null;
  message: string;
  occurredAt: string;
};

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
};

type SeriesOverviewContainerProps = {
  titleId: string;
  t: Translate;
  setGlobalStatus: (status: string) => void;
  onTitleNotFound?: () => void;
  onBackToList?: () => void;
};

export function SeriesOverviewContainer({
  titleId,
  t,
  setGlobalStatus,
  onTitleNotFound,
  onBackToList,
}: SeriesOverviewContainerProps) {
  const client = useClient();
  const [title, setTitle] = React.useState<TitleDetail | null>(null);
  const [collections, setCollections] = React.useState<TitleCollection[]>([]);
  const [events, setEvents] = React.useState<TitleEvent[]>([]);
  const [releaseBlocklistEntries, setReleaseBlocklistEntries] = React.useState<
    TitleReleaseBlocklistEntry[]
  >([]);
  const [loading, setLoading] = React.useState(true);
  const [episodesByCollection, setEpisodesByCollection] = React.useState<
    Record<string, CollectionEpisode[]>
  >({});
  const [qualityProfiles, setQualityProfiles] = React.useState<{ id: string; name: string }[]>([]);
  const [defaultRootFolder, setDefaultRootFolder] = React.useState(DEFAULT_SERIES_LIBRARY_PATH);
  const [mediaFilesByEpisode, setMediaFilesByEpisode] = React.useState<
    Record<string, EpisodeMediaFile[]>
  >({});
  const [completedDownloads, setCompletedDownloads] = React.useState<DownloadQueueItem[]>([]);
  const [manualImportItem, setManualImportItem] = React.useState<DownloadQueueItem | null>(null);

  const refreshTitleDetail = React.useCallback(async () => {
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
      } catch {
        // Settings fetch is best-effort
      }
    };
    void load();
    return () => { cancelled = true; };
  }, [client]);

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

  const handleAutoSearchEpisode = React.useCallback(
    async (episode: CollectionEpisode) => {
      if (!title) return;

      const tvdbId = title.externalIds
        ?.find((id) => id.source.toLowerCase() === "tvdb")
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

  // Only re-fetch episodes when the set of collection IDs changes (add/remove),
  // not when a property like `monitored` is updated on an existing collection.
  // eslint-disable-next-line react-hooks/exhaustive-deps
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
    } catch {
      // Media files fetch is best-effort
    }
  }, [title, client]);

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

  // Subscribe to activity events via WebSocket — refetch media files when an
  // import completes for this title (movie_downloaded / series_episode_imported).
  const IMPORT_KINDS = React.useMemo(
    () => new Set(["movie_downloaded", "series_episode_imported"]),
    [],
  );

  const [activitySub] = useSubscription({
    query: activitySubscriptionQuery,
    pause: !title,
  });

  React.useEffect(() => {
    if (!title || !activitySub.data?.activityEvents) return;
    const rawEvents = collectActivityEventsFromPayload(activitySub.data.activityEvents);
    for (const raw of rawEvents) {
      const activity = normalizeActivityEvent(
        raw as Partial<ReturnType<typeof normalizeActivityEvent>>,
      );
      if (activity.titleId === title.id && IMPORT_KINDS.has(activity.kind)) {
        void refreshMediaFiles();
        return;
      }
    }
  }, [title, IMPORT_KINDS, refreshMediaFiles, activitySub.data]);

  return (
    <>
      <SeriesOverviewView
        t={t}
        loading={loading}
        title={title}
        collections={collections}
        events={events}
        episodesByCollection={episodesByCollection}
        mediaFilesByEpisode={mediaFilesByEpisode}
        releaseBlocklistEntries={releaseBlocklistEntries}
        setGlobalStatus={setGlobalStatus}
        onTitleChanged={refreshTitleDetail}
        onBackToList={onBackToList}
        onSetCollectionMonitored={handleSetCollectionMonitored}
        onSetEpisodeMonitored={handleSetEpisodeMonitored}
        onAutoSearchEpisode={handleAutoSearchEpisode}
        qualityProfiles={qualityProfiles}
        defaultRootFolder={defaultRootFolder}
        onUpdateTitleTags={handleUpdateTitleTags}
        completedDownloads={completedDownloads}
        onOpenManualImport={handleOpenManualImport}
      />
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
}
