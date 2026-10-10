import { ActionTooltip } from "@/components/ui/tooltip";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListCounts } from "@/lib/types/lists";
import {
  listCoverageSegmentLabelKey,
  listCoverageSegments,
  listCoverageSegmentTone,
  type ListCoverageSegment,
  type ListCoverageSegmentKey,
  type ListTone,
} from "@/lib/utils/lists";
import { cn } from "@/lib/utils";

/** The solid fill for each pill tone. */
const TONE_CLASS: Record<ListTone, string> = {
  positive: "bg-[var(--scry-success-solid)]",
  info: "bg-[var(--scry-info-solid)]",
  accent: "bg-[var(--scry-accent)]",
  warning: "bg-[var(--scry-warning-solid)]",
  negative: "bg-[var(--scry-danger-solid)]",
  neutral: "bg-[var(--scry-muted2)]",
  outline: "bg-[var(--scry-muted2)]",
};

/** The second state of a tone is drawn lighter so neighbours in the bar stay apart. */
const LIGHTER: ReadonlySet<ListCoverageSegmentKey> = new Set(["held", "filtered"]);

function segmentClass(key: ListCoverageSegmentKey): string {
  return cn(TONE_CLASS[listCoverageSegmentTone(key)], LIGHTER.has(key) && "opacity-60");
}

type ListCoverageBarProps = {
  counts: ListCounts;
  /** Where the per-state counts go: under the bar, or in a flyout on hover. */
  legend: "inline" | "hover";
  className?: string;
};

/** How much of a list is already covered, one coloured segment per outcome. */
export function ListCoverageBar({ counts, legend, className }: ListCoverageBarProps) {
  const t = useTranslate();
  const segments = listCoverageSegments(counts);
  const summary = segments
    .map((segment) => `${t(listCoverageSegmentLabelKey(segment.key))}: ${segment.count}`)
    .join(", ");

  const bar = (
    <div
      role="img"
      aria-label={summary || t("lists.counts.empty")}
      className="flex h-1.5 w-full overflow-hidden rounded-full bg-[var(--scry-inset)]"
    >
      {segments.map((segment) => (
        <span
          key={segment.key}
          className={segmentClass(segment.key)}
          style={{ width: `${(segment.fraction * 100).toFixed(2)}%` }}
        />
      ))}
    </div>
  );

  if (legend === "hover") {
    return (
      <ActionTooltip
        content={segments.length ? <CoverageCounts segments={segments} /> : t("lists.counts.empty")}
        // Taller than the bar, so the flyout is easy to reach.
        wrapperClassName={cn("flex w-full min-w-0 py-2.5", className)}
      >
        {bar}
      </ActionTooltip>
    );
  }

  return (
    <div className={cn("min-w-0", className)}>
      {bar}
      <ul className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[12px] text-[var(--scry-muted)]">
        {segments.map((segment) => (
          <li key={segment.key} className="inline-flex items-center gap-1.5">
            <span className={cn("h-2 w-2 rounded-full", segmentClass(segment.key))} aria-hidden="true" />
            {t(listCoverageSegmentLabelKey(segment.key))} {segment.count}
          </li>
        ))}
      </ul>
    </div>
  );
}

function CoverageCounts({ segments }: { segments: ListCoverageSegment[] }) {
  const t = useTranslate();
  return (
    <ul className="space-y-1">
      {segments.map((segment) => (
        <li key={segment.key} className="flex items-center gap-2">
          <span className={cn("h-2 w-2 shrink-0 rounded-full", segmentClass(segment.key))} aria-hidden="true" />
          <span className="flex-1">{t(listCoverageSegmentLabelKey(segment.key))}</span>
          <span className="pl-4 font-medium tabular-nums">{segment.count}</span>
        </li>
      ))}
    </ul>
  );
}
