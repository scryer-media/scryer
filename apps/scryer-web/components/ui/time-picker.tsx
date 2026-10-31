import * as React from "react";
import { Clock3 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

const HOURS = Array.from({ length: 24 }, (_, index) =>
  index.toString().padStart(2, "0"),
);
const MINUTES = Array.from({ length: 60 }, (_, index) =>
  index.toString().padStart(2, "0"),
);

type TimePickerProps = {
  id?: string;
  value: string;
  onChange: (value: string) => void;
  /** Called when the picker closes, after any changes made while it was open. */
  onClose?: () => void;
  disabled?: boolean;
  className?: string;
  hourLabel?: string;
  minuteLabel?: string;
};

function normalizeTimeValue(value: string): { hour: string; minute: string } {
  const [rawHour, rawMinute] = value.trim().split(":");
  const parsedHour = Number.parseInt(rawHour ?? "", 10);
  const parsedMinute = Number.parseInt(rawMinute ?? "", 10);

  const hour =
    Number.isFinite(parsedHour) && parsedHour >= 0 && parsedHour <= 23
      ? parsedHour.toString().padStart(2, "0")
      : "00";
  const minute =
    Number.isFinite(parsedMinute) && parsedMinute >= 0 && parsedMinute <= 59
      ? parsedMinute.toString().padStart(2, "0")
      : "00";

  return { hour, minute };
}

export function TimePicker({
  id,
  value,
  onChange,
  onClose,
  disabled = false,
  className,
  hourLabel = "Hour",
  minuteLabel = "Minute",
}: TimePickerProps) {
  const { hour, minute } = React.useMemo(
    () => normalizeTimeValue(value),
    [value],
  );

  const updateValue = React.useCallback(
    (nextHour: string, nextMinute: string) => {
      onChange(`${nextHour}:${nextMinute}`);
    },
    [onChange],
  );

  return (
    <Popover
      onOpenChange={(open) => {
        if (!open) {
          onClose?.();
        }
      }}
    >
      <PopoverTrigger asChild>
        <Button
          id={id}
          type="button"
          variant="outline"
          disabled={disabled}
          className={cn(
            "bg-field text-foreground h-10 w-full justify-between px-3 font-normal shadow-xs",
            className,
          )}
        >
          <span>
            {hour}:{minute}
          </span>
          <Clock3 className="size-4 text-muted-foreground" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[18rem] space-y-3 p-3">
        <div className="grid grid-cols-[1fr_auto_1fr] items-end gap-2">
          <div className="space-y-2">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {hourLabel}
            </p>
            <Select
              value={hour}
              onValueChange={(nextHour) => updateValue(nextHour, minute)}
              disabled={disabled}
            >
              <SelectTrigger
                id={id ? `${id}-hour` : undefined}
                className="w-full"
                aria-label={hourLabel}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent position="popper" className="max-h-72">
                {HOURS.map((entry) => (
                  <SelectItem key={entry} value={entry}>
                    {entry}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="pb-2 text-lg font-medium text-muted-foreground">
            :
          </div>

          <div className="space-y-2">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {minuteLabel}
            </p>
            <Select
              value={minute}
              onValueChange={(nextMinute) => updateValue(hour, nextMinute)}
              disabled={disabled}
            >
              <SelectTrigger
                id={id ? `${id}-minute` : undefined}
                className="w-full"
                aria-label={minuteLabel}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent position="popper" className="max-h-72">
                {MINUTES.map((entry) => (
                  <SelectItem key={entry} value={entry}>
                    {entry}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
