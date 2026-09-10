import { Fragment } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import { ConvergenceBadge } from "@/components/views/convergence-badge";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableActionsCell,
  TableActionsHead,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Loader2, Search, Zap } from "lucide-react";
import { Link } from "react-router";
import { useTranslate } from "@/lib/context/translate-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { buildOverviewDetailPath } from "@/lib/utils/routing";
import type {
  AcquisitionSearchJob,
  ConvergenceState,
  Facet,
  LibraryRecord,
  Release,
} from "@/lib/types";
import type { ViewId } from "@/components/root/types";

export type CutoffUnmetItem = {
  titleId: string;
  titleName: string;
  titleSlug?: string | null;
  titleFacet: Facet;
  libraryId: string;
  libraryName?: string | null;
  librarySlug?: string | null;
  episodeId?: string | null;
  seasonNumber?: string | null;
  episodeNumber?: string | null;
  currentTier: string;
  targetTier: string;
  convergenceState: ConvergenceState;
  indexersCovered: number;
  indexersRouted: number;
};

type CutoffUnmetViewState = {
  items: CutoffUnmetItem[];
  total: number;
  offset: number;
  setOffset: (v: number) => void;
  limit: number;
  loading: boolean;
  facetFilter: string | undefined;
  setFacetFilter: (v: string | undefined) => void;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  setSelectedLibraryIds: (value: string[]) => void;
  autoSearchingId: string | null;
  interactiveSearchingId: string | null;
  activeInteractiveItemId: string | null;
  searchResultsByItemId: Record<string, Release[]>;
  searchJob: AcquisitionSearchJob | null;
  searchJobStarting: boolean;
  triggerAutoSearch: (item: CutoffUnmetItem) => Promise<void>;
  triggerInteractiveSearch: (item: CutoffUnmetItem) => Promise<void>;
  queueRelease: (item: CutoffUnmetItem, release: Release) => Promise<void>;
  triggerBulkSearch: () => void;
  cancelBulkSearch: () => Promise<void>;
};

function cutoffItemKey(item: CutoffUnmetItem) {
  return item.episodeId?.trim() || item.titleId;
}

function cutoffEpisodeCode(item: CutoffUnmetItem): string | null {
  const seasonDigits = item.seasonNumber?.match(/\d+/)?.[0] ?? null;
  const episodeDigits = item.episodeNumber?.match(/\d+/)?.[0] ?? null;
  if (!seasonDigits || !episodeDigits) {
    return null;
  }
  return `S${seasonDigits.padStart(2, "0")}E${episodeDigits.padStart(2, "0")}`;
}

function cutoffOverviewView(facet: Facet): ViewId | null {
  switch (facet) {
    case "MOVIE":
      return "movies";
    case "SERIES":
      return "series";
    case "ANIME":
      return "anime";
    default:
      return null;
  }
}

function cutoffOverviewHref(item: CutoffUnmetItem, includeEpisode: boolean): string | null {
  const targetView = cutoffOverviewView(item.titleFacet);
  if (!targetView) {
    return null;
  }

  const normalizedSlug = item.titleSlug?.trim() || null;
  const targetPath = buildOverviewDetailPath(
    targetView,
    item.librarySlug ?? null,
    normalizedSlug,
  );
  const params = new URLSearchParams();
  if (!normalizedSlug || !item.librarySlug?.trim()) {
    params.set("id", item.titleId);
  }
  if (includeEpisode && item.episodeId) {
    params.set("episodeId", item.episodeId);
  }

  const query = params.toString();
  return `${targetPath}${query ? `?${query}` : ""}`;
}

