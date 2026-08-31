import * as React from "react";
import {
  CalendarDays,
  Clock3,
  Eye,
  EyeOff,
  Loader2,
  Search,
  Zap,
} from "lucide-react";
import { TextActionButton } from "@/components/ui/text-action-button";
import { Badge } from "@/components/ui/badge";
import {
  TableActionsCell,
  TableCell,
  TableCodeCell,
  TableRow,
} from "@/components/ui/table";
import { EpisodeQueueIndicator } from "@/components/common/download-queue-overview";
import { TitleDownloadActivityPill } from "@/components/common/title-download-activity";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  selectorId,
  seriesOverviewEpisodeAutoSearchId,
  seriesOverviewEpisodeInteractiveSearchId,
  seriesOverviewEpisodeRowId,
} from "@/lib/utils/dom-ids";
import { episodeAvailabilityPill } from "@/lib/utils/episode-media-availability";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { EpisodePanelTab } from "./episode-panel-reducer";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import {
  formatDate,
  formatRuntimeFromSeconds,
} from "./helpers";
import { EpisodePanelContent } from "./episode-panel-content";
import {
  EpisodeTableActionButton,
  type TranslateFn,
} from "./season-section-utils";

function renderEpisodeTypeBadges(episode: CollectionEpisode, t: TranslateFn) {
  return (
    <>
      {episode.episodeType === "SPECIAL" ? (
        <Badge tone="info" className="px-1.5 text-[10px]">
          {t("episode.special")}
        </Badge>
      ) : episode.episodeType === "OVA" ? (
        <Badge tone="info" className="px-1.5 text-[10px]">
          {t("episode.ova")}
        </Badge>
      ) : episode.episodeType === "ONA" ? (
        <Badge tone="positive" className="px-1.5 text-[10px]">
          {t("episode.ona")}
        </Badge>
      ) : episode.episodeType === "ALTERNATE" ? (
        <Badge tone="info" className="px-1.5 text-[10px]">
          {t("episode.alternate")}
        </Badge>
      ) : null}
      {episode.isFiller ? (
        <Badge tone="warning" className="px-1.5 text-[10px]">
          {t("episode.filler")}
        </Badge>
      ) : null}
      {episode.isRecap ? (
        <Badge tone="warning" className="px-1.5 text-[10px]">
          {t("episode.recap")}
        </Badge>
      ) : null}
    </>
  );
}

function renderEpisodeQualityBadge(episode: CollectionEpisode, t: TranslateFn) {
  const pill = episodeAvailabilityPill(episode.mediaAvailability, t);
  return pill ? (
    <Badge
      id={selectorId("series-overview-episode-availability", episode.id)}
      tone={pill.tone}
      className="px-1.5 text-[10px]"
    >
      {pill.label}
    </Badge>
  ) : null;
}

function renderEpisodeQualityCell(
  episode: CollectionEpisode,
  queueItem: DownloadQueueItem | undefined,
  t: TranslateFn,
) {
  const qualityBadge = renderEpisodeQualityBadge(episode, t);

  if (!qualityBadge && !queueItem) {
    return null;
  }

  return (
    <div className="flex flex-col items-center gap-1">
      {qualityBadge}
      {queueItem ? <EpisodeQueueIndicator item={queueItem} /> : null}
    </div>
  );
}

export type EpisodeRowProps = {
  autoSearching: boolean;
  collection: TitleCollection;
  clearingReleaseBlocklistEntryId?: string | null;
  downloadActive: boolean;
  episode: CollectionEpisode;
  episodeFiles: EpisodeMediaFile[];
  episodeIndexerProgress: InteractiveSearchIndexerProgress[];
  episodeResults: Release[];
  facet: string;
  hasSearchResults: boolean;
  initiallyOpen: boolean;
  isMobile: boolean;
  onLoadEpisodeDetail?: (episodeId: string) => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => void;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
  onOpenHistory?: (episode: CollectionEpisode) => void;
  onQueueFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onQueueAdditionalFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onRefreshSubtitles?: () => Promise<void> | void;
  onRunEpisodeSearch?: (episode: CollectionEpisode) => void;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  queueItem?: DownloadQueueItem;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  searchBlocked: boolean;
  searchLoading: boolean;
  subtitleDownloads: ExternalSubtitleRecord[];
  primaryMovieFileUpdatingId?: string | null;
};

