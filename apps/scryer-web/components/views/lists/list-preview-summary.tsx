import * as React from "react";
import { CalendarDays, Languages, Tags } from "lucide-react";
import { TitlePoster } from "@/components/title-poster";
import { TitleRatingsDisplay } from "@/components/common/title-ratings-display";
import { Popover, PopoverTrigger, PopoverContent } from "@/components/ui/popover";
import { facetById } from "@/lib/facets/registry";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListPreview, ListPreviewItem } from "@/lib/types/lists";
import { listKindLabelKey } from "@/lib/utils/lists";
import { formatLanguage } from "@/lib/utils/media-info-format";

type ListPreviewSummaryProps = {
  preview: ListPreview;
  idPrefix: string;
};

function PreviewItem({ item }: { item: ListPreviewItem }) {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);
  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const clearTimer = () => { if (timer.current) clearTimeout(timer.current); };
  const show = () => { clearTimer(); setOpen(true); };
  const hide = () => { clearTimer(); timer.current = setTimeout(() => setOpen(false), 150); };
  React.useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);
  const FacetIcon = item.kind ? facetById(item.kind)?.icon : null;

  return (
    <Popover open={open} onOpenChange={(value) => { clearTimer(); setOpen(value); }}>
      <PopoverTrigger asChild>
        <button type="button" className="w-full min-w-0 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-[8px]"
          onMouseEnter={() => { clearTimer(); timer.current = setTimeout(() => setOpen(true), 250); }}
          onMouseLeave={hide} onFocus={(event) => { if (event.currentTarget.matches(":focus-visible")) show(); }} onBlur={hide}>
          <div className="aspect-[2/3] overflow-hidden rounded-[8px] border border-[var(--scry-border3)] bg-[var(--scry-inset)]">
            <TitlePoster src={item.posterUrl} alt="" className="h-full w-full object-cover" />
          </div>
          <p className="mt-1 truncate text-[12px] font-medium text-[var(--scry-ink2)]">{item.displayTitle}</p>
          <p className="truncate text-[11px] text-[var(--scry-muted)]">
            {[item.year, item.kind ? t(listKindLabelKey(item.kind)) : null].filter(Boolean).join(" · ")}
          </p>
        </button>
      </PopoverTrigger>
      <PopoverContent side="right" align="center" sideOffset={12} collisionPadding={12}
        aria-label={item.displayTitle}
        className="z-[90] w-[360px] max-w-[calc(100vw-24px)] max-h-[var(--radix-popover-content-available-height)] overflow-y-auto rounded-xl border-[var(--scry-border3)] text-[var(--scry-ink)] shadow-xl"
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        onMouseEnter={clearTimer} onMouseLeave={hide} onFocus={clearTimer} onBlur={hide}>
        <div className="space-y-3">
          <div>
            <p className="mb-1 flex items-center gap-2 text-xs text-[var(--scry-muted)]">
              {FacetIcon ? <FacetIcon className="h-4 w-4" aria-hidden="true" /> : null}
              {[item.kind ? t(listKindLabelKey(item.kind)) : null, item.year].filter(Boolean).join(" · ")}
            </p>
            <h3 className="font-display text-base font-semibold">{item.displayTitle}</h3>
          </div>
          <TitleRatingsDisplay externalRatings={item.externalRatings} />
          {item.genresAndThemes?.length ? <p className="flex gap-2 text-sm text-[var(--scry-ink2)]"><Tags aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0" />{item.genresAndThemes.join(" · ")}</p> : null}
          {item.originalLanguage ? <p className="flex gap-2 text-sm text-[var(--scry-muted)]"><Languages aria-hidden="true" className="h-4 w-4 shrink-0" />{formatLanguage(item.originalLanguage)}</p> : null}
          {item.releaseDate ? <p className="flex gap-2 text-sm text-[var(--scry-muted)]"><CalendarDays aria-hidden="true" className="h-4 w-4 shrink-0" /><time dateTime={item.releaseDate}>{item.releaseDate}</time></p> : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/** What the next sync would do with the list as it stands: counts, then the titles it would add. */
export function ListPreviewSummary({ preview, idPrefix }: ListPreviewSummaryProps) {
  const t = useTranslate();
  const stats: Array<[string, number]> = [
    ["lists.preview.total", preview.total],
    ["lists.preview.inLibrary", preview.inLibrary],
    ["lists.preview.wouldAdd", preview.wouldAdd.length],
    ["lists.preview.filtered", preview.filtered],
    ["lists.preview.excluded", preview.excluded],
    ["lists.preview.unresolved", preview.unresolved],
  ];

  return (
    <div id={`${idPrefix}-preview-summary`} className="space-y-3">
      <dl className="grid grid-cols-3 gap-2 sm:grid-cols-6">
        {stats.map(([key, value]) => (
          <div
            key={key}
            className="rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2.5 py-2"
          >
            <dt className="truncate text-[11.5px] text-[var(--scry-muted)]">{t(key)}</dt>
            <dd className="font-display text-[17px] font-bold text-[var(--scry-ink)]">{value}</dd>
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
                <PreviewItem item={item} />
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <p className="text-[12.5px] text-[var(--scry-muted)]">{t("lists.preview.nothingToAdd")}</p>
      )}
    </div>
  );
}
