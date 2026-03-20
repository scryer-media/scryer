import * as React from "react";
import {
  CalendarDays,
  ChevronDown,
  ChevronRight,
  Clock3,
  Eye,
  EyeOff,
  Film,
  Loader2,
  Search,
  Star,
  Zap,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { useTranslate } from "@/lib/context/translate-context";
import type { Release } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { EpisodePanelTab } from "./episode-panel-reducer";
import {
  isSpecialsCollection,
  seasonHeading,
  formatDate,
  formatRuntimeFromSeconds,
  blocklistEntryMatchesEpisode,
} from "./helpers";
import { EpisodeDetailsPanel } from "./episode-details-panel";
import { InterstitialMoviePanel } from "./interstitial-movie-panel";
import { EpisodeBlocklistPanel } from "./episode-blocklist-panel";

function EpisodeTableActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "auto" | "search">;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={label}
      aria-label={label}
      className={cn(
        boxedActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
}

export function SeasonSection({
  collection,
  episodes,
  expanded,
  facet,
  onToggle,
  expandedEpisodeRows,
  episodeActiveTab,
  mediaFilesByEpisode,
  releaseBlocklistEntries,
  searchResultsByEpisode,
  searchLoadingByEpisode,
  onToggleEpisodeSearch,
  onToggleEpisodeDetails,
  onEpisodeTabChange,
  onRunEpisodeSearch,
  onQueueFromEpisodeSearch,
  autoSearchLoadingByEpisode,
  onAutoSearchEpisode,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  seasonSearchResults,
  seasonSearchLoading,
  onRunSeasonSearch,
  onQueueFromSeasonSearch,
  onDeleteFile,
  onAutoSearchInterstitialMovie,
  autoSearchInterstitialMovieLoading,
}: {
  collection: TitleCollection;
  facet: string;
  episodes: CollectionEpisode[];
  expanded: boolean;
  onToggle: () => void;
  expandedEpisodeRows: Set<string>;
  episodeActiveTab: Record<string, EpisodePanelTab>;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  subtitleDownloads?: { id: string; mediaFileId: string; language: string; provider: string; hearingImpaired: boolean; forced: boolean }[];
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  searchResultsByEpisode: Record<string, Release[]>;
  searchLoadingByEpisode: Record<string, boolean>;
  autoSearchLoadingByEpisode: Record<string, boolean>;
  onToggleEpisodeSearch: (episode: CollectionEpisode) => void;
  onToggleEpisodeDetails: (episode: CollectionEpisode) => void;
  onEpisodeTabChange: (episodeId: string, tab: EpisodePanelTab, episode: CollectionEpisode) => void;
  onRunEpisodeSearch: (episode: CollectionEpisode) => void;
  onQueueFromEpisodeSearch: (release: Release) => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  seasonSearchResults?: Release[];
  seasonSearchLoading?: boolean;
  onRunSeasonSearch?: () => void;
  onQueueFromSeasonSearch?: (release: Release) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onAutoSearchInterstitialMovie?: (collection: TitleCollection) => void;
  autoSearchInterstitialMovieLoading?: boolean;
}) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const [seasonToggling, setSeasonToggling] = React.useState(false);
  const [episodeToggling, setEpisodeToggling] = React.useState<Set<string>>(new Set());

  const seasonCheckedState: boolean | "indeterminate" = React.useMemo(() => {
    if (episodes.length === 0) return collection.monitored;
    const monitoredCount = episodes.filter((ep) => ep.monitored).length;
    if (monitoredCount === 0) return false;
    if (monitoredCount === episodes.length) return true;
    return "indeterminate";
  }, [episodes, collection.monitored]);

  const renderEpisodeTypeBadges = React.useCallback((episode: CollectionEpisode) => (
    <>
      {episode.episodeType === "special" ? (
        <span className="rounded border border-indigo-500/30 bg-indigo-500/15 px-1.5 py-0.5 text-[10px] font-medium text-indigo-700 dark:text-indigo-300">
          {t("episode.special")}
        </span>
      ) : episode.episodeType === "ova" ? (
        <span className="rounded border border-violet-500/30 bg-violet-500/15 px-1.5 py-0.5 text-[10px] font-medium text-violet-700 dark:text-violet-300">
          {t("episode.ova")}
        </span>
      ) : episode.episodeType === "ona" ? (
        <span className="rounded border border-emerald-500/30 bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
          {t("episode.ona")}
        </span>
      ) : episode.episodeType === "alternate" ? (
        <span className="rounded border border-sky-500/30 bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-medium text-sky-700 dark:text-sky-300">
          {t("episode.alternate")}
        </span>
      ) : null}
      {episode.isFiller ? (
        <span className="rounded border border-orange-500/30 bg-orange-500/15 px-1.5 py-0.5 text-[10px] font-medium text-orange-700 dark:text-orange-300">
          {t("episode.filler")}
        </span>
      ) : null}
      {episode.isRecap ? (
        <span className="rounded border border-amber-500/30 bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-300">
          {t("episode.recap")}
        </span>
      ) : null}
      {episode.hasMultiAudio ? (
        <span className="rounded border border-purple-500/30 bg-purple-500/15 px-1.5 py-0.5 text-[10px] font-medium text-purple-700 dark:text-purple-300">
          {t("episode.multiAudio")}
        </span>
      ) : null}
    </>
  ), [t]);

  const renderEpisodeQualityBadge = React.useCallback(
    (episode: CollectionEpisode, episodeFiles: EpisodeMediaFile[]) => {
      if (episodeFiles.length > 0 && episodeFiles[0].qualityLabel) {
        return (
          <span className="rounded border border-emerald-500/40 bg-emerald-500/20 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:border-emerald-500/30 dark:bg-emerald-500/15 dark:text-emerald-300">
            {episodeFiles[0].qualityLabel}
          </span>
        );
      }

      if (episode.monitored) {
        return (
          <span className="rounded border border-amber-500/30 bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-300">
            {t("episode.missing")}
          </span>
        );
      }

      return null;
    },
    [t],
  );

  const renderEpisodePanel = React.useCallback(
    (
      episode: CollectionEpisode,
      activeTab: EpisodePanelTab,
      episodeResults: Release[],
      episodeLoading: boolean,
      episodeFiles: EpisodeMediaFile[],
    ) => (
      <Tabs
        value={activeTab}
        onValueChange={(val) => onEpisodeTabChange(episode.id, val as EpisodePanelTab, episode)}
      >
        <TabsList className="flex w-full flex-nowrap overflow-x-auto">
          <TabsTrigger value="details" className="shrink-0">{t("episode.details")}</TabsTrigger>
          <TabsTrigger value="search" className="shrink-0">{t("episode.search")}</TabsTrigger>
          <TabsTrigger value="blocklist" className="shrink-0">Blocklist</TabsTrigger>
        </TabsList>
        <TabsContent value="details">
          <EpisodeDetailsPanel episode={episode} mediaFiles={episodeFiles} onDeleteFile={onDeleteFile} />
        </TabsContent>
        <TabsContent value="search">
          <div className="mb-2 flex items-center justify-end">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => onRunEpisodeSearch(episode)}
              disabled={episodeLoading}
              aria-label={t("label.search")}
            >
              <Search className="h-4 w-4" />
              <span className="ml-1">
                {episodeLoading ? t("label.searching") : t("label.refresh")}
              </span>
            </Button>
          </div>
          {episodeLoading ? (
            <div className="flex flex-col items-center justify-center gap-4 py-16">
              <Loader2 className="h-10 w-10 animate-spin text-emerald-500" />
              <p className="text-lg text-muted-foreground">{t("label.searching")}</p>
            </div>
          ) : episodeResults.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("nzb.noResultsYet")}</p>
          ) : (
            <SearchResultBuckets
              results={episodeResults}
              onQueue={onQueueFromEpisodeSearch}
            />
          )}
        </TabsContent>
        <TabsContent value="blocklist">
          <EpisodeBlocklistPanel entries={releaseBlocklistEntries.filter((entry) =>
            blocklistEntryMatchesEpisode(entry, episode, collection),
          )} />
        </TabsContent>
      </Tabs>
    ),
    [collection, onDeleteFile, onEpisodeTabChange, onQueueFromEpisodeSearch, onRunEpisodeSearch, releaseBlocklistEntries, t],
  );

  return (
    <div className="overflow-hidden rounded-lg border border-border bg-background/40">
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
        <div className="flex items-center gap-2">
          <button
            type="button"
            disabled={seasonToggling}
            aria-label={t("title.seasonMonitored")}
            className={cn(
              "inline-flex size-6 shrink-0 items-center justify-center rounded transition-colors",
              seasonToggling && "opacity-50",
              seasonCheckedState === true
                ? "text-emerald-600 dark:text-emerald-300"
                : seasonCheckedState === "indeterminate"
                  ? "text-amber-500 dark:text-amber-400"
                  : "text-muted-foreground/60",
            )}
            onClick={(e) => {
              e.stopPropagation();
              if (!onSetCollectionMonitored) return;
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
          <div>
            <p className="text-sm font-semibold text-foreground">
              {seasonHeading(collection)}
            </p>
            {collection.firstEpisodeNumber || collection.lastEpisodeNumber ? (
              <p className="text-xs text-muted-foreground">
                Episodes {collection.firstEpisodeNumber ?? "?"} - {collection.lastEpisodeNumber ?? "?"}
              </p>
            ) : null}
          </div>
        </div>
        <div className="flex items-center gap-1">
          <span className="text-xs text-muted-foreground">
            {collection.collectionType === "interstitial" ? (
              <span className="inline-flex items-center gap-1">
                <Film className="h-3 w-3" />
                Movie
              </span>
            ) : isSpecialsCollection(collection) ? (
              <span className="inline-flex items-center gap-1">
                <Star className="h-3 w-3" />
                {collection.specialsMovies.length > 0
                  ? `${collection.specialsMovies.length} movie${collection.specialsMovies.length === 1 ? "" : "s"} · `
                  : ""}
                {episodes.length} special{episodes.length === 1 ? "" : "s"}
              </span>
            ) : (
              <>
                {episodes.length} episode
                {episodes.length === 1 ? "" : "s"}
              </>
            )}
          </span>
          {onRunSeasonSearch && !isSpecialsCollection(collection) && collection.collectionType !== "interstitial" ? (
            <HoverCard openDelay={600} closeDelay={75}>
              <HoverCardTrigger asChild>
                <EpisodeTableActionButton
                  tone="search"
                  aria-label={t("series.searchSeason")}
                  disabled={seasonSearchLoading === true}
                  onClick={(e) => {
                    e.stopPropagation();
                    onRunSeasonSearch();
                  }}
                  label={t("series.searchSeason")}
                >
                  {seasonSearchLoading === true ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Search className="h-4 w-4" />
                  )}
                </EpisodeTableActionButton>
              </HoverCardTrigger>
              <HoverCardContent side="left" className="w-auto p-2 text-xs">
                {t("help.seasonSearchTooltip")}
              </HoverCardContent>
            </HoverCard>
          ) : null}
        </div>
      </div>

      {expanded ? (
        collection.collectionType === "interstitial" ? (
          <div className="border-t border-border px-4 py-3 text-sm text-muted-foreground">
            {collection.interstitialMovie ? (
              <div className="flex flex-col gap-3">
                <InterstitialMoviePanel
                  movie={collection.interstitialMovie}
                  hasFile={collection.orderedPath != null}
                  monitored={collection.monitored}
                />
                {collection.monitored && !collection.orderedPath && onAutoSearchInterstitialMovie ? (
                  <button
                    type="button"
                    disabled={autoSearchInterstitialMovieLoading}
                    onClick={() => onAutoSearchInterstitialMovie(collection)}
                    className="inline-flex items-center gap-1.5 self-start rounded-md border border-border bg-card/45 px-3 py-1.5 text-xs text-card-foreground transition hover:bg-muted disabled:opacity-50"
                  >
                    <Search className="h-3.5 w-3.5" />
                    {autoSearchInterstitialMovieLoading ? "Searching..." : "Search Movie"}
                  </button>
                ) : null}
              </div>
            ) : (
              <p>No local metadata has been hydrated for this movie yet.</p>
            )}
          </div>
        ) : (
          <>
            {isSpecialsCollection(collection) && collection.specialsMovies.length > 0 ? (
              <div className="border-t border-border px-4 py-3">
                <div className="space-y-4">
                  {collection.specialsMovies.map((movie) => (
                    <div
                      key={`${collection.id}-${movie.tvdbId || movie.name}`}
                      className="rounded-xl border border-border/70 bg-card/40 p-3"
                    >
                      <InterstitialMoviePanel movie={movie} />
                    </div>
                  ))}
                </div>
              </div>
            ) : null}
            {seasonSearchResults && seasonSearchResults.length > 0 && onQueueFromSeasonSearch ? (
              <div className="border-t border-border px-4 py-3">
                <p className="mb-2 text-xs font-medium text-muted-foreground">Season pack results</p>
                <SearchResultBuckets
                  results={seasonSearchResults}
                  onQueue={onQueueFromSeasonSearch}
                />
              </div>
            ) : null}
            {episodes.length === 0 ? (
              <div className="border-t border-border px-4 py-3 text-sm text-muted-foreground">
                {collection.specialsMovies.length > 0
                  ? "No episode records for this season."
                  : "No episode records for this season."}
              </div>
            ) : (
              isMobile ? (
                <div className="border-t border-border px-3 py-3">
                  <div className="space-y-3">
                    {episodes.map((episode) => {
                      const isPanelOpen = expandedEpisodeRows.has(episode.id);
                      const activeTab = episodeActiveTab[episode.id] ?? "details";
                      const episodeResults = searchResultsByEpisode[episode.id] ?? [];
                      const episodeLoading = searchLoadingByEpisode[episode.id] === true;
                      const autoSearching = autoSearchLoadingByEpisode[episode.id] === true;
                      const episodeFiles = mediaFilesByEpisode[episode.id] ?? [];
                      const episodeRuntime = formatRuntimeFromSeconds(episode.durationSeconds);

                      return (
                        <div
                          key={episode.id}
                          data-episode-id={episode.id}
                          className={cn(
                            "rounded-lg border border-border bg-card/50 p-3",
                            !episode.monitored && "opacity-60",
                          )}
                        >
                          <div className="flex items-start gap-3">
                            <button
                              type="button"
                              disabled={episodeToggling.has(episode.id)}
                              aria-label={t("title.episodeMonitored")}
                              className={cn(
                                "mt-0.5 inline-flex size-6 shrink-0 items-center justify-center rounded transition-colors",
                                episodeToggling.has(episode.id) && "opacity-50",
                                episode.monitored
                                  ? "text-emerald-600 dark:text-emerald-300"
                                  : "text-muted-foreground/60",
                              )}
                              onClick={() => {
                                if (!onSetEpisodeMonitored) return;
                                setEpisodeToggling((prev) => new Set(prev).add(episode.id));
                                onSetEpisodeMonitored(episode.id, !episode.monitored)
                                  .finally(() => {
                                    setEpisodeToggling((prev) => {
                                      const next = new Set(prev);
                                      next.delete(episode.id);
                                      return next;
                                    });
                                  });
                              }}
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
                                  type="button"
                                  className="min-w-0 flex-1 text-left"
                                  onClick={() => onToggleEpisodeDetails(episode)}
                                >
                                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                                    <span className="rounded bg-accent px-2 py-0.5 font-mono text-card-foreground">
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
                                  {renderEpisodeQualityBadge(episode, episodeFiles)}
                                </div>
                              </div>
                              <div className="mt-2 flex flex-wrap gap-2">
                                {renderEpisodeTypeBadges(episode)}
                              </div>
                              <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
                                <span className="inline-flex items-center gap-1">
                                  <CalendarDays className="h-3.5 w-3.5" />
                                  {formatDate(episode.airDate)}
                                </span>
                                {episodeRuntime ? (
                                  <span className="inline-flex items-center gap-1">
                                    <Clock3 className="h-3 w-3" />
                                    {episodeRuntime}
                                  </span>
                                ) : null}
                              </div>
                              <div className="mt-3 flex flex-wrap gap-2">
                                {onAutoSearchEpisode ? (
                                  <Button
                                    type="button"
                                    size="sm"
                                    variant="secondary"
                                    className="flex-1 sm:flex-none"
                                    onClick={() => onAutoSearchEpisode(episode)}
                                    disabled={autoSearching}
                                  >
                                    {autoSearching ? (
                                      <Loader2 className="h-4 w-4 animate-spin" />
                                    ) : (
                                      <Zap className="h-4 w-4" />
                                    )}
                                    <span>{t("label.search")}</span>
                                  </Button>
                                ) : null}
                                <Button
                                  type="button"
                                  size="sm"
                                  variant="outline"
                                  className="flex-1 sm:flex-none"
                                  onClick={() => onToggleEpisodeSearch(episode)}
                                >
                                  <Search className="h-4 w-4" />
                                  <span>{t("label.interactiveSearch")}</span>
                                </Button>
                              </div>
                              {isPanelOpen ? (
                                <div className="mt-3 border-t border-border pt-3">
                                  {renderEpisodePanel(
                                    episode,
                                    activeTab,
                                    episodeResults,
                                    episodeLoading,
                                    episodeFiles,
                                  )}
                                </div>
                              ) : null}
                            </div>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              ) : (
                <div className="overflow-x-auto">
                  <Table className="min-w-[760px]">
                    <TableHeader>
                      <TableRow>
                        <TableHead className="w-10 text-center" />
                        <TableHead className="w-16 text-center">Episode</TableHead>
                        <TableHead>Title</TableHead>
                        <TableHead className="w-40">Air Date</TableHead>
                        <TableHead className="w-28 text-center">{t("episode.quality")}</TableHead>
                        <TableHead className="w-28 text-right">Actions</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {episodes.map((episode) => {
                        const isPanelOpen = expandedEpisodeRows.has(episode.id);
                        const activeTab = episodeActiveTab[episode.id] ?? "details";
                        const episodeResults = searchResultsByEpisode[episode.id] ?? [];
                        const episodeLoading = searchLoadingByEpisode[episode.id] === true;
                        const autoSearching = autoSearchLoadingByEpisode[episode.id] === true;
                        const episodeFiles = mediaFilesByEpisode[episode.id] ?? [];
                        const episodeRuntime = formatRuntimeFromSeconds(episode.durationSeconds);

                        return (
                          <React.Fragment key={episode.id}>
                            <TableRow data-episode-id={episode.id} className={`cv-auto-row-sm${episode.monitored ? "" : " opacity-50"}`}>
                              <TableCell className="pl-2 pr-0 text-right align-middle">
                                <div className="flex items-center justify-end">
                                  <button
                                    type="button"
                                    disabled={episodeToggling.has(episode.id)}
                                    aria-label={t("title.episodeMonitored")}
                                    className={cn(
                                      "inline-flex size-6 items-center justify-center rounded transition-colors",
                                      episodeToggling.has(episode.id) && "opacity-50",
                                      episode.monitored
                                        ? "text-emerald-600 dark:text-emerald-300"
                                        : "text-muted-foreground/60",
                                    )}
                                    onClick={() => {
                                      if (!onSetEpisodeMonitored) return;
                                      setEpisodeToggling((prev) => new Set(prev).add(episode.id));
                                      onSetEpisodeMonitored(episode.id, !episode.monitored)
                                        .finally(() => {
                                          setEpisodeToggling((prev) => {
                                            const next = new Set(prev);
                                            next.delete(episode.id);
                                            return next;
                                          });
                                        });
                                    }}
                                  >
                                    {episode.monitored ? (
                                      <Eye className="size-5" />
                                    ) : (
                                      <EyeOff className="size-5" />
                                    )}
                                  </button>
                                </div>
                              </TableCell>
                              <TableCell className="text-center align-middle font-mono text-sm text-card-foreground">
                                <div className="flex flex-col items-center gap-0.5">
                                  <span>{episode.episodeNumber ?? episode.episodeLabel ?? "—"}</span>
                                  {episode.absoluteNumber && facet === "anime" ? (
                                    <span className="text-[10px] text-muted-foreground">
                                      #{episode.absoluteNumber}
                                    </span>
                                  ) : null}
                                </div>
                              </TableCell>
                              <TableCell
                                className="cursor-pointer align-middle text-sm text-card-foreground hover:text-foreground"
                                onClick={() => onToggleEpisodeDetails(episode)}
                              >
                                <div className="flex items-center gap-1.5">
                                  <span>{episode.title || episode.episodeLabel || "—"}</span>
                                  {renderEpisodeTypeBadges(episode)}
                                </div>
                                {episodeRuntime ? (
                                  <span className="inline-flex items-center gap-1 text-[10px] text-muted-foreground">
                                    <Clock3 className="h-3 w-3" />
                                    {episodeRuntime}
                                  </span>
                                ) : null}
                              </TableCell>
                              <TableCell className="text-muted-foreground">
                                <span className="inline-flex items-center gap-1">
                                  <CalendarDays className="h-3.5 w-3.5" />
                                  {formatDate(episode.airDate)}
                                </span>
                              </TableCell>
                              <TableCell className="text-center">
                                <div className="inline-flex items-center gap-1">
                                  {renderEpisodeQualityBadge(episode, episodeFiles)}
                                </div>
                              </TableCell>
                              <TableCell className="text-right">
                                <div className="inline-flex items-center justify-end gap-2">
                                  {onAutoSearchEpisode ? (
                                    <HoverCard openDelay={3000} closeDelay={75}>
                                      <HoverCardTrigger asChild>
                                        <EpisodeTableActionButton
                                          tone="auto"
                                          onClick={() => onAutoSearchEpisode?.(episode)}
                                          disabled={autoSearching}
                                          label={t("label.search")}
                                        >
                                          {autoSearching ? (
                                            <Loader2 className="h-4 w-4 animate-spin" />
                                          ) : (
                                            <Zap className="h-4 w-4" />
                                          )}
                                        </EpisodeTableActionButton>
                                      </HoverCardTrigger>
                                      <HoverCardContent>
                                        <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                                          {t("help.autoSearchTooltip")}
                                        </p>
                                      </HoverCardContent>
                                    </HoverCard>
                                  ) : null}
                                  <HoverCard openDelay={3000} closeDelay={75}>
                                    <HoverCardTrigger asChild>
                                      <EpisodeTableActionButton
                                        tone="search"
                                        onClick={() => onToggleEpisodeSearch(episode)}
                                        label={t("label.search")}
                                      >
                                        <Search className="h-4 w-4" />
                                      </EpisodeTableActionButton>
                                    </HoverCardTrigger>
                                    <HoverCardContent>
                                      <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                                        {t("help.interactiveSearchTooltip")}
                                      </p>
                                    </HoverCardContent>
                                  </HoverCard>
                                </div>
                              </TableCell>
                            </TableRow>
                            {isPanelOpen ? (
                              <TableRow>
                                <TableCell colSpan={6} className="border-t border-border bg-background/40 p-0">
                                  <div className="px-4 py-3">
                                    {renderEpisodePanel(
                                      episode,
                                      activeTab,
                                      episodeResults,
                                      episodeLoading,
                                      episodeFiles,
                                    )}
                                  </div>
                                </TableCell>
                              </TableRow>
                            ) : null}
                          </React.Fragment>
                        );
                      })}
                    </TableBody>
                  </Table>
                </div>
              )
            )}
          </>
        )
      ) : null}
    </div>
  );
}
