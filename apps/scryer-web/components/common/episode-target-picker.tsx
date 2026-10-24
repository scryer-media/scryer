import * as React from "react";
import { Check, ChevronsUpDown, Search } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useTranslate } from "@/lib/context/translate-context";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  EPISODE_TARGET_PREFIX,
  SERIES_MOVIE_TARGET_PREFIX,
  UNASSIGNED_TARGET,
  buildEpisodeTargetRows,
  episodeTargetIds,
  episodeTargetValue,
  type EpisodeTargetEpisode,
  type EpisodeTargetSeriesMovie,
} from "@/lib/utils/episode-target";

/** The words a picker shows; each caller supplies its own copy. */
export type EpisodeTargetPickerLabels = {
  /** The "leave unassigned" option; omit to offer no such option. */
  unassigned?: string;
  seriesMovies: string;
  seasonGroup: (groupKey: string) => string;
  filterPlaceholder: string;
  filterLabel: string;
  listLabel: string;
  noMatches: string;
  selectPlaceholder: string;
};

type EpisodeTargetPickerContentProps = {
  /** Prefix of the option element ids, e.g. `activity-manual-import`. */
  idPrefix: string;
  value: string;
  groupedEpisodes: ReadonlyMap<string, readonly EpisodeTargetEpisode[]>;
  seriesMovies: readonly EpisodeTargetSeriesMovie[];
  labels: EpisodeTargetPickerLabels;
  multiple: boolean;
  onMultipleChange: (multiple: boolean) => void;
  onSelect: (value: string) => void;
};

