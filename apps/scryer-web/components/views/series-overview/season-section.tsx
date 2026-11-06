import * as React from "react";
import { RefreshScoresButton } from "@/components/common/refresh-scores-button";
import { useCanManageOverviewTitle } from "@/lib/hooks/use-title-overview-access";
import {
  ChevronDown,
  ChevronRight,
  EllipsisVertical,
  Eye,
  EyeOff,
  History,
  Search,
  Zap,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableActionsHead,
  TableBody,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { IconButton } from "@/components/ui/icon-button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import {
  ReleaseSearchLoader,
  ReleaseSearchSummary,
} from "@/components/common/release-search-status";
import { deriveInteractiveSearchPresentation } from "@/lib/utils/interactive-search-presentation";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import { useTranslate } from "@/lib/context/translate-context";
import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";
import { cn } from "@/lib/utils";
import { InteractiveSearchDialog } from "@/components/common/interactive-search-dialog";
import { useIsNarrowElement } from "@/lib/hooks/use-mobile";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
} from "@/lib/utils/action-button-styles";
import {
  selectorId,
  seriesOverviewSeasonInteractiveSearchId,
  seriesOverviewSeasonMonitorId,
  seriesOverviewSeasonSectionId,
  seriesOverviewSeasonSearchId,
  seriesOverviewSeasonSelectId,
  seriesOverviewSeasonToggleId,
} from "@/lib/utils/dom-ids";
import {
  EpisodeProgressBar,
  getCollectionEpisodeProgressPresentation,
} from "@/components/views/media-content/title-table-shared";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import { sameDownloadQueueItem } from "@/lib/utils/download-queue";
import { seasonHasSelectableEpisodes } from "@/lib/utils/season-selection";
import {
  formatFileSize,
  isEpisodeCountableForProgress,
  isSpecialsCollection,
  seasonHeading,
} from "./helpers";
import { EpisodeRow } from "./episode-row";
import {
  EpisodeTableActionButton,
  EMPTY_EPISODE_FILES,
  EMPTY_RELEASES,
  EMPTY_SUBTITLE_DOWNLOADS,
} from "./season-section-utils";
import { LoadingMark } from "@/components/common/loading-mark";

const EMPTY_INDEXER_PROGRESS: InteractiveSearchIndexerProgress[] = [];
// Below this section width the episode table squeezes titles into a
// one-word column, so episodes render as cards instead.
const EPISODE_TABLE_MIN_WIDTH = 720;

export { SeriesMovieTimelineSection } from "./series-movie-row";

// The inline actions, size and progress take about 340px and the heading
// truncates into the rest, so they still fit a desktop side panel (about
// 620px at a 1440px window). Below this the heading would be squeezed to a
// few characters, so phones get the actions menu instead.
const SEASON_ACTIONS_MENU_MAX_WIDTH = 600;

function SeasonActionsMenuItem(
  props: Omit<React.ComponentProps<typeof Button>, "size" | "variant">,
) {
  return (
    <Button
      type="button"
      size="sm"
      variant="ghost"
      className="w-full flex-row-reverse justify-start"
      {...props}
    />
  );
}

