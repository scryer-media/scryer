import { TitlePoster } from "@/components/title-poster";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListPreview } from "@/lib/types/lists";
import { listKindLabelKey } from "@/lib/utils/lists";

type ListPreviewSummaryProps = {
  preview: ListPreview;
  idPrefix: string;
};

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
                <div className="aspect-[2/3] overflow-hidden rounded-[8px] border border-[var(--scry-border3)] bg-[var(--scry-inset)]">
                  <TitlePoster
                    src={item.posterUrl}
                    alt=""
                    className="h-full w-full object-cover"
                  />
                </div>
                <p className="mt-1 truncate text-[12px] font-medium text-[var(--scry-ink2)]" title={item.displayTitle}>
                  {item.displayTitle}
                </p>
                <p className="truncate text-[11px] text-[var(--scry-muted)]">
                  {[item.year, item.kind ? t(listKindLabelKey(item.kind)) : null].filter(Boolean).join(" · ")}
                </p>
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
