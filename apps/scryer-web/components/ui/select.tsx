import * as React from "react";
import { CheckIcon, ChevronDownIcon, ChevronUpIcon } from "lucide-react";
import { Select as SelectPrimitive } from "radix-ui";

import { cn } from "@/lib/utils";

export type SelectSize = "sm" | "compact" | "default" | "large";
export type SelectChrome = "form" | "toolbar" | "dialog";
export type SelectValueKind = "text" | "path" | "code";

export type SingleSelectOption = {
  value: string;
  label: React.ReactNode;
  disabled?: boolean;
  ariaLabel?: string;
};

type SelectTriggerStyleOptions = {
  size?: SelectSize;
  chrome?: SelectChrome;
  valueKind?: SelectValueKind;
  className?: string;
};

const SELECT_TRIGGER_SIZE_CLASS: Record<SelectSize, string> = {
  sm: "h-8",
  compact: "h-8",
  default: "h-10",
  large: "h-12",
};

const SELECT_TRIGGER_CHROME_CLASS: Record<SelectChrome, string> = {
  form: "rounded-[10px]",
  toolbar: "rounded-[10px] text-[13px]",
  dialog: "rounded-[11px]",
};

const SELECT_VALUE_KIND_CLASS: Record<SelectValueKind, string> = {
  text: "",
  path: "font-[var(--font-code)]",
  code: "font-[var(--font-code)]",
};

function selectTriggerClassName({
  size = "default",
  chrome = "form",
  valueKind = "text",
  className,
}: SelectTriggerStyleOptions = {}) {
  return cn(
    "flex w-fit items-center justify-between gap-2 whitespace-nowrap border px-3 text-sm text-[var(--scry-ink2)] outline-none transition-[border-color,background-color,box-shadow,color]",
    "border-[var(--scry-border2)] bg-[var(--scry-inset)] shadow-[0_1px_0_rgba(255,255,255,0.035)]",
    "hover:border-[var(--scry-bhover2)] hover:bg-[var(--scry-bg)]",
    "focus-visible:border-[rgba(var(--scry-accent-rgb),0.72)] focus-visible:ring-2 focus-visible:ring-[rgba(var(--scry-accent-rgb),0.28)]",
    "disabled:cursor-not-allowed disabled:opacity-55",
    "aria-invalid:border-[var(--scry-danger-border)] aria-invalid:ring-2 aria-invalid:ring-[var(--scry-danger-ring)]",
    "data-[placeholder]:text-[var(--scry-faint)]",
    "*:data-[slot=select-value]:line-clamp-1 *:data-[slot=select-value]:flex *:data-[slot=select-value]:min-w-0 *:data-[slot=select-value]:items-center *:data-[slot=select-value]:gap-2",
    "[&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
    "[&_svg:not([class*='text-'])]:text-[var(--scry-faint)]",
    SELECT_TRIGGER_SIZE_CLASS[size],
    SELECT_TRIGGER_CHROME_CLASS[chrome],
    SELECT_VALUE_KIND_CLASS[valueKind],
    className,
  );
}

function selectContentClassName(className?: string) {
  return cn(
    "relative z-[100] max-h-[var(--radix-select-content-available-height)] min-w-[8rem] origin-[var(--radix-select-content-transform-origin)] overflow-x-hidden overflow-y-auto rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[var(--scry-ink2)] shadow-[0_18px_42px_rgba(0,0,0,0.42)]",
    "data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95",
    "data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2",
    className,
  );
}

function selectItemClassName(className?: string) {
  return cn(
    "relative flex w-full cursor-default select-none items-center gap-2 rounded-[8px] py-1.5 pl-2 pr-8 text-sm text-[var(--scry-ink2)] outline-none transition-colors",
    "focus:bg-[var(--scry-hover)] focus:text-[var(--scry-ink)]",
    "data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
    "[&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 [&_svg:not([class*='text-'])]:text-[var(--scry-faint)]",
    "*:[span]:last:flex *:[span]:last:items-center *:[span]:last:gap-2",
    className,
  );
}

