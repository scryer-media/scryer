import * as React from "react";
import { ChevronDown } from "lucide-react";

import { CheckboxField } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  type SelectChrome,
  type SelectSize,
  selectContentClassName,
  selectTriggerClassName,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

export type MultiSelectOption = {
  value: string;
  label: React.ReactNode;
  disabled?: boolean;
  description?: React.ReactNode;
  id?: string;
  title?: string;
};

export type MultiSelectGroup = {
  label?: React.ReactNode;
  options: MultiSelectOption[];
};

export type MultiSelectAllOption = {
  label: React.ReactNode;
  selected: boolean;
  onSelect: () => void;
  id?: string;
};

type MultiSelectOptionRowProps = {
  option: MultiSelectOption;
  checked: boolean;
  disabled: boolean;
  optionIdPrefix?: string;
  optionClassName?: string;
  optionLabelClassName?: string;
  onToggle: (value: string) => void;
};

const MultiSelectOptionRow = React.memo(function MultiSelectOptionRow({
  option,
  checked,
  disabled,
  optionIdPrefix,
  optionClassName,
  optionLabelClassName,
  onToggle,
}: MultiSelectOptionRowProps) {
  const optionDisabled = disabled || option.disabled === true;
  const fallbackId = React.useId();
  return (
    <div title={option.title}>
      <CheckboxField
        id={option.id ?? (optionIdPrefix ? `${optionIdPrefix}-${option.value}` : fallbackId)}
        checked={checked}
        disabled={optionDisabled}
        size="compact"
        onCheckedChange={() => onToggle(option.value)}
        className={cn("w-full items-center gap-2 rounded-[8px] px-2 py-1.5 hover:bg-[var(--scry-hover)]", optionClassName)}
        checkboxClassName="mt-0"
        labelClassName={cn("block truncate font-normal text-[var(--scry-ink2)]", optionLabelClassName)}
        label={option.label}
      />
    </div>
  );
});

type MultiSelectOptionListProps = {
  groups: MultiSelectGroup[];
  selectedValues: string[];
  onSelectedValuesChange: (values: string[]) => void;
  allOption?: MultiSelectAllOption;
  disabled?: boolean;
  optionIdPrefix?: string;
  className?: string;
  groupLabelClassName?: string;
  allOptionClassName?: string;
  optionClassName?: string;
  optionLabelClassName?: string;
  maxHeightClassName?: string;
};

function flattenOptions(groups: MultiSelectGroup[]) {
  return groups.flatMap((group) => group.options);
}

