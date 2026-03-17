import * as React from "react";
import { ExternalLink, FileInput, FolderOpen, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Clapperboard } from "lucide-react";
import { useClient } from "urql";
import type { Release } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { searchSeriesEpisodeQuery } from "@/lib/graphql/queries";
import { queueExistingMutation } from "@/lib/graphql/mutations";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleDetail,
  TitleEvent,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import {
  episodePanelReducer,
  initialEpisodePanelState,
  type EpisodePanelTab,
} from "./episode-panel-reducer";
import {
  sortDbCollections,
  findLatestSeasonKey,
  episodeSortValue,
  isSpecialsCollection,
  formatDate,
} from "./helpers";
import { OverviewControlPanel } from "../overview-control-panel";
import { OverviewBackLink } from "../overview-back-link";
import { TitleSettingsPanel } from "./title-settings-panel";
import { SeasonSection } from "./season-section";

type Props = {
  loading: boolean;
  hydrating: boolean;
  title: TitleDetail | null;
  collections: TitleCollection[];
  events: TitleEvent[];
  episodesByCollection: Record<string, CollectionEpisode[]>;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  subtitleDownloads?: { id: string; mediaFileId: string; language: string; provider: string; hearingImpaired: boolean; forced: boolean }[];
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  onTitleChanged?: () => Promise<void>;
  onBackToList?: () => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  onSetTitleMonitored?: (monitored: boolean) => Promise<void>;
  onSearchMonitored?: () => Promise<void> | void;
  onRefreshAndScan?: () => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => Promise<void> | void;
  onAutoSearchInterstitialMovie?: (collection: TitleCollection) => Promise<void> | void;
  qualityProfiles?: { id: string; name: string }[];
  defaultRootFolder?: string;
  rootFolders?: { path: string; isDefault: boolean }[];
  onUpdateTitleTags?: (newTags: string[]) => Promise<void>;
  completedDownloads?: DownloadQueueItem[];
  onOpenManualImport?: (item: DownloadQueueItem) => void;
  initialEpisodeId?: string | null;
  seasonSearchResultsByCollection?: Record<string, Release[]>;
  seasonSearchLoadingByCollection?: Record<string, boolean>;
  onRunSeasonSearch?: (collection: TitleCollection) => Promise<void> | void;
  onQueueFromSeasonSearch?: (release: Release) => Promise<void> | void;
  monitoredUpdating?: boolean;
  searchMonitoredLoading?: boolean;
  refreshAndScanLoading?: boolean;
  onRequestDeleteTitle?: () => void;
  deleteLoading?: boolean;
  onDeleteFile?: (fileId: string) => void;
};

