import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { TitleRecord } from "@/lib/types";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

type Facet = "MOVIE" | "SERIES" | "ANIME";
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
  addTvdbCandidateToCatalog: (candidate: TvdbSearchItem) => Promise<void> | void;
  titleFilter: string;
  onTitleFilterChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  onRefreshTitles: () => void;
  titleLoading: boolean;
  monitoredTitles: TitleRecord[];
  onOpenOverview: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
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
  addTvdbCandidateToCatalog,
  titleFilter,
  onTitleFilterChange,
  onRefreshTitles,
  titleLoading,
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

  const handleAddTvdbToCatalog = React.useCallback(
    (candidate: TvdbSearchItem) => {
      void addTvdbCandidateToCatalog(candidate);
    },
    [addTvdbCandidateToCatalog],
  );

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
                  <SelectItem value="MOVIE">{t("search.facetMovie")}</SelectItem>
                  <SelectItem value="SERIES">{t("search.facetSeries")}</SelectItem>
                  <SelectItem value="ANIME">{t("search.facetAnime")}</SelectItem>
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
            {queueFacet === "MOVIE" && (
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
            {queueFacet !== "MOVIE" && (
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
                  key={String(result.smgId ?? result.tvdbId ?? result.name)}
                  className="rounded-lg border border-border p-3"
                >
                  <div className="mb-2 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                    <div className="flex min-h-20 gap-3">
                      <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                        <TitlePosterSlot
                          src={result.posterUrl}
                          alt={t("media.posterAlt", { name: result.name })}
                          className="h-full w-full object-cover"
                          placeholderClassName="h-full w-full"
                          emptyLabel={t("label.noArt")}
                          fallbackTitle={result.name}
                          fallbackTone={queueFacet}
                          fallbackShowText={false}
                          loading="lazy"
                        />
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
            </div>
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
          {isMobile ? (
            <div className="space-y-2">
              {monitoredTitles.map((item) => {
                const overviewTargetView = item.facet === "MOVIE"
                  ? "movies"
                  : item.facet === "SERIES"
                    ? "series"
                    : item.facet === "ANIME"
                      ? "anime"
                      : null;
                return (
                  <div key={item.id} className="rounded-lg border border-border p-3">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        {overviewTargetView ? (
                          <button
                            type="button"
                            onClick={() => onOpenOverview(overviewTargetView, item)}
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
                  const overviewTargetView = item.facet === "MOVIE"
                    ? "movies"
                    : item.facet === "SERIES"
                      ? "series"
                      : item.facet === "ANIME"
                        ? "anime"
                        : null;
                  return (
                    <TableRow key={item.id}>
                      <TableCell>
                        {overviewTargetView ? (
                          <button
                            type="button"
                            onClick={() => onOpenOverview(overviewTargetView, item)}
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
