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
  /** `lg` is for a page's main sections rather than a filter over one list. */
  size?: "md" | "lg";
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
    size = "md",
    className,
    ...buttonProps
  },
  ref,
) {
  const ariaLabel = buttonProps["aria-label"] ?? (badge ? `${label} (${badge})` : label);
  const ariaPressed = buttonProps["aria-pressed"] ?? selected;
  const large = size === "lg";

  return (
    <button
      {...buttonProps}
      ref={ref}
      type="button"
      aria-label={ariaLabel}
      aria-pressed={ariaPressed}
      className={cn(
        "relative inline-flex shrink-0 items-center gap-2 font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]",
        large
          ? "h-12 rounded-t-[10px] px-4 py-3 text-[15.5px]"
          : "h-10 px-3.5 py-2.5 text-[13.5px]",
        selected
          ? cn("text-white", large && "bg-[rgba(var(--scry-accent-rgb),0.14)]")
          : cn("text-[var(--scry-muted)] hover:text-[var(--scry-ink2)]", large && "hover:bg-[var(--scry-hover)]"),
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
            "inline-flex justify-center rounded-[6px] font-bold leading-none tabular-nums",
            large ? "min-w-[3ch] px-2 py-1 text-[12px]" : "min-w-[6ch] px-1.5 py-0.5 text-[11px]",
            selected
              ? cn("text-[var(--scry-accent-text)]", large ? "bg-[rgba(var(--scry-accent-rgb),0.3)]" : "bg-[rgba(var(--scry-accent-rgb),0.18)]")
              : "bg-[var(--scry-chip)] text-[var(--scry-muted2)]",
          )}
        >
          {count.toLocaleString()}
        </span>
      ) : null}
      {selected ? (
        <span
          className={cn(
            "absolute bottom-[-1px] rounded-full bg-[var(--scry-accent-ring)]",
            large ? "left-0 right-0 h-[3px]" : "left-2 right-2 h-[2.5px]",
          )}
        />
      ) : null}
    </button>
  );
});
