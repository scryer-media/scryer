
import {
  ActivitySquare,
  ArrowDown,
  ArrowDownToLine,
  ArrowUp,
  ChevronRight,
  CircleOff,
  CircleAlert,
  Clock3,
  Filter,
  HardDrive,
  Loader2,
  Pause,
  Trash2,
  XCircle,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  type UIEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { DownloadClientTypeLogo } from "@/components/common/download-client-type-logo";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Card, CardContent } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { QueueRowItem } from "@/components/views/activity/queue-row-item";
import { QueueTableRow } from "@/components/views/activity/queue-table-row";
import {
  Table,
  TableBody,
  TableCell,
  TableCheckboxHead,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type {
  ActivitySortKey,
  ActiveImportStream,
  DownloadActivityStatus,
  DownloadClientFilterOption,
  DownloadImportStatus,
  DownloadQueueItem,
  SortConfig,
} from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";
import { downloadQueueItemIdentityKey } from "@/lib/utils/download-queue";
import {
  activityStatusRank,
  type ActivityTab,
  canDeleteImportItem,
  canIgnoreImportItem,
  compareStrings,
  deriveQueueRowPresentation,
  downloadQueueItemRowSelectorKey,
  effectiveQueueItemProgress,
  formatByteCount,
  parseByteCount,
  type QueueRowPresentation,
  queueStateLabels,
  type TranslateFn,
} from "@/lib/utils/activity-utils";

/**
 * Seeding rows are already imported, so only a handful stay on screen; the rest
 * sit behind a disclosure row.
 */
const VISIBLE_SEEDING_ROW_LIMIT = 5;

/**
 * How many consecutive pages may be fetched to fill a queue list that folded
 * its seeding rows away before the view stops asking for more.
 */
const MAX_QUEUE_AUTO_FILL_PAGES = 10;

type ActivityViewState = {
  queueItems: DownloadQueueItem[];
  activeImportStreams: ActiveImportStream[];
  queueLoading: boolean;
  queueLoadingMore: boolean;
  queueError: string | null;
  queueStale: boolean;
  onVisibleQueueOffsetChange?: (offset: number) => void;
  requestManualImport: (item: DownloadQueueItem) => Promise<void>;
  requestAssignTitle: (item: DownloadQueueItem) => Promise<void>;
  requestIgnore: (item: DownloadQueueItem) => Promise<void>;
  requestMarkFailed: (item: DownloadQueueItem, skipReacquire: boolean) => Promise<void>;
  requestIgnoreItems: (items: DownloadQueueItem[]) => Promise<void>;
  requestPause: (item: DownloadQueueItem) => Promise<void>;
  requestResume: (item: DownloadQueueItem) => Promise<void>;
  requestDelete: (item: DownloadQueueItem) => Promise<void>;
  requestDeleteItems: (items: DownloadQueueItem[]) => Promise<void>;
  requestCancelActiveImport: (stream: ActiveImportStream) => Promise<void>;
  activeTab: ActivityTab;
  sortConfigByTab: Record<ActivityTab, SortConfig>;
  toggleSort: (tab: ActivityTab, key: ActivitySortKey) => void;
  activityScryerSubmittedOnly: boolean;
  toggleActivityScryerSubmittedOnly: () => void;
  selectedImportStatuses: DownloadImportStatus[];
  toggleImportStatus: (status: DownloadImportStatus) => void;
  selectedActivityStatuses: DownloadActivityStatus[];
  toggleActivityStatus: (status: DownloadActivityStatus) => void;
  activityAvailableClients: DownloadClientFilterOption[];
  selectedActivityClientIds: string[];
  toggleActivityClientId: (clientId: string) => void;
  visibleHasMore: boolean;
  requestMoreItems: () => Promise<void>;
};

type ActivityFilterChipOption<T extends string> = {
  value: T;
  labelKey: string;
  icon: LucideIcon;
  iconClassName?: string;
};

const importFilterOptions: ActivityFilterChipOption<DownloadImportStatus>[] = [
  {
    value: "PENDING",
    labelKey: "activity.importFilter.pending",
    icon: Clock3,
    iconClassName: "text-[var(--scry-accent-text)]",
  },
  {
    value: "BLOCKED",
    labelKey: "activity.importFilter.blocked",
    icon: CircleAlert,
    iconClassName: "text-[var(--scry-warning-text)]",
  },
  {
    value: "FAILED",
    labelKey: "activity.importFilter.failed",
    icon: XCircle,
    iconClassName: "text-[var(--scry-danger-text-soft)]",
  },
];

const activityFilterOptions: ActivityFilterChipOption<DownloadActivityStatus>[] = [
  {
    value: "DOWNLOADING",
    labelKey: "activity.activityFilter.downloading",
    icon: ArrowDownToLine,
    iconClassName: "text-[var(--scry-info-text-soft)]",
  },
  {
    value: "QUEUED",
    labelKey: "activity.activityFilter.queued",
    icon: Clock3,
    iconClassName: "text-[var(--scry-warning-text)]",
  },
  {
    value: "PAUSED",
    labelKey: "activity.activityFilter.paused",
    icon: Pause,
    iconClassName: "text-[var(--scry-warning-text)]",
  },
  {
    value: "POST_PROCESSING",
    labelKey: "activity.activityFilter.postProcessing",
    icon: HardDrive,
    iconClassName: "text-[var(--scry-info-text-soft)]",
  },
  {
    value: "SEEDING",
    labelKey: "activity.activityFilter.seeding",
    icon: ArrowUp,
    iconClassName: "text-[var(--scry-success-text)]",
  },
  {
    value: "WARNING",
    labelKey: "activity.activityFilter.warning",
    icon: CircleAlert,
    iconClassName: "text-[var(--scry-warning-text)]",
  },
];

function ActivityFilterSection<T extends string>({
  title,
  options,
  selectedValues,
  onToggle,
  t,
}: {
  title: string;
  options: ActivityFilterChipOption<T>[];
  selectedValues: string[];
  onToggle: (value: T) => void;
  t: TranslateFn;
}) {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs font-medium text-muted-foreground">{title}</p>
      <div className="flex flex-col gap-1">
        {options.map((option) => {
          const Icon = option.icon;
          const isSelected = selectedValues.includes(option.value);
          return (
            <label
              key={option.value}
              className="flex cursor-pointer items-center gap-2 rounded-md px-1.5 py-1 text-sm hover:bg-accent/50"
            >
              <Checkbox
                checked={isSelected}
                size="compact"
                onCheckedChange={() => onToggle(option.value)}
              />
              <Icon
                className={cn(
                  "h-[14px] w-[14px] shrink-0",
                  option.iconClassName ?? "text-muted-foreground",
                )}
                aria-hidden="true"
              />
              <span>{t(option.labelKey)}</span>
            </label>
          );
        })}
      </div>
    </div>
  );
}

