import { Check } from "lucide-react";

interface SetupProgressBarProps {
  currentStep: number;
  stepLabels: string[];
  onStepClick?: (stepIndex: number) => void;
}

export function SetupProgressBar({
  currentStep,
  stepLabels,
  onStepClick,
}: SetupProgressBarProps) {
  return (
    <div className="flex flex-wrap items-center justify-center gap-x-1 gap-y-2 sm:flex-nowrap sm:gap-x-2">
      {stepLabels.map((label, index) => {
        const isComplete = index < currentStep;
        const isCurrent = index === currentStep;
        const isClickable = isComplete && onStepClick != null;

        const node = (
          <div className="flex items-center gap-1.5">
            <div
              className={`flex h-6 w-6 items-center justify-center rounded-full text-xs font-semibold ${
                isComplete
                  ? "text-white shadow-[0_6px_16px_rgba(var(--scry-accent-rgb),0.4)]"
                  : isCurrent
                    ? "border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-text)]"
                    : "bg-[var(--scry-chip)] text-[var(--scry-muted2)]"
              }`}
              style={
                isComplete
                  ? { backgroundImage: "var(--scry-accent-grad)" }
                  : undefined
              }
            >
              {isComplete ? <Check className="h-3.5 w-3.5" /> : index + 1}
            </div>
            {/* On phones only the current step is named, so the bar fits. */}
            <span
              className={`whitespace-nowrap text-xs ${
                isCurrent ? "" : "sr-only sm:not-sr-only"
              } ${
                isCurrent
                  ? "font-semibold text-[var(--scry-ink2)]"
                  : isComplete
                    ? "text-[var(--scry-text2)]"
                    : "text-[var(--scry-muted)]"
              }`}
            >
              {label}
            </span>
          </div>
        );

        return (
          <div key={label} className="flex items-center gap-1 sm:gap-2">
            {index > 0 && (
              <div
                className="h-px w-3 rounded-full sm:w-6"
                style={{
                  background: isComplete
                    ? "var(--scry-accent)"
                    : "var(--scry-border2)",
                }}
              />
            )}
            {isClickable ? (
              <button
                type="button"
                onClick={() => onStepClick(index)}
                aria-label={`Go to ${label}`}
                className="cursor-pointer rounded-full transition-opacity hover:opacity-80 focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-baccent)]"
              >
                {node}
              </button>
            ) : (
              <div aria-current={isCurrent ? "step" : undefined}>{node}</div>
            )}
          </div>
        );
      })}
    </div>
  );
}
