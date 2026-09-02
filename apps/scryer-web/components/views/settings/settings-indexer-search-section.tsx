// Indexers › Search — the aggregate, title-less search surface (spec 0002).
// Every row, health dot and facet count is re-derived from the snapshot the
// interactive-search job hands back on each poll, so the table refines live
// while indexers are still answering; nothing here waits for the job to finish.
import {
  Activity,
  ArrowDownWideNarrow,
  Bookmark,
  ChevronDown,
  Database,
  Download,
  ExternalLink,
  Film,
  FolderTree,
  Funnel,
  RefreshCw,
  ScanSearch,
  Search,
  SlidersHorizontal,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { MultiSelectDropdown } from "@/components/ui/multi-select-dropdown";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type {
  InteractiveSearchIndexerProgress,
  InteractiveSearchKind,
} from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";
import { cn } from "@/lib/utils";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  indexerSearchResultExpandId,
  indexerSearchResultGrabId,
  indexerSearchResultRowId,
  indexerSearchResultSelectId,
  selectorId,
} from "@/lib/utils/dom-ids";
import {
  formatReleaseAge,
  formatReleaseSize,
  indexerHealthTone,
  indexerSearchRowKey,
  isReleaseRejected,
  releaseAgeMs,
  releaseBlockCode,
  releaseProtocol,
  summarizeIndexerHealth,
  totalReleaseBytes,
  type IndexerHealthTone,
  type IndexerSearchFacetGroup,
  type IndexerSearchSortKey,
} from "@/lib/utils/indexer-search";

export type IndexerSearchIndexerOption = {
  id: string;
  name: string;
};

export type IndexerSearchAdvancedLimits = {
  minSizeGiB: string;
  maxSizeGiB: string;
  minSeeders: string;
  maxAgeDays: string;
  limit: string;
};

export type SettingsIndexerSearchSectionProps = {
  query: string;
  onQueryChange: (query: string) => void;
  kind: InteractiveSearchKind;
  onKindChange: (kind: InteractiveSearchKind) => void;
  indexerOptions: IndexerSearchIndexerOption[];
  selectedIndexerIds: string[];
  onSelectedIndexerIdsChange: (indexerIds: string[]) => void;
  categories: string;
  onCategoriesChange: (categories: string) => void;
  advancedOpen: boolean;
  onAdvancedOpenChange: (open: boolean) => void;
  advanced: IndexerSearchAdvancedLimits;
  onAdvancedChange: (advanced: IndexerSearchAdvancedLimits) => void;
  savedSearchLabels: string[];
  onSaveSearch: () => void;
  onApplySavedSearch: (index: number) => void;
  onRemoveSavedSearch: (index: number) => void;
  onSearch: () => void;
  onCancelSearch: () => void;
  searching: boolean;
  hasSearched: boolean;
  indexers: InteractiveSearchIndexerProgress[];
  onRetryFailed: () => void;
  facetGroups: IndexerSearchFacetGroup[];
  selectedFacets: string[];
  onToggleFacet: (facetKey: string) => void;
  onResetRefine: () => void;
  sizeBoundsGiB: [number, number] | null;
  sizeRangeGiB: [number, number] | null;
  onSizeRangeChange: (range: [number, number] | null) => void;
  sort: IndexerSearchSortKey;
  onSortChange: (sort: IndexerSearchSortKey) => void;
  matchedCount: number;
  passingCount: number;
  rows: Release[];
  priorityByIndexer: ReadonlyMap<string, number>;
  nowMs: number;
  selectedRowKeys: string[];
  onToggleRow: (release: Release) => void;
  expandedRowKey: string | null;
  onToggleExpanded: (release: Release) => void;
  /** WP5 owns the grab dialog; this pane only names the releases to grab. */
  onGrab: (releases: Release[]) => void;
};

const SEARCH_KINDS: InteractiveSearchKind[] = [
  "MOVIE",
  "SERIES",
  "ANIME",
  "RAW",
];

const KIND_LABEL_KEYS: Record<InteractiveSearchKind, string> = {
  MOVIE: "indexerSearch.kind.movie",
  SERIES: "indexerSearch.kind.series",
  ANIME: "indexerSearch.kind.anime",
  RAW: "indexerSearch.kind.raw",
};

const SORT_KEYS: IndexerSearchSortKey[] = [
  "newest",
  "size",
  "age",
  "seeders",
  "priority",
];

const SORT_LABEL_KEYS: Record<IndexerSearchSortKey, string> = {
  newest: "indexerSearch.sort.newest",
  size: "indexerSearch.sort.size",
  age: "indexerSearch.sort.age",
  seeders: "indexerSearch.sort.seeders",
  priority: "indexerSearch.sort.priority",
};

const HEALTH_DOT_CLASS: Record<IndexerHealthTone, string> = {
  ok: "bg-[var(--scry-success-solid)]",
  slow: "bg-[var(--scry-warning-solid)]",
  failed: "bg-[var(--scry-danger-solid)]",
  skipped: "bg-[var(--scry-faint3)]",
  pending: "bg-[var(--scry-faint4)]",
};

