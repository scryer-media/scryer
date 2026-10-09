import type {
  ScheduleWeekday,
  ScriptSchedule,
  ScriptScheduleKind,
} from "../types/scripts.ts";
import { SCHEDULE_WEEKDAYS } from "../types/scripts.ts";

export type IntervalUnit = "minutes" | "hours" | "days";

export const INTERVAL_UNIT_SECONDS: Record<IntervalUnit, number> = {
  minutes: 60,
  hours: 60 * 60,
  days: 24 * 60 * 60,
};

/** The shortest interval a schedule accepts. */
export const MIN_INTERVAL_SECONDS = 60;

const DEFAULT_TIME = "03:00";
const DEFAULT_INTERVAL_SECONDS = 60 * 60;
const DEFAULT_CRON_EXPRESSION = "0 3 * * *";

/** Seconds for an interval amount in a unit, never shorter than one minute. */
export function intervalToSeconds(amount: number, unit: IntervalUnit): number {
  const whole = Number.isFinite(amount) ? Math.floor(amount) : 0;
  return Math.max(MIN_INTERVAL_SECONDS, whole * INTERVAL_UNIT_SECONDS[unit]);
}

/** The largest unit that expresses a stored interval as a whole amount. */
export function splitIntervalSeconds(
  seconds: number | null | undefined,
): { amount: number; unit: IntervalUnit } {
  const value = Math.max(MIN_INTERVAL_SECONDS, seconds ?? DEFAULT_INTERVAL_SECONDS);
  for (const unit of ["days", "hours"] as const) {
    if (value % INTERVAL_UNIT_SECONDS[unit] === 0) {
      return { amount: value / INTERVAL_UNIT_SECONDS[unit], unit };
    }
  }
  return { amount: Math.max(1, Math.round(value / INTERVAL_UNIT_SECONDS.minutes)), unit: "minutes" };
}

/** Days in Monday-first week order without repeats. */
export function orderWeekdays(days: readonly ScheduleWeekday[]): ScheduleWeekday[] {
  return SCHEDULE_WEEKDAYS.filter((day) => days.includes(day));
}

export function toggleWeekday(
  days: readonly ScheduleWeekday[],
  day: ScheduleWeekday,
): ScheduleWeekday[] {
  if (days.includes(day)) {
    // A weekly schedule always keeps at least one day.
    if (days.length === 1) return orderWeekdays(days);
    return orderWeekdays(days.filter((candidate) => candidate !== day));
  }
  return orderWeekdays([...days, day]);
}

/**
 * Switches a schedule to another kind, keeping what the new kind shares with
 * the old one (the time of day between daily and weekly) and filling the rest
 * with defaults. Fields the new kind does not use are cleared.
 */
export function scheduleForKind(
  kind: ScriptScheduleKind,
  previous: ScriptSchedule | null,
): ScriptSchedule {
  const empty: ScriptSchedule = {
    kind,
    everySeconds: null,
    timeLocal: null,
    days: null,
    expression: null,
  };
  switch (kind) {
    case "MANUAL":
      return empty;
    case "INTERVAL":
      return { ...empty, everySeconds: previous?.everySeconds ?? DEFAULT_INTERVAL_SECONDS };
    case "DAILY":
      return { ...empty, timeLocal: previous?.timeLocal ?? DEFAULT_TIME };
    case "WEEKLY":
      return {
        ...empty,
        timeLocal: previous?.timeLocal ?? DEFAULT_TIME,
        days: previous?.days?.length ? orderWeekdays(previous.days) : ["MONDAY"],
      };
    case "CRON":
      return { ...empty, expression: previous?.expression ?? DEFAULT_CRON_EXPRESSION };
  }
}

export function defaultScriptSchedule(): ScriptSchedule {
  return scheduleForKind("DAILY", null);
}

/**
 * The `ScriptScheduleInput` sent to the server: only the fields the kind
 * reads, with a cron expression trimmed of surrounding whitespace.
 */
export function toScriptScheduleInput(schedule: ScriptSchedule): ScriptSchedule {
  const normalized = scheduleForKind(schedule.kind, null);
  switch (schedule.kind) {
    case "MANUAL":
      return normalized;
    case "INTERVAL":
      return {
        ...normalized,
        everySeconds: Math.max(MIN_INTERVAL_SECONDS, schedule.everySeconds ?? DEFAULT_INTERVAL_SECONDS),
      };
    case "DAILY":
      return { ...normalized, timeLocal: schedule.timeLocal ?? DEFAULT_TIME };
    case "WEEKLY":
      return {
        ...normalized,
        timeLocal: schedule.timeLocal ?? DEFAULT_TIME,
        days: orderWeekdays(schedule.days ?? []),
      };
    case "CRON":
      return { ...normalized, expression: (schedule.expression ?? "").trim() };
  }
}

const SCHEDULE_KINDS: readonly ScriptScheduleKind[] = ["MANUAL", "INTERVAL", "DAILY", "WEEKLY", "CRON"];

/** Reads a schedule payload, or null when the value is not one. */
export function normalizeScriptSchedule(value: unknown): ScriptSchedule | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const record = value as Record<string, unknown>;
  const kind = SCHEDULE_KINDS.find((candidate) => candidate === record.kind);
  if (!kind) {
    return null;
  }
  return {
    kind,
    everySeconds: typeof record.everySeconds === "number" ? record.everySeconds : null,
    timeLocal: typeof record.timeLocal === "string" ? record.timeLocal : null,
    days: Array.isArray(record.days)
      ? orderWeekdays(record.days.filter((day): day is ScheduleWeekday =>
          SCHEDULE_WEEKDAYS.includes(day as ScheduleWeekday),
        ))
      : null,
    expression: typeof record.expression === "string" ? record.expression : null,
  };
}
