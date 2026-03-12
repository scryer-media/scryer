
import * as React from "react";
import {
  activitySubscriptionQuery,
  adminSettingsQuery,
  mediaRenamePreviewQuery,
  searchQuery,
  titleMediaFilesQuery,
  titleOverviewInitQuery,
  wantedItemsQuery,
} from "@/lib/graphql/queries";
import {
  applyMediaRenameMutation,
  deleteTitleMutation,
  queueExistingMutation,
  scanTitleLibraryMutation,
  setTitleMonitoredMutation,
  triggerTitleWantedSearchMutation,
  pauseWantedItemMutation,
  resumeWantedItemMutation,
  resetWantedItemMutation,
  updateTitleMutation,
} from "@/lib/graphql/mutations";
import type { AdminSetting } from "@/lib/types/admin-settings";
import { QUALITY_PROFILE_CATALOG_KEY, MOVIE_FOLDER_KEY, DEFAULT_MOVIE_LIBRARY_PATH } from "@/lib/constants/settings";
import { getSettingDisplayValue } from "@/lib/utils/settings";
import { parseQualityProfileCatalog } from "@/lib/utils/quality-profiles";
import {
  collectActivityEventsFromPayload,
  normalizeActivityEvent,
} from "@/lib/utils/activity";
import { useClient, useSubscription } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { Release, WantedItem } from "@/lib/types";
import { MovieOverviewView } from "@/components/views/movie-overview-view";
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