const HEALTH_COUNT_CLASS: Record<IndexerHealthTone, string> = {
  ok: "text-[var(--scry-ink2)]",
  slow: "text-[var(--scry-ink2)]",
  failed: "text-[var(--scry-danger-text-soft)]",
  skipped: "text-[var(--scry-muted3)]",
  pending: "text-[var(--scry-muted3)]",
};

type BadgeTone = "neutral" | "accent" | "success" | "warning" | "danger";

const BADGE_TONE_CLASS: Record<BadgeTone, string> = {
  neutral:
    "bg-[var(--scry-chip)] border-[var(--scry-border2)] text-[var(--scry-text4)]",
  accent:
    "bg-[rgba(var(--scry-accent-rgb),0.13)] border-[rgba(var(--scry-accent-rgb),0.3)] text-[var(--scry-accent-text)]",
  success:
    "bg-[var(--scry-success-bg)] border-[var(--scry-success-border)] text-[var(--scry-success-text-soft)]",
  warning:
    "bg-[var(--scry-warning-bg)] border-[var(--scry-warning-border)] text-[var(--scry-warning-text)]",
  danger:
    "bg-[var(--scry-danger-bg)] border-[var(--scry-danger-border)] text-[var(--scry-danger-text-soft)]",
};

const RESULT_GRID_CLASS =
  "grid grid-cols-[34px_1fr_140px_92px_80px_112px_74px] items-center gap-2.5 px-4";

/** At most three badges per row; the rest live in the expanded detail. */
const MAX_ROW_BADGES = 3;

function ReleaseBadge({ text, tone }: { text: string; tone: BadgeTone }) {
  return (
    <span
      className={cn(
        "rounded-[5px] border px-[7px] py-px text-[10.5px] font-semibold",
        BADGE_TONE_CLASS[tone],
      )}
    >
      {text}
    </span>
  );
}

function ProtocolBadge({ release }: { release: Release }) {
  const t = useTranslate();
  const protocol = releaseProtocol(release);
  if (!protocol) {
    return null;
  }
  return (
    <span
      className={cn(
        "shrink-0 rounded-[5px] border px-1.5 py-px text-[9.5px] font-extrabold tracking-[0.04em]",
        protocol === "usenet"
          ? "border-[rgba(var(--scry-accent-rgb),0.32)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-text)]"
          : "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text-soft)]",
      )}
    >
      {t(
        protocol === "usenet"
          ? "indexerSearch.protocol.nzb"
          : "indexerSearch.protocol.torrent",
      )}
    </span>
  );
}

function useReleaseBadges(release: Release): { text: string; tone: BadgeTone }[] {
  const t = useTranslate();
  const badges: { text: string; tone: BadgeTone }[] = [];
  const blockCode = releaseBlockCode(release);
  if (blockCode) {
    badges.push({
      text: t("indexerSearch.row.rejected", { code: blockCode }),
      tone: "danger",
    });
  }
  const parsed = release.parsedRelease;
  if (parsed?.quality) {
    badges.push({ text: parsed.quality, tone: "accent" });
  }
  if (parsed?.isRemux) {
    badges.push({ text: "REMUX", tone: "neutral" });
  } else if (parsed?.source) {
    badges.push({ text: parsed.source, tone: "neutral" });
  }
  if (parsed?.isDolbyVision) {
    badges.push({ text: t("indexerSearch.facet.dolbyVision"), tone: "accent" });
  } else if (parsed?.detectedHdr) {
    badges.push({ text: t("indexerSearch.facet.hdr"), tone: "accent" });
  }
  if (parsed?.isAtmos) {
    badges.push({ text: t("indexerSearch.facet.atmos"), tone: "neutral" });
  }
  if (release.freeleech === true) {
    badges.push({ text: t("indexerSearch.facet.freeleech"), tone: "success" });
  }
  if (parsed?.isProperUpload) {
    badges.push({ text: t("indexerSearch.facet.proper"), tone: "warning" });
  }
  return badges.slice(0, MAX_ROW_BADGES);
}

