import { Eye, EyeOff, Play, Square } from "lucide-react";
import type { ReactNode } from "react";
import { UnderlineFilterButton } from "@/components/common/underline-filter-button";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleRecord } from "@/lib/types";

export type TitleQuickFilters = {
  monitored: boolean;
  unmonitored: boolean;
  continuing: boolean;
  ended: boolean;
};

export type TitleQuickFilterCounts = {
  all: number;
  monitored: number;
  unmonitored: number;
  continuing: number;
  ended: number;
  missing?: number;
  partial?: number;
  complete?: number;
  needsAttention?: number;
};

function normalizeQuickFilterStatus(
  status: string | null | undefined,
): "continuing" | "ended" | null {
  const normalized = status?.trim().toLowerCase();
  switch (normalized) {
    case "continuing":
    case "returning":
      return "continuing";
    case "ended":
    case "finished":
      return "ended";
    default:
      return null;
  }
}

export function hasActiveTitleQuickFilters(
  filters: TitleQuickFilters,
  view: "movies" | "series" | "anime",
) {
  return (
    filters.monitored ||
    filters.unmonitored ||
    (view !== "movies" && (filters.continuing || filters.ended))
  );
}

export function getTitleQuickFilterCounts(
  titles: TitleRecord[],
  filters?: TitleQuickFilters,
  view: "movies" | "series" | "anime" = "series",
): TitleQuickFilterCounts {
  const effectiveFilters = filters
    ? {
        ...filters,
        continuing: view === "movies" ? false : filters.continuing,
        ended: view === "movies" ? false : filters.ended,
      }
    : null;
  const titleMatchesActiveMonitoring = (title: TitleRecord) => {
    if (!effectiveFilters?.monitored && !effectiveFilters?.unmonitored) {
      return true;
    }
    return (
      (effectiveFilters.monitored && title.monitored) ||
      (effectiveFilters.unmonitored && !title.monitored)
    );
  };
  const titleMatchesActiveStatus = (title: TitleRecord) => {
    if (!effectiveFilters?.continuing && !effectiveFilters?.ended) {
      return true;
    }
    const status = normalizeQuickFilterStatus(title.contentStatus);
    return (
      (effectiveFilters.continuing && status === "continuing") ||
      (effectiveFilters.ended && status === "ended")
    );
  };

  return titles.reduce<TitleQuickFilterCounts>(
    (counts, title) => {
      counts.all += 1;
      if (title.monitored && titleMatchesActiveStatus(title)) {
        counts.monitored += 1;
      } else if (!title.monitored && titleMatchesActiveStatus(title)) {
        counts.unmonitored += 1;
      }

      const status = normalizeQuickFilterStatus(title.contentStatus);
      if (status === "continuing" && titleMatchesActiveMonitoring(title)) {
        counts.continuing += 1;
      } else if (status === "ended" && titleMatchesActiveMonitoring(title)) {
        counts.ended += 1;
      }

      return counts;
    },
    {
      all: 0,
      monitored: 0,
      unmonitored: 0,
      continuing: 0,
      ended: 0,
    },
  );
}

export function filterTitlesByQuickFilters(
  titles: TitleRecord[],
  filters: TitleQuickFilters,
): TitleRecord[] {
  return titles.filter((title) => {
    const monitoringFiltersActive = filters.monitored || filters.unmonitored;
    if (monitoringFiltersActive) {
      const matchesMonitoring =
        (filters.monitored && title.monitored) ||
        (filters.unmonitored && !title.monitored);
      if (!matchesMonitoring) {
        return false;
      }
    }

    const statusFiltersActive = filters.continuing || filters.ended;
    if (statusFiltersActive) {
      const normalizedStatus = normalizeQuickFilterStatus(title.contentStatus);
      const matchesStatus =
        (filters.continuing && normalizedStatus === "continuing") ||
        (filters.ended && normalizedStatus === "ended");
      if (!matchesStatus) {
        return false;
      }
    }

    return true;
  });
}

/**
 * Frames a two-up filter as one control: a shared border the two buttons divide,
 * rather than two buttons that happen to sit next to each other.
 */
export const SPLIT_FILTER_GROUP_CLASS_NAME =
  "flex w-full overflow-hidden rounded-[10px] border border-[rgba(var(--scry-accent-rgb),0.55)]";

/**
 * One segment inside {@link SPLIT_FILTER_GROUP_CLASS_NAME}. Square off the
 * button's own rounding, let the second segment carry the divider, and fill the
 * selected one.
 */
export function splitFilterSegmentClassName(
  selected: boolean,
  divider = false,
): string {
  return [
    "h-11 min-w-0 flex-1 justify-center rounded-none border-0 px-3",
    divider ? "border-l !border-l-[rgba(var(--scry-accent-rgb),0.35)]" : "",
    selected
      ? "bg-[rgba(var(--scry-accent-rgb),0.38)] text-white [&>span:last-child]:hidden"
      : "bg-[var(--scry-inset)] hover:bg-[var(--scry-surface2)]",
  ].join(" ");
}

