import { ChevronDown, ChevronUp, Lock } from "lucide-react";
import { Link } from "react-router";

import { ActionTooltip } from "@/components/ui/tooltip";
import type { DownloadQueueItem } from "@/lib/types";
import { queueStateClasses, type TranslateFn } from "@/lib/utils/activity-utils";
import { cn } from "@/lib/utils";
import {
  deriveSeedingProgress,
  isPrivateTorrentRow,
} from "@/lib/utils/seeding-progress";

export function ActivityQueueStatusBadge({
  stateKey,
  statusLabel,
  isExpandable,
  isExpanded,
  detailId,
  expandLabel,
  onToggle,
}: {
  stateKey: string;
  statusLabel: string;
  isExpandable: boolean;
  isExpanded: boolean;
  detailId: string;
  expandLabel: string;
  onToggle: () => void;
}) {
  const className = `inline-flex items-center gap-1.5 rounded border px-2 py-1 text-xs font-medium ${queueStateClasses[stateKey.toLowerCase()] ?? "border-border bg-muted text-card-foreground"}`;

  if (!isExpandable) {
    return <span className={className}>{statusLabel}</span>;
  }

  return (
    <button
      type="button"
      className={className}
      aria-expanded={isExpanded}
      aria-controls={detailId}
      aria-label={`${statusLabel}. ${expandLabel}`}
      onClick={onToggle}
    >
      <span>{statusLabel}</span>
      {isExpanded ? (
        <ChevronUp className="h-3.5 w-3.5 opacity-80" aria-hidden="true" />
      ) : (
        <ChevronDown className="h-3.5 w-3.5 opacity-80" aria-hidden="true" />
      )}
    </button>
  );
}

export function ActivityQueueTitleContent({
  displayTitle,
  releaseTitle,
  titleHref,
}: {
  displayTitle: string;
  releaseTitle: string;
  titleHref?: string | null;
}) {
  return (
    <div className="space-y-1">
      <p className="break-words whitespace-normal text-sm text-foreground">
        {titleHref ? (
          <Link to={titleHref} className="rounded underline decoration-muted-foreground underline-offset-4 hover:text-primary focus-visible:outline-2 focus-visible:outline-ring">
            {displayTitle}
          </Link>
        ) : displayTitle}
      </p>
      {releaseTitle !== displayTitle ? (
        <p
          className="break-words whitespace-normal text-xs text-muted-foreground"
          title={releaseTitle}
        >
          {releaseTitle}
        </p>
      ) : null}
    </div>
  );
}

export type ActivityQueueSeedingProgressItem = Pick<
  DownloadQueueItem,
  | "seedingState"
  | "seedRatio"
  | "seedRatioGoal"
  | "seedTimeSeconds"
  | "seedTimeGoalSeconds"
  | "isPrivate"
>;

/**
 * Read-only seeding summary for a queue row: how far the torrent has got
 * against whatever goal was resolved at grab time, and whether the tracker is
 * private. Renders nothing at all when the row is not a torrent, when the
 * client reports no seeding state, or when the state is `NONE`; renders no
 * number for an axis that was never observed. There are no controls here —
 * the existing remove action is the manual escape hatch.
 */
export function ActivityQueueSeedingProgress({
  queueItem,
  className,
  t,
}: {
  queueItem: ActivityQueueSeedingProgressItem;
  className?: string;
  t: TranslateFn;
}) {
  const seeding = deriveSeedingProgress(queueItem);
  const isPrivate = isPrivateTorrentRow(queueItem);

  if (!seeding && !isPrivate) {
    return null;
  }

  return (
    <div
      data-ui="queue-seeding"
      // Omitted rather than emptied: an unknown private flag must not be
      // selectable as if it had been answered.
      data-seeding-state={seeding?.stateKey}
      data-seeding-private={isPrivate ? "true" : undefined}
      className={cn(
        "flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground",
        className,
      )}
    >
      {isPrivate ? (
        <ActionTooltip
          content={t("queue.seeding.privateTooltip")}
          wrapperTabIndex={0}
        >
          <span
            data-ui="queue-seeding-private"
            className="inline-flex items-center text-[var(--scry-accent-text)]"
            aria-label={t("queue.seeding.private")}
          >
            <Lock className="h-3 w-3" aria-hidden="true" />
          </span>
        </ActionTooltip>
      ) : null}
      {seeding ? (
        <span data-ui="queue-seeding-state" className={seeding.toneClass}>
          {t(seeding.labelKey)}
        </span>
      ) : null}
      {seeding?.ratioLabel ? (
        <span data-ui="queue-seeding-ratio">
          {t("queue.seeding.ratio", { value: seeding.ratioLabel })}
        </span>
      ) : null}
      {seeding?.seedTimeLabel ? (
        <span data-ui="queue-seeding-time">
          {t("queue.seeding.seedTime", { value: seeding.seedTimeLabel })}
        </span>
      ) : null}
    </div>
  );
}

export function ActivityQueueDetailsPanel({
  detailId,
  releaseTitle,
  errorCode,
  failureReason,
  t,
}: {
  detailId: string;
  releaseTitle: string;
  errorCode?: string | null;
  failureReason: string;
  t: TranslateFn;
}) {
  return (
    <div
      id={detailId}
      className="rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3"
    >
      <div className="grid gap-4 md:grid-cols-2">
        <div>
          <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
            {t("queue.releaseTitle")}
          </p>
          <p className="mt-1 break-words text-sm text-foreground">{releaseTitle}</p>
        </div>
        <div>
          {errorCode ? (
            <>
              <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                {t("queue.errorCode")}
              </p>
              <p className="mt-1 break-words text-sm font-[var(--font-code)] text-foreground">{errorCode}</p>
            </>
          ) : null}
        </div>
      </div>
      {failureReason ? <div className="mt-4">
        <div>
          <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
            {t("queue.blockReason")}
          </p>
          <p className="mt-1 whitespace-pre-wrap break-words text-sm text-foreground">
            {failureReason}
          </p>
        </div>
      </div> : null}
    </div>
  );
}