function AdvancedField({
  id,
  labelKey,
  unitKey,
  value,
  onChange,
}: {
  id: string;
  labelKey: string;
  unitKey?: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const t = useTranslate();
  return (
    <div className="min-w-0">
      <label
        htmlFor={id}
        className="mb-1.5 block text-[11px] font-bold uppercase tracking-[0.05em] text-[var(--scry-faint2)]"
      >
        {t(labelKey)}
      </label>
      <div className="flex h-[38px] items-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3">
        <Input
          id={id}
          type="number"
          inputMode="numeric"
          min={0}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          className="h-auto w-full border-0 bg-transparent p-0 text-[13px] tabular-nums shadow-none focus-visible:ring-0"
        />
        {unitKey ? (
          <span className="shrink-0 text-[11px] text-[var(--scry-faint3)]">
            {t(unitKey)}
          </span>
        ) : null}
      </div>
    </div>
  );
}

function QueryCard({
  query,
  onQueryChange,
  kind,
  onKindChange,
  indexerOptions,
  selectedIndexerIds,
  onSelectedIndexerIdsChange,
  categories,
  onCategoriesChange,
  advancedOpen,
  onAdvancedOpenChange,
  advanced,
  onAdvancedChange,
  savedSearchLabels,
  onSaveSearch,
  onApplySavedSearch,
  onRemoveSavedSearch,
  onSearch,
  onCancelSearch,
  searching,
}: Pick<
  SettingsIndexerSearchSectionProps,
  | "query"
  | "onQueryChange"
  | "kind"
  | "onKindChange"
  | "indexerOptions"
  | "selectedIndexerIds"
  | "onSelectedIndexerIdsChange"
  | "categories"
  | "onCategoriesChange"
  | "advancedOpen"
  | "onAdvancedOpenChange"
  | "advanced"
  | "onAdvancedChange"
  | "savedSearchLabels"
  | "onSaveSearch"
  | "onApplySavedSearch"
  | "onRemoveSavedSearch"
  | "onSearch"
  | "onCancelSearch"
  | "searching"
>) {
  const t = useTranslate();
  const chipClassName =
    "flex h-8 items-center gap-2 whitespace-nowrap rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-chip2)] px-3 text-[12.5px] text-[var(--scry-text3)] transition hover:bg-[var(--scry-hover)]";
  const indexerChipLabel =
    selectedIndexerIds.length === 0
      ? t("indexerSearch.scope.indexersAll", { count: indexerOptions.length })
      : t("indexerSearch.scope.indexersSome", {
          count: selectedIndexerIds.length,
        });

  return (
    <div className="mb-3.5 rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surf)] p-4">
      <div className="flex flex-wrap items-center gap-2.5">
        <div className="flex h-[46px] min-w-[280px] flex-1 items-center gap-2.5 rounded-[11px] border border-[var(--scry-baccent)] bg-[var(--scry-bg)] px-3.5 shadow-[0_0_0_3px_rgba(var(--scry-accent-rgb),0.10)]">
          <Search className="h-[17px] w-[17px] shrink-0 text-[var(--scry-accent-text)]" />
          <Input
            id="indexer-search-query"
            value={query}
            placeholder={t("indexerSearch.queryPlaceholder")}
            onChange={(event) => onQueryChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                onSearch();
              }
            }}
            className="h-auto w-full border-0 bg-transparent p-0 text-[14.5px] text-[var(--scry-ink2)] shadow-none focus-visible:ring-0"
          />
          {query ? (
            <IconButton
              id="indexer-search-query-clear"
              label={t("label.clear")}
              tone="neutral"
              className="h-[22px] w-[22px] rounded-[6px]"
              onClick={() => onQueryChange("")}
            >
              <X className="h-3 w-3" />
            </IconButton>
          ) : null}
        </div>
        <Select
          value={kind}
          onValueChange={(value) => onKindChange(value as InteractiveSearchKind)}
        >
          <SelectTrigger
            id="indexer-search-kind"
            size="large"
            chrome="dialog"
            aria-label={t("indexerSearch.kindLabel")}
            className="h-[46px] min-w-[168px]"
          >
            <Film className="h-[15px] w-[15px] text-[var(--scry-faint)]" />
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SEARCH_KINDS.map((option) => (
              <SelectItem key={option} value={option}>
                {t(KIND_LABEL_KEYS[option])}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {searching ? (
          <Button
            id="indexer-search-cancel"
            type="button"
            variant="outline"
            onClick={onCancelSearch}
            className="h-[46px] rounded-[11px] px-5 text-[13.5px]"
          >
            {t("label.cancel")}
          </Button>
        ) : (
          <Button
            id="indexer-search-run"
            type="button"
            disabled={query.trim().length === 0}
            onClick={onSearch}
            className="h-[46px] rounded-[11px] border-0 bg-[linear-gradient(135deg,var(--scry-accent),var(--scry-accent2))] px-[22px] text-[13.5px] font-semibold text-white shadow-[0_8px_20px_rgba(var(--scry-accent-rgb),0.32)] hover:opacity-90"
          >
            <ScanSearch className="h-4 w-4" />
            {t("label.search")}
          </Button>
        )}
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <MultiSelectDropdown
          id="indexer-search-scope-indexers"
          size="compact"
          chrome="toolbar"
          ariaLabel={t("indexerSearch.scope.indexers")}
          className="h-8 w-auto min-w-[210px] rounded-[8px] bg-[var(--scry-chip2)]"
          options={indexerOptions.map((option) => ({
            value: option.id,
            label: option.name,
            id: selectorId("indexer-search-scope-indexer", option.name),
          }))}
          selectedValues={selectedIndexerIds}
          onSelectedValuesChange={onSelectedIndexerIdsChange}
          allOption={{
            id: "indexer-search-scope-indexers-all",
            label: t("indexerSearch.scope.indexersAllOption"),
            selected: selectedIndexerIds.length === 0,
            onSelect: () => onSelectedIndexerIdsChange([]),
          }}
          triggerLabel={
            <span className="flex items-center gap-2">
              <Database className="h-3.5 w-3.5 text-[var(--scry-faint)]" />
              <span className="text-[var(--scry-faint2)]">
                {t("indexerSearch.scope.indexers")}
              </span>
              {indexerChipLabel}
            </span>
          }
        />
        <Popover>
          <PopoverTrigger asChild>
            <button
              id="indexer-search-scope-categories"
              type="button"
              className={chipClassName}
            >
              <FolderTree className="h-3.5 w-3.5 text-[var(--scry-faint)]" />
              <span className="text-[var(--scry-faint2)]">
                {t("indexerSearch.scope.categories")}
              </span>
              {categories.trim()
                ? categories.trim()
                : t("indexerSearch.scope.categoriesDefault")}
              <ChevronDown className="h-3.5 w-3.5 text-[var(--scry-faint3)]" />
            </button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-[280px] space-y-2 p-3">
            <label
              htmlFor="indexer-search-categories-input"
              className="block text-[12px] font-semibold text-[var(--scry-ink2)]"
            >
              {t("indexerSearch.scope.categories")}
            </label>
            <Input
              id="indexer-search-categories-input"
              value={categories}
              inputMode="numeric"
              placeholder={t("indexerSearch.scope.categoriesDefault")}
              onChange={(event) => onCategoriesChange(event.target.value)}
            />
            <p className="text-[11px] text-[var(--scry-faint2)]">
              {t("indexerSearch.scope.categoriesHelp")}
            </p>
          </PopoverContent>
        </Popover>
        <div className="min-w-2 flex-1" />
        <button
          id="indexer-search-advanced-toggle"
          type="button"
          aria-expanded={advancedOpen}
          onClick={() => onAdvancedOpenChange(!advancedOpen)}
          className={cn(
            chipClassName,
            "font-semibold",
            advancedOpen &&
              "border-[rgba(var(--scry-accent-rgb),0.34)] bg-[rgba(var(--scry-accent-rgb),0.16)] text-[var(--scry-accent-text)]",
          )}
        >
          <SlidersHorizontal className="h-3.5 w-3.5" />
          {t("indexerSearch.advanced")}
        </button>
        <Popover>
          <PopoverTrigger asChild>
            <button
              id="indexer-search-saved"
              type="button"
              className={chipClassName}
            >
              <Bookmark className="h-3.5 w-3.5 text-[var(--scry-faint)]" />
              {t("indexerSearch.saveSearch")}
            </button>
          </PopoverTrigger>
          <PopoverContent align="end" className="w-[300px] space-y-2 p-3">
            <Button
              id="indexer-search-saved-add"
              type="button"
              variant="outline"
              size="sm"
              className="w-full"
              disabled={query.trim().length === 0}
              onClick={onSaveSearch}
            >
              <Bookmark className="h-3.5 w-3.5" />
              {t("indexerSearch.saveCurrentSearch")}
            </Button>
            {savedSearchLabels.length === 0 ? (
              <p className="text-[12px] text-[var(--scry-muted3)]">
                {t("indexerSearch.savedSearchesEmpty")}
              </p>
            ) : (
              <ul className="space-y-1">
                {savedSearchLabels.map((label, index) => (
                  <li key={`${index}:${label}`} className="flex items-center gap-1">
                    <button
                      id={selectorId("indexer-search-saved-apply", label)}
                      type="button"
                      onClick={() => onApplySavedSearch(index)}
                      className="min-w-0 flex-1 truncate rounded-[7px] px-2 py-1 text-left text-[12.5px] text-[var(--scry-text3)] hover:bg-[var(--scry-hover)]"
                    >
                      {label}
                    </button>
                    <IconButton
                      id={selectorId("indexer-search-saved-remove", label)}
                      label={t("label.remove")}
                      tone="neutral"
                      className="h-7 w-7"
                      onClick={() => onRemoveSavedSearch(index)}
                    >
                      <X className="h-3.5 w-3.5" />
                    </IconButton>
                  </li>
                ))}
              </ul>
            )}
          </PopoverContent>
        </Popover>
      </div>

      {advancedOpen ? (
        <div className="mt-3.5 grid grid-cols-1 gap-3 border-t border-[var(--scry-line)] pt-3.5 sm:grid-cols-2 xl:grid-cols-5">
          <AdvancedField
            id="indexer-search-min-size"
            labelKey="indexerSearch.advanced.minSize"
            unitKey="indexerSearch.unit.gib"
            value={advanced.minSizeGiB}
            onChange={(value) =>
              onAdvancedChange({ ...advanced, minSizeGiB: value })
            }
          />
          <AdvancedField
            id="indexer-search-max-size"
            labelKey="indexerSearch.advanced.maxSize"
            unitKey="indexerSearch.unit.gib"
            value={advanced.maxSizeGiB}
            onChange={(value) =>
              onAdvancedChange({ ...advanced, maxSizeGiB: value })
            }
          />
          <AdvancedField
            id="indexer-search-min-seeders"
            labelKey="indexerSearch.advanced.minSeeders"
            value={advanced.minSeeders}
            onChange={(value) =>
              onAdvancedChange({ ...advanced, minSeeders: value })
            }
          />
          <AdvancedField
            id="indexer-search-max-age"
            labelKey="indexerSearch.advanced.maxAge"
            unitKey="indexerSearch.unit.days"
            value={advanced.maxAgeDays}
            onChange={(value) =>
              onAdvancedChange({ ...advanced, maxAgeDays: value })
            }
          />
          <AdvancedField
            id="indexer-search-limit"
            labelKey="indexerSearch.advanced.limit"
            unitKey="indexerSearch.unit.perIndexer"
            value={advanced.limit}
            onChange={(value) => onAdvancedChange({ ...advanced, limit: value })}
          />
        </div>
      ) : null}
    </div>
  );
}

function HealthLine({
  indexers,
  matchedCount,
  onToggleFacet,
  selectedFacets,
  onRetryFailed,
  searching,
}: Pick<
  SettingsIndexerSearchSectionProps,
  | "indexers"
  | "matchedCount"
  | "onToggleFacet"
  | "selectedFacets"
  | "onRetryFailed"
  | "searching"
>) {
  const t = useTranslate();
  const summary = summarizeIndexerHealth(indexers);
  const hasFailures = summary.failedIndexerIds.length > 0;

  return (
    <div className="mb-3 flex flex-wrap items-center gap-3 px-0.5">
      <span className="flex items-center gap-1.5 whitespace-nowrap text-[12.5px] text-[var(--scry-muted2)]">
        <Activity className="h-3.5 w-3.5 text-[var(--scry-success-text-soft)]" />
        <strong className="font-semibold text-[var(--scry-ink2)]">
          {t("indexerSearch.health.releases", { count: matchedCount })}
        </strong>
        <span>·</span>
        <span id="indexer-search-health-indexers">
          {t("indexerSearch.health.indexers", {
            answered: summary.answered,
            total: summary.total,
          })}
        </span>
        {summary.elapsedMs > 0 ? (
          <>
            <span>·</span>
            <span className="tabular-nums">
              {t("indexerSearch.health.elapsed", {
                seconds: (summary.elapsedMs / 1000).toFixed(1),
              })}
            </span>
          </>
        ) : null}
        {summary.pending > 0 ? (
          <>
            <span>·</span>
            <span
              id="indexer-search-health-pending"
              className="text-[var(--scry-warning-text)]"
            >
              {t("indexerSearch.health.stillSearching", {
                count: summary.pending,
                total: summary.total,
              })}
            </span>
          </>
        ) : null}
      </span>
      <span className="h-3 w-px bg-[var(--scry-border2)]" />
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-3">
        {indexers.map((entry) => {
          const tone = indexerHealthTone(entry);
          const facetKey = `indexer:${entry.name}`;
          const active = selectedFacets.includes(facetKey);
          return (
            <button
              key={entry.indexerId}
              id={selectorId("indexer-search-health", entry.name)}
              type="button"
              title={entry.failureReason ?? undefined}
              aria-pressed={active}
              onClick={() => onToggleFacet(facetKey)}
              className={cn(
                "flex items-center gap-1.5 whitespace-nowrap rounded-[6px] px-1 py-0.5 text-[11.5px] text-[var(--scry-faint)] transition hover:bg-[var(--scry-hover)]",
                active && "bg-[rgba(var(--scry-accent-rgb),0.12)]",
              )}
            >
              <span
                className={cn(
                  "h-1.5 w-1.5 shrink-0 rounded-full",
                  HEALTH_DOT_CLASS[tone],
                )}
              />
              {entry.name}
              <span className={cn("tabular-nums", HEALTH_COUNT_CLASS[tone])}>
                {tone === "failed"
                  ? t("indexerSearch.health.failed")
                  : tone === "skipped"
                    ? t("indexerSearch.health.skipped")
                    : tone === "pending"
                      ? "…"
                      : entry.resultCount}
              </span>
            </button>
          );
        })}
      </div>
      {hasFailures && !searching ? (
        <button
          id="indexer-search-retry-failed"
          type="button"
          onClick={onRetryFailed}
          className="flex items-center gap-1.5 whitespace-nowrap text-[11.5px] font-semibold text-[var(--scry-accent-text)] hover:underline"
        >
          <RefreshCw className="h-3 w-3" />
          {t("indexerSearch.health.retryFailed")}
        </button>
      ) : null}
    </div>
  );
}

function RefineRail({
  facetGroups,
  selectedFacets,
  onToggleFacet,
  onResetRefine,
  sizeBoundsGiB,
  sizeRangeGiB,
  onSizeRangeChange,
}: Pick<
  SettingsIndexerSearchSectionProps,
  | "facetGroups"
  | "selectedFacets"
  | "onToggleFacet"
  | "onResetRefine"
  | "sizeBoundsGiB"
  | "sizeRangeGiB"
  | "onSizeRangeChange"
>) {
  const t = useTranslate();
  const [low, high] = sizeRangeGiB ?? sizeBoundsGiB ?? [0, 0];

  return (
    <aside className="w-full shrink-0 self-start rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] pb-2.5 pt-1.5 lg:sticky lg:top-0 lg:w-[232px]">
      <div className="flex items-center justify-between border-b border-[var(--scry-line)] px-3.5 pb-2.5 pt-3">
        <span className="flex items-center gap-1.5 text-[12px] font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
          <Funnel className="h-3.5 w-3.5" />
          {t("indexerSearch.refine.title")}
        </span>
        <button
          id="indexer-search-refine-reset"
          type="button"
          onClick={onResetRefine}
          className="text-[11.5px] text-[var(--scry-accent-text)] hover:underline"
        >
          {t("indexerSearch.refine.reset")}
        </button>
      </div>
      {facetGroups.map((group) => (
        <div
          key={group.key}
          className="border-b border-[var(--scry-line2)] px-3.5 pb-2.5 pt-3"
        >
          <div className="mb-2 text-[11px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
            {t(group.labelKey)}
          </div>
          <div className="flex flex-col gap-0.5">
            {group.items.map((item) => {
              const checked = selectedFacets.includes(item.key);
              return (
                <label
                  key={item.key}
                  className={cn(
                    "-mx-1.5 flex cursor-pointer items-center gap-2 rounded-[7px] px-1.5 py-1 transition hover:bg-[var(--scry-rowHover)]",
                    checked && "bg-[rgba(var(--scry-accent-rgb),0.10)]",
                  )}
                >
                  <Checkbox
                    id={selectorId("indexer-search-facet", group.key, item.value)}
                    size="compact"
                    className="size-[15px]"
                    checked={checked}
                    onCheckedChange={() => onToggleFacet(item.key)}
                  />
                  <span
                    className={cn(
                      "min-w-0 flex-1 truncate text-[12.5px]",
                      checked
                        ? "text-[var(--scry-ink3)]"
                        : "text-[var(--scry-text4)]",
                    )}
                  >
                    {item.labelKey ? t(item.labelKey) : item.label}
                  </span>
                  <span className="text-[11px] tabular-nums text-[var(--scry-faint3)]">
                    {item.count}
                  </span>
                </label>
              );
            })}
          </div>
        </div>
      ))}
      {sizeBoundsGiB ? (
        <div className="px-3.5 pb-1 pt-3">
          <div className="mb-2 flex items-center justify-between">
            <span className="text-[11px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
              {t("indexerSearch.facet.size")}
            </span>
            <span className="text-[11px] tabular-nums text-[var(--scry-text4)]">
              {t("indexerSearch.sizeRange", {
                min: low.toFixed(1),
                max: high.toFixed(1),
              })}
            </span>
          </div>
          <div className="flex flex-col gap-2">
            <input
              id="indexer-search-size-min"
              type="range"
              aria-label={t("indexerSearch.advanced.minSize")}
              min={sizeBoundsGiB[0]}
              max={sizeBoundsGiB[1]}
              step={0.1}
              value={low}
              onChange={(event) =>
                onSizeRangeChange([
                  Math.min(Number(event.target.value), high),
                  high,
                ])
              }
              className="h-1 w-full accent-[var(--scry-accent)]"
            />
            <input
              id="indexer-search-size-max"
              type="range"
              aria-label={t("indexerSearch.advanced.maxSize")}
              min={sizeBoundsGiB[0]}
              max={sizeBoundsGiB[1]}
              step={0.1}
              value={high}
              onChange={(event) =>
                onSizeRangeChange([
                  low,
                  Math.max(Number(event.target.value), low),
                ])
              }
              className="h-1 w-full accent-[var(--scry-accent)]"
            />
          </div>
        </div>
      ) : null}
    </aside>
  );
}

function ReleaseDetail({ release }: { release: Release }) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const fields = [
    {
      key: "publishedAt",
      label: t("indexerSearch.detail.publishDate"),
      value: formatUiDateTime(release.publishedAt, dateTimeFormat, {
        fallback: t("indexerSearch.detail.notAvailable"),
      }),
    },
    {
      key: "releaseGroup",
      label: t("indexerSearch.detail.releaseGroup"),
      value:
        release.parsedRelease?.releaseGroup ??
        t("indexerSearch.detail.notAvailable"),
    },
  ];

  return (
    <div className="border-b border-[var(--scry-border)] bg-[var(--scry-inset)] py-[18px] pl-4 pr-4 md:pl-[52px]">
      <div className="mb-3 text-[10.5px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
        {t("indexerSearch.detail.title")}
      </div>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(180px,1fr))] gap-x-7 gap-y-2">
        {fields.map((field) => (
          <div
            key={field.key}
            className="flex min-w-0 flex-col gap-1 border-b border-[var(--scry-line2)] py-1.5"
          >
            <span className="text-[11px] text-[var(--scry-faint)]">
              {field.label}
            </span>
            <span className="truncate text-[12.5px] tabular-nums text-[var(--scry-text2)]">
              {field.value}
            </span>
          </div>
        ))}
      </div>
      {release.link ? (
        <a
          id={selectorId(
            "indexer-search-open",
            release.source ?? "indexer",
            release.title,
          )}
          href={release.link}
          target="_blank"
          rel="noreferrer noopener"
          className="mt-2.5 inline-flex items-center gap-1.5 text-[11.5px] font-semibold text-[var(--scry-accent-text)] hover:underline"
        >
          <ExternalLink className="h-3 w-3" />
          {t("indexerSearch.detail.openOnIndexer")}
        </a>
      ) : null}
    </div>
  );
}

