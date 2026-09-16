import * as React from "react";
import {
  ArrowUpRight,
  Check,
  CircleAlert,
  CircleCheck,
  CircleDashed,
  ListChecks,
  LoaderCircle,
  PictureInPicture2,
  Sparkles,
  Timer,
  TriangleAlert,
  X,
  type LucideIcon,
} from "lucide-react";
import { useClient, useQuery } from "urql";

import { toast } from "@/components/ui/sonner";
import type { Translate } from "@/components/root/types";
import { cancelLibraryScanMutation } from "@/lib/graphql/mutations";
import { librariesQuery } from "@/lib/graphql/queries";
import { facetById } from "@/lib/facets/registry";
import type { Facet } from "@/lib/types/titles";
import type { LibraryScanPhaseProgress, LibraryScanProgress } from "@/lib/types";
import { cn } from "@/lib/utils";
import { LoadingMark } from "@/components/common/loading-mark";

type FacetConfig = {
  Icon: LucideIcon;
  /** rgb triple for translucent chip/glow backgrounds */
  rgb: string;
  /** solid base color (active spinner + count) */
  base: string;
  /** gradient for the accent rail + active bar fill */
  grad: string;
};

// State colors are literal hex (not yet in the core token set) per the design
// handoff — promote to --scry-success / --scry-warning / --scry-danger-soft later.
function navIconForFacet(facet: Facet): LucideIcon {
  return facetById(facet)?.icon ?? ListChecks;
}

const FACET_CONFIG: Record<Facet, FacetConfig> = {
  MOVIE: {
    Icon: navIconForFacet("MOVIE"),
    rgb: "var(--scry-facet-movie-rgb)",
    base: "var(--scry-facet-movie)",
    grad: "var(--scry-facet-movie-grad)",
  },
  SERIES: {
    Icon: navIconForFacet("SERIES"),
    rgb: "var(--scry-facet-series-rgb)",
    base: "var(--scry-facet-series)",
    grad: "var(--scry-facet-series-grad)",
  },
  ANIME: {
    Icon: navIconForFacet("ANIME"),
    rgb: "var(--scry-facet-anime-rgb)",
    base: "var(--scry-facet-anime)",
    grad: "var(--scry-facet-anime-grad)",
  },
};

const DEFAULT_AUTO_DISMISS_MS = 5_000;

type ToastVisualState =
  | "scanning"
  | "success"
  | "issues"
  | "failed"
  | "canceled";
type PhaseStatus = "done" | "active" | "pending";

function facetLabel(facet: Facet, t: Translate): string {
  switch (facet) {
    case "MOVIE":
      return t("nav.movies");
    case "SERIES":
      return t("nav.series");
    case "ANIME":
      return t("nav.anime");
    default:
      return t("settings.libraryScanTitle");
  }
}

function isTerminal(status: LibraryScanProgress["status"]): boolean {
  return (
    status === "COMPLETED" ||
    status === "CANCELED" ||
    status === "WARNING" ||
    status === "FAILED"
  );
}

function phaseDone(phase: LibraryScanPhaseProgress): number {
  return phase.completed + phase.failed;
}

function percentForPhase(
  phase: LibraryScanPhaseProgress,
  totalKnown: boolean,
  terminal: boolean,
): number {
  if (!totalKnown && !terminal) {
    return 0;
  }
  if (phase.total <= 0) {
    return terminal || totalKnown ? 100 : 0;
  }
  return Math.max(
    0,
    Math.min(100, Math.round((phaseDone(phase) / phase.total) * 100)),
  );
}

function phaseComplete(
  phase: LibraryScanPhaseProgress,
  totalKnown: boolean,
  terminal: boolean,
): boolean {
  if (terminal) {
    return true;
  }
  if (!totalKnown) {
    return false;
  }
  return phase.total === 0 || phaseDone(phase) >= phase.total;
}

function phaseCountLabel(
  phase: LibraryScanPhaseProgress,
  totalKnown: boolean,
  terminal: boolean,
  emptyLabel: string,
  t: Translate,
): string {
  if (!totalKnown && !terminal) {
    const done = phaseDone(phase);
    if (done > 0) {
      return t("settings.libraryScanProgressScannedCount", {
        count: done.toLocaleString(),
      });
    }
    return t("settings.libraryScanProgressCalculatingTotal");
  }
  if (phase.total <= 0) {
    return terminal || totalKnown
      ? emptyLabel
      : t("settings.libraryScanProgressPending");
  }
  const done = Math.min(phase.total, phaseDone(phase));
  return t("settings.libraryScanProgressCount", {
    current: done.toLocaleString(),
    total: phase.total.toLocaleString(),
  });
}

