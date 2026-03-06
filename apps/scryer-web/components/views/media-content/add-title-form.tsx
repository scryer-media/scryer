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
  monitorSpecialsForQueue: boolean;
  setMonitorSpecialsForQueue: (value: boolean) => void;
  interSeasonMoviesForQueue: boolean;
  setInterSeasonMoviesForQueue: (value: boolean) => void;
  preferredSubGroupForQueue: string;
  setPreferredSubGroupForQueue: (value: string) => void;
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
  monitorSpecialsForQueue,
  setMonitorSpecialsForQueue,
  interSeasonMoviesForQueue,
  setInterSeasonMoviesForQueue,
  preferredSubGroupForQueue,
  setPreferredSubGroupForQueue,
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
          <form className="grid gap-4 md:grid-cols-5" onSubmit={onAddSubmit}>
            <label className="md:col-span-3">
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
            <label className="flex items-center gap-2 pt-7">
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
              <label className="flex items-center gap-2 pt-7">
                <Checkbox
                  checked={seasonFoldersForQueue}
                  onCheckedChange={(checked) =>
                    setSeasonFoldersForQueue(checked === true)
                  }
                />
                <span className="text-sm">{t("search.addConfigSeasonFolder")}</span>
              </label>
            )}
            {queueFacet === "anime" && (
              <>
                <label className="flex items-center gap-2 pt-7">
                  <Checkbox
                    checked={monitorSpecialsForQueue}
                    onCheckedChange={(checked) =>
                      setMonitorSpecialsForQueue(checked === true)
                    }
                  />
                  <span className="text-sm">{t("settings.monitorSpecialsLabel")}</span>
                </label>
                <label className="flex items-center gap-2 pt-7">
                  <Checkbox
                    checked={interSeasonMoviesForQueue}
                    onCheckedChange={(checked) =>
                      setInterSeasonMoviesForQueue(checked === true)
                    }
                  />
                  <span className="text-sm">{t("settings.interSeasonMoviesLabel")}</span>
                </label>
                <label className="md:col-span-2">
                  <Label className="mb-2 block">{t("settings.preferredSubGroupLabel")}</Label>
                  <Input
                    value={preferredSubGroupForQueue}
                    onChange={(e) => setPreferredSubGroupForQueue(e.target.value)}
                    placeholder={t("settings.preferredSubGroupPlaceholder")}
                  />
                </label>
              </>
            )}
            <div className="md:col-span-5 flex justify-end">
              <Button type="submit">{t("tvdb.searchByTvdb")}</Button>
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
                  <div className="mb-2 flex items-start justify-between gap-3">
                    <div className="flex min-h-20 gap-3">
                      <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                        {result.posterUrl ? (
                          <img
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
                      <div>
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
                    <div className="flex flex-col items-end gap-2">
                      <Button
                        size="sm"
                        variant={String(result.tvdbId) === selectedTvdbId ? "secondary" : "ghost"}
                        onClick={() => handleSelectTvdbCandidate(result)}
                      >
                        {t("tvdb.select")}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
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
          <div className="mb-3 flex gap-2">
            <Input
              placeholder={t("title.filterPlaceholder")}
              value={titleFilter}
              onChange={onTitleFilterChange}
            />
            <Button variant="secondary" onClick={onRefreshTitles} disabled={titleLoading}>
              {titleLoading ? t("label.refreshing") : t("label.refresh")}
            </Button>
          </div>
          <p className="mb-2 text-sm text-muted-foreground">{titleStatus}</p>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("title.table.name")}</TableHead>
                <TableHead>{t("title.table.facet")}</TableHead>
                <TableHead>{t("title.table.monitored")}</TableHead>
                <TableHead className="text-right">{t("title.table.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {monitoredTitles.map((item) => {
                const overviewTargetView = item.facet === "movie"
                  ? "movies"
                  : item.facet === "tv"
                    ? "series"
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
        </CardContent>
      </Card>
    </>
  );
}
