import * as React from "react";
import {
  AlertTriangle,
  LibraryBig,
  CalendarDays,
  Film,
  Folder,
  PanelRightClose,
  RefreshCw,
  Search,
  SlidersHorizontal,
  Sparkles,
  Star,
  Tag,
  X,
} from "lucide-react";

import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { useTitleTagDefinitions } from "@/lib/hooks/use-title-tag-definitions";
import { availableTitleTagLabels } from "@/lib/utils/title-tags";
import { Input } from "@/components/ui/input";
import {
  MultiSelectDropdown,
  type MultiSelectGroup,
} from "@/components/ui/multi-select-dropdown";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  LibraryRecord,
  TitleCatalogFilterOptionsRecord,
} from "@/lib/types";
import type { TitleCatalogAdvancedFilters } from "@/lib/utils/title-catalog-query";
import { cn } from "@/lib/utils";

import {
  hasActiveTitleQuickFilters,
  TitleQuickFilterBar,
  type TitleQuickFilterCounts,
  type TitleQuickFilters,
} from "./title-quick-filters";

const DEFAULT_MINIMUM_YEAR = 1900;
const FILTER_RANGE_CLASS_NAME =
  "h-1.5 w-full appearance-none rounded-full bg-transparent accent-[var(--scry-accent)] [&::-moz-range-progress]:h-1.5 [&::-moz-range-progress]:rounded-full [&::-moz-range-progress]:bg-transparent [&::-moz-range-thumb]:h-[15px] [&::-moz-range-thumb]:w-[15px] [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)] [&::-moz-range-track]:h-1.5 [&::-moz-range-track]:rounded-full [&::-moz-range-track]:bg-transparent [&::-webkit-slider-runnable-track]:h-1.5 [&::-webkit-slider-runnable-track]:rounded-full [&::-webkit-slider-runnable-track]:bg-transparent [&::-webkit-slider-thumb]:mt-[-4.5px] [&::-webkit-slider-thumb]:h-[15px] [&::-webkit-slider-thumb]:w-[15px] [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)]";
const FILTER_RANGE_THUMB_POINTER_CLASS_NAME =
  "pointer-events-none [&::-moz-range-thumb]:pointer-events-auto [&::-webkit-slider-thumb]:pointer-events-auto";

type CatalogFiltersPanelProps = {
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  filters: TitleCatalogAdvancedFilters;
  options: TitleCatalogFilterOptionsRecord;
  optionsError: boolean;
  onRetryOptions: () => void;
  onFiltersChange: (updates: Partial<TitleCatalogAdvancedFilters>) => void;
  searchValue: string;
  onSearchValueChange: (value: string) => void;
  onClear: () => void;
  quickFilters: TitleQuickFilters;
  quickFilterCounts?: TitleQuickFilterCounts;
  quickFilterView: "movies" | "series" | "anime";
  onToggleQuickMonitoring: (filter: "monitored" | "unmonitored") => void;
  onToggleQuickStatus: (filter: "continuing" | "ended") => void;
  onClearQuickFilters: () => void;
  onCollapse?: () => void;
  className?: string;
};

function defaultMaximumYear() {
  return new Date().getFullYear() + 3;
}

function FilterLabel({
  children,
  icon,
}: {
  children: React.ReactNode;
  icon?: React.ReactNode;
}) {
  return (
    <div className="mb-2.5 flex items-center gap-1.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
      {icon}
      {children}
    </div>
  );
}

function FilterChips({
  values,
  labels,
  onRemove,
}: {
  values: string[];
  labels: Map<string, string>;
  onRemove: (value: string) => void;
}) {
  if (values.length === 0) return null;
  return (
    <div className="mt-2.5 flex flex-wrap gap-2">
      {values.map((value) => (
        <button
          key={value}
          type="button"
          onClick={() => onRemove(value)}
          className="inline-flex max-w-full items-center gap-2 rounded-[8px] border border-[rgba(var(--scry-accent-rgb),0.34)] bg-[rgba(var(--scry-accent-rgb),0.15)] px-2.5 py-1 text-xs font-semibold text-[var(--scry-accent-text)] transition hover:border-[rgba(var(--scry-accent-rgb),0.48)] hover:bg-[rgba(var(--scry-accent-rgb),0.22)]"
        >
          <span className="truncate">{labels.get(value) ?? value}</span>
          <X className="h-3.5 w-3.5 opacity-75" aria-hidden="true" />
        </button>
      ))}
    </div>
  );
}

