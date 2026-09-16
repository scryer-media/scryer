import { LoadingMark } from "@/components/common/loading-mark";

export function ViewLoadingFallback() {
  return (
    <div
      className="flex min-h-[18rem] items-center justify-center px-6 py-12 text-[var(--scry-body)]"
      role="status"
      aria-live="polite"
    >
      <div className="flex items-center gap-3 rounded-[12px] border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] px-4 py-3 shadow-[0_12px_28px_rgba(2,6,23,0.16)]">
        <span className="flex h-9 w-9 items-center justify-center rounded-[9px] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-ring)]">
          <LoadingMark className="h-4 w-4" />
        </span>
        <span className="text-sm font-medium text-[var(--scry-muted)]">Loading view…</span>
      </div>
    </div>
  );
}