export const EpisodeRow = React.memo(function EpisodeRow({
  autoSearching,
  collection,
  clearingReleaseBlocklistEntryId,
  downloadActive,
  episode,
  episodeFiles,
  episodeIndexerProgress,
  episodeResults,
  facet,
  hasSearchResults,
  initiallyOpen,
  isMobile,
  onLoadEpisodeDetail,
  onAutoSearchEpisode,
  onClearReleaseBlocklistEntry,
  onDeleteFile,
  onMakePrimaryFile,
  onOpenHistory,
  onQueueFromEpisodeSearch,
  onQueueAdditionalFromEpisodeSearch,
  onRefreshSubtitles,
  onRunEpisodeSearch,
  onSetEpisodeMonitored,
  queueItem,
  releaseBlocklistEntries,
  searchBlocked,
  searchLoading,
  subtitleDownloads,
  primaryMovieFileUpdatingId = null,
}: EpisodeRowProps) {
  const t = useTranslate();
  const [isPanelOpen, setIsPanelOpen] = React.useState(initiallyOpen);
  const [activeTab, setActiveTab] = React.useState<EpisodePanelTab>("details");
  const [episodeToggling, setEpisodeToggling] = React.useState(false);
  const initializedDeepLinkEpisodeIdRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (!initiallyOpen) {
      initializedDeepLinkEpisodeIdRef.current = null;
      return;
    }

    if (initializedDeepLinkEpisodeIdRef.current === episode.id) {
      return;
    }

    initializedDeepLinkEpisodeIdRef.current = episode.id;
    setIsPanelOpen(true);
    setActiveTab("details");
    void onLoadEpisodeDetail?.(episode.id);
  }, [episode.id, initiallyOpen, onLoadEpisodeDetail]);

  const dateTimeFormat = useUiDateTimeFormat();
  const formattedAirDate = React.useMemo(
    () => formatDate(episode.airDate, dateTimeFormat),
    [dateTimeFormat, episode.airDate],
  );
  const episodeRuntime = React.useMemo(
    () => formatRuntimeFromSeconds(episode.durationSeconds),
    [episode.durationSeconds],
  );

  const openPanelTab = React.useCallback(
    (tab: EpisodePanelTab) => {
      setActiveTab(tab);
      setIsPanelOpen(true);
      void onLoadEpisodeDetail?.(episode.id);
      if (tab === "search" && onRunEpisodeSearch && (searchBlocked || !hasSearchResults)) {
        onRunEpisodeSearch(episode);
      }
    },
    [episode, hasSearchResults, onLoadEpisodeDetail, onRunEpisodeSearch, searchBlocked],
  );

  const handleToggleEpisodeDetails = React.useCallback(() => {
    if (isPanelOpen && activeTab === "details") {
      setIsPanelOpen(false);
      return;
    }

    openPanelTab("details");
  }, [activeTab, isPanelOpen, openPanelTab]);

  const handleToggleEpisodeSearch = React.useCallback(() => {
    if (!onRunEpisodeSearch || !onQueueFromEpisodeSearch) {
      return;
    }
    if (isPanelOpen && activeTab === "search") {
      setIsPanelOpen(false);
      return;
    }

    openPanelTab("search");
  }, [activeTab, isPanelOpen, onQueueFromEpisodeSearch, onRunEpisodeSearch, openPanelTab]);

  const handleEpisodeTabChange = React.useCallback(
    (tab: EpisodePanelTab | "history") => {
      if (tab === "history") {
        if (!onOpenHistory) {
          return;
        }
        setIsPanelOpen(true);
        onOpenHistory(episode);
        return;
      }

      openPanelTab(tab);
    },
    [episode, onOpenHistory, openPanelTab],
  );

  const handleAutoSearchClick = React.useCallback(() => {
    onAutoSearchEpisode?.(episode);
  }, [episode, onAutoSearchEpisode]);

  const handleToggleEpisodeMonitored = React.useCallback(() => {
    if (!onSetEpisodeMonitored) {
      return;
    }

    setEpisodeToggling(true);
    onSetEpisodeMonitored(episode.id, !episode.monitored)
      .finally(() => setEpisodeToggling(false));
  }, [episode.id, episode.monitored, onSetEpisodeMonitored]);

  const panelContent = isPanelOpen ? (
    <EpisodePanelContent
      activeTab={activeTab}
      canClearBlocklistEntries={Boolean(onClearReleaseBlocklistEntry)}
      collection={collection}
      clearingReleaseBlocklistEntryId={clearingReleaseBlocklistEntryId}
      episode={episode}
      episodeFiles={episodeFiles}
      episodeIndexerProgress={episodeIndexerProgress}
      episodeLoading={searchLoading}
      episodeResults={episodeResults}
      facet={facet}
      hasSearchResults={hasSearchResults}
      onClearReleaseBlocklistEntry={onClearReleaseBlocklistEntry}
      onDeleteFile={onDeleteFile}
      onMakePrimaryFile={onMakePrimaryFile}
      onQueueFromEpisodeSearch={onQueueFromEpisodeSearch}
      onQueueAdditionalFromEpisodeSearch={onQueueAdditionalFromEpisodeSearch}
      onRefreshSubtitles={onRefreshSubtitles}
      onRunEpisodeSearch={onRunEpisodeSearch}
      onTabChange={handleEpisodeTabChange}
      showHistoryTab={Boolean(onOpenHistory)}
      showSearchTab={Boolean(onRunEpisodeSearch && onQueueFromEpisodeSearch)}
      releaseBlocklistEntries={releaseBlocklistEntries}
      searchBlocked={searchBlocked}
      subtitleDownloads={subtitleDownloads}
      primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
    />
  ) : null;

  const episodeTypeBadges = renderEpisodeTypeBadges(episode, t);
  const qualityCell = renderEpisodeQualityCell(episode, queueItem, t);

  if (isMobile) {
    return (
      <div
        id={selectorId("series-overview-episode", episode.id)}
        data-episode-id={episode.id}
        className={cn(
          "rounded-lg border border-border bg-card/50 p-3",
          !episode.monitored && "opacity-60",
        )}
      >
        <div className="flex items-start gap-3">
          <button
            id={selectorId("series-overview-episode-monitor", episode.id)}
            type="button"
            disabled={!onSetEpisodeMonitored || episodeToggling}
            aria-label={t("title.episodeMonitored")}
            className={cn(
              "mt-0.5 inline-flex size-6 shrink-0 items-center justify-center rounded transition-colors",
              episodeToggling && "opacity-50",
              episode.monitored
                ? "text-[var(--scry-success-text-soft)]"
                : "text-muted-foreground/60",
            )}
            onClick={handleToggleEpisodeMonitored}
          >
            {episode.monitored ? (
              <Eye className="size-5" />
            ) : (
              <EyeOff className="size-5" />
            )}
          </button>
          <div className="min-w-0 flex-1">
            <div className="flex items-start justify-between gap-3">
              <button
                id={selectorId("series-overview-episode-details-toggle", episode.id)}
                type="button"
                className="min-w-0 flex-1 text-left"
                onClick={handleToggleEpisodeDetails}
              >
                <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                  <span className="rounded bg-accent px-2 py-0.5 font-[var(--font-code)] text-card-foreground">
                    {episode.episodeNumber ?? episode.episodeLabel ?? "—"}
                  </span>
                  {episode.absoluteNumber && facet === "anime" ? (
                    <span>#{episode.absoluteNumber}</span>
                  ) : null}
                </div>
                <p className="mt-1 text-sm font-medium text-card-foreground">
                  {episode.title || episode.episodeLabel || "—"}
                </p>
              </button>
              <div className="flex shrink-0 items-center gap-1">
                {qualityCell}
              </div>
            </div>
            <div className="mt-2 flex flex-wrap gap-2">
              {episodeTypeBadges}
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
              <span className="inline-flex items-center gap-1">
                <CalendarDays className="h-3.5 w-3.5" />
                {formattedAirDate}
              </span>
              {episodeRuntime ? (
                <span className="inline-flex items-center gap-1">
                  <Clock3 className="h-3 w-3" />
                  {episodeRuntime}
                </span>
              ) : null}
            </div>
            <div className="mt-3 flex flex-col gap-2">
              {onAutoSearchEpisode ? (
                <TextActionButton
                  id={selectorId("series-overview-episode-auto-search", episode.id)}
                  type="button"
                  size="sm"
                  tone="auto"
                  className="w-full"
                  onClick={handleAutoSearchClick}
                  disabled={autoSearching}
                  leadingIcon={
                    autoSearching ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <Zap className="h-4 w-4" />
                    )
                  }
                >
                  <span>{t("label.search")}</span>
                </TextActionButton>
              ) : null}
              {onRunEpisodeSearch && onQueueFromEpisodeSearch ? (
                <TextActionButton
                  id={selectorId("series-overview-episode-interactive-search", episode.id)}
                  type="button"
                  size="sm"
                  tone="search"
                  className="w-full"
                  onClick={handleToggleEpisodeSearch}
                  leadingIcon={<Search className="h-4 w-4" />}
                >
                  <span>{t("label.interactiveSearch")}</span>
                </TextActionButton>
              ) : null}
            </div>
            {isPanelOpen ? (
              <div className="mt-3 border-t border-border pt-3">
                {panelContent}
              </div>
            ) : null}
          </div>
        </div>
      </div>
    );
  }

  return (
    <React.Fragment>
      <TableRow
        id={seriesOverviewEpisodeRowId(
          facet,
          episode.seasonNumber,
          episode.episodeNumber,
          episode.absoluteNumber,
        )}
        data-episode-id={episode.id}
        className={`cv-auto-row-sm${episode.monitored ? "" : " opacity-50"}`}
      >
        <TableCell className="px-2 text-center align-middle">
          <div className="flex items-center justify-center">
            <button
              id={selectorId("series-overview-episode-monitor", episode.id)}
              type="button"
              disabled={episodeToggling}
              aria-label={t("title.episodeMonitored")}
              className={cn(
                "inline-flex size-6 translate-x-2 items-center justify-center rounded transition-colors",
                episodeToggling && "opacity-50",
                episode.monitored
                  ? "text-[var(--scry-success-text-soft)]"
                  : "text-muted-foreground/60",
              )}
              onClick={handleToggleEpisodeMonitored}
            >
              {episode.monitored ? (
                <Eye className="size-5" />
              ) : (
                <EyeOff className="size-5" />
              )}
            </button>
          </div>
        </TableCell>
        <TableCodeCell className="text-center align-middle text-sm text-card-foreground">
          <div className="flex flex-col items-center gap-0.5">
            <span>{episode.episodeNumber ?? episode.episodeLabel ?? "—"}</span>
            {episode.absoluteNumber && facet === "anime" ? (
              <span className="text-[10px] text-muted-foreground">
                #{episode.absoluteNumber}
              </span>
            ) : null}
          </div>
        </TableCodeCell>
        <TableCell
          id={selectorId("series-overview-episode-details-toggle", episode.id)}
          className="cursor-pointer align-middle text-sm text-card-foreground hover:text-foreground"
          onClick={handleToggleEpisodeDetails}
        >
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="min-w-0 break-words">
              {episode.title || episode.episodeLabel || "—"}
            </span>
            {downloadActive ? <TitleDownloadActivityPill /> : null}
            {episodeTypeBadges}
          </div>
          {episodeRuntime ? (
            <span className="inline-flex items-center gap-1 text-[10px] text-muted-foreground">
              <Clock3 className="h-3 w-3" />
              {episodeRuntime}
            </span>
          ) : null}
        </TableCell>
        <TableCell className="text-center text-muted-foreground">
          <span className="inline-flex items-center justify-center gap-1">
            <CalendarDays className="h-3.5 w-3.5" />
            {formattedAirDate}
          </span>
        </TableCell>
        <TableCell className="text-center">
          <div className="inline-flex items-center justify-center gap-1">
            {qualityCell}
          </div>
        </TableCell>
        <TableActionsCell className="w-28">
          <div className="inline-flex items-center justify-center gap-2">
            {onAutoSearchEpisode ? (
              <EpisodeTableActionButton
                id={seriesOverviewEpisodeAutoSearchId(
                  facet,
                  episode.seasonNumber,
                  episode.episodeNumber,
                  episode.absoluteNumber,
                )}
                tone="auto"
                onClick={handleAutoSearchClick}
                disabled={autoSearching}
                label={t("label.search")}
                tooltip={t("help.autoSearchTooltip")}
                showTitleAttribute={false}
              >
                {autoSearching ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Zap className="h-4 w-4" />
                )}
              </EpisodeTableActionButton>
            ) : null}
            {onRunEpisodeSearch && onQueueFromEpisodeSearch ? (
              <EpisodeTableActionButton
                id={seriesOverviewEpisodeInteractiveSearchId(
                  facet,
                  episode.seasonNumber,
                  episode.episodeNumber,
                  episode.absoluteNumber,
                )}
                tone="search"
                onClick={handleToggleEpisodeSearch}
                label={t("label.search")}
                tooltip={t("help.interactiveSearchTooltip")}
                showTitleAttribute={false}
              >
                <Search className="h-4 w-4" />
              </EpisodeTableActionButton>
            ) : null}
          </div>
        </TableActionsCell>
      </TableRow>
      {isPanelOpen ? (
        <TableRow id={selectorId("series-overview-episode-panel", episode.id)}>
          <TableCell colSpan={6} className="border-t border-border bg-background/40 p-0">
            <div className="px-4 py-3">
              {panelContent}
            </div>
          </TableCell>
        </TableRow>
      ) : null}
    </React.Fragment>
  );
});