function Select({
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Root>) {
  return <SelectPrimitive.Root data-slot="select" {...props} />;
}

function SelectGroup({
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Group>) {
  return <SelectPrimitive.Group data-slot="select-group" {...props} />;
}

function SelectValue({
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Value>) {
  return <SelectPrimitive.Value data-slot="select-value" {...props} />;
}

function SelectTrigger({
  className,
  size = "default",
  chrome = "form",
  valueKind = "text",
  children,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Trigger> & {
  size?: SelectSize;
  chrome?: SelectChrome;
  valueKind?: SelectValueKind;
}) {
  return (
    <SelectPrimitive.Trigger
      data-slot="select-trigger"
      data-size={size}
      className={selectTriggerClassName({ size, chrome, valueKind, className })}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDownIcon className="size-4 opacity-70" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

function SelectContent({
  className,
  children,
  position = "item-aligned",
  align = "center",
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Content>) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        data-slot="select-content"
        className={cn(
          selectContentClassName(className),
          position === "popper" &&
            "data-[side=bottom]:translate-y-1 data-[side=left]:-translate-x-1 data-[side=right]:translate-x-1 data-[side=top]:-translate-y-1",
        )}
        position={position}
        align={align}
        {...props}
      >
        <SelectScrollUpButton />
        <SelectPrimitive.Viewport
          className={cn(
            "p-1",
            position === "popper" &&
              "h-[var(--radix-select-trigger-height)] w-full min-w-[var(--radix-select-trigger-width)] scroll-my-1",
          )}
        >
          {children}
        </SelectPrimitive.Viewport>
        <SelectScrollDownButton />
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

function SelectLabel({
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Label>) {
  return (
    <SelectPrimitive.Label
      data-slot="select-label"
      className={cn(
        "px-2 py-1.5 text-xs font-semibold uppercase tracking-wide text-[var(--scry-faint)]",
        className,
      )}
      {...props}
    />
  );
}

function SelectItem({
  className,
  children,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Item>) {
  return (
    <SelectPrimitive.Item
      data-slot="select-item"
      data-option-value={props.value}
      className={selectItemClassName(className)}
      {...props}
    >
      <span
        data-slot="select-item-indicator"
        className="absolute right-2 flex size-3.5 items-center justify-center text-[var(--scry-accent-text)]"
      >
        <SelectPrimitive.ItemIndicator>
          <CheckIcon className="size-4" />
        </SelectPrimitive.ItemIndicator>
      </span>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    </SelectPrimitive.Item>
  );
}

function SelectSeparator({
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Separator>) {
  return (
    <SelectPrimitive.Separator
      data-slot="select-separator"
      className={cn(
        "pointer-events-none -mx-1 my-1 h-px bg-[var(--scry-border3)]",
        className,
      )}
      {...props}
    />
  );
}

function SelectScrollUpButton({
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.ScrollUpButton>) {
  return (
    <SelectPrimitive.ScrollUpButton
      data-slot="select-scroll-up-button"
      className={cn(
        "flex cursor-default items-center justify-center py-1 text-[var(--scry-faint)]",
        className,
      )}
      {...props}
    >
      <ChevronUpIcon className="size-4" />
    </SelectPrimitive.ScrollUpButton>
  );
}

function SelectScrollDownButton({
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.ScrollDownButton>) {
  return (
    <SelectPrimitive.ScrollDownButton
      data-slot="select-scroll-down-button"
      className={cn(
        "flex cursor-default items-center justify-center py-1 text-[var(--scry-faint)]",
        className,
      )}
      {...props}
    >
      <ChevronDownIcon className="size-4" />
    </SelectPrimitive.ScrollDownButton>
  );
}

type SingleSelectFieldProps = Omit<
  React.ComponentProps<typeof SelectPrimitive.Root>,
  "children"
> & {
  id?: string;
  label?: React.ReactNode;
  description?: React.ReactNode;
  options: SingleSelectOption[];
  placeholder?: string;
  required?: boolean;
  size?: SelectSize;
  chrome?: SelectChrome;
  valueKind?: SelectValueKind;
  className?: string;
  labelClassName?: string;
  triggerClassName?: string;
  contentClassName?: string;
  itemClassName?: string;
  contentPosition?: React.ComponentProps<
    typeof SelectPrimitive.Content
  >["position"];
  contentAlign?: React.ComponentProps<typeof SelectPrimitive.Content>["align"];
};

function SingleSelectField({
  id,
  label,
  description,
  options,
  placeholder,
  required,
  size = "default",
  chrome = "form",
  valueKind = "text",
  className,
  labelClassName,
  triggerClassName,
  contentClassName,
  itemClassName,
  contentPosition,
  contentAlign,
  disabled,
  ...selectProps
}: SingleSelectFieldProps) {
  return (
    <div className={cn("min-w-0 space-y-1.5", className)}>
      {label ? (
        <label
          htmlFor={id}
          className={cn(
            "block text-sm font-medium text-[var(--scry-ink2)]",
            disabled && "opacity-60",
            labelClassName,
          )}
        >
          {label}
          {required ? <span aria-hidden="true"> *</span> : null}
        </label>
      ) : null}
      <Select disabled={disabled} {...selectProps}>
        <SelectTrigger
          id={id}
          size={size}
          chrome={chrome}
          valueKind={valueKind}
          className={cn("w-full", triggerClassName)}
        >
          <SelectValue placeholder={placeholder} />
        </SelectTrigger>
        <SelectContent
          position={contentPosition}
          align={contentAlign}
          className={contentClassName}
        >
          {options.map((option) => (
            <SelectItem
              key={option.value}
              value={option.value}
              disabled={option.disabled}
              aria-label={option.ariaLabel}
              className={cn(
                valueKind !== "text" && "font-[var(--font-code)]",
                itemClassName,
              )}
            >
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {description ? (
        <p
          className={cn(
            "text-xs leading-5 text-[var(--scry-muted3)]",
            disabled && "opacity-60",
          )}
        >
          {description}
        </p>
      ) : null}
    </div>
  );
}

export {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectScrollDownButton,
  SelectScrollUpButton,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
  SingleSelectField,
  selectContentClassName,
  selectItemClassName,
  selectTriggerClassName,
};
