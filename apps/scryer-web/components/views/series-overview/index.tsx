import * as React from "react";
import { ArrowLeft, ExternalLink, FileInput, FolderOpen, Settings2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Clapperboard } from "lucide-react";
import { useClient } from "urql";
import type { Release } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { searchSeriesEpisodeQuery } from "@/lib/graphql/queries";
import { queueExistingMutation } from "@/lib/graphql/mutations";
import { searchMetadataQuery } from "@/lib/graphql/queries";
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
  dedupeInsensitive,
  normalizeMovieCollectionLabel,
  formatDate,
} from "./helpers";
import { TitleSettingsPanel } from "./title-settings-panel";
import { SeasonSection } from "./season-section";

type Props = {
  loading: boolean;
  title: TitleDetail | null;
  collections: TitleCollection[];
  events: TitleEvent[];
  episodesByCollection: Record<string, CollectionEpisode[]>;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  onTitleChanged?: () => Promise<void>;
  onBackToList?: () => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => Promise<void> | void;
  qualityProfiles?: { id: string; name: string }[];
  defaultRootFolder?: string;
  onUpdateTitleTags?: (newTags: string[]) => Promise<void>;
  completedDownloads?: DownloadQueueItem[];
  onOpenManualImport?: (item: DownloadQueueItem) => void;
  initialEpisodeId?: string | null;
  seasonSearchResultsByCollection?: Record<string, Release[]>;
  seasonSearchLoadingByCollection?: Record<string, boolean>;
  onRunSeasonSearch?: (collection: TitleCollection) => Promise<void> | void;
  onQueueFromSeasonSearch?: (release: Release) => Promise<void> | void;
};

