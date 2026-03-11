import * as React from "react"

import { cn } from "@/lib/utils"

export const integerInputProps = {
  type: "text",
  inputMode: "numeric",
  pattern: "[0-9]*",
} satisfies Pick<
  React.ComponentProps<"input">,
  "type" | "inputMode" | "pattern"
>

export const signedIntegerInputProps = {
  type: "number",
  inputMode: "numeric",
  step: 1,
} satisfies Pick<
  React.ComponentProps<"input">,
  "type" | "inputMode" | "step"
>

export function sanitizeDigits(raw: string): string {
  return raw.replace(/\D+/g, "")
}

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "file:text-foreground placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground bg-field text-foreground border-input h-9 w-full min-w-0 rounded-md border px-3 py-1 text-base shadow-xs transition-[color,box-shadow] outline-none file:inline-flex file:h-7 file:border-0 file:bg-transparent file:text-sm file:font-medium disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
        "focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]",
        "aria-invalid:ring-destructive/20 aria-invalid:ring-destructive/40 aria-invalid:border-destructive",
        className
      )}
      {...props}
    />
  )
}

export { Input }
