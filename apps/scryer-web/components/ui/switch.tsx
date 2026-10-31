import * as React from "react";
import { Switch as SwitchPrimitive } from "radix-ui";

import { cn } from "@/lib/utils";

type SwitchProps = React.ComponentProps<typeof SwitchPrimitive.Root> & {
  size?: "default" | "lg";
};

function Switch({ className, size = "default", ...props }: SwitchProps) {
  const track = size === "lg" ? "h-8 w-14 p-1" : "h-6 w-11 p-0.5";
  const thumb =
    size === "lg"
      ? "size-6 data-[state=checked]:translate-x-6"
      : "size-5 data-[state=checked]:translate-x-5";

  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        "peer inline-flex shrink-0 items-center rounded-full border border-transparent transition-colors duration-200 outline-none focus-visible:border-[var(--scry-accent-ring)] focus-visible:ring-2 focus-visible:ring-[rgba(var(--scry-accent-rgb),0.4)] disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:border-[var(--scry-success-border-strong)] data-[state=checked]:bg-[var(--scry-success-solid)] data-[state=unchecked]:border-[var(--scry-danger-border)] data-[state=unchecked]:bg-[var(--scry-danger-bg)]",
        track,
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className={cn(
          "pointer-events-none inline-block rounded-full bg-background shadow-sm transition-transform duration-200 data-[state=unchecked]:translate-x-0",
          thumb,
        )}
      />
    </SwitchPrimitive.Root>
  );
}

export { Switch };
