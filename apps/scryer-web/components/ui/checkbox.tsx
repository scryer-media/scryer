
import * as React from "react"
import { CheckIcon, MinusIcon } from "lucide-react"
import { Checkbox as CheckboxPrimitive } from "radix-ui"

import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"

/**
 * `large` is the prominent on/off treatment: an oversized box for a single
 * toggle that speaks for a whole form -- an editor's Enabled switch, say --
 * rather than one field among many. It takes the same accent as every other
 * checkbox, so a form never invents its own colour for "on".
 */
type CheckboxSize = "default" | "compact" | "table" | "large"

type CheckboxProps = Omit<
  React.ComponentProps<typeof CheckboxPrimitive.Root>,
  "children"
> & {
  size?: CheckboxSize
}

const CHECKBOX_SIZE_CLASS: Record<CheckboxSize, string> = {
  default: "size-5 rounded-[6px]",
  compact: "size-[18px] rounded-[5px]",
  table: "size-[18px] rounded-[5px]",
  large: "size-[31px] rounded-md",
}

const CHECKBOX_ICON_CLASS: Record<CheckboxSize, string> = {
  default: "size-3.5",
  compact: "size-3",
  table: "size-3",
  // Deliberately not scaled with the box: the large treatment reads as a
  // panel that fills, not as a bigger tick.
  large: "size-3.5",
}

function Checkbox({
  className,
  size = "default",
  ...props
}: CheckboxProps) {
  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      className={cn(
        "peer shrink-0 border border-[var(--scry-border3)] bg-[var(--scry-inset)] text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] outline-none transition-colors focus-visible:border-[var(--scry-accent)] focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)] disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-[var(--scry-danger)] aria-invalid:ring-2 aria-invalid:ring-[var(--scry-danger-border)] data-[state=checked]:border-[var(--scry-accent)] data-[state=checked]:bg-[var(--scry-accent)] data-[state=checked]:text-white data-[state=indeterminate]:border-[var(--scry-accent)] data-[state=indeterminate]:bg-[var(--scry-accent)] data-[state=indeterminate]:text-white",
        CHECKBOX_SIZE_CLASS[size],
        className
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator
        data-slot="checkbox-indicator"
        className="grid place-content-center text-current transition-none"
      >
        {props.checked === "indeterminate" ? (
          <MinusIcon className={CHECKBOX_ICON_CLASS[size]} />
        ) : (
          <CheckIcon className={CHECKBOX_ICON_CLASS[size]} />
        )}
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  )
}

type CheckboxFieldProps = Omit<CheckboxProps, "className"> & {
  id: string
  label: React.ReactNode
  description?: React.ReactNode
  labelAccessory?: React.ReactNode
  className?: string
  checkboxClassName?: string
  labelClassName?: string
  descriptionClassName?: string
}

function CheckboxField({
  id,
  label,
  description,
  labelAccessory,
  className,
  checkboxClassName,
  labelClassName,
  descriptionClassName,
  disabled,
  size = "default",
  ...props
}: CheckboxFieldProps) {
  return (
    <div
      className={cn(
        "flex min-w-0 items-start gap-3",
        disabled && "opacity-60",
        className
      )}
      data-disabled={disabled ? "true" : undefined}
    >
      <Checkbox
        id={id}
        disabled={disabled}
        size={size}
        className={cn("mt-0.5", checkboxClassName)}
        {...props}
      />
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex min-w-0 items-center gap-2">
          <Label
            htmlFor={id}
            className={cn(
              "cursor-pointer text-sm font-medium leading-5 text-[var(--scry-ink3)]",
              disabled && "cursor-not-allowed",
              labelClassName
            )}
          >
            {label}
          </Label>
          {labelAccessory}
        </div>
        {description ? (
          <p
            className={cn(
              "text-sm leading-5 text-[var(--scry-muted2)]",
              descriptionClassName
            )}
          >
            {description}
          </p>
        ) : null}
      </div>
    </div>
  )
}

export { Checkbox, CheckboxField }