function ActivityClientFilterSection({
  title,
  options,
  selectedValues,
  onToggle,
}: {
  title: string;
  options: DownloadClientFilterOption[];
  selectedValues: string[];
  onToggle: (clientId: string) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs font-medium text-muted-foreground">{title}</p>
      <div className="flex flex-col gap-1">
        {options.map((option) => {
          const isSelected = selectedValues.includes(option.clientId);
          return (
            <label
              key={option.clientId}
              className="flex cursor-pointer items-center gap-2 rounded-md px-1.5 py-1 text-sm hover:bg-accent/50"
              title={`${option.clientName} • ${option.clientType}`}
            >
              <Checkbox
                checked={isSelected}
                size="compact"
                onCheckedChange={() => onToggle(option.clientId)}
              />
              <DownloadClientTypeLogo
                typeValue={option.clientType}
                className="h-[14px] w-[14px] shrink-0"
              />
              <span>{option.clientName || option.clientType}</span>
            </label>
          );
        })}
      </div>
    </div>
  );
}

function ActivityBooleanFilterSection({
  title,
  label,
  checked,
  onToggle,
}: {
  title: string;
  label: string;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs font-medium text-muted-foreground">{title}</p>
      <label className="flex cursor-pointer items-center gap-2 rounded-md px-1.5 py-1 text-sm hover:bg-accent/50">
        <Checkbox
          checked={checked}
          size="compact"
          onCheckedChange={onToggle}
        />
        <span>{label}</span>
      </label>
    </div>
  );
}

function ActivityTableLoadingMask({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center py-16">
      <div className="inline-flex items-center gap-2 rounded-full border border-border/70 bg-background/90 px-4 py-2 text-sm text-muted-foreground shadow-sm backdrop-blur-sm">
        <Loader2 className="h-4 w-4 animate-spin" />
        <span>{label}</span>
      </div>
    </div>
  );
}

