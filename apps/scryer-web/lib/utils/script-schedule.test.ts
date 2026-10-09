import assert from "node:assert/strict";
import test from "node:test";

import {
  intervalToSeconds,
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
  assert.deepEqual(splitIntervalSeconds(5_400), { amount: 90, unit: "minutes" });
  assert.deepEqual(splitIntervalSeconds(7_200), { amount: 2, unit: "hours" });
});

test("weekly days toggle on and off and stay in week order", () => {
  let days = toggleWeekday([], "FRIDAY");
  days = toggleWeekday(days, "MONDAY");
  days = toggleWeekday(days, "WEDNESDAY");
  assert.deepEqual(days, ["MONDAY", "WEDNESDAY", "FRIDAY"]);
  assert.deepEqual(toggleWeekday(days, "MONDAY"), ["WEDNESDAY", "FRIDAY"]);
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

test("a cron expression is passed through as typed", () => {
  const input = toScriptScheduleInput({
    kind: "CRON",
    everySeconds: null,
    timeLocal: "03:00",
    days: null,
    expression: "*/5 1-3 * * MON-FRI",
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