export function CatalogFiltersPanel({
  libraries,
  librariesLoading,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  filters,
  options,
  optionsError,
  onRetryOptions,
  onFiltersChange,
  searchValue,
  onSearchValueChange,
  onClear,
  quickFilters,
  quickFilterCounts,
  quickFilterView,
  onToggleQuickMonitoring,
  onToggleQuickStatus,
  onClearQuickFilters,
  onCollapse,
  className,
}: CatalogFiltersPanelProps) {
  const t = useTranslate();
  const eligibleLibraryIds = React.useMemo(() => {
    const explicitLibraryIds = selectedLibraryIds.filter(Boolean);
    return new Set(
        explicitLibraryIds.length > 0
          ? explicitLibraryIds
          : libraries.map((library) => library.id),
    );
  }, [libraries, selectedLibraryIds]);
  const rootGroups = React.useMemo<MultiSelectGroup[]>(
    () =>
      libraries
        .filter((library) => eligibleLibraryIds.has(library.id))
        .map((library) => ({
          label: library.name,
          options: library.roots.map((root) => ({
            value: root.id,
            label: root.path,
            title: root.path,
          })),
        }))
        .filter((group) => group.options.length > 0),
    [eligibleLibraryIds, libraries],
  );
  const rootLabel =
    filters.rootFolderIds.length === 0
      ? t("title.catalogFilters.allRootFolders")
      : filters.rootFolderIds.length === 1
        ? (rootGroups
            .flatMap((group) => group.options)
            .find((option) => option.value === filters.rootFolderIds[0])
            ?.label ?? t("title.rootFolder"))
        : t("title.catalogFilters.selectedCount", {
            count: filters.rootFolderIds.length,
          });
  const genreLabels = React.useMemo(
    () => new Map(options.genres.map((option) => [option.key, option.name])),
    [options.genres],
  );
  const genreOptions = React.useMemo(
    () =>
      options.genres.map((option) => ({
        value: option.key,
        label: option.name,
      })),
    [options.genres],
  );
  const themeLabels = React.useMemo(
    () => new Map(options.themes.map((option) => [option.key, option.name])),
    [options.themes],
  );
  const themeOptions = React.useMemo(
    () =>
      options.themes.map((option) => ({
        value: option.key,
        label: option.name,
      })),
    [options.themes],
  );
  const handleRootFolderChange = React.useCallback(
    (rootFolderIds: string[]) => onFiltersChange({ rootFolderIds }),
    [onFiltersChange],
  );
  const handleGenreChange = React.useCallback(
    (genreTagKeys: string[]) => onFiltersChange({ genreTagKeys }),
    [onFiltersChange],
  );
  const handleThemeChange = React.useCallback(
    (themeTagKeys: string[]) => onFiltersChange({ themeTagKeys }),
    [onFiltersChange],
  );
  // The user-tag vocabulary comes from the registry rather than from the
  // catalog's own facet counts: a defined tag nothing carries yet should still
  // be offerable, and there is no free text to fall back on.
  const { definitions: tagDefinitions, loading: tagDefinitionsLoading } =
    useTitleTagDefinitions();
  const userTagOptions = React.useMemo(
    () =>
      availableTitleTagLabels(tagDefinitions, []).map((label) => ({
        value: label,
        label,
      })),
    [tagDefinitions],
  );
  const userTagLabels = React.useMemo(
    () => new Map(userTagOptions.map((option) => [option.value, option.label])),
    [userTagOptions],
  );
  const handleUserTagChange = React.useCallback(
    (userTagLabelValues: string[]) =>
      onFiltersChange({ userTagLabels: userTagLabelValues }),
    [onFiltersChange],
  );
  const minimumYearBound = options.minimumYear ?? DEFAULT_MINIMUM_YEAR;
  const maximumYearBound = Math.max(
    options.maximumYear ?? defaultMaximumYear(),
    minimumYearBound,
  );
  const minimumYear = Math.min(
    Math.max(filters.minimumYear ?? minimumYearBound, minimumYearBound),
    maximumYearBound,
  );
  const maximumYear = Math.max(
    Math.min(filters.maximumYear ?? maximumYearBound, maximumYearBound),
    minimumYear,
  );
  const yearSpan = Math.max(1, maximumYearBound - minimumYearBound);
  const minimumYearPercent =
    ((minimumYear - minimumYearBound) / yearSpan) * 100;
  const maximumYearPercent =
    ((maximumYear - minimumYearBound) / yearSpan) * 100;
  const minimumRating = filters.minimumRating ?? 0;
  const hasActiveFilters =
    hasActiveTitleQuickFilters(quickFilters, quickFilterView) ||
    searchValue.trim().length > 0 ||
    selectedLibraryIds.length > 0 ||
    filters.rootFolderIds.length > 0 ||
    filters.genreTagKeys.length > 0 ||
    filters.themeTagKeys.length > 0 ||
    filters.userTagLabels.length > 0 ||
    filters.minimumYear !== null ||
    filters.maximumYear !== null ||
    minimumRating > 0;

  return (
    <aside
      data-testid="catalog-filters-panel"
      className={cn(
        "relative flex min-h-0 flex-col overflow-y-auto bg-[var(--scry-surf)] px-[18px] py-4",
        className,
      )}
    >
      <div className="mb-4 flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <div className="flex size-9 shrink-0 items-center justify-center rounded-[10px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.32),rgba(155,91,255,0.2))] text-[var(--scry-accent-text)]">
            <SlidersHorizontal className="h-[18px] w-[18px]" />
          </div>
          <span className="text-[16px] font-semibold text-[var(--scry-ink2)]">
            {t("discovery.filters")}
          </span>
          <button
            type="button"
            disabled={!hasActiveFilters}
            onClick={() => {
              onClear();
              onSearchValueChange("");
              onClearQuickFilters();
            }}
            className="text-xs font-medium text-[var(--scry-accent-ring)] transition disabled:cursor-default disabled:opacity-40"
          >
            {t("discovery.clearAll")}
          </button>
        </div>
        {onCollapse ? (
          <button
            type="button"
            onClick={onCollapse}
            className="flex size-7 shrink-0 items-center justify-center rounded-[7px] border border-[var(--scry-baccent)] bg-[var(--scry-inset)] text-[var(--scry-accent-text)] transition hover:bg-[var(--scry-hover)]"
            aria-label={t("label.close")}
            title={t("label.close")}
          >
            <PanelRightClose className="h-3.5 w-3.5" />
          </button>
        ) : null}
      </div>

      <div className="relative mb-4">
        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--scry-muted2)]" />
        <Input
          placeholder={t("title.filterPlaceholder")}
          value={searchValue}
          onChange={(event) => onSearchValueChange(event.target.value)}
          className="h-10 w-full rounded-[10px] border-[rgba(var(--scry-accent-rgb),0.38)] bg-[var(--scry-inset)] pl-9 text-[13px] text-[var(--scry-body)] shadow-none placeholder:text-[var(--scry-faint2)] focus-visible:ring-[var(--scry-focus)]"
        />
      </div>

      <div className="mb-4 border-b border-[var(--scry-border2)] pb-4">
        <TitleQuickFilterBar
          view={quickFilterView}
          filters={quickFilters}
          counts={quickFilterCounts}
          onToggleMonitoring={onToggleQuickMonitoring}
          onToggleStatus={onToggleQuickStatus}
          onClear={onClearQuickFilters}
          appearance="panel"
        />
      </div>

      {optionsError ? (
        <div
          role="alert"
          className="mb-4 flex items-center gap-2 rounded-[8px] border border-[rgba(255,112,112,0.3)] bg-[rgba(255,80,80,0.08)] px-2.5 py-2 text-[11.5px] text-[var(--scry-danger-text)]"
        >
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          <span className="min-w-0 flex-1">
            {t("title.catalogFilters.loadError")}
          </span>
          <button
            type="button"
            onClick={onRetryOptions}
            aria-label={t("title.catalogFilters.retry")}
            title={t("title.catalogFilters.retry")}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] text-[var(--scry-danger-text)] transition hover:bg-[rgba(255,255,255,0.08)]"
          >
            <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
          </button>
        </div>
      ) : null}

      <div className="grid gap-x-4 md:grid-cols-2">
        <div className="mb-4 min-w-0">
          <FilterLabel icon={<LibraryBig className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
            {t("settings.librariesLabel")}
          </FilterLabel>
          <LibraryMultiSelect
            libraries={libraries}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
            disabled={librariesLoading || libraries.length === 0}
            triggerClassName="h-9 w-full rounded-[9px] border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[12.5px]"
            contentClassName="max-w-[min(30rem,90vw)]"
          />
        </div>
        <div className="mb-4 min-w-0">
          <FilterLabel icon={<Folder className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
            {t("title.rootFolder")}
          </FilterLabel>
          <MultiSelectDropdown
            groups={rootGroups}
            selectedValues={filters.rootFolderIds}
            onSelectedValuesChange={handleRootFolderChange}
            triggerLabel={rootLabel}
            ariaLabel={t("title.rootFolder")}
            disabled={librariesLoading || rootGroups.length === 0}
            size="compact"
            chrome="toolbar"
          />
        </div>
      </div>

      <div className="grid gap-x-4 md:grid-cols-2">
        <div className="mb-4 min-w-0">
          <FilterLabel icon={<Film className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
            {t("discovery.genres")}
          </FilterLabel>
          <MultiSelectDropdown
            options={genreOptions}
            selectedValues={filters.genreTagKeys}
            onSelectedValuesChange={handleGenreChange}
            triggerLabel={
              filters.genreTagKeys.length === 0
                ? t("discovery.selectGenres")
                : filters.genreTagKeys.length === 1
                  ? (genreLabels.get(filters.genreTagKeys[0]) ??
                    filters.genreTagKeys[0])
                  : t("title.catalogFilters.selectedCount", {
                      count: filters.genreTagKeys.length,
                    })
            }
            ariaLabel={t("discovery.genres")}
            size="compact"
            chrome="toolbar"
          />
          <FilterChips
            values={filters.genreTagKeys}
            labels={genreLabels}
            onRemove={(key) =>
              onFiltersChange({
                genreTagKeys: filters.genreTagKeys.filter(
                  (candidate) => candidate !== key,
                ),
              })
            }
          />
        </div>
        {/* Themes are SMG-derived canonical tag keys, not user tags. They used
            to be labelled with the generic "Tags" wording, which now belongs to
            the administrator-defined registry below. */}
        <div className="mb-4 min-w-0">
          <FilterLabel icon={<Sparkles className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
            {t("title.catalogFilters.themes")}
          </FilterLabel>
          <MultiSelectDropdown
            options={themeOptions}
            selectedValues={filters.themeTagKeys}
            onSelectedValuesChange={handleThemeChange}
            triggerLabel={
              filters.themeTagKeys.length === 0
                ? t("title.catalogFilters.selectThemes")
                : filters.themeTagKeys.length === 1
                  ? (themeLabels.get(filters.themeTagKeys[0]) ??
                    filters.themeTagKeys[0])
                  : t("title.catalogFilters.selectedCount", {
                      count: filters.themeTagKeys.length,
                    })
            }
            ariaLabel={t("title.catalogFilters.themes")}
            size="compact"
            chrome="toolbar"
          />
          <FilterChips
            values={filters.themeTagKeys}
            labels={themeLabels}
            onRemove={(key) =>
              onFiltersChange({
                themeTagKeys: filters.themeTagKeys.filter(
                  (candidate) => candidate !== key,
                ),
              })
            }
          />
        </div>
      </div>

      {/* User tags: any-of, drawn from the administrator-defined registry. An
          empty registry has nothing to offer and no free text to fall back on,
          so the control says where tags come from instead. */}
      <div className="mb-4 min-w-0">
        <FilterLabel icon={<Tag className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
          {t("title.catalogFilters.userTags")}
        </FilterLabel>
        {userTagOptions.length === 0 && !tagDefinitionsLoading ? (
          <p className="text-[11.5px] text-[var(--scry-faint)]">
            {t("title.tagsEmptyRegistry")}
          </p>
        ) : (
          <MultiSelectDropdown
            options={userTagOptions}
            selectedValues={filters.userTagLabels}
            onSelectedValuesChange={handleUserTagChange}
            triggerLabel={
              filters.userTagLabels.length === 0
                ? t("title.catalogFilters.selectUserTags")
                : filters.userTagLabels.length === 1
                  ? filters.userTagLabels[0]
                  : t("title.catalogFilters.selectedCount", {
                      count: filters.userTagLabels.length,
                    })
            }
            ariaLabel={t("title.catalogFilters.userTags")}
            disabled={tagDefinitionsLoading}
            size="compact"
            chrome="toolbar"
          />
        )}
        <FilterChips
          values={filters.userTagLabels}
          labels={userTagLabels}
          onRemove={(label) =>
            onFiltersChange({
              userTagLabels: filters.userTagLabels.filter(
                (candidate) => candidate !== label,
              ),
            })
          }
        />
      </div>

      <div className="mb-2.5 flex items-center justify-between">
        <FilterLabel icon={<CalendarDays className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
          {t("discovery.releaseYear")}
        </FilterLabel>
        <span className="mb-2.5 text-[11.5px] text-[var(--scry-faint)]">
          {minimumYear} - {maximumYear}
        </span>
      </div>
      <div className="relative mb-5 h-5">
        <div className="absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[var(--scry-border2)]" />
        <div
          className="absolute top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-gradient-to-r from-[var(--scry-accent)] to-[var(--scry-accent-ring)]"
          style={{
            left: `${minimumYearPercent}%`,
            right: `${100 - maximumYearPercent}%`,
          }}
        />
        <input
          type="range"
          min={minimumYearBound}
          max={maximumYearBound}
          value={minimumYear}
          aria-label={t("title.catalogFilters.minimumYear")}
          onChange={(event) => {
            const value = Math.min(Number(event.target.value), maximumYear);
            onFiltersChange({
              minimumYear: value === minimumYearBound ? null : value,
            });
          }}
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2 bg-transparent",
            FILTER_RANGE_CLASS_NAME,
            FILTER_RANGE_THUMB_POINTER_CLASS_NAME,
          )}
        />
        <input
          type="range"
          min={minimumYearBound}
          max={maximumYearBound}
          value={maximumYear}
          aria-label={t("title.catalogFilters.maximumYear")}
          onChange={(event) => {
            const value = Math.max(Number(event.target.value), minimumYear);
            onFiltersChange({
              maximumYear: value === maximumYearBound ? null : value,
            });
          }}
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2 bg-transparent",
            FILTER_RANGE_CLASS_NAME,
            FILTER_RANGE_THUMB_POINTER_CLASS_NAME,
          )}
        />
      </div>

      <div className="mb-2.5 flex items-center justify-between">
        <FilterLabel icon={<Star className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}>
          {t("discovery.minimumRating")}
        </FilterLabel>
        <span className="mb-2.5 text-[11.5px] font-bold text-[var(--scry-accent-ring)]">
          {minimumRating.toFixed(1)}+
        </span>
      </div>
      <div className="relative h-5">
        <div className="absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[var(--scry-border2)]" />
        <div
          className="absolute left-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-gradient-to-r from-[var(--scry-accent)] to-[var(--scry-accent-ring)]"
          style={{ width: `${minimumRating * 10}%` }}
        />
        <input
          type="range"
          min={0}
          max={10}
          step={0.5}
          value={minimumRating}
          aria-label={t("discovery.minimumRating")}
          onChange={(event) => {
            const value = Number(event.target.value);
            onFiltersChange({ minimumRating: value > 0 ? value : null });
          }}
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2",
            FILTER_RANGE_CLASS_NAME,
          )}
        />
      </div>
    </aside>
  );
}
