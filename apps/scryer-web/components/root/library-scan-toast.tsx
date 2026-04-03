import { CheckCircle2, CircleAlert, Loader2 } from "lucide-react";

import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import type { Translate } from "@/components/root/types";
import type { LibraryScanPhaseProgress, LibraryScanProgress } from "@/lib/types";

function facetLabel(facet: LibraryScanProgress["facet"], t: Translate): string {
  switch (facet) {
    case "movie":
      return t("nav.movies");
    case "tv":
      return t("nav.series");
    case "anime":
      return t("nav.anime");
    default:
      return t("settings.libraryScanTitle");
  }
}

function isTerminal(status: LibraryScanProgress["status"]): boolean {
  return status === "completed" || status === "warning" || status === "failed";
}

function percentForPhase(
  phase: LibraryScanPhaseProgress,
  totalKnown: boolean,
  terminal: boolean,
): number {
  if (phase.total <= 0) {
    return terminal || totalKnown ? 100 : 0;
  }
  const done = phase.completed + phase.failed;
  return Math.max(0, Math.min(100, Math.round((done / phase.total) * 100)));
}

function phaseLabel(
  status: LibraryScanProgress["status"],
  phase: LibraryScanPhaseProgress,
  totalKnown: boolean,
  emptyLabel: string,
  t: Translate,
): string {
  if (!totalKnown && !isTerminal(status)) {
    return t("settings.libraryScanProgressCalculatingTotal");
  }
  if (phase.total <= 0) {
    return isTerminal(status) || totalKnown
      ? emptyLabel
      : t("settings.libraryScanProgressPending");
  }
  const done = Math.min(phase.total, phase.completed + phase.failed);
  return t("settings.libraryScanProgressCount", {
    current: done,
    total: phase.total,
  });
}

function statusIcon(status: LibraryScanProgress["status"]) {
  if (status === "failed") {
    return <CircleAlert className="h-4 w-4 text-red-400" />;
  }
  if (status === "completed" || status === "warning") {
    return <CheckCircle2 className="h-4 w-4 text-emerald-400" />;
  }
  return <Loader2 className="h-4 w-4 animate-spin text-sky-400" />;
}

export function LibraryScanToast({
  session,
  t,
}: {
  session: LibraryScanProgress;
  t: Translate;
}) {
  const terminal = isTerminal(session.status);
  const metadataPercent = percentForPhase(
    session.metadataProgress,
    session.metadataTotalKnown,
    terminal,
  );
  const filePercent = percentForPhase(
    session.fileProgress,
    session.fileTotalKnown,
    terminal,
  );
  const metadataIndeterminate = !terminal && !session.metadataTotalKnown;
  const fileIndeterminate = !terminal && !session.fileTotalKnown;

  return (
    <div className="w-[min(26rem,calc(100vw-3rem))] p-4">
      <div className="min-w-0 space-y-3">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <p className="text-sm font-semibold text-foreground">
              {t("settings.libraryScanToastTitle", {
                facet: facetLabel(session.facet, t),
              })}
            </p>
            {statusIcon(session.status)}
          </div>
          <p className="text-xs text-muted-foreground">
            {session.foundTitles > 0 || terminal
              ? t("settings.libraryScanFoundTitles", {
                  count: session.foundTitles,
                })
              : t("settings.libraryScanDiscovering")}
          </p>
        </div>

        <div className="space-y-3">
          <div className="space-y-1">
            <p className="text-xs font-medium text-foreground">
              {t("settings.libraryScanFetchingMetadata")}
            </p>
            <ActivityProgressBar
              percent={metadataPercent}
              indeterminate={metadataIndeterminate}
              remainingLabel={phaseLabel(
                session.status,
                session.metadataProgress,
                session.metadataTotalKnown,
                t("settings.libraryScanNoMetadataNeeded"),
                t,
              )}
              colorClass="bg-sky-500"
            />
          </div>

          <div className="space-y-1">
            <p className="text-xs font-medium text-foreground">
              {t("settings.libraryScanFilesScanned")}
            </p>
            <ActivityProgressBar
              percent={filePercent}
              indeterminate={fileIndeterminate}
              remainingLabel={phaseLabel(
                session.status,
                session.fileProgress,
                session.fileTotalKnown,
                t("settings.libraryScanNoFilesToScan"),
                t,
              )}
              colorClass="bg-emerald-500"
            />
          </div>
        </div>

        {terminal ? (
          <p className="text-xs text-muted-foreground">
            {session.status === "failed"
              ? t("settings.libraryScanFailed")
              : session.summary
                ? t("settings.libraryScanSummary", {
                    imported: session.summary.imported,
                    skipped: session.summary.skipped,
                    unmatched: session.summary.unmatched,
                  })
                : t("settings.libraryScanCompleted")}
          </p>
        ) : null}
      </div>
    </div>
  );
}
