import {
  ArrowDownToLine,
  CircleAlert,
  CircleOff,
  Link2,
  Loader2,
  Pause,
  Play,
  Trash2,
  XCircle,
} from "lucide-react";
import { Fragment, memo, type ReactNode, useLayoutEffect, useRef } from "react";

import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import {
  ActivityQueueDetailsPanel,
  ActivityQueueSeedingProgress,
  ActivityQueueStatusBadge,
  ActivityQueueTitleContent,
} from "@/components/views/activity/queue-row-presentation";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  TableCell,
  TableCheckboxCell,
  TableCodeCell,
  TableRow,
} from "@/components/ui/table";
import { ActionTooltip } from "@/components/ui/tooltip";
import type { DownloadQueueItem } from "@/lib/types";
import {
  type ActivityTab,
  formatBytes,
  getProgressBarColor,
  type QueueRowPresentation,
  type TranslateFn,
} from "@/lib/utils/activity-utils";
import { sameDownloadQueueItem } from "@/lib/utils/download-queue";
import { selectorId } from "@/lib/utils/dom-ids";

export type QueueTableRowProps = {
  queueItem: DownloadQueueItem;
  row: QueueRowPresentation;
  activeTab: ActivityTab;
  rowId: string;
  rowSelectorKey: string;
  detailId: string;
  isActionLoading: boolean;
  isRowBlocked: boolean;
  isRowFullyBusy: boolean;
  isManualImportPending: boolean;
  isExpanded: boolean;
  isImportSelected: boolean;
  rowActionVisualClass: string;
  virtualIndex?: number;
  measureElement?: (element: HTMLTableRowElement | null) => void;
  /**
   * Marks the row as static content that sits above the virtualised rows, so
   * the activity view can fold its height into the virtualiser scroll margin.
   */
  isVirtualPrefix?: boolean;
  t: TranslateFn;
  onToggleImportSelected: () => void;
  onToggleExpanded: () => void;
  onPause: () => void;
  onResume: () => void;
  onManualImport: () => void;
  onAssignTitle: () => void;
  onIgnore: () => void;
  onMarkFailedSearchAgain: () => void;
  onMarkFailedOnly: () => void;
  onRequestDelete: () => void;
};

type QueueIconActionProps = {
  id?: string;
  className: string;
  disabled: boolean;
  label: string;
  tooltip?: ReactNode;
  children: ReactNode;
  onClick: () => void;
};

function QueueIconAction({
  id,
  className,
  disabled,
  label,
  tooltip,
  children,
  onClick,
}: QueueIconActionProps) {
  return (
    <ActionTooltip content={tooltip ?? label} wrapperTabIndex={disabled ? 0 : undefined}>
      <Button
        id={id}
        type="button"
        size="sm"
        variant="secondary"
        className={className}
        disabled={disabled}
        aria-label={label}
        onClick={onClick}
      >
        {children}
      </Button>
    </ActionTooltip>
  );
}

function queueTableRowPropsEqual(
  previous: Readonly<QueueTableRowProps>,
  next: Readonly<QueueTableRowProps>,
): boolean {
  // `row` is derived solely from the item and translator, and the action
  // closures close over that same immutable item. Ignore their recreated
  // identities so virtual-scroll updates do not rerender retained rows.
  return (
    sameDownloadQueueItem(previous.queueItem, next.queueItem) &&
    previous.activeTab === next.activeTab &&
    previous.rowId === next.rowId &&
    previous.rowSelectorKey === next.rowSelectorKey &&
    previous.detailId === next.detailId &&
    previous.isActionLoading === next.isActionLoading &&
    previous.isRowBlocked === next.isRowBlocked &&
    previous.isRowFullyBusy === next.isRowFullyBusy &&
    previous.isManualImportPending === next.isManualImportPending &&
    previous.isExpanded === next.isExpanded &&
    previous.isImportSelected === next.isImportSelected &&
    previous.rowActionVisualClass === next.rowActionVisualClass &&
    previous.virtualIndex === next.virtualIndex &&
    previous.measureElement === next.measureElement &&
    previous.isVirtualPrefix === next.isVirtualPrefix &&
    previous.t === next.t
  );
}

