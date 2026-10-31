import * as React from "react";

import { HistoryEventIcon } from "@/components/common/history-event-icon";
import { getTitleHistoryFilterLabel } from "@/components/common/title-history-event-meta";
import {
  MultiSelectDropdown,
  type MultiSelectOption,
} from "@/components/ui/multi-select-dropdown";
import { useTranslate } from "@/lib/context/translate-context";

const MAX_INLINE_EVENT_TYPES = 2;

function EventTypeLabel({ eventType }: { eventType: string }) {
  const t = useTranslate();
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5">
      <HistoryEventIcon eventType={eventType} size={14} />
      <span className="truncate">
        {getTitleHistoryFilterLabel(eventType, t)}
      </span>
    </span>
  );
}

/**
 * Pick which history event types to show. No selection means every type:
 * "All Events" is ticked and the types are not, so ticking one narrows the
 * history to it, and ticking them all, or none, goes back to "All Events".
 */
export function HistoryEventTypeMultiSelect({
  eventTypes,
  selectedEventTypes,
  onSelectedEventTypesChange,
  id,
  triggerClassName,
}: {
  eventTypes: readonly string[];
  selectedEventTypes: string[];
  onSelectedEventTypesChange: (eventTypes: string[]) => void;
  id?: string;
  triggerClassName?: string;
}) {
  const t = useTranslate();
  const allSelected = selectedEventTypes.length === 0;
  const options = React.useMemo<MultiSelectOption[]>(
    () =>
      eventTypes.map((eventType) => ({
        value: eventType,
        label: <EventTypeLabel eventType={eventType} />,
      })),
    [eventTypes],
  );

  const triggerLabel = allSelected ? (
    t("history.allEvents")
  ) : selectedEventTypes.length <= MAX_INLINE_EVENT_TYPES ? (
    <span className="inline-flex min-w-0 items-center gap-3">
      {selectedEventTypes.map((eventType) => (
        <EventTypeLabel key={eventType} eventType={eventType} />
      ))}
    </span>
  ) : (
    <span className="inline-flex min-w-0 items-center gap-1.5">
      {selectedEventTypes.map((eventType) => (
        <HistoryEventIcon key={eventType} eventType={eventType} size={14} />
      ))}
      <span className="truncate">
        {t("history.eventTypesSelected", { count: selectedEventTypes.length })}
      </span>
    </span>
  );

  const handleChange = React.useCallback(
    (next: string[]) => {
      onSelectedEventTypesChange(next.length === eventTypes.length ? [] : next);
    },
    [eventTypes.length, onSelectedEventTypesChange],
  );
  const selectAll = React.useCallback(
    () => onSelectedEventTypesChange([]),
    [onSelectedEventTypesChange],
  );

  return (
    <MultiSelectDropdown
      id={id}
      options={options}
      selectedValues={selectedEventTypes}
      onSelectedValuesChange={handleChange}
      triggerLabel={triggerLabel}
      ariaLabel={t("history.eventTypes")}
      optionIdPrefix={id ? `${id}-option` : undefined}
      triggerClassName={triggerClassName}
      contentClassName="min-w-[240px]"
      allOption={{
        id: id ? `${id}-all` : undefined,
        label: t("history.allEvents"),
        selected: allSelected,
        onSelect: selectAll,
      }}
    />
  );
}
