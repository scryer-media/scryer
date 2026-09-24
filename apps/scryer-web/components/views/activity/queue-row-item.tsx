import {
  ArrowDownToLine,
  CircleAlert,
  CircleOff,
  Link2,
  Pause,
  Play,
  Trash2,
  XCircle,
} from "lucide-react";
import { memo } from "react";

import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import {
  ActivityQueueDetailsPanel,
  ActivityQueueSeedingProgress,
  ActivityQueueStatusBadge,
  ActivityQueueTitleContent,
} from "@/components/views/activity/queue-row-presentation";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import type { DownloadQueueItem } from "@/lib/types";
import {
  type ActivityTab,
  formatBytes,
  getProgressBarColor,
  type QueueRowPresentation,
  type TranslateFn,
} from "@/lib/utils/activity-utils";
import { selectorId } from "@/lib/utils/dom-ids";
import { LoadingMark } from "@/components/common/loading-mark";
import { diskSpaceReason, importTitleHref, isWaitingForDiskSpace } from "@/lib/utils/import-row";

export type QueueRowItemProps = {
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

export const QueueRowItem = memo(function QueueRowItem({
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
}: QueueRowItemProps) {
  return (
    <div
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
      className="rounded-xl border border-border bg-card/40 p-3"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 flex-1 items-start gap-3">
          {activeTab === "import" ? (
            <Checkbox
              checked={isImportSelected}
              aria-label={t("activity.selectImportItem")}
              className="mt-0.5"
              onCheckedChange={onToggleImportSelected}
            />
          ) : null}
          <div className="min-w-0 flex-1">
            <ActivityQueueTitleContent
              titleHref={importTitleHref(queueItem)}
              displayTitle={row.displayTitle}
              releaseTitle={row.releaseTitle}
            />
            <p className="mt-1 text-xs text-muted-foreground">
              {queueItem.clientName || queueItem.clientType} • {queueItem.clientType}
              {queueItem.sourceProvider ? (
                <span
                  data-ui="activity-source-provider"
                  data-source-provider={queueItem.sourceProvider}
                >{` • ${queueItem.sourceProvider}`}</span>
              ) : null}
            </p>
          </div>
        </div>
        <div className="shrink-0">
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
        </div>
      </div>
      {(isWaitingForDiskSpace(queueItem) ||
        ((queueItem.deleteErrorMessage || queueItem.importErrorMessage) && !row.hasStatusDetails)) ? (
        <p className="mt-2 break-words text-xs text-[var(--scry-danger-text-soft)]">
          {isWaitingForDiskSpace(queueItem) ? diskSpaceReason(queueItem) : queueItem.deleteErrorMessage ?? queueItem.importErrorMessage}
        </p>
      ) : null}
      {row.hasExpandableDetails && isExpanded ? (
        <div className="mt-3">
          <ActivityQueueDetailsPanel
            detailId={detailId}
            releaseTitle={row.releaseTitle}
            errorCode={queueItem.importErrorCode}
            failureReason={isWaitingForDiskSpace(queueItem) ? "" : row.failureReason}
            t={t}
          />
        </div>
      ) : null}
      <div className="mt-3">
        <ActivityProgressBar
          percent={row.percent}
          remainingLabel={row.remainingLabel}
          colorClass={getProgressBarColor(row.displayStateKey)}
        />
      </div>
      <div className="mt-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span>{formatBytes(queueItem.sizeBytes)}</span>
        <ActivityQueueSeedingProgress queueItem={queueItem} t={t} />
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        {row.canPause && (
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className={`flex-1 ${rowActionVisualClass}`}
            disabled={isRowFullyBusy}
            onClick={() => {
              if (
                isActionLoading || isRowBlocked
              ) {
                return;
              }
              onPause();
            }}
          >
            <Pause className="h-4 w-4" />
            <span>{t("queue.pause")}</span>
          </Button>
        )}
        {row.canResume && (
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className={`flex-1 ${rowActionVisualClass}`}
            disabled={isRowFullyBusy}
            onClick={() => {
              if (
                isActionLoading || isRowBlocked
              ) {
                return;
              }
              onResume();
            }}
          >
            <Play className="h-4 w-4" />
            <span>{t("queue.resume")}</span>
          </Button>
        )}
        {(row.canInteractiveManualImport || row.canDirectManualImport) && (
          <Button
            id={selectorId("activity", activeTab, "manual-import", rowSelectorKey)}
            type="button"
            size="sm"
            variant="secondary"
            className={`flex-1 ${rowActionVisualClass}`}
            disabled={isRowFullyBusy}
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
              <LoadingMark className="h-4 w-4" />
            ) : (
              <ArrowDownToLine className="h-4 w-4" />
            )}
            <span>
              {isManualImportPending
                ? t("queue.manualImporting")
                : t("queue.manualImportTooltip")}
            </span>
          </Button>
        )}
        {row.canAssignTitle && (
          <Button
            id={selectorId("activity", activeTab, "assign-title", rowSelectorKey)}
            type="button"
            size="sm"
            variant="secondary"
            className={`flex-1 ${rowActionVisualClass}`}
            disabled={isRowFullyBusy}
            onClick={() => {
              if (
                isActionLoading || isRowBlocked
              ) {
                return;
              }
              onAssignTitle();
            }}
          >
            <Link2 className="h-4 w-4" />
            <span>
              {row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId
                ? t("queue.assignTitle")
                : t("queue.reassignTitle")}
            </span>
          </Button>
        )}
        {row.canIgnore && (
          <Button
            type="button"
            id={selectorId("activity", activeTab, "ignore", rowSelectorKey)}
            size="sm"
            variant="secondary"
            className={`flex-1 ${rowActionVisualClass}`}
            disabled={isRowFullyBusy}
            onClick={() => {
              if (
                isActionLoading || isRowBlocked
              ) {
                return;
              }
              onIgnore();
            }}
          >
            <CircleOff className="h-4 w-4" />
            <span>{t("queue.ignore")}</span>
          </Button>
        )}
        {row.canMarkFailed && (
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className={`flex-1 ${rowActionVisualClass}`}
            disabled={isRowFullyBusy}
            onClick={() => {
              if (isActionLoading || isRowBlocked) {
                return;
              }
              onMarkFailedSearchAgain();
            }}
          >
            <CircleAlert className="h-4 w-4" />
            <span>{t("queue.markFailedSearchAgain")}</span>
          </Button>
        )}
        {row.canMarkFailed && (
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className={`flex-1 ${rowActionVisualClass}`}
            disabled={isRowFullyBusy}
            onClick={() => {
              if (isActionLoading || isRowBlocked) {
                return;
              }
              onMarkFailedOnly();
            }}
          >
            <XCircle className="h-4 w-4" />
            <span>{t("queue.markFailedOnly")}</span>
          </Button>
        )}
        <Button
          type="button"
          size="sm"
          variant="destructive"
          className={`flex-1 ${rowActionVisualClass}`}
          disabled={isRowFullyBusy}
          onClick={() => {
            if (
              isActionLoading || isRowBlocked
            ) {
              return;
            }
            onRequestDelete();
          }}
        >
          <Trash2 className="h-4 w-4" />
          <span>{t("label.delete")}</span>
        </Button>
      </div>
    </div>
  );
});
