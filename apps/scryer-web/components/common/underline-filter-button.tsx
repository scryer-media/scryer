import * as React from "react";
import { cn } from "@/lib/utils";

export type UnderlineFilterButtonTone =
  | "neutral"
  | "success"
  | "danger"
  | "muted";

type UnderlineFilterButtonProps = Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "children" | "type"
> & {
  selected: boolean;
  icon?: React.ReactNode;
  label: string;
  count?: number;
  /** A short tag after the label, such as one marking the tab as advanced. */
  badge?: string;
  tone?: UnderlineFilterButtonTone;
};

export const UnderlineFilterButton = React.forwardRef<
  HTMLButtonElement,
  UnderlineFilterButtonProps
>(function UnderlineFilterButton(
  {
    selected,
    icon,
    label,
    count,
    badge,
    tone = "neutral",
    className,
    ...buttonProps
  },
  ref,
) {
  const ariaLabel = buttonProps["aria-label"] ?? (badge ? `${label} (${badge})` : label);
  const ariaPressed = buttonProps["aria-pressed"] ?? selected;

  return (
    <button
      {...buttonProps}
      ref={ref}
      type="button"
      aria-label={ariaLabel}
      aria-pressed={ariaPressed}
      className={cn(
        "relative inline-flex h-10 shrink-0 items-center gap-2 px-3.5 py-2.5 text-[13.5px] font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]",
        selected
          ? "text-white"
          : "text-[var(--scry-muted)] hover:text-[var(--scry-ink2)]",
        className,
      )}
    >
      {icon ? (
        <span
          className={cn(
            "shrink-0",
            selected
              ? "text-[var(--scry-accent-text)]"
              : tone === "success"
                ? "text-[var(--scry-success-text-soft)]"
                : tone === "danger"
                  ? "text-[var(--scry-danger-text-soft)]"
                  : tone === "muted"
                    ? "text-zinc-400"
                    : "text-[var(--scry-muted2)]",
          )}
        >
          {icon}
        </span>
      ) : null}
      <span className="whitespace-nowrap">{label}</span>
      {badge ? (
        <span className="rounded-[6px] border border-[var(--scry-border2)] px-1.5 py-0.5 text-[10px] font-bold uppercase leading-none tracking-[0.06em] text-[var(--scry-muted2)]">
          {badge}
        </span>
      ) : null}
      {typeof count === "number" ? (
        <span
          className={cn(
            "inline-flex min-w-[6ch] justify-center rounded-[6px] px-1.5 py-0.5 text-[11px] font-bold leading-none tabular-nums",
            selected
              ? "bg-[rgba(var(--scry-accent-rgb),0.18)] text-[var(--scry-accent-text)]"
              : "bg-[var(--scry-chip)] text-[var(--scry-muted2)]",
          )}
        >
          {count.toLocaleString()}
        </span>
      ) : null}
      {selected ? (
        <span className="absolute bottom-[-1px] left-2 right-2 h-[2.5px] rounded-full bg-[var(--scry-accent-ring)]" />
      ) : null}
    </button>
  );
});