/**
 * A standalone toggle in the filter panel, for a filter that is one question
 * rather than a set of states.
 */
export function panelFilterButtonClassName(selected: boolean, fullWidth = false): string {
  return [
    "h-11 w-full justify-center rounded-[10px] border px-3",
    fullWidth ? "col-span-2" : "",
    selected
      ? "border-[rgba(var(--scry-accent-rgb),0.62)] bg-[rgba(var(--scry-accent-rgb),0.28)] [&>span:last-child]:hidden"
      : "border-[var(--scry-border2)] bg-[var(--scry-inset)] hover:border-[var(--scry-border3)] hover:bg-[var(--scry-surface2)]",
  ].join(" ");
}

export function TitleQuickFilterBar({
  view,
  filters,
  counts,
  onToggleMonitoring,
  onToggleStatus,
  onClear,
  trailingContent,
  hideFilters = false,
  appearance = "underline",
}: {
  view: "movies" | "series" | "anime";
  filters: TitleQuickFilters;
  counts?: TitleQuickFilterCounts;
  onToggleMonitoring: (filter: "monitored" | "unmonitored") => void;
  onToggleStatus: (filter: "continuing" | "ended") => void;
  onClear: () => void;
  trailingContent?: ReactNode;
  hideFilters?: boolean;
  appearance?: "underline" | "panel";
}) {
  const t = useTranslate();
  const showStatusFilters = view !== "movies";
  const allSelected = !hasActiveTitleQuickFilters(filters, view);
  const panelAppearance = appearance === "panel";
  const panelButtonClassName = (selected: boolean, fullWidth = false) =>
    panelAppearance ? panelFilterButtonClassName(selected, fullWidth) : undefined;

  return (
    <div
      className={`flex flex-wrap items-end gap-3 ${
        hideFilters ? "justify-end" : "justify-between"
      }`}
    >
      {!hideFilters ? (
        <div
          role="group"
          aria-label={t("label.filters")}
          className={
            panelAppearance
              ? "grid w-full min-w-0 grid-cols-2 gap-2"
              : "relative top-px flex min-h-10 min-w-0 max-w-full flex-1 flex-wrap items-center justify-start gap-x-5 gap-y-1 border-0 bg-transparent p-0 shadow-none"
          }
        >
        {panelAppearance ? (
          <div className="col-span-2 flex w-full justify-center">
            <div className={SPLIT_FILTER_GROUP_CLASS_NAME}>
              <UnderlineFilterButton
                selected={filters.monitored}
                onClick={() => onToggleMonitoring("monitored")}
                icon={<Eye className="h-3.5 w-3.5" />}
                label={t("title.monitored")}
                count={counts?.monitored}
                tone="success"
                className={splitFilterSegmentClassName(filters.monitored)}
              />
              <UnderlineFilterButton
                selected={filters.unmonitored}
                onClick={() => onToggleMonitoring("unmonitored")}
                icon={<EyeOff className="h-3.5 w-3.5" />}
                label={t("search.monitorType.unmonitored")}
                count={counts?.unmonitored}
                tone="danger"
                className={splitFilterSegmentClassName(filters.unmonitored, true)}
              />
            </div>
          </div>
        ) : (
          <>
            <UnderlineFilterButton
              selected={allSelected}
              onClick={onClear}
              label={t("activity.historyFilter.all")}
              count={counts?.all}
            />
            <UnderlineFilterButton
              selected={filters.monitored}
              onClick={() => onToggleMonitoring("monitored")}
              icon={<Eye className="h-3.5 w-3.5" />}
              label={t("title.monitored")}
              count={counts?.monitored}
              tone="success"
            />
            <UnderlineFilterButton
              selected={filters.unmonitored}
              onClick={() => onToggleMonitoring("unmonitored")}
              icon={<EyeOff className="h-3.5 w-3.5" />}
              label={t("search.monitorType.unmonitored")}
              count={counts?.unmonitored}
              tone="danger"
            />
          </>
        )}
        {showStatusFilters ? (
          <>
            <UnderlineFilterButton
              selected={filters.continuing}
              onClick={() => onToggleStatus("continuing")}
              icon={<Play className="h-3.5 w-3.5" />}
              label={t("title.continuing")}
              count={counts?.continuing}
              tone="success"
              className={panelButtonClassName(filters.continuing)}
            />
            <UnderlineFilterButton
              selected={filters.ended}
              onClick={() => onToggleStatus("ended")}
              icon={<Square className="h-3.5 w-3.5" />}
              label={t("title.ended")}
              count={counts?.ended}
              tone="muted"
              className={panelButtonClassName(filters.ended)}
            />
          </>
        ) : null}
        </div>
      ) : null}
      {trailingContent ? (
        <div className="w-full shrink-0 sm:w-auto">{trailingContent}</div>
      ) : null}
    </div>
  );
}
