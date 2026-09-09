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

export const decimalInputProps = {
  type: "text",
  inputMode: "decimal",
} satisfies Pick<React.ComponentProps<"input">, "type" | "inputMode">

/** Digits and at most one decimal point; everything else is dropped. */
export function sanitizeDecimal(raw: string): string {
  const digitsAndDots = raw.replace(/[^\d.]+/g, "")
  const firstDot = digitsAndDots.indexOf(".")
  if (firstDot === -1) {
    return digitsAndDots
  }
  return (
    digitsAndDots.slice(0, firstDot + 1) +
    digitsAndDots.slice(firstDot + 1).replace(/\./g, "")
  )
}

type InputProps = React.ComponentProps<"input"> & {
  "data-1p-ignore"?: string
  "data-lpignore"?: string
  "data-bwignore"?: string
  "data-form-type"?: string
  "data-protonpass-ignore"?: string
  /**
   * Force the password-manager opt-out even for credential-shaped fields.
   * Use for third-party credentials that are not the user's Scryer login, so
   * managers do not offer (or offer to save) the wrong password.
   */
  ignorePasswordManagers?: boolean
}

const credentialAutocompleteValues = new Set([
  "current-password",
  "new-password",
  "one-time-code",
  "username",
])

function shouldSuppressPasswordManager(
  type: React.HTMLInputTypeAttribute | undefined,
  autoComplete: string | undefined,
  force: boolean | undefined,
): boolean {
  if (force) {
    return true
  }

  if (type === "password") {
    return false
  }

  const normalizedAutoComplete = autoComplete?.toLowerCase()
  return !(
    normalizedAutoComplete &&
    credentialAutocompleteValues.has(normalizedAutoComplete)
  )
}

function Input({
  className,
  type,
  autoComplete,
  ignorePasswordManagers,
  "data-1p-ignore": onePasswordIgnore,
  "data-lpignore": lastPassIgnore,
  "data-bwignore": bitwardenIgnore,
  "data-form-type": formType,
  "data-protonpass-ignore": protonPassIgnore,
  ...props
}: InputProps) {
  const suppressPasswordManager = shouldSuppressPasswordManager(
    type,
    autoComplete,
    ignorePasswordManagers,
  )
  // An explicit opt-out overrides any credential autocomplete token the caller
  // passed, so the field cannot advertise itself as fillable.
  const resolvedAutoComplete = ignorePasswordManagers ? "off" : autoComplete

  return (
    <input
      type={type}
      data-slot="input"
      autoComplete={resolvedAutoComplete ?? (suppressPasswordManager ? "off" : undefined)}
      data-1p-ignore={onePasswordIgnore ?? (suppressPasswordManager ? "true" : undefined)}
      data-lpignore={lastPassIgnore ?? (suppressPasswordManager ? "true" : undefined)}
      data-bwignore={bitwardenIgnore ?? (suppressPasswordManager ? "true" : undefined)}
      data-form-type={formType ?? (suppressPasswordManager ? "other" : undefined)}
      data-protonpass-ignore={protonPassIgnore ?? (suppressPasswordManager ? "true" : undefined)}
      className={cn(
        "file:text-foreground placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground bg-field text-foreground border-input h-10 w-full min-w-0 rounded-md border px-3 py-1 text-base shadow-xs transition-[color,box-shadow] outline-none file:inline-flex file:h-7 file:border-0 file:bg-transparent file:text-sm file:font-medium disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
        "focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]",
        "aria-invalid:ring-destructive/20 aria-invalid:ring-destructive/40 aria-invalid:border-destructive",
        className
      )}
      {...props}
    />
  )
}

export { Input }
