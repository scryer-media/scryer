import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  Eye,
  EyeOff,
  Film,
  Loader2,
  Search,
  Zap,
} from "lucide-react";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import { useTranslate } from "@/lib/context/translate-context";
import type { Release } from "@/lib/types";
import { cn } from "@/lib/utils";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import {
  seriesOverviewSeriesMovieAutoSearchId,
  seriesOverviewSeriesMovieInteractiveSearchId,
  seriesOverviewSeriesMovieRowId,
} from "@/lib/utils/dom-ids";
import type {
  EpisodeMediaFile,
  SeriesMovieLink,
} from "@/components/containers/series-overview-container";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import { formatFileSize } from "./helpers";
import { MediaFilesOnDiskPanel } from "@/components/common/media-files-on-disk-panel";
import { TitleFilesOnDiskRail } from "@/components/common/title-files-on-disk-rail";
import { SeriesMoviePanel } from "./series-movie-panel";
import {
  EpisodeTableActionButton,
  EMPTY_EPISODE_FILES,
} from "./season-section-utils";

export type SeriesMovieTimelineContentProps = {
  link: SeriesMovieLink;
  expanded: boolean;
  onToggle: () => void;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  mediaFilesBySeriesMovieLink: Record<string, EpisodeMediaFile[]>;
  subtitleDownloads?: ExternalSubtitleRecord[];
  onRefreshSubtitles?: () => Promise<void> | void;
  onLoadSeriesMovieDetail?: (link: SeriesMovieLink) => Promise<void> | void;
  seriesMovieSearchResultsByLink: Record<string, Release[]>;
  seriesMovieSearchLoadingByLink: Record<string, boolean>;
  seriesMovieSearchAttemptedByLink: Record<string, boolean>;
  searchBlockedBySeriesMovie: Record<string, boolean>;
  onRunSeriesMovieSearch?: (link: SeriesMovieLink) => void;
  onQueueFromSeriesMovieSearch?: (link: SeriesMovieLink, release: Release) => Promise<void> | void;
  onQueueAdditionalFromSeriesMovieSearch?: (link: SeriesMovieLink, release: Release) => Promise<void> | void;
  onAutoSearchSeriesMovie?: (link: SeriesMovieLink) => void;
  onSetSeriesMovieMonitored?: (seriesMovieLinkId: string, monitored: boolean) => Promise<void>;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
  primaryMovieFileUpdatingId?: string | null;
  autoSearchSeriesMovieLoadingByLink: Record<string, boolean>;
};