export function ActivityView({ state }: { state: ActivityViewState }) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const {
    queueItems,
    activeImportStreams,
    queueLoading,
    queueLoadingMore,
    queueError,
    queueStale,
    onVisibleQueueOffsetChange,
    requestManualImport,
    requestAssignTitle,
    requestIgnore,
    requestMarkFailed,
    requestIgnoreItems,
    requestPause,
    requestResume,
    requestDelete,
    requestDeleteItems,
    requestCancelActiveImport,
    activeTab,
    sortConfigByTab,
    toggleSort,
    activityScryerSubmittedOnly,
    toggleActivityScryerSubmittedOnly,
    selectedImportStatuses,
    toggleImportStatus,
    selectedActivityStatuses,
    toggleActivityStatus,
    activityAvailableClients,
    selectedActivityClientIds,
    toggleActivityClientId,
    visibleHasMore,
    requestMoreItems,
  } = state;
  const [actionLoadingId, setActionLoadingId] = useState<string | null>(null);
  const [deleteConfirmItem, setDeleteConfirmItem] = useState<DownloadQueueItem | null>(null);
  const [cancelImportConfirmStream, setCancelImportConfirmStream] =
    useState<ActiveImportStream | null>(null);
  const [cancelImportInProgress, setCancelImportInProgress] = useState(false);
  const [bulkDeleteConfirmItems, setBulkDeleteConfirmItems] = useState<DownloadQueueItem[]>([]);
  const [deleteInProgress, setDeleteInProgress] = useState(false);
  const [bulkActionInProgress, setBulkActionInProgress] = useState<"ignore" | "delete" | null>(
    null,
  );
  const [rowActionBusy, setRowActionBusy] = useState<Record<string, true>>({});
  const [expandedItemIds, setExpandedItemIds] = useState<Record<string, true>>({});
  const [selectedImportItemKeys, setSelectedImportItemKeys] = useState<Record<string, true>>({});
  const [filterPopoverOpen, setFilterPopoverOpen] = useState(false);
  const rowActionBusyRef = useRef<Record<string, true>>({});
  const resultsScrollRef = useRef<HTMLDivElement>(null);
  const [queueScrollMargin, setQueueScrollMargin] = useState(0);
  const [activeImportsExpanded, setActiveImportsExpanded] = useState(false);
  const [seedingExpanded, setSeedingExpanded] = useState(false);
  const scrollHeightClass = isMobile ? "max-h-[70vh]" : "max-h-[1700px]";

  const setRowBusy = useCallback((rowId: string, busy: boolean) => {
    rowActionBusyRef.current = busy
      ? { ...rowActionBusyRef.current, [rowId]: true }
      : Object.fromEntries(
          Object.entries(rowActionBusyRef.current).filter(([id]) => id !== rowId),
        );
    setRowActionBusy((current) => {
      if (!busy) {
        const { [rowId]: _removed, ...next } = current;
        return next;
      }
      if (current[rowId]) {
        return current;
      }
      return {
        ...current,
        [rowId]: true,
      };
    });
  }, []);

  const handleDelete = useCallback(async () => {
    if (!deleteConfirmItem) return;
    const rowId = downloadQueueItemIdentityKey(deleteConfirmItem);
    setRowBusy(rowId, true);
    setDeleteInProgress(true);
    try {
      await requestDelete(deleteConfirmItem);
    } finally {
      setDeleteInProgress(false);
      setRowBusy(rowId, false);
      setDeleteConfirmItem(null);
    }
  }, [deleteConfirmItem, requestDelete, setRowBusy]);

  const handleCancelActiveImport = useCallback(async () => {
    if (!cancelImportConfirmStream) {
      return;
    }
    setCancelImportInProgress(true);
    try {
      await requestCancelActiveImport(cancelImportConfirmStream);
      setCancelImportConfirmStream(null);
    } finally {
      setCancelImportInProgress(false);
    }
  }, [cancelImportConfirmStream, requestCancelActiveImport]);

  const clearSelectedImportItems = useCallback((items: DownloadQueueItem[]) => {
    const keys = new Set(items.map(downloadQueueItemIdentityKey));
    setSelectedImportItemKeys((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(([key]) => !keys.has(key)),
      );
      return Object.keys(next).length === Object.keys(current).length ? current : next;
    });
  }, []);

  const handleBulkIgnore = useCallback(async (items: DownloadQueueItem[]) => {
    if (items.length === 0) {
      return;
    }

    const rowIds = items.map(downloadQueueItemIdentityKey);
    rowIds.forEach((rowId) => setRowBusy(rowId, true));
    setBulkActionInProgress("ignore");
    try {
      await requestIgnoreItems(items);
      clearSelectedImportItems(items);
    } finally {
      rowIds.forEach((rowId) => setRowBusy(rowId, false));
      setBulkActionInProgress(null);
    }
  }, [clearSelectedImportItems, requestIgnoreItems, setRowBusy]);

  const handleBulkDelete = useCallback(async () => {
    if (bulkDeleteConfirmItems.length === 0) {
      return;
    }

    const items = bulkDeleteConfirmItems;
    const rowIds = items.map(downloadQueueItemIdentityKey);
    rowIds.forEach((rowId) => setRowBusy(rowId, true));
    setBulkActionInProgress("delete");
    setDeleteInProgress(true);
    try {
      await requestDeleteItems(items);
      clearSelectedImportItems(items);
      setBulkDeleteConfirmItems([]);
    } finally {
      setDeleteInProgress(false);
      setBulkActionInProgress(null);
      rowIds.forEach((rowId) => setRowBusy(rowId, false));
    }
  }, [
    bulkDeleteConfirmItems,
    clearSelectedImportItems,
    requestDeleteItems,
    setRowBusy,
  ]);

  const toggleExpandedDetails = useCallback((rowId: string) => {
    setExpandedItemIds((current) => {
      if (current[rowId]) {
        const { [rowId]: _removed, ...next } = current;
        return next;
      }

      return {
        ...current,
        [rowId]: true,
      };
    });
  }, []);

  const handleResultsScroll = useCallback(
    (event: UIEvent<HTMLDivElement>) => {
      const element = event.currentTarget;
      if (
        !queueLoadingMore &&
        visibleHasMore &&
        !queueLoading &&
        element.scrollHeight - element.scrollTop - element.clientHeight <= 160
      ) {
        void requestMoreItems();
      }
    },
    [queueLoading, queueLoadingMore, requestMoreItems, visibleHasMore],
  );

  const emptyStateLabel =
    activeTab === "import"
      ? t("activity.importEmpty")
      : t("activity.activityEmpty");
  const activeSortConfig = sortConfigByTab[activeTab];

  const handleSort = useCallback(
    (nextKey: ActivitySortKey) => {
      toggleSort(activeTab, nextKey);
    },
    [activeTab, toggleSort],
  );

  const renderSortIcon = useCallback(
    (key: ActivitySortKey) => {
      if (activeSortConfig.key !== key) {
        return null;
      }

      return activeSortConfig.direction === "ASC" ? (
        <ArrowUp className="h-3.5 w-3.5" />
      ) : (
        <ArrowDown className="h-3.5 w-3.5" />
      );
    },
    [activeSortConfig.direction, activeSortConfig.key],
  );

  const renderSortableHeader = useCallback(
    (key: ActivitySortKey, label: string, className?: string) => {
      const fixedActivitySort = activeTab === "activity";
      return (
        <TableHead
          className={className}
          aria-sort={
            fixedActivitySort
              ? key === "PROGRESS"
                ? "descending"
                : "none"
              : activeSortConfig.key === key
                ? activeSortConfig.direction === "ASC"
                  ? "ascending"
                  : "descending"
                : "none"
          }
        >
          {fixedActivitySort ? (
            <span className="inline-flex w-full items-center gap-1 text-left font-medium text-foreground">
              <span>{label}</span>
              {key === "PROGRESS" ? <ArrowDown className="h-3.5 w-3.5" /> : null}
            </span>
          ) : (
          <button
            type="button"
            className="inline-flex w-full items-center gap-1 text-left font-medium text-foreground transition-colors hover:text-foreground/80"
            onClick={() => handleSort(key)}
          >
            <span>{label}</span>
            {renderSortIcon(key)}
          </button>
          )}
        </TableHead>
      );
    },
    [activeSortConfig.direction, activeSortConfig.key, activeTab, handleSort, renderSortIcon],
  );

  const sortedQueueItems = useMemo(() => {
    if (activeTab === "activity") {
      return queueItems;
    }

    const directionMultiplier = activeSortConfig.direction === "ASC" ? 1 : -1;
    const items = [...queueItems];

    items.sort((leftItem, rightItem) => {
      let comparison = 0;

      switch (activeSortConfig.key) {
        case "TITLE": {
          const leftTitle = leftItem.titleName.trim() || leftItem.downloadClientItemId.trim();
          const rightTitle = rightItem.titleName.trim() || rightItem.downloadClientItemId.trim();
          comparison = compareStrings(leftTitle, rightTitle);
          break;
        }
        case "CLIENT": {
          const leftClient = leftItem.clientName.trim() || leftItem.clientType.trim();
          const rightClient = rightItem.clientName.trim() || rightItem.clientType.trim();
          comparison = compareStrings(leftClient, rightClient);
          if (comparison === 0) {
            comparison = compareStrings(leftItem.clientType, rightItem.clientType);
          }
          break;
        }
        case "STATUS": {
          comparison =
            activityStatusRank(activeTab, leftItem.displayState) -
            activityStatusRank(activeTab, rightItem.displayState);
          if (comparison === 0) {
            const leftStatus = t(queueStateLabels[leftItem.displayState.toLowerCase()] ?? "queue.state.unknown");
            const rightStatus = t(
              queueStateLabels[rightItem.displayState.toLowerCase()] ?? "queue.state.unknown",
            );
            comparison = compareStrings(leftStatus, rightStatus);
          }
          break;
        }
        case "PROGRESS": {
          comparison =
            effectiveQueueItemProgress(leftItem) - effectiveQueueItemProgress(rightItem);
          break;
        }
        case "SIZE": {
          const leftSize = parseByteCount(leftItem.sizeBytes) ?? 0;
          const rightSize = parseByteCount(rightItem.sizeBytes) ?? 0;
          comparison = leftSize - rightSize;
          break;
        }
      }

      if (comparison === 0) {
        const leftTitle = leftItem.titleName.trim() || leftItem.downloadClientItemId.trim();
        const rightTitle = rightItem.titleName.trim() || rightItem.downloadClientItemId.trim();
        comparison = compareStrings(leftTitle, rightTitle);
      }

      return comparison * directionMultiplier;
    });

    return items;
  }, [activeSortConfig.direction, activeSortConfig.key, activeTab, queueItems, t]);

  const hiddenActiveImportCount = Math.max(0, activeImportStreams.length - 7);
  const visibleActiveImportStreams = activeImportsExpanded
    ? activeImportStreams
    : activeImportStreams.slice(0, 7);
  const activeImportDisclosureLabel = activeImportsExpanded
    ? "Show fewer import activities"
    : `Click to see ${hiddenActiveImportCount} more import activities`;
  // Seeding rows are finished work that keeps sitting in the queue, so they are
  // lifted out of the virtualised list and disclosed as their own capped group
  // the same way active imports are.
  const seedingQueueItems = useMemo(
    () =>
      activeTab === "activity"
        ? sortedQueueItems.filter((item) => item.displayState === "IMPORTED_SEEDING")
        : [],
    [activeTab, sortedQueueItems],
  );
  const virtualQueueItems = useMemo(
    () =>
      activeTab === "activity" && seedingQueueItems.length > 0
        ? sortedQueueItems.filter((item) => item.displayState !== "IMPORTED_SEEDING")
        : sortedQueueItems,
    [activeTab, seedingQueueItems.length, sortedQueueItems],
  );
  const hiddenSeedingCount = Math.max(0, seedingQueueItems.length - VISIBLE_SEEDING_ROW_LIMIT);
  const visibleSeedingQueueItems = seedingExpanded
    ? seedingQueueItems
    : seedingQueueItems.slice(0, VISIBLE_SEEDING_ROW_LIMIT);
  const seedingDisclosureLabel = seedingExpanded
    ? "Show fewer seeding activities"
    : `Click to see ${hiddenSeedingCount} more seeding activities`;
  const activeImportLayoutKey = `${visibleActiveImportStreams
    .map((stream) => stream.id)
    .join("|")}:${hiddenActiveImportCount}:${activeImportsExpanded}:${visibleSeedingQueueItems
    .map((item) => {
      // Expanding a seeding row grows the prefix, so the key has to move with it.
      const key = downloadQueueItemIdentityKey(item);
      return expandedItemIds[key] ? `${key}:open` : key;
    })
    .join("|")}:${hiddenSeedingCount}:${seedingExpanded}`;
  useLayoutEffect(() => {
    if (activeTab !== "activity" || isMobile) {
      setQueueScrollMargin(0);
      return;
    }

    const scrollElement = resultsScrollRef.current;
    if (!scrollElement) {
      return;
    }

    const prefixElements = Array.from(
      scrollElement.querySelectorAll<HTMLElement>("[data-activity-virtual-prefix]"),
    );
    const measurePrefix = () => {
      const nextMargin = prefixElements.reduce(
        (height, element) => height + element.getBoundingClientRect().height,
        0,
      );
      setQueueScrollMargin((currentMargin) =>
        Math.abs(currentMargin - nextMargin) < 0.5 ? currentMargin : nextMargin,
      );
    };

    measurePrefix();
    const resizeObserver = new ResizeObserver(measurePrefix);
    resizeObserver.observe(scrollElement);
    prefixElements.forEach((element) => resizeObserver.observe(element));
    return () => resizeObserver.disconnect();
  }, [activeImportLayoutKey, activeTab, isMobile, queueLoading]);

  const queueVirtualizer = useVirtualizer({
    count: activeTab === "activity" ? virtualQueueItems.length : 0,
    getScrollElement: () => resultsScrollRef.current,
    getItemKey: (index) =>
      downloadQueueItemIdentityKey(virtualQueueItems[index] ?? queueItems[index]),
    estimateSize: () => (isMobile ? 180 : 64),
    measureElement: (element) => {
      const baseHeight = element.getBoundingClientRect().height;
      const index = (element as HTMLElement).dataset.index;
      const detailRow = element.nextElementSibling as HTMLElement | null;
      return detailRow && detailRow.dataset.virtualDetailIndex === index
        ? baseHeight + detailRow.getBoundingClientRect().height
        : baseHeight;
    },
    scrollMargin: queueScrollMargin,
    overscan: 8,
  });
  const virtualRows = activeTab === "activity" ? queueVirtualizer.getVirtualItems() : [];
  const firstVisibleQueueIndex = virtualRows.find(
    (virtualRow) => virtualRow.end > (queueVirtualizer.scrollOffset ?? 0),
  )?.index;
  useEffect(() => {
    if (activeTab === "activity" && firstVisibleQueueIndex !== undefined) {
      onVisibleQueueOffsetChange?.(firstVisibleQueueIndex);
    }
  }, [activeTab, firstVisibleQueueIndex, onVisibleQueueOffsetChange]);
  const renderedQueueItems =
    activeTab === "activity"
      ? virtualRows
          .map((virtualRow) => virtualQueueItems[virtualRow.index])
          .filter((item): item is DownloadQueueItem => Boolean(item))
      : virtualQueueItems;
  const virtualPaddingTop = Math.max(
    0,
    (virtualRows[0]?.start ?? queueScrollMargin) - queueScrollMargin,
  );
  const virtualPaddingBottom = virtualRows.length
    ? Math.max(
        0,
        queueVirtualizer.getTotalSize() -
          (virtualRows[virtualRows.length - 1].end - queueScrollMargin),
      )
    : 0;

  // The queue arrives sorted by progress, so a queue with a lot of seeding
  // entries can fill whole server pages with rows that are folded away here.
  // Without this the list never overflows, the scroll handler never fires, and
  // the downloads sitting on later pages stay unreachable. Give up once a run
  // of pages has added nothing visible so a wholly seeding queue cannot walk
  // itself to the end.
  const autoFillAttemptsRef = useRef(0);
  const autoFilledRowCountRef = useRef(0);
  useEffect(() => {
    if (activeTab !== "activity") {
      autoFillAttemptsRef.current = 0;
      autoFilledRowCountRef.current = 0;
      return;
    }
    if (virtualQueueItems.length > autoFilledRowCountRef.current) {
      autoFillAttemptsRef.current = 0;
    }
    autoFilledRowCountRef.current = virtualQueueItems.length;
    if (
      queueLoading ||
      queueLoadingMore ||
      !visibleHasMore ||
      autoFillAttemptsRef.current >= MAX_QUEUE_AUTO_FILL_PAGES
    ) {
      return;
    }
    const scrollElement = resultsScrollRef.current;
    if (!scrollElement || scrollElement.scrollHeight > scrollElement.clientHeight + 160) {
      return;
    }
    autoFillAttemptsRef.current += 1;
    void requestMoreItems();
  }, [
    activeTab,
    queueLoading,
    queueLoadingMore,
    requestMoreItems,
    virtualQueueItems.length,
    visibleHasMore,
  ]);

  const visibleImportItems = useMemo(
    () => (activeTab === "import" ? sortedQueueItems : []),
    [activeTab, sortedQueueItems],
  );
  const selectedImportItems = useMemo(
    () =>
      visibleImportItems.filter((item) => selectedImportItemKeys[downloadQueueItemIdentityKey(item)]),
    [selectedImportItemKeys, visibleImportItems],
  );
  const selectedImportCount = selectedImportItems.length;
  const visibleImportKeys = useMemo(
    () => visibleImportItems.map(downloadQueueItemIdentityKey),
    [visibleImportItems],
  );
  const allVisibleImportItemsSelected =
    visibleImportKeys.length > 0 &&
    visibleImportKeys.every((key) => selectedImportItemKeys[key]);
  const someVisibleImportItemsSelected =
    !allVisibleImportItemsSelected &&
    visibleImportKeys.some((key) => selectedImportItemKeys[key]);
  const selectedIgnoreItems = useMemo(
    () =>
      selectedImportItems.filter((item) => {
        const rowId = downloadQueueItemIdentityKey(item);
        return (
          canIgnoreImportItem(item) &&
          !rowActionBusy[rowId] &&
          actionLoadingId !== rowId
        );
      }),
    [actionLoadingId, rowActionBusy, selectedImportItems],
  );
  const selectedDeleteItems = useMemo(
    () =>
      selectedImportItems.filter((item) => {
        const rowId = downloadQueueItemIdentityKey(item);
        return (
          canDeleteImportItem(item) &&
          !rowActionBusy[rowId] &&
          actionLoadingId !== rowId
        );
      }),
    [actionLoadingId, rowActionBusy, selectedImportItems],
  );

  useEffect(() => {
    if (activeTab !== "import") {
      setSelectedImportItemKeys({});
      return;
    }

    const visibleKeys = new Set(visibleImportKeys);
    setSelectedImportItemKeys((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(([key]) => visibleKeys.has(key)),
      );
      return Object.keys(next).length === Object.keys(current).length ? current : next;
    });
  }, [activeTab, visibleImportKeys]);

  const toggleImportItemSelected = useCallback((item: DownloadQueueItem) => {
    const rowId = downloadQueueItemIdentityKey(item);
    setSelectedImportItemKeys((current) => {
      if (current[rowId]) {
        const { [rowId]: _removed, ...next } = current;
        return next;
      }
      return {
        ...current,
        [rowId]: true,
      };
    });
  }, []);

  const toggleAllVisibleImportItemsSelected = useCallback(() => {
    setSelectedImportItemKeys((current) => {
      if (visibleImportKeys.length === 0) {
        return current;
      }

      const allSelected = visibleImportKeys.every((key) => current[key]);
      if (allSelected) {
        const visibleKeySet = new Set(visibleImportKeys);
        return Object.fromEntries(
          Object.entries(current).filter(([key]) => !visibleKeySet.has(key)),
        );
      }

      const next = { ...current };
      for (const key of visibleImportKeys) {
        next[key] = true;
      }
      return next;
    });
  }, [visibleImportKeys]);

  const renderFilterPopoverContent = useCallback(() => {
    if (activeTab === "import") {
      return (
        <ActivityFilterSection
          title={t("queue.status")}
          options={importFilterOptions}
          selectedValues={selectedImportStatuses}
          onToggle={(value) => toggleImportStatus(value as DownloadImportStatus)}
          t={t}
        />
      );
    }

    return (
      <div className="flex flex-col gap-4">
        <ActivityFilterSection
          title={t("queue.status")}
          options={activityFilterOptions}
          selectedValues={selectedActivityStatuses}
          onToggle={(value) => toggleActivityStatus(value as DownloadActivityStatus)}
          t={t}
        />
        <ActivityBooleanFilterSection
          title={t("queue.source")}
          label={t("activity.scryerSubmitted")}
          checked={activityScryerSubmittedOnly}
          onToggle={toggleActivityScryerSubmittedOnly}
        />
        {activityAvailableClients.length > 0 ? (
          <ActivityClientFilterSection
            title={t("queue.client")}
            options={activityAvailableClients}
            selectedValues={selectedActivityClientIds}
            onToggle={toggleActivityClientId}
          />
        ) : null}
      </div>
    );
  }, [
    activeTab,
    activityAvailableClients,
    activityScryerSubmittedOnly,
    selectedActivityClientIds,
    selectedActivityStatuses,
    selectedImportStatuses,
    t,
    toggleActivityClientId,
    toggleActivityScryerSubmittedOnly,
    toggleActivityStatus,
    toggleImportStatus,
  ]);

  const buildQueueRowProps = useCallback(
    (queueItem: DownloadQueueItem) => {
      const rowId = downloadQueueItemIdentityKey(queueItem);
      const row: QueueRowPresentation = deriveQueueRowPresentation(queueItem, t);
      const rowSelectorKey = selectorId(
        downloadQueueItemRowSelectorKey(queueItem, rowId),
      );
      const isActionLoading = actionLoadingId === rowId;
      const isRowBusy = rowActionBusy[rowId] ?? false;
      const isManualImportPending = row.displayStateKey.toLowerCase() === "importing";
      const isDeletePending = row.displayStateKey.toLowerCase() === "removing";
      const isRowBlocked =
        isRowBusy || isManualImportPending || isDeletePending || isActionLoading;
      const isDeleteConfirming =
        deleteConfirmItem !== null &&
        downloadQueueItemIdentityKey(deleteConfirmItem) === rowId;
      const isRowFullyBusy = isRowBlocked || isDeleteConfirming;
      const isExpanded = Boolean(expandedItemIds[rowId]);
      const detailId = `activity-queue-details-${rowId}`;
      const rowActionVisualClass = isRowFullyBusy
        ? "pointer-events-none opacity-45 grayscale"
        : "";
      const isImportSelected = Boolean(selectedImportItemKeys[rowId]);

      return {
        queueItem,
        row,
        activeTab,
        rowId,
        rowSelectorKey,
        detailId,
        isActionLoading,
        isRowBlocked,
        isRowFullyBusy,
        isManualImportPending,
        isExpanded,
        isImportSelected,
        rowActionVisualClass,
        t,
        onToggleImportSelected: () => toggleImportItemSelected(queueItem),
        onToggleExpanded: () => toggleExpandedDetails(rowId),
        onPause: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestPause(queueItem).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((c) => (c === rowId ? null : c));
          });
        },
        onResume: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestResume(queueItem).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((c) => (c === rowId ? null : c));
          });
        },
        onManualImport: () => {
          setRowBusy(rowId, true);
          void requestManualImport(queueItem).finally(() => {
            setRowBusy(rowId, false);
          });
        },
        onAssignTitle: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestAssignTitle(queueItem).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((current) => (current === rowId ? null : current));
          });
        },
        onIgnore: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestIgnore(queueItem).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((current) => (current === rowId ? null : current));
          });
        },
        onMarkFailedSearchAgain: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestMarkFailed(queueItem, false).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((current) => (current === rowId ? null : current));
          });
        },
        onMarkFailedOnly: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestMarkFailed(queueItem, true).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((current) => (current === rowId ? null : current));
          });
        },
        onRequestDelete: () => {
          setRowBusy(rowId, true);
          setDeleteConfirmItem(queueItem);
        },
      };
    },
    [
      activeTab,
      actionLoadingId,
      deleteConfirmItem,
      expandedItemIds,
      requestAssignTitle,
      requestIgnore,
      requestManualImport,
      requestMarkFailed,
      requestPause,
      requestResume,
      rowActionBusy,
      selectedImportItemKeys,
      setRowBusy,
      t,
      toggleExpandedDetails,
      toggleImportItemSelected,
    ],
  );

  const renderMobileQueueCards = (
    items: DownloadQueueItem[],
    showHistorySpinner = false,
  ) => (
    <div className="space-y-3">
      {items.map((queueItem) => {
        const rowProps = buildQueueRowProps(queueItem);
        return <QueueRowItem key={rowProps.rowId} {...rowProps} />;
      })}
      {showHistorySpinner ? (
        <div className="flex items-center justify-center py-3 text-sm text-muted-foreground">
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          {t("label.loading")}
        </div>
      ) : null}
    </div>
  );

  const renderVirtualMobileQueueCards = () => (
    <div style={{ height: queueVirtualizer.getTotalSize(), position: "relative" }}>
      {virtualRows.map((virtualRow) => {
        const queueItem = virtualQueueItems[virtualRow.index];
        if (!queueItem) {
          return null;
        }
        const rowProps = buildQueueRowProps(queueItem);
        return (
          <div
            key={rowProps.rowId}
            ref={queueVirtualizer.measureElement}
            data-index={virtualRow.index}
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              transform: `translateY(${virtualRow.start}px)`,
              paddingBottom: 12,
            }}
          >
            <QueueRowItem {...rowProps} />
          </div>
        );
      })}
      {queueLoadingMore ? (
        <div
          className="absolute left-0 flex w-full items-center justify-center py-3 text-sm text-muted-foreground"
          style={{ top: queueVirtualizer.getTotalSize() }}
        >
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          {t("label.loading")}
        </div>
      ) : null}
    </div>
  );

  const renderDesktopQueueRows = (
    items: DownloadQueueItem[],
    showHistorySpinner = false,
  ) => (
    <>
      {items.map((queueItem, itemIndex) => {
        const rowProps = buildQueueRowProps(queueItem);
        const virtualRow = activeTab === "activity" ? virtualRows[itemIndex] : undefined;
        return (
          <QueueTableRow
            key={rowProps.rowId}
            {...rowProps}
            virtualIndex={virtualRow?.index}
            measureElement={
              virtualRow ? queueVirtualizer.measureElement : undefined
            }
          />
        );
      })}
      {showHistorySpinner ? (
        <TableRow>
          <TableCell
            colSpan={activeTab === "activity" ? 6 : activeTab === "import" ? 7 : 5}
            className="py-4 text-center text-sm text-muted-foreground"
          >
            <span className="inline-flex items-center">
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              {t("label.loading")}
            </span>
          </TableCell>
        </TableRow>
      ) : null}
    </>
  );
  const renderActiveImportRows = () => (
    <>
      {visibleActiveImportStreams.map((stream) => {
      const progress =
        stream.totalBytes > 0
          ? Math.min(100, Math.round((stream.bytes / stream.totalBytes) * 100))
          : null;
      const startedAt = stream.startedAt ?? stream.queuedAt;
      const status = stream.cancellationRequested
        ? "Cancelling"
        : stream.phase === "QUEUED"
          ? "Queued for import"
          : stream.phase[0] + stream.phase.slice(1).toLowerCase();
      const destinationName = stream.destinationPath.split(/[\\/]/).pop() || stream.destinationPath;

      return (
        <TableRow
          key={`active-import-${stream.id}`}
          className="bg-[var(--scry-accent-bg)]/25"
          data-activity-virtual-prefix
          data-ui="activity-row"
        >
          <TableCell className="min-w-0 align-middle">
            <div className="truncate font-medium text-foreground" title={stream.destinationPath}>
              {destinationName}
            </div>
            <div className="truncate text-xs text-muted-foreground" title={`${stream.sourcePath} → ${stream.destinationPath}`}>
              {stream.sourcePath} → {stream.destinationPath}
            </div>
          </TableCell>
          <TableCell className="text-sm text-muted-foreground">Filesystem</TableCell>
          <TableCell className="text-sm">
            <div>{status}</div>
            <div className="text-xs text-muted-foreground">
              Started {new Date(startedAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
            </div>
          </TableCell>
          <TableCell>
            {progress === null ? (
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                <div className="h-full w-1/2 animate-pulse rounded-full bg-primary" />
              </div>
            ) : (
              <div className="space-y-1">
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                  <div className="h-full rounded-full bg-primary" style={{ width: `${progress}%` }} />
                </div>
                <div className="text-xs text-muted-foreground">
                  {formatByteCount(stream.bytes)} / {formatByteCount(stream.totalBytes)}
                </div>
              </div>
            )}
          </TableCell>
          <TableCell className="text-center text-sm text-muted-foreground">
            {stream.totalBytes > 0 ? formatByteCount(stream.totalBytes) : "—"}
          </TableCell>
          <TableCell className="text-center">
            {stream.cancellable ? (
              <Button
                type="button"
                size="icon"
                variant="ghost"
                aria-label="Cancel import"
                title="Cancel import"
                onClick={() => setCancelImportConfirmStream(stream)}
              >
                <XCircle className="h-4 w-4 text-[var(--scry-danger-text)]" />
              </Button>
            ) : null}
          </TableCell>
        </TableRow>
      );
      })}
      {hiddenActiveImportCount > 0 ? (
        <TableRow data-activity-virtual-prefix>
          <TableCell colSpan={6} className="p-0">
            <button
              type="button"
              aria-expanded={activeImportsExpanded}
              className="flex w-full items-center justify-center gap-2 px-4 py-2.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-[var(--scry-rowHover)] hover:text-foreground"
              onClick={() => setActiveImportsExpanded((expanded) => !expanded)}
            >
              <ChevronRight
                className={cn(
                  "h-3.5 w-3.5 transition-transform",
                  activeImportsExpanded && "rotate-90",
                )}
                aria-hidden="true"
              />
              {activeImportDisclosureLabel}
            </button>
          </TableCell>
        </TableRow>
      ) : null}
    </>
  );
  const renderSeedingRows = () => (
    <>
      {visibleSeedingQueueItems.map((queueItem) => {
        const rowProps = buildQueueRowProps(queueItem);
        return <QueueTableRow key={rowProps.rowId} {...rowProps} isVirtualPrefix />;
      })}
      {hiddenSeedingCount > 0 ? (
        <TableRow data-activity-virtual-prefix>
          <TableCell colSpan={6} className="p-0">
            <button
              type="button"
              aria-expanded={seedingExpanded}
              className="flex w-full items-center justify-center gap-2 px-4 py-2.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-[var(--scry-rowHover)] hover:text-foreground"
              onClick={() => setSeedingExpanded((expanded) => !expanded)}
            >
              <ChevronRight
                className={cn(
                  "h-3.5 w-3.5 transition-transform",
                  seedingExpanded && "rotate-90",
                )}
                aria-hidden="true"
              />
              {seedingDisclosureLabel}
            </button>
          </TableCell>
        </TableRow>
      ) : null}
    </>
  );

  const activeActivityLabel =
    activeTab === "import"
      ? t("activity.import")
      : t("activity.activity");

  return (
    <>
      <ConfirmDialog
        open={cancelImportConfirmStream !== null}
        title="Cancel import?"
        description="This stops the queued or active import. Source files are preserved and only temporary import output is removed."
        confirmLabel="Cancel import"
        cancelLabel={t("label.cancel")}
        isBusy={cancelImportInProgress}
        onConfirm={handleCancelActiveImport}
        onCancel={() => setCancelImportConfirmStream(null)}
      />
      <ConfirmDialog
        open={deleteConfirmItem !== null}
        title={t("queue.deleteConfirmTitle")}
        description={t("queue.deleteConfirmDescription")}
        confirmLabel={t("queue.removeFromDownloader")}
        cancelLabel={t("label.cancel")}
        isBusy={deleteInProgress}
        onConfirm={handleDelete}
        onCancel={() => {
          if (deleteConfirmItem) {
            setRowBusy(downloadQueueItemIdentityKey(deleteConfirmItem), false);
          }
          setDeleteConfirmItem(null);
        }}
      />
      <ConfirmDialog
        open={bulkDeleteConfirmItems.length > 0}
        title={t("queue.bulkDeleteConfirmTitle")}
        description={t("queue.bulkDeleteConfirmDescription", {
          count: bulkDeleteConfirmItems.length,
        })}
        confirmLabel={t("queue.removeFromDownloader")}
        cancelLabel={t("label.cancel")}
        isBusy={deleteInProgress}
        onConfirm={handleBulkDelete}
        onCancel={() => {
          setBulkDeleteConfirmItems([]);
        }}
      />
      <div className="min-w-0 flex-1 overflow-y-auto bg-transparent">
        <div className="mx-auto flex min-h-0 w-full max-w-none flex-1 flex-col px-4 py-5 sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]">
          <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
            <span>{t("nav.group.automation")}</span>
            <ChevronRight className="h-3.5 w-3.5" />
            <span className="font-semibold text-[var(--scry-accent-text)]">
              {activeActivityLabel}
            </span>
          </div>
          <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
            <div className="flex min-w-0 items-center gap-4">
              <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
                <ActivitySquare className="h-[23px] w-[23px]" />
              </div>
              <div className="min-w-0">
                <h1 className="text-[25px] font-bold tracking-normal text-[var(--scry-ink2)]">
                  {activeActivityLabel}
                </h1>
              </div>
            </div>
          </div>
        <Card
          id={selectorId("activity-view", activeTab)}
          className="min-h-0 flex-1 rounded-none border-0 bg-transparent shadow-none"
        >
          <CardContent className="space-y-3 p-0">
          {queueError ? (
            <p className="rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-2 text-sm text-[var(--scry-danger-text)]">
              {queueError}
            </p>
          ) : null}
          {queueStale ? (
            <p className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-2 text-sm text-[var(--scry-warning-text)]">
              {t("activity.queueStale")}
            </p>
          ) : null}
          <div
            className={cn(
              "flex flex-col gap-3 sm:flex-row sm:items-center",
              activeTab === "import" && selectedImportCount > 0
                ? "sm:justify-between"
                : "sm:justify-end",
            )}
          >
            {activeTab === "import" && selectedImportCount > 0 ? (
              <div className="flex flex-wrap items-center gap-2 rounded-lg border border-border/70 bg-card/60 px-3 py-2">
                <span className="text-sm text-muted-foreground">
                  {t("activity.selectedImportCount", { count: selectedImportCount })}
                </span>
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  disabled={bulkActionInProgress !== null || selectedIgnoreItems.length === 0}
                  onClick={() => {
                    void handleBulkIgnore(selectedIgnoreItems);
                  }}
                >
                  {bulkActionInProgress === "ignore" ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : (
                    <CircleOff className="mr-2 h-4 w-4" />
                  )}
                  {t("queue.ignore")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="destructive"
                  disabled={bulkActionInProgress !== null || selectedDeleteItems.length === 0}
                  onClick={() => {
                    setBulkDeleteConfirmItems(selectedDeleteItems);
                  }}
                >
                  {bulkActionInProgress === "delete" ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : (
                    <Trash2 className="mr-2 h-4 w-4" />
                  )}
                  {t("queue.removeFromDownloader")}
                </Button>
              </div>
            ) : null}
            <Popover open={filterPopoverOpen} onOpenChange={setFilterPopoverOpen}>
              <PopoverTrigger asChild>
                <Button
                  id={selectorId("activity", activeTab, "filter-button")}
                  type="button"
                  variant="outline"
                  size="sm"
                  className="inline-flex items-center gap-2"
                  aria-label={t("activity.filterBarLabel")}
                >
                  <Filter className="h-4 w-4" />
                  <span>{t("label.filters")}</span>
                </Button>
              </PopoverTrigger>
              <PopoverContent align="end" className="w-72 p-4">
                {renderFilterPopoverContent()}
              </PopoverContent>
            </Popover>
          </div>

          {activeTab === "activity" && activeImportStreams.length > 0 ? (
            <div className="space-y-2 sm:hidden">
              {visibleActiveImportStreams.map((stream) => {
                const progress =
                  stream.totalBytes > 0
                    ? Math.min(100, Math.round((stream.bytes / stream.totalBytes) * 100))
                    : null;
                const startedAt = stream.startedAt ?? stream.queuedAt;
                return (
                  <div
                    key={`active-import-mobile-${stream.id}`}
                    className="rounded-lg border border-primary/25 bg-primary/5 p-3"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <div className="truncate text-sm font-medium">
                          {stream.destinationPath.split(/[\\/]/).pop() || stream.destinationPath}
                        </div>
                        <div
                          className="truncate text-xs text-muted-foreground"
                          title={`${stream.sourcePath} → ${stream.destinationPath}`}
                        >
                          {stream.sourcePath} → {stream.destinationPath}
                        </div>
                        <div className="mt-1 text-xs text-muted-foreground">
                          {stream.cancellationRequested
                            ? "Cancelling"
                            : stream.phase === "QUEUED"
                              ? "Queued for import"
                              : stream.phase[0] + stream.phase.slice(1).toLowerCase()}
                        </div>
                        <div className="mt-1 text-xs text-muted-foreground">
                          Started{" "}
                          {new Date(startedAt).toLocaleTimeString([], {
                            hour: "numeric",
                            minute: "2-digit",
                          })}
                        </div>
                        {progress === null ? (
                          <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-muted">
                            <div className="h-full w-1/2 animate-pulse rounded-full bg-primary" />
                          </div>
                        ) : (
                          <div className="mt-2 space-y-1">
                            <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                              <div
                                className="h-full rounded-full bg-primary"
                                style={{ width: `${progress}%` }}
                              />
                            </div>
                            <div className="text-xs text-muted-foreground">
                              {formatByteCount(stream.bytes)} / {formatByteCount(stream.totalBytes)}
                            </div>
                          </div>
                        )}
                      </div>
                      {stream.cancellable ? (
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          onClick={() => setCancelImportConfirmStream(stream)}
                          aria-label="Cancel import"
                        >
                          <XCircle className="h-4 w-4 text-[var(--scry-danger-text)]" />
                        </Button>
                      ) : null}
                    </div>
                  </div>
                );
              })}
              {hiddenActiveImportCount > 0 ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="w-full gap-2 text-xs text-muted-foreground"
                  aria-expanded={activeImportsExpanded}
                  onClick={() => setActiveImportsExpanded((expanded) => !expanded)}
                >
                  <ChevronRight
                    className={cn(
                      "h-3.5 w-3.5 transition-transform",
                      activeImportsExpanded && "rotate-90",
                    )}
                    aria-hidden="true"
                  />
                  {activeImportDisclosureLabel}
                </Button>
              ) : null}
            </div>
          ) : null}
          {isMobile && activeTab === "activity" && seedingQueueItems.length > 0 ? (
            <div className="space-y-3">
              {visibleSeedingQueueItems.map((queueItem) => {
                const rowProps = buildQueueRowProps(queueItem);
                return <QueueRowItem key={rowProps.rowId} {...rowProps} />;
              })}
              {hiddenSeedingCount > 0 ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="w-full gap-2 text-xs text-muted-foreground"
                  aria-expanded={seedingExpanded}
                  onClick={() => setSeedingExpanded((expanded) => !expanded)}
                >
                  <ChevronRight
                    className={cn(
                      "h-3.5 w-3.5 transition-transform",
                      seedingExpanded && "rotate-90",
                    )}
                    aria-hidden="true"
                  />
                  {seedingDisclosureLabel}
                </Button>
              ) : null}
            </div>
          ) : null}
          {isMobile ? (
            sortedQueueItems.length === 0 && !queueLoading ? (
              activeTab === "activity" && activeImportStreams.length > 0 ? null : (
                <p className="text-sm text-muted-foreground">{emptyStateLabel}</p>
              )
            ) : sortedQueueItems.length === 0 ? (
              <div className={`${scrollHeightClass} overflow-y-auto pr-1`}>
                <div className="rounded-xl border border-border/60 bg-card/30">
                  <ActivityTableLoadingMask label={t("label.loading")} />
                </div>
              </div>
            ) : (
                <div
                  ref={resultsScrollRef}
                  onScroll={handleResultsScroll}
                  className={`${scrollHeightClass} overflow-y-auto pr-1`}
                >
                  {activeTab === "activity" ? (
                    renderVirtualMobileQueueCards()
                  ) : (
                    renderMobileQueueCards(renderedQueueItems, queueLoadingMore)
                  )}
                </div>
              )
          ) : (
            <div
              ref={resultsScrollRef}
              onScroll={handleResultsScroll}
              className={`${scrollHeightClass} overflow-y-auto rounded-xl border border-border/60`}
            >
              <Table overflow="clip" layout="fixed" density="dense">
                <TableHeader
                  data-activity-virtual-prefix={activeTab === "activity" ? "" : undefined}
                >
                  <TableRow>
                    {activeTab === "import" ? (
                      <TableCheckboxHead>
                        <Checkbox
                          checked={
                            allVisibleImportItemsSelected
                              ? true
                              : someVisibleImportItemsSelected
                                ? "indeterminate"
                                : false
                          }
                          disabled={visibleImportKeys.length === 0}
                          aria-label={t("activity.selectAllImportItems")}
                          onCheckedChange={toggleAllVisibleImportItemsSelected}
                          size="table"
                          className="mx-auto"
                        />
                      </TableCheckboxHead>
                    ) : null}
                    {renderSortableHeader(
                      "TITLE",
                      t("queue.title"),
                      "w-[32%]",
                    )}
                    {renderSortableHeader(
                      "CLIENT",
                      t("queue.client"),
                      "w-[13%]",
                    )}
                    {renderSortableHeader(
                      "STATUS",
                      t("queue.status"),
                      "w-[15%]",
                    )}
                    {activeTab === "activity" || activeTab === "import"
                      ? renderSortableHeader(
                          "PROGRESS",
                          t("queue.progress"),
                          "w-[16%]",
                        )
                      : null}
                    {renderSortableHeader(
                      "SIZE",
                      t("queue.size"),
                      "w-28 text-center [&_button]:justify-center [&_button]:text-center",
                    )}
                    <TableHead className="w-52 text-center">
                      {t("label.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {sortedQueueItems.length === 0 &&
                  !(activeTab === "activity" && activeImportStreams.length > 0) ? (
                    <TableRow>
                      <TableCell
                        colSpan={
                          activeTab === "activity"
                            ? 6
                            : activeTab === "import"
                              ? 7
                              : 5
                        }
                        className={
                          queueLoading
                            ? "p-0"
                            : "text-sm text-muted-foreground"
                        }
                      >
                        {queueLoading ? (
                          <ActivityTableLoadingMask label={t("label.loading")} />
                        ) : (
                          emptyStateLabel
                        )}
                      </TableCell>
                    </TableRow>
                  ) : (
                    <>
                      {activeTab === "activity" ? renderActiveImportRows() : null}
                      {activeTab === "activity" ? renderSeedingRows() : null}
                      {activeTab === "activity" && virtualPaddingTop > 0 ? (
                        <TableRow aria-hidden="true">
                          <TableCell colSpan={6} style={{ height: virtualPaddingTop, padding: 0 }} />
                        </TableRow>
                      ) : null}
                      {renderDesktopQueueRows(renderedQueueItems, queueLoadingMore)}
                      {activeTab === "activity" && virtualPaddingBottom > 0 ? (
                        <TableRow aria-hidden="true">
                          <TableCell
                            colSpan={6}
                            style={{ height: virtualPaddingBottom, padding: 0 }}
                          />
                        </TableRow>
                      ) : null}
                    </>
                  )}
                </TableBody>
              </Table>
            </div>
          )}
          </CardContent>
        </Card>
        </div>
      </div>
    </>
  );
}