export const QueueTableRow = memo(function QueueTableRow({
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
  virtualIndex,
  measureElement,
  isVirtualPrefix,
  t,
  onToggleImportSelected,
  onToggleExpanded,
  onPause,
  onResume,
  onManualImport,
  onAssignTitle,
  onIgnore,
  onMarkFailedSearchAgain,
  onMarkFailedOnly,
  onRequestDelete,
}: QueueTableRowProps) {
  const rowElementRef = useRef<HTMLTableRowElement | null>(null);
  useLayoutEffect(() => {
    if (rowElementRef.current) {
      measureElement?.(rowElementRef.current);
    }
  }, [isExpanded, measureElement]);

  return (
    <Fragment>
      <TableRow
        ref={(element: HTMLTableRowElement | null) => {
          rowElementRef.current = element;
          measureElement?.(element);
        }}
        data-index={virtualIndex}
        data-activity-virtual-prefix={isVirtualPrefix ? "" : undefined}
        id={selectorId("activity", activeTab, "row", rowSelectorKey)}
        data-ui="activity-row"
        data-activity-tab={activeTab}
        data-activity-row-id={rowId}
        data-activity-download-id={queueItem.id}
        data-activity-client-item-id={queueItem.downloadClientItemId}
        data-activity-title-id={queueItem.titleId ?? ""}
        data-activity-client-id={queueItem.clientId}
        data-activity-client-name={queueItem.clientName ?? ""}
        data-activity-client-type={queueItem.clientType}
      >
        {activeTab === "import" ? (
          <TableCheckboxCell>
            <Checkbox
              checked={isImportSelected}
              aria-label={t("activity.selectImportItem")}
              onCheckedChange={onToggleImportSelected}
              size="table"
              className="mx-auto"
            />
          </TableCheckboxCell>
        ) : null}
        <TableCell className="w-[32%]">
          <ActivityQueueTitleContent
            displayTitle={row.displayTitle}
            releaseTitle={row.releaseTitle}
          />
        </TableCell>
        <TableCell className="w-[13%] align-middle">
          <p className="break-words whitespace-normal text-sm">
            {queueItem.clientName || queueItem.clientType}
          </p>
          <p className="text-xs text-muted-foreground">{queueItem.clientType}</p>
        </TableCell>
        <TableCell className="w-[15%] align-middle">
          <ActivityQueueStatusBadge
            stateKey={row.statusBadgeKey}
            statusLabel={row.statusLabel}
            isExpandable={row.hasExpandableDetails}
            isExpanded={isExpanded}
            detailId={detailId}
            expandLabel={t(
              isExpanded ? "queue.hideDetails" : "queue.showDetails",
            )}
            onToggle={onToggleExpanded}
          />
          <ActivityQueueSeedingProgress
            queueItem={queueItem}
            className="mt-1"
            t={t}
          />
          {(queueItem.deleteErrorMessage || queueItem.importErrorMessage) &&
            !row.hasStatusDetails && (
            <p
              className="mt-1 max-w-full break-words whitespace-normal text-xs text-[var(--scry-danger-text-soft)]"
              title={queueItem.deleteErrorMessage ?? queueItem.importErrorMessage ?? ""}
            >
              {queueItem.deleteErrorMessage ?? queueItem.importErrorMessage}
            </p>
          )}
        </TableCell>
        {activeTab === "activity" || activeTab === "import" ? (
          <TableCell className="w-[16%] align-middle">
            <ActivityProgressBar
              percent={row.percent}
              remainingLabel={row.remainingLabel}
              colorClass={getProgressBarColor(row.displayStateKey)}
            />
          </TableCell>
        ) : null}
        <TableCodeCell className="w-28 text-center align-middle text-muted-foreground">
          {formatBytes(queueItem.sizeBytes)}
        </TableCodeCell>
        <TableCell className="w-52 align-middle text-center">
          <div className="flex flex-wrap items-center justify-center gap-1.5">
            {row.canPause && (
              <QueueIconAction
                className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                label={t("queue.pause")}
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onPause();
                }}
              >
                <Pause className="h-6 w-6" />
              </QueueIconAction>
            )}
            {row.canResume && (
              <QueueIconAction
                className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                label={t("queue.resume")}
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onResume();
                }}
              >
                <Play className="h-6 w-6" />
              </QueueIconAction>
            )}
            {(row.canInteractiveManualImport || row.canDirectManualImport) && (
              <QueueIconAction
                id={selectorId("activity", activeTab, "manual-import", rowSelectorKey)}
                className={`h-10 w-10 border border-[var(--scry-success-border-strong)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)] hover:bg-[var(--scry-success-bg-strong)] ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                label={
                  isManualImportPending
                    ? t("queue.manualImporting")
                    : t("queue.manualImportTooltip")
                }
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onManualImport();
                }}
              >
                {isManualImportPending ? (
                  <Loader2 className="h-5 w-5 animate-spin" />
                ) : (
                  <ArrowDownToLine className="h-5 w-5" />
                )}
              </QueueIconAction>
            )}
            {row.canAssignTitle && (
              <QueueIconAction
                id={selectorId("activity", activeTab, "assign-title", rowSelectorKey)}
                className={`h-10 w-10 border border-[var(--scry-warning-border-strong)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)] hover:bg-[var(--scry-warning-bg-strong)] ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                label={
                  row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId
                    ? t("queue.assignTitle")
                    : t("queue.reassignTitle")
                }
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onAssignTitle();
                }}
              >
                <Link2 className="h-5 w-5" />
              </QueueIconAction>
            )}
            {row.canIgnore && (
              <QueueIconAction
                className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                label={t("queue.ignore")}
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onIgnore();
                }}
              >
                <CircleOff className="h-5 w-5" />
              </QueueIconAction>
            )}
            {row.canMarkFailed && (
              <QueueIconAction
                className={`h-10 w-10 border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)] hover:bg-[var(--scry-warning-bg-strong)] ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                label={t("queue.markFailedSearchAgain")}
                onClick={() => {
                  if (isActionLoading || isRowBlocked) {
                    return;
                  }
                  onMarkFailedSearchAgain();
                }}
              >
                <CircleAlert className="h-5 w-5" />
              </QueueIconAction>
            )}
            {row.canMarkFailed && (
              <QueueIconAction
                className={`h-10 w-10 border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)] hover:bg-[var(--scry-danger-bg-strong)] ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                label={t("queue.markFailedOnly")}
                onClick={() => {
                  if (isActionLoading || isRowBlocked) {
                    return;
                  }
                  onMarkFailedOnly();
                }}
              >
                <XCircle className="h-5 w-5" />
              </QueueIconAction>
            )}
            <QueueIconAction
              className={`h-10 w-10 border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)] hover:bg-[var(--scry-danger-bg-strong)] ${rowActionVisualClass}`}
              disabled={isRowFullyBusy}
              label={t("queue.removeFromDownloader")}
              onClick={() => {
                if (
                  isActionLoading || isRowBlocked
                ) {
                  return;
                }
                onRequestDelete();
              }}
            >
              <Trash2 className="h-6 w-6" />
            </QueueIconAction>
          </div>
        </TableCell>
      </TableRow>
      {row.hasExpandableDetails && isExpanded ? (
        <TableRow
          data-virtual-detail-index={virtualIndex}
          data-activity-virtual-prefix={isVirtualPrefix ? "" : undefined}
        >
          <TableCell
            colSpan={activeTab === "activity" ? 6 : activeTab === "import" ? 7 : 5}
            className="bg-muted/10 p-3"
          >
            <ActivityQueueDetailsPanel
              detailId={detailId}
              releaseTitle={row.releaseTitle}
              errorCode={queueItem.importErrorCode}
              failureReason={row.failureReason}
              t={t}
            />
          </TableCell>
        </TableRow>
      ) : null}
    </Fragment>
  );
}, queueTableRowPropsEqual);