function scanSummaryText(
  summary: LibraryScanProgress["summary"],
  t: Translate,
  canceled = false,
): string | null {
  if (!summary) {
    return null;
  }
  return t(
    canceled
      ? "settings.libraryScanCanceledSummary"
      : "settings.libraryScanSummary",
    {
      imported: summary.imported,
      skipped: summary.skipped,
      unmatched: summary.unmatched,
    },
  );
}

function formatEtaCountdown(totalSeconds: number): string {
  const seconds = Math.max(1, Math.round(totalSeconds));
  const minutes = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${String(minutes).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
}

type EtaSample = {
  atMs: number;
  completed: number;
};

const ETA_HISTORY_WINDOW_MS = 4 * 60 * 1000;
const ETA_MIN_HISTORY_MS = 45 * 1000;
const ETA_MIN_COMPLETED_DELTA = 2;
const ETA_SMOOTHING_ALPHA = 0.18;

function estimateEtaSecondsFromHistory({
  samples,
  nowMs,
  completed,
  remaining,
  fallbackElapsedMs,
}: {
  samples: EtaSample[];
  nowMs: number;
  completed: number;
  remaining: number;
  fallbackElapsedMs: number;
}): number | null {
  if (completed <= 0 || remaining <= 0) {
    return null;
  }

  const cutoffMs = nowMs - ETA_HISTORY_WINDOW_MS;
  const baseline =
    samples.find(
      (sample) => sample.atMs >= cutoffMs && sample.completed < completed,
    ) ?? samples.find((sample) => sample.completed < completed);

  if (baseline) {
    const elapsedSeconds = Math.max(1, (nowMs - baseline.atMs) / 1000);
    const completedDelta = completed - baseline.completed;
    if (
      elapsedSeconds * 1000 >= ETA_MIN_HISTORY_MS &&
      completedDelta >= ETA_MIN_COMPLETED_DELTA
    ) {
      return remaining / (completedDelta / elapsedSeconds);
    }
  }

  if (fallbackElapsedMs < ETA_MIN_HISTORY_MS) {
    return null;
  }

  return remaining / (completed / Math.max(1, fallbackElapsedMs / 1000));
}

function ScanPhaseBar({
  facet,
  label,
  status,
  count,
  percent,
  indeterminate,
}: {
  facet: FacetConfig;
  label: string;
  status: PhaseStatus;
  count: string;
  percent: number;
  indeterminate: boolean;
}) {
  let Icon: LucideIcon;
  let iconColor: string;
  let spin = false;
  let fillStyle: React.CSSProperties;
  let labelColor: string;
  let countColor: string;

  if (status === "done") {
    Icon = CircleCheck;
    iconColor = "var(--scry-success-text-soft)";
    fillStyle = {
      width: `${percent}%`,
      background: "var(--scry-success-bg-strong)",
    };
    labelColor = "var(--scry-text2)";
    countColor = "var(--scry-muted2)";
  } else if (status === "active") {
    Icon = LoaderCircle;
    iconColor = facet.base;
    spin = true;
    fillStyle = {
      width: `${percent}%`,
      background: facet.grad,
      boxShadow: `0 0 12px rgba(${facet.rgb},.5)`,
    };
    labelColor = "var(--scry-ink2)";
    countColor = facet.base;
  } else {
    Icon = CircleDashed;
    iconColor = "var(--scry-faint4)";
    fillStyle = { width: "0%", background: "transparent" };
    labelColor = "var(--scry-faint3)";
    countColor = "var(--scry-faint4)";
  }

  return (
    <div className="mb-[11px] last:mb-0">
      <div className="mb-1.5 flex items-center gap-2">
        {spin ? (
          <LoadingMark className="h-3.5 w-3.5 shrink-0" />
        ) : (
          <Icon
            className="h-3.5 w-3.5 shrink-0"
            style={{ color: iconColor }}
            aria-hidden="true"
          />
        )}
        <span className="text-xs font-semibold" style={{ color: labelColor }}>
          {label}
        </span>
        <span
          className="ml-auto text-[11.5px] font-semibold tabular-nums"
          style={{ color: countColor }}
        >
          {count}
        </span>
      </div>
      <div className="relative h-[5px] overflow-hidden rounded-full bg-white/[0.07]">
        <div
          className="absolute left-0 top-0 h-full rounded-full transition-[width] duration-500 ease-out"
          style={fillStyle}
        />
        {status === "active" && indeterminate ? (
          <div
            className="absolute left-0 top-0 h-full w-2/5"
            style={{
              background:
                "linear-gradient(90deg,transparent,rgba(255,255,255,.45),transparent)",
              animation: "scry-shimmer 1.4s ease-in-out infinite",
            }}
            aria-hidden="true"
          />
        ) : null}
      </div>
    </div>
  );
}

export function LibraryScanToast({
  session,
  t,
  onRunInBackground,
  onDismiss,
  onViewTitles,
  onReviewUnmatched,
  autoDismissMs = DEFAULT_AUTO_DISMISS_MS,
}: {
  session: LibraryScanProgress;
  t: Translate;
  onRunInBackground?: () => void;
  onDismiss?: () => void;
  onViewTitles?: () => void;
  onReviewUnmatched?: () => void;
  autoDismissMs?: number;
}) {
  const client = useClient();
  const [{ data: libraryData }] = useQuery<{ libraries: { id: string; name: string }[] }>({
    query: librariesQuery,
    variables: { facet: session.facet },
    requestPolicy: "cache-and-network",
    pause: !session.libraryId,
  });
  const libraryName = libraryData?.libraries.find((library) => library.id === session.libraryId)?.name;
  const facet = FACET_CONFIG[session.facet] ?? FACET_CONFIG.MOVIE;
  const FacetIcon = facet.Icon;
  const terminal = isTerminal(session.status);
  const unmatched = session.summary?.unmatched ?? 0;

  const visualState: ToastVisualState = !terminal
    ? "scanning"
    : session.status === "FAILED"
      ? "failed"
      : session.status === "CANCELED"
        ? "canceled"
        // The Import review route resolves unmatched files; a non-blocking
        // scan warning with nothing to resolve must still complete normally.
        : unmatched > 0
          ? "issues"
          : "success";

  const showCancel = session.mode === "FULL" && !terminal;
  const autoDismiss =
    !!onDismiss && (visualState === "success" || visualState === "canceled");
  const countdownColor =
    visualState === "success"
      ? "rgba(var(--scry-success-rgb),.2)"
      : "rgba(255,255,255,.12)";

  const [cancelPending, setCancelPending] = React.useState(false);
  const [nowMs, setNowMs] = React.useState(() => Date.now());
  const [paused, setPaused] = React.useState(false);
  const [smoothedEtaSeconds, setSmoothedEtaSeconds] = React.useState<
    number | null
  >(null);
  const [mediaAnalysisStartedAtMs, setMediaAnalysisStartedAtMs] =
    React.useState<number | null>(null);
  const mediaAnalysisStartedAtRef = React.useRef<number | null>(null);
  const etaSamplesRef = React.useRef<EtaSample[]>([]);
  const etaSmoothingAtRef = React.useRef<number | null>(null);

  const mediaAnalysisDone =
    session.mediaAnalysisProgress.completed +
    session.mediaAnalysisProgress.failed;
  const mediaAnalysisTotal = session.mediaAnalysisProgress.total;
  const mediaAnalysisRemaining = Math.max(
    0,
    mediaAnalysisTotal - mediaAnalysisDone,
  );
  const mediaAnalysisActive =
    !terminal && mediaAnalysisTotal > 0 && mediaAnalysisRemaining > 0;

  // Refresh the clock once a second while media analysis runs so the ETA
  // estimate keeps recomputing.
  React.useEffect(() => {
    if (!mediaAnalysisActive) {
      return;
    }
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [mediaAnalysisActive]);

  // Success/canceled auto-dismiss. The remaining time is tracked across pauses
  // so the JS timer stays in sync with the CSS countdown fill (both pause when
  // the toast is hovered).
  const remainingRef = React.useRef(autoDismissMs);
  React.useEffect(() => {
    if (!autoDismiss || paused) {
      return;
    }
    const startedAt = Date.now();
    const timer = window.setTimeout(() => {
      onDismiss?.();
    }, remainingRef.current);
    return () => {
      window.clearTimeout(timer);
      remainingRef.current = Math.max(
        0,
        remainingRef.current - (Date.now() - startedAt),
      );
    };
  }, [autoDismiss, paused, onDismiss]);

  const handleCancel = React.useCallback(async () => {
    if (cancelPending) {
      return;
    }
    setCancelPending(true);
    try {
      const result = await client
        .mutation(cancelLibraryScanMutation, { sessionId: session.sessionId })
        .toPromise();
      if (result.error) {
        throw result.error;
      }
    } catch (error) {
      setCancelPending(false);
      toast.error(
        error instanceof Error
          ? error.message
          : t("settings.libraryScanCancelFailed"),
      );
    }
  }, [cancelPending, client, session.sessionId, t]);

  // --- Media-analysis ETA estimate (smoothed, from recent throughput) ---
  React.useEffect(() => {
    if (!mediaAnalysisActive) {
      mediaAnalysisStartedAtRef.current = null;
      setMediaAnalysisStartedAtMs(null);
      etaSamplesRef.current = [];
      etaSmoothingAtRef.current = null;
      setSmoothedEtaSeconds(null);
      return;
    }
    if (mediaAnalysisStartedAtRef.current == null) {
      mediaAnalysisStartedAtRef.current =
        Date.parse(session.updatedAt) || Date.now();
    }
    setMediaAnalysisStartedAtMs(mediaAnalysisStartedAtRef.current);
  }, [mediaAnalysisActive, session.sessionId, session.updatedAt]);

  React.useEffect(() => {
    if (!mediaAnalysisActive) {
      return;
    }

    const sampleAt = Math.max(
      mediaAnalysisStartedAtRef.current ?? nowMs,
      Date.parse(session.updatedAt) || nowMs,
    );
    const samples = etaSamplesRef.current;
    const last = samples.at(-1);

    if (!last) {
      samples.push({
        atMs: mediaAnalysisStartedAtRef.current ?? sampleAt,
        completed: 0,
      });
    } else if (mediaAnalysisDone < last.completed) {
      etaSamplesRef.current = [
        {
          atMs: mediaAnalysisStartedAtRef.current ?? sampleAt,
          completed: 0,
        },
      ];
    }

    const currentSamples = etaSamplesRef.current;
    const currentLast = currentSamples.at(-1);
    if (currentLast?.completed !== mediaAnalysisDone) {
      currentSamples.push({
        atMs: Math.max(sampleAt, (currentLast?.atMs ?? 0) + 1),
        completed: mediaAnalysisDone,
      });
    }

    const firstRecentIndex = currentSamples.findIndex(
      (sample) => sample.atMs >= nowMs - ETA_HISTORY_WINDOW_MS,
    );
    etaSamplesRef.current =
      firstRecentIndex <= 0
        ? currentSamples
        : currentSamples.slice(firstRecentIndex - 1);
  }, [
    mediaAnalysisActive,
    mediaAnalysisDone,
    nowMs,
    session.sessionId,
    session.updatedAt,
  ]);

  const mediaAnalysisElapsedMs =
    mediaAnalysisStartedAtMs == null
      ? 0
      : Math.max(0, nowMs - mediaAnalysisStartedAtMs);
  const shouldShowEta =
    showCancel &&
    mediaAnalysisActive &&
    session.mediaAnalysisTotalKnown &&
    mediaAnalysisDone > 0 &&
    mediaAnalysisElapsedMs >= 30_000;

  React.useEffect(() => {
    if (!shouldShowEta) {
      etaSmoothingAtRef.current = null;
      setSmoothedEtaSeconds(null);
      return;
    }

    const estimate = estimateEtaSecondsFromHistory({
      samples: etaSamplesRef.current,
      nowMs,
      completed: mediaAnalysisDone,
      remaining: mediaAnalysisRemaining,
      fallbackElapsedMs: mediaAnalysisElapsedMs,
    });

    if (estimate == null || !Number.isFinite(estimate)) {
      etaSmoothingAtRef.current = null;
      setSmoothedEtaSeconds(null);
      return;
    }

    setSmoothedEtaSeconds((current) => {
      const previousAt = etaSmoothingAtRef.current;
      etaSmoothingAtRef.current = nowMs;
      if (current == null || previousAt == null) {
        return estimate;
      }

      const elapsedSeconds = Math.max(0, (nowMs - previousAt) / 1000);
      const expectedCountdown = Math.max(1, current - elapsedSeconds);
      return (
        expectedCountdown + (estimate - expectedCountdown) * ETA_SMOOTHING_ALPHA
      );
    });
  }, [
    mediaAnalysisDone,
    mediaAnalysisElapsedMs,
    mediaAnalysisRemaining,
    nowMs,
    shouldShowEta,
  ]);

  const titleMatchStatus: PhaseStatus = phaseComplete(
    session.titleMatchProgress,
    session.titleMatchTotalKnown,
    terminal,
  )
    ? "done"
    : "active";
  const mediaStatus: PhaseStatus = phaseComplete(
    session.mediaAnalysisProgress,
    session.mediaAnalysisTotalKnown,
    terminal,
  )
    ? "done"
    : mediaAnalysisDone > 0 || mediaAnalysisTotal > 0 || titleMatchStatus === "done"
      ? "active"
      : "pending";

  const titleText = t("settings.libraryScanToastTitle", {
    facet: libraryName || facetLabel(session.facet, t),
  });
  const subtitle =
    visualState === "scanning"
      ? session.foundTitles > 0
        ? t("settings.libraryScanScanningSubtitle", {
            count: session.foundTitles,
          })
        : t("settings.libraryScanDiscovering")
      : visualState === "success"
        ? t("settings.libraryScanDoneSubtitle", { count: session.foundTitles })
        : visualState === "issues"
          ? t("settings.libraryScanIssuesSubtitle", {
              count: session.foundTitles,
            })
          : visualState === "failed"
            ? t("settings.libraryScanFailedSubtitle")
            : t("settings.libraryScanCanceledSubtitle");

  const etaCountdown =
    smoothedEtaSeconds != null && Number.isFinite(smoothedEtaSeconds)
      ? formatEtaCountdown(smoothedEtaSeconds)
      : null;

  const accent =
    visualState === "scanning"
      ? facet.grad
      : visualState === "success"
        ? "linear-gradient(90deg,var(--scry-success-solid-hover),var(--scry-success-solid))"
        : visualState === "issues"
          ? "linear-gradient(90deg,var(--scry-warning-solid-hover),var(--scry-warning-solid))"
          : visualState === "failed"
            ? "linear-gradient(90deg,var(--scry-danger-solid-hover),var(--scry-danger-text-soft))"
            : "linear-gradient(90deg,#5b6478,#8b94a8)";

  type Badge = {
    label: string;
    Icon: LucideIcon;
    color: string;
    bg: string;
    border: string;
  };
  const badge: Badge | null =
    visualState === "success"
      ? {
          label: t("settings.libraryScanBadgeDone"),
          Icon: Check,
          color: "var(--scry-success-text-soft)",
          bg: "var(--scry-success-bg)",
          border: "var(--scry-success-border)",
        }
      : visualState === "issues"
        ? {
            label: t("settings.libraryScanBadgeReview"),
            Icon: TriangleAlert,
            color: "var(--scry-warning-text)",
            bg: "var(--scry-warning-bg)",
            border: "var(--scry-warning-border)",
          }
        : visualState === "failed"
          ? {
              label: t("settings.libraryScanBadgeFailed"),
              Icon: CircleAlert,
              color: "var(--scry-danger-text-soft)",
              bg: "var(--scry-danger-bg)",
              border: "var(--scry-danger-border)",
            }
          : visualState === "canceled"
            ? {
                label: t("settings.libraryScanBadgeCanceled"),
                Icon: X,
                color: "var(--scry-muted2)",
                bg: "var(--scry-chip)",
                border: "var(--scry-border2)",
              }
            : null;

  type Chip = {
    Icon: LucideIcon;
    color: string;
    bg: string;
    border: string;
    text: string;
  };
  let chip: Chip | null = null;
  if (visualState === "success") {
    const text = scanSummaryText(session.summary, t);
    if (text) {
      chip = {
        Icon: Sparkles,
        color: "var(--scry-success-text-soft)",
        bg: "var(--scry-success-bg)",
        border: "var(--scry-success-border)",
        text,
      };
    }
  } else if (visualState === "issues") {
    const text = scanSummaryText(session.summary, t);
    if (text) {
      chip = {
        Icon: CircleAlert,
        color: "var(--scry-warning-text)",
        bg: "var(--scry-warning-bg)",
        border: "var(--scry-warning-border)",
        text,
      };
    }
  } else if (visualState === "failed") {
    chip = {
      Icon: CircleAlert,
      color: "var(--scry-danger-text-soft)",
      bg: "var(--scry-danger-bg)",
      border: "var(--scry-danger-border)",
      text: session.warningMessage || t("settings.libraryScanFailed"),
    };
  }

  const renderDismiss = (fullWidth: boolean) =>
    autoDismiss ? (
      <button
        type="button"
        onClick={onDismiss}
        className={cn(
          "relative flex h-9 items-center justify-center gap-[7px] overflow-hidden rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[12.5px] font-semibold text-[var(--scry-text2)] transition hover:brightness-110",
          fullWidth ? "flex-1" : "w-28",
        )}
      >
        <span
          className="absolute inset-y-0 left-0 w-full origin-left"
          style={{
            background: countdownColor,
            animation: `scry-deplete ${autoDismissMs}ms linear forwards`,
            animationPlayState: paused ? "paused" : "running",
          }}
          aria-hidden="true"
        />
        <Timer className="relative z-[1] h-[13px] w-[13px]" aria-hidden="true" />
        <span className="relative z-[1]">{t("label.dismiss")}</span>
      </button>
    ) : (
      <button
        type="button"
        onClick={onDismiss}
        className={cn(
          "h-9 rounded-[9px] border border-[var(--scry-border2)] bg-transparent text-[12.5px] font-semibold text-[var(--scry-muted)] transition hover:text-[var(--scry-text2)]",
          fullWidth ? "flex-1" : "w-20",
        )}
      >
        {t("label.dismiss")}
      </button>
    );

  let footer: React.ReactNode = null;
  if (visualState === "scanning" && (onRunInBackground || showCancel)) {
    footer = (
      <div className="mt-[14px] flex items-center gap-[9px]">
        {onRunInBackground ? (
          <button
            type="button"
            onClick={onRunInBackground}
            disabled={cancelPending}
            className="flex h-9 flex-1 items-center justify-center gap-[7px] rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[12.5px] font-semibold text-[var(--scry-text2)] transition hover:brightness-110 disabled:opacity-60"
          >
            <PictureInPicture2 className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.libraryScanRunInBackground")}
          </button>
        ) : null}
        {showCancel ? (
          <button
            type="button"
            onClick={handleCancel}
            disabled={cancelPending}
            title={t("settings.libraryScanCancel")}
            className="flex h-9 w-[88px] items-center justify-center gap-1.5 rounded-[9px] text-[12.5px] font-semibold transition hover:brightness-110 disabled:opacity-60"
            style={{
              border: "1px solid var(--scry-danger-border)",
              background: "var(--scry-danger-bg)",
              color: "var(--scry-danger-text-soft)",
            }}
          >
            <X className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.libraryScanCancel")}
          </button>
        ) : null}
      </div>
    );
  } else if (visualState === "success") {
    footer = (
      <div className="mt-[14px] flex items-center gap-[9px]">
        <button
          type="button"
          onClick={onViewTitles}
          className="flex h-9 flex-1 items-center justify-center gap-[7px] rounded-[9px] text-[12.5px] font-semibold transition hover:brightness-110"
          style={{
            background: "rgba(var(--scry-accent-rgb),.16)",
            color: "var(--scry-accent-text)",
          }}
        >
          <ArrowUpRight className="h-3.5 w-3.5" aria-hidden="true" />
          {t("settings.libraryScanViewTitles", { count: session.foundTitles })}
        </button>
        {renderDismiss(false)}
      </div>
    );
  } else if (visualState === "issues") {
    footer = (
      <div className="mt-[14px] flex items-center gap-[9px]">
        {unmatched > 0 ? (
          <button
            type="button"
            onClick={onReviewUnmatched}
            className="flex h-9 flex-1 items-center justify-center gap-[7px] rounded-[9px] text-[12.5px] font-semibold transition hover:brightness-110"
            style={{
              border: "1px solid var(--scry-warning-border)",
              background: "var(--scry-warning-bg)",
              color: "var(--scry-warning-text)",
            }}
          >
            <ListChecks className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.libraryScanReviewUnmatched", { count: unmatched })}
          </button>
        ) : null}
        {renderDismiss(unmatched === 0)}
      </div>
    );
  } else if (visualState === "failed" || visualState === "canceled") {
    footer = (
      <div className="mt-[14px] flex items-center gap-[9px]">
        {renderDismiss(true)}
      </div>
    );
  }

  return (
    <div
      className="relative w-[392px] max-w-[calc(100vw-2rem)] overflow-hidden rounded-2xl border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_20px_44px_rgba(0,0,0,0.5)] [backdrop-filter:blur(14px)]"
      onMouseEnter={autoDismiss ? () => setPaused(true) : undefined}
      onMouseLeave={autoDismiss ? () => setPaused(false) : undefined}
    >
      <div
        className="absolute inset-y-0 left-0 w-[3px]"
        style={{ background: accent }}
        aria-hidden="true"
      />
      <div className="px-[18px] pb-[17px] pt-4">
        <div className="flex items-center gap-3">
          <div
            className="flex h-[38px] w-[38px] shrink-0 items-center justify-center rounded-[11px]"
            style={{
              background: `rgba(${facet.rgb},.14)`,
              border: `1px solid rgba(${facet.rgb},.32)`,
            }}
          >
            <FacetIcon
              className="h-[19px] w-[19px]"
              style={{ color: facet.base }}
              aria-hidden="true"
            />
          </div>
          <div className="min-w-0 flex-1">
            <div
              className="truncate text-sm font-bold text-white"
              style={{ letterSpacing: "-0.01em" }}
            >
              {titleText}
            </div>
            <div className="mt-0.5 truncate text-xs text-[var(--scry-muted)]">
              {subtitle}
            </div>
          </div>
          <div className="flex items-center gap-2.5">
            {visualState === "scanning" ? (
              <>
                {etaCountdown ? (
                  <span className="text-xs font-semibold tabular-nums text-[var(--scry-muted2)]">
                    {etaCountdown}
                  </span>
                ) : null}
                <LoadingMark className="h-4 w-4" />
              </>
            ) : badge ? (
              <span
                className="inline-flex h-[22px] items-center gap-1.5 rounded-[7px] px-[9px] text-[11px] font-bold"
                style={{
                  background: badge.bg,
                  border: `1px solid ${badge.border}`,
                  color: badge.color,
                }}
              >
                <badge.Icon className="h-3 w-3" aria-hidden="true" />
                {badge.label}
              </span>
            ) : null}
          </div>
        </div>

        <div className="mt-[14px]">
          <ScanPhaseBar
            facet={facet}
            label={t("settings.libraryScanTitleMatch")}
            status={titleMatchStatus}
            count={phaseCountLabel(
              session.titleMatchProgress,
              session.titleMatchTotalKnown,
              terminal,
              t("settings.libraryScanNoTitleMatchNeeded"),
              t,
            )}
            percent={percentForPhase(
              session.titleMatchProgress,
              session.titleMatchTotalKnown,
              terminal,
            )}
            indeterminate={!terminal && !session.titleMatchTotalKnown}
          />
          <ScanPhaseBar
            facet={facet}
            label={t("settings.libraryScanFilesScanned")}
            status={mediaStatus}
            count={phaseCountLabel(
              session.mediaAnalysisProgress,
              session.mediaAnalysisTotalKnown,
              terminal,
              t("settings.libraryScanNoFilesToScan"),
              t,
            )}
            percent={percentForPhase(
              session.mediaAnalysisProgress,
              session.mediaAnalysisTotalKnown,
              terminal,
            )}
            indeterminate={!terminal && !session.mediaAnalysisTotalKnown}
          />
        </div>

        {chip ? (
          <div
            className="mt-1 flex items-center gap-2.5 rounded-[10px] px-3 py-2.5"
            style={{ background: chip.bg, border: `1px solid ${chip.border}` }}
          >
            <chip.Icon
              className="h-[15px] w-[15px] shrink-0"
              style={{ color: chip.color }}
              aria-hidden="true"
            />
            <span className="text-xs font-semibold text-[var(--scry-text2)]">
              {chip.text}
            </span>
          </div>
        ) : null}

        {footer}
      </div>
    </div>
  );
}
