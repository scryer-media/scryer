import type * as React from "react";

import { cn } from "@/lib/utils";

/**
 * A bordered group whose label sits in its top border and names everything
 * inside it.
 */
export function LabeledFieldset({
  label,
  className,
  children,
  ...props
}: React.ComponentProps<"fieldset"> & { label: React.ReactNode }) {
  return (
    <fieldset className={cn("min-w-0 space-y-3 rounded-lg border border-border px-3 pb-3", className)} {...props}>
      {/* Sits in the top border, its text level with the fields below. */}
      <legend className="-ml-1.5 px-1.5 text-sm leading-none font-medium">{label}</legend>
      {children}
    </fieldset>
  );
}
