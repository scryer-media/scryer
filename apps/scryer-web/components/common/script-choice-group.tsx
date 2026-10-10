import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { cn } from "@/lib/utils";

export type ScriptChoiceOption = {
  value: string;
  label: string;
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
      <ToggleGroup
        id={id}
        type="single"
        variant="outline"
        size="sm"
        aria-labelledby={labelId}
        className="flex h-auto flex-wrap gap-1 p-1"
        value={value}
        disabled={disabled}
        onValueChange={(next) => {
          // A single toggle group clears its value when the active item is
          // clicked again; a choice always keeps one option selected.
          if (next) onValueChange(next);
        }}
      >
        {options.map((option) => (
          <ToggleGroupItem
            key={option.value}
            id={option.id}
            value={option.value}
            data-value={option.dataValue ?? option.value}
            size="sm"
            variant="outline"
          >
            {option.label}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
      {description ? (
        <p className="text-xs leading-5 text-[var(--scry-muted3)]">{description}</p>
      ) : null}
    </div>
  );
}