type SeasonSectionProps = {
  collection: TitleCollection;
  facet: string;
  episodes: CollectionEpisode[];
  episodesReady?: boolean;
  expanded: boolean;
  onToggle: () => void;
  initiallyOpenEpisodeId?: string | null;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  onLoadEpisodeDetail?: (episodeId: string) => Promise<void> | void;
  activeDownloadEpisodeIds?: ReadonlySet<string>;
  downloadQueueItemByEpisodeId?: Record<string, DownloadQueueItem | undefined>;
  subtitleDownloads?: ExternalSubtitleRecord[];
  /** Library the series' media files belong to; gates subtitle controls. */
  libraryId: string | null;
  onRefreshSubtitles?: () => Promise<void> | void;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  clearingReleaseBlocklistEntryId?: string | null;
  searchResultsByEpisode: Record<string, Release[]>;
  searchIndexerProgressByEpisode: Record<
    string,
    InteractiveSearchIndexerProgress[]
  >;
  searchLoadingByEpisode: Record<string, boolean>;
  searchBlockedByEpisode: Record<string, boolean>;
  autoSearchLoadingByEpisode: Record<string, boolean>;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onRunEpisodeSearch?: (episode: CollectionEpisode) => void;
  onOpenEpisodeHistory?: (episode: CollectionEpisode) => void;
  onOpenSeasonHistory?: (collection: TitleCollection) => void;
  onQueueFromEpisodeSearch?: (
    episode: CollectionEpisode,
    release: Release,
  ) => Promise<void> | void;
  onQueueAdditionalFromEpisodeSearch?: (
    episode: CollectionEpisode,
    release: Release,
  ) => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => void;
  onSetCollectionMonitored?: (
    collectionId: string,
    monitored: boolean,
  ) => Promise<void>;
  onSetEpisodeMonitored?: (
    episodeId: string,
    monitored: boolean,
  ) => Promise<void>;
  seasonSearchResults?: Release[];
  seasonSearchLoading?: boolean;
  searchBlocked?: boolean;
  onRunSeasonSearch?: () => void;
  seasonInteractiveSearchLoading?: boolean;
  seasonSearchIndexerProgress?: InteractiveSearchIndexerProgress[];
  /** Interactive search of the whole season, listed under this header. */
  onRunSeasonInteractiveSearch?: () => void;
  onQueueFromSeasonSearch?: (
    collection: TitleCollection,
    release: Release,
  ) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
  onAssignFileToEpisode?: (fileId: string) => void;
  primaryMovieFileUpdatingId?: string | null;
  /**
   * Episode ids currently selected for the bulk file delete. Selection is only
   * wired when the viewer can manage the title.
   */
  selectedEpisodeIds?: ReadonlySet<string>;
  /** Episodes locked by an in-flight file-deletion job; their checkboxes are disabled. */
  pendingEpisodeIds?: ReadonlySet<string>;
  onToggleEpisodeSelected?: (episodeId: string) => void;
  onSetSeasonSelected?: (episodeIds: string[], selected: boolean) => void;
  /**
   * Ticks or unticks the header checkbox of a season whose episodes have not
   * loaded yet. The owner loads them and selects the season once they arrive.
   */
  onToggleUnloadedSeasonSelected?: (collectionId: string) => void;
  /** The season was ticked before its episodes loaded and is waiting on them. */
  seasonSelectionQueued?: boolean;
};

const EPISODE_SCOPED_PROPS = new Set<keyof SeasonSectionProps>([
  "activeDownloadEpisodeIds",
  "autoSearchLoadingByEpisode",
  "downloadQueueItemByEpisodeId",
  "mediaFilesByEpisode",
  "searchBlockedByEpisode",
  "searchIndexerProgressByEpisode",
  "searchLoadingByEpisode",
  "searchResultsByEpisode",
  // Compared per episode below: the Set identity changes on every selection
  // change, but only the seasons whose episodes actually changed must re-render.
  "selectedEpisodeIds",
  "pendingEpisodeIds",
]);

function sameSeasonSectionProps(
  previous: SeasonSectionProps,
  next: SeasonSectionProps,
): boolean {
  for (const key of [
    ...new Set([...Object.keys(previous), ...Object.keys(next)]),
  ] as (keyof SeasonSectionProps)[]) {
    if (
      !EPISODE_SCOPED_PROPS.has(key) &&
      !Object.is(previous[key], next[key])
    ) {
      return false;
    }
  }

  for (const episode of previous.episodes) {
    const episodeId = episode.id;
    if (
      (previous.activeDownloadEpisodeIds?.has(episodeId) ?? false) !==
        (next.activeDownloadEpisodeIds?.has(episodeId) ?? false) ||
      previous.autoSearchLoadingByEpisode[episodeId] !==
        next.autoSearchLoadingByEpisode[episodeId] ||
      previous.mediaFilesByEpisode[episodeId] !==
        next.mediaFilesByEpisode[episodeId] ||
      previous.searchBlockedByEpisode[episodeId] !==
        next.searchBlockedByEpisode[episodeId] ||
      previous.searchIndexerProgressByEpisode[episodeId] !==
        next.searchIndexerProgressByEpisode[episodeId] ||
      previous.searchLoadingByEpisode[episodeId] !==
        next.searchLoadingByEpisode[episodeId] ||
      previous.searchResultsByEpisode[episodeId] !==
        next.searchResultsByEpisode[episodeId] ||
      (previous.selectedEpisodeIds?.has(episodeId) ?? false) !==
        (next.selectedEpisodeIds?.has(episodeId) ?? false) ||
      (previous.pendingEpisodeIds?.has(episodeId) ?? false) !==
        (next.pendingEpisodeIds?.has(episodeId) ?? false)
    ) {
      return false;
    }

    const previousQueueItem =
      previous.downloadQueueItemByEpisodeId?.[episodeId];
    const nextQueueItem = next.downloadQueueItemByEpisodeId?.[episodeId];
    if (
      (previousQueueItem === undefined) !== (nextQueueItem === undefined) ||
      (previousQueueItem !== undefined &&
        nextQueueItem !== undefined &&
        !sameDownloadQueueItem(previousQueueItem, nextQueueItem))
    ) {
      return false;
    }
  }

  return true;
}

