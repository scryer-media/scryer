import type * as React from "react";
import * as ToggleGroupPrimitive from "@radix-ui/react-toggle-group";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

export type ScriptChoiceOption = {
  value: string;
  label: string;
  /** A mark shown ahead of the label, such as a language logo. */
  icon?: React.ReactNode;
  /** The value exposed on the option element when it differs from `value`. */
  dataValue?: string;
  /** The element id of the option, when a page or test addresses it directly. */
  id?: string;
};

/**
 * A segmented single choice. Each option carries its value in `data-value`
 * and its selection in `aria-checked`, so the current choice is readable
 * without opening a menu.
 */
export function ScriptChoiceGroup({
  id,
  label,
  description,
  value,
  options,
  onValueChange,
  disabled,
  className,
}: {
  id: string;
  label: string;
  description?: string;
  value: string;
  options: readonly ScriptChoiceOption[];
  onValueChange: (value: string) => void;
  disabled?: boolean;
  className?: string;
}) {
  const labelId = `${id}-label`;
  return (
    <div className={cn("min-w-0 space-y-1.5", className)}>
      <Label id={labelId} className="block">
        {label}
      </Label>
      <ToggleGroupPrimitive.Root
        id={id}
        type="single"
        aria-labelledby={labelId}
        // Wraps instead of overflowing when the options outgrow a narrow form.
        className="inline-flex max-w-full flex-wrap rounded-md border border-border p-1"
        value={value}
        disabled={disabled}
        onValueChange={(next) => {
          // A single toggle group clears its value when the active item is
          // clicked again; a choice always keeps one option selected.
          if (next) onValueChange(next);
        }}
      >
        {options.map((option) => (
          <ToggleGroupPrimitive.Item
            key={option.value}
            id={option.id}
            value={option.value}
            data-value={option.dataValue ?? option.value}
            asChild
          >
            <Button
              type="button"
              size="sm"
              variant={option.value === value ? "default" : "ghost"}
            >
              {option.icon}
              {option.label}
            </Button>
          </ToggleGroupPrimitive.Item>
        ))}
      </ToggleGroupPrimitive.Root>
      {description ? (
        <p className="text-xs leading-5 text-[var(--scry-muted3)]">{description}</p>
      ) : null}
    </div>
  );
}
