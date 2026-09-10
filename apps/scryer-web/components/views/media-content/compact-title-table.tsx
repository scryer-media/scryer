import * as React from "react";
import { useLocation } from "react-router";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import {
  persistOverviewScrollValue,
  readOverviewSavedScroll,
  useOverviewElementScrollRestoration,
} from "@/lib/hooks/use-overview-window-scroll-restoration";
import { Button } from "@/components/ui/button";
import {
  ArrowDown,
  ArrowUp,
  ChevronsUpDown,
  Eye,
  EyeOff,
  Loader2,
  Search,
  Trash2,
  Zap,
} from "lucide-react";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitleDownloadActivityPill } from "@/components/common/title-download-activity";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { Checkbox } from "@/components/ui/checkbox";
import { ActionTooltip, TooltipProvider } from "@/components/ui/tooltip";
import {
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import type { Release, TitleRecord } from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  titleOverviewDeleteButtonId,
  titleOverviewInteractiveSearchButtonId,
  titleOverviewInteractiveSearchPanelId,
  titleOverviewOpenButtonId,
  titleOverviewRowId,
  titleOverviewSearchButtonId,
  titleOverviewSelectId,
} from "@/lib/utils/dom-ids";
import {
  bytesToReadable,
  formatAudioCodecLabel,
  formatCatalogPopularity,
  formatHdrLabel,
  formatResolutionLabel,
  formatRuntimeMinutes,
  formatTitleDate,
  resolveDisplayedQualityLabel,
  resolveOverviewTargetView,
  StatusBadge,
  TitleEpisodeProgressBar,
  TitleCollectionEmptyState,
  TitleTableEmptyState,
  TitleTableTooltipActionButton,
  TitleTableLoadingState,
  COMPACT_TITLE_TABLE_ACTION_BUTTON_CLASS,
  DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS,
  TITLE_TABLE_HEADER_CELL_CLASS,
  TITLE_TABLE_HEADER_ROW_CLASS,
  TITLE_TABLE_INTERACTIVE_PANEL_BODY_CLASS,
  TITLE_TABLE_INTERACTIVE_PANEL_ESTIMATED_HEIGHT,
  TITLE_TABLE_ROW_CLASS,
  titleTableRatingColumnLabel,
  titleTableRatingColumnValue,
  titleTableRatingColumnWidthRem,
  titleTableSupportedRatingColumnsForView,
  type TitleTableSortDirection,
  type TitleTableSortKey,
  type TitleTableVisibleColumns,
  VirtualizedTitleTableBody,
  type VirtualizedTitleTableBodyHandle,
} from "./title-table-shared";

type CompactTitleTableProps = {
  view: string;
  titles: TitleRecord[];
  titleLoading: boolean;
  catalogHasMoreTitles?: boolean;
  catalogLoadingMoreTitles?: boolean;
  catalogPagingEnabled?: boolean;
  onCatalogEndReached?: () => Promise<void> | void;
  sortKey: TitleTableSortKey;
  sortDirection: TitleTableSortDirection;
  onSortChange: (key: TitleTableSortKey) => void;
  visibleColumns?: TitleTableVisibleColumns;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  selectedTitleId?: string | null;
  selectedDrawerMode?: boolean;
  contextPanelId?: string;
  onSelectTitle?: (title: TitleRecord) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => Promise<void> | void;
  onToggleMonitored?: (
    title: TitleRecord,
    monitored: boolean,
  ) => Promise<void> | void;
  onInteractiveSearch: (title: TitleRecord) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (title: TitleRecord, release: Release) => void;
  onQueueAdditionalFromInteractive?: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  isDeletingById: Record<string, boolean>;
  isTogglingMonitoredById?: Record<string, boolean>;
  selectedTitleIds: ReadonlySet<string>;
  onToggleSelected: (titleId: string) => void;
  onToggleSelectAll: (checked: boolean) => void;
  selectionMode?: boolean;
  bulkActionBusy: boolean;
  showScanLibraryAction?: boolean;
  showConfigureRootsAction?: boolean;
  configureRootsReason?: "missing" | "invalid";
  configureRootsHref?: string;
  onScanLibrary?: () => Promise<void> | void;
  scanLibraryLoading?: boolean;
  scanLibraryDisabled?: boolean;
  scanLibraryNotice?: string | null;
  /** Titles with live, pending download work — shown as a pulsing row pill. */
  activeDownloadTitleIds?: ReadonlySet<string>;
};