export function SeriesOverviewView({
  loading,
  hydrating,
  title,
  collections,
  events,
  episodesByCollection,
  mediaFilesByEpisode,
  subtitleDownloads,
  releaseBlocklistEntries,
  onTitleChanged,
  onBackToList,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  onSetTitleMonitored,
  onSearchMonitored,
  onRefreshAndScan,
  onAutoSearchEpisode,
  onAutoSearchInterstitialMovie,
  qualityProfiles,
  defaultRootFolder,
  rootFolders,
  onUpdateTitleTags,
  completedDownloads,
  onOpenManualImport,
  initialEpisodeId,
  seasonSearchResultsByCollection,
  seasonSearchLoadingByCollection,
  onRunSeasonSearch,
  onQueueFromSeasonSearch,
  monitoredUpdating = false,
  searchMonitoredLoading = false,
  refreshAndScanLoading = false,
  onRequestDeleteTitle,
  deleteLoading = false,
  onDeleteFile,
}: Props) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const backLabel = title?.facet === "anime" ? t("nav.anime") : t("nav.series");
  const sortedCollections = React.useMemo(
    () => sortDbCollections(collections),
    [collections],
  );

  const latestKey = React.useMemo(
    () => findLatestSeasonKey(sortedCollections),
    [sortedCollections],
  );

  const [expandedKeys, setExpandedKeys] = React.useState<Set<string>>(new Set());
  const [episodePanel, dispatchEpisodePanel] = React.useReducer(episodePanelReducer, initialEpisodePanelState);

  // Initialize expanded state when data arrives
  const initializedRef = React.useRef(false);
  React.useEffect(() => {
    if (initializedRef.current) return;

    // If we have an initialEpisodeId, find which collection it belongs to and expand that
    if (initialEpisodeId && Object.keys(episodesByCollection).length > 0) {
      for (const [collectionId, episodes] of Object.entries(episodesByCollection)) {
        const match = episodes.find((ep) => ep.id === initialEpisodeId);
        if (match) {
          initializedRef.current = true;
          setExpandedKeys(new Set([`s-${collectionId}`]));
          dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId: initialEpisodeId });
          // Scroll to the episode row after DOM updates
          requestAnimationFrame(() => {
            const el = document.querySelector(`[data-episode-id="${initialEpisodeId}"]`);
            el?.scrollIntoView({ behavior: "smooth", block: "center" });
          });
          return;
        }
      }
    }

    if (latestKey) {
      initializedRef.current = true;
      setExpandedKeys(new Set([latestKey]));
    }
  }, [latestKey, initialEpisodeId, episodesByCollection]);

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

  const handleRunEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode) => {
      if (!title) return;
      const episodeId = episode.id;
      dispatchEpisodePanel({ type: "SET_SEARCH_LOADING", episodeId, loading: true });

      const tvdbId =
        title.externalIds
          ?.find((eid) => eid.source.toLowerCase() === "tvdb")
          ?.value?.trim() ?? "";
      const anidbId =
        title.externalIds
          ?.find((eid) => eid.source.toLowerCase() === "anidb")
          ?.value?.trim() || null;
      const collection = collections.find((c) => c.id === episode.collectionId);
      const seasonNum = episode.seasonNumber?.trim().replace(/\D+/g, "")
        || collection?.collectionIndex?.trim().replace(/\D+/g, "")
        || "1";
      const episodeNum = episode.episodeNumber?.trim().replace(/\D+/g, "") || "1";
      const absoluteEpisode = episode.absoluteNumber
        ? parseInt(episode.absoluteNumber.replace(/\D+/g, ""), 10) || null
        : null;

      client.query(searchSeriesEpisodeQuery, {
        title: title.name,
        season: seasonNum,
        episode: episodeNum,
        tvdbId,
        anidbId,
        category: title.facet,
        absoluteEpisode,
        limit: 25,
      }).toPromise()
        .then(({ data, error: queryError }) => {
          if (queryError) throw queryError;
          dispatchEpisodePanel({
            type: "SET_SEARCH_RESULTS",
            episodeId,
            results: data.searchIndexersEpisode ?? [],
          });
        })
        .catch(() => {
          dispatchEpisodePanel({ type: "SET_SEARCH_RESULTS", episodeId, results: [] });
        })
        .finally(() => {
          dispatchEpisodePanel({ type: "SET_SEARCH_LOADING", episodeId, loading: false });
        });
    },
    [client, title, collections],
  );

  const handleToggleEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode) => {
      const episodeId = episode.id;
      const isOpen = episodePanel.expandedEpisodeRows.has(episodeId);
      const currentTab = episodePanel.episodeActiveTab[episodeId] ?? "details";

      if (isOpen && currentTab === "search") {
        dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId });
      } else {
        if (!isOpen) {
          dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId });
        }
        dispatchEpisodePanel({ type: "SET_EPISODE_TAB", episodeId, tab: "search" });
        if (!Object.prototype.hasOwnProperty.call(episodePanel.searchResultsByEpisode, episodeId)) {
          handleRunEpisodeSearch(episode);
        }
      }
    },
    [episodePanel.expandedEpisodeRows, episodePanel.episodeActiveTab, handleRunEpisodeSearch, episodePanel.searchResultsByEpisode],
  );

  const handleToggleEpisodeDetails = React.useCallback(
    (episode: CollectionEpisode) => {
      const episodeId = episode.id;
      const isOpen = episodePanel.expandedEpisodeRows.has(episodeId);
      const currentTab = episodePanel.episodeActiveTab[episodeId] ?? "details";

      if (isOpen && currentTab === "details") {
        dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId });
      } else {
        if (!isOpen) {
          dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId });
        }
        dispatchEpisodePanel({ type: "SET_EPISODE_TAB", episodeId, tab: "details" });
      }
    },
    [episodePanel.expandedEpisodeRows, episodePanel.episodeActiveTab],
  );

  const handleEpisodeTabChange = React.useCallback(
    (episodeId: string, tab: EpisodePanelTab, episode: CollectionEpisode) => {
      dispatchEpisodePanel({ type: "SET_EPISODE_TAB", episodeId, tab });
      if (tab === "search" && !Object.prototype.hasOwnProperty.call(episodePanel.searchResultsByEpisode, episodeId)) {
        handleRunEpisodeSearch(episode);
      }
    },
    [handleRunEpisodeSearch, episodePanel.searchResultsByEpisode],
  );

  const handleQueueFromEpisodeSearch = React.useCallback(
    (release: Release) => {
      if (!title) return Promise.resolve();
      if (release.qualityProfileDecision && release.qualityProfileDecision.allowed === false) {
        const reason = release.qualityProfileDecision.blockCodes.join(", ") || "unknown";
        setGlobalStatus(t("status.qualityProfileBlocked", { reason }));
        return Promise.resolve();
      }

      const sourceHint = release.downloadUrl || release.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noSource", { name: title.name }));
        return Promise.resolve();
      }

      return client.mutation(queueExistingMutation, {
        input: {
          titleId: title.id,
          sourceHint,
          sourceKind: release.sourceKind ?? null,
          sourceTitle: release.title,
        },
      }).toPromise()
        .then(async ({ error: mutationError }) => {
          if (mutationError) throw mutationError;
          const queuedMessage = t("status.queuedLatest", { name: title.name });
          setGlobalStatus(queuedMessage);
          await onTitleChanged?.();
        })
        .catch((error: unknown) => {
          setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        });
    },
    [onTitleChanged, client, setGlobalStatus, t, title],
  );

  const handleAutoSearchEpisode = React.useCallback(
    (episode: CollectionEpisode) => {
      if (!onAutoSearchEpisode) return;
      const episodeId = episode.id;
      dispatchEpisodePanel({ type: "SET_AUTO_SEARCH_LOADING", episodeId, loading: true });
      Promise.resolve(onAutoSearchEpisode(episode))
        .catch((error: unknown) => {
          setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        })
        .finally(() => {
          dispatchEpisodePanel({ type: "SET_AUTO_SEARCH_LOADING", episodeId, loading: false });
        });
    },
    [onAutoSearchEpisode, setGlobalStatus, t],
  );

  const [interstitialSearchLoading, setInterstitialSearchLoading] = React.useState(false);
  const handleAutoSearchInterstitialMovie = React.useCallback(
    (collection: TitleCollection) => {
      if (!onAutoSearchInterstitialMovie) return;
      setInterstitialSearchLoading(true);
      Promise.resolve(onAutoSearchInterstitialMovie(collection))
        .catch((error: unknown) => {
          setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        })
        .finally(() => setInterstitialSearchLoading(false));
    },
    [onAutoSearchInterstitialMovie, setGlobalStatus, t],
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
          label={`Back to ${backLabel}`}
          onClick={() => onBackToList?.()}
        />
        <Card>
          <CardContent className="pt-6">
            <p className="text-muted-foreground">Title not found.</p>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <OverviewBackLink
        label={`Back to ${backLabel}`}
        onClick={() => onBackToList?.()}
      />

      <Card
        className="relative overflow-hidden p-0"
        style={(title.backgroundUrl ?? title.bannerUrl) ? {
          backgroundImage: `linear-gradient(to top, var(--color-card) 0%, var(--color-card) 5%, color-mix(in srgb, var(--color-card) 80%, transparent), color-mix(in srgb, var(--color-card) 50%, transparent)), url(${title.backgroundUrl ?? title.bannerUrl})`,
          backgroundSize: "cover",
          backgroundPosition: "center top",
          backgroundClip: "padding-box",
        } : undefined}
      >
        <CardContent className="relative p-4">
          <div className="flex flex-col gap-4 sm:flex-row sm:gap-5">
            <div className="mx-auto shrink-0 sm:mx-0">
              {title.posterUrl ? (
                <img
                  src={title.posterUrl}
                  alt={title.name}
                  className="block h-auto w-32 rounded-lg object-cover shadow-lg sm:w-[180px]"
                />
              ) : (
                <div className="flex h-48 w-32 items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60 sm:h-[270px] sm:w-[180px]">
                  No Poster
                </div>
              )}
            </div>

            <div className="min-w-0 flex-1">
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
                      ? "bg-emerald-500/20 text-emerald-700 dark:text-emerald-300"
                      : "bg-accent text-muted-foreground"
                  }`}
                >
                  {title.monitored ? "Monitored" : "Unmonitored"}
                </span>
                <span className="inline-flex items-center rounded-full border border-border px-2.5 py-0.5 text-xs font-medium capitalize text-muted-foreground">
                  {title.facet}
                </span>
                {title.contentStatus ? (
                  <span className="inline-flex items-center rounded-full border border-border px-2.5 py-0.5 text-xs font-medium capitalize text-muted-foreground">
                    {title.contentStatus}
                  </span>
                ) : null}
                {title.network ? (
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <Clapperboard className="h-3.5 w-3.5" />
                    {title.network}
                  </span>
                ) : null}
                {title.facet === "anime" ? (
                  <>
                    {(() => { const e = title.externalIds.find((e) => e.source === "mal"); return e ? (
                      <a href={`https://myanimelist.net/anime/${e.value}`} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-xs text-primary hover:underline">
                        <ExternalLink className="h-3 w-3" />
                        {t("anime.malLink")}
                      </a>
                    ) : null; })()}
                    {(() => { const e = title.externalIds.find((e) => e.source === "anilist"); return e ? (
                      <a href={`https://anilist.co/anime/${e.value}`} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-xs text-primary hover:underline">
                        <ExternalLink className="h-3 w-3" />
                        {t("anime.anilistLink")}
                      </a>
                    ) : null; })()}
                    {(() => { const e = title.externalIds.find((e) => e.source === "anidb"); return e ? (
                      <a href={`https://anidb.net/anime/${e.value}`} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-xs text-primary hover:underline">
                        <ExternalLink className="h-3 w-3" />
                        {t("anime.anidbLink")}
                      </a>
                    ) : null; })()}
                  </>
                ) : null}
              </div>

              {title.genres.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {title.genres.map((genre) => (
                    <span
                      key={genre}
                      className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground"
                    >
                      {genre}
                    </span>
                  ))}
                </div>
              ) : null}

              {title.overview ? (
                <p className="mt-4 text-sm leading-relaxed text-foreground/70">
                  {title.overview}
                </p>
              ) : null}

              <p className="mt-2 text-left text-xs text-muted-foreground/60 sm:text-right">
                Added {formatDate(title.createdAt)}
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

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
        settingsPanel={
          onUpdateTitleTags && qualityProfiles && defaultRootFolder ? (
            <TitleSettingsPanel
              title={title}
              qualityProfiles={qualityProfiles}
              defaultRootFolder={defaultRootFolder}
              rootFolders={rootFolders ?? []}
              onUpdateTitleTags={onUpdateTitleTags}
            />
          ) : undefined
        }
      />

      <div>
        <Card className="relative overflow-hidden">
          <CardHeader>
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <CardTitle className="flex items-center gap-2 text-base">
                <FolderOpen className="h-4 w-4" />
                Seasons and Episodes
              </CardTitle>
              {onOpenManualImport && completedDownloads && completedDownloads.length > 0 ? (
                <Button
                  className="w-full sm:w-auto"
                  variant="outline"
                  size="sm"
                  onClick={() => onOpenManualImport(completedDownloads[0])}
                >
                  <FileInput className="mr-1.5 h-4 w-4" />
                  Manual Import
                </Button>
              ) : null}
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            {sortedCollections.length > 0 ? (
              sortedCollections.map((collection) => {
                const key = `s-${collection.id}`;
                const sortedEpisodes = [
                  ...(episodesByCollection[collection.id] ?? []),
                ].sort((left, right) => episodeSortValue(right) - episodeSortValue(left));

                // Hide specials section when it has no episodes and no movies
                if (isSpecialsCollection(collection) && sortedEpisodes.length === 0 && collection.specialsMovies.length === 0) {
                  return null;
                }

                return (
                  <SeasonSection
                    key={key}
                    collection={collection}
                    episodes={sortedEpisodes}
                    facet={title.facet}
                    expanded={expandedKeys.has(key)}
                    onToggle={() => toggleKey(key)}
                    expandedEpisodeRows={episodePanel.expandedEpisodeRows}
                    episodeActiveTab={episodePanel.episodeActiveTab}
                    mediaFilesByEpisode={mediaFilesByEpisode}
                    subtitleDownloads={subtitleDownloads}
                    releaseBlocklistEntries={releaseBlocklistEntries}
                    searchResultsByEpisode={episodePanel.searchResultsByEpisode}
                    searchLoadingByEpisode={episodePanel.searchLoadingByEpisode}
                    autoSearchLoadingByEpisode={episodePanel.autoSearchLoadingByEpisode}
                    onToggleEpisodeSearch={handleToggleEpisodeSearch}
                    onToggleEpisodeDetails={handleToggleEpisodeDetails}
                    onEpisodeTabChange={handleEpisodeTabChange}
                    onRunEpisodeSearch={handleRunEpisodeSearch}
                    onQueueFromEpisodeSearch={handleQueueFromEpisodeSearch}
                    onAutoSearchEpisode={handleAutoSearchEpisode}
                    onSetCollectionMonitored={onSetCollectionMonitored}
                    onSetEpisodeMonitored={onSetEpisodeMonitored}
                    seasonSearchResults={seasonSearchResultsByCollection?.[collection.id]}
                    seasonSearchLoading={seasonSearchLoadingByCollection?.[collection.id] === true}
                    onRunSeasonSearch={onRunSeasonSearch ? () => onRunSeasonSearch(collection) : undefined}
                    onQueueFromSeasonSearch={onQueueFromSeasonSearch}
                    onDeleteFile={onDeleteFile}
                    onAutoSearchInterstitialMovie={onAutoSearchInterstitialMovie ? handleAutoSearchInterstitialMovie : undefined}
                    autoSearchInterstitialMovieLoading={interstitialSearchLoading}
                  />
                );
              })
            ) : (
              <p className="text-sm text-muted-foreground">
                No seasons are tracked for this show yet.
              </p>
            )}
          </CardContent>
          {hydrating ? (
            <div className="absolute inset-0 z-10 flex items-center justify-center bg-background/75 backdrop-blur-sm">
              <div className="flex items-center gap-3 rounded-full border border-border bg-card/95 px-4 py-2 text-sm font-medium text-foreground shadow-lg">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>Fetching data</span>
              </div>
            </div>
          ) : null}
        </Card>
      </div>

      {events.length > 0 ? (
        <div>
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Recent Activity</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="space-y-2">
                {events.map((event) => (
                  <div key={event.id} className="flex items-start gap-3 text-sm">
                    <span className="shrink-0 text-xs text-muted-foreground/60">
                      {formatDate(event.occurredAt)}
                    </span>
                    <span className="capitalize text-muted-foreground">
                      {event.eventType.replace(/_/g, " ")}
                    </span>
                    <span className="text-muted-foreground">{event.message}</span>
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        </div>
      ) : null}
    </div>
  );
}