function SeasonSectionImpl({
  collection,
  episodes,
  episodesReady = true,
  expanded,
  facet,
  onToggle,
  initiallyOpenEpisodeId,
  mediaFilesByEpisode,
  onLoadEpisodeDetail,
  activeDownloadEpisodeIds,
  downloadQueueItemByEpisodeId,
  releaseBlocklistEntries,
  clearingReleaseBlocklistEntryId,
  subtitleDownloads,
  libraryId,
  onRefreshSubtitles,
  searchResultsByEpisode,
  searchIndexerProgressByEpisode,
  searchLoadingByEpisode,
  searchBlockedByEpisode,
  onRunEpisodeSearch,
  onOpenEpisodeHistory,
  onOpenSeasonHistory,
  onQueueFromEpisodeSearch,
  onQueueAdditionalFromEpisodeSearch,
  autoSearchLoadingByEpisode,
  onAutoSearchEpisode,
  onClearReleaseBlocklistEntry,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  seasonSearchResults,
  seasonSearchLoading,
  searchBlocked = false,
  onRunSeasonSearch,
  seasonInteractiveSearchLoading = false,
  seasonSearchIndexerProgress = EMPTY_INDEXER_PROGRESS,
  onRunSeasonInteractiveSearch,
  onQueueFromSeasonSearch,
  onDeleteFile,
  onMakePrimaryFile,
  onAssignFileToEpisode,
  primaryMovieFileUpdatingId = null,
  selectedEpisodeIds,
  pendingEpisodeIds,
  onToggleEpisodeSelected,
  onSetSeasonSelected,
  onToggleUnloadedSeasonSelected,
  seasonSelectionQueued = false,
}: SeasonSectionProps) {
  const t = useTranslate();
  const seasonSearchPresentation = React.useMemo(
    () =>
      deriveInteractiveSearchPresentation({
        hasSnapshot: seasonSearchResults !== undefined,
        loading: seasonInteractiveSearchLoading,
        resultCount: seasonSearchResults?.length ?? 0,
        indexers: seasonSearchIndexerProgress,
      }),
    [
      seasonInteractiveSearchLoading,
      seasonSearchIndexerProgress,
      seasonSearchResults,
    ],
  );
  const canRefreshScores = useCanManageOverviewTitle(libraryId);
  const sectionRef = React.useRef<HTMLDivElement>(null);
  // The overview also renders inside a side panel, so the episode table
  // gives way to cards by the section's own width, not the window's.
  const isMobile = useIsNarrowElement(sectionRef, EPISODE_TABLE_MIN_WIDTH);
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const [seasonToggling, setSeasonToggling] = React.useState(false);
  const [seasonSearchOpen, setSeasonSearchOpen] = React.useState(false);
  const actionsInMenu = useIsNarrowElement(
    sectionRef,
    SEASON_ACTIONS_MENU_MAX_WIDTH,
  );
  const [actionsMenuOpen, setActionsMenuOpen] = React.useState(false);
  const stableSubtitleDownloads = subtitleDownloads ?? EMPTY_SUBTITLE_DOWNLOADS;
  const selectionEnabled = Boolean(onToggleEpisodeSelected);

  // Episodes an in-flight deletion job holds are out of the season checkbox's
  // reach, so its state reflects only the rows the operator can still act on.
  const selectableEpisodeIds = React.useMemo(
    () =>
      episodes
        .filter((episode) => !(pendingEpisodeIds?.has(episode.id) ?? false))
        .map((episode) => episode.id),
    [episodes, pendingEpisodeIds],
  );

  const seasonSelectionState: boolean | "indeterminate" = React.useMemo(() => {
    if (selectableEpisodeIds.length === 0) {
      return false;
    }
    const selectedCount = selectableEpisodeIds.filter(
      (episodeId) => selectedEpisodeIds?.has(episodeId) ?? false,
    ).length;
    if (selectedCount === 0) {
      return false;
    }
    return selectedCount === selectableEpisodeIds.length
      ? true
      : "indeterminate";
  }, [selectableEpisodeIds, selectedEpisodeIds]);

  const seasonSelectable = seasonHasSelectableEpisodes({
    episodesReady,
    selectableEpisodeCount: selectableEpisodeIds.length,
    episodeRecordsTotal: collection.episodeRecordsTotal,
  });
  const seasonCheckboxState =
    !episodesReady && seasonSelectionQueued ? true : seasonSelectionState;

  const handleToggleSeasonSelection = React.useCallback(() => {
    if (!episodesReady) {
      // A collapsed season has not loaded its episodes; the owner loads them
      // and applies the selection once they arrive.
      onToggleUnloadedSeasonSelected?.(collection.id);
      return;
    }
    if (!onSetSeasonSelected) {
      return;
    }
    onSetSeasonSelected(selectableEpisodeIds, seasonSelectionState !== true);
  }, [
    collection.id,
    episodesReady,
    onSetSeasonSelected,
    onToggleUnloadedSeasonSelected,
    seasonSelectionState,
    selectableEpisodeIds,
  ]);

  const seasonCheckedState: boolean | "indeterminate" = React.useMemo(() => {
    if (episodes.length === 0) {
      // Episodes hydrate lazily; before they load, derive the eye state from
      // the SQL aggregate counts so collapsed seasons still reflect episode
      // monitoring.
      const aggregateTotal = collection.episodesTotal;
      const aggregateMonitored = collection.episodesMonitored;
      if (
        typeof aggregateTotal === "number" &&
        aggregateTotal > 0 &&
        typeof aggregateMonitored === "number"
      ) {
        if (aggregateMonitored === 0) {
          return false;
        }
        if (aggregateMonitored >= aggregateTotal) {
          return true;
        }
        return "indeterminate";
      }
      return collection.monitored;
    }

    const monitoredCount = episodes.filter(
      (episode) => episode.monitored,
    ).length;
    if (monitoredCount === 0) {
      return false;
    }
    if (monitoredCount === episodes.length) {
      return true;
    }
    return "indeterminate";
  }, [
    collection.episodesMonitored,
    collection.episodesTotal,
    collection.monitored,
    episodes,
  ]);

  const episodeRangeLabel = React.useMemo(() => {
    if (!collection.firstEpisodeNumber && !collection.lastEpisodeNumber) {
      return null;
    }

    return t("title.episodeRange", {
      start: collection.firstEpisodeNumber ?? "?",
      end: collection.lastEpisodeNumber ?? "?",
    });
  }, [collection.firstEpisodeNumber, collection.lastEpisodeNumber, t]);

  const collectionMetrics = React.useMemo(() => {
    const aggregateTotalEpisodes =
      typeof collection.episodesTotal === "number" &&
      collection.episodesTotal >= 0
        ? collection.episodesTotal
        : null;
    const aggregateMonitoredEpisodes =
      typeof collection.episodesMonitored === "number" &&
      collection.episodesMonitored >= 0
        ? collection.episodesMonitored
        : null;
    const aggregateOwnedEpisodes =
      typeof collection.episodesOwned === "number" &&
      collection.episodesOwned >= 0
        ? collection.episodesOwned
        : null;
    let totalEpisodes = 0;
    let monitoredEpisodes = 0;
    let ownedEpisodes = 0;

    for (const episode of episodes) {
      if (!isEpisodeCountableForProgress(episode)) {
        continue;
      }

      totalEpisodes += 1;

      if (episode.monitored) {
        monitoredEpisodes += 1;
      }

      const episodeFiles =
        mediaFilesByEpisode[episode.id] ?? EMPTY_EPISODE_FILES;
      if (episodeFiles.length > 0) {
        ownedEpisodes += 1;
      }
    }

    return {
      totalEpisodes: aggregateTotalEpisodes ?? totalEpisodes,
      monitoredEpisodes: aggregateMonitoredEpisodes ?? monitoredEpisodes,
      ownedEpisodes: aggregateOwnedEpisodes ?? ownedEpisodes,
    };
  }, [
    collection.episodesMonitored,
    collection.episodesOwned,
    collection.episodesTotal,
    episodes,
    mediaFilesByEpisode,
  ]);

  const collectionEpisodeProgress = React.useMemo(() => {
    if (!collectionMetrics || collectionMetrics.totalEpisodes <= 0) {
      return null;
    }

    return getCollectionEpisodeProgressPresentation({
      ownedEpisodes: collectionMetrics.ownedEpisodes,
      totalEpisodes: collectionMetrics.totalEpisodes,
      monitoredEpisodes: collectionMetrics.monitoredEpisodes,
      t,
    });
  }, [collectionMetrics, t]);

  const collectionSizeLabel = React.useMemo(() => {
    if (typeof collection.sizeBytes === "number" && collection.sizeBytes >= 0) {
      return formatFileSize(collection.sizeBytes);
    }

    return null;
  }, [collection.sizeBytes]);

  const isSpecials = isSpecialsCollection(collection);
  const showCollectionHeader = true;
  const showSectionContent = expanded;

  return (
    <div
      ref={sectionRef}
      id={seriesOverviewSeasonSectionId(collection.id)}
      data-timeline-kind="collection"
      data-collection-id={collection.id}
      className="overflow-hidden rounded-lg border border-border bg-background/40"
    >
      {showCollectionHeader ? (
        <div
          id={seriesOverviewSeasonToggleId(collection.collectionIndex)}
          role="button"
          tabIndex={0}
          aria-expanded={expanded}
          onClick={onToggle}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              onToggle();
            }
          }}
          // The section clips to its rounded border, so the focus ring is drawn
          // inset and the header takes the section's corner radius, keeping the
          // ring whole at the corners.
          className={cn(
            "flex w-full cursor-pointer flex-wrap items-center justify-between gap-3 bg-card/60 px-4 py-2 text-left transition hover:bg-accent/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--scry-focus)]",
            expanded ? "rounded-t-[inherit]" : "rounded-[inherit]",
          )}
        >
          <div className="flex min-w-0 flex-1 items-center gap-2">
            {selectionEnabled ? (
              // On desktop this mirrors the table's 40px selection column
              // (pulled back through the header's px-4) so the season and
              // episode checkboxes share one vertical line.
              <div
                className={cn(
                  "flex shrink-0 items-center justify-center",
                  !isMobile && "-ml-4 mr-2 w-10",
                )}
              >
                <Checkbox
                  id={seriesOverviewSeasonSelectId(collection.id)}
                  size="table"
                  className="shrink-0"
                  checked={seasonCheckboxState}
                  disabled={
                    !seasonSelectable ||
                    (!episodesReady && !onToggleUnloadedSeasonSelected)
                  }
                  aria-label={t("seriesOverview.selectSeasonForDelete", {
                    name: seasonHeading(collection, t),
                  })}
                  onClick={(event) => event.stopPropagation()}
                  onCheckedChange={handleToggleSeasonSelection}
                />
              </div>
            ) : null}
            <button
              id={seriesOverviewSeasonMonitorId(collection.id)}
              type="button"
              disabled={!onSetCollectionMonitored || seasonToggling}
              aria-label={t("title.seasonMonitored")}
              className={cn(
                "inline-flex size-6 shrink-0 items-center justify-center rounded transition-colors",
                seasonToggling && "opacity-50",
                seasonCheckedState === true
                  ? "text-[var(--scry-success-text-soft)]"
                  : seasonCheckedState === "indeterminate"
                    ? "text-[var(--scry-warning-text)]"
                    : "text-muted-foreground/60",
              )}
              onClick={(event) => {
                event.stopPropagation();
                if (!onSetCollectionMonitored) {
                  return;
                }

                setSeasonToggling(true);
                const nextMonitored = seasonCheckedState !== true;
                onSetCollectionMonitored(collection.id, nextMonitored).finally(
                  () => setSeasonToggling(false),
                );
              }}
            >
              {seasonCheckedState === false ? (
                <EyeOff className="size-5" />
              ) : (
                <Eye className="size-5" />
              )}
            </button>
            <Chevron className="h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0">
              <p className="truncate text-sm font-semibold text-foreground">
                {seasonHeading(collection, t)}
              </p>
              {episodeRangeLabel ? (
                <p className="text-xs text-muted-foreground">
                  {episodeRangeLabel}
                </p>
              ) : null}
            </div>
          </div>
          <div
            className={cn(
              "ml-auto flex shrink-0 items-center gap-2 empty:hidden",
              actionsInMenu && "gap-1.5",
            )}
          >
            {collectionSizeLabel ? (
              <span className="text-xs tabular-nums text-muted-foreground">
                {collectionSizeLabel}
              </span>
            ) : null}
            {!isSpecials && collectionEpisodeProgress ? (
              <EpisodeProgressBar
                progress={collectionEpisodeProgress}
                compact
                className={actionsInMenu ? "w-20" : "w-[6.75rem]"}
              />
            ) : null}
            {actionsInMenu ? (
              <Popover open={actionsMenuOpen} onOpenChange={setActionsMenuOpen}>
                <PopoverTrigger asChild>
                  <Button
                    id={`season-actions-${collection.id}`}
                    type="button"
                    size="icon-sm"
                    variant="secondary"
                    aria-label={`${seasonHeading(collection, t)} — ${t("label.actions")}`}
                    className={cn(
                      boxedActionButtonBaseClass,
                      boxedActionButtonToneClass.neutral,
                      "ml-auto shrink-0",
                    )}
                    onKeyDown={(event) => event.stopPropagation()}
                    onClick={(event) => event.stopPropagation()}
                  >
                    <EllipsisVertical className="h-4 w-4" />
                  </Button>
                </PopoverTrigger>
                <PopoverContent
                  align="end"
                  className="flex w-max flex-col gap-0.5 p-1"
                  onKeyDown={(event) => event.stopPropagation()}
                  onClick={(event) => event.stopPropagation()}
                >
                  {canRefreshScores ? (
                    <RefreshScoresButton
                      appearance="menu"
                      id={`season-refresh-scores-${collection.id}`}
                      selection={{ seasonIds: [collection.id] }}
                    />
                  ) : null}
                  {onOpenSeasonHistory ? (
                    <SeasonActionsMenuItem
                      id={`season-history-${collection.id}`}
                      onClick={() => {
                        setActionsMenuOpen(false);
                        onOpenSeasonHistory(collection);
                      }}
                    >
                      <History className="h-4 w-4" />
                      {t("history.title")}
                    </SeasonActionsMenuItem>
                  ) : null}
                  {onRunSeasonSearch ? (
                    <SeasonActionsMenuItem
                      id={seriesOverviewSeasonSearchId(collection.id)}
                      disabled={seasonSearchLoading === true}
                      onClick={() => {
                        setActionsMenuOpen(false);
                        onRunSeasonSearch();
                      }}
                    >
                      {seasonSearchLoading === true ? (
                        <LoadingMark className="h-4 w-4" />
                      ) : (
                        <Zap className="h-4 w-4" />
                      )}
                      {t("label.search")}
                    </SeasonActionsMenuItem>
                  ) : null}
                  {onRunSeasonInteractiveSearch && onQueueFromSeasonSearch ? (
                    <SeasonActionsMenuItem
                      id={seriesOverviewSeasonInteractiveSearchId(
                        collection.id,
                      )}
                      disabled={seasonInteractiveSearchLoading}
                      onClick={() => {
                        setActionsMenuOpen(false);
                        setSeasonSearchOpen(true);
                        onRunSeasonInteractiveSearch();
                      }}
                    >
                      {seasonInteractiveSearchLoading ? (
                        <LoadingMark className="h-4 w-4" />
                      ) : (
                        <Search className="h-4 w-4" />
                      )}
                      {t("label.interactiveSearch")}
                    </SeasonActionsMenuItem>
                  ) : null}
                </PopoverContent>
              </Popover>
            ) : null}
            {canRefreshScores && !actionsInMenu ? (
              <RefreshScoresButton
                appearance="icon"
                id={`season-refresh-scores-${collection.id}`}
                selection={{ seasonIds: [collection.id] }}
              />
            ) : null}
            {onOpenSeasonHistory && !actionsInMenu ? (
              <IconButton
                id={`season-history-${collection.id}`}
                label={`${seasonHeading(collection, t)} — ${t("history.title")}`}
                onKeyDown={(event) => event.stopPropagation()}
                onClick={(event) => {
                  event.stopPropagation();
                  onOpenSeasonHistory(collection);
                }}
              >
                <History className="h-4 w-4" />
              </IconButton>
            ) : null}
            {onRunSeasonSearch && !actionsInMenu ? (
              <EpisodeTableActionButton
                id={seriesOverviewSeasonSearchId(collection.id)}
                tone="auto"
                aria-label={t("series.searchSeason")}
                showTitleAttribute={false}
                disabled={seasonSearchLoading === true}
                onClick={(event) => {
                  event.stopPropagation();
                  onRunSeasonSearch();
                }}
                label={t("series.searchSeason")}
                tooltip={t("help.seasonSearchTooltip")}
                tooltipSide="left"
                tooltipClassName="w-auto text-left"
              >
                {seasonSearchLoading === true ? (
                  <LoadingMark className="h-4 w-4" />
                ) : (
                  <Zap className="h-4 w-4" />
                )}
              </EpisodeTableActionButton>
            ) : null}
            {onRunSeasonInteractiveSearch &&
            onQueueFromSeasonSearch &&
            !actionsInMenu ? (
              <EpisodeTableActionButton
                id={seriesOverviewSeasonInteractiveSearchId(collection.id)}
                tone="search"
                aria-label={t("series.interactiveSearchSeason")}
                showTitleAttribute={false}
                disabled={seasonInteractiveSearchLoading}
                onClick={(event) => {
                  event.stopPropagation();
                  setSeasonSearchOpen(true);
                  onRunSeasonInteractiveSearch();
                }}
                label={t("series.interactiveSearchSeason")}
                tooltip={t("help.seasonInteractiveSearchTooltip")}
                tooltipSide="left"
                tooltipClassName="w-auto text-left"
              >
                {seasonInteractiveSearchLoading ? (
                  <LoadingMark className="h-4 w-4" />
                ) : (
                  <Search className="h-4 w-4" />
                )}
              </EpisodeTableActionButton>
            ) : null}
          </div>
        </div>
      ) : null}

      {searchBlocked &&
      (onRunSeasonSearch || onRunSeasonInteractiveSearch) &&
      showCollectionHeader ? (
        <div className="border-t border-border bg-card/40 p-4">
          <TitleSearchDownloadClientNotice />
        </div>
      ) : null}

      {seasonSearchOpen && onQueueFromSeasonSearch ? (
        <InteractiveSearchDialog
          id={selectorId("series-overview-season-search-dialog", collection.id)}
          open={seasonSearchOpen}
          onOpenChange={setSeasonSearchOpen}
          title={t("seasonSection.seasonSearchResults", {
            season: seasonHeading(collection, t),
          })}
          description={t("series.interactiveSearchSeason")}
        >
          {searchBlocked ? (
            <TitleSearchDownloadClientNotice />
          ) : (
            <>
              <div className="mb-2 flex items-start gap-4">
                <ReleaseSearchSummary
                  presentation={seasonSearchPresentation}
                  results={seasonSearchResults ?? EMPTY_RELEASES}
                  dataUi="season-release-search-summary"
                />
              </div>
              {seasonSearchPresentation.showInitialLoader ? (
                <ReleaseSearchLoader />
              ) : seasonSearchResults && seasonSearchResults.length > 0 ? (
                <SearchResultBuckets
                  titleId={collection.titleId}
                  ownedScope={{ collectionId: collection.id }}
                  results={seasonSearchResults}
                  onQueue={(release) =>
                    onQueueFromSeasonSearch(collection, release)
                  }
                  requireCandidateToken
                />
              ) : (
                <p className="text-sm text-muted-foreground">
                  {t("nzb.noResultsYet")}
                </p>
              )}
            </>
          )}
        </InteractiveSearchDialog>
      ) : null}

      {showSectionContent ? (
        <>
          {!episodesReady ? (
            <div
              className={cn(
                showCollectionHeader && "border-t border-border",
                "flex items-center gap-2 px-4 py-3 text-sm text-muted-foreground",
              )}
            >
              <LoadingMark className="h-4 w-4" />
              {t("label.loading")}
            </div>
          ) : episodes.length === 0 ? (
            <div
              className={cn(
                showCollectionHeader && "border-t border-border",
                "px-4 py-3 text-sm text-muted-foreground",
              )}
            >
              {t("seasonSection.noEpisodeRecords")}
            </div>
          ) : isMobile ? (
            <div
              className={cn(
                showCollectionHeader && "border-t border-border",
                "p-2",
              )}
            >
              <div className="space-y-2">
                {episodes.map((episode) => (
                  <EpisodeRow
                    key={episode.id}
                    autoSearching={
                      autoSearchLoadingByEpisode[episode.id] === true
                    }
                    collection={collection}
                    clearingReleaseBlocklistEntryId={
                      clearingReleaseBlocklistEntryId
                    }
                    episode={episode}
                    episodeFiles={
                      mediaFilesByEpisode[episode.id] ?? EMPTY_EPISODE_FILES
                    }
                    episodeIndexerProgress={
                      searchIndexerProgressByEpisode[episode.id] ??
                      EMPTY_INDEXER_PROGRESS
                    }
                    episodeResults={
                      searchResultsByEpisode[episode.id] ?? EMPTY_RELEASES
                    }
                    facet={facet}
                    hasSearchResults={Object.prototype.hasOwnProperty.call(
                      searchResultsByEpisode,
                      episode.id,
                    )}
                    initiallyOpen={episode.id === initiallyOpenEpisodeId}
                    isMobile={true}
                    onLoadEpisodeDetail={onLoadEpisodeDetail}
                    onAutoSearchEpisode={onAutoSearchEpisode}
                    onClearReleaseBlocklistEntry={onClearReleaseBlocklistEntry}
                    onDeleteFile={onDeleteFile}
                    onMakePrimaryFile={onMakePrimaryFile}
                    onAssignFileToEpisode={onAssignFileToEpisode}
                    onOpenHistory={onOpenEpisodeHistory}
                    onQueueFromEpisodeSearch={onQueueFromEpisodeSearch}
                    onQueueAdditionalFromEpisodeSearch={
                      onQueueAdditionalFromEpisodeSearch
                    }
                    onRefreshSubtitles={onRefreshSubtitles}
                    onRunEpisodeSearch={onRunEpisodeSearch}
                    onSetEpisodeMonitored={onSetEpisodeMonitored}
                    onToggleSelected={onToggleEpisodeSelected}
                    selected={selectedEpisodeIds?.has(episode.id) ?? false}
                    selectionPending={
                      pendingEpisodeIds?.has(episode.id) ?? false
                    }
                    downloadActive={
                      activeDownloadEpisodeIds?.has(episode.id) ?? false
                    }
                    queueItem={downloadQueueItemByEpisodeId?.[episode.id]}
                    releaseBlocklistEntries={releaseBlocklistEntries}
                    searchBlocked={searchBlockedByEpisode[episode.id] === true}
                    searchLoading={searchLoadingByEpisode[episode.id] === true}
                    subtitleDownloads={stableSubtitleDownloads}
                    libraryId={libraryId}
                    primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
                  />
                ))}
              </div>
            </div>
          ) : (
            <div
              className={cn(showCollectionHeader && "border-t border-border")}
            >
              <Table overflow="clip" layout="fixed" density="dense">
                <TableHeader aria-hidden="true">
                  <TableRow className="collapse">
                    {selectionEnabled ? (
                      <TableHead className="w-10 text-center">
                        {t("seriesOverview.selectColumn")}
                      </TableHead>
                    ) : null}
                    <TableHead className="w-10 text-center" />
                    <TableHead className="w-12 text-center">
                      {t("episode.numberLabel")}
                    </TableHead>
                    <TableHead>{t("label.title")}</TableHead>
                    <TableHead className="w-32 text-center">
                      {t("episode.airDate")}
                    </TableHead>
                    <TableHead className="w-16 text-center">
                      {t("mediaInfo.size")}
                    </TableHead>
                    <TableHead className="w-32 text-center">
                      {t("episode.quality")}
                    </TableHead>
                    <TableActionsHead className="w-28">
                      {t("label.actions")}
                    </TableActionsHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {episodes.map((episode) => (
                    <EpisodeRow
                      key={episode.id}
                      autoSearching={
                        autoSearchLoadingByEpisode[episode.id] === true
                      }
                      collection={collection}
                      clearingReleaseBlocklistEntryId={
                        clearingReleaseBlocklistEntryId
                      }
                      episode={episode}
                      episodeFiles={
                        mediaFilesByEpisode[episode.id] ?? EMPTY_EPISODE_FILES
                      }
                      episodeIndexerProgress={
                        searchIndexerProgressByEpisode[episode.id] ??
                        EMPTY_INDEXER_PROGRESS
                      }
                      episodeResults={
                        searchResultsByEpisode[episode.id] ?? EMPTY_RELEASES
                      }
                      facet={facet}
                      hasSearchResults={Object.prototype.hasOwnProperty.call(
                        searchResultsByEpisode,
                        episode.id,
                      )}
                      initiallyOpen={episode.id === initiallyOpenEpisodeId}
                      isMobile={false}
                      onLoadEpisodeDetail={onLoadEpisodeDetail}
                      onAutoSearchEpisode={onAutoSearchEpisode}
                      onClearReleaseBlocklistEntry={
                        onClearReleaseBlocklistEntry
                      }
                      onDeleteFile={onDeleteFile}
                      onMakePrimaryFile={onMakePrimaryFile}
                      onAssignFileToEpisode={onAssignFileToEpisode}
                      onOpenHistory={onOpenEpisodeHistory}
                      onQueueFromEpisodeSearch={onQueueFromEpisodeSearch}
                      onQueueAdditionalFromEpisodeSearch={
                        onQueueAdditionalFromEpisodeSearch
                      }
                      onRefreshSubtitles={onRefreshSubtitles}
                      onRunEpisodeSearch={onRunEpisodeSearch}
                      onSetEpisodeMonitored={onSetEpisodeMonitored}
                      onToggleSelected={onToggleEpisodeSelected}
                      selected={selectedEpisodeIds?.has(episode.id) ?? false}
                      selectionPending={
                        pendingEpisodeIds?.has(episode.id) ?? false
                      }
                      downloadActive={
                        activeDownloadEpisodeIds?.has(episode.id) ?? false
                      }
                      queueItem={downloadQueueItemByEpisodeId?.[episode.id]}
                      releaseBlocklistEntries={releaseBlocklistEntries}
                      searchBlocked={
                        searchBlockedByEpisode[episode.id] === true
                      }
                      searchLoading={
                        searchLoadingByEpisode[episode.id] === true
                      }
                      subtitleDownloads={stableSubtitleDownloads}
                      libraryId={libraryId}
                      primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
                    />
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </>
      ) : null}
    </div>
  );
}

export const SeasonSection = React.memo(
  SeasonSectionImpl,
  sameSeasonSectionProps,
);
