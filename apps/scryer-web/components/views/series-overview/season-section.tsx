import * as React from "react";
import {
  CalendarDays,
  ChevronDown,
  ChevronRight,
  Clock3,
  Film,
  Loader2,
  Search,
  Star,
  Zap,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { EpisodePanelTab } from "./episode-panel-reducer";
import {
  dedupeInsensitive,
  normalizeMovieCollectionLabel,
  isSpecialsCollection,
  seasonHeading,
  formatDate,
  formatRuntimeFromSeconds,
  blocklistEntryMatchesEpisode,
} from "./helpers";
import { EpisodeDetailsPanel } from "./episode-details-panel";
import { InterstitialMoviePanel } from "./interstitial-movie-panel";
import { EpisodeBlocklistPanel } from "./episode-blocklist-panel";

export function SeasonSection({
  collection,
  episodes,
  titleName,
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
  onLoadInterstitialMovieMetadata,
  interstitialMovieMetadata,
  interstitialMovieMetadataLoaded,
  interstitialMovieMetadataLoading,
}: {
  collection: TitleCollection;
  facet: string;
  episodes: CollectionEpisode[];
  titleName: string;
  expanded: boolean;
  onToggle: () => void;
  expandedEpisodeRows: Set<string>;
  episodeActiveTab: Record<string, EpisodePanelTab>;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
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
  onLoadInterstitialMovieMetadata: (collectionId: string, candidates: string[]) => void;
  interstitialMovieMetadata: MetadataTvdbSearchItem | null;
  interstitialMovieMetadataLoaded: boolean;
  interstitialMovieMetadataLoading: boolean;
}) {
  const t = useTranslate();
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

  React.useEffect(() => {
    if (!expanded || collection.collectionType !== "interstitial") return;
    const candidates = dedupeInsensitive([
      ...episodes
        .map((episode) => episode.title?.trim() ?? episode.episodeLabel?.trim() ?? "")
        .filter((candidate): candidate is string => candidate.length > 0),
      normalizeMovieCollectionLabel(collection.label) ?? "",
      titleName,
    ]);
    onLoadInterstitialMovieMetadata(collection.id, candidates);
  }, [collection.collectionType, collection.id, collection.label, episodes, expanded, titleName, onLoadInterstitialMovieMetadata]);

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
          <Checkbox
            checked={seasonCheckedState}
            disabled={seasonToggling}
            aria-label={t("title.seasonMonitored")}
            className="size-6 [&_svg]:size-4"
            onCheckedChange={() => {
              if (!onSetCollectionMonitored) return;
              setSeasonToggling(true);
              const nextMonitored = seasonCheckedState !== true;
              onSetCollectionMonitored(collection.id, nextMonitored)
                .finally(() => setSeasonToggling(false));
            }}
            onClick={(e) => e.stopPropagation()}
          />
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
        <span className="text-xs text-muted-foreground">
          {collection.collectionType === "interstitial" ? (
            <span className="inline-flex items-center gap-1">
              <Film className="h-3 w-3" />
              Movie
            </span>
          ) : isSpecialsCollection(collection) ? (
            <span className="inline-flex items-center gap-1">
              <Star className="h-3 w-3" />
              {episodes.length} special{episodes.length === 1 ? "" : "s"}
            </span>
          ) : (
            <>
              {episodes.length} episode
              {episodes.length === 1 ? "" : "s"}
            </>
          )}
        </span>
      </div>

      {expanded ? (
        collection.collectionType === "interstitial" ? (
          <div className="border-t border-border px-4 py-3 text-sm text-muted-foreground">
            {interstitialMovieMetadataLoading ? (
              <div className="flex items-center gap-2">
                <Loader2 className="h-4 w-4 animate-spin text-emerald-500" />
                <span>Loading movie metadata…</span>
              </div>
            ) : interstitialMovieMetadataLoaded ? (
              interstitialMovieMetadata ? (
                <InterstitialMoviePanel movie={interstitialMovieMetadata} />
              ) : (
                <p>No metadata found for this movie in the catalog.</p>
              )
            ) : (
              <p>Unable to identify a movie title to look up metadata.</p>
            )}
          </div>
        ) : episodes.length === 0 ? (
          <div className="border-t border-border px-4 py-3 text-sm text-muted-foreground">
            No episode records for this season.
          </div>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-10 text-center" />
                <TableHead className="w-28 text-center">Episode</TableHead>
                <TableHead>Title</TableHead>
                <TableHead className="w-40">Air Date</TableHead>
                <TableHead className="w-28 text-center">{t("episode.quality")}</TableHead>
                <TableHead className="w-20 text-right">Actions</TableHead>
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
                      <TableCell className="px-2 text-center align-middle">
                        <Checkbox
                          checked={episode.monitored}
                          disabled={episodeToggling.has(episode.id)}
                          aria-label={t("title.episodeMonitored")}
                          className="size-6 [&_svg]:size-4"
                          onCheckedChange={() => {
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
                        />
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
                        className="align-middle text-sm text-card-foreground cursor-pointer hover:text-foreground"
                        onClick={() => onToggleEpisodeDetails(episode)}
                      >
                        <div className="flex items-center gap-1.5">
                          <span>{episode.title || episode.episodeLabel || "—"}</span>
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
                        {episodeFiles.length > 0 && episodeFiles[0].qualityLabel ? (
                          <span className="rounded border border-emerald-500/40 dark:border-emerald-500/30 bg-emerald-500/20 dark:bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                            {episodeFiles[0].qualityLabel}
                          </span>
                        ) : episode.monitored ? (
                          <span className="rounded border border-amber-500/30 bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-300">
                            {t("episode.missing")}
                          </span>
                        ) : null}
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="inline-flex items-center justify-end gap-2">
                          {onAutoSearchEpisode ? (
                            <HoverCard openDelay={3000} closeDelay={75}>
                              <HoverCardTrigger asChild>
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  aria-label={t("label.search")}
                                  onClick={() => onAutoSearchEpisode?.(episode)}
                                  disabled={autoSearching}
                                >
                                  {autoSearching ? (
                                    <Loader2 className="h-4 w-4 animate-spin" />
                                  ) : (
                                    <Zap className="h-4 w-4" />
                                  )}
                                </Button>
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
                              <Button
                                variant="ghost"
                                size="sm"
                                aria-label={t("label.search")}
                                onClick={() => onToggleEpisodeSearch(episode)}
                              >
                                <Search className="h-4 w-4" />
                              </Button>
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
                            <Tabs
                              value={activeTab}
                              onValueChange={(val) => onEpisodeTabChange(episode.id, val as EpisodePanelTab, episode)}
                            >
                              <TabsList>
                                <TabsTrigger value="details">{t("episode.details")}</TabsTrigger>
                                <TabsTrigger value="search">{t("episode.search")}</TabsTrigger>
                                <TabsTrigger value="blocklist">Blocklist</TabsTrigger>
                              </TabsList>
                              <TabsContent value="details">
                                <EpisodeDetailsPanel episode={episode} mediaFiles={episodeFiles} />
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
                                  <div className="flex items-center gap-3 py-3">
                                    <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
                                    <p className="text-sm text-muted-foreground">{t("label.searching")}</p>
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
                          </div>
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </React.Fragment>
                );
              })}
            </TableBody>
          </Table>
        )
      ) : null}
    </div>
  );
}
