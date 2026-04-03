import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import type { ViewId } from "@/components/root/types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Release, TitleRecord } from "@/lib/types";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { TitlePoster } from "@/components/title-poster";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

type Facet = "movie" | "tv" | "anime";
type TvdbSearchItem = MetadataTvdbSearchItem;

type AddTitleFormProps = {
  titleNameForQueue: string;
  setTitleNameForQueue: (value: string) => void;
  queueFacet: Facet;
  setQueueFacet: (value: Facet) => void;
  monitoredForQueue: boolean;
  setMonitoredForQueue: (value: boolean) => void;
  seasonFoldersForQueue: boolean;
  setSeasonFoldersForQueue: (value: boolean) => void;
  minAvailabilityForQueue: string;
  setMinAvailabilityForQueue: (value: string) => void;
  onAddSubmit: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  tvdbCandidates: TvdbSearchItem[];
  selectedTvdbId: string | null;
  selectTvdbCandidate: (candidate: TvdbSearchItem) => void;
  addTvdbCandidateToCatalog: (candidate: TvdbSearchItem) => Promise<void> | void;
  searchNzbForSelectedTvdb: () => Promise<void>;
  selectedTvdb: TvdbSearchItem | null;
  searchResults: Release[];
  queueFromSearch: (release: Release) => Promise<void> | void;
  titleFilter: string;
  onTitleFilterChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  onRefreshTitles: () => void;
  titleLoading: boolean;
  titleStatus: string;
  monitoredTitles: TitleRecord[];
  onOpenOverview: (targetView: ViewId, titleId: string) => void;
  queueExisting: (title: TitleRecord) => Promise<void> | void;
};

