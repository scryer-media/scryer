import * as React from "react";
import { TitlePoster } from "@/components/title-poster";
import {
  Popover,
  PopoverTrigger,
  PopoverContent,
} from "@/components/ui/popover";
import { facetById } from "@/lib/facets/registry";
import { useTranslate } from "@/lib/context/translate-context";
import { selectorId } from "@/lib/utils/dom-ids";
import type { ListPreview, ListPreviewItem } from "@/lib/types/lists";
import { listKindLabelKey, listPreviewMetadataLookup } from "@/lib/utils/lists";
import {
  LIST_TITLE_FLYOUT_CLASS,
  ListTitleFlyoutBody,
} from "./list-title-flyout";
import { SimklAttribution } from "./simkl-attribution";
import { useListTitleMetadata } from "./use-list-title-metadata";

type ListPreviewSummaryProps = {
  preview: ListPreview;
  idPrefix: string;
  providerType: string;
};

/** One previewed title: its poster, and on hover or focus what is known about it. */
export function ListPreviewPoster({
  item,
  idPrefix,
}: {
  item: ListPreviewItem;
  idPrefix: string;
}) {
  const posterId = selectorId(idPrefix, "preview-item", item.itemKey);
  const detailsId = `${posterId}-details`;
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);
  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const clearTimer = () => {
    if (timer.current) clearTimeout(timer.current);
  };
  // The description and backdrop are read from the metadata service as the flyout opens.
  const [metadata, requestMetadata] = useListTitleMetadata(
    listPreviewMetadataLookup(item),
  );
  const show = () => {
    clearTimer();
    requestMetadata();
    setOpen(true);
  };
  const hide = () => {
    clearTimer();
    timer.current = setTimeout(() => setOpen(false), 150);
  };
  React.useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );
  const FacetIcon = item.kind ? facetById(item.kind)?.icon : null;

  return (
    <Popover
      open={open}
      onOpenChange={(value) => {
        clearTimer();
        if (value) requestMetadata();
        setOpen(value);
      }}
    >
      <PopoverTrigger asChild>
        <button
          id={posterId}
          data-ui="list-preview-item"
          data-item-key={item.itemKey}
          data-item-title={item.displayTitle}
          aria-controls={detailsId}
          type="button"
          className="w-full min-w-0 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-[8px]"
          onMouseEnter={() => {
            clearTimer();
            requestMetadata();
            timer.current = setTimeout(() => setOpen(true), 250);
          }}
          onMouseLeave={hide}
          onFocus={(event) => {
            if (event.currentTarget.matches(":focus-visible")) show();
          }}
          onBlur={hide}
        >
          <div className="aspect-[2/3] overflow-hidden rounded-[8px] border border-[var(--scry-border3)] bg-[var(--scry-inset)]">
            <TitlePoster
              src={item.posterUrl}
              alt=""
              className="h-full w-full object-cover"
            />
          </div>
          <p className="mt-1 truncate text-[12px] font-medium text-[var(--scry-ink2)]">
            {item.displayTitle}
          </p>
          <p className="truncate text-[11px] text-[var(--scry-muted)]">
            {[item.year, item.kind ? t(listKindLabelKey(item.kind)) : null]
              .filter(Boolean)
              .join(" · ")}
          </p>
        </button>
      </PopoverTrigger>
      <PopoverContent
        id={detailsId}
        side="right"
        align="center"
        sideOffset={12}
        collisionPadding={12}
        aria-label={item.displayTitle}
        className={LIST_TITLE_FLYOUT_CLASS}
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        onMouseEnter={clearTimer}
        onMouseLeave={hide}
        onFocus={clearTimer}
        onBlur={hide}
      >
        <ListTitleFlyoutBody
          title={item.displayTitle}
          kindIcon={
            FacetIcon ? (
              <FacetIcon className="h-4 w-4" aria-hidden="true" />
            ) : null
          }
          kindLabel={item.kind ? t(listKindLabelKey(item.kind)) : null}
          year={item.year}
          runtimeMinutes={metadata?.runtimeMinutes}
          posterUrl={item.posterUrl ?? metadata?.posterUrl}
          backgroundUrl={metadata?.backgroundUrl}
          externalRatings={item.externalRatings}
          overview={metadata?.overview}
          genres={item.genresAndThemes}
          originalLanguage={item.originalLanguage}
          releaseDate={item.releaseDate}
        />
      </PopoverContent>
    </Popover>
  );
}

/** What the next sync would do with the list as it stands: counts, then the titles it would add. */
export function ListPreviewSummary({
  preview,
  idPrefix,
  providerType,
}: ListPreviewSummaryProps) {
  const t = useTranslate();
  const stats: Array<[string, string, number]> = [
    ["total", "lists.preview.total", preview.total],
    ["inLibrary", "lists.preview.inLibrary", preview.inLibrary],
    ["wouldAdd", "lists.preview.wouldAdd", preview.wouldAdd.length],
    ["filtered", "lists.preview.filtered", preview.filtered],
    ["excluded", "lists.preview.excluded", preview.excluded],
    ["unresolved", "lists.preview.unresolved", preview.unresolved],
  ];

  return (
    <div id={`${idPrefix}-preview-summary`} className="space-y-3">
      {providerType === "simkl" ? <SimklAttribution /> : null}
      <dl className="grid grid-cols-3 gap-2 sm:grid-cols-6">
        {stats.map(([key, labelKey, value]) => (
          <div
            key={key}
            id={`${idPrefix}-preview-stat-${key}`}
            className="rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2.5 py-2"
          >
            <dt className="truncate text-[11.5px] text-[var(--scry-muted)]">
              {t(labelKey)}
            </dt>
            <dd className="font-display text-[17px] font-bold text-[var(--scry-ink)]">
              {value}
            </dd>
          </div>
        ))}
      </dl>
      {preview.unresolved > 0 ? (
        <p className="text-[12.5px] text-[var(--scry-muted)]">
          {t("lists.preview.unresolvedHelp", { count: preview.unresolved })}
        </p>
      ) : null}
      {preview.wouldAdd.length > 0 ? (
        <div>
          <p className="mb-2 text-[12.5px] font-semibold text-[var(--scry-ink2)]">
            {t("lists.preview.wouldAddHeading")}
          </p>
          <ul className="grid grid-cols-3 gap-2 sm:grid-cols-6">
            {preview.wouldAdd.map((item) => (
              <li key={item.itemKey} className="min-w-0">
                <ListPreviewPoster item={item} idPrefix={idPrefix} />
                {providerType === "simkl" ? (
                  <SimklAttribution itemKey={item.itemKey} />
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <p className="text-[12.5px] text-[var(--scry-muted)]">
          {t("lists.preview.nothingToAdd")}
        </p>
      )}
    </div>
  );
}
