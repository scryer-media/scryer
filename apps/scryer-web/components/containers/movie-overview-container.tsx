
import * as React from "react";
import {
  mediaRenamePreviewQuery,
  movieOverviewSettingsInitQuery,
  searchForTitleQuery,
  subtitleDownloadsQuery,
} from "@/lib/graphql/queries";
import {
  applyMediaRenameMutation,
  deleteMediaFileMutation,
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
import { DEFAULT_MOVIE_LIBRARY_PATH } from "@/lib/constants/settings";
import { qualityProfileSettingsToEntries } from "@/lib/utils/quality-profiles";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTitleOverviewReactiveRefresh } from "@/lib/hooks/use-title-overview-reactive-refresh";
import { handleFixTitleMatchComplete as applyFixTitleMatchCompletion } from "@/lib/fix-title-match";
import type { Release, WantedItem } from "@/lib/types";
import { fetchTitleOverviewSnapshot } from "@/lib/title-overview-loader";
import { MovieOverviewView } from "@/components/views/movie-overview-view";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Checkbox } from "@/components/ui/checkbox";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import { FixTitleMatchDialog } from "@/components/dialogs/fix-title-match-dialog";

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

import type { TitleHistoryEvent } from "@/lib/types";
export type { TitleHistoryEvent };

export type TitleReleaseBlocklistEntry = {
  sourceHint: string | null;
  sourceTitle: string | null;
  errorMessage: string | null;
  attemptedAt: string;
};

export type SubtitleDownloadRecord = {
  id: string;
  mediaFileId: string;
  language: string;
  provider: string;
  filePath: string;
  score: number | null;
  hearingImpaired: boolean;
  forced: boolean;
  aiTranslated: boolean;
  machineTranslated: boolean;
  uploader: string | null;
  releaseInfo: string | null;
  synced: boolean;
  downloadedAt: string;
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

type MovieOverviewSnapshotTitle = TitleDetail & {
  collections?: TitleCollection[];
  mediaFiles?: TitleMediaFile[];
  wantedItems?: WantedItem[];
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
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
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
  const [subtitleDownloads, setSubtitleDownloads] = React.useState<SubtitleDownloadRecord[]>([]);
  const [wantedItem, setWantedItem] = React.useState<WantedItem | null>(null);
  const [monitoredUpdating, setMonitoredUpdating] = React.useState(false);
  const [searchMonitoredLoading, setSearchMonitoredLoading] = React.useState(false);
  const [refreshAndScanLoading, setRefreshAndScanLoading] = React.useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = React.useState(false);
  const [deleteFilesOnDisk, setDeleteFilesOnDisk] = React.useState(false);
  const [deleteLoading, setDeleteLoading] = React.useState(false);
  const [fixMatchOpen, setFixMatchOpen] = React.useState(false);
  const [wantedActionLoading, setWantedActionLoading] = React.useState<
    "pause" | "resume" | "reset" | null
  >(null);

  const refreshTitleDetail = React.useCallback(async () => {
    const snapshot = await fetchTitleOverviewSnapshot<
      MovieOverviewSnapshotTitle,
      TitleHistoryEvent,
      TitleReleaseBlocklistEntry,
      SubtitleDownloadRecord
    >(client, titleId, 200);
    const nextTitle = snapshot.title;
    setTitle(nextTitle);
    setCollections(nextTitle?.collections ?? []);
    setEvents(snapshot.titleEvents);
    setBlocklistEntries(snapshot.titleReleaseBlocklist);
    setMediaFiles(nextTitle?.mediaFiles ?? []);
    setWantedItem(nextTitle?.wantedItems?.[0] ?? null);
    setSubtitleDownloads(snapshot.subtitleDownloads);
    setRenamePlan(null);
  }, [titleId, client]);

  const refreshSubtitleDownloads = React.useCallback(() => {
    if (!titleId) return;
    client.query(subtitleDownloadsQuery, { titleId }).toPromise().then((subResult) => {
      setSubtitleDownloads(subResult.data?.subtitleDownloads ?? []);
    }).catch(() => {});
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

  // Fetch quality profile catalog and default root folder
  React.useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client.query(movieOverviewSettingsInitQuery, {}).toPromise();
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
      } catch {
        // Settings fetch is best-effort
      }
    };
    void load();
    return () => { cancelled = true; };
  }, [client]);

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
      const { data, error } = await client.query(searchForTitleQuery, {
        titleId: title.id,
      }).toPromise();
      if (error) throw error;
      const results = data.searchReleases ?? [];
      setSearchResults(results);
      setGlobalStatus(t("status.foundNzb", { count: results.length }));
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      setSearchResults([]);
    } finally {
      setSearching(false);
    }
  }, [title, client, t, setGlobalStatus]);

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
            release: {
              sourceHint,
              sourceKind: release.sourceKind ?? null,
              sourceTitle: release.title,
            },
          },
        }).toPromise();
        if (error) throw error;
        const queuedMessage = t("status.queueSuccess", { name: release.title });
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

  const IMPORT_KINDS = React.useMemo(
    () =>
      new Set([
        "movie_downloaded",
        "series_episode_imported",
        "file_upgraded",
        "subtitle_downloaded",
      ]),
    [],
  );

  useTitleOverviewReactiveRefresh({
    titleId,
    refresh: refreshTitleDetail,
    importKinds: IMPORT_KINDS,
    pause: !titleId,
  });

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
        onUpdateTitleOptions={handleUpdateTitleOptions}
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
        subtitleDownloads={subtitleDownloads}
        onDeleteFile={handleDeleteMediaFile}
        onRefreshSubtitles={refreshSubtitleDownloads}
        onOpenFixMatch={() => setFixMatchOpen(true)}
      />
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
