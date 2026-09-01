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
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitleDownloadActivityPill } from "@/components/common/title-download-activity";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
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
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
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
  TITLE_TABLE_ACTION_BUTTON_CLASS,
  TITLE_TABLE_HEADER_CELL_CLASS,
  TITLE_TABLE_HEADER_ROW_CLASS,
  TITLE_TABLE_INTERACTIVE_PANEL_BODY_CLASS,
  TITLE_TABLE_INTERACTIVE_PANEL_ESTIMATED_HEIGHT,
  TITLE_TABLE_ROW_CLASS,
  DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS,
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

type TitleTableProps = {
  view: string;
  titles: TitleRecord[];
  titleLoading: boolean;
  catalogHasMoreTitles?: boolean;
  catalogLoadingMoreTitles?: boolean;
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
  selectedPaneMode?: boolean;
  contextPanelId?: string;
  onSelectTitle?: (title: TitleRecord) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => void;
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

export const TitleTable = React.memo(function TitleTable({
  view,
  titles,
  titleLoading,
  catalogHasMoreTitles: catalogHasMoreTitlesProp,
  catalogLoadingMoreTitles: catalogLoadingMoreTitlesProp,
  onCatalogEndReached,
  sortKey,
  sortDirection,
  onSortChange,
  visibleColumns: visibleColumnsProp,
  onOpenOverview,
  selectedTitleId,
  selectedPaneMode: selectedPaneModeProp,
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
}: TitleTableProps) {
  const catalogHasMoreTitles = catalogHasMoreTitlesProp ?? false;
  const catalogLoadingMoreTitles = catalogLoadingMoreTitlesProp ?? false;
  const visibleColumns =
    visibleColumnsProp ?? DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS;
  const selectedPaneMode = selectedPaneModeProp ?? false;
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
  const showActionsColumn = visibleColumns.actions;
  const showMonitoredColumn = visibleColumns.monitored;
  const showQualityColumn = visibleColumns.quality;
  const showEpisodesColumn = !isMovieView && visibleColumns.episodes;
  const showSizeColumn = visibleColumns.size;
  const showLibraryColumn = visibleColumns.library;
  const showAddedColumn = visibleColumns.added;
  const showYearColumn = isMovieView && visibleColumns.year;
  const showRuntimeColumn = visibleColumns.runtime;
  const showStatusColumn = !isMovieView && visibleColumns.status;
  const showRootColumn = visibleColumns.root;
  const showPopularityColumn = isMovieView && visibleColumns.popularity;
  const showResolutionColumn = isMovieView && visibleColumns.resolution;
  const showHdrColumn = isMovieView && visibleColumns.hdr;
  const showAudioCodecColumn = isMovieView && visibleColumns.audioCodec;
  const supportedRatingColumns =
    titleTableSupportedRatingColumnsForView(overviewTargetView);
  const showRatingColumns = supportedRatingColumns.filter(
    (key) => visibleColumns[key],
  );
  const titleTableMinWidthRem =
    3 +
    18 +
    (showYearColumn ? 5 : 0) +
    (showLibraryColumn ? 7 : 0) +
    (showMonitoredColumn ? 5.25 : 0) +
    (showQualityColumn ? 8 : 0) +
    (showEpisodesColumn ? 9.5 : 0) +
    (showRuntimeColumn ? 6.5 : 0) +
    (showStatusColumn ? 7 : 0) +
    (showSizeColumn ? 7 : 0) +
    (showResolutionColumn ? 6 : 0) +
    (showHdrColumn ? 7.5 : 0) +
    (showAudioCodecColumn ? 8 : 0) +
    (showPopularityColumn ? 6.5 : 0) +
    (showRootColumn ? 12 : 0) +
    showRatingColumns.reduce(
      (total, key) => total + titleTableRatingColumnWidthRem(key),
      0,
    ) +
    (showAddedColumn ? 7.5 : 0) +
    (showActionsColumn ? 11.5 : 0);
  const columnCount =
    2 +
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
  const titleTableColGroup = (
    <colgroup>
      <col style={{ width: "3rem" }} />
      <col />
      {showYearColumn ? <col style={{ width: "5rem" }} /> : null}
      {showRatingColumns.map((key) => (
        <col
          key={key}
          style={{ width: `${titleTableRatingColumnWidthRem(key)}rem` }}
        />
      ))}
      {showLibraryColumn ? <col style={{ width: "7rem" }} /> : null}
      {showMonitoredColumn ? <col style={{ width: "5.25rem" }} /> : null}
      {showQualityColumn ? <col style={{ width: "8rem" }} /> : null}
      {showEpisodesColumn ? <col style={{ width: "9.5rem" }} /> : null}
      {showRuntimeColumn ? <col style={{ width: "6.5rem" }} /> : null}
      {showStatusColumn ? <col style={{ width: "7rem" }} /> : null}
      {showSizeColumn ? <col style={{ width: "7rem" }} /> : null}
      {showResolutionColumn ? <col style={{ width: "6rem" }} /> : null}
      {showHdrColumn ? <col style={{ width: "7.5rem" }} /> : null}
      {showAudioCodecColumn ? <col style={{ width: "8rem" }} /> : null}
      {showPopularityColumn ? <col style={{ width: "6.5rem" }} /> : null}
      {showRootColumn ? <col style={{ width: "12rem" }} /> : null}
      {showAddedColumn ? <col style={{ width: "7.5rem" }} /> : null}
      {showActionsColumn ? <col style={{ width: "11.5rem" }} /> : null}
    </colgroup>
  );
  const visibleColumnSignature = [
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
    ? "poster-table-selected"
    : "poster-table";
  const initialScrollOffset = React.useMemo(
    () =>
      readOverviewSavedScroll(location.pathname, scrollStorageKeySuffix) ?? 0,
    [location.pathname, scrollStorageKeySuffix],
  );
  const expandedInteractiveRowSignature = React.useMemo(
    () => Array.from(expandedInteractiveRows).sort().join("|"),
    [expandedInteractiveRows],
  );

  const titleTableVirtualizerRef =
    React.useRef<VirtualizedTitleTableBodyHandle>(null);
  const estimateTitleRowSize = React.useCallback(
    (index: number) => {
      const titleId = sortedTitles[index]?.id;
      return (
        100 +
        (titleId && expandedInteractiveRows.has(titleId)
          ? TITLE_TABLE_INTERACTIVE_PANEL_ESTIMATED_HEIGHT
          : 0)
      );
    },
    [expandedInteractiveRows, sortedTitles],
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
      const titleId = title.id;
      setAutoQueueLoadingByTitle((prev) => ({ ...prev, [titleId]: true }));
      void Promise.resolve(onAutoQueue(title)).finally(() => {
        setAutoQueueLoadingByTitle((prev) => {
          if (!prev[titleId]) return prev;
          const next = { ...prev };
          delete next[titleId];
          return next;
        });
      });
    },
    [onAutoQueue],
  );

  const handleRunInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      setInteractiveSearchLoadingByTitle((prev) => ({
        ...prev,
        [titleId]: true,
      }));
      void Promise.resolve(onInteractiveSearch(title))
        .then((results) => {
          setInteractiveSearchResultsByTitle((prev) => ({
            ...prev,
            [titleId]: results ?? [],
          }));
        })
        .finally(() => {
          setInteractiveSearchLoadingByTitle((prev) => {
            if (!prev[titleId]) return prev;
            const next = { ...prev };
            delete next[titleId];
            return next;
          });
        });
    },
    [onInteractiveSearch],
  );

  const handleToggleInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      const isOpen = expandedInteractiveRows.has(titleId);
      setExpandedInteractiveRows((prev) => {
        const next = new Set(prev);
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
    const posterThumbUrl = selectPosterVariantUrl(item.posterUrl, "w70");
    const posterActionIconClassName = "h-[18px] w-[18px]";
    const isSelected = selectedTitleId === item.id;
    const isRowHighlighted =
      isSelected || (selectionMode && selectedTitleIds.has(item.id));
    const contextPanelControlsId = onSelectTitle ? contextPanelId : undefined;
    const selectedContextPanelControlsId = isSelected
      ? contextPanelControlsId
      : undefined;
    const downloadActive = activeDownloadTitleIds?.has(item.id) ?? false;
    const addedLabel =
      formatTitleDate(item.createdAt, dateTimeFormat) ?? t("label.unknown");

    return (
      <React.Fragment key={item.id}>
        <TableRow
          id={titleOverviewRowId(item.id)}
          data-ui="title-table-row"
          data-selected={isRowHighlighted ? "true" : undefined}
          aria-selected={onSelectTitle ? isSelected : undefined}
          aria-current={isSelected ? "true" : undefined}
          aria-controls={selectedContextPanelControlsId}
          aria-label={
            onSelectTitle ? t("title.selectTitle", { name: item.name }) : undefined
          }
          aria-keyshortcuts={onSelectTitle ? "Enter Space" : undefined}
          tabIndex={onSelectTitle ? 0 : undefined}
          onClick={(event) => handleTitleRowClick(event, item)}
          onKeyDown={(event) => handleTitleRowKeyDown(event, item)}
          className={cn(
            "h-[100px]",
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
          <TableCell className="align-middle overflow-hidden">
            <div className="flex min-w-0 items-center gap-2">
              <button
                id={titleOverviewOpenButtonId(item.id)}
                type="button"
                onClick={() => handleActivateTitle(item)}
                data-ui="title-name"
                aria-current={isSelected ? "true" : undefined}
                aria-controls={selectedContextPanelControlsId}
                tabIndex={onSelectTitle ? -1 : undefined}
                className="flex min-w-0 flex-1 items-center gap-3 overflow-hidden text-left text-[14px] font-semibold leading-5 text-[var(--scry-ink3)] hover:text-foreground"
              >
                <span
                  data-ui="poster-thumb"
                  className="relative h-[71px] w-12 shrink-0 overflow-hidden rounded-[7px] border border-[var(--scry-border2)] bg-[var(--scry-soft)]"
                >
                  <TitlePosterSlot
                    src={posterThumbUrl}
                    metadataFetchedAt={item.metadataFetchedAt}
                    createdAt={item.createdAt}
                    alt=""
                    className="h-full w-full object-cover"
                    placeholderClassName="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground"
                    emptyLabel={t("label.noArt")}
                    loading="lazy"
                  />
                  <span
                    aria-hidden="true"
                    className="pointer-events-none absolute inset-0 bg-[linear-gradient(180deg,transparent_60%,rgba(4,6,12,0.55))]"
                  />
                </span>
                <span className="block min-w-0 truncate">{item.name}</span>
              </button>
              {downloadActive ? (
                <TitleDownloadActivityPill />
              ) : null}
            </div>
          </TableCell>
          {showYearColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {item.year ?? "—"}
            </TableCell>
          ) : null}
          {showRatingColumns.map((columnKey) => (
            <TableCell
              key={columnKey}
              className="whitespace-nowrap text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]"
            >
              {titleTableRatingColumnValue(item, columnKey)}
            </TableCell>
          ))}
          {showLibraryColumn ? (
            <TableCell className="overflow-hidden text-center align-middle text-[12.5px] text-[var(--scry-muted)]">
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
                  className="inline-flex h-6 w-6 shrink-0 items-center justify-center"
                  aria-label={`${t("title.table.monitored")}: ${item.name}`}
                >
                  {item.monitored ? (
                    <Eye className="size-[17px] text-[var(--scry-success-text-soft)]" />
                  ) : (
                    <EyeOff className="size-[17px] text-[var(--scry-faint2)]" />
                  )}
                </span>
              </ActionTooltip>
            </TableCell>
          ) : null}
          {showQualityColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle text-[12.5px] text-[var(--scry-text4)]">
              {resolveDisplayedQualityLabel(item, t("label.unknown"))}
            </TableCell>
          ) : null}
          {showEpisodesColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle">
              <TitleEpisodeProgressBar item={item} t={t} />
            </TableCell>
          ) : null}
          {showRuntimeColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {formatRuntimeMinutes(item.runtimeMinutes)}
            </TableCell>
          ) : null}
          {showStatusColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle text-[12px]">
              {item.contentStatus ? (
                <StatusBadge status={item.contentStatus} t={t} />
              ) : (
                <span className="text-[var(--scry-faint2)]">—</span>
              )}
            </TableCell>
          ) : null}
          {showSizeColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {bytesToReadable(item.sizeBytes)}
            </TableCell>
          ) : null}
          {showResolutionColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {formatResolutionLabel(item.mediaResolution)}
            </TableCell>
          ) : null}
          {showHdrColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle text-[12.5px] text-[var(--scry-text4)]">
              {formatHdrLabel(item.mediaHdr)}
            </TableCell>
          ) : null}
          {showAudioCodecColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle text-[12.5px] text-[var(--scry-text4)]">
              {formatAudioCodecLabel(item.mediaAudioCodec)}
            </TableCell>
          ) : null}
          {showPopularityColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {formatCatalogPopularity(item.popularity)}
            </TableCell>
          ) : null}
          {showRootColumn ? (
            <TableCell className="overflow-hidden text-center align-middle text-[12px] text-[var(--scry-muted)]">
              <span className="block truncate" title={item.rootFolderPath ?? ""}>
                {item.rootFolderPath ?? item.rootFolderId ?? "—"}
              </span>
            </TableCell>
          ) : null}
          {showAddedColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle text-[12px] text-[var(--scry-muted)]">
              {addedLabel}
            </TableCell>
          ) : null}
          {showActionsColumn ? (
            <TableCell className="overflow-hidden px-2 text-center align-middle">
              <div
                data-ui="row-actions"
                className={cn(
                  "flex w-full min-w-0 items-center justify-center gap-1.5",
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
                  className={TITLE_TABLE_ACTION_BUTTON_CLASS}
                >
                  {autoQueueLoading ? (
                    <Loader2
                      className={cn(
                        posterActionIconClassName,
                        "animate-spin text-[var(--scry-accent-text)]",
                      )}
                    />
                  ) : (
                    <Zap className={posterActionIconClassName} />
                  )}
                </TitleTableTooltipActionButton>
                <TitleTableTooltipActionButton
                  id={titleOverviewInteractiveSearchButtonId(item.id)}
                  tone="accent"
                  label={t("label.interactiveSearch")}
                  tooltip={t("help.interactiveSearchTooltip")}
                  onClick={() => handleToggleInteractiveSearch(item)}
                  disabled={bulkActionBusy}
                  className={TITLE_TABLE_ACTION_BUTTON_CLASS}
                >
                  <Search className={posterActionIconClassName} />
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
                    className={TITLE_TABLE_ACTION_BUTTON_CLASS}
                  >
                    {monitorToggleLoading ? (
                      <Loader2
                        className={cn(
                          posterActionIconClassName,
                          "animate-spin",
                        )}
                      />
                    ) : item.monitored ? (
                      <EyeOff className={posterActionIconClassName} />
                    ) : (
                      <Eye className={posterActionIconClassName} />
                    )}
                  </TitleTableTooltipActionButton>
                ) : null}
                <TitleTableTooltipActionButton
                  id={titleOverviewDeleteButtonId(item.id)}
                  tone="delete"
                  label={t("label.delete")}
                  onClick={() => onDelete(item)}
                  disabled={deleteLoading || bulkActionBusy}
                  className={TITLE_TABLE_ACTION_BUTTON_CLASS}
                >
                  {deleteLoading ? (
                    <Loader2
                      className={cn(posterActionIconClassName, "animate-spin")}
                    />
                  ) : (
                    <Trash2 className={posterActionIconClassName} />
                  )}
                </TitleTableTooltipActionButton>
              </div>
            </TableCell>
          ) : null}
        </TableRow>
        {isPanelOpen ? (
          <TableRow
            id={titleOverviewInteractiveSearchPanelId(item.id)}
            data-ui="title-table-panel-row"
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
                    onQueue={(release) => onQueueFromInteractive(item, release)}
                    onQueueAdditional={
                      onQueueAdditionalFromInteractive
                        ? (release) =>
                            onQueueAdditionalFromInteractive(item, release)
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

  const titleTableHeader = (
    <TableHeader>
      <TableRow className={TITLE_TABLE_HEADER_ROW_CLASS}>
        <TableHead className="w-12 bg-[var(--scry-surfD)] text-center">
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
              "whitespace-nowrap px-2 text-center",
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
      <div
        data-slot="title-list-root-config-empty"
        className="flex h-full min-h-[22rem] w-full items-start justify-center px-4 pt-12"
      >
        <TitleCollectionEmptyState
          t={t}
          showConfigureRootsAction={showConfigureRootsAction}
          configureRootsReason={configureRootsReason}
          configureRootsHref={configureRootsHref}
        />
      </div>
    );
  }

  return (
    <div
      data-slot="title-list-scroll"
      ref={titleTableScrollRef}
      className={cn(
        "relative h-full min-h-[22rem] w-full overflow-x-auto overflow-y-auto rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surfD)]",
      )}
    >
      <TooltipProvider delayDuration={300}>
        <table
          data-ui="title-table"
          data-view={view}
          className="w-full table-fixed caption-bottom text-sm"
          style={{ minWidth: `${titleTableMinWidthRem}rem` }}
        >
        {titleTableColGroup}
        {titleTableHeader}
        <VirtualizedTitleTableBody
          ref={titleTableVirtualizerRef}
          titles={sortedTitles}
          scrollRef={titleTableScrollRef}
          initialScrollOffset={initialScrollOffset}
          estimateSize={estimateTitleRowSize}
          overscan={5}
          rebuildKey={`${visibleColumnSignature}:${expandedInteractiveRowSignature}`}
          selectedTitleId={selectedTitleId}
          selectedTitleScrollKey={selectedTitleScrollKey}
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
  );
});