function MultiSelectOptionList({
  groups,
  selectedValues,
  onSelectedValuesChange,
  allOption,
  disabled = false,
  optionIdPrefix,
  className,
  groupLabelClassName,
  allOptionClassName,
  optionClassName,
  optionLabelClassName,
  maxHeightClassName = "max-h-72",
}: MultiSelectOptionListProps) {
  const allOptions = React.useMemo(() => flattenOptions(groups), [groups]);
  const allOptionId = React.useId();
  const selectedSet = React.useMemo(
    () => new Set(selectedValues),
    [selectedValues],
  );
  const allOptionsRef = React.useRef(allOptions);
  const selectedValuesRef = React.useRef(selectedValues);
  const onSelectedValuesChangeRef = React.useRef(onSelectedValuesChange);
  React.useLayoutEffect(() => {
    allOptionsRef.current = allOptions;
    selectedValuesRef.current = selectedValues;
    onSelectedValuesChangeRef.current = onSelectedValuesChange;
  }, [allOptions, onSelectedValuesChange, selectedValues]);

  const toggleOption = React.useCallback(
    (value: string) => {
      const nextSet = new Set(selectedValuesRef.current);
      if (nextSet.has(value)) {
        nextSet.delete(value);
      } else {
        nextSet.add(value);
      }

      onSelectedValuesChangeRef.current(
        allOptionsRef.current
          .map((option) => option.value)
          .filter((optionValue) => nextSet.has(optionValue)),
      );
    },
    [],
  );

  return (
    <div className={cn("flex flex-col gap-1 overflow-y-auto", maxHeightClassName, className)}>
      {allOption ? (
        <CheckboxField
          id={allOption.id ?? allOptionId}
          onCheckedChange={allOption.onSelect}
          disabled={disabled}
          checked={allOption.selected}
          size="compact"
          checkboxClassName="mt-0"
          labelClassName={cn("block truncate font-normal text-[var(--scry-ink2)]", optionLabelClassName)}
          label={allOption.label}
          className={cn(
            "w-full items-center gap-2 rounded-[8px] px-2 py-1.5 hover:bg-[var(--scry-hover)]",
            allOptionClassName,
            optionClassName,
          )}
        />
      ) : null}

      {groups.map((group, index) => (
        <div key={index} className="flex flex-col gap-1">
          {group.label ? (
            <div
              className={cn(
                "px-2 pt-1 text-xs font-semibold uppercase tracking-wide text-[var(--scry-faint)]",
                groupLabelClassName,
              )}
            >
              {group.label}
            </div>
          ) : null}
          {group.options.map((option) => (
            <MultiSelectOptionRow
              key={option.value}
              option={option}
              checked={selectedSet.has(option.value)}
              disabled={disabled}
              optionIdPrefix={optionIdPrefix}
              optionClassName={optionClassName}
              optionLabelClassName={optionLabelClassName}
              onToggle={toggleOption}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

type MultiSelectDropdownProps = {
  groups?: MultiSelectGroup[];
  options?: MultiSelectOption[];
  selectedValues: string[];
  onSelectedValuesChange: (values: string[]) => void;
  triggerLabel: React.ReactNode;
  placeholder?: React.ReactNode;
  allOption?: MultiSelectAllOption;
  disabled?: boolean;
  id?: string;
  ariaLabel?: string;
  size?: SelectSize;
  chrome?: SelectChrome;
  className?: string;
  triggerClassName?: string;
  contentClassName?: string;
  optionIdPrefix?: string;
  align?: React.ComponentProps<typeof PopoverContent>["align"];
};

function MultiSelectDropdown({
  groups,
  options,
  selectedValues,
  onSelectedValuesChange,
  triggerLabel,
  placeholder,
  allOption,
  disabled = false,
  id,
  ariaLabel,
  size = "default",
  chrome = "form",
  className,
  triggerClassName,
  contentClassName,
  optionIdPrefix,
  align = "start",
}: MultiSelectDropdownProps) {
  const normalizedGroups = React.useMemo<MultiSelectGroup[]>(
    () => groups ?? [{ options: options ?? [] }],
    [groups, options],
  );
  const hasSelection = selectedValues.length > 0 || allOption?.selected;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          id={id}
          type="button"
          aria-label={ariaLabel}
          disabled={disabled}
          className={selectTriggerClassName({
            size,
            chrome,
            className: cn("w-full", className, triggerClassName),
          })}
        >
          <span
            className={cn(
              "min-w-0 truncate text-left",
              !hasSelection && "text-[var(--scry-faint)]",
            )}
          >
            {hasSelection ? triggerLabel : placeholder ?? triggerLabel}
          </span>
          <ChevronDown className="h-4 w-4 shrink-0 text-[var(--scry-faint)]" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align={align}
        className={cn(
          selectContentClassName("w-[var(--radix-popover-trigger-width)] p-2"),
          contentClassName,
        )}
      >
        <MultiSelectOptionList
          groups={normalizedGroups}
          selectedValues={selectedValues}
          onSelectedValuesChange={onSelectedValuesChange}
          allOption={allOption}
          disabled={disabled}
          optionIdPrefix={optionIdPrefix}
        />
      </PopoverContent>
    </Popover>
  );
}

export { MultiSelectDropdown, MultiSelectOptionList };
