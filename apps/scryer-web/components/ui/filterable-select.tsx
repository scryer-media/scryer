import * as React from "react";
import { Check, ChevronDown } from "lucide-react";

import {
  Command,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  type SelectChrome,
  type SelectSize,
  type SelectValueKind,
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
  /// Longer help shown in a flyout beside the list while the option is
  /// highlighted, by pointer or keyboard. Searched by the filter too.
  detail?: string;
  disabled?: boolean;
};

type FilterableSelectProps = {
  value: string;
  onValueChange: (value: string) => void;
  options: FilterableSelectOption[];
  id?: string;
  placeholder?: string;
  filterPlaceholder?: string;
  filterByValue?: boolean;
  emptyLabel?: string;
  ariaLabel?: string;
  disabled?: boolean;
  size?: SelectSize;
  chrome?: SelectChrome;
  valueKind?: SelectValueKind;
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
  filterByValue = true,
  emptyLabel,
  ariaLabel,
  disabled = false,
  size = "default",
  chrome = "form",
  valueKind = "text",
  className,
  contentClassName,
  optionIdPrefix,
  align = "start",
}: FilterableSelectProps) {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);
  const [filter, setFilter] = React.useState("");
  // The item cmdk has highlighted, by pointer or arrow keys.
  const [active, setActive] = React.useState("");
  const selected = options.find((option) => option.value === value) ?? null;
  const filteredOptions = React.useMemo(() => {
    const query = filter.trim().toLocaleLowerCase();
    if (!query) {
      return options;
    }

    return options.filter((option) =>
      `${option.label} ${option.description ?? ""} ${option.detail ?? ""} ${filterByValue ? option.value : ""}`
        .toLocaleLowerCase()
        .includes(query),
    );
  }, [filter, filterByValue, options]);
  const itemValue = (option: FilterableSelectOption) =>
    `${option.label} ${option.description ?? ""} ${option.value}`;
  const activeDetail =
    filteredOptions.find((option) => itemValue(option) === active)?.detail ??
    null;
  const hasDetail = options.some((option) => option.detail);
  // The flyout lines up with the highlighted row, so it follows the pointer
  // and the arrow keys. It is hidden (null) while that row is scrolled out of
  // the list's view, and from a wheel or touch scroll until the pointer moves
  // again: the highlight does not follow the content under a still pointer.
  const contentRef = React.useRef<HTMLDivElement>(null);
  const [detailTop, setDetailTop] = React.useState<number | null>(null);
  // Set by a wheel or touch scroll, cleared when the pointer moves or the
  // highlight changes; the scroll events that follow leave it hidden.
  const userScrolled = React.useRef(false);
  const placeDetail = React.useCallback(() => {
    if (userScrolled.current) return;
    const content = contentRef.current;
    const list = content?.querySelector<HTMLElement>("[cmdk-list]");
    const item = list?.querySelector<HTMLElement>(
      '[cmdk-item][data-selected="true"]',
    );
    if (!content || !list || !item) {
      setDetailTop(null);
      return;
    }
    const itemRect = item.getBoundingClientRect();
    const listRect = list.getBoundingClientRect();
    const inView =
      itemRect.top >= listRect.top - 1 &&
      itemRect.bottom <= listRect.bottom + 1;
    setDetailTop(
      inView ? itemRect.top - content.getBoundingClientRect().top : null,
    );
  }, []);
  const hideDetail = React.useCallback(() => {
    userScrolled.current = true;
    setDetailTop(null);
  }, []);
  const resumeDetail = React.useCallback(() => {
    userScrolled.current = false;
    placeDetail();
  }, [placeDetail]);
  React.useLayoutEffect(() => {
    userScrolled.current = false;
    if (activeDetail) placeDetail();
  }, [active, activeDetail, filteredOptions, placeDetail]);
  const codeFont =
    valueKind !== "text" && "font-[family-name:var(--font-code)]";

  return (
    <Popover
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (!nextOpen) {
          setFilter("");
          setActive("");
        }
      }}
    >
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
            valueKind,
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
        ref={contentRef}
        align={align}
        className={cn(
          selectContentClassName("w-[var(--radix-popover-trigger-width)] p-0"),
          // The detail flyout sits outside the list's box; the list scrolls
          // itself, so the panel need not clip.
          hasDetail && "overflow-visible",
          contentClassName,
        )}
      >
        {activeDetail && detailTop !== null ? (
          <div
            role="note"
            style={{ top: detailTop }}
            className="pointer-events-none absolute left-full ml-2 hidden w-72 rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] p-3 text-xs leading-5 text-[var(--scry-ink2)] shadow-[0_18px_42px_rgba(0,0,0,0.42)] md:block"
          >
            {activeDetail}
          </div>
        ) : null}
        <Command
          // The trigger already names the field; a second label here would be
          // read out twice.
          label={ariaLabel}
          shouldFilter={false}
          value={active}
          onValueChange={setActive}
          className="bg-transparent"
        >
          <CommandInput
            value={filter}
            onValueChange={setFilter}
            placeholder={filterPlaceholder ?? t("label.filterOptions")}
          />
          <CommandList
            onScroll={activeDetail ? placeDetail : undefined}
            onWheel={activeDetail ? hideDetail : undefined}
            onTouchMove={activeDetail ? hideDetail : undefined}
            onPointerMove={
              activeDetail && detailTop === null ? resumeDetail : undefined
            }
          >
            {filteredOptions.length === 0 ? (
              <CommandEmpty>
                {emptyLabel ?? t("label.noMatchingOptions")}
              </CommandEmpty>
            ) : null}
            {filteredOptions.map((option) => (
              <CommandItem
                key={option.value}
                id={
                  optionIdPrefix
                    ? `${optionIdPrefix}-${option.value}`
                    : undefined
                }
                // Both halves are searchable: operators look for a tracker by
                // name, but paste an id at least as often.
                value={itemValue(option)}
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
                  <span className={cn("block truncate", codeFont)}>
                    {option.label}
                  </span>
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
