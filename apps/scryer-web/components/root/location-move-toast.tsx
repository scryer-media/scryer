import * as React from "react";
import {
  ArrowUpRight,
  Check,
  CircleAlert,
  CircleCheck,
  FolderInput,
  LoaderCircle,
  PictureInPicture2,
  Timer,
  TriangleAlert,
  X,
  type LucideIcon,
} from "lucide-react";

import type { Translate } from "@/components/root/types";
import { operationStateLabelKey, toCount } from "@/lib/location-operations";
import {
  formatMoveEta,
  moveEtaSeconds,
  moveToastVisualState,
} from "@/lib/location-move-toasts";
import {
  transferOperationProgress,
  type TransferSnapshot,
} from "@/lib/location-transfers";
import { formatByteCount } from "@/lib/utils/activity-utils";
import { cn } from "@/lib/utils";

const DEFAULT_AUTO_DISMISS_MS = 5_000;

// Moves are not facet-scoped, so the card wears the accent instead of a
// facet colour; the shape is the library-scan toast's, cut down to one bar.
const ACCENT = {
  rgb: "var(--scry-accent-rgb)",
  base: "var(--scry-accent)",
  grad: "linear-gradient(135deg, var(--scry-accent-text), var(--scry-accent))",
};

function MoveProgressBar({
  label,
  count,
  percent,
  state,
  indeterminate,
}: {
  label: string;
  count: string;
  percent: number;
  state: ReturnType<typeof moveToastVisualState>;
  indeterminate: boolean;
}) {
  let Icon: LucideIcon;
  let iconColor: string;
  let spin = false;
  let fillStyle: React.CSSProperties;
  let labelColor: string;
  let countColor: string;

  if (state === "success") {
    Icon = CircleCheck;
    iconColor = "var(--scry-success-text-soft)";
    fillStyle = { width: "100%", background: "var(--scry-success-bg-strong)" };
    labelColor = "var(--scry-text2)";
    countColor = "var(--scry-muted2)";
  } else if (state === "issues") {
    Icon = TriangleAlert;
    iconColor = "var(--scry-warning-text)";
    fillStyle = { width: "100%", background: "var(--scry-warning-solid)" };
    labelColor = "var(--scry-text2)";
    countColor = "var(--scry-muted2)";
  } else if (state === "failed") {
    Icon = CircleAlert;
    iconColor = "var(--scry-danger-text-soft)";
    fillStyle = {
      width: `${percent}%`,
      background: "var(--scry-danger-text-soft)",
    };
    labelColor = "var(--scry-text2)";
    countColor = "var(--scry-muted2)";
  } else if (state === "canceled") {
    Icon = X;
    iconColor = "var(--scry-muted2)";
    fillStyle = { width: `${percent}%`, background: "var(--scry-muted2)" };
    labelColor = "var(--scry-text2)";
    countColor = "var(--scry-muted2)";
  } else {
    Icon = LoaderCircle;
    iconColor = ACCENT.base;
    spin = true;
    fillStyle = {
      width: `${percent}%`,
      background: ACCENT.grad,
      boxShadow: `0 0 12px rgba(${ACCENT.rgb},.5)`,
    };
    labelColor = "var(--scry-ink2)";
    countColor = ACCENT.base;
  }

  return (
    <div>
      <div className="mb-1.5 flex items-center gap-2">
        <Icon
          className={cn("h-3.5 w-3.5 shrink-0", spin && "animate-spin")}
          style={{ color: iconColor }}
          aria-hidden="true"
        />
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
        {state === "moving" && indeterminate ? (
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

/**
 * The library-scan toast's card for one location operation: one bar, the
 * server's ETA, and a way into Activity. `snapshot` is null until the first
 * transfer summary lands, which reads as "preparing".
 */
export function LocationMoveToast({
  snapshot,
  t,
  onRunInBackground,
  onDismiss,
  onSeeInActivity,
  autoDismissMs = DEFAULT_AUTO_DISMISS_MS,
}: {
  snapshot: TransferSnapshot | null;
  t: Translate;
  onRunInBackground?: () => void;
  onDismiss?: () => void;
  onSeeInActivity?: () => void;
  autoDismissMs?: number;
}) {
  const operation = snapshot?.operation ?? null;
  const visualState = moveToastVisualState(operation?.state);
  const moving = visualState === "moving";
  const autoDismiss =
    !!onDismiss && (visualState === "success" || visualState === "canceled");
  const countdownColor =
    visualState === "success"
      ? "rgba(var(--scry-success-rgb),.2)"
      : "rgba(255,255,255,.12)";

  const [paused, setPaused] = React.useState(false);

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

  const titlesTotal = toCount(operation?.counters.titlesTotal);
  const titlesProcessed = Math.min(
    titlesTotal,
    toCount(operation?.counters.titlesProcessed),
  );
  const bytesTotal = toCount(operation?.counters.bytesTotal);
  const bytesProcessed = Math.min(
    bytesTotal,
    toCount(operation?.counters.bytesProcessed),
  );
  const percent = snapshot ? transferOperationProgress(snapshot) : 0;
  const etaSeconds = moving ? moveEtaSeconds(snapshot?.etaSeconds) : null;
  const etaCountdown = etaSeconds != null ? formatMoveEta(etaSeconds) : null;

  const titlesProgress = t("move.toastTitlesProgress", {
    current: titlesProcessed.toLocaleString(),
    total: titlesTotal.toLocaleString(),
  });
  const titleText = operation
    ? t(`move.operationType.${operation.operationType}`)
    : t("move.toastPreparing");
  const subtitle = !operation
    ? t("move.toastPreparing")
    : visualState === "moving"
      ? `${t(operationStateLabelKey(operation.state))} · ${titlesProgress}`
      : visualState === "success"
        ? t("move.toastDoneSubtitle", { count: titlesTotal })
        : visualState === "issues"
          ? t("move.toastWarningsSubtitle", { count: titlesTotal })
          : visualState === "failed"
            ? t("move.toastFailedSubtitle")
            : t("move.toastCanceledSubtitle");
  const barCount = !operation
    ? t("move.toastPreparing")
    : bytesTotal > 0
      ? `${formatByteCount(bytesProcessed)} / ${formatByteCount(bytesTotal)}`
      : titlesProgress;

  const accent =
    visualState === "moving"
      ? ACCENT.grad
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
  if (visualState === "issues") {
    chip = {
      Icon: CircleAlert,
      color: "var(--scry-warning-text)",
      bg: "var(--scry-warning-bg)",
      border: "var(--scry-warning-border)",
      text: t("move.toastWarningsChip"),
    };
  } else if (visualState === "failed") {
    chip = {
      Icon: CircleAlert,
      color: "var(--scry-danger-text-soft)",
      bg: "var(--scry-danger-bg)",
      border: "var(--scry-danger-border)",
      text: operation?.detail || t("move.toastFailedSubtitle"),
    };
  }

  const seeInActivityButton = onSeeInActivity ? (
    <button
      type="button"
      onClick={onSeeInActivity}
      className="flex h-9 flex-1 items-center justify-center gap-[7px] rounded-[9px] text-[12.5px] font-semibold transition hover:brightness-110"
      style={{
        background: "rgba(var(--scry-accent-rgb),.16)",
        color: "var(--scry-accent-text)",
      }}
    >
      <ArrowUpRight className="h-3.5 w-3.5" aria-hidden="true" />
      {t("move.viewInActivity")}
    </button>
  ) : null;

  const dismissButton = autoDismiss ? (
    <button
      type="button"
      onClick={onDismiss}
      className={cn(
        "relative flex h-9 items-center justify-center gap-[7px] overflow-hidden rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[12.5px] font-semibold text-[var(--scry-text2)] transition hover:brightness-110",
        seeInActivityButton ? "w-28" : "flex-1",
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
        seeInActivityButton ? "w-20" : "flex-1",
      )}
    >
      {t("label.dismiss")}
    </button>
  );

  let footer: React.ReactNode = null;
  if (moving && (onRunInBackground || seeInActivityButton)) {
    footer = (
      <div className="mt-[14px] flex items-center gap-[9px]">
        {onRunInBackground ? (
          <button
            type="button"
            onClick={onRunInBackground}
            className="flex h-9 flex-1 items-center justify-center gap-[7px] rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[12.5px] font-semibold text-[var(--scry-text2)] transition hover:brightness-110"
          >
            <PictureInPicture2 className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.libraryScanRunInBackground")}
          </button>
        ) : null}
        {seeInActivityButton}
      </div>
    );
  } else if (!moving && (onDismiss || seeInActivityButton)) {
    footer = (
      <div className="mt-[14px] flex items-center gap-[9px]">
        {seeInActivityButton}
        {onDismiss ? dismissButton : null}
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
              background: `rgba(${ACCENT.rgb},.14)`,
              border: `1px solid rgba(${ACCENT.rgb},.32)`,
            }}
          >
            <FolderInput
              className="h-[19px] w-[19px]"
              style={{ color: ACCENT.base }}
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
            {moving ? (
              <>
                {etaCountdown ? (
                  <span className="text-xs font-semibold tabular-nums text-[var(--scry-muted2)]">
                    {etaCountdown}
                  </span>
                ) : null}
                <LoaderCircle
                  className="h-4 w-4 animate-spin"
                  style={{ color: ACCENT.base }}
                  aria-hidden="true"
                />
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
          <MoveProgressBar
            label={t("move.toastTransferred")}
            count={barCount}
            percent={percent}
            state={visualState}
            indeterminate={!operation || percent <= 0}
          />
        </div>

        {chip ? (
          <div
            className="mt-[14px] flex items-center gap-2.5 rounded-[10px] px-3 py-2.5"
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