function SeriesMovieTimelineContent({
  link,
  mediaFilesByEpisode,
  mediaFilesBySeriesMovieLink,
  subtitleDownloads,
  onRefreshSubtitles,
  seriesMovieSearchResultsByLink,
  seriesMovieSearchLoadingByLink,
  seriesMovieSearchAttemptedByLink,
  searchBlockedBySeriesMovie,
  onQueueFromSeriesMovieSearch,
  onQueueAdditionalFromSeriesMovieSearch,
  onDeleteFile,
  onMakePrimaryFile,
  primaryMovieFileUpdatingId = null,
}: SeriesMovieTimelineContentProps) {
  const t = useTranslate();
  const searchBlockedForMovie = searchBlockedBySeriesMovie[link.id] === true;
  const searchLoading = seriesMovieSearchLoadingByLink[link.id] === true;
  const searchAttempted = seriesMovieSearchAttemptedByLink[link.id] === true;
  const searchResults = seriesMovieSearchResultsByLink[link.id];
  const mediaFiles = getSeriesMovieMediaFiles(
    link,
    mediaFilesByEpisode,
    mediaFilesBySeriesMovieLink,
  );

  return (
    <div className="space-y-3">
      <SeriesMoviePanel
        link={link}
        hasFile={mediaFiles.length > 0}
        filesOnDisk={
          <TitleFilesOnDiskRail>
            <MediaFilesOnDiskPanel
              emptyMessage={t("title.noFilesTracked")}
              mediaFiles={mediaFiles}
              subtitleDownloads={subtitleDownloads}
              onRefreshSubtitles={onRefreshSubtitles}
              onDeleteFile={onDeleteFile}
              onMakePrimaryFile={onMakePrimaryFile}
              primaryFileUpdatingId={primaryMovieFileUpdatingId}
              showPrimaryRoleBadge
              fileRowIdPrefix="series-overview-series-movie-file"
              subtitleSearchIdPrefix="series-overview-series-movie-search-subtitles"
              deleteFileIdPrefix="series-overview-series-movie-delete-file"
              makePrimaryFileIdPrefix="series-overview-series-movie-make-primary-file"
            />
          </TitleFilesOnDiskRail>
        }
      />
      {searchBlockedForMovie ? <TitleSearchDownloadClientNotice /> : null}
      {!searchBlockedForMovie && searchLoading ? (
        <div className="flex flex-col items-center justify-center gap-4 py-16">
          <Loader2 className="h-10 w-10 animate-spin text-[var(--scry-accent-text)]" />
          <p className="text-lg text-muted-foreground">{t("label.searching")}</p>
        </div>
      ) : null}
      {!searchBlockedForMovie
      && !searchLoading
      && searchResults
      && searchResults.length > 0
      && onQueueFromSeriesMovieSearch ? (
        <div className="space-y-2">
          <p className="text-xs font-medium text-muted-foreground">
            {t("title.searchReleasesAction")}
          </p>
          <SearchResultBuckets
            results={searchResults}
            onQueue={(release) => onQueueFromSeriesMovieSearch(link, release)}
            onQueueAdditional={onQueueAdditionalFromSeriesMovieSearch
              ? (release) => onQueueAdditionalFromSeriesMovieSearch(link, release)
              : undefined}
            canQueueAdditional={(release) =>
              releaseSupportsAdditionalFileQueue(release, null)
            }
            requireCandidateToken
          />
        </div>
      ) : null}
      {!searchBlockedForMovie
      && !searchLoading
      && searchAttempted
      && (!searchResults || searchResults.length === 0) ? (
        <p className="text-xs text-muted-foreground">
          {t("title.noReleasesFound", { name: link.movie.title })}
        </p>
      ) : null}
    </div>
  );
}

function mergeSeriesMovieMediaFiles(
  seriesMovieFiles: EpisodeMediaFile[],
  linkedEpisodeFiles: EpisodeMediaFile[],
) {
  const byId = new Map<string, EpisodeMediaFile>();
  for (const file of [...seriesMovieFiles, ...linkedEpisodeFiles]) {
    if (!byId.has(file.id)) {
      byId.set(file.id, file);
    }
  }
  return Array.from(byId.values());
}

function getSeriesMovieMediaFiles(
  link: SeriesMovieLink,
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>,
  mediaFilesBySeriesMovieLink: Record<string, EpisodeMediaFile[]>,
) {
  const linkedEpisodeFiles = link.linkedEpisodeId
    ? mediaFilesByEpisode[link.linkedEpisodeId] ?? EMPTY_EPISODE_FILES
    : EMPTY_EPISODE_FILES;
  const seriesMovieFiles =
    mediaFilesBySeriesMovieLink[link.id] ?? EMPTY_EPISODE_FILES;
  return mergeSeriesMovieMediaFiles(seriesMovieFiles, linkedEpisodeFiles);
}

function getMediaFilesSizeLabel(mediaFiles: EpisodeMediaFile[]) {
  let matchedSizeBytes = 0;
  for (const file of mediaFiles) {
    const sizeBytes = file.sizeBytes;
    if (Number.isFinite(sizeBytes) && sizeBytes > 0) {
      matchedSizeBytes += sizeBytes;
    }
  }

  return matchedSizeBytes > 0 ? formatFileSize(matchedSizeBytes) : null;
}