export function SeriesOverviewView({
  loading,
  title,
  collections,
  events,
  episodesByCollection,
  mediaFilesByEpisode,
  releaseBlocklistEntries,
  onTitleChanged,
  onBackToList,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  onAutoSearchEpisode,
  qualityProfiles,
  defaultRootFolder,
  onUpdateTitleTags,
  completedDownloads,
  onOpenManualImport,
  initialEpisodeId,
  seasonSearchResultsByCollection,
  seasonSearchLoadingByCollection,
  onRunSeasonSearch,
  onQueueFromSeasonSearch,
}: Props) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
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

  const handleLoadInterstitialMovieMetadata = React.useCallback((collectionId: string, candidates: string[]) => {
    if (
      episodePanel.interstitialMovieMetadataLoadedByCollection[collectionId] ||
      episodePanel.interstitialMovieMetadataLoadingByCollection[collectionId]
    ) {
      return;
    }

    const searchCandidates = dedupeInsensitive(
      candidates
        .map((candidate) => candidate.replace(/\s+/g, " "))
        .filter((candidate) => candidate.trim().length > 0)
        .map((candidate) => normalizeMovieCollectionLabel(candidate))
        .filter((candidate): candidate is string => candidate != null),
    );
    if (title?.name) {
      searchCandidates.push(title.name.trim());
    }
    const searchQuery = searchCandidates[0];
    if (!searchQuery) {
      dispatchEpisodePanel({ type: "SET_INTERSTITIAL_LOADED", collectionId });
      dispatchEpisodePanel({ type: "SET_INTERSTITIAL_METADATA", collectionId, metadata: null });
      return;
    }

    dispatchEpisodePanel({ type: "SET_INTERSTITIAL_LOADING", collectionId, loading: true });
    const metadataLanguage = title?.metadataLanguage?.trim() || "eng";
    const query = searchQuery;

    client
      .query(searchMetadataQuery, {
        query,
        type: "movie",
        limit: 6,
        language: metadataLanguage,
      })
      .toPromise()
      .then((result) => {
        if (result.error) {
          throw result.error;
        }
        const found = result.data?.searchMetadata?.[0] ?? null;
        dispatchEpisodePanel({ type: "SET_INTERSTITIAL_METADATA", collectionId, metadata: found });
      })
      .catch(() => {
        dispatchEpisodePanel({ type: "SET_INTERSTITIAL_METADATA", collectionId, metadata: null });
      })
      .finally(() => {
        dispatchEpisodePanel({ type: "SET_INTERSTITIAL_LOADING", collectionId, loading: false });
        dispatchEpisodePanel({ type: "SET_INTERSTITIAL_LOADED", collectionId });
      });
  }, [title, episodePanel.interstitialMovieMetadataLoadedByCollection, episodePanel.interstitialMovieMetadataLoadingByCollection, client]);

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
        <button
          type="button"
          className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
          onClick={() => onBackToList?.()}
        >
          <ArrowLeft className="h-4 w-4" /> Back to {t("nav.series")}
        </button>
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
      <button
        type="button"
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        onClick={() => onBackToList?.()}
      >
        <ArrowLeft className="h-4 w-4" /> Back to {t("nav.series")}
      </button>

      <Card>
        <CardContent className="p-4">
          <div className="flex gap-5">
            <div className="shrink-0">
              {title.posterUrl ? (
                <img
                  src={title.posterUrl}
                  alt={title.name}
                  className="h-auto w-[180px] rounded-lg object-cover shadow-lg block"
                />
              ) : (
                <div className="flex h-[270px] w-[180px] items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60">
                  No Poster
                </div>
              )}
            </div>

            <div className="min-w-0 flex-1">
              <h1 className="text-2xl font-bold text-foreground">
                {title.name}
                {title.year ? (
                  <span className="ml-2 text-lg font-normal text-muted-foreground">
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
                <p className="mt-4 text-sm leading-relaxed text-muted-foreground">
                  {title.overview}
                </p>
              ) : null}

              <p className="mt-2 text-right text-xs text-muted-foreground/60">
                Added {formatDate(title.createdAt)}
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle className="flex items-center gap-2 text-base">
              <FolderOpen className="h-4 w-4" />
              Seasons and Episodes
            </CardTitle>
            {onOpenManualImport && completedDownloads && completedDownloads.length > 0 && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => onOpenManualImport(completedDownloads[0])}
              >
                <FileInput className="mr-1.5 h-4 w-4" />
                Manual Import
              </Button>
            )}
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {sortedCollections.length > 0 ? (
            sortedCollections.map((collection) => {
              const key = `s-${collection.id}`;
              const sortedEpisodes = [
                ...(episodesByCollection[collection.id] ?? []),
              ].sort((left, right) => episodeSortValue(right) - episodeSortValue(left));

              return (
                <SeasonSection
                  key={key}
                  collection={collection}
                  episodes={sortedEpisodes}
                  titleName={title.name}
                  facet={title.facet}
                  expanded={expandedKeys.has(key)}
                  onToggle={() => toggleKey(key)}
                  onLoadInterstitialMovieMetadata={handleLoadInterstitialMovieMetadata}
                  interstitialMovieMetadata={episodePanel.interstitialMovieMetadataByCollection[collection.id] ?? null}
                  interstitialMovieMetadataLoaded={episodePanel.interstitialMovieMetadataLoadedByCollection[collection.id] ?? false}
                  interstitialMovieMetadataLoading={episodePanel.interstitialMovieMetadataLoadingByCollection[collection.id] ?? false}
                  expandedEpisodeRows={episodePanel.expandedEpisodeRows}
                  episodeActiveTab={episodePanel.episodeActiveTab}
                  mediaFilesByEpisode={mediaFilesByEpisode}
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
                />
              );
            })
          ) : (
            <p className="text-sm text-muted-foreground">
              No seasons are tracked for this show yet.
            </p>
          )}
        </CardContent>
      </Card>

      {onUpdateTitleTags && qualityProfiles && defaultRootFolder ? (
        <details className="rounded-xl border border-border bg-card text-card-foreground overflow-hidden">
          <summary className="cursor-pointer select-none px-4 py-3 text-sm font-medium text-card-foreground">
            <span className="inline-flex items-center gap-2">
              <Settings2 className="h-4 w-4" />
              {t("title.settings")}
            </span>
          </summary>
          <div className="border-t border-border">
            <TitleSettingsPanel
              title={title}
              qualityProfiles={qualityProfiles}
              defaultRootFolder={defaultRootFolder}
              onUpdateTitleTags={onUpdateTitleTags}
            />
          </div>
        </details>
      ) : null}

      {events.length > 0 ? (
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
      ) : null}
    </div>
  );
}
