export const boxedActionButtonBaseClass =
  "h-9 w-9 border shadow-[0_1px_2px_rgba(15,23,42,0.05),inset_0_1px_0_rgba(255,255,255,0.8)] transition-[color,background-color,border-color,box-shadow,transform] hover:-translate-y-px hover:shadow-[0_10px_20px_rgba(15,23,42,0.08),inset_0_1px_0_rgba(255,255,255,0.85)] dark:shadow-none dark:hover:translate-y-0 dark:hover:shadow-none";

export const boxedTextActionButtonBaseClass =
  "h-9 border px-3 shadow-[0_1px_2px_rgba(15,23,42,0.05),inset_0_1px_0_rgba(255,255,255,0.8)] transition-[color,background-color,border-color,box-shadow,transform] hover:-translate-y-px hover:shadow-[0_10px_20px_rgba(15,23,42,0.08),inset_0_1px_0_rgba(255,255,255,0.85)] dark:shadow-none dark:hover:translate-y-0 dark:hover:shadow-none";

export const boxedActionButtonToneClass = {
  auto:
    "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)] hover:border-[var(--scry-success-border-strong)] hover:bg-[var(--scry-success-bg-strong)] hover:text-[var(--scry-success-text)]",
  search:
    "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)] hover:border-[var(--scry-info-border-strong)] hover:bg-[var(--scry-info-bg-strong)] hover:text-[var(--scry-info-text)]",
  accent:
    "border-[rgba(var(--scry-accent-rgb),0.3)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[var(--scry-accent-text)] hover:border-[rgba(var(--scry-accent-rgb),0.45)] hover:bg-[rgba(var(--scry-accent-rgb),0.2)] hover:text-[var(--scry-accent-text)]",
  accentBright:
    "border-[rgba(var(--scry-accent-rgb),0.7)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[var(--scry-accent-text)] hover:border-[rgba(var(--scry-accent-rgb),0.9)] hover:bg-[rgba(var(--scry-accent-rgb),0.2)] hover:text-[var(--scry-accent-text)]",
  edit:
    "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)] hover:border-[var(--scry-info-border-strong)] hover:bg-[var(--scry-info-bg-strong)] hover:text-[var(--scry-info-text)]",
  reorder:
    "border-[rgba(var(--scry-accent-rgb),0.3)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[var(--scry-accent-text)] hover:border-[rgba(var(--scry-accent-rgb),0.45)] hover:bg-[rgba(var(--scry-accent-rgb),0.2)] hover:text-[var(--scry-accent-text)]",
  neutral:
    "border-[var(--scry-border2)] bg-[var(--scry-chip)] text-[var(--scry-muted)] hover:border-[var(--scry-bhover)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]",
  install:
    "border-[rgba(var(--scry-accent-rgb),0.55)] bg-[rgb(var(--scry-accent-rgb))] text-white hover:border-[rgba(var(--scry-accent-rgb),0.72)] hover:bg-[var(--scry-accent-dark)] hover:text-white",
  upgrade:
    "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)] hover:border-[var(--scry-warning-border-strong)] hover:bg-[var(--scry-warning-bg-strong)] hover:text-[var(--scry-warning-text)]",
  enabled:
    "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)] hover:border-[var(--scry-success-border-strong)] hover:bg-[var(--scry-success-bg-strong)] hover:text-[var(--scry-success-text)]",
  disabled:
    "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)] hover:border-[var(--scry-danger-border-strong)] hover:bg-[var(--scry-danger-bg-strong)] hover:text-[var(--scry-danger-text)]",
  delete:
    "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)] hover:border-[var(--scry-danger-border-strong)] hover:bg-[var(--scry-danger-bg-strong)] hover:text-[var(--scry-danger-text)]",
} as const;

export type BoxedActionButtonTone = keyof typeof boxedActionButtonToneClass;
