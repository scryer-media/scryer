import * as React from "react";
import { FileInput, FolderOpen, Loader2, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Clapperboard } from "lucide-react";
import { useClient } from "urql";
import type { Release } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { isAbortError } from "@/lib/graphql/urql-client";
import { runIterativeReleaseSearch } from "@/lib/graphql/release-search";
import {
  hasPrimaryMediaFile,
  releaseQueueScopeInput,
} from "@/lib/utils/release-queue-scope";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import {
  SERIES_OVERVIEW_CLEAR_EPISODE_SELECTION_ID,
  SERIES_OVERVIEW_DELETE_SELECTED_EPISODES_ID,
} from "@/lib/utils/dom-ids";
import {
  queueExistingMutation,
  queueReplacementMutation,
} from "@/lib/graphql/mutations";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  SeriesMovieLink,
  TitleCollection,
  TitleDetail,
  TitleHistoryEvent,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import { TitleHistoryModal } from "@/components/common/title-history-modal";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import {
  episodePanelReducer,
  initialEpisodePanelState,
} from "./episode-panel-reducer";
import {
  buildSeriesTimelineItems,
  sortDbCollections,
  findLatestSeasonKey,
  episodeSortValue,
  isSpecialsCollection,
  formatDate,
} from "./helpers";
import { OverviewControlPanel } from "../overview-control-panel";
import { OverviewBackLink } from "../overview-back-link";
import {
  TitleMoreLikeThisStrip,
  type TitleMoreLikeThisStripActions,
} from "../title-more-like-this-strip";
import { TitleCastStrip } from "../title-cast-strip";
import { TitleDubCastStrip } from "../title-dub-cast-strip";
import { titleCastOriginalCredits } from "@/lib/utils/title-cast";
import { TitleRatingsStrip } from "../title-ratings-strip";
import { TitleSettingsPanel } from "./title-settings-panel";
import { SeasonSection, SeriesMovieTimelineSection } from "./season-section";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import type { LibraryRootRecord } from "@/lib/types/titles";
import { localizedTitleStatus } from "../overview-localization";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import {
  AnidbExternalLink,
  AnilistExternalLink,
  ImdbExternalLink,
  MalExternalLink,
  TmdbExternalLink,
  TvdbSeriesExternalLink,
} from "@/components/common/external-media-links";
import { titleGenreLabels } from "@/lib/utils/title-genres";
import {
  collectActiveDownloadEpisodeIds,
  coveredEpisodeIdsForQueueItem,
} from "@/lib/utils/episode-download-activity";

const EPISODE_QUEUE_PRECEDENCE: Record<string, number> = {
  downloading: 0,
  post_processing: 1,
  queued: 2,
  paused: 3,
  import_pending: 4,
  importing: 5,
};

function compareEpisodeQueueItems(
  left: DownloadQueueItem,
  right: DownloadQueueItem,
): number {
  const leftRank =
    EPISODE_QUEUE_PRECEDENCE[left.displayState.toLowerCase()] ?? Number.MAX_SAFE_INTEGER;
  const rightRank =
    EPISODE_QUEUE_PRECEDENCE[right.displayState.toLowerCase()] ?? Number.MAX_SAFE_INTEGER;
  if (leftRank !== rightRank) {
    return leftRank - rightRank;
  }

  const leftUpdatedAt = Date.parse(left.lastUpdatedAt ?? "");
  const rightUpdatedAt = Date.parse(right.lastUpdatedAt ?? "");
  if (Number.isFinite(leftUpdatedAt) && Number.isFinite(rightUpdatedAt) && leftUpdatedAt !== rightUpdatedAt) {
    return rightUpdatedAt - leftUpdatedAt;
  }

  return right.progressPercent - left.progressPercent;
}

type Props = {
  canManageTitle: boolean;
  fullBleedHero?: boolean;
  loading: boolean;
  hydrating: boolean;
  title: TitleDetail | null;
  collections: TitleCollection[];
  seriesMovieLinks: SeriesMovieLink[];
  events: TitleHistoryEvent[];
  episodesByCollection: Record<string, CollectionEpisode[]>;
  collectionEpisodesLoading?: Record<string, boolean>;
  onLoadCollectionEpisodes?: (collectionId: string) => Promise<void> | void;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  mediaFilesBySeriesMovieLink: Record<string, EpisodeMediaFile[]>;
  onLoadEpisodeDetail?: (episodeId: string) => Promise<void> | void;
  onLoadSeriesMovieDetail?: (link: SeriesMovieLink) => Promise<void> | void;
  downloadQueueItems?: DownloadQueueItem[];
  subtitleDownloads?: ExternalSubtitleRecord[];
  onRefreshSubtitles?: () => Promise<void> | void;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  clearingReleaseBlocklistEntryId?: string | null;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onTitleChanged?: () => Promise<void>;
  onBackToList?: () => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  onSetSeriesMovieMonitored?: (seriesMovieLinkId: string, monitored: boolean) => Promise<void>;
  onSetTitleMonitored?: (monitored: boolean) => Promise<void>;
  onSearchMonitored?: () => Promise<void> | void;
  onRefreshAndScan?: () => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => Promise<void> | void;
  onAutoSearchSeriesMovie?: (link: SeriesMovieLink) => Promise<void> | void;
  qualityProfiles?: { id: string; name: string }[];
  defaultRootFolder?: string;
  renameEnabled?: boolean;
  rootFolders?: LibraryRootRecord[];
  onUpdateTitleOptions?: (options: TitleOptionUpdates) => Promise<void>;
  completedDownloads?: DownloadQueueItem[];
  onOpenManualImport?: (item: DownloadQueueItem) => void;
  initialEpisodeId?: string | null;
  seasonSearchResultsByCollection?: Record<string, Release[]>;
  seasonSearchLoadingByCollection?: Record<string, boolean>;
  onRunSeasonSearch?: (collection: TitleCollection) => Promise<void> | void;
  onQueueFromSeasonSearch?: (collection: TitleCollection, release: Release) => Promise<void> | void;
  monitoredUpdating?: boolean;
  searchMonitoredLoading?: boolean;
  hasDownloadClients: boolean;
  showSearchPrerequisiteNotice: boolean;
  refreshAndScanLoading?: boolean;
  onRequestDeleteTitle?: () => void;
  deleteLoading?: boolean;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
  primaryMovieFileUpdatingId?: string | null;
  /**
   * Open the confirm flow for deleting the media files of the selected
   * episodes. Only supplied when the viewer can manage the title.
   */
  onRequestDeleteEpisodeFiles?: (episodeIds: string[]) => void;
  /**
   * Bumped by the container each time a deletion job is accepted; the selection
   * is cleared whenever it changes.
   */
  episodeSelectionResetToken?: number;
  /**
   * Episodes whose media files an in-flight deletion job is working through.
   * Their rows cannot be selected until that run finishes.
   */
  pendingEpisodeIds?: ReadonlySet<string>;
  onOpenFixMatch?: () => void;
  moreLikeThisActions?: TitleMoreLikeThisStripActions;
};

