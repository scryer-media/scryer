import * as React from "react";
import { Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import { useTranslate } from "@/lib/context/translate-context";
import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";
import { deriveInteractiveSearchPresentation } from "@/lib/utils/interactive-search-presentation";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import { selectorId } from "@/lib/utils/dom-ids";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { EpisodePanelTab } from "./episode-panel-reducer";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import { blocklistEntryMatchesEpisode } from "./helpers";
import { EpisodeDetailsPanel } from "./episode-details-panel";
import { EpisodeBlocklistPanel } from "./episode-blocklist-panel";
import { EMPTY_BLOCKLIST_ENTRIES } from "./season-section-utils";
import { LoadingMark } from "@/components/common/loading-mark";

export type EpisodePanelContentProps = {
  activeTab: EpisodePanelTab;
  canClearBlocklistEntries: boolean;
  collection: TitleCollection;
  clearingReleaseBlocklistEntryId?: string | null;
  episode: CollectionEpisode;
  episodeFiles: EpisodeMediaFile[];
  episodeIndexerProgress: InteractiveSearchIndexerProgress[];
  episodeLoading: boolean;
  episodeResults: Release[];
  facet: string;
  hasSearchResults: boolean;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
  onQueueFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onQueueAdditionalFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onRefreshSubtitles?: () => Promise<void> | void;
  onRunEpisodeSearch?: (episode: CollectionEpisode) => void;
  onTabChange: (tab: EpisodePanelTab | "history") => void;
  showHistoryTab: boolean;
  showSearchTab: boolean;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  searchBlocked: boolean;
  subtitleDownloads: ExternalSubtitleRecord[];
  primaryMovieFileUpdatingId?: string | null;
};

export const EpisodePanelContent = React.memo(function EpisodePanelContent({
  activeTab,
  canClearBlocklistEntries,
  collection,
  clearingReleaseBlocklistEntryId,
  episode,
  episodeFiles,
  episodeIndexerProgress,
  episodeLoading,
  episodeResults,
  facet,
  hasSearchResults,
  onClearReleaseBlocklistEntry,
  onDeleteFile,
  onMakePrimaryFile,
  onQueueFromEpisodeSearch,
  onQueueAdditionalFromEpisodeSearch,
  onRefreshSubtitles,
  onRunEpisodeSearch,
  onTabChange,
  showHistoryTab,
  showSearchTab,
  releaseBlocklistEntries,
  searchBlocked,
  subtitleDownloads,
  primaryMovieFileUpdatingId = null,
}: EpisodePanelContentProps) {
  const t = useTranslate();
  const searchPresentation = React.useMemo(
    () =>
      deriveInteractiveSearchPresentation({
        hasSnapshot: hasSearchResults,
        loading: episodeLoading,
        resultCount: episodeResults.length,
        indexers: episodeIndexerProgress,
      }),
    [episodeIndexerProgress, episodeLoading, episodeResults.length, hasSearchResults],
  );
  const searchDescription = React.useMemo(() => {
    if (searchPresentation.showProgress) {
      return t("title.contextReleaseSearchProgress", {
        releaseCount: episodeResults.length,
        done: searchPresentation.completedIndexerCount,
        total: searchPresentation.totalIndexerCount,
      });
    }

    // Report what the run did, not which sources happened to return
    // results: an indexer that answered with nothing still searched, and one
    // that failed or was skipped is called out on its own line below.
    if (searchPresentation.totalIndexerCount > 0) {
      return t("title.contextReleaseSearchSummaryDetailed", {
        releaseCount: episodeResults.length,
        searched: searchPresentation.searchedIndexerCount,
        total: searchPresentation.totalIndexerCount,
      });
    }
    const sourceCount = new Set(
      episodeResults
        .map((release) => release.source?.trim())
        .filter((source): source is string => Boolean(source)),
    ).size;
    return t("title.contextReleaseSearchSummary", {
      releaseCount: episodeResults.length,
      indexerCount: sourceCount,
    });
  }, [episodeResults, searchPresentation, t]);
  const filteredBlocklistEntries = React.useMemo(() => {
    if (activeTab !== "blocklist") {
      return EMPTY_BLOCKLIST_ENTRIES;
    }

    return releaseBlocklistEntries.filter((entry) =>
      blocklistEntryMatchesEpisode(entry, episode, collection),
    );
  }, [activeTab, collection, episode, releaseBlocklistEntries]);

  return (
    <Tabs
      value={activeTab}
      onValueChange={(value) => onTabChange(value as EpisodePanelTab | "history")}
    >
      <TabsList className="flex w-full flex-nowrap overflow-x-auto">
        <TabsTrigger
          id={selectorId("series-overview-episode-tab", episode.id, "details")}
          value="details"
          className="shrink-0"
        >{t("episode.details")}</TabsTrigger>
        {showSearchTab ? (
          <TabsTrigger
            id={selectorId("series-overview-episode-tab", episode.id, "search")}
            value="search"
            className="shrink-0"
          >{t("episode.search")}</TabsTrigger>
        ) : null}
        {showHistoryTab ? (
          <TabsTrigger
            id={selectorId("series-overview-episode-tab", episode.id, "history")}
            value="history"
            className="shrink-0"
          >{t("history.title")}</TabsTrigger>
        ) : null}
        <TabsTrigger
          id={selectorId("series-overview-episode-tab", episode.id, "blocklist")}
          value="blocklist"
          className="shrink-0"
        >{t("episode.blocklist")}</TabsTrigger>
      </TabsList>
      <TabsContent value="details">
        <EpisodeDetailsPanel
          episode={episode}
          facet={facet}
          mediaFiles={episodeFiles}
          subtitleDownloads={subtitleDownloads}
          onRefreshSubtitles={onRefreshSubtitles}
          onDeleteFile={onDeleteFile}
          onMakePrimaryFile={onMakePrimaryFile}
          primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
        />
      </TabsContent>
      {showSearchTab ? (
        <TabsContent value="search">
          {searchBlocked ? (
            <TitleSearchDownloadClientNotice />
          ) : (
            <div className="mb-2 flex items-start gap-4">
              {searchPresentation.showProgress || searchPresentation.showFinalSummary ? (
                <div className="min-w-0 flex-1">
                  <p
                    className="flex items-center gap-1.5 truncate text-[11.5px] text-[var(--scry-faint)]"
                    data-ui="episode-release-search-summary"
                    data-search-state={searchPresentation.showProgress ? "searching" : "done"}
                  >
                    {searchPresentation.showProgress ? (
                      <LoadingMark className="h-3 w-3 shrink-0" label={t("label.searching")} />
                    ) : null}
                    <span className="truncate">{searchDescription}</span>
                  </p>
                  {searchPresentation.showFinalSummary &&
                  searchPresentation.failedIndexerNames.length > 0 ? (
                    <p className="mt-0.5 truncate text-[11.5px] text-[var(--scry-danger-text)]">
                      {t("title.contextReleaseSearchIndexerFailures", {
                        count: searchPresentation.failedIndexerNames.length,
                        names: searchPresentation.failedIndexerNames.join(", "),
                      })}
                    </p>
                  ) : null}
                  {searchPresentation.showFinalSummary &&
                  searchPresentation.skippedIndexers.length > 0 ? (
                    <p
                      className="mt-0.5 truncate text-[11.5px] text-[var(--scry-faint)]"
                      title={searchPresentation.skippedIndexers
                        .map((indexer) =>
                          indexer.reason ? `${indexer.name}: ${indexer.reason}` : indexer.name,
                        )
                        .join("\n")}
                    >
                      {t("title.contextReleaseSearchIndexerSkipped", {
                        count: searchPresentation.skippedIndexers.length,
                        names: searchPresentation.skippedIndexers
                          .map((indexer) => indexer.name)
                          .join(", "),
                      })}
                    </p>
                  ) : null}
                </div>
              ) : null}
              <Button
                id={selectorId("series-overview-episode-search-refresh", episode.id)}
                type="button"
                variant="ghost"
                size="sm"
                className="ml-auto"
                onClick={() => onRunEpisodeSearch?.(episode)}
                disabled={episodeLoading}
                aria-label={t("label.search")}
              >
                <Search className="h-4 w-4" />
                <span className="ml-1">
                  {episodeLoading ? t("label.searching") : t("label.refresh")}
                </span>
              </Button>
            </div>
          )}
          {searchBlocked ? null : searchPresentation.showInitialLoader ? (
            <div className="flex flex-col items-center justify-center gap-4 py-16">
              <LoadingMark className="h-10 w-10 text-[var(--scry-accent-text)]" />
              <p className="text-lg text-muted-foreground">{t("label.searching")}</p>
            </div>
          ) : !searchPresentation.showResults ? (
            <p className="text-sm text-muted-foreground">{t("nzb.noResultsYet")}</p>
          ) : (
            <SearchResultBuckets
              results={episodeResults}
              onQueue={(release) => onQueueFromEpisodeSearch?.(episode, release)}
              onQueueAdditional={(release) => onQueueAdditionalFromEpisodeSearch?.(episode, release)}
              canQueueAdditional={(release) =>
                releaseSupportsAdditionalFileQueue(release, facet)
              }
              requireCandidateToken
            />
          )}
        </TabsContent>
      ) : null}
      <TabsContent value="blocklist">
        <EpisodeBlocklistPanel
          entries={filteredBlocklistEntries}
          canClear={canClearBlocklistEntries}
          clearingEntryId={clearingReleaseBlocklistEntryId}
          onClear={onClearReleaseBlocklistEntry}
        />
      </TabsContent>
    </Tabs>
  );
});
