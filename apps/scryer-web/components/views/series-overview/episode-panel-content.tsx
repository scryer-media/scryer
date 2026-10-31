import * as React from "react";
import { Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import {
  ReleaseSearchLoader,
  ReleaseSearchSummary,
} from "@/components/common/release-search-status";
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

export type EpisodePanelContentProps = {
  activeTab: EpisodePanelTab;
  canClearBlocklistEntries: boolean;
  collection: TitleCollection;
  clearingReleaseBlocklistEntryId?: string | null;
  episode: CollectionEpisode;
  episodeFiles: EpisodeMediaFile[];
  facet: string;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
  onAssignFileToEpisode?: (fileId: string) => void;
  onRefreshSubtitles?: () => Promise<void> | void;
  onTabChange: (tab: EpisodePanelTab | "history") => void;
  showHistoryTab: boolean;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  subtitleDownloads: ExternalSubtitleRecord[];
  /** Library the episode's media files belong to; gates subtitle controls. */
  libraryId: string | null;
  primaryMovieFileUpdatingId?: string | null;
};

export const EpisodePanelContent = React.memo(function EpisodePanelContent({
  activeTab,
  canClearBlocklistEntries,
  collection,
  clearingReleaseBlocklistEntryId,
  episode,
  episodeFiles,
  facet,
  onClearReleaseBlocklistEntry,
  onDeleteFile,
  onMakePrimaryFile,
  onAssignFileToEpisode,
  onRefreshSubtitles,
  onTabChange,
  showHistoryTab,
  releaseBlocklistEntries,
  subtitleDownloads,
  libraryId,
  primaryMovieFileUpdatingId = null,
}: EpisodePanelContentProps) {
  const t = useTranslate();
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
      onValueChange={(value) =>
        onTabChange(value as EpisodePanelTab | "history")
      }
    >
      <TabsList className="flex w-full flex-nowrap overflow-x-auto">
        <TabsTrigger
          id={selectorId("series-overview-episode-tab", episode.id, "details")}
          value="details"
          className="shrink-0"
        >
          {t("episode.details")}
        </TabsTrigger>
        {showHistoryTab ? (
          <TabsTrigger
            id={selectorId(
              "series-overview-episode-tab",
              episode.id,
              "history",
            )}
            value="history"
            className="shrink-0"
          >
            {t("history.title")}
          </TabsTrigger>
        ) : null}
        <TabsTrigger
          id={selectorId(
            "series-overview-episode-tab",
            episode.id,
            "blocklist",
          )}
          value="blocklist"
          className="shrink-0"
        >
          {t("episode.blocklist")}
        </TabsTrigger>
      </TabsList>
      <TabsContent value="details">
        <EpisodeDetailsPanel
          episode={episode}
          facet={facet}
          mediaFiles={episodeFiles}
          subtitleDownloads={subtitleDownloads}
          libraryId={libraryId}
          onRefreshSubtitles={onRefreshSubtitles}
          onDeleteFile={onDeleteFile}
          onMakePrimaryFile={onMakePrimaryFile}
          onAssignFileToEpisode={onAssignFileToEpisode}
          primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
        />
      </TabsContent>
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

export type EpisodeSearchResultsProps = {
  collection: TitleCollection;
  episode: CollectionEpisode;
  episodeIndexerProgress: InteractiveSearchIndexerProgress[];
  episodeLoading: boolean;
  episodeResults: Release[];
  facet: string;
  hasSearchResults: boolean;
  onQueueFromEpisodeSearch?: (
    episode: CollectionEpisode,
    release: Release,
  ) => Promise<void> | void;
  onQueueAdditionalFromEpisodeSearch?: (
    episode: CollectionEpisode,
    release: Release,
  ) => Promise<void> | void;
  onRunEpisodeSearch?: (episode: CollectionEpisode) => void;
  searchBlocked: boolean;
};

/** An episode's interactive search progress and results. */
export function EpisodeSearchResults({
  collection,
  episode,
  episodeIndexerProgress,
  episodeLoading,
  episodeResults,
  facet,
  hasSearchResults,
  onQueueFromEpisodeSearch,
  onQueueAdditionalFromEpisodeSearch,
  onRunEpisodeSearch,
  searchBlocked,
}: EpisodeSearchResultsProps) {
  const t = useTranslate();
  const searchPresentation = React.useMemo(
    () =>
      deriveInteractiveSearchPresentation({
        hasSnapshot: hasSearchResults,
        loading: episodeLoading,
        resultCount: episodeResults.length,
        indexers: episodeIndexerProgress,
      }),
    [
      episodeIndexerProgress,
      episodeLoading,
      episodeResults.length,
      hasSearchResults,
    ],
  );
  return (
    <>
      {searchBlocked ? (
        <TitleSearchDownloadClientNotice />
      ) : (
        <div className="mb-2 flex items-start gap-4">
          <ReleaseSearchSummary
            presentation={searchPresentation}
            results={episodeResults}
            dataUi="episode-release-search-summary"
          />
          <Button
            id={selectorId(
              "series-overview-episode-search-refresh",
              episode.id,
            )}
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
        <ReleaseSearchLoader />
      ) : !searchPresentation.showResults ? (
        <p className="text-sm text-muted-foreground">{t("nzb.noResultsYet")}</p>
      ) : (
        <SearchResultBuckets
          titleId={collection.titleId}
          ownedScope={{ episodeId: episode.id }}
          results={episodeResults}
          onQueue={(release) => onQueueFromEpisodeSearch?.(episode, release)}
          onQueueAdditional={(release) =>
            onQueueAdditionalFromEpisodeSearch?.(episode, release)
          }
          canQueueAdditional={(release) =>
            releaseSupportsAdditionalFileQueue(release, facet)
          }
          requireCandidateToken
        />
      )}
    </>
  );
}