export function SeriesMovieTimelineSection(props: SeriesMovieTimelineContentProps) {
  const {
    expanded,
    link,
    mediaFilesByEpisode,
    mediaFilesBySeriesMovieLink,
    onToggle,
    onLoadSeriesMovieDetail,
    onAutoSearchSeriesMovie,
    onRunSeriesMovieSearch,
    onSetSeriesMovieMonitored,
  } = props;
  const t = useTranslate();
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const [seriesMovieToggling, setSeriesMovieToggling] = React.useState(false);
  const mediaFiles = getSeriesMovieMediaFiles(
    link,
    mediaFilesByEpisode,
    mediaFilesBySeriesMovieLink,
  );
  const sizeLabel = getMediaFilesSizeLabel(mediaFiles);
  const autoSearchLoading = props.autoSearchSeriesMovieLoadingByLink[link.id] === true;
  const searchLoading = props.seriesMovieSearchLoadingByLink[link.id] === true;

  const loadSeriesMovieDetail = React.useCallback(() => {
    void onLoadSeriesMovieDetail?.(link);
  }, [link, onLoadSeriesMovieDetail]);

  React.useEffect(() => {
    if (expanded) {
      loadSeriesMovieDetail();
    }
  }, [expanded, loadSeriesMovieDetail]);

  const handleToggleSeriesMovieMonitored = React.useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      if (!onSetSeriesMovieMonitored) {
        return;
      }

      setSeriesMovieToggling(true);
      onSetSeriesMovieMonitored(link.id, !link.monitored)
        .finally(() => setSeriesMovieToggling(false));
    },
    [link.id, link.monitored, onSetSeriesMovieMonitored],
  );

  const handleAutoSearch = React.useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      onAutoSearchSeriesMovie?.(link);
    },
    [link, onAutoSearchSeriesMovie],
  );

  const handleInteractiveSearch = React.useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      if (!expanded) {
        onToggle();
      }
      onRunSeriesMovieSearch?.(link);
    },
    [expanded, link, onRunSeriesMovieSearch, onToggle],
  );

  return (
    <div
      id={seriesOverviewSeriesMovieRowId(link.id)}
      data-timeline-kind="series-movie"
      data-series-movie-link-id={link.id}
      data-expanded={expanded ? "true" : "false"}
      className="overflow-hidden rounded-lg border border-border bg-background/40"
    >
      <div
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
        <div className="flex min-w-0 items-center gap-2">
          <button
            type="button"
            disabled={!onSetSeriesMovieMonitored || seriesMovieToggling}
            aria-label={t("title.seriesMovieMonitored")}
            className={cn(
              "inline-flex size-6 shrink-0 items-center justify-center rounded transition-colors",
              seriesMovieToggling && "opacity-50",
              link.monitored
                ? "text-[var(--scry-success-text-soft)]"
                : "text-muted-foreground/60",
            )}
            onClick={handleToggleSeriesMovieMonitored}
          >
            {link.monitored ? (
              <Eye className="size-5" />
            ) : (
              <EyeOff className="size-5" />
            )}
          </button>
          <Chevron className="h-4 w-4 shrink-0 text-muted-foreground" />
          <Film className="h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-foreground">
              {link.movie.title}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {sizeLabel ? (
            <span className="text-xs tabular-nums text-muted-foreground">
              {sizeLabel}
            </span>
          ) : null}
          <div className="flex items-center justify-end gap-2">
            {onAutoSearchSeriesMovie ? (
              <EpisodeTableActionButton
                id={seriesOverviewSeriesMovieAutoSearchId(link.id)}
                tone="auto"
                onClick={handleAutoSearch}
                disabled={autoSearchLoading}
                label={t("title.queueLatest")}
                tooltip={t("help.autoSearchTooltip")}
                showTitleAttribute={false}
              >
                {autoSearchLoading ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Zap className="h-4 w-4" />
                )}
              </EpisodeTableActionButton>
            ) : null}
            {onRunSeriesMovieSearch ? (
              <EpisodeTableActionButton
                id={seriesOverviewSeriesMovieInteractiveSearchId(link.id)}
                tone="search"
                onClick={handleInteractiveSearch}
                disabled={searchLoading}
                label={t("title.searchReleasesAction")}
                tooltip={t("help.interactiveSearchTooltip")}
                showTitleAttribute={false}
              >
                <Search className="h-4 w-4" />
              </EpisodeTableActionButton>
            ) : null}
          </div>
        </div>
      </div>
      {expanded ? (
        <div className="border-t border-border px-4 py-3">
          <SeriesMovieTimelineContent {...props} />
        </div>
      ) : null}
    </div>
  );
}