function EpisodeTargetPickerContent({
  idPrefix,
  value,
  groupedEpisodes,
  seriesMovies,
  labels,
  multiple,
  onMultipleChange,
  onSelect,
}: EpisodeTargetPickerContentProps) {
  const t = useTranslate();
  const [query, setQuery] = React.useState("");
  const searchRef = React.useRef<HTMLInputElement>(null);
  const selectedEpisodeIds = React.useMemo(() => new Set(episodeTargetIds(value)), [value]);
  const isSelected = (rowValue: string) => {
    const ids = episodeTargetIds(rowValue);
    if (ids.length === 0) return rowValue === value;
    return ids.every((id) => selectedEpisodeIds.has(id));
  };
  // A file that holds two episodes must be assignable to both at once; the
  // single-target picker stays the default so the common case is unchanged.
  const toggleEpisode = (rowValue: string) => {
    const [episodeId] = episodeTargetIds(rowValue);
    if (!episodeId) {
      onSelect(rowValue);
      return;
    }
    const next = new Set(selectedEpisodeIds);
    if (next.has(episodeId)) next.delete(episodeId);
    else next.add(episodeId);
    onSelect(next.size === 0 ? UNASSIGNED_TARGET : episodeTargetValue(Array.from(next)));
  };
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const rows = React.useMemo(
    () => buildEpisodeTargetRows(groupedEpisodes, seriesMovies, query, t, labels),
    [groupedEpisodes, labels, query, seriesMovies, t],
  );
  React.useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [query]);

  return (
    <PopoverContent
      align="start"
      className="z-[90] w-[var(--radix-popover-trigger-width)] min-w-[280px] p-2"
      onOpenAutoFocus={(event) => {
        event.preventDefault();
        searchRef.current?.focus();
      }}
    >
      <label className="mb-2 flex items-center gap-2 px-1 text-xs text-muted-foreground">
        <input
          type="checkbox"
          checked={multiple}
          onChange={(event) => onMultipleChange(event.target.checked)}
          aria-label={t("queue.manualImportMultipleEpisodesLabel")}
        />
        <span>{t("queue.manualImportMultipleEpisodes")}</span>
      </label>
      <div className="relative mb-2">
        <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          ref={searchRef}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={labels.filterPlaceholder}
          aria-label={labels.filterLabel}
          className="h-8 pl-8 text-xs"
        />
      </div>
      {rows.length === 0 ? (
        <div className="px-2 py-6 text-center text-xs text-muted-foreground">
          {labels.noMatches}
        </div>
      ) : (
        <div
          ref={scrollRef}
          role="listbox"
          aria-label={labels.listLabel}
          className="h-[280px] overflow-y-auto overscroll-contain"
        >
          <div className="w-full">
            {rows.map((row) => (
              <div key={row.key}>
                {row.kind === "group" ? (
                  <div className="px-2 pt-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/60">
                    {row.label}
                  </div>
                ) : (
                  <button
                    type="button"
                    role="option"
                    aria-selected={isSelected(row.value)}
                    id={
                      row.value === UNASSIGNED_TARGET
                        ? selectorId(`${idPrefix}-skip-option`)
                        : row.value.startsWith(SERIES_MOVIE_TARGET_PREFIX)
                          ? selectorId(
                              `${idPrefix}-series-movie-option`,
                              row.value.slice(SERIES_MOVIE_TARGET_PREFIX.length),
                            )
                          : selectorId(
                              `${idPrefix}-episode-option`,
                              row.value.slice(EPISODE_TARGET_PREFIX.length),
                            )
                    }
                    className="flex h-9 w-full items-center gap-2 rounded-md px-2 text-left text-xs text-foreground hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
                    onClick={() => {
                      if (multiple) toggleEpisode(row.value);
                      else onSelect(row.value);
                    }}
                  >
                    <Check
                      className={`h-3.5 w-3.5 shrink-0 ${
                        isSelected(row.value) ? "opacity-100" : "opacity-0"
                      }`}
                    />
                    <span className="truncate">{row.label}</span>
                  </button>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </PopoverContent>
  );
}

export type EpisodeTargetSelectProps = {
  /** Id of the combobox trigger. */
  triggerId: string;
  idPrefix: string;
  value: string;
  groupedEpisodes: ReadonlyMap<string, readonly EpisodeTargetEpisode[]>;
  seriesMovies: readonly EpisodeTargetSeriesMovie[];
  /** Display label for each option value, used on the trigger. */
  targetLabels: ReadonlyMap<string, string>;
  labels: EpisodeTargetPickerLabels;
  onChange: (value: string) => void;
};

/**
 * A searchable season/episode combobox. Episodes are grouped by season and a
 * checkbox switches to multi-select for a file that holds several episodes.
 */
export const EpisodeTargetSelect = React.memo(function EpisodeTargetSelect({
  triggerId,
  idPrefix,
  value,
  groupedEpisodes,
  seriesMovies,
  targetLabels,
  labels,
  onChange,
}: EpisodeTargetSelectProps) {
  const [open, setOpen] = React.useState(false);
  const selectedEpisodeIds = React.useMemo(() => episodeTargetIds(value), [value]);
  // A selection of several episodes opens the picker already in multi-select
  // mode, so "change" edits the set instead of replacing it.
  const [multiple, setMultiple] = React.useState(selectedEpisodeIds.length > 1);
  React.useEffect(() => {
    if (selectedEpisodeIds.length > 1) setMultiple(true);
  }, [selectedEpisodeIds.length]);

  return (
    <Popover modal open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          id={triggerId}
          variant="outline"
          role="combobox"
          aria-expanded={open}
          className="h-auto min-h-8 w-full justify-between gap-2 px-3 py-1 text-xs font-normal"
        >
          {selectedEpisodeIds.length > 1 ? (
            <span className="flex min-w-0 flex-wrap items-center gap-1">
              {selectedEpisodeIds.map((episodeId) => (
                <Badge key={episodeId} tone="info" className="max-w-full px-1.5 text-[10px]">
                  <span className="truncate">
                    {targetLabels.get(episodeTargetValue([episodeId])) ?? episodeId}
                  </span>
                </Badge>
              ))}
            </span>
          ) : (
            <span className="truncate">
              {targetLabels.get(value) ?? labels.selectPlaceholder}
            </span>
          )}
          <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 opacity-60" />
        </Button>
      </PopoverTrigger>
      {open ? (
        <EpisodeTargetPickerContent
          idPrefix={idPrefix}
          value={value}
          groupedEpisodes={groupedEpisodes}
          seriesMovies={seriesMovies}
          labels={labels}
          multiple={multiple}
          onMultipleChange={setMultiple}
          onSelect={(nextValue) => {
            onChange(nextValue);
            if (!multiple) setOpen(false);
          }}
        />
      ) : null}
    </Popover>
  );
});
