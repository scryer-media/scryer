
import * as React from "react";
import {
  activitySubscriptionQuery,
  adminSettingsQuery,
  mediaRenamePreviewQuery,
  searchQuery,
  titleOverviewInitQuery,
} from "@/lib/graphql/queries";
import {
  applyMediaRenameMutation,
  queueExistingMutation,
  scanMovieLibraryMutation,
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
import type { Release } from "@/lib/types";
import { MovieOverviewView } from "@/components/views/movie-overview-view";

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
  const [searching, setSearching] = React.useState(false);
  const [renamePlan, setRenamePlan] = React.useState<MediaRenamePlan | null>(null);
  const [renamePreviewing, setRenamePreviewing] = React.useState(false);
  const [renameApplying, setRenameApplying] = React.useState(false);
  const [titleLookupAttempted, setTitleLookupAttempted] = React.useState(false);
  const [titleLookupFailed, setTitleLookupFailed] = React.useState(false);
  const [qualityProfiles, setQualityProfiles] = React.useState<{ id: string; name: string }[]>([]);
  const [defaultRootFolder, setDefaultRootFolder] = React.useState(DEFAULT_MOVIE_LIBRARY_PATH);

  const refreshTitleDetail = React.useCallback(async () => {
    const { data, error } = await client.query(titleOverviewInitQuery, { id: titleId, blocklistLimit: 200 }).toPromise();
    if (error) throw error;
    setTitle(data.title ?? null);
    setCollections(data.titleCollections ?? []);
    setEvents(data.titleEvents ?? []);
    setBlocklistEntries(data.titleReleaseBlocklist ?? []);
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
      setRenamePlan(null);
      setRenamePreviewing(false);
      setRenameApplying(false);
      setTitleLookupAttempted(false);
      setTitleLookupFailed(false);
      setLoading(false);
      return () => {
        cancelled = true;
      };
    }

    setTitleLookupAttempted(false);
    setTitleLookupFailed(false);
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
  }, [refreshTitleDetail, setGlobalStatus, t]);

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

  const runIndexerSearch = React.useCallback(async () => {
    if (!title) return;
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
      if (!release.downloadUrl) {
        setGlobalStatus(t("status.noReleaseSource"));
        return;
      }
      try {
        const { error } = await client.mutation(queueExistingMutation, {
          input: {
            titleId: title.id,
            sourceHint: release.downloadUrl,
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

  const scanLibrary = React.useCallback(async () => {
    try {
      const { data, error } = await client.mutation(scanMovieLibraryMutation, {}).toPromise();
      if (error) throw error;
      setGlobalStatus(
        t("settings.libraryScanSuccess", {
          imported: data.scanMovieLibrary.imported,
          skipped: data.scanMovieLibrary.skipped,
          unmatched: data.scanMovieLibrary.unmatched,
        }),
      );
      await refreshTitleDetail();
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("settings.libraryScanFailed"));
    }
  }, [refreshTitleDetail, client, setGlobalStatus, t]);

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

  // Subscribe to activity events via WebSocket — refresh title detail (including
  // collection quality labels) when an import completes for this movie.
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
        void refreshTitleDetail();
        return;
      }
    }
  }, [title, IMPORT_KINDS, refreshTitleDetail, activitySub.data]);

  return (
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
      onSearch={runIndexerSearch}
      onQueue={queueRelease}
      onScanLibrary={scanLibrary}
      onPreviewRename={previewRename}
      onApplyRename={applyRename}
      onBackToList={onBackToList}
      qualityProfiles={qualityProfiles}
      defaultRootFolder={defaultRootFolder}
      onUpdateTitleTags={handleUpdateTitleTags}
      blocklistEntries={blocklistEntries}
    />
  );
});
