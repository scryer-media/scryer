import * as React from "react";
import { Check, ChevronDown } from "lucide-react";

import {
  Command,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  type SelectChrome,
  type SelectSize,
  selectContentClassName,
  selectTriggerClassName,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

export type FilterableSelectOption = {
  value: string;
  /// Plain text so the filter has something to match on. Rich labels belong in
  /// `description`, which is searched too but rendered separately.
  label: string;
  description?: string;
  disabled?: boolean;
};

type FilterableSelectProps = {
  value: string;
  onValueChange: (value: string) => void;
  options: FilterableSelectOption[];
  id?: string;
  placeholder?: string;
  filterPlaceholder?: string;
  emptyLabel?: string;
  ariaLabel?: string;
  disabled?: boolean;
  size?: SelectSize;
  chrome?: SelectChrome;
  className?: string;
  contentClassName?: string;
  optionIdPrefix?: string;
  align?: "start" | "center" | "end";
};

/// A single-select whose options are filtered as you type.
///
/// Same trigger chrome as [`SingleSelectField`], so the two sit together in a
/// form without looking like different controls. Reach for this when the list
/// is long enough that scanning it unaided is the wrong ask — a plain select is
/// better for a handful of options, because it costs no keystrokes.
export function FilterableSelect({
  value,
  onValueChange,
  options,
  id,
  placeholder,
  filterPlaceholder,
  emptyLabel,
  ariaLabel,
  disabled = false,
  size = "default",
  chrome = "form",
  className,
  contentClassName,
  optionIdPrefix,
  align = "start",
}: FilterableSelectProps) {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);
  const selected = options.find((option) => option.value === value) ?? null;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          id={id}
          type="button"
          role="combobox"
          aria-expanded={open}
          aria-label={ariaLabel}
          disabled={disabled}
          className={selectTriggerClassName({
            size,
            chrome,
            className: cn("w-full", className),
          })}
        >
          <span
            className={cn(
              "min-w-0 truncate text-left",
              !selected && "text-[var(--scry-faint)]",
            )}
          >
            {selected?.label ?? placeholder ?? ""}
          </span>
          <ChevronDown className="h-4 w-4 shrink-0 text-[var(--scry-faint)]" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align={align}
        className={cn(
          selectContentClassName("w-[var(--radix-popover-trigger-width)] p-0"),
          contentClassName,
        )}
      >
        <Command
          // The trigger already names the field; a second label here would be
          // read out twice.
          label={ariaLabel}
          className="bg-transparent"
        >
          <CommandInput
            placeholder={filterPlaceholder ?? t("label.filterOptions")}
          />
          <CommandList>
            <CommandEmpty>
              {emptyLabel ?? t("label.noMatchingOptions")}
            </CommandEmpty>
            {options.map((option) => (
              <CommandItem
                key={option.value}
                id={
                  optionIdPrefix ? `${optionIdPrefix}-${option.value}` : undefined
                }
                // Both halves are searchable: operators look for a tracker by
                // name, but paste an id at least as often.
                value={`${option.label} ${option.description ?? ""} ${option.value}`}
                disabled={option.disabled}
                onSelect={() => {
                  onValueChange(option.value);
                  setOpen(false);
                }}
              >
                <Check
                  className={cn(
                    "h-4 w-4 shrink-0",
                    option.value === value ? "opacity-100" : "opacity-0",
                  )}
                />
                <span className="min-w-0 flex-1">
                  <span className="block truncate">{option.label}</span>
                  {option.description ? (
                    <span className="block truncate text-xs text-[var(--scry-muted)]">
                      {option.description}
                    </span>
                  ) : null}
                </span>
              </CommandItem>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
