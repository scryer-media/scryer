import * as React from "react";

import type {
  TitleCollection,
  TitleDetail,
} from "@/components/containers/series-overview-container";
import { LoadingMark } from "@/components/common/loading-mark";
import { Button } from "@/components/ui/button";
import { CheckboxField } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { useSearchContext } from "@/lib/context/search-context";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import { requestableSeriesSeasons } from "@/lib/utils/requestable-series-seasons";

export function SeriesSeasonRequest({
  title,
  collections,
}: {
  title: TitleDetail;
  collections: readonly TitleCollection[];
}) {
  const t = useTranslate();
  const { requestMetadataSearchResult } = useSearchContext();
  const [open, setOpen] = React.useState(false);
  const [selected, setSelected] = React.useState<number[]>([]);
  const [submitting, setSubmitting] = React.useState(false);
  const seasons = requestableSeriesSeasons(collections);
  if (!seasons.length || !title.externalIds.length) return null;

  // A season can become unavailable while the dialog is open.
  const seasonNumbers = seasons
    .map(([number]) => number)
    .filter((number) => selected.includes(number));

  function changeOpen(next: boolean) {
    if (submitting) return;
    setOpen(next);
    if (!next) setSelected([]);
  }

  async function submit() {
    if (!seasonNumbers.length || submitting) return;
    setSubmitting(true);
    try {
      const result: MetadataTvdbSearchItem = {
        name: title.name,
        tvdbId: "",
        externalIds: title.externalIds,
        imdbId: title.imdbId,
        slug: title.slug,
        type: "series",
        year: title.year,
        status: title.contentStatus,
        overview: title.overview,
        popularity: null,
        posterUrl: title.posterUrl,
        language: title.language,
        runtimeMinutes: title.runtimeMinutes,
        sortTitle: title.sortTitle,
      };
      const accepted = await requestMetadataSearchResult(result, "SERIES", {
        libraryId: title.libraryId,
        requestedMonitorType: "ADVANCED",
        requestedMonitorSelection: { seasonNumbers, seriesMovies: [] },
      });
      if (accepted) {
        setOpen(false);
        setSelected([]);
      }
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogTrigger asChild>
        <Button
          id="series-overview-request-seasons"
          variant="outline"
          size="sm"
        >
          {t("requests.requestSeasons")}
        </Button>
      </DialogTrigger>
      <DialogContent id="series-season-request-dialog">
        <DialogHeader>
          <DialogTitle>
            {t("requests.requestSeasonsFor", { name: title.name })}
          </DialogTitle>
          <DialogDescription>{t("requests.chooseSeasons")}</DialogDescription>
        </DialogHeader>
        <div className="max-h-[45vh] space-y-2 overflow-y-auto">
          {seasons.map(([number, name]) => (
            <CheckboxField
              id={`series-season-request-${number}`}
              key={number}
              label={
                name
                  ? t("monitorSelection.seasonNamed", { number, name })
                  : t("monitorSelection.season", { number })
              }
              checked={selected.includes(number)}
              disabled={submitting}
              onCheckedChange={(checked) =>
                setSelected((previous) =>
                  checked === true
                    ? [...previous, number]
                    : previous.filter((value) => value !== number),
                )
              }
            />
          ))}
        </div>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            disabled={submitting}
            onClick={() => changeOpen(false)}
          >
            {t("label.cancel")}
          </Button>
          <Button
            id="series-season-request-submit"
            type="button"
            disabled={submitting || seasonNumbers.length === 0}
            onClick={() => void submit()}
          >
            {submitting ? <LoadingMark className="h-4 w-4" /> : null}
            {submitting ? t("search.requesting") : t("search.request")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
