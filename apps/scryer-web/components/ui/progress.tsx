
import * as React from "react";
import * as ProgressPrimitive from "@radix-ui/react-progress";

import { cn } from "@/lib/utils";

type ProgressProps = React.ComponentProps<typeof ProgressPrimitive.Root> & {
  indicatorClassName?: string;
  indeterminate?: boolean;
};

function normalizeProgressValue(value: number | null | undefined): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return 0;
  }
  return Math.min(100, Math.max(0, value));
}

function Progress({
  className,
  value,
  indicatorClassName,
  indeterminate = false,
  ...props
}: ProgressProps) {
  const progressValue = normalizeProgressValue(value);

  return (
    <ProgressPrimitive.Root
      data-slot="progress"
      className={cn("relative h-2.5 w-full overflow-hidden rounded-full bg-muted", className)}
      value={progressValue}
      {...props}
    >
      <ProgressPrimitive.Indicator
        data-slot="progress-indicator"
        className={cn(
          "h-full bg-primary will-change-transform motion-reduce:transition-none",
          indeterminate
            ? "w-2/5 animate-[scryer-progress-indeterminate_1.35s_cubic-bezier(0.4,0,0.2,1)_infinite] motion-reduce:animate-none"
            : "w-full transition-transform duration-700 ease-[cubic-bezier(0.22,1,0.36,1)]",
          indicatorClassName,
        )}
        style={
          indeterminate
            ? undefined
            : { transform: `translateX(-${100 - progressValue}%)` }
        }
      />
    </ProgressPrimitive.Root>
  );
}

export { Progress };