export function AddTitleForm({
  titleNameForQueue,
  setTitleNameForQueue,
  queueFacet,
  setQueueFacet,
  monitoredForQueue,
  setMonitoredForQueue,
  seasonFoldersForQueue,
  setSeasonFoldersForQueue,
  minAvailabilityForQueue,
  setMinAvailabilityForQueue,
  onAddSubmit,
  tvdbCandidates,
  selectedTvdbId,
  selectTvdbCandidate,
  addTvdbCandidateToCatalog,
  searchNzbForSelectedTvdb,
  selectedTvdb,
  searchResults,
  queueFromSearch,
  titleFilter,
  onTitleFilterChange,
  onRefreshTitles,
  titleLoading,
  titleStatus,
  monitoredTitles,
  onOpenOverview,
  queueExisting,
}: AddTitleFormProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const handleTitleNameChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setTitleNameForQueue(event.target.value);
    },
    [setTitleNameForQueue],
  );

  const handleQueueFacetChange = React.useCallback(
    (value: string) => {
      setQueueFacet(value as Facet);
    },
    [setQueueFacet],
  );

  const handleSelectTvdbCandidate = React.useCallback(
    (candidate: TvdbSearchItem) => {
      selectTvdbCandidate(candidate);
    },
    [selectTvdbCandidate],
  );

  const handleAddTvdbToCatalog = React.useCallback(
    (candidate: TvdbSearchItem) => {
      void addTvdbCandidateToCatalog(candidate);
    },
    [addTvdbCandidateToCatalog],
  );

  const handleQueueFromSearch = React.useCallback(
    (release: Release) => {
      return Promise.resolve(queueFromSearch(release));
    },
    [queueFromSearch],
  );

  const handleSearchNzbForSelectedTvdb = React.useCallback(() => {
    void searchNzbForSelectedTvdb();
  }, [searchNzbForSelectedTvdb]);

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>{t("title.addAndQueue")}</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="grid gap-4 lg:grid-cols-5" onSubmit={onAddSubmit}>
            <label className="lg:col-span-3">
              <Label className="mb-2 block">{t("title.name")}</Label>
              <Input
                name="titleName"
                placeholder={t("title.namePlaceholder")}
                value={titleNameForQueue}
                onChange={handleTitleNameChange}
                required
              />
            </label>
            <label>
              <Label className="mb-2 block">{t("title.facet")}</Label>
              <Select value={queueFacet} onValueChange={handleQueueFacetChange}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="movie">{t("search.facetMovie")}</SelectItem>
                  <SelectItem value="tv">{t("search.facetTv")}</SelectItem>
                  <SelectItem value="anime">{t("search.facetAnime")}</SelectItem>
                </SelectContent>
              </Select>
            </label>
            <label className="flex items-start gap-2 pt-0 sm:items-center sm:pt-7">
              <Checkbox
                checked={monitoredForQueue}
                onCheckedChange={(checked) =>
                  setMonitoredForQueue(checked === true)
                }
              />
              <span className="text-sm">{t("title.monitored")}</span>
            </label>
            {queueFacet === "movie" && (
              <label>
                <Label className="mb-2 block">{t("settings.minAvailabilityLabel")}</Label>
                <Select value={minAvailabilityForQueue} onValueChange={setMinAvailabilityForQueue}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="announced">{t("settings.minAvailability.announced")}</SelectItem>
                    <SelectItem value="in_cinemas">{t("settings.minAvailability.in_cinemas")}</SelectItem>
                    <SelectItem value="released">{t("settings.minAvailability.released")}</SelectItem>
                  </SelectContent>
                </Select>
              </label>
            )}
            {queueFacet !== "movie" && (
              <label className="flex items-start gap-2 pt-0 sm:items-center sm:pt-7">
                <Checkbox
                  checked={seasonFoldersForQueue}
                  onCheckedChange={(checked) =>
                    setSeasonFoldersForQueue(checked === true)
                  }
                />
                <span className="text-sm">{t("search.addConfigSeasonFolder")}</span>
              </label>
            )}
            <div className="flex justify-end lg:col-span-5">
              <Button type="submit" className="w-full sm:w-auto">{t("tvdb.searchByTvdb")}</Button>
            </div>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("tvdb.searchResults")}</CardTitle>
        </CardHeader>
        <CardContent>
          {tvdbCandidates.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("tvdb.searchPrompt")}</p>
          ) : (
            <div className="space-y-2">
              {tvdbCandidates.map((result) => (
                <div
                  key={`${result.tvdbId}-${result.name}`}
                  className="rounded-lg border border-border p-3"
                >
                  <div className="mb-2 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                    <div className="flex min-h-20 gap-3">
                      <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                        {result.posterUrl ? (
                          <TitlePoster
                            src={result.posterUrl}
                            alt={t("media.posterAlt", { name: result.name })}
                            className="h-full w-full object-cover"
                            loading="lazy"
                          />
                        ) : (
                          <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground">
                            {t("label.noArt")}
                          </div>
                        )}
                      </div>
                      <div className="min-w-0">
                        <p className="text-sm font-medium text-foreground">{result.name}</p>
                        <p className="text-xs text-muted-foreground">
                          {result.type || t("label.unknownType")} • {result.year ? result.year : t("label.yearUnknown")} •{" "}
                          {result.sortTitle || result.slug || t("label.unknown")}
                        </p>
                        {result.overview ? (
                          <p className="mt-2 text-xs text-muted-foreground line-clamp-2">
                            {result.overview}
                          </p>
                        ) : null}
                      </div>
                    </div>
                    <div className="flex flex-col gap-2 sm:items-end">
                      <Button
                        size="sm"
                        className="w-full sm:w-auto"
                        variant={String(result.tvdbId) === selectedTvdbId ? "secondary" : "ghost"}
                        onClick={() => handleSelectTvdbCandidate(result)}
                      >
                        {t("tvdb.select")}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        className="w-full sm:w-auto"
                        onClick={() => handleAddTvdbToCatalog(result)}
                      >
                        {t("title.addToCatalog")}
                      </Button>
                    </div>
                  </div>
                </div>
              ))}
              <div className="pt-2">
                <Button
                  type="button"
                  className="w-full sm:w-auto"
                  onClick={handleSearchNzbForSelectedTvdb}
                  disabled={!selectedTvdbId}
                >
                  {t("tvdb.searchButton")}
                </Button>
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>
            {selectedTvdb ? t("nzb.searchResultsFor", { name: selectedTvdb.name }) : t("nzb.searchResults")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {searchResults.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {selectedTvdb ? t("nzb.noResultsYet") : t("tvdb.selectPrompt")}
            </p>
          ) : (
            <SearchResultBuckets
              results={searchResults}
              onQueue={handleQueueFromSearch}
            />
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>
            {t("title.monitoredSection", {
              facet: t("search.facetAnime"),
            })}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="mb-3 flex flex-col gap-2 sm:flex-row">
            <Input
              placeholder={t("title.filterPlaceholder")}
              value={titleFilter}
              onChange={onTitleFilterChange}
            />
            <Button className="w-full sm:w-auto" variant="primary" onClick={onRefreshTitles} disabled={titleLoading}>
              {t("label.refresh")}
            </Button>
          </div>
          <p className="mb-2 text-sm text-muted-foreground">{titleStatus}</p>
          {isMobile ? (
            <div className="space-y-2">
              {monitoredTitles.map((item) => {
                const overviewTargetView = item.facet === "movie"
                  ? "movies"
                  : item.facet === "tv"
                    ? "series"
                    : item.facet === "anime"
                      ? "anime"
                      : null;
                return (
                  <div key={item.id} className="rounded-lg border border-border p-3">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        {overviewTargetView ? (
                          <button
                            type="button"
                            onClick={() => onOpenOverview(overviewTargetView, item.id)}
                            className="block text-left text-sm font-medium text-foreground hover:underline"
                          >
                            {item.name}
                          </button>
                        ) : (
                          <p className="text-sm font-medium text-foreground">{item.name}</p>
                        )}
                        <div className="mt-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
                          <span className="rounded bg-muted px-2 py-0.5 capitalize">{item.facet}</span>
                          <span className="rounded bg-muted px-2 py-0.5">
                            {item.monitored ? t("label.yes") : t("label.no")}
                          </span>
                        </div>
                      </div>
                      <Button variant="secondary" size="sm" className="shrink-0" onClick={() => queueExisting(item)}>
                        {t("title.queueLatest")}
                      </Button>
                    </div>
                  </div>
                );
              })}
              {monitoredTitles.length === 0 && !titleLoading ? (
                <p className="text-sm text-muted-foreground">{t("title.noManaged")}</p>
              ) : null}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("label.name")}</TableHead>
                  <TableHead>{t("title.table.facet")}</TableHead>
                  <TableHead>{t("title.table.monitored")}</TableHead>
                  <TableHead className="text-right">{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {monitoredTitles.map((item) => {
                  const overviewTargetView = item.facet === "movie"
                    ? "movies"
                    : item.facet === "tv"
                      ? "series"
                      : item.facet === "anime"
                        ? "anime"
                        : null;
                  return (
                    <TableRow key={item.id}>
                      <TableCell>
                        {overviewTargetView ? (
                          <button
                            type="button"
                            onClick={() => onOpenOverview(overviewTargetView, item.id)}
                            className="hover:text-foreground hover:underline"
                          >
                            {item.name}
                          </button>
                        ) : (
                          item.name
                        )}
                      </TableCell>
                      <TableCell>{item.facet}</TableCell>
                      <TableCell>{item.monitored ? t("label.yes") : t("label.no")}</TableCell>
                      <TableCell className="text-right">
                        <Button variant="ghost" size="sm" onClick={() => queueExisting(item)}>
                          {t("title.queueLatest")}
                        </Button>
                      </TableCell>
                    </TableRow>
                  );
                })}
                {monitoredTitles.length === 0 && !titleLoading ? (
                  <TableRow>
                    <TableCell colSpan={4} className="text-muted-foreground">
                      {t("title.noManaged")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </>
  );
}