export type TitleMediaFile = {
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

export type MediaRenamePlanItem = {
  collectionId: string | null;
  currentPath: string;
  proposedPath: string | null;
  normalizedFilename: string | null;
  collision: boolean;
  reasonCode: string;
  writeAction: string;
  sourceSizeBytes: string | null;
  sourceMtimeUnixMs: string | null;
};

export type MediaRenamePlan = {
  facet: string;
  titleId: string | null;
  template: string;
  collisionPolicy: string;
  missingMetadataPolicy: string;
  fingerprint: string;
  total: number;
  renamable: number;
  noop: number;
  conflicts: number;
  errors: number;
  items: MediaRenamePlanItem[];
};

export type MediaRenameApplyResult = {
  planFingerprint: string;
  total: number;
  applied: number;
  skipped: number;
  failed: number;
};

type MovieOverviewContainerProps = {
  titleId: string;
  onTitleNotFound?: () => void;
  onBackToList?: () => void;
};

export const MovieOverviewContainer = React.memo(function MovieOverviewContainer({
  titleId,
  onTitleNotFound,
  onBackToList,
}: MovieOverviewContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [title, setTitle] = React.useState<TitleDetail | null>(null);
  const [collections, setCollections] = React.useState<TitleCollection[]>([]);
  const [events, setEvents] = React.useState<TitleEvent[]>([]);
  const [blocklistEntries, setBlocklistEntries] = React.useState<
    TitleReleaseBlocklistEntry[]
  >([]);
  const [loading, setLoading] = React.useState(true);

  const [searchResults, setSearchResults] = React.useState<Release[]>([]);
  const [interactiveSearchAttempted, setInteractiveSearchAttempted] = React.useState(false);
  const [searching, setSearching] = React.useState(false);
  const [renamePlan, setRenamePlan] = React.useState<MediaRenamePlan | null>(null);
  const [renamePreviewing, setRenamePreviewing] = React.useState(false);
  const [renameApplying, setRenameApplying] = React.useState(false);
  const [titleLookupAttempted, setTitleLookupAttempted] = React.useState(false);
  const [titleLookupFailed, setTitleLookupFailed] = React.useState(false);
  const [qualityProfiles, setQualityProfiles] = React.useState<{ id: string; name: string }[]>([]);
  const [defaultRootFolder, setDefaultRootFolder] = React.useState(DEFAULT_MOVIE_LIBRARY_PATH);
  const [mediaFiles, setMediaFiles] = React.useState<TitleMediaFile[]>([]);
  const [wantedItem, setWantedItem] = React.useState<WantedItem | null>(null);
  const [monitoredUpdating, setMonitoredUpdating] = React.useState(false);
  const [searchMonitoredLoading, setSearchMonitoredLoading] = React.useState(false);
  const [refreshAndScanLoading, setRefreshAndScanLoading] = React.useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = React.useState(false);
  const [deleteFilesOnDisk, setDeleteFilesOnDisk] = React.useState(false);
  const [deleteLoading, setDeleteLoading] = React.useState(false);
  const [wantedActionLoading, setWantedActionLoading] = React.useState<
    "pause" | "resume" | "reset" | null
  >(null);

  const refreshTitleDetail = React.useCallback(async () => {
    const [titleResult, mediaResult, wantedResult] = await Promise.all([
      client.query(titleOverviewInitQuery, { id: titleId, blocklistLimit: 200 }).toPromise(),
      client.query(titleMediaFilesQuery, { titleId }).toPromise(),
      client
        .query(wantedItemsQuery, { titleId, limit: 1, offset: 0 }, { requestPolicy: "network-only" })
        .toPromise(),
    ]);
    if (titleResult.error) throw titleResult.error;
    setTitle(titleResult.data.title ?? null);
    setCollections(titleResult.data.titleCollections ?? []);
    setEvents(titleResult.data.titleEvents ?? []);
    setBlocklistEntries(titleResult.data.titleReleaseBlocklist ?? []);
    setMediaFiles(mediaResult.data?.titleMediaFiles ?? []);
    setWantedItem(wantedResult.data?.wantedItems?.items?.[0] ?? null);
    setRenamePlan(null);
  }, [titleId, client]);

  // Load title detail on mount
  React.useEffect(() => {
    let cancelled = false;

    if (!titleId) {
      setTitle(null);
      setCollections([]);
      setEvents([]);
      setBlocklistEntries([]);
      setSearchResults([]);
      setInteractiveSearchAttempted(false);
      setMediaFiles([]);
      setRenamePlan(null);
      setRenamePreviewing(false);
      setRenameApplying(false);
      setTitleLookupAttempted(false);
      setTitleLookupFailed(false);
      setLoading(false);
      setWantedItem(null);
      return () => {
        cancelled = true;
      };
    }

    setTitleLookupAttempted(false);
    setTitleLookupFailed(false);
    setSearchResults([]);
    setInteractiveSearchAttempted(false);
    setLoading(true);
    refreshTitleDetail()
      .catch((err: unknown) => {
        if (!cancelled) {
          setTitleLookupFailed(true);
          setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
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
  }, [refreshTitleDetail, setGlobalStatus, t, titleId]);

  React.useEffect(() => {
    if (titleId && titleLookupAttempted && !loading && !titleLookupFailed && !title) {
      onTitleNotFound?.();
    }
  }, [loading, titleId, titleLookupAttempted, titleLookupFailed, title, onTitleNotFound]);

  const tvdbId = React.useMemo(
    () => title?.externalIds.find((e) => e.source === "tvdb")?.value ?? null,
    [title],
  );

  const imdbId = React.useMemo(
    () => title?.externalIds.find((e) => e.source === "imdb")?.value ?? null,
    [title],
  );

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
          (item: AdminSetting) => item.keyName === MOVIE_FOLDER_KEY,
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
      } catch (err) {
        setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      } finally {
        setMonitoredUpdating(false);
      }
    },
    [title, client, refreshTitleDetail, setGlobalStatus, t],
  );

  const runWantedAction = React.useCallback(
    async (
      action: "pause" | "resume" | "reset",
      mutation: string,
      successMessage?: string,
    ) => {
      if (!wantedItem) return;
      setWantedActionLoading(action);
      try {
        const { error } = await client.mutation(mutation, {
          input: { wantedItemId: wantedItem.id },
        }).toPromise();
        if (error) throw error;
        if (successMessage) {
          setGlobalStatus(successMessage);
        }
        await refreshTitleDetail();
      } catch (err) {
        setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      } finally {
        setWantedActionLoading(null);
      }
    },
    [wantedItem, client, refreshTitleDetail, setGlobalStatus, t],
  );

  const handleSearchMonitored = React.useCallback(
    async () => {
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
        await refreshTitleDetail();
      } catch (err) {
        setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      } finally {
        setSearchMonitoredLoading(false);
      }
    },
    [title, client, refreshTitleDetail, setGlobalStatus, t],
  );

  const handlePauseWanted = React.useCallback(
    async () => {
      await runWantedAction("pause", pauseWantedItemMutation);
    },
    [runWantedAction],
  );

  const handleResumeWanted = React.useCallback(
    async () => {
      await runWantedAction("resume", resumeWantedItemMutation);
    },
    [runWantedAction],
  );

  const handleResetWanted = React.useCallback(
    async () => {
      await runWantedAction("reset", resetWantedItemMutation);
    },
    [runWantedAction],
  );

  const runIndexerSearch = React.useCallback(async () => {
    if (!title) return;
    setInteractiveSearchAttempted(true);
    setSearching(true);
    setGlobalStatus(t("status.searchingNzb", { query: title.name, category: "" }));
    try {
      const { data, error } = await client.query(searchQuery, {
        query: title.name,
        tvdbId,
        imdbId,
        category: "movie",
        limit: 50,
      }).toPromise();
      if (error) throw error;
      const results = data.searchIndexers ?? [];
      setSearchResults(results);
      setGlobalStatus(t("status.foundNzb", { count: results.length }));
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      setSearchResults([]);
    } finally {
      setSearching(false);
    }
  }, [title, tvdbId, imdbId, client, t, setGlobalStatus]);

  const queueRelease = React.useCallback(
    async (release: Release) => {
      if (!title) return;
      if (release.qualityProfileDecision && !release.qualityProfileDecision.allowed) {
        const reason = release.qualityProfileDecision.blockCodes.join(", ") || t("status.unknownError");
        setGlobalStatus(t("nzb.blockedByProfile", { reason }));
        return;
      }
      const sourceHint = release.downloadUrl || release.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noReleaseSource"));
        return;
      }
      try {
        const { error } = await client.mutation(queueExistingMutation, {
          input: {
            titleId: title.id,
            sourceHint,
            sourceKind: release.sourceKind ?? null,
            sourceTitle: release.title,
          },
        }).toPromise();
        if (error) throw error;
        const queuedMessage = t("status.queued", { name: release.title });
        setGlobalStatus(queuedMessage);
        await refreshTitleDetail();
      } catch (err) {
        setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      }
    },
    [title, client, t, setGlobalStatus, refreshTitleDetail],
  );

  const handleRefreshAndScan = React.useCallback(async () => {
    if (!title) return;
    setRefreshAndScanLoading(true);
    try {
      const { data, error } = await client.mutation(scanTitleLibraryMutation, {
        input: { titleId: title.id },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(
        t("status.titleScanSuccess", {
          imported: data.scanTitleLibrary.imported,
          skipped: data.scanTitleLibrary.skipped,
          unmatched: data.scanTitleLibrary.unmatched,
        }),
      );
      await refreshTitleDetail();
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("settings.libraryScanFailed"));
    } finally {
      setRefreshAndScanLoading(false);
    }
  }, [title, refreshTitleDetail, client, setGlobalStatus, t]);

  const previewRename = React.useCallback(async () => {
    if (!title) return;
    setRenamePreviewing(true);
    try {
      const { data, error } = await client.query(mediaRenamePreviewQuery, {
        input: {
          facet: "movie",
          titleId: title.id,
          dryRun: true,
        },
      }).toPromise();
      if (error) throw error;
      const plan = data.mediaRenamePreview;
      setRenamePlan(plan);
      setGlobalStatus(
        t("status.renamePreviewGenerated", {
          total: plan.total,
          renamable: plan.renamable,
        }),
      );
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      setRenamePlan(null);
    } finally {
      setRenamePreviewing(false);
    }
  }, [title, client, setGlobalStatus, t]);

  const applyRename = React.useCallback(async () => {
    if (!title || !renamePlan) return;
    setRenameApplying(true);
    try {
      const { data, error } = await client.mutation(
        applyMediaRenameMutation,
        {
          input: {
            facet: "movie",
            titleId: title.id,
            fingerprint: renamePlan.fingerprint,
          },
        },
      ).toPromise();
      if (error) throw error;
      const result = data.applyMediaRename;
      setGlobalStatus(
        t("status.renameApplied", {
          applied: result.applied,
          skipped: result.skipped,
          failed: result.failed,
        }),
      );
      await refreshTitleDetail();
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
    } finally {
      setRenameApplying(false);
    }
  }, [title, renamePlan, refreshTitleDetail, client, setGlobalStatus, t]);

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
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.failedToDelete"));
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

  // Subscribe to activity events via WebSocket — refresh title detail (including
  // collection quality labels) when an import or upgrade completes for this movie.
  const IMPORT_KINDS = React.useMemo(
    () => new Set(["movie_downloaded", "series_episode_imported", "file_upgraded"]),
    [],
  );
  const HYDRATION_COMPLETED_KIND = "metadata_hydration_completed";

  // Use a ref for title so the effect only fires on new subscription data,
  // not when refreshTitleDetail() updates the title state (which would loop).
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
      if (activity.kind === HYDRATION_COMPLETED_KIND || IMPORT_KINDS.has(activity.kind)) {
        void refreshTitleDetail();
        return;
      }
    }
  }, [HYDRATION_COMPLETED_KIND, IMPORT_KINDS, refreshTitleDetail, activitySub.data]);

  return (
    <>
      <MovieOverviewView
        loading={loading}
        title={title}
        collections={collections}
        events={events}
        searchResults={searchResults}
        searching={searching}
        renamePlan={renamePlan}
        renamePreviewing={renamePreviewing}
        renameApplying={renameApplying}
        interactiveSearchAttempted={interactiveSearchAttempted}
        searchMonitoredLoading={searchMonitoredLoading}
        refreshAndScanLoading={refreshAndScanLoading}
        deleteLoading={deleteLoading}
        onSearch={runIndexerSearch}
        onQueue={queueRelease}
        onSearchMonitored={handleSearchMonitored}
        onRefreshAndScan={handleRefreshAndScan}
        onPreviewRename={previewRename}
        onApplyRename={applyRename}
        onBackToList={onBackToList}
        qualityProfiles={qualityProfiles}
        defaultRootFolder={defaultRootFolder}
        onUpdateTitleTags={handleUpdateTitleTags}
        onSetTitleMonitored={handleSetTitleMonitored}
        monitoredUpdating={monitoredUpdating}
        wantedItem={wantedItem}
        wantedActionLoading={wantedActionLoading}
        onPauseWanted={handlePauseWanted}
        onResumeWanted={handleResumeWanted}
        onResetWanted={handleResetWanted}
        onRequestDeleteTitle={handleRequestDeleteTitle}
        blocklistEntries={blocklistEntries}
        mediaFiles={mediaFiles}
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
    </>
  );
});