function ReleaseRow({
  release,
  selected,
  expanded,
  priority,
  nowMs,
  onToggleRow,
  onToggleExpanded,
  onGrab,
}: {
  release: Release;
  selected: boolean;
  expanded: boolean;
  priority: number | undefined;
  nowMs: number;
  onToggleRow: (release: Release) => void;
  onToggleExpanded: (release: Release) => void;
  onGrab: (releases: Release[]) => void;
}) {
  const t = useTranslate();
  const badges = useReleaseBadges(release);
  const rejected = isReleaseRejected(release);
  const age = formatReleaseAge(releaseAgeMs(release, nowMs));
  const peers =
    release.seeders != null
      ? `${release.seeders} / ${release.peers ?? 0}`
      : release.grabs != null
        ? t("indexerSearch.row.grabs", { count: release.grabs })
        : "—";

  return (
    <div>
      <div
        id={indexerSearchResultRowId(release)}
        data-ui="indexer-search-row"
        className={cn(
          RESULT_GRID_CLASS,
          "border-b border-l-2 border-b-[var(--scry-line2)] border-l-transparent py-3 hover:bg-[var(--scry-rowHover)]",
          rejected && "border-l-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)]",
          selected &&
            "border-l-[var(--scry-accent)] bg-[rgba(var(--scry-accent-rgb),0.07)]",
        )}
      >
        <Checkbox
          id={indexerSearchResultSelectId(release)}
          size="table"
          aria-label={t("indexerSearch.row.select")}
          checked={selected}
          onCheckedChange={() => onToggleRow(release)}
        />
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-2">
            <ProtocolBadge release={release} />
            <button
              type="button"
              onClick={() => onToggleExpanded(release)}
              className="min-w-0 truncate text-left text-[13.5px] font-semibold text-[var(--scry-ink3)] hover:underline"
            >
              {release.title}
            </button>
          </div>
          {badges.length > 0 ? (
            <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
              {badges.map((badge) => (
                <ReleaseBadge
                  key={badge.text}
                  text={badge.text}
                  tone={badge.tone}
                />
              ))}
            </div>
          ) : null}
        </div>
        <div className="min-w-0">
          <div className="truncate text-[12.5px] text-[var(--scry-text2)]">
            {release.source ?? "—"}
          </div>
          {priority != null ? (
            <div className="mt-1 text-[11px] text-[var(--scry-faint2)]">
              {t("indexerSearch.row.priority", { value: priority })}
            </div>
          ) : null}
        </div>
        <span className="text-right text-[13px] font-semibold tabular-nums text-[var(--scry-ink3)]">
          {formatReleaseSize(release.sizeBytes)}
        </span>
        <span className="text-right text-[12.5px] tabular-nums text-[var(--scry-muted2)]">
          {age ? t(age.unitKey, { count: age.value }) : "—"}
        </span>
        <span
          className={cn(
            "text-right text-[12.5px] tabular-nums",
            rejected
              ? "text-[var(--scry-danger-text-soft)]"
              : "text-[var(--scry-muted2)]",
          )}
        >
          {peers}
        </span>
        <div className="flex justify-end gap-1.5">
          <IconButton
            id={indexerSearchResultGrabId(release)}
            label={t("indexerSearch.row.grab")}
            tone="enabled"
            className="h-[29px] w-[29px]"
            onClick={() => onGrab([release])}
          >
            <Download className="h-3.5 w-3.5" />
          </IconButton>
          <IconButton
            id={indexerSearchResultExpandId(release)}
            label={
              expanded
                ? t("indexerSearch.row.collapse")
                : t("indexerSearch.row.expand")
            }
            tone="neutral"
            aria-expanded={expanded}
            className="h-[29px] w-[29px]"
            onClick={() => onToggleExpanded(release)}
          >
            <ChevronDown
              className={cn("h-3.5 w-3.5 transition", expanded && "rotate-180")}
            />
          </IconButton>
        </div>
      </div>
      {expanded ? <ReleaseDetail release={release} /> : null}
    </div>
  );
}

