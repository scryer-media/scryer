
import * as React from "react";
import * as ProgressPrimitive from "@radix-ui/react-progress";

import { cn } from "@/lib/utils";

type ProgressProps = React.ComponentProps<typeof ProgressPrimitive.Root> & {
  indicatorClassName?: string;
};

function normalizeProgressValue(value: number | null | undefined): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return 0;
  }
  return Math.min(100, Math.max(0, value));
}

function Progress({ className, value, indicatorClassName, ...props }: ProgressProps) {
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
        className={cn("h-full w-full flex-1 transition-all duration-500 ease-out motion-reduce:transition-none bg-primary", indicatorClassName)}
        style={{ transform: `translateX(-${100 - progressValue}%)` }}
      />
    </ProgressPrimitive.Root>
  );
}

export { Progress };
