import { useTranslate } from "@/lib/context/translate-context";
import type { ListCounts } from "@/lib/types/lists";
import {
  listCoverageSegmentLabelKey,
  listCoverageSegments,
  type ListCoverageSegmentKey,
} from "@/lib/utils/lists";
import { cn } from "@/lib/utils";

const SEGMENT_CLASS: Record<ListCoverageSegmentKey, string> = {
  inLibrary: "bg-[var(--scry-success-solid)]",
  added: "bg-[var(--scry-accent)]",
  requested: "bg-[var(--scry-info-solid)]",
  held: "bg-[var(--scry-info-solid)] opacity-60",
  filtered: "bg-[var(--scry-muted2)] opacity-60",
  excluded: "bg-[var(--scry-muted2)]",
  unresolved: "bg-[var(--scry-warning-solid)]",
};

type ListCoverageBarProps = {
  counts: ListCounts;
  showLegend?: boolean;
  className?: string;
};

/** How much of a list is already covered, one coloured segment per outcome. */
export function ListCoverageBar({ counts, showLegend = false, className }: ListCoverageBarProps) {
  const t = useTranslate();
  const segments = listCoverageSegments(counts);
  const summary = segments
    .map((segment) => `${t(listCoverageSegmentLabelKey(segment.key))}: ${segment.count}`)
    .join(", ");

  return (
    <div className={cn("min-w-0", className)}>
      <div
        role="img"
        aria-label={summary || t("lists.counts.empty")}
        title={summary || undefined}
        className="flex h-1.5 w-full overflow-hidden rounded-full bg-[var(--scry-inset)]"
      >
        {segments.map((segment) => (
          <span
            key={segment.key}
            className={SEGMENT_CLASS[segment.key]}
            style={{ width: `${(segment.fraction * 100).toFixed(2)}%` }}
          />
        ))}
      </div>
      {showLegend ? (
        <ul className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[12px] text-[var(--scry-muted)]">
          {segments.map((segment) => (
            <li key={segment.key} className="inline-flex items-center gap-1.5">
              <span className={cn("h-2 w-2 rounded-full", SEGMENT_CLASS[segment.key])} aria-hidden="true" />
              {t(listCoverageSegmentLabelKey(segment.key))} {segment.count}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