export function SettingsIndexerSearchSection(
  props: SettingsIndexerSearchSectionProps,
) {
  const t = useTranslate();
  const {
    hasSearched,
    searching,
    indexers,
    matchedCount,
    passingCount,
    rows,
    sort,
    onSortChange,
    priorityByIndexer,
    nowMs,
    selectedRowKeys,
    onToggleRow,
    expandedRowKey,
    onToggleExpanded,
    onGrab,
  } = props;

  const selectedKeys = new Set(selectedRowKeys);
  const selectedReleases = rows.filter((release) =>
    selectedKeys.has(indexerSearchRowKey(release)),
  );
  const hasSelection = selectedReleases.length > 0;

  return (
    <div className="w-full min-w-0 max-w-full">
      <QueryCard {...props} />
      {hasSearched ? <HealthLine {...props} /> : null}
      <div className="flex w-full min-w-0 max-w-full flex-col items-start gap-4 lg:flex-row">
        {hasSearched && props.facetGroups.length > 0 ? (
          <RefineRail {...props} />
        ) : null}
        <div className="w-full min-w-0 max-w-full flex-1 overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)]">
          <div className="flex flex-wrap items-center gap-3 border-b border-[var(--scry-border)] px-4 py-3">
            <span
              id="indexer-search-totals"
              className="text-[13px] text-[var(--scry-text3)]"
            >
              {t("indexerSearch.results.matched", { count: matchedCount })}
              {" · "}
              {t("indexerSearch.results.passing", { count: passingCount })}
            </span>
            <div className="min-w-2 flex-1" />
            <Select
              value={sort}
              onValueChange={(value) =>
                onSortChange(value as IndexerSearchSortKey)
              }
            >
              <SelectTrigger
                id="indexer-search-sort"
                size="compact"
                chrome="toolbar"
                aria-label={t("indexerSearch.sort.label")}
                className="h-[34px]"
              >
                <ArrowDownWideNarrow className="h-3.5 w-3.5 text-[var(--scry-faint)]" />
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {SORT_KEYS.map((option) => (
                  <SelectItem key={option} value={option}>
                    {t(SORT_LABEL_KEYS[option])}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="w-full min-w-0 max-w-full overflow-x-auto">
            <div className="min-w-[1020px]">
              <div
                className={cn(
                  RESULT_GRID_CLASS,
                  "border-b border-[var(--scry-border)] py-2.5 text-[10.5px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]",
                )}
              >
                <span />
                <span>{t("indexerSearch.column.release")}</span>
                <span>{t("indexerSearch.column.indexer")}</span>
                <span className="text-right">
                  {t("indexerSearch.column.size")}
                </span>
                <span className="text-right">
                  {t("indexerSearch.column.age")}
                </span>
                <span className="text-right">
                  {t("indexerSearch.column.peers")}
                </span>
                <span className="text-right">{t("label.actions")}</span>
              </div>
              {rows.map((release) => {
                const key = indexerSearchRowKey(release);
                return (
                  <ReleaseRow
                    key={key}
                    release={release}
                    selected={selectedKeys.has(key)}
                    expanded={expandedRowKey === key}
                    priority={priorityByIndexer.get(release.source ?? "")}
                    nowMs={nowMs}
                    onToggleRow={onToggleRow}
                    onToggleExpanded={onToggleExpanded}
                    onGrab={onGrab}
                  />
                );
              })}
              {rows.length === 0 ? (
                <p
                  id="indexer-search-empty"
                  className="px-4 py-10 text-center text-[13px] text-[var(--scry-muted3)]"
                >
                  {!hasSearched
                    ? t("indexerSearch.empty.prompt")
                    : searching || indexers.length === 0
                      ? t("indexerSearch.empty.searching")
                      : t("indexerSearch.empty.noResults")}
                </p>
              ) : null}
            </div>
          </div>

          <div
            className={cn(
              "flex flex-wrap items-center gap-3 border-t border-[var(--scry-border)] px-4 py-3",
              hasSelection
                ? "bg-[rgba(var(--scry-accent-rgb),0.09)]"
                : "bg-[var(--scry-surfD)]",
            )}
          >
            <span
              id="indexer-search-selection"
              className={cn(
                "text-[13px]",
                hasSelection
                  ? "text-[var(--scry-ink2)]"
                  : "text-[var(--scry-muted3)]",
              )}
            >
              <strong className="font-bold">
                {t("indexerSearch.footer.selected", {
                  count: selectedReleases.length,
                })}
              </strong>
              {" · "}
              {hasSelection
                ? formatReleaseSize(totalReleaseBytes(selectedReleases))
                : t("indexerSearch.footer.nothingQueued")}
            </span>
            <div className="min-w-2 flex-1" />
            <Button
              id="indexer-search-grab-selected"
              type="button"
              variant="success"
              size="sm"
              disabled={!hasSelection}
              onClick={() => onGrab(selectedReleases)}
            >
              <Download className="h-3.5 w-3.5" />
              {t("indexerSearch.footer.grabSelected")}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
