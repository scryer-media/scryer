import assert from "node:assert/strict";
import test from "node:test";

import {
  intervalToSeconds,
  scheduleDescriptionKey,
  scheduleForKind,
  splitIntervalSeconds,
  toggleWeekday,
  toScriptScheduleInput,
} from "./script-schedule.ts";

test("an interval amount is stored as seconds in its unit", () => {
  assert.equal(intervalToSeconds(15, "minutes"), 900);
  assert.equal(intervalToSeconds(6, "hours"), 21_600);
  assert.equal(intervalToSeconds(2, "days"), 172_800);
});

test("an interval is never shorter than one minute", () => {
  assert.equal(intervalToSeconds(0, "minutes"), 60);
  assert.equal(intervalToSeconds(Number.NaN, "hours"), 60);
});

test("a stored interval reads back in the largest whole unit", () => {
  assert.deepEqual(splitIntervalSeconds(172_800), { amount: 2, unit: "days" });
  assert.deepEqual(splitIntervalSeconds(5_400), {
    amount: 90,
    unit: "minutes",
  });
  assert.deepEqual(splitIntervalSeconds(7_200), { amount: 2, unit: "hours" });
});

test("a stored interval that is not whole minutes reads back in seconds", () => {
  assert.deepEqual(splitIntervalSeconds(90), { amount: 90, unit: "seconds" });
  assert.equal(intervalToSeconds(90, "seconds"), 90);
  assert.equal(intervalToSeconds(30, "seconds"), 60);
});

test("weekly days toggle on and off and stay in week order", () => {
  let days = toggleWeekday([], "FRIDAY");
  days = toggleWeekday(days, "MONDAY");
  days = toggleWeekday(days, "WEDNESDAY");
  assert.deepEqual(days, ["MONDAY", "WEDNESDAY", "FRIDAY"]);
  assert.deepEqual(toggleWeekday(days, "MONDAY"), ["WEDNESDAY", "FRIDAY"]);
});

test("the last selected weekly day cannot be toggled off", () => {
  assert.deepEqual(toggleWeekday(["THURSDAY"], "THURSDAY"), ["THURSDAY"]);
});

test("the schedule input keeps only the fields its kind reads", () => {
  const weekly = toScriptScheduleInput({
    kind: "WEEKLY",
    everySeconds: 600,
    timeLocal: "21:30",
    days: ["SUNDAY", "TUESDAY"],
    expression: "0 0 * * *",
  });
  assert.deepEqual(weekly, {
    kind: "WEEKLY",
    everySeconds: null,
    timeLocal: "21:30",
    days: ["TUESDAY", "SUNDAY"],
    expression: null,
  });
});

test("a cron expression is sent without surrounding whitespace", () => {
  const input = toScriptScheduleInput({
    kind: "CRON",
    everySeconds: null,
    timeLocal: "03:00",
    days: null,
    expression: "  */5 1-3 * * MON-FRI \n",
  });
  assert.equal(input.expression, "*/5 1-3 * * MON-FRI");
  assert.equal(input.timeLocal, null);
});

test("switching kind keeps the shared time of day", () => {
  const daily = scheduleForKind("DAILY", null);
  const weekly = scheduleForKind("WEEKLY", { ...daily, timeLocal: "06:15" });
  assert.equal(weekly.timeLocal, "06:15");
  assert.deepEqual(weekly.days, ["MONDAY"]);
  assert.deepEqual(scheduleForKind("MANUAL", weekly), {
    kind: "MANUAL",
    everySeconds: null,
    timeLocal: null,
    days: null,
    expression: null,
  });
});

test("a schedule is worded in the largest whole interval unit, singular for one", () => {
  const interval = (everySeconds: number) =>
    scheduleDescriptionKey({
      ...scheduleForKind("INTERVAL", null),
      everySeconds,
    });
  assert.deepEqual(interval(8 * 3600), {
    key: "script.schedule.describe.everyHours",
    values: { count: 8 },
  });
  assert.deepEqual(interval(3600), {
    key: "script.schedule.describe.everyHour",
    values: {},
  });
  assert.deepEqual(interval(5_400), {
    key: "script.schedule.describe.everyMinutes",
    values: { count: 90 },
  });
  assert.deepEqual(interval(172_800), {
    key: "script.schedule.describe.everyDays",
    values: { count: 2 },
  });
});

test("weekly wording carries its days in week order and cron is left to the expression", () => {
  assert.deepEqual(
    scheduleDescriptionKey({
      kind: "WEEKLY",
      everySeconds: null,
      timeLocal: "06:15",
      days: ["SUNDAY", "MONDAY"],
      expression: null,
    }),
    {
      key: "script.schedule.describe.weekly",
      values: { time: "06:15" },
      days: ["MONDAY", "SUNDAY"],
    },
  );
  assert.equal(scheduleDescriptionKey(scheduleForKind("CRON", null)), null);
});