function qualityBadge(tier: string, variant: "current" | "target") {
  const cls =
    variant === "current"
      ? "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]"
      : "bg-[var(--scry-success-bg-strong)] text-[var(--scry-success-text)]";
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${cls}`}
    >
      {tier}
    </span>
  );
}

function TitleCell({ item }: { item: CutoffUnmetItem }) {
  const titleHref = cutoffOverviewHref(item, false);
  const episodeHref = cutoffOverviewHref(item, true);
  const episodeCode = cutoffEpisodeCode(item);

  return (
    <div className="space-y-1">
      {titleHref ? (
        <Link
          to={titleHref}
          className="font-medium text-foreground underline-offset-4 hover:underline"
        >
          {item.titleName}
        </Link>
      ) : (
        <span className="font-medium text-foreground">{item.titleName}</span>
      )}
      {episodeCode ? (
        episodeHref ? (
          <Link
            to={episodeHref}
            className="block text-sm text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          >
            {episodeCode}
          </Link>
        ) : (
          <span className="block text-sm text-muted-foreground">{episodeCode}</span>
        )
      ) : null}
    </div>
  );
}

function ActionButtons({
  item,
  autoSearchingId,
  interactiveSearchingId,
  bulkSearching,
  triggerAutoSearch,
  triggerInteractiveSearch,
}: {
  item: CutoffUnmetItem;
  autoSearchingId: string | null;
  interactiveSearchingId: string | null;
  bulkSearching: boolean;
  triggerAutoSearch: (item: CutoffUnmetItem) => Promise<void>;
  triggerInteractiveSearch: (item: CutoffUnmetItem) => Promise<void>;
}) {
  const t = useTranslate();
  const itemKey = cutoffItemKey(item);
  const autoSearching = autoSearchingId === itemKey;
  const interactiveSearching = interactiveSearchingId === itemKey;

  return (
    <div className="flex flex-wrap gap-2">
      <Button
        size="sm"
        disabled={autoSearching || interactiveSearching || bulkSearching}
        onClick={() => void triggerAutoSearch(item)}
      >
        {autoSearching ? (
          <Loader2 className="mr-1 h-4 w-4 animate-spin" />
        ) : (
          <Zap className="mr-1 h-4 w-4" />
        )}
        {t("label.autoSearch")}
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={autoSearching || interactiveSearching || bulkSearching}
        onClick={() => void triggerInteractiveSearch(item)}
      >
        {interactiveSearching ? (
          <Loader2 className="mr-1 h-4 w-4 animate-spin" />
        ) : (
          <Search className="mr-1 h-4 w-4" />
        )}
        {t("label.interactiveSearch")}
      </Button>
    </div>
  );
}

export function CutoffUnmetView({ state }: { state: CutoffUnmetViewState }) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const {
    items,
    total,
    offset,
    setOffset,
    limit,
    loading,
    facetFilter,
    setFacetFilter,
    libraries,
    librariesLoading,
    selectedLibraryIds,
    setSelectedLibraryIds,
    autoSearchingId,
    interactiveSearchingId,
    activeInteractiveItemId,
    searchResultsByItemId,
    searchJob,
    searchJobStarting,
    triggerAutoSearch,
    triggerInteractiveSearch,
    queueRelease,
    triggerBulkSearch,
    cancelBulkSearch,
  } = state;

  const jobRunning = searchJob?.state === "RUNNING";
  const bulkSearching = jobRunning || searchJobStarting;
  const hasPrev = offset > 0;
  const hasNext = offset + limit < total;

  return (
    <Card className="overflow-hidden rounded-none border-0 bg-transparent shadow-none">
      <CardHeader className="px-4 pt-4 pb-0 sm:px-5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-end">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            {jobRunning && searchJob ? (
              <>
                <div className="w-full min-w-[220px] sm:w-64">
                  <ActivityProgressBar
                    percent={
                      searchJob.total > 0
                        ? Math.round((searchJob.processed / searchJob.total) * 100)
                        : 0
                    }
                    remainingLabel={
                      searchJob.currentTitle ??
                      t("cutoff.searchProgress", {
                        current: searchJob.processed,
                        total: searchJob.total,
                      })
                    }
                    colorClass="bg-[var(--scry-accent)]"
                    indeterminate={searchJob.total === 0}
                  />
                </div>
                <Button
                  size="sm"
                  variant="destructive"
                  className="w-full sm:w-auto"
                  onClick={() => void cancelBulkSearch()}
                >
                  {t("label.cancel")}
                </Button>
              </>
            ) : (
              <Button
                size="sm"
                className="w-full sm:w-auto"
                onClick={triggerBulkSearch}
                disabled={items.length === 0 || loading || searchJobStarting}
              >
                <Search className="mr-1 h-3 w-3" />
                {t("cutoff.searchAll")}
              </Button>
            )}
          </div>
        </div>
      </CardHeader>
      <CardContent className="p-4 sm:p-5">
        <div className="mb-4 flex flex-col gap-3 rounded-[14px] border border-[var(--scry-border3)] bg-[var(--scry-surfC)] p-3 sm:flex-row sm:flex-wrap sm:items-center">
          <LibraryMultiSelect
            libraries={libraries}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={setSelectedLibraryIds}
            disabled={librariesLoading}
            triggerClassName="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[13px] text-[var(--scry-body)] shadow-none sm:w-[210px]"
          />

          <Select
            value={facetFilter ?? "__all__"}
            onValueChange={(value) =>
              setFacetFilter(value === "__all__" ? undefined : value)
            }
          >
            <SelectTrigger className="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[13px] text-[var(--scry-body)] shadow-none sm:w-[150px]">
              <SelectValue placeholder={t("cutoff.filterFacet")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t("cutoff.allFacets")}</SelectItem>
              <SelectItem value="MOVIE">movie</SelectItem>
              <SelectItem value="SERIES">series</SelectItem>
              <SelectItem value="ANIME">anime</SelectItem>
            </SelectContent>
          </Select>

          <span className="self-center text-sm font-medium text-[var(--scry-muted3)] sm:ml-auto">
            {t("cutoff.totalCount", { count: total })}
          </span>
        </div>

        {isMobile ? (
          items.length === 0 && !loading ? (
            <p className="text-center text-[var(--scry-muted3)]">{t("cutoff.noItems")}</p>
          ) : (
            <div className="space-y-3">
              {items.map((item) => {
                const itemKey = cutoffItemKey(item);
                const searchResults = searchResultsByItemId[itemKey] ?? [];
                const showResults = activeInteractiveItemId === itemKey;

                return (
                  <div
                    key={itemKey}
                    className="space-y-3 rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)] p-3 shadow-[0_12px_28px_rgba(2,6,23,0.10)]"
                  >
                    <div className="space-y-3">
                      <TitleCell item={item} />
                      <span className="text-xs text-muted-foreground">
                        {item.libraryName ?? item.libraryId}
                      </span>
                      <div className="flex flex-wrap gap-2">
                        {qualityBadge(item.currentTier, "current")}
                        {qualityBadge(item.targetTier, "target")}
                        <ConvergenceBadge
                          state={item.convergenceState}
                          indexersCovered={item.indexersCovered}
                          indexersRouted={item.indexersRouted}
                        />
                      </div>
                      <ActionButtons
                        item={item}
                        autoSearchingId={autoSearchingId}
                        interactiveSearchingId={interactiveSearchingId}
                        bulkSearching={bulkSearching}
                        triggerAutoSearch={triggerAutoSearch}
                        triggerInteractiveSearch={triggerInteractiveSearch}
                      />
                    </div>
                    {showResults ? (
                      <SearchResultBuckets
                        results={searchResults}
                        onQueue={(release) => queueRelease(item, release)}
                        requireCandidateToken
                      />
                    ) : null}
                  </div>
                );
              })}
            </div>
          )
        ) : (
          <div className="overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]">
            <Table overflow="clip" layout="fixed" density="dense">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("cutoff.colTitleEpisode")}</TableHead>
                  <TableHead className="w-36 text-center">Library</TableHead>
                  <TableHead className="w-36 text-center">{t("cutoff.colCurrentQuality")}</TableHead>
                  <TableHead className="w-36 text-center">{t("cutoff.colTargetQuality")}</TableHead>
                  <TableHead className="w-40 text-center">{t("wanted.colConvergence")}</TableHead>
                  <TableActionsHead className="w-64">{t("label.actions")}</TableActionsHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((item) => {
                  const itemKey = cutoffItemKey(item);
                  const searchResults = searchResultsByItemId[itemKey] ?? [];
                  const showResults = activeInteractiveItemId === itemKey;

                  return (
                    <Fragment key={itemKey}>
                      <TableRow>
                        <TableCell className="align-top">
                          <TitleCell item={item} />
                        </TableCell>
                        <TableCell className="align-top text-center text-sm text-muted-foreground">
                          {item.libraryName ?? item.libraryId}
                        </TableCell>
                        <TableCell className="align-top text-center">
                          {qualityBadge(item.currentTier, "current")}
                        </TableCell>
                        <TableCell className="align-top text-center">
                          {qualityBadge(item.targetTier, "target")}
                        </TableCell>
                        <TableCell className="align-top text-center">
                          <ConvergenceBadge
                            state={item.convergenceState}
                            indexersCovered={item.indexersCovered}
                            indexersRouted={item.indexersRouted}
                          />
                        </TableCell>
                        <TableActionsCell className="w-64 align-top">
                          <ActionButtons
                            item={item}
                            autoSearchingId={autoSearchingId}
                            interactiveSearchingId={interactiveSearchingId}
                            bulkSearching={bulkSearching}
                            triggerAutoSearch={triggerAutoSearch}
                            triggerInteractiveSearch={triggerInteractiveSearch}
                          />
                        </TableActionsCell>
                      </TableRow>
                      {showResults ? (
                        <TableRow>
                          <TableCell colSpan={6} className="bg-background/20">
                            <SearchResultBuckets
                              results={searchResults}
                              onQueue={(release) => queueRelease(item, release)}
                              requireCandidateToken
                            />
                          </TableCell>
                        </TableRow>
                      ) : null}
                    </Fragment>
                  );
                })}
                {items.length === 0 && !loading ? (
                  <TableRow>
                    <TableCell colSpan={6} className="text-center text-muted-foreground">
                      {t("cutoff.noItems")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </div>
        )}

        {total > limit && (
          <div className="mt-4 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <Button
              className="w-full sm:w-auto"
              size="sm"
              variant="outline"
              disabled={!hasPrev}
              onClick={() => setOffset(Math.max(0, offset - limit))}
            >
              {t("wanted.prev")}
            </Button>
            <span className="text-sm text-muted-foreground">
              {offset + 1}–{Math.min(offset + limit, total)} / {total}
            </span>
            <Button
              className="w-full sm:w-auto"
              size="sm"
              variant="outline"
              disabled={!hasNext}
              onClick={() => setOffset(offset + limit)}
            >
              {t("wanted.next")}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