export const CompactTitleTable = React.memo(function CompactTitleTable({
  view,
  titles,
  titleLoading,
  catalogHasMoreTitles: catalogHasMoreTitlesProp,
  catalogLoadingMoreTitles: catalogLoadingMoreTitlesProp,
  catalogPagingEnabled: catalogPagingEnabledProp,
  onCatalogEndReached,
  sortKey,
  sortDirection,
  onSortChange,
  visibleColumns: visibleColumnsProp,
  onOpenOverview,
  selectedTitleId,
  selectedDrawerMode: selectedDrawerModeProp,
  contextPanelId,
  onSelectTitle,
  onDelete,
  onAutoQueue,
  onToggleMonitored,
  onInteractiveSearch,
  onQueueFromInteractive,
  onQueueAdditionalFromInteractive,
  isDeletingById,
  isTogglingMonitoredById,
  selectedTitleIds,
  onToggleSelected,
  onToggleSelectAll,
  selectionMode: selectionModeProp,
  bulkActionBusy,
  showScanLibraryAction: showScanLibraryActionProp,
  showConfigureRootsAction: showConfigureRootsActionProp,
  configureRootsReason: configureRootsReasonProp,
  configureRootsHref,
  onScanLibrary,
  scanLibraryLoading: scanLibraryLoadingProp,
  scanLibraryDisabled: scanLibraryDisabledProp,
  scanLibraryNotice,
  activeDownloadTitleIds,
}: CompactTitleTableProps) {
  const catalogHasMoreTitles = catalogHasMoreTitlesProp ?? false;
  const catalogLoadingMoreTitles = catalogLoadingMoreTitlesProp ?? false;
  const catalogPagingEnabled = catalogPagingEnabledProp ?? true;
  const visibleColumns =
    visibleColumnsProp ?? DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS;
  const selectedDrawerMode = selectedDrawerModeProp ?? false;
  const selectionMode = selectionModeProp ?? false;
  const showScanLibraryAction = showScanLibraryActionProp ?? false;
  const showConfigureRootsAction = showConfigureRootsActionProp ?? false;
  const configureRootsReason = configureRootsReasonProp ?? "missing";
  const scanLibraryLoading = scanLibraryLoadingProp ?? false;
  const scanLibraryDisabled = scanLibraryDisabledProp ?? false;
  const location = useLocation();
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const isMovieView = view === "movies";
  const overviewTargetView: ViewId = resolveOverviewTargetView(view);
  const selectedPaneMode =
    selectedTitleId !== null && onSelectTitle !== undefined;
  const showActionsColumn = !selectedDrawerMode && visibleColumns.actions;
  const showMonitoredColumn =
    !selectedDrawerMode && visibleColumns.monitored;
  const showQualityColumn =
    !selectedDrawerMode && visibleColumns.quality;
  const showEpisodesColumn =
    !selectedDrawerMode &&
    !isMovieView &&
    visibleColumns.episodes;
  const showSizeColumn =
    !selectedDrawerMode && visibleColumns.size;
  const showLibraryColumn =
    !selectedDrawerMode && visibleColumns.library;
  const showAddedColumn =
    !selectedDrawerMode && visibleColumns.added;
  const showYearColumn =
    !selectedDrawerMode &&
    isMovieView &&
    visibleColumns.year;
  const showRuntimeColumn =
    !selectedDrawerMode && visibleColumns.runtime;
  const showStatusColumn =
    !selectedDrawerMode &&
    !isMovieView &&
    visibleColumns.status;
  const showRootColumn =
    !selectedDrawerMode && visibleColumns.root;
  const showPopularityColumn =
    !selectedDrawerMode &&
    isMovieView &&
    visibleColumns.popularity;
  const showResolutionColumn =
    !selectedDrawerMode &&
    isMovieView &&
    visibleColumns.resolution;
  const showHdrColumn =
    !selectedDrawerMode &&
    isMovieView &&
    visibleColumns.hdr;
  const showAudioCodecColumn =
    !selectedDrawerMode &&
    isMovieView &&
    visibleColumns.audioCodec;
  const supportedRatingColumns =
    titleTableSupportedRatingColumnsForView(overviewTargetView);
  const showRatingColumns = selectedDrawerMode
    ? []
    : supportedRatingColumns.filter(
        (key) => visibleColumns[key],
      );
  const titleTableMinWidthRem = selectedDrawerMode
    ? null
    : 3 +
      16 +
      (showYearColumn ? 4.75 : 0) +
      (showLibraryColumn ? 7 : 0) +
      (showMonitoredColumn ? 4 : 0) +
      (showQualityColumn ? 7 : 0) +
      (showEpisodesColumn ? 7.5 : 0) +
      (showRuntimeColumn ? 6 : 0) +
      (showStatusColumn ? 6.75 : 0) +
      (showSizeColumn ? 7.5 : 0) +
      (showResolutionColumn ? 5.75 : 0) +
      (showHdrColumn ? 7 : 0) +
      (showAudioCodecColumn ? 7.25 : 0) +
      (showPopularityColumn ? 6 : 0) +
      (showRootColumn ? 11 : 0) +
      showRatingColumns.reduce(
        (total, key) => total + titleTableRatingColumnWidthRem(key),
        0,
      ) +
      (showAddedColumn ? 6.5 : 0) +
      (showActionsColumn ? 8.5 : 0);
  const columnCount = selectedDrawerMode
    ? 3
    : 2 +
      (showYearColumn ? 1 : 0) +
      (showLibraryColumn ? 1 : 0) +
      (showMonitoredColumn ? 1 : 0) +
      (showQualityColumn ? 1 : 0) +
      (showEpisodesColumn ? 1 : 0) +
      (showRuntimeColumn ? 1 : 0) +
      (showStatusColumn ? 1 : 0) +
      (showSizeColumn ? 1 : 0) +
      (showResolutionColumn ? 1 : 0) +
      (showHdrColumn ? 1 : 0) +
      (showAudioCodecColumn ? 1 : 0) +
      (showPopularityColumn ? 1 : 0) +
      (showRootColumn ? 1 : 0) +
      showRatingColumns.length +
      (showAddedColumn ? 1 : 0) +
      (showActionsColumn ? 1 : 0);
  const selectedVisibleCount = titles.filter((title) =>
    selectedTitleIds.has(title.id),
  ).length;
  const allVisibleSelected =
    titles.length > 0 && selectedVisibleCount === titles.length;
  const selectAllState = allVisibleSelected
    ? true
    : selectedVisibleCount > 0
      ? "indeterminate"
      : false;
  const titleTableColGroup = selectedDrawerMode ? (
    <colgroup>
      <col />
      <col style={{ width: "44px" }} />
      <col style={{ width: "76px" }} />
    </colgroup>
  ) : (
    <colgroup>
      <col style={{ width: "3rem" }} />
      <col />
      {showYearColumn ? <col style={{ width: "4.75rem" }} /> : null}
      {showRatingColumns.map((key) => (
        <col
          key={key}
          style={{ width: `${titleTableRatingColumnWidthRem(key)}rem` }}
        />
      ))}
      {showLibraryColumn ? <col style={{ width: "7rem" }} /> : null}
      {showMonitoredColumn ? <col style={{ width: "4rem" }} /> : null}
      {showQualityColumn ? <col style={{ width: "7rem" }} /> : null}
      {showEpisodesColumn ? <col style={{ width: "7.5rem" }} /> : null}
      {showRuntimeColumn ? <col style={{ width: "6rem" }} /> : null}
      {showStatusColumn ? <col style={{ width: "6.75rem" }} /> : null}
      {showSizeColumn ? <col style={{ width: "7.5rem" }} /> : null}
      {showResolutionColumn ? <col style={{ width: "5.75rem" }} /> : null}
      {showHdrColumn ? <col style={{ width: "7rem" }} /> : null}
      {showAudioCodecColumn ? <col style={{ width: "7.25rem" }} /> : null}
      {showPopularityColumn ? <col style={{ width: "6rem" }} /> : null}
      {showRootColumn ? <col style={{ width: "11rem" }} /> : null}
      {showAddedColumn ? <col style={{ width: "6.5rem" }} /> : null}
      {showActionsColumn ? <col style={{ width: "8.5rem" }} /> : null}
    </colgroup>
  );
  const visibleColumnSignature = selectedDrawerMode
    ? "drawer"
    : [
        showYearColumn && "year",
        ...showRatingColumns,
        showLibraryColumn && "library",
        showMonitoredColumn && "monitored",
        showQualityColumn && "quality",
        showEpisodesColumn && "episodes",
        showRuntimeColumn && "runtime",
        showStatusColumn && "status",
        showSizeColumn && "size",
        showResolutionColumn && "resolution",
        showHdrColumn && "hdr",
        showAudioCodecColumn && "audioCodec",
        showPopularityColumn && "popularity",
        showRootColumn && "root",
        showAddedColumn && "added",
        showActionsColumn && "actions",
      ]
        .filter(Boolean)
        .join(":");

  const [expandedInteractiveRows, setExpandedInteractiveRows] = React.useState(
    new Set<string>(),
  );
  const [interactiveSearchResultsByTitle, setInteractiveSearchResultsByTitle] =
    React.useState<Record<string, Release[]>>({});
  const [interactiveSearchLoadingByTitle, setInteractiveSearchLoadingByTitle] =
    React.useState<Record<string, boolean>>({});
  const [autoQueueLoadingByTitle, setAutoQueueLoadingByTitle] = React.useState<
    Record<string, boolean>
  >({});

  const titleTableScrollRef = React.useRef<HTMLDivElement>(null);
  const sortedTitles = titles;
  const scrollStorageKeySuffix = selectedPaneMode
    ? "compact-selected"
    : "compact";
  const initialScrollOffset = React.useMemo(
    () =>
      readOverviewSavedScroll(location.pathname, scrollStorageKeySuffix) ?? 0,
    [location.pathname, scrollStorageKeySuffix],
  );
  const expandedInteractiveRowSignature = React.useMemo(
    () => Array.from(expandedInteractiveRows).sort().join("|"),
    [expandedInteractiveRows],
  );
  const compactTitleRowHeight = selectedDrawerMode ? 70 : 48;
  const titleTableVirtualizerRef =
    React.useRef<VirtualizedTitleTableBodyHandle>(null);
  const estimateTitleRowSize = React.useCallback(
    (index: number) => {
      const titleId = sortedTitles[index]?.id;
      return (
        compactTitleRowHeight +
        (!selectedDrawerMode &&
        titleId &&
        expandedInteractiveRows.has(titleId)
          ? TITLE_TABLE_INTERACTIVE_PANEL_ESTIMATED_HEIGHT
          : 0)
      );
    },
    [compactTitleRowHeight, expandedInteractiveRows, selectedDrawerMode, sortedTitles],
  );
  const getTitleTableMaxScrollTop = React.useCallback(
    (element: HTMLElement) =>
      titleTableVirtualizerRef.current?.getMaxScrollTop(element) ??
      Math.max(0, element.scrollHeight - element.clientHeight),
    [],
  );
  const restoreTitleTableScroll = React.useCallback(
    (nextTop: number) => {
      titleTableVirtualizerRef.current?.scrollToOffset(nextTop);
    },
    [],
  );
  useOverviewElementScrollRestoration({
    enabled: true,
    ready: titles.length > 0,
    storageKeySuffix: scrollStorageKeySuffix,
    scrollRef: titleTableScrollRef,
    getMaxScrollTop: getTitleTableMaxScrollTop,
    restoreScrollTop: restoreTitleTableScroll,
  });

  const selectedTitleScrollKey = selectedTitleId
    ? `${selectedTitleId}:${sortKey}:${sortDirection}`
    : null;

  const handleOpenOverview = React.useCallback(
    (item: OverviewTitleTarget) => {
      persistOverviewScrollValue(
        location.pathname,
        scrollStorageKeySuffix,
        titleTableScrollRef.current?.scrollTop,
      );
      onOpenOverview(resolveOverviewTargetView(view), item);
    },
    [location.pathname, onOpenOverview, scrollStorageKeySuffix, view],
  );

  const handleActivateTitle = React.useCallback(
    (item: TitleRecord) => {
      if (selectionMode) {
        onToggleSelected(item.id);
        return;
      }
      if (onSelectTitle) {
        persistOverviewScrollValue(
          location.pathname,
          scrollStorageKeySuffix,
          titleTableScrollRef.current?.scrollTop,
        );
        onSelectTitle(item);
        return;
      }
      handleOpenOverview(item);
    },
    [
      handleOpenOverview,
      location.pathname,
      onSelectTitle,
      onToggleSelected,
      scrollStorageKeySuffix,
      selectionMode,
    ],
  );

  const isInteractiveTitleRowTarget = React.useCallback(
    (target: EventTarget | null) =>
      target instanceof Element &&
      target.closest(
        'a[href], button, input, select, textarea, [role="button"], [role="checkbox"], [role="menuitem"]',
      ) !== null,
    [],
  );

  const handleTitleRowClick = React.useCallback(
    (event: React.MouseEvent<HTMLTableRowElement>, item: TitleRecord) => {
      if (isInteractiveTitleRowTarget(event.target)) {
        return;
      }
      if (selectionMode || onSelectTitle) {
        handleActivateTitle(item);
      }
    },
    [handleActivateTitle, isInteractiveTitleRowTarget, onSelectTitle, selectionMode],
  );

  const handleTitleRowKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLTableRowElement>, item: TitleRecord) => {
      if (isInteractiveTitleRowTarget(event.target)) {
        return;
      }
      if (!selectionMode && !onSelectTitle) {
        return;
      }

      if (event.key !== "Enter" && event.key !== " ") {
        return;
      }

      event.preventDefault();
      handleActivateTitle(item);
    },
    [handleActivateTitle, isInteractiveTitleRowTarget, onSelectTitle, selectionMode],
  );

  const handleSort = React.useCallback(
    (nextKey: TitleTableSortKey) => {
      onSortChange(nextKey);
    },
    [onSortChange],
  );

  const renderSortIcon = React.useCallback(
    (key: TitleTableSortKey) => {
      if (sortKey !== key) {
        return (
          <ChevronsUpDown className="h-3.5 w-3.5 text-[var(--scry-muted3)]" />
        );
      }
      return sortDirection === "asc" ? (
        <ArrowUp className="h-3.5 w-3.5" />
      ) : (
        <ArrowDown className="h-3.5 w-3.5" />
      );
    },
    [sortDirection, sortKey],
  );

  const renderSortableHeader = React.useCallback(
    (
      key: TitleTableSortKey,
      label: string,
      className?: string,
      buttonClassName?: string,
    ) => (
      <TableHead
        className={className}
        aria-sort={
          sortKey === key
            ? sortDirection === "asc"
              ? "ascending"
              : "descending"
            : "none"
        }
      >
        <button
          type="button"
          className={cn(
            "inline-flex w-full items-center gap-1 text-left text-[11px] font-bold uppercase tracking-[0.05em] text-[var(--scry-faint2)] transition-colors hover:text-[var(--scry-muted2)]",
            buttonClassName,
          )}
          onClick={() => handleSort(key)}
        >
          <span>{label}</span>
          {renderSortIcon(key)}
        </button>
      </TableHead>
    ),
    [handleSort, renderSortIcon, sortDirection, sortKey],
  );

  const handleQueueExisting = React.useCallback(
    (title: TitleRecord) => {
      if (bulkActionBusy) {
        return;
      }
      const titleId = title.id;
      setAutoQueueLoadingByTitle((previous) => ({
        ...previous,
        [titleId]: true,
      }));
      void Promise.resolve(onAutoQueue(title)).finally(() => {
        setAutoQueueLoadingByTitle((previous) => {
          if (!previous[titleId]) {
            return previous;
          }
          const next = { ...previous };
          delete next[titleId];
          return next;
        });
      });
    },
    [bulkActionBusy, onAutoQueue],
  );

  const handleRunInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      if (bulkActionBusy) {
        return;
      }
      const titleId = title.id;
      setInteractiveSearchLoadingByTitle((previous) => ({
        ...previous,
        [titleId]: true,
      }));
      void Promise.resolve(onInteractiveSearch(title))
        .then((results) => {
          setInteractiveSearchResultsByTitle((previous) => ({
            ...previous,
            [titleId]: results ?? [],
          }));
        })
        .finally(() => {
          setInteractiveSearchLoadingByTitle((previous) => {
            if (!previous[titleId]) {
              return previous;
            }
            const next = { ...previous };
            delete next[titleId];
            return next;
          });
        });
    },
    [bulkActionBusy, onInteractiveSearch],
  );

  const handleToggleInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      const isOpen = expandedInteractiveRows.has(titleId);
      setExpandedInteractiveRows((previous) => {
        const next = new Set(previous);
        if (next.has(titleId)) {
          next.delete(titleId);
        } else {
          next.add(titleId);
        }
        return next;
      });
      if (
        !isOpen &&
        !Object.prototype.hasOwnProperty.call(
          interactiveSearchResultsByTitle,
          titleId,
        )
      ) {
        handleRunInteractiveSearch(title);
      }
    },
    [
      expandedInteractiveRows,
      handleRunInteractiveSearch,
      interactiveSearchResultsByTitle,
    ],
  );

  const renderTitleRow = (item: TitleRecord) => {
    const isPanelOpen = expandedInteractiveRows.has(item.id);
    const interactiveSearchResults =
      interactiveSearchResultsByTitle[item.id] ?? [];
    const interactiveSearchLoading =
      interactiveSearchLoadingByTitle[item.id] === true;
    const autoQueueLoading = autoQueueLoadingByTitle[item.id] === true;
    const deleteLoading = isDeletingById[item.id] === true;
    const monitorToggleLoading = isTogglingMonitoredById?.[item.id] === true;
    const isSelected = selectedTitleId === item.id;
    const isRowHighlighted =
      isSelected || (selectionMode && selectedTitleIds.has(item.id));
    const downloadActive = activeDownloadTitleIds?.has(item.id) ?? false;
    const addedLabel =
      formatTitleDate(item.createdAt, dateTimeFormat) ?? t("label.unknown");

    const contextPanelControlsId = selectedPaneMode ? contextPanelId : undefined;
    const selectedContextPanelControlsId = isSelected
      ? contextPanelControlsId
      : undefined;

    if (selectedDrawerMode) {
      const posterUrl = selectPosterVariantUrl(item.posterUrl, "w70");
      const yearLabel = item.year ? String(item.year) : null;
      const qualityLabel = resolveDisplayedQualityLabel(item, t("label.unknown"));
      const subline = [yearLabel, qualityLabel].filter(Boolean).join(" · ");
      const libraryLabel = item.libraryName ?? item.libraryId ?? null;

      return (
        <TableRow
          id={titleOverviewRowId(item.id)}
          data-ui="compact-title-table-row"
          data-selected={isRowHighlighted ? "true" : undefined}
          aria-selected={selectedPaneMode ? isSelected : undefined}
          aria-current={isSelected ? "true" : undefined}
          aria-controls={selectedContextPanelControlsId}
          aria-label={
            selectedPaneMode
              ? t("title.selectTitle", { name: item.name })
              : undefined
          }
          aria-keyshortcuts={selectedPaneMode ? "Enter Space" : undefined}
          tabIndex={selectedPaneMode ? 0 : undefined}
          onClick={(event) => handleTitleRowClick(event, item)}
          onKeyDown={(event) => handleTitleRowKeyDown(event, item)}
          className={cn("h-[70px]", TITLE_TABLE_ROW_CLASS)}
        >
          <TableCell className="align-middle overflow-hidden py-2 pl-4 pr-2">
            <div className="flex min-w-0 items-center gap-2">
              <button
                id={titleOverviewOpenButtonId(item.id)}
                type="button"
                onClick={() => handleActivateTitle(item)}
                data-ui="title-name"
                aria-current={isSelected ? "true" : undefined}
                aria-controls={selectedContextPanelControlsId}
                tabIndex={selectedPaneMode ? -1 : undefined}
                className="flex min-w-0 flex-1 items-center gap-3 overflow-hidden text-left hover:text-foreground"
              >
                <span className="relative h-[50px] w-[34px] shrink-0 overflow-hidden rounded-[5px] border border-[var(--scry-border2)] bg-[var(--scry-card2)]">
                  <TitlePosterSlot
                    src={posterUrl}
                    metadataFetchedAt={item.metadataFetchedAt}
                    createdAt={item.createdAt}
                    alt={t("media.posterAlt", { name: item.name })}
                    className="h-full w-full object-cover"
                    placeholderClassName="flex h-full w-full items-center justify-center px-1 text-center text-[8px] text-muted-foreground"
                    emptyLabel={t("label.noArt")}
                    loading="lazy"
                    decoding="async"
                  />
                  <span
                    aria-hidden="true"
                    className="pointer-events-none absolute inset-0 bg-gradient-to-b from-transparent from-45% to-black/80"
                  />
                </span>
                <span className="min-w-0">
                  <span className="block truncate text-[13.5px] font-semibold text-[var(--scry-ink3)]">
                    {item.name}
                  </span>
                  <span className="mt-0.5 flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[11.5px] text-[var(--scry-faint)]">
                    <span
                      aria-hidden="true"
                      className="size-1.5 shrink-0 rounded-full bg-[var(--scry-accent)]"
                    />
                    {libraryLabel ? (
                      <span className="min-w-0 truncate">{libraryLabel}</span>
                    ) : null}
                    {libraryLabel && subline ? (
                      <span className="shrink-0">·</span>
                    ) : null}
                    {subline ? (
                      <span className="min-w-0 truncate">{subline}</span>
                    ) : null}
                  </span>
                </span>
              </button>
              {downloadActive ? (
                <TitleDownloadActivityPill />
              ) : null}
            </div>
          </TableCell>
          <TableCell className="text-center align-middle">
            <ActionTooltip
              useProvider={false}
              content={`${t("title.table.monitored")}: ${item.name}`}
            >
              <span
                className="inline-flex h-4 w-4 shrink-0 items-center justify-center"
                aria-label={`${t("title.table.monitored")}: ${item.name}`}
              >
                {item.monitored ? (
                  <Eye className="size-4 text-[var(--scry-success-text-soft)]" />
                ) : (
                  <EyeOff className="size-4 text-[var(--scry-faint2)]" />
                )}
              </span>
            </ActionTooltip>
          </TableCell>
          <TableCell className="whitespace-nowrap px-2 py-2 text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
            {bytesToReadable(item.sizeBytes)}
          </TableCell>
        </TableRow>
      );
    }

    return (
      <React.Fragment key={item.id}>
        <TableRow
          id={titleOverviewRowId(item.id)}
          data-ui="compact-title-table-row"
          data-selected={isRowHighlighted ? "true" : undefined}
          aria-selected={selectedPaneMode ? isSelected : undefined}
          aria-current={isSelected ? "true" : undefined}
          aria-controls={selectedContextPanelControlsId}
          aria-label={
            selectedPaneMode
              ? t("title.selectTitle", { name: item.name })
              : undefined
          }
          aria-keyshortcuts={selectedPaneMode ? "Enter Space" : undefined}
          tabIndex={selectedPaneMode ? 0 : undefined}
          onClick={(event) => handleTitleRowClick(event, item)}
          onKeyDown={(event) => handleTitleRowKeyDown(event, item)}
          className={cn(
            "h-12",
            TITLE_TABLE_ROW_CLASS,
            selectionMode && "cursor-pointer",
          )}
        >
          <TableCell className="px-0 text-center align-middle">
            <Checkbox
              id={titleOverviewSelectId(item.id)}
              checked={selectedTitleIds.has(item.id)}
              onCheckedChange={() => onToggleSelected(item.id)}
              aria-label={t("title.selectTitle", { name: item.name })}
              disabled={bulkActionBusy}
              size="table"
              className="mx-auto"
            />
          </TableCell>
          <TableCell className="align-middle overflow-hidden py-1.5">
            <div className="flex min-w-0 items-center gap-2">
              <button
                id={titleOverviewOpenButtonId(item.id)}
                type="button"
                onClick={() => handleActivateTitle(item)}
                data-ui="title-name"
                aria-current={isSelected ? "true" : undefined}
                aria-controls={selectedContextPanelControlsId}
                tabIndex={selectedPaneMode ? -1 : undefined}
                className="block min-w-0 flex-1 overflow-hidden text-left text-[13px] font-medium text-[var(--scry-ink3)] hover:text-foreground"
              >
                <span className="block truncate">{item.name}</span>
              </button>
              {downloadActive ? (
                <TitleDownloadActivityPill />
              ) : null}
            </div>
          </TableCell>
          {showYearColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {item.year ?? "—"}
            </TableCell>
          ) : null}
          {showRatingColumns.map((columnKey) => (
            <TableCell
              key={columnKey}
              className="whitespace-nowrap py-1.5 text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]"
            >
              {titleTableRatingColumnValue(item, columnKey)}
            </TableCell>
          ))}
          {showLibraryColumn ? (
            <TableCell className="overflow-hidden py-1.5 text-center align-middle text-[12px] text-[var(--scry-muted)]">
              <span className="block truncate">
                {item.libraryName ?? item.libraryId}
              </span>
            </TableCell>
          ) : null}
          {showMonitoredColumn ? (
            <TableCell className="text-center align-middle">
              <ActionTooltip
                useProvider={false}
                content={`${t("title.table.monitored")}: ${item.name}`}
              >
                <span
                  className="inline-flex h-4 w-4 shrink-0 items-center justify-center"
                  aria-label={`${t("title.table.monitored")}: ${item.name}`}
                >
                  {item.monitored ? (
                    <Eye className="size-4 text-[var(--scry-success-text-soft)]" />
                  ) : (
                    <EyeOff className="size-4 text-[var(--scry-faint2)]" />
                  )}
                </span>
              </ActionTooltip>
            </TableCell>
          ) : null}
          {showQualityColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle text-[12.5px] text-[var(--scry-text4)]">
              {resolveDisplayedQualityLabel(item, t("label.unknown"))}
            </TableCell>
          ) : null}
          {showEpisodesColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle">
              <TitleEpisodeProgressBar item={item} t={t} compact />
            </TableCell>
          ) : null}
          {showRuntimeColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {formatRuntimeMinutes(item.runtimeMinutes)}
            </TableCell>
          ) : null}
          {showStatusColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle text-[12px]">
              {item.contentStatus ? (
                <StatusBadge status={item.contentStatus} t={t} />
              ) : (
                <span className="text-[var(--scry-faint2)]">—</span>
              )}
            </TableCell>
          ) : null}
          {showSizeColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {bytesToReadable(item.sizeBytes)}
            </TableCell>
          ) : null}
          {showResolutionColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {formatResolutionLabel(item.mediaResolution)}
            </TableCell>
          ) : null}
          {showHdrColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle text-[12.5px] text-[var(--scry-text4)]">
              {formatHdrLabel(item.mediaHdr)}
            </TableCell>
          ) : null}
          {showAudioCodecColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle text-[12.5px] text-[var(--scry-text4)]">
              {formatAudioCodecLabel(item.mediaAudioCodec)}
            </TableCell>
          ) : null}
          {showPopularityColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {formatCatalogPopularity(item.popularity)}
            </TableCell>
          ) : null}
          {showRootColumn ? (
            <TableCell className="overflow-hidden py-1.5 text-center align-middle text-[12px] text-[var(--scry-muted)]">
              <span className="block truncate" title={item.rootFolderPath ?? ""}>
                {item.rootFolderPath ?? item.rootFolderId ?? "—"}
              </span>
            </TableCell>
          ) : null}
          {showAddedColumn ? (
            <TableCell className="whitespace-nowrap py-1.5 text-center align-middle text-[12px] text-[var(--scry-muted)]">
              {addedLabel}
            </TableCell>
          ) : null}
          {showActionsColumn ? (
            <TableCell className="overflow-hidden px-1.5 py-1.5 text-center align-middle">
              <div
                data-ui="row-actions"
                className={cn(
                  "flex w-full min-w-0 items-center justify-center gap-1",
                  selectionMode && "pointer-events-none opacity-40",
                )}
              >
                <TitleTableTooltipActionButton
                  id={titleOverviewSearchButtonId(item.id)}
                  tone="auto"
                  label={t("label.search")}
                  tooltip={t("help.autoSearchTooltip")}
                  onClick={() => handleQueueExisting(item)}
                  disabled={autoQueueLoading || bulkActionBusy}
                  className={COMPACT_TITLE_TABLE_ACTION_BUTTON_CLASS}
                >
                  {autoQueueLoading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-[var(--scry-accent-text)]" />
                  ) : (
                    <Zap className="h-3.5 w-3.5" />
                  )}
                </TitleTableTooltipActionButton>
                <TitleTableTooltipActionButton
                  id={titleOverviewInteractiveSearchButtonId(item.id)}
                  tone="accent"
                  label={t("label.interactiveSearch")}
                  tooltip={t("help.interactiveSearchTooltip")}
                  onClick={() => handleToggleInteractiveSearch(item)}
                  disabled={bulkActionBusy}
                  className={COMPACT_TITLE_TABLE_ACTION_BUTTON_CLASS}
                >
                  <Search className="h-3.5 w-3.5" />
                </TitleTableTooltipActionButton>
                {onToggleMonitored ? (
                  <TitleTableTooltipActionButton
                    tone="search"
                    label={t(
                      item.monitored
                        ? "title.unmonitorAction"
                        : "title.monitorAction",
                    )}
                    onClick={() => onToggleMonitored(item, !item.monitored)}
                    disabled={monitorToggleLoading || bulkActionBusy}
                    className={COMPACT_TITLE_TABLE_ACTION_BUTTON_CLASS}
                  >
                    {monitorToggleLoading ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : item.monitored ? (
                      <EyeOff className="h-3.5 w-3.5" />
                    ) : (
                      <Eye className="h-3.5 w-3.5" />
                    )}
                  </TitleTableTooltipActionButton>
                ) : null}
                <TitleTableTooltipActionButton
                  id={titleOverviewDeleteButtonId(item.id)}
                  tone="delete"
                  label={t("label.delete")}
                  onClick={() => onDelete(item)}
                  disabled={deleteLoading || bulkActionBusy}
                  className={COMPACT_TITLE_TABLE_ACTION_BUTTON_CLASS}
                >
                  {deleteLoading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="h-3.5 w-3.5" />
                  )}
                </TitleTableTooltipActionButton>
              </div>
            </TableCell>
          ) : null}
        </TableRow>
        {isPanelOpen ? (
          <TableRow
            id={titleOverviewInteractiveSearchPanelId(item.id)}
            data-ui="compact-title-table-panel-row"
          >
            <TableCell
              colSpan={columnCount}
              className="border-t border-border bg-popover/40 p-0"
            >
              <div className={TITLE_TABLE_INTERACTIVE_PANEL_BODY_CLASS}>
                <div className="mb-2 flex items-center justify-between gap-3">
                  <p className="text-sm text-card-foreground">
                    {t("nzb.searchResultsFor", { name: item.name })}
                  </p>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => handleRunInteractiveSearch(item)}
                    disabled={interactiveSearchLoading || bulkActionBusy}
                    aria-label={t("label.search")}
                  >
                    <Search className="h-4 w-4" />
                    <span className="ml-1">
                      {interactiveSearchLoading
                        ? t("label.searching")
                        : t("label.refresh")}
                    </span>
                  </Button>
                </div>
                {interactiveSearchLoading ? (
                  <div className="flex items-center gap-3 py-3">
                    <Loader2 className="h-5 w-5 animate-spin text-[var(--scry-accent-text)]" />
                    <p className="text-sm text-muted-foreground">
                      {t("label.searching")}
                    </p>
                  </div>
                ) : interactiveSearchResults.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    {t("nzb.noResultsYet")}
                  </p>
                ) : (
                  <SearchResultBuckets
                    results={interactiveSearchResults}
                    onQueue={(release) => {
                      if (bulkActionBusy) {
                        return;
                      }
                      return onQueueFromInteractive(item, release);
                    }}
                    onQueueAdditional={
                      onQueueAdditionalFromInteractive
                        ? (release) => {
                            if (bulkActionBusy) {
                              return;
                            }
                            return onQueueAdditionalFromInteractive(
                              item,
                              release,
                            );
                          }
                        : undefined
                    }
                    canQueueAdditional={(release) =>
                      releaseSupportsAdditionalFileQueue(release, item.facet)
                    }
                    disabled={bulkActionBusy}
                    requireCandidateToken
                  />
                )}
              </div>
            </TableCell>
          </TableRow>
        ) : null}
      </React.Fragment>
    );
  };

  const titleTableHeader = selectedDrawerMode ? (
    <TableHeader>
      <TableRow className={TITLE_TABLE_HEADER_ROW_CLASS}>
        {renderSortableHeader(
          "name",
          t("label.title"),
          cn("pl-4", TITLE_TABLE_HEADER_CELL_CLASS),
          "uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
        )}
        <TableHead
          className={cn(
            "whitespace-nowrap text-center",
            TITLE_TABLE_HEADER_CELL_CLASS,
          )}
          title={t("title.table.monitored")}
        >
          MON.
        </TableHead>
        {renderSortableHeader(
          "size",
          t("title.table.size"),
          cn("whitespace-nowrap px-2 text-center", TITLE_TABLE_HEADER_CELL_CLASS),
          "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
        )}
      </TableRow>
    </TableHeader>
  ) : (
    <TableHeader>
      <TableRow className={TITLE_TABLE_HEADER_ROW_CLASS}>
        <TableHead className="w-12 text-center">
          <Checkbox
            checked={selectAllState}
            onCheckedChange={(checked) => onToggleSelectAll(checked === true)}
            aria-label={t("title.selectAllTitles")}
            disabled={bulkActionBusy}
            size="table"
            className="mx-auto"
          />
        </TableHead>
        {renderSortableHeader(
          "name",
          t("label.name"),
          TITLE_TABLE_HEADER_CELL_CLASS,
        )}
        {showYearColumn
          ? renderSortableHeader(
              "year",
              t("title.table.year"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showRatingColumns.map((columnKey) =>
          renderSortableHeader(
            columnKey as TitleTableSortKey,
            titleTableRatingColumnLabel(columnKey),
            cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
            "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
          ),
        )}
        {showLibraryColumn
          ? renderSortableHeader(
              "library",
              t("title.table.library"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showMonitoredColumn
          ? renderSortableHeader(
              "monitored",
              t("title.table.monitored"),
              cn("text-center whitespace-nowrap", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showQualityColumn
          ? renderSortableHeader(
              "quality",
              t("title.table.qualityTier"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showEpisodesColumn
          ? renderSortableHeader(
              "episodes",
              t("title.table.episodes"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showRuntimeColumn
          ? renderSortableHeader(
              "runtime",
              isMovieView
                ? t("title.table.runtime")
                : t("title.table.avgRuntime"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showStatusColumn
          ? renderSortableHeader(
              "status",
              t("title.table.status"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showSizeColumn
          ? renderSortableHeader(
              "size",
              t("title.table.size"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showResolutionColumn
          ? renderSortableHeader(
              "resolution",
              t("title.table.resolution"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showHdrColumn
          ? renderSortableHeader(
              "hdr",
              t("title.table.hdr"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showAudioCodecColumn
          ? renderSortableHeader(
              "audioCodec",
              t("title.table.audioCodec"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showPopularityColumn
          ? renderSortableHeader(
              "popularity",
              t("title.table.popularity"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showRootColumn
          ? renderSortableHeader(
              "root",
              t("title.table.root"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showAddedColumn
          ? renderSortableHeader(
              "added",
              t("title.contextAdded"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showActionsColumn ? (
          <TableHead
            className={cn(
              "whitespace-nowrap px-1.5 text-center",
              TITLE_TABLE_HEADER_CELL_CLASS,
            )}
          >
            {t("label.actions")}
          </TableHead>
        ) : null}
      </TableRow>
    </TableHeader>
  );

  if (
    !titleLoading &&
    sortedTitles.length === 0 &&
    showConfigureRootsAction &&
    configureRootsHref
  ) {
    return (
      <div className="flex h-full min-h-0 flex-col gap-3">
        <div
          data-slot="compact-title-list-root-config-empty"
          className={cn(
            "flex flex-1 items-start justify-center px-4 pt-12",
            selectedDrawerMode ? "min-h-[18rem]" : "min-h-[22rem]",
          )}
        >
          <TitleCollectionEmptyState
            t={t}
            showConfigureRootsAction={showConfigureRootsAction}
            configureRootsReason={configureRootsReason}
            configureRootsHref={configureRootsHref}
          />
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div
        data-slot="title-list-scroll"
        ref={titleTableScrollRef}
        className={cn(
          "relative flex-1 overflow-x-auto overflow-y-auto rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surfD)]",
          selectedDrawerMode ? "min-h-0" : "min-h-[22rem]",
        )}
      >
        <TooltipProvider delayDuration={300}>
          <table
            data-ui="compact-title-table"
            data-view={view}
            className="w-full table-fixed caption-bottom text-sm"
            style={
              titleTableMinWidthRem === null
                ? undefined
                : { minWidth: `${titleTableMinWidthRem}rem` }
            }
          >
          {titleTableColGroup}
          {titleTableHeader}
          <VirtualizedTitleTableBody
            ref={titleTableVirtualizerRef}
            titles={sortedTitles}
            scrollRef={titleTableScrollRef}
            initialScrollOffset={initialScrollOffset}
            estimateSize={estimateTitleRowSize}
            overscan={8}
            rebuildKey={`${visibleColumnSignature}:${expandedInteractiveRowSignature}`}
            selectedTitleId={selectedTitleId}
            selectedTitleScrollKey={selectedTitleScrollKey}
            catalogPagingEnabled={catalogPagingEnabled}
            catalogHasMoreTitles={catalogHasMoreTitles}
            catalogLoadingMoreTitles={catalogLoadingMoreTitles}
            onCatalogEndReached={onCatalogEndReached}
            columnCount={columnCount}
            renderRow={renderTitleRow}
            emptyContent={
              titleLoading ? (
              <TitleTableLoadingState colSpan={columnCount} />
              ) : (
              <TitleTableEmptyState
                colSpan={columnCount}
                t={t}
                showScanAction={showScanLibraryAction}
                showConfigureRootsAction={showConfigureRootsAction}
                configureRootsReason={configureRootsReason}
                configureRootsHref={configureRootsHref}
                onScan={onScanLibrary}
                scanLoading={scanLibraryLoading}
                scanDisabled={scanLibraryDisabled}
                scanNotice={scanLibraryNotice}
              />
              )
            }
          />
          </table>
        </TooltipProvider>
      </div>
    </div>
  );
});