function SeriesOverviewViewImpl({
  canManageTitle,
  fullBleedHero = false,
  loading,
  hydrating,
  title,
  collections,
  seriesMovieLinks,
  events: _events,
  episodesByCollection,
  collectionEpisodesLoading,
  onLoadCollectionEpisodes,
  mediaFilesByEpisode,
  mediaFilesBySeriesMovieLink,
  onLoadEpisodeDetail,
  onLoadSeriesMovieDetail,
  downloadQueueItems = [],
  subtitleDownloads,
  onRefreshSubtitles,
  releaseBlocklistEntries,
  clearingReleaseBlocklistEntryId,
  onClearReleaseBlocklistEntry,
  onTitleChanged,
  onBackToList,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  onSetSeriesMovieMonitored,
  onSetTitleMonitored,
  onSearchMonitored,
  onRefreshAndScan,
  onAutoSearchEpisode,
  onAutoSearchSeriesMovie,
  qualityProfiles,
  defaultRootFolder,
  renameEnabled,
  rootFolders,
  onUpdateTitleOptions,
  completedDownloads,
  onOpenManualImport,
  initialEpisodeId,
  seasonSearchResultsByCollection,
  seasonSearchLoadingByCollection,
  onRunSeasonSearch,
  onQueueFromSeasonSearch,
  monitoredUpdating = false,
  searchMonitoredLoading = false,
  hasDownloadClients,
  showSearchPrerequisiteNotice,
  refreshAndScanLoading = false,
  onRequestDeleteTitle,
  deleteLoading = false,
  onDeleteFile,
  onMakePrimaryFile,
  primaryMovieFileUpdatingId = null,
  onRequestDeleteEpisodeFiles,
  episodeSelectionResetToken,
  pendingEpisodeIds,
  onOpenFixMatch,
  moreLikeThisActions,
}: Props) {
  const emptyEpisodes = React.useMemo<CollectionEpisode[]>(() => [], []);
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const client = useClient();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();
  const backLabel = title?.facet === "ANIME" ? t("nav.anime") : t("nav.series");
  const sortedCollections = React.useMemo(
    () => sortDbCollections(collections),
    [collections],
  );

  const latestKey = React.useMemo(
    () => findLatestSeasonKey(sortedCollections),
    [sortedCollections],
  );

  const sortedEpisodesByCollection = React.useMemo(
    () => Object.fromEntries(
      sortedCollections.map((collection) => [
        collection.id,
        [...(episodesByCollection[collection.id] ?? emptyEpisodes)].sort(
          (left, right) => episodeSortValue(right) - episodeSortValue(left),
        ),
      ]),
    ) as Record<string, CollectionEpisode[]>,
    [emptyEpisodes, episodesByCollection, sortedCollections],
  );

  const timelineItems = React.useMemo(
    () => buildSeriesTimelineItems(sortedCollections, seriesMovieLinks),
    [seriesMovieLinks, sortedCollections],
  );

  const [expandedKeys, setExpandedKeys] = React.useState<Set<string>>(new Set());
  const [selectedEpisodeIds, setSelectedEpisodeIds] = React.useState<Set<string>>(
    () => new Set(),
  );
  const [historyOpen, setHistoryOpen] = React.useState(false);
  const [historyEpisodeScope, setHistoryEpisodeScope] = React.useState<{
    episodeId: string;
    episodeLabel: string;
  } | null>(null);
  const [episodePanel, dispatchEpisodePanel] = React.useReducer(episodePanelReducer, initialEpisodePanelState);
  const [searchBlockedByEpisode, setSearchBlockedByEpisode] = React.useState<Record<string, boolean>>({});
  const [searchBlockedByCollection, setSearchBlockedByCollection] = React.useState<Record<string, boolean>>({});
  const [searchBlockedBySeriesMovie, setSearchBlockedBySeriesMovie] = React.useState<Record<string, boolean>>({});
  const [seriesMovieSearchResultsByLink, setSeriesMovieSearchResultsByLink] =
    React.useState<Record<string, Release[]>>({});
  const [seriesMovieSearchLoadingByLink, setSeriesMovieSearchLoadingByLink] =
    React.useState<Record<string, boolean>>({});
  const [seriesMovieSearchAttemptedByLink, setSeriesMovieSearchAttemptedByLink] =
    React.useState<Record<string, boolean>>({});
  const [autoSearchSeriesMovieLoadingByLink, setAutoSearchSeriesMovieLoadingByLink] =
    React.useState<Record<string, boolean>>({});
  const episodeSearchAbortByIdRef = React.useRef<Record<string, AbortController>>({});
  const seriesMovieSearchAbortByLinkRef = React.useRef<Record<string, AbortController>>({});
  React.useEffect(() => {
    return () => {
      Object.values(episodeSearchAbortByIdRef.current).forEach((controller) => controller.abort());
      Object.values(seriesMovieSearchAbortByLinkRef.current).forEach((controller) => controller.abort());
      episodeSearchAbortByIdRef.current = {};
      seriesMovieSearchAbortByLinkRef.current = {};
    };
  }, []);
  const titleId = title?.id ?? null;
  React.useEffect(() => {
    setSelectedEpisodeIds(new Set());
  }, [titleId]);

  React.useEffect(() => {
    if (!episodeSelectionResetToken) {
      return;
    }
    setSelectedEpisodeIds(new Set());
  }, [episodeSelectionResetToken]);

  const handleToggleEpisodeSelected = React.useCallback((episodeId: string) => {
    setSelectedEpisodeIds((current) => {
      const next = new Set(current);
      if (next.has(episodeId)) {
        next.delete(episodeId);
      } else {
        next.add(episodeId);
      }
      return next;
    });
  }, []);

  const handleSetSeasonSelected = React.useCallback(
    (episodeIds: string[], selected: boolean) => {
      setSelectedEpisodeIds((current) => {
        const next = new Set(current);
        for (const episodeId of episodeIds) {
          if (selected) {
            next.add(episodeId);
          } else {
            next.delete(episodeId);
          }
        }
        return next;
      });
    },
    [],
  );

  const handleClearEpisodeSelection = React.useCallback(() => {
    setSelectedEpisodeIds(new Set());
  }, []);

  const handleDeleteSelectedEpisodeFiles = React.useCallback(() => {
    if (!onRequestDeleteEpisodeFiles || selectedEpisodeIds.size === 0) {
      return;
    }
    onRequestDeleteEpisodeFiles([...selectedEpisodeIds]);
  }, [onRequestDeleteEpisodeFiles, selectedEpisodeIds]);

  const episodeSelectionEnabled = canManageTitle && Boolean(onRequestDeleteEpisodeFiles);

  const searchPrerequisiteNotice = canManageTitle && !hasDownloadClients && showSearchPrerequisiteNotice
    ? <TitleSearchDownloadClientNotice />
    : null;
  const { activeDownloadEpisodeIds, primaryQueueItemByEpisodeId } = React.useMemo(() => {
    const queueItemsByEpisodeId: Record<string, DownloadQueueItem[]> = {};

    for (const item of downloadQueueItems) {
      for (const episodeId of coveredEpisodeIdsForQueueItem(item, sortedEpisodesByCollection)) {
        (queueItemsByEpisodeId[episodeId] ??= []).push(item);
      }
    }

    const primaryByEpisodeId = Object.fromEntries(
      Object.entries(queueItemsByEpisodeId).map(([episodeId, items]) => [
        episodeId,
        [...items].sort(compareEpisodeQueueItems)[0],
      ]),
    ) as Record<string, DownloadQueueItem | undefined>;

    return {
      activeDownloadEpisodeIds: collectActiveDownloadEpisodeIds(
        downloadQueueItems,
        sortedEpisodesByCollection,
      ),
      primaryQueueItemByEpisodeId: primaryByEpisodeId,
    };
  }, [downloadQueueItems, sortedEpisodesByCollection]);

  React.useEffect(() => {
    setSearchBlockedByEpisode({});
    setSearchBlockedByCollection({});
    setSearchBlockedBySeriesMovie({});
    setSeriesMovieSearchResultsByLink({});
    setSeriesMovieSearchLoadingByLink({});
    setSeriesMovieSearchAttemptedByLink({});
    setAutoSearchSeriesMovieLoadingByLink({});
  }, [title?.id]);

  React.useEffect(() => {
    if (hasDownloadClients) {
      setSearchBlockedByEpisode({});
      setSearchBlockedByCollection({});
    }
  }, [hasDownloadClients]);

  const handleOpenTitleHistory = React.useCallback(() => {
    setHistoryEpisodeScope(null);
    setHistoryOpen(true);
  }, []);

  const handleOpenEpisodeHistory = React.useCallback((episode: CollectionEpisode) => {
    setHistoryEpisodeScope({
      episodeId: episode.id,
      episodeLabel:
        episode.title ?? episode.episodeLabel ?? episode.episodeNumber ?? episode.id,
    });
    setHistoryOpen(true);
  }, []);

  const defaultExpandedRef = React.useRef(false);
  const lastDeepLinkedEpisodeIdRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    defaultExpandedRef.current = false;
  }, [title?.id]);

  React.useEffect(() => {
    lastDeepLinkedEpisodeIdRef.current = null;
  }, [initialEpisodeId, title?.id]);

  React.useEffect(() => {
    if (!initialEpisodeId) return;

    let targetCollectionKey: string | null = null;
    for (const [collectionId, episodes] of Object.entries(episodesByCollection)) {
      if (episodes.some((episode) => episode.id === initialEpisodeId)) {
        targetCollectionKey = `s-${collectionId}`;
        break;
      }
    }

    if (!targetCollectionKey) {
      return;
    }

    setExpandedKeys((current) => {
      if (current.has(targetCollectionKey)) {
        return current;
      }
      const next = new Set(current);
      next.add(targetCollectionKey);
      return next;
    });

    if (lastDeepLinkedEpisodeIdRef.current === initialEpisodeId) {
      return;
    }

    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        const episodeElement = Array.from(
          document.querySelectorAll<HTMLElement>("[data-episode-id]"),
        ).find((element) => element.dataset.episodeId === initialEpisodeId);
        if (!episodeElement) {
          return;
        }
        lastDeepLinkedEpisodeIdRef.current = initialEpisodeId;
        episodeElement.scrollIntoView({ behavior: "smooth", block: "center" });
      });
    });
  }, [expandedKeys, initialEpisodeId, episodesByCollection]);

  React.useEffect(() => {
    if (initialEpisodeId || defaultExpandedRef.current || !latestKey) {
      return;
    }

    defaultExpandedRef.current = true;
    const nextExpanded = new Set<string>();
    nextExpanded.add(latestKey);
    setExpandedKeys(nextExpanded);
  }, [initialEpisodeId, latestKey]);

  // Seasons hydrate lazily: fetch a collection's episodes once its section is
  // expanded (default latest-season expansion included) and nothing is cached
  // for it yet.
  React.useEffect(() => {
    if (!onLoadCollectionEpisodes) {
      return;
    }
    for (const item of timelineItems) {
      if (item.kind !== "collection" || !expandedKeys.has(item.key)) {
        continue;
      }
      const collectionId = item.collection.id;
      if (collectionId in episodesByCollection) {
        continue;
      }
      if (collectionEpisodesLoading?.[collectionId]) {
        continue;
      }
      void onLoadCollectionEpisodes(collectionId);
    }
  }, [
    collectionEpisodesLoading,
    episodesByCollection,
    expandedKeys,
    onLoadCollectionEpisodes,
    timelineItems,
  ]);

  const toggleKey = React.useCallback((key: string) => {
    setExpandedKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  const toggleActionByKey = React.useMemo(
    () =>
      new Map(
        timelineItems.map((item) => [item.key, () => toggleKey(item.key)]),
      ),
    [timelineItems, toggleKey],
  );
  const seasonSearchActionByKey = React.useMemo(() => {
    const actions = new Map<string, (() => void) | undefined>();
    for (const item of timelineItems) {
      if (item.kind === "seriesMovie") {
        continue;
      }
      const { collection } = item;
      actions.set(
        item.key,
        canManageTitle && onRunSeasonSearch
          ? () => {
              if (!hasDownloadClients) {
                setSearchBlockedByCollection((previous) => ({
                  ...previous,
                  [collection.id]: true,
                }));
                return;
              }
              setSearchBlockedByCollection((previous) => {
                if (!previous[collection.id]) {
                  return previous;
                }
                const next = { ...previous };
                delete next[collection.id];
                return next;
              });
              void onRunSeasonSearch(collection);
            }
          : undefined,
      );
    }
    return actions;
  }, [canManageTitle, hasDownloadClients, onRunSeasonSearch, timelineItems]);

  const handleRunEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode) => {
      if (!title) return;
      const episodeId = episode.id;

      if (!hasDownloadClients) {
        episodeSearchAbortByIdRef.current[episodeId]?.abort();
        delete episodeSearchAbortByIdRef.current[episodeId];
        setSearchBlockedByEpisode((prev) => ({ ...prev, [episodeId]: true }));
        dispatchEpisodePanel({ type: "RESET_SEARCH", episodeId });
        dispatchEpisodePanel({ type: "SET_SEARCH_LOADING", episodeId, loading: false });
        return;
      }

      setSearchBlockedByEpisode((prev) => {
        if (!prev[episodeId]) return prev;
        const next = { ...prev };
        delete next[episodeId];
        return next;
      });
      episodeSearchAbortByIdRef.current[episodeId]?.abort();
      const abortController = new AbortController();
      episodeSearchAbortByIdRef.current[episodeId] = abortController;
      dispatchEpisodePanel({ type: "RESET_SEARCH", episodeId });
      dispatchEpisodePanel({ type: "SET_SEARCH_LOADING", episodeId, loading: true });

      const collection = collections.find((c) => c.id === episode.collectionId);
      const seasonNum = episode.seasonNumber?.trim().replace(/\D+/g, "")
        || collection?.collectionIndex?.trim().replace(/\D+/g, "")
        || "1";
      const episodeNum = episode.episodeNumber?.trim().replace(/\D+/g, "") || "1";

      runIterativeReleaseSearch(client, {
        titleId: title.id,
        season: seasonNum,
        episode: episodeNum,
      }, {
        signal: abortController.signal,
        onUpdate: (snapshot) => {
          if (abortController.signal.aborted) return;
          dispatchEpisodePanel({
            type: "SET_SEARCH_SNAPSHOT",
            episodeId,
            results: snapshot.releases,
            indexers: snapshot.indexers,
          });
        },
      })
        .catch((error) => {
          if (isAbortError(error) || abortController.signal.aborted) return;
          dispatchEpisodePanel({ type: "RESET_SEARCH", episodeId });
        })
        .finally(() => {
          if (episodeSearchAbortByIdRef.current[episodeId] === abortController) {
            delete episodeSearchAbortByIdRef.current[episodeId];
            dispatchEpisodePanel({ type: "SET_SEARCH_LOADING", episodeId, loading: false });
          }
        });
    },
    [client, hasDownloadClients, title, collections],
  );

  const handleQueueFromEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode, release: Release) => {
      if (!title) return Promise.resolve();

      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return Promise.resolve();
      }

      const input = {
        titleId: title.id,
        scope: { episode: episode.id },
        candidateToken: release.candidateToken,
        sizeBytes: release.sizeBytes ?? null,
      };
      const replacesPrimary = hasPrimaryMediaFile(mediaFilesByEpisode[episode.id]);
      const mutation = replacesPrimary
        ? queueReplacementMutation
        : queueExistingMutation;
      return retryWithReplaceOnConflict(
        input,
        async (nextInput) => {
          const { data, error: mutationError } = await client
            .mutation(mutation, { input: nextInput })
            .toPromise();
          if (mutationError) throw mutationError;
          return replacesPrimary
            ? data?.queueReplacementRelease
            : data?.queueExistingTitleDownload;
        },
        "A download is already in progress for this episode.",
        confirmReplaceConflict,
      )
        .then(async (payload) => {
          assertNoReplaceConflict(payload, "A download is already in progress for this episode.");
          const queuedMessage = t("status.queuedLatest", { name: title.name });
          setGlobalStatus(queuedMessage);
          await onTitleChanged?.();
        })
        .catch((error: unknown) => {
          setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
        });
    },
    [
      onTitleChanged,
      client,
      confirmReplaceConflict,
      mediaFilesByEpisode,
      setGlobalStatus,
      t,
      title,
    ],
  );

  const handleQueueAdditionalFromEpisodeSearch = React.useCallback(
    async (episode: CollectionEpisode, release: Release) => {
      if (!title) return;

      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }

      try {
        const { data, error: mutationError } = await client.mutation(queueExistingMutation, {
          input: {
            titleId: title.id,
            scope: { episode: episode.id },
            candidateToken: release.candidateToken,
            sizeBytes: release.sizeBytes ?? null,
            purpose: "ADDITIONAL_FILE",
          },
        }).toPromise();
        if (mutationError) throw mutationError;
        assertNoReplaceConflict(
          data?.queueExistingTitleDownload,
          "A download is already in progress for this episode.",
        );
        setGlobalStatus(t("status.queuedLatest", { name: title.name }));
        await onTitleChanged?.();
      } catch (error: unknown) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
      }
    },
    [onTitleChanged, client, setGlobalStatus, t, title],
  );

  const handleAutoSearchEpisode = React.useCallback(
    (episode: CollectionEpisode) => {
      if (!hasDownloadClients) {
        const episodeId = episode.id;
        dispatchEpisodePanel({ type: "RESET_SEARCH", episodeId });
        setSearchBlockedByEpisode((prev) => ({ ...prev, [episodeId]: true }));
        return;
      }
      if (!onAutoSearchEpisode) return;
      const episodeId = episode.id;
      setSearchBlockedByEpisode((prev) => {
        if (!prev[episodeId]) return prev;
        const next = { ...prev };
        delete next[episodeId];
        return next;
      });
      dispatchEpisodePanel({ type: "SET_AUTO_SEARCH_LOADING", episodeId, loading: true });
      Promise.resolve(onAutoSearchEpisode(episode))
        .catch((error: unknown) => {
          setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
        })
        .finally(() => {
          dispatchEpisodePanel({ type: "SET_AUTO_SEARCH_LOADING", episodeId, loading: false });
        });
    },
    [hasDownloadClients, onAutoSearchEpisode, setGlobalStatus, t],
  );

  const handleRunSeriesMovieSearch = React.useCallback(
    (link: SeriesMovieLink) => {
      if (!title) return;

      if (!hasDownloadClients) {
        setSearchBlockedBySeriesMovie((prev) => ({ ...prev, [link.id]: true }));
        setSeriesMovieSearchLoadingByLink((prev) => ({
          ...prev,
          [link.id]: false,
        }));
        return;
      }

      setSearchBlockedBySeriesMovie((prev) => {
        if (!prev[link.id]) return prev;
        const next = { ...prev };
        delete next[link.id];
        return next;
      });
      setSeriesMovieSearchLoadingByLink((prev) => ({
        ...prev,
        [link.id]: true,
      }));
      setSeriesMovieSearchAttemptedByLink((prev) => ({
        ...prev,
        [link.id]: true,
      }));

      seriesMovieSearchAbortByLinkRef.current[link.id]?.abort();
      const abortController = new AbortController();
      seriesMovieSearchAbortByLinkRef.current[link.id] = abortController;
      runIterativeReleaseSearch(client, {
        titleId: title.id,
        seriesMovieLinkId: link.id,
      }, {
        signal: abortController.signal,
        onUpdate: (snapshot) => {
          if (abortController.signal.aborted) return;
          setSeriesMovieSearchResultsByLink((prev) => ({
            ...prev,
            [link.id]: snapshot.releases,
          }));
        },
      })
        .then((results) => {
          if (abortController.signal.aborted) return;
          setSeriesMovieSearchResultsByLink((prev) => ({
            ...prev,
            [link.id]: results,
          }));
        })
        .catch((error) => {
          if (isAbortError(error) || abortController.signal.aborted) return;
          setSeriesMovieSearchResultsByLink((prev) => ({
            ...prev,
            [link.id]: [],
          }));
        })
        .finally(() => {
          if (seriesMovieSearchAbortByLinkRef.current[link.id] === abortController) {
            delete seriesMovieSearchAbortByLinkRef.current[link.id];
            setSeriesMovieSearchLoadingByLink((prev) => ({
              ...prev,
              [link.id]: false,
            }));
          }
        });
    },
    [client, hasDownloadClients, title],
  );

  const handleQueueFromSeriesMovieSearch = React.useCallback(
    (link: SeriesMovieLink, release: Release) => {
      if (!title) return Promise.resolve();

      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return Promise.resolve();
      }

      const input = {
        titleId: title.id,
        scope: releaseQueueScopeInput(release, { seriesMovie: link.id }),
        candidateToken: release.candidateToken,
        sizeBytes: release.sizeBytes ?? null,
      };
      const replacesPrimary = hasPrimaryMediaFile(
        mediaFilesBySeriesMovieLink[link.id],
      );
      const mutation = replacesPrimary
        ? queueReplacementMutation
        : queueExistingMutation;
      return retryWithReplaceOnConflict(
        input,
        async (nextInput) => {
          const { data, error: mutationError } = await client
            .mutation(mutation, { input: nextInput })
            .toPromise();
          if (mutationError) throw mutationError;
          return replacesPrimary
            ? data?.queueReplacementRelease
            : data?.queueExistingTitleDownload;
        },
        "A download is already in progress for this series movie.",
        confirmReplaceConflict,
      )
        .then(async (payload) => {
          assertNoReplaceConflict(
            payload,
            "A download is already in progress for this series movie.",
          );
          setGlobalStatus(
            t("status.queuedLatest", { name: link.movie.title }),
          );
          await onTitleChanged?.();
        })
        .catch((error: unknown) => {
          setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
        });
    },
    [
      client,
      confirmReplaceConflict,
      mediaFilesBySeriesMovieLink,
      onTitleChanged,
      setGlobalStatus,
      t,
      title,
    ],
  );

  const handleQueueAdditionalFromSeriesMovieSearch = React.useCallback(
    async (link: SeriesMovieLink, release: Release) => {
      if (!title) return;

      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }

      try {
        const { data, error: mutationError } = await client
          .mutation(queueExistingMutation, {
            input: {
              titleId: title.id,
              scope: releaseQueueScopeInput(release, { seriesMovie: link.id }),
              candidateToken: release.candidateToken,
              sizeBytes: release.sizeBytes ?? null,
              purpose: "ADDITIONAL_FILE",
            },
          })
          .toPromise();
        if (mutationError) throw mutationError;
        assertNoReplaceConflict(
          data?.queueExistingTitleDownload,
          "A download is already in progress for this series movie.",
        );
        setGlobalStatus(t("status.queueSuccess", { name: release.title }));
        await onTitleChanged?.();
      } catch (error: unknown) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
      }
    },
    [client, onTitleChanged, setGlobalStatus, t, title],
  );

  const handleAutoSearchSeriesMovie = React.useCallback(
    (link: SeriesMovieLink) => {
      if (!hasDownloadClients) {
        setSearchBlockedBySeriesMovie((prev) => ({ ...prev, [link.id]: true }));
        return;
      }
      if (!onAutoSearchSeriesMovie) return;
      setSearchBlockedBySeriesMovie((prev) => {
        if (!prev[link.id]) return prev;
        const next = { ...prev };
        delete next[link.id];
        return next;
      });
      setAutoSearchSeriesMovieLoadingByLink((prev) => ({
        ...prev,
        [link.id]: true,
      }));
      Promise.resolve(onAutoSearchSeriesMovie(link))
        .catch((error: unknown) => {
          setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
        })
        .finally(() => {
          setAutoSearchSeriesMovieLoadingByLink((prev) => ({
            ...prev,
            [link.id]: false,
          }));
        });
    },
    [hasDownloadClients, onAutoSearchSeriesMovie, setGlobalStatus, t],
  );

  if (loading) {
    return (
      <div className="space-y-4">
        <div className="h-8 w-48 animate-pulse rounded bg-muted" />
        <div className="h-32 animate-pulse rounded-lg bg-muted" />
        <div className="h-48 animate-pulse rounded-lg bg-muted" />
      </div>
    );
  }

  if (!title) {
    return (
      <div className="space-y-4">
        <OverviewBackLink
          id="series-overview-back-link"
          label={t("title.backToFacet", { facet: backLabel })}
          onClick={() => onBackToList?.()}
        />
        <Card>
          <CardContent className="pt-6">
            <p className="text-muted-foreground">{t("title.notFound")}</p>
          </CardContent>
        </Card>
      </div>
    );
  }

  const overviewBackdropUrl = title.backgroundUrl;

  return (
    <>
      <div className="space-y-4">
      <Card
        className={
          fullBleedHero
            ? "relative -mx-4 -mt-4 overflow-hidden rounded-none border-0 p-0 sm:-mx-5 sm:-mt-5"
            : "relative overflow-hidden p-0"
        }
        style={overviewBackdropUrl ? { backdropFilter: "none", WebkitBackdropFilter: "none" } : undefined}
      >
        {overviewBackdropUrl ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0"
            style={{ position: "absolute", inset: 0, zIndex: 0 }}
          >
            <div
              className="absolute -inset-1 scale-[1.03] bg-cover bg-no-repeat blur-[2px] brightness-[0.82] saturate-[0.9]"
              style={{
                backgroundImage: `url(${overviewBackdropUrl})`,
                backgroundPosition: "center top",
              }}
            />
            <div
              className="absolute inset-0"
              style={{
                background:
                  "linear-gradient(to top, var(--color-card) 0%, var(--color-card) 5%, color-mix(in srgb, var(--color-card) 82%, transparent), color-mix(in srgb, var(--color-card) 52%, transparent)), linear-gradient(135deg, rgba(255, 255, 255, 0.03), rgba(255, 255, 255, 0.012) 40%, transparent 100%)",
              }}
            />
            {fullBleedHero ? (
              <div className="absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-b from-transparent to-[var(--scry-bg)]" />
            ) : null}
          </div>
        ) : null}
        <CardContent className="relative p-4">
          <div className="flex flex-col gap-4 sm:flex-row sm:gap-5">
            <div className="mx-auto shrink-0 sm:mx-0">
              <TitlePosterSlot
                src={title.posterUrl}
                metadataFetchedAt={title.metadataFetchedAt}
                createdAt={title.createdAt}
                alt={title.name}
                className="block h-[300px] w-[200px] rounded-lg object-cover shadow-lg"
                placeholderClassName="flex h-[300px] w-[200px] items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60"
                emptyLabel={t("title.noPoster")}
              />
            </div>

            <div className="relative min-w-0 flex-1 flex flex-col pr-12">
              {onBackToList ? (
                <IconButton
                  label={t("label.close")}
                  tone="neutral"
                  className="absolute right-0 top-0 z-20 size-10 rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] text-[var(--scry-ink2)] shadow-[0_12px_30px_rgba(0,0,0,0.35)] backdrop-blur-sm transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]"
                  onClick={() => onBackToList()}
                >
                  <X className="h-5 w-5" />
                </IconButton>
              ) : null}
              <h1 className="text-xl font-bold text-foreground sm:text-2xl">
                {title.name}
                {title.year ? (
                  <span className="block text-base font-normal text-muted-foreground sm:ml-2 sm:inline sm:text-lg">
                    ({title.year})
                  </span>
                ) : null}
              </h1>

              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span
                  className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${
                    title.monitored
                      ? "bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]"
                      : "bg-accent text-muted-foreground"
                  }`}
                >
                  {title.monitored
                    ? t("title.monitored")
                    : t("search.monitorType.unmonitored")}
                </span>
                {localizedTitleStatus(t, title.contentStatus) ? (
                  <span className="inline-flex items-center rounded-full border border-border px-2.5 py-0.5 text-xs font-medium capitalize text-muted-foreground">
                    {localizedTitleStatus(t, title.contentStatus)}
                  </span>
                ) : null}
                {title.network ? (
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <Clapperboard className="h-3.5 w-3.5" />
                    {title.network}
                  </span>
                ) : null}
              </div>

              {titleGenreLabels(title).length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {titleGenreLabels(title).map((genre) => (
                    <span
                      key={genre}
                      className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground"
                    >
                      {genre}
                    </span>
                  ))}
                </div>
              ) : null}

              <TitleRatingsStrip ratings={title.ratings} />

              {title.overview ? (
                <p className="mt-4 text-sm leading-relaxed text-foreground/70">
                  {title.overview}
                </p>
              ) : null}

              <div className="mt-auto flex flex-wrap items-center gap-2 pt-3">
                {(() => {
                  const externalIds = title.externalIds ?? [];
                  return (
                    <>
                      <ImdbExternalLink
                        imdbId={externalIds.find((e) => e.source === "imdb")?.value}
                        size="compact"
                      />
                      <TvdbSeriesExternalLink
                        tvdbId={externalIds.find((e) => e.source === "tvdb")?.value}
                        slug={title.slug}
                        size="compact"
                      />
                      <TmdbExternalLink
                        mediaType="tv"
                        tmdbId={externalIds.find((e) => e.source === "tmdb")?.value}
                        size="compact"
                      />
                    </>
                  );
                })()}
                {title.facet === "ANIME" ? (
                  <>
                    {(() => {
                      const externalIds = title.externalIds ?? [];
                      return (
                        <>
                          <MalExternalLink
                            malId={externalIds.find((e) => e.source === "mal")?.value}
                            size="compact"
                          />
                          <AnilistExternalLink
                            anilistId={externalIds.find((e) => e.source === "anilist")?.value}
                            size="compact"
                          />
                          <AnidbExternalLink
                            anidbId={externalIds.find((e) => e.source === "anidb")?.value}
                            size="compact"
                          />
                        </>
                      );
                    })()}
                  </>
                ) : null}
                <span className="ml-auto text-xs text-muted-foreground/60">
                  {t("title.addedAt", {
                    date: formatDate(title.createdAt, dateTimeFormat),
                  })}
                </span>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

      {canManageTitle ? (
        <OverviewControlPanel
          monitored={title.monitored}
          monitoredUpdating={monitoredUpdating}
          searchMonitoredLoading={searchMonitoredLoading}
          refreshAndScanLoading={refreshAndScanLoading}
          deleteLoading={deleteLoading}
          onToggleMonitoring={onSetTitleMonitored ? () => void onSetTitleMonitored(!title.monitored) : undefined}
          onSearchMonitored={onSearchMonitored ? () => void onSearchMonitored() : undefined}
          onRefreshAndScan={onRefreshAndScan ? () => void onRefreshAndScan() : undefined}
          onRequestDelete={onRequestDeleteTitle}
          onHistory={handleOpenTitleHistory}
          searchNotice={searchPrerequisiteNotice}
          settingsPanel={
            onUpdateTitleOptions && qualityProfiles && defaultRootFolder ? (
              <TitleSettingsPanel
                title={title}
                qualityProfiles={qualityProfiles}
                defaultRootFolder={defaultRootFolder}
                renameEnabled={renameEnabled !== false}
                rootFolders={rootFolders ?? []}
                onUpdateTitleOptions={onUpdateTitleOptions}
                onTitleChanged={onTitleChanged}
                onOpenFixMatch={onOpenFixMatch}
              />
            ) : undefined
          }
        />
      ) : null}

      <div>
        <Card className="relative overflow-hidden">
          <CardHeader>
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <CardTitle className="flex items-center gap-2 text-base">
                <FolderOpen className="h-4 w-4" />
                {t("title.seasonsAndEpisodes")}
              </CardTitle>
              {canManageTitle && onOpenManualImport && completedDownloads && completedDownloads.length > 0 ? (
                <Button
                  className="w-full sm:w-auto"
                  variant="outline"
                  size="sm"
                  onClick={() => onOpenManualImport(completedDownloads[0])}
                >
                  <FileInput className="mr-1.5 h-4 w-4" />
                  {t("queue.manualImport")}
                </Button>
              ) : null}
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            {episodeSelectionEnabled && selectedEpisodeIds.size > 0 ? (
              <div className="flex flex-wrap items-center justify-end gap-2">
                <Button
                  id={SERIES_OVERVIEW_CLEAR_EPISODE_SELECTION_ID}
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={handleClearEpisodeSelection}
                >
                  {t("seriesOverview.clearEpisodeSelection")}
                </Button>
                <Button
                  id={SERIES_OVERVIEW_DELETE_SELECTED_EPISODES_ID}
                  type="button"
                  variant="destructive"
                  size="sm"
                  onClick={handleDeleteSelectedEpisodeFiles}
                >
                  <Trash2 className="mr-1.5 h-4 w-4" />
                  {t("seriesOverview.deleteSelectedEpisodeFiles", {
                    count: selectedEpisodeIds.size,
                  })}
                </Button>
              </div>
            ) : null}
            {timelineItems.length > 0 ? (
              <>
              {timelineItems.map((item) => {
                if (item.kind === "seriesMovie") {
                  return (
                    <SeriesMovieTimelineSection
                      key={item.key}
                      link={item.link}
                      expanded={expandedKeys.has(item.key)}
                      onToggle={toggleActionByKey.get(item.key)!}
                      mediaFilesByEpisode={mediaFilesByEpisode}
                      mediaFilesBySeriesMovieLink={mediaFilesBySeriesMovieLink}
                      onLoadSeriesMovieDetail={onLoadSeriesMovieDetail}
                      subtitleDownloads={subtitleDownloads}
                      onRefreshSubtitles={canManageTitle ? onRefreshSubtitles : undefined}
                      seriesMovieSearchResultsByLink={seriesMovieSearchResultsByLink}
                      seriesMovieSearchLoadingByLink={seriesMovieSearchLoadingByLink}
                      seriesMovieSearchAttemptedByLink={seriesMovieSearchAttemptedByLink}
                      searchBlockedBySeriesMovie={searchBlockedBySeriesMovie}
                      onRunSeriesMovieSearch={canManageTitle ? handleRunSeriesMovieSearch : undefined}
                      onQueueFromSeriesMovieSearch={canManageTitle ? handleQueueFromSeriesMovieSearch : undefined}
                      onQueueAdditionalFromSeriesMovieSearch={canManageTitle ? handleQueueAdditionalFromSeriesMovieSearch : undefined}
                      onAutoSearchSeriesMovie={canManageTitle && onAutoSearchSeriesMovie ? handleAutoSearchSeriesMovie : undefined}
                      onSetSeriesMovieMonitored={canManageTitle ? onSetSeriesMovieMonitored : undefined}
                      onDeleteFile={canManageTitle ? onDeleteFile : undefined}
                      onMakePrimaryFile={canManageTitle ? onMakePrimaryFile : undefined}
                      primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
                      autoSearchSeriesMovieLoadingByLink={autoSearchSeriesMovieLoadingByLink}
                    />
                  );
                }

                const { collection } = item;
                const sortedEpisodes = sortedEpisodesByCollection[collection.id] ?? emptyEpisodes;

                if (
                  isSpecialsCollection(collection) &&
                  (collection.episodeRecordsTotal ?? 0) === 0 &&
                  sortedEpisodes.length === 0
                ) {
                  return null;
                }

                return (
                  <SeasonSection
                    key={item.key}
                    collection={collection}
                    episodes={sortedEpisodes}
                    episodesReady={collection.id in episodesByCollection}
                    facet={title.facet}
                    expanded={expandedKeys.has(item.key)}
                    onToggle={toggleActionByKey.get(item.key)!}
                    initiallyOpenEpisodeId={initialEpisodeId}
                    mediaFilesByEpisode={mediaFilesByEpisode}
              onLoadEpisodeDetail={onLoadEpisodeDetail}
                    activeDownloadEpisodeIds={activeDownloadEpisodeIds}
                    downloadQueueItemByEpisodeId={primaryQueueItemByEpisodeId}
                    subtitleDownloads={subtitleDownloads}
                    onRefreshSubtitles={canManageTitle ? onRefreshSubtitles : undefined}
                    onMakePrimaryFile={canManageTitle ? onMakePrimaryFile : undefined}
                    primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
                    releaseBlocklistEntries={releaseBlocklistEntries}
                    clearingReleaseBlocklistEntryId={clearingReleaseBlocklistEntryId}
                    onClearReleaseBlocklistEntry={
                      canManageTitle ? onClearReleaseBlocklistEntry : undefined
                    }
                    searchResultsByEpisode={episodePanel.searchResultsByEpisode}
                    searchIndexerProgressByEpisode={episodePanel.searchIndexerProgressByEpisode}
                    searchLoadingByEpisode={episodePanel.searchLoadingByEpisode}
                    searchBlockedByEpisode={searchBlockedByEpisode}
                    autoSearchLoadingByEpisode={episodePanel.autoSearchLoadingByEpisode}
                    onRunEpisodeSearch={canManageTitle ? handleRunEpisodeSearch : undefined}
                    onOpenEpisodeHistory={canManageTitle ? handleOpenEpisodeHistory : undefined}
                    onQueueFromEpisodeSearch={canManageTitle ? handleQueueFromEpisodeSearch : undefined}
                    onQueueAdditionalFromEpisodeSearch={
                      canManageTitle ? handleQueueAdditionalFromEpisodeSearch : undefined
                    }
                    onAutoSearchEpisode={canManageTitle ? handleAutoSearchEpisode : undefined}
                    onSetCollectionMonitored={canManageTitle ? onSetCollectionMonitored : undefined}
                    onSetEpisodeMonitored={canManageTitle ? onSetEpisodeMonitored : undefined}
                    seasonSearchResults={seasonSearchResultsByCollection?.[collection.id]}
                    seasonSearchLoading={seasonSearchLoadingByCollection?.[collection.id] === true}
                    onRunSeasonSearch={seasonSearchActionByKey.get(item.key)}
                    searchBlocked={searchBlockedByCollection[collection.id] === true}
                    onQueueFromSeasonSearch={canManageTitle ? onQueueFromSeasonSearch : undefined}
                    onDeleteFile={canManageTitle ? onDeleteFile : undefined}
                    selectedEpisodeIds={episodeSelectionEnabled ? selectedEpisodeIds : undefined}
                    pendingEpisodeIds={episodeSelectionEnabled ? pendingEpisodeIds : undefined}
                    onToggleEpisodeSelected={
                      episodeSelectionEnabled ? handleToggleEpisodeSelected : undefined
                    }
                    onSetSeasonSelected={
                      episodeSelectionEnabled ? handleSetSeasonSelected : undefined
                    }
                  />
                );
              })}
              </>
            ) : (
              <p className="text-sm text-muted-foreground">
                {t("title.noTrackedSeasons")}
              </p>
            )}
          </CardContent>
          {hydrating ? (
            <div className="absolute inset-0 z-10 flex items-center justify-center bg-background/75 backdrop-blur-sm">
              <div className="flex items-center gap-3 rounded-full border border-border bg-card/95 px-4 py-2 text-sm font-medium text-foreground shadow-lg">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>{t("title.fetchingData")}</span>
              </div>
            </div>
          ) : null}
        </Card>
      </div>

      <TitleMoreLikeThisStrip
        items={title.moreLikeThis ?? []}
        fallbackYearLabel={title.facet === "ANIME" ? t("nav.anime") : t("nav.series")}
        {...moreLikeThisActions}
      />

      <TitleCastStrip credits={titleCastOriginalCredits(title.credits)} />

      <TitleDubCastStrip credits={title.credits} />

      <details className="rounded-xl border border-border bg-card text-card-foreground overflow-hidden">
        <summary className="cursor-pointer select-none px-4 py-3 text-sm font-medium text-card-foreground">
          <span className="inline-flex items-center gap-2">
            {t("title.blockedReleases")}
            <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
              {releaseBlocklistEntries.length}
            </span>
          </span>
        </summary>
        <div className="border-t border-border p-4">
          {releaseBlocklistEntries.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {t("title.noBlockedReleases")}
            </p>
          ) : (
            <div className="space-y-2">
              {releaseBlocklistEntries.map((entry) => (
                <div
                  key={entry.id}
                  className="rounded-lg border border-border bg-background/35 p-3"
                >
                  <div className="flex items-center justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <p className="break-words text-sm text-card-foreground">
                        {entry.releaseName || t("episode.untitledRelease")}
                      </p>
                      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
                        <span className="text-muted-foreground/60">
                          {formatDate(entry.attemptedAt, dateTimeFormat)}
                        </span>
                        {entry.errorMessage ? (
                          <span className="rounded bg-[var(--scry-danger-bg)] px-2 py-0.5 text-[var(--scry-danger-text)]">
                            {entry.errorMessage}
                          </span>
                        ) : null}
                      </div>
                    </div>
                    {canManageTitle && onClearReleaseBlocklistEntry ? (
                      <Button
                        type="button"
                        variant="destructive"
                        size="sm"
                        className="h-8 shrink-0 px-3"
                        disabled={clearingReleaseBlocklistEntryId === entry.id}
                        onClick={() => onClearReleaseBlocklistEntry(entry.id)}
                      >
                        {clearingReleaseBlocklistEntryId === entry.id ? (
                          <Loader2 className="size-3.5 animate-spin" />
                        ) : null}
                        <span>{t("label.clear")}</span>
                      </Button>
                    ) : null}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </details>

      {title ? (
        <TitleHistoryModal
          open={historyOpen}
          onOpenChange={setHistoryOpen}
          titleId={title.id}
          titleName={title.name}
          scopedEpisode={historyEpisodeScope}
        />
      ) : null}
      {replaceConflictDialog}
      </div>
    </>
  );
}

export const SeriesOverviewView = React.memo(SeriesOverviewViewImpl);
