
import * as React from "react";
import { ChevronDown, ChevronUp, ArrowDown, ArrowUp, Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import type { Release } from "@/lib/types";

type SortKey = "score" | "size";
type SortDirection = "asc" | "desc";

function getScoreText(score: number | undefined) {
  if (score == null) {
    return "—";
  }
  return score > 0 ? `+${score}` : `${score}`;
}

function bytesToWholeReadable(raw: number | null | undefined) {
  if (!raw || raw <= 0) {
    return "—";
  }
  const gb = 1024 * 1024 * 1024;
  const mb = 1024 * 1024;
  const kb = 1024;

  if (raw > gb) {
    return `${Math.floor(raw / gb)} GB`;
  }
  if (raw > mb) {
    return `${Math.floor(raw / mb)} MB`;
  }
  if (raw > kb) {
    return `${Math.floor(raw / kb)} KB`;
  }
  return `${Math.floor(raw)} B`;
}

function getSortableValue(a: Release, key: SortKey): number {
  if (key === "score") {
    return a.qualityProfileDecision?.releaseScore ?? Number.NEGATIVE_INFINITY;
  }
  return a.sizeBytes ?? 0;
}

function sortBy(
  releaseList: Release[],
  sortKey: SortKey,
  sortDirection: SortDirection,
): Release[] {
  const factor = sortDirection === "asc" ? 1 : -1;

  return [...releaseList].sort((left, right) => {
    const leftValue = getSortableValue(left, sortKey);
    const rightValue = getSortableValue(right, sortKey);
    const delta = (leftValue - rightValue) * factor;
    if (delta !== 0) {
      return delta;
    }
    return left.title.localeCompare(right.title);
  });
}

function SearchResultRow({
  result,
  onQueue,
  blocked,
}: {
  result: Release;
  onQueue: (r: Release) => Promise<void> | void;
  blocked: boolean;
}) {
  const t = useTranslate();
  const [expanded, setExpanded] = React.useState(false);
  const [queueRequested, setQueueRequested] = React.useState(false);
  const decision = result.qualityProfileDecision;
  const hasLog = decision && decision.scoringLog.length > 0;
  const parsedBits = [
    result.parsedRelease?.quality,
    result.parsedRelease?.videoCodec,
    result.parsedRelease?.videoEncoding,
    result.parsedRelease?.audio,
  ]
    .filter((value) => value)
    .filter((value) => typeof value === "string" && value.trim().length > 0);
  const parsedMetadata = result.parsedRelease
    ? [
        result.parsedRelease.detectedHdr
          ? { label: "HDR", className: "bg-cyan-500/20 text-cyan-300" }
          : null,
        result.parsedRelease.isDolbyVision
          ? { label: "Dolby Vision", className: "bg-indigo-500/20 text-indigo-300" }
          : null,
        result.parsedRelease.isProperUpload
          ? { label: "Proper", className: "bg-amber-500/20 text-amber-300" }
          : null,
        result.parsedRelease.isRemux
          ? { label: "Remux", className: "bg-violet-500/20 text-violet-300" }
          : null,
        result.parsedRelease.isBdDisk
          ? { label: "BD", className: "bg-rose-500/20 text-rose-300" }
          : null,
        result.parsedRelease.isAiEnhanced
          ? { label: "AI Enhanced", className: "bg-red-500/20 text-red-300" }
          : null,
        result.parsedRelease.isAtmos
          ? { label: "Atmos", className: "bg-purple-500/20 text-purple-300" }
          : null,
      ]
        .filter(Boolean)
        .filter((value) => value !== null && value !== undefined && typeof value === "object")
        .map((entry) => entry as { label: string; className: string })
    : [];

  const handleQueueClick = React.useCallback(() => {
    if (blocked || queueRequested) {
      return;
    }

    setQueueRequested(true);

    try {
      const maybePromise = onQueue(result);
      if (maybePromise && typeof (maybePromise as Promise<void>).then === "function") {
        void (maybePromise as Promise<void>).catch(() => {
          setQueueRequested(false);
        });
      }
    } catch {
      setQueueRequested(false);
    }
  }, [blocked, onQueue, queueRequested, result]);

  return (
    <>
      <tr>
        <td className="rounded-l-lg border border-border border-r-0 px-4 py-2 align-middle">
          <div className="space-y-1">
            <p className="min-w-0 whitespace-normal break-words text-base font-semibold text-foreground">{result.title}</p>
            <p className="text-xs text-muted-foreground">
              {result.source ?? t("label.unknown")} • {result.publishedAt ?? t("label.unknown")}
            </p>
            {parsedBits.length > 0 ? (
              <div className="mt-1 flex flex-wrap gap-1.5">
                {parsedBits.map((metadataBit) => (
                  <span
                    key={metadataBit}
                    className="inline-flex items-center rounded-full border border-border bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground"
                  >
                    {metadataBit}
                  </span>
                ))}
              </div>
            ) : null}
            {parsedMetadata.length > 0 ? (
              <div className="mt-1 flex flex-wrap gap-1.5">
                {parsedMetadata.map((metadataBit) => (
                  <span
                    key={metadataBit.label}
                    className={`inline-flex items-center rounded-full border border-transparent px-2 py-0.5 text-[11px] font-medium ${metadataBit.className}`}
                  >
                    {metadataBit.label}
                  </span>
                ))}
              </div>
            ) : null}
            {decision && decision.blockCodes.length > 0 ? (
              <p className="mt-1 text-xs text-red-400">{decision.blockCodes.join(" · ")}</p>
            ) : null}
          </div>
        </td>
        <td className="border border-border border-x-0 px-4 py-2 text-center align-middle">
          {decision ? (
            hasLog ? (
              <button
                type="button"
                className={`text-sm font-mono underline-offset-2 hover:underline ${decision.releaseScore < 0 ? "text-red-400" : "text-emerald-700 dark:text-emerald-300"}`}
                onClick={() => setExpanded((prev) => !prev)}
                aria-label={expanded ? t("nzb.hideScoringLog") : t("nzb.showScoringLog")}
              >
                {getScoreText(decision.releaseScore)}
              </button>
            ) : (
              <span className={`text-sm font-mono ${decision.releaseScore < 0 ? "text-red-400" : "text-emerald-700 dark:text-emerald-300"}`}>{getScoreText(decision.releaseScore)}</span>
            )
          ) : (
            <span className="text-sm font-mono text-muted-foreground">{getScoreText(undefined)}</span>
          )}
        </td>
        <td className="border border-border border-x-0 px-4 py-2 text-center text-xl font-semibold text-foreground align-middle">
          {bytesToWholeReadable(result.sizeBytes)}
        </td>
        <td className="rounded-r-lg border border-border border-l-0 px-4 py-2 text-center align-middle">
          <Button
            size="default"
            onClick={handleQueueClick}
            disabled={blocked || queueRequested}
            className={
              blocked
                ? "h-10"
                : queueRequested
                  ? "h-10 border border-emerald-500/50 dark:border-emerald-300/70 bg-emerald-200 dark:bg-emerald-900/80 text-emerald-800 dark:text-emerald-100"
                  : "h-10 bg-emerald-600 text-foreground hover:bg-emerald-500 focus-visible:ring-emerald-300/70 border border-emerald-500/60 dark:border-emerald-400/50"
            }
            variant={blocked ? "ghost" : "default"}
          >
            {queueRequested ? (
              <span className="inline-flex items-center gap-1.5">
                <Check className="h-3.5 w-3.5" />
                {t("queue.state.queued")}
              </span>
            ) : (
              t("nzb.queue")
            )}
          </Button>
        </td>
      </tr>
      {expanded && hasLog ? (
        <tr>
          <td colSpan={4} className="border border-x border-t-0 border-border p-0">
            <div className="bg-background/80 px-3 py-2">
              <p className="mb-1 text-xs font-semibold text-muted-foreground">{t("nzb.scoringLog")}</p>
              <div className="space-y-0.5">
                {decision.scoringLog.map((entry) => (
                  <div key={entry.code} className="flex justify-between gap-4 font-mono text-xs">
                    <span className="text-muted-foreground">{entry.code}</span>
                    <span className={entry.delta < 0 ? "text-red-400" : "text-emerald-600 dark:text-emerald-400"}>
                      {entry.delta > 0 ? "+" : ""}
                      {entry.delta}
                    </span>
                  </div>
                ))}
              </div>
              <div className="mt-1.5 flex justify-between border-t border-border pt-1.5 font-mono text-xs font-semibold">
                <span className="text-muted-foreground">{t("nzb.total")}</span>
                <span className={decision.releaseScore < 0 ? "text-red-400" : "text-emerald-600 dark:text-emerald-400"}>
                  {getScoreText(decision.releaseScore)}
                </span>
              </div>
            </div>
          </td>
        </tr>
      ) : null}
    </>
  );
}

export function SearchResultBuckets({
  results,
  onQueue,
}: {
  results: Release[];
  onQueue: (r: Release) => Promise<void> | void;
}) {
  const t = useTranslate();
  const considered = React.useMemo(
    () => results.filter((r) => !r.qualityProfileDecision || r.qualityProfileDecision.allowed),
    [results],
  );
  const blocked = React.useMemo(
    () => results.filter((r) => r.qualityProfileDecision && !r.qualityProfileDecision.allowed),
    [results],
  );
  const [showBlocked, setShowBlocked] = React.useState(false);
  const [sortKey, setSortKey] = React.useState<SortKey>("score");
  const [sortDirection, setSortDirection] = React.useState<SortDirection>("desc");

  const sortedConsidered = React.useMemo(
    () => sortBy(considered, sortKey, sortDirection),
    [considered, sortDirection, sortKey],
  );
  const sortedBlocked = React.useMemo(
    () => sortBy(blocked, sortKey, sortDirection),
    [blocked, sortDirection, sortKey],
  );

  const handleSort = React.useCallback(
    (next: SortKey) => {
      if (sortKey === next) {
        setSortDirection((previous) => (previous === "asc" ? "desc" : "asc"));
        return;
      }

      setSortKey(next);
      setSortDirection("desc");
    },
    [sortKey],
  );

  const renderSortIcon = React.useCallback(
    (key: SortKey) => {
      if (sortKey !== key) {
        return <ChevronDown className="h-3 w-3 opacity-30" />;
      }
      return sortDirection === "desc" ? <ArrowDown className="h-3 w-3" /> : <ArrowUp className="h-3 w-3" />;
    },
    [sortDirection, sortKey],
  );

  const renderTable = React.useCallback(
    (entries: Release[], isBlocked: boolean) => {
      return (
        <div className="overflow-x-auto rounded-md border border-border bg-background/30">
          <table className="w-full table-fixed text-left">
            <thead className="bg-card/80">
              <tr>
                <th className="w-[68%] px-4 py-3 text-base font-bold text-foreground">Release</th>
                <th className="w-[8%] px-4 py-3 text-center text-base font-bold text-foreground">
                  <button
                    type="button"
                    className="inline-flex w-full items-center justify-center gap-1"
                    onClick={() => handleSort("score")}
                  >
                    Score {renderSortIcon("score")}
                  </button>
                </th>
                <th className="w-[10%] px-4 py-3 text-center text-base font-bold text-foreground">
                  <button
                    type="button"
                    className="inline-flex w-full items-center justify-center gap-1"
                    onClick={() => handleSort("size")}
                  >
                    Size {renderSortIcon("size")}
                  </button>
                </th>
                <th className="w-[14%] px-4 py-3 text-center text-base font-bold text-foreground">Actions</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((result) => (
                <SearchResultRow
                  key={`${result.source}-${result.title}-${result.link}`}
                  result={result}
                  onQueue={onQueue}
                  blocked={isBlocked}
                />
              ))}
            </tbody>
          </table>
        </div>
      );
    },
    [handleSort, onQueue, renderSortIcon],
  );

  return (
    <div className="space-y-3">
      {considered.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("nzb.noConsideredResults")}</p>
      ) : (
        renderTable(sortedConsidered, false)
      )}
      {blocked.length > 0 ? (
        <div>
          <button
            type="button"
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-card-foreground"
            onClick={() => setShowBlocked((v) => !v)}
          >
            {showBlocked ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
            {t("nzb.blockedResults", { count: blocked.length })}
          </button>
          {showBlocked ? <div className="mt-2 space-y-2">{renderTable(sortedBlocked, true)}</div> : null}
        </div>
      ) : null}
    </div>
  );
}
