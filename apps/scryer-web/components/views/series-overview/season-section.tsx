import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  Eye,
  EyeOff,
  Loader2,
  Zap,
} from "lucide-react";
import {
  Table,
  TableActionsHead,
  TableBody,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Checkbox } from "@/components/ui/checkbox";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import { useTranslate } from "@/lib/context/translate-context";
import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import {
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

const EMPTY_INDEXER_PROGRESS: InteractiveSearchIndexerProgress[] = [];

export { SeriesMovieTimelineSection } from "./series-movie-row";

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
  onRefreshSubtitles?: () => Promise<void> | void;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  clearingReleaseBlocklistEntryId?: string | null;
  searchResultsByEpisode: Record<string, Release[]>;
  searchIndexerProgressByEpisode: Record<string, InteractiveSearchIndexerProgress[]>;
  searchLoadingByEpisode: Record<string, boolean>;
  searchBlockedByEpisode: Record<string, boolean>;
  autoSearchLoadingByEpisode: Record<string, boolean>;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onRunEpisodeSearch?: (episode: CollectionEpisode) => void;
  onOpenEpisodeHistory?: (episode: CollectionEpisode) => void;
  onQueueFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onQueueAdditionalFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  seasonSearchResults?: Release[];
  seasonSearchLoading?: boolean;
  searchBlocked?: boolean;
  onRunSeasonSearch?: () => void;
  onQueueFromSeasonSearch?: (collection: TitleCollection, release: Release) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
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
    if (!EPISODE_SCOPED_PROPS.has(key) && !Object.is(previous[key], next[key])) {
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
      previous.mediaFilesByEpisode[episodeId] !== next.mediaFilesByEpisode[episodeId] ||
      previous.searchBlockedByEpisode[episodeId] !== next.searchBlockedByEpisode[episodeId] ||
      previous.searchIndexerProgressByEpisode[episodeId] !==
        next.searchIndexerProgressByEpisode[episodeId] ||
      previous.searchLoadingByEpisode[episodeId] !== next.searchLoadingByEpisode[episodeId] ||
      previous.searchResultsByEpisode[episodeId] !== next.searchResultsByEpisode[episodeId] ||
      (previous.selectedEpisodeIds?.has(episodeId) ?? false) !==
        (next.selectedEpisodeIds?.has(episodeId) ?? false) ||
      (previous.pendingEpisodeIds?.has(episodeId) ?? false) !==
        (next.pendingEpisodeIds?.has(episodeId) ?? false)
    ) {
      return false;
    }

    const previousQueueItem = previous.downloadQueueItemByEpisodeId?.[episodeId];
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
  onRefreshSubtitles,
  searchResultsByEpisode,
  searchIndexerProgressByEpisode,
  searchLoadingByEpisode,
  searchBlockedByEpisode,
  onRunEpisodeSearch,
  onOpenEpisodeHistory,
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
  onQueueFromSeasonSearch,
  onDeleteFile,
  onMakePrimaryFile,
  primaryMovieFileUpdatingId = null,
  selectedEpisodeIds,
  pendingEpisodeIds,
  onToggleEpisodeSelected,
  onSetSeasonSelected,
}: SeasonSectionProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const [seasonToggling, setSeasonToggling] = React.useState(false);
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
    return selectedCount === selectableEpisodeIds.length ? true : "indeterminate";
  }, [selectableEpisodeIds, selectedEpisodeIds]);

  const handleToggleSeasonSelection = React.useCallback(() => {
    if (!onSetSeasonSelected) {
      return;
    }
    onSetSeasonSelected(selectableEpisodeIds, seasonSelectionState !== true);
  }, [onSetSeasonSelected, seasonSelectionState, selectableEpisodeIds]);

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

    const monitoredCount = episodes.filter((episode) => episode.monitored).length;
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
    const uniqueFiles = new Map<string, EpisodeMediaFile>();
    const aggregateTotalEpisodes =
      typeof collection.episodesTotal === "number" && collection.episodesTotal >= 0
        ? collection.episodesTotal
        : null;
    const aggregateMonitoredEpisodes =
      typeof collection.episodesMonitored === "number" &&
      collection.episodesMonitored >= 0
        ? collection.episodesMonitored
        : null;
    const aggregateOwnedEpisodes =
      typeof collection.episodesOwned === "number" && collection.episodesOwned >= 0
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

      const episodeFiles = mediaFilesByEpisode[episode.id] ?? EMPTY_EPISODE_FILES;
      if (episodeFiles.length > 0) {
        ownedEpisodes += 1;
      }

      for (const file of episodeFiles) {
        if (!uniqueFiles.has(file.id)) {
          uniqueFiles.set(file.id, file);
        }
      }
    }

    let matchedSizeBytes = 0;
    for (const file of uniqueFiles.values()) {
      const sizeBytes = file.sizeBytes;
      if (Number.isFinite(sizeBytes) && sizeBytes > 0) {
        matchedSizeBytes += sizeBytes;
      }
    }

    return {
      totalEpisodes: aggregateTotalEpisodes ?? totalEpisodes,
      monitoredEpisodes: aggregateMonitoredEpisodes ?? monitoredEpisodes,
      ownedEpisodes: aggregateOwnedEpisodes ?? ownedEpisodes,
      matchedSizeBytes,
    };
  }, [
    collection.episodesMonitored,
    collection.episodesOwned,
    collection.episodesTotal,
    episodes,
    mediaFilesByEpisode,
  ]);

  const collectionEpisodeProgress = React.useMemo(
    () => {
      if (!collectionMetrics || collectionMetrics.totalEpisodes <= 0) {
        return null;
      }

      return getCollectionEpisodeProgressPresentation({
        ownedEpisodes: collectionMetrics.ownedEpisodes,
        totalEpisodes: collectionMetrics.totalEpisodes,
        monitoredEpisodes: collectionMetrics.monitoredEpisodes,
        t,
      });
    },
    [collectionMetrics, t],
  );

  const collectionSizeLabel = React.useMemo(() => {
    const derivedSizeBytes = collectionMetrics?.matchedSizeBytes ?? 0;
    if (derivedSizeBytes > 0) {
      return formatFileSize(derivedSizeBytes);
    }

    return null;
  }, [collectionMetrics]);

  const isSpecials = isSpecialsCollection(collection);
  const showCollectionHeader = true;
  const showSectionContent = expanded;

  return (
    <div
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
          className="flex w-full cursor-pointer flex-wrap items-center justify-between gap-3 bg-card/60 px-4 py-2 text-left transition hover:bg-accent/80"
        >
          <div className="flex items-center gap-2">
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
                  checked={seasonSelectionState}
                  disabled={!episodesReady || selectableEpisodeIds.length === 0}
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
                onSetCollectionMonitored(collection.id, nextMonitored)
                  .finally(() => setSeasonToggling(false));
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
              <p className="text-sm font-semibold text-foreground">
                {seasonHeading(collection, t)}
              </p>
              {episodeRangeLabel ? <p className="text-xs text-muted-foreground">{episodeRangeLabel}</p> : null}
            </div>
          </div>
          <div className="flex items-center gap-2">
            {!isSpecials && collectionSizeLabel ? (
              <span className="text-xs tabular-nums text-muted-foreground">
                {collectionSizeLabel}
              </span>
            ) : null}
            {!isSpecials && collectionEpisodeProgress ? (
              <EpisodeProgressBar
                progress={collectionEpisodeProgress}
                compact
                className="w-[6.75rem]"
              />
            ) : null}
            {onRunSeasonSearch ? (
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
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Zap className="h-4 w-4" />
                )}
              </EpisodeTableActionButton>
            ) : null}
          </div>
        </div>
      ) : null}

      {searchBlocked && onRunSeasonSearch && showCollectionHeader ? (
        <div className="border-t border-border bg-card/40 p-4">
          <TitleSearchDownloadClientNotice />
        </div>
      ) : null}

      {showSectionContent ? (
        <>
            {seasonSearchResults && seasonSearchResults.length > 0 && onQueueFromSeasonSearch ? (
              <div className={cn(showCollectionHeader && "border-t border-border", "px-4 py-3")}>
                <p className="mb-2 text-xs font-medium text-muted-foreground">{t("seasonSection.seasonPackResults")}</p>
                <SearchResultBuckets
                  results={seasonSearchResults}
                  onQueue={(release) => onQueueFromSeasonSearch(collection, release)}
                  requireCandidateToken
                />
              </div>
            ) : null}
            {!episodesReady ? (
              <div
                className={cn(
                  showCollectionHeader && "border-t border-border",
                  "flex items-center gap-2 px-4 py-3 text-sm text-muted-foreground",
                )}
              >
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("label.loading")}
              </div>
            ) : episodes.length === 0 ? (
              <div className={cn(showCollectionHeader && "border-t border-border", "px-4 py-3 text-sm text-muted-foreground")}>
                {t("seasonSection.noEpisodeRecords")}
              </div>
            ) : isMobile ? (
              <div className={cn(showCollectionHeader && "border-t border-border", "px-3 py-3")}>
                <div className="space-y-3">
                  {episodes.map((episode) => (
                    <EpisodeRow
                      key={episode.id}
                      autoSearching={autoSearchLoadingByEpisode[episode.id] === true}
                      collection={collection}
                      clearingReleaseBlocklistEntryId={clearingReleaseBlocklistEntryId}
                      episode={episode}
                      episodeFiles={mediaFilesByEpisode[episode.id] ?? EMPTY_EPISODE_FILES}
                      episodeIndexerProgress={
                        searchIndexerProgressByEpisode[episode.id] ?? EMPTY_INDEXER_PROGRESS
                      }
                      episodeResults={searchResultsByEpisode[episode.id] ?? EMPTY_RELEASES}
                      facet={facet}
                      hasSearchResults={Object.prototype.hasOwnProperty.call(searchResultsByEpisode, episode.id)}
                      initiallyOpen={episode.id === initiallyOpenEpisodeId}
                      isMobile={true}
                      onLoadEpisodeDetail={onLoadEpisodeDetail}
                      onAutoSearchEpisode={onAutoSearchEpisode}
                      onClearReleaseBlocklistEntry={onClearReleaseBlocklistEntry}
                      onDeleteFile={onDeleteFile}
                      onMakePrimaryFile={onMakePrimaryFile}
                      onOpenHistory={onOpenEpisodeHistory}
                      onQueueFromEpisodeSearch={onQueueFromEpisodeSearch}
                      onQueueAdditionalFromEpisodeSearch={onQueueAdditionalFromEpisodeSearch}
                      onRefreshSubtitles={onRefreshSubtitles}
                      onRunEpisodeSearch={onRunEpisodeSearch}
                      onSetEpisodeMonitored={onSetEpisodeMonitored}
                      onToggleSelected={onToggleEpisodeSelected}
                      selected={selectedEpisodeIds?.has(episode.id) ?? false}
                      selectionPending={pendingEpisodeIds?.has(episode.id) ?? false}
                      downloadActive={activeDownloadEpisodeIds?.has(episode.id) ?? false}
                      queueItem={downloadQueueItemByEpisodeId?.[episode.id]}
                      releaseBlocklistEntries={releaseBlocklistEntries}
                      searchBlocked={searchBlockedByEpisode[episode.id] === true}
                      searchLoading={searchLoadingByEpisode[episode.id] === true}
                      subtitleDownloads={stableSubtitleDownloads}
                      primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
                    />
                  ))}
                </div>
              </div>
            ) : (
              <div className={cn(showCollectionHeader && "border-t border-border")}>
                <Table overflow="clip" layout="fixed" density="dense">
                  <TableHeader aria-hidden="true">
                    <TableRow className="collapse">
                      {selectionEnabled ? (
                        <TableHead className="w-10 text-center">
                          {t("seriesOverview.selectColumn")}
                        </TableHead>
                      ) : null}
                      <TableHead className="w-10 text-center" />
                      <TableHead className="w-12 text-center">{t("episode.numberLabel")}</TableHead>
                      <TableHead>{t("label.title")}</TableHead>
                      <TableHead className="w-44 text-center">{t("episode.airDate")}</TableHead>
                      <TableHead className="w-32 text-center">{t("episode.quality")}</TableHead>
                      <TableActionsHead className="w-28">{t("label.actions")}</TableActionsHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {episodes.map((episode) => (
                      <EpisodeRow
                        key={episode.id}
                        autoSearching={autoSearchLoadingByEpisode[episode.id] === true}
                        collection={collection}
                        clearingReleaseBlocklistEntryId={clearingReleaseBlocklistEntryId}
                        episode={episode}
                        episodeFiles={mediaFilesByEpisode[episode.id] ?? EMPTY_EPISODE_FILES}
                        episodeIndexerProgress={
                          searchIndexerProgressByEpisode[episode.id] ?? EMPTY_INDEXER_PROGRESS
                        }
                        episodeResults={searchResultsByEpisode[episode.id] ?? EMPTY_RELEASES}
                        facet={facet}
                        hasSearchResults={Object.prototype.hasOwnProperty.call(searchResultsByEpisode, episode.id)}
                        initiallyOpen={episode.id === initiallyOpenEpisodeId}
                        isMobile={false}
                        onLoadEpisodeDetail={onLoadEpisodeDetail}
                        onAutoSearchEpisode={onAutoSearchEpisode}
                        onClearReleaseBlocklistEntry={onClearReleaseBlocklistEntry}
                        onDeleteFile={onDeleteFile}
                        onMakePrimaryFile={onMakePrimaryFile}
                        onOpenHistory={onOpenEpisodeHistory}
                        onQueueFromEpisodeSearch={onQueueFromEpisodeSearch}
                        onQueueAdditionalFromEpisodeSearch={onQueueAdditionalFromEpisodeSearch}
                        onRefreshSubtitles={onRefreshSubtitles}
                        onRunEpisodeSearch={onRunEpisodeSearch}
                        onSetEpisodeMonitored={onSetEpisodeMonitored}
                        onToggleSelected={onToggleEpisodeSelected}
                        selected={selectedEpisodeIds?.has(episode.id) ?? false}
                        selectionPending={pendingEpisodeIds?.has(episode.id) ?? false}
                        downloadActive={activeDownloadEpisodeIds?.has(episode.id) ?? false}
                        queueItem={downloadQueueItemByEpisodeId?.[episode.id]}
                        releaseBlocklistEntries={releaseBlocklistEntries}
                        searchBlocked={searchBlockedByEpisode[episode.id] === true}
                        searchLoading={searchLoadingByEpisode[episode.id] === true}
                        subtitleDownloads={stableSubtitleDownloads}
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

export const SeasonSection = React.memo(SeasonSectionImpl, sameSeasonSectionProps);
