export type CalendarViewMode = "dayGridMonth" | "dayGridWeek";

export const CALENDAR_VIEW_MODE_STORAGE_KEY = "scryer:calendar-view-mode";

export function parseStoredCalendarViewMode(
  value: string | null,
): CalendarViewMode | null {
  switch (value) {
    case "dayGridMonth":
    case "dayGridWeek":
      return value;
    default:
      return null;
  }
}

export function readStoredCalendarViewMode(): CalendarViewMode {
  if (typeof window === "undefined") {
    return "dayGridMonth";
  }

  try {
    return (
      parseStoredCalendarViewMode(
        window.localStorage.getItem(CALENDAR_VIEW_MODE_STORAGE_KEY),
      ) ?? "dayGridMonth"
    );
  } catch {
    return "dayGridMonth";
  }
}

export function writeStoredCalendarViewMode(mode: CalendarViewMode) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(CALENDAR_VIEW_MODE_STORAGE_KEY, mode);
  } catch {
    // Ignore persistence failures.
  }
}
